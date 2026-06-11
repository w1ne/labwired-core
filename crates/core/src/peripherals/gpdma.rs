// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32H5 GPDMA — 8-channel general-purpose DMA (RM0481 §16).
//!
//! Register map follows CMSIS `stm32h563xx.h`: top-level block at offsets
//! 0x00..0x10 (SECCFGR / PRIVCFGR / RCFGLOCKR / MISR / SMISR), then eight
//! channel blocks of 0x80 bytes starting at 0x50.
//!
//! Behavior is pinned against bench measurements: silicon capture
//! 2026-06-11 (NUCLEO-H563ZI), mem-to-mem transfers driven over SWD.
//! Key pinned facts encoded here and in the tests below:
//!
//! - reset: every channel reads `CSR = 0x0000_0001` (IDLEF), all other
//!   channel registers 0;
//! - a completed mem-to-mem block leaves `CSR = 0x0000_0301`
//!   (IDLEF | TCF | HTF — HTF latches at the half-transfer point and
//!   remains), `CBR1.BNDT = 0`, and CSAR/CDAR advanced by the block size
//!   (the H5 GPDMA, unlike the classic STM32 DMA, updates the
//!   user-visible address registers as the transfer runs);
//! - `CCR.EN` auto-clears on transfer complete (CCR reads 0 after TC);
//! - `CFCR` is write-1-to-clear for the CSR flags; `CCR.RESET`
//!   self-clears and does NOT clear CSR flags.
//!
//! Modeling notes (KISS, documented deviations from full silicon):
//! - `MISR` / `SMISR` would read the OR of per-channel flag&enable; this
//!   model returns 0 — firmware that polls per-channel CSR (the common
//!   HAL path) is unaffected.
//! - `SWREQ = 0` (peripheral-request mode, RM0481 §16.4.2) is not yet
//!   modeled: enabling a channel without SWREQ does not start a
//!   transfer and CSR stays IDLEF.
//! - Linked-list mode is not modeled: CLBAR / CLLR are plain storage
//!   the engine ignores.
//! - Transfers are paced one byte per `tick()` per channel and modeled
//!   byte-wise internally; `CBR1.BNDT` counts bytes (RM0481 §16.4.6),
//!   so SDW/DDW widths wider than a byte still produce byte-exact
//!   destination data and correct final addresses.

use crate::{DmaDirection, DmaRequest, Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;

// ---- Channel-relative register offsets (CMSIS stm32h563xx.h) ----
const CHAN_BASE: u64 = 0x50;
const CHAN_STRIDE: u64 = 0x80;
const NUM_CHANNELS: usize = 8;

const OFF_CLBAR: u64 = 0x00;
const OFF_CFCR: u64 = 0x0C;
const OFF_CSR: u64 = 0x10;
const OFF_CCR: u64 = 0x14;
const OFF_CTR1: u64 = 0x40;
const OFF_CTR2: u64 = 0x44;
const OFF_CBR1: u64 = 0x48;
const OFF_CSAR: u64 = 0x4C;
const OFF_CDAR: u64 = 0x50;
const OFF_CTR3: u64 = 0x54;
const OFF_CBR2: u64 = 0x58;
const OFF_CLLR: u64 = 0x7C;

// ---- Bit fields (RM0481 §16.5) ----
const CCR_EN: u32 = 1 << 0;
const CCR_RESET: u32 = 1 << 1;
const CCR_TCIE: u32 = 1 << 8;
const CCR_HTIE: u32 = 1 << 9;

const CSR_IDLEF: u32 = 1 << 0;
const CSR_TCF: u32 = 1 << 8;
const CSR_HTF: u32 = 1 << 9;

const CTR1_SINC: u32 = 1 << 3;
const CTR1_DINC: u32 = 1 << 19;
// Data-handling fields (RM0481 §15) — widths, padding/alignment, exchanges.
const CTR1_SDW_SHIFT: u32 = 0; // SDW_LOG2[1:0]
const CTR1_DDW_SHIFT: u32 = 16; // DDW_LOG2[17:16]
const CTR1_PAM_SHIFT: u32 = 11; // PAM[12:11]
const CTR1_SBX: u32 = 1 << 13;
const CTR1_DBX: u32 = 1 << 26;
const CTR1_DHX: u32 = 1 << 27;

const CTR2_SWREQ: u32 = 1 << 9;

const BNDT_MASK: u32 = 0xFFFF;

#[derive(Debug, Default, serde::Serialize)]
struct GpdmaChannel {
    clbar: u32,
    ccr: u32,
    /// CSR event flags (TCF/HTF/...). IDLEF (bit 0) is *not* stored here:
    /// it is derived from `active` at read time, so reset and post-CFCR
    /// reads naturally return 0x0000_0001 as pinned on silicon.
    flags: u32,
    ctr1: u32,
    ctr2: u32,
    cbr1: u32,
    csar: u32,
    cdar: u32,
    ctr3: u32,
    cbr2: u32,
    cllr: u32,
    /// Transfer in flight. Set on EN rising edge when CTR2.SWREQ = 1.
    active: bool,
    /// BNDT value latched at EN, used for the half-transfer (HTF) mark.
    bndt_initial: u32,
}

impl GpdmaChannel {
    fn csr(&self) -> u32 {
        self.flags | if self.active { 0 } else { CSR_IDLEF }
    }
}

/// STM32H5 GPDMA controller — 8 channels.
///
/// Pinned against RM0481 and silicon capture 2026-06-11 (NUCLEO-H563ZI);
/// see the module docs for the truth table and modeling limits.
#[derive(Debug, Default, serde::Serialize)]
pub struct Gpdma {
    /// SECCFGR / PRIVCFGR / RCFGLOCKR: plain R/W storage (all reset 0,
    /// silicon-pinned). The sim has no TrustZone filtering, so these
    /// only need to round-trip for firmware that programs them.
    seccfgr: u32,
    privcfgr: u32,
    rcfglockr: u32,
    channels: [GpdmaChannel; NUM_CHANNELS],
    /// Per-channel NVIC routing base (channel n pends irq_base + n via
    /// `explicit_irqs`); `None` falls back to the single configured line.
    #[serde(default)]
    irq_base: Option<u32>,
}

impl Gpdma {
    pub fn new() -> Self {
        Self::default()
    }

    /// First NVIC position of the per-channel interrupt lines. On the
    /// STM32H563, GPDMA1 channels 0..7 sit on contiguous IRQs 27..34
    /// (stm32h563xx.h); the yaml sets it via `config: { irq_base: 27 }`.
    pub fn with_irq_base(mut self, irq_base: u32) -> Self {
        self.irq_base = Some(irq_base);
        self
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.seccfgr,
            0x04 => self.privcfgr,
            0x08 => self.rcfglockr,
            // MISR: bit n mirrors (channel n CSR flags & CCR interrupt
            // enables) != 0, live — silicon-pinned 2026-06-11 (capture11):
            // ch7 TCF+TCIE reads 0x80, drops when TCIE clears, returns when
            // re-enabled, clears with CFCR. HAL_DMA_IRQHandler gates on it.
            // Flag bits [14:8] of CSR pair with enable bits [14:8] of CCR.
            0x0C => {
                let mut misr = 0u32;
                for (n, ch) in self.channels.iter().enumerate() {
                    if (ch.flags >> 8) & (ch.ccr >> 8) & 0x7F != 0 {
                        misr |= 1 << n;
                    }
                }
                misr
            }
            // SMISR: secure view — TrustZone off, reads 0.
            0x10 => 0,
            _ if offset >= CHAN_BASE => {
                let chan_idx = ((offset - CHAN_BASE) / CHAN_STRIDE) as usize;
                let reg_off = (offset - CHAN_BASE) % CHAN_STRIDE;
                if chan_idx >= NUM_CHANNELS {
                    return 0;
                }
                let ch = &self.channels[chan_idx];
                match reg_off {
                    OFF_CLBAR => ch.clbar,
                    // CFCR is write-only (w1c); reads as 0.
                    OFF_CFCR => 0,
                    OFF_CSR => ch.csr(),
                    OFF_CCR => ch.ccr,
                    OFF_CTR1 => ch.ctr1,
                    OFF_CTR2 => ch.ctr2,
                    OFF_CBR1 => ch.cbr1,
                    OFF_CSAR => ch.csar,
                    OFF_CDAR => ch.cdar,
                    OFF_CTR3 => ch.ctr3,
                    OFF_CBR2 => ch.cbr2,
                    OFF_CLLR => ch.cllr,
                    _ => 0, // reserved channel offsets read 0
                }
            }
            _ => 0, // reserved top-level offsets read 0
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.seccfgr = value,
            0x04 => self.privcfgr = value,
            0x08 => self.rcfglockr = value,
            0x0C | 0x10 => {} // MISR / SMISR are read-only
            _ if offset >= CHAN_BASE => {
                let chan_idx = ((offset - CHAN_BASE) / CHAN_STRIDE) as usize;
                let reg_off = (offset - CHAN_BASE) % CHAN_STRIDE;
                if chan_idx >= NUM_CHANNELS {
                    return;
                }
                let ch = &mut self.channels[chan_idx];
                match reg_off {
                    OFF_CLBAR => ch.clbar = value, // linked-list base: storage only
                    OFF_CFCR => {
                        // Write-1-to-clear of CSR flags. Pinned: CFCR =
                        // 0xFFFF_FFFF returns CSR to 0x0000_0001 — IDLEF
                        // is a status bit, not a clearable flag.
                        ch.flags &= !value;
                    }
                    OFF_CSR => {} // read-only
                    OFF_CCR => {
                        if value & CCR_RESET != 0 {
                            // RESET self-clears and aborts the channel.
                            // Pinned: CCR reads 0 afterwards and CSR
                            // flags are NOT cleared (still 0x301 after a
                            // post-TC RESET) — flags only clear via CFCR.
                            ch.ccr = 0;
                            ch.active = false;
                            return;
                        }
                        let old_en = ch.ccr & CCR_EN != 0;
                        ch.ccr = value;
                        let new_en = value & CCR_EN != 0;
                        // EN rising edge with software request (mem-to-mem)
                        // starts the transfer immediately. SWREQ = 0
                        // (peripheral-request mode) is not yet modeled —
                        // the channel stays idle (CSR = IDLEF).
                        if !old_en && new_en && ch.ctr2 & CTR2_SWREQ != 0 {
                            ch.active = true;
                            ch.bndt_initial = ch.cbr1 & BNDT_MASK;
                        }
                    }
                    OFF_CTR1 => ch.ctr1 = value,
                    OFF_CTR2 => ch.ctr2 = value,
                    OFF_CBR1 => ch.cbr1 = value,
                    OFF_CSAR => ch.csar = value,
                    OFF_CDAR => ch.cdar = value,
                    OFF_CTR3 => ch.ctr3 = value,
                    OFF_CBR2 => ch.cbr2 = value,
                    OFF_CLLR => ch.cllr = value, // linked-list link: storage only
                    _ => {}                      // reserved channel offsets ignore writes
                }
            }
            _ => {} // reserved top-level offsets ignore writes
        }
    }
}

impl Peripheral for Gpdma {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        let mut reg_val = self.read_reg(reg_offset);
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut dma_requests = None;
        let mut irq = false;
        let mut explicit_irqs: Option<Vec<u32>> = None;
        let irq_base = self.irq_base;
        // Real GPDMA wires one NVIC line per channel (H563: 27 + n,
        // silicon-pinned via the ch7 TCIE probe). With `irq_base` set,
        // channel n pends its own line; otherwise the block's single
        // configured line is used (legacy behavior).
        let mut pend = |ch_idx: usize, irq_flag: &mut bool| match irq_base {
            Some(base) => explicit_irqs
                .get_or_insert_with(Vec::new)
                .push(base + ch_idx as u32),
            None => *irq_flag = true,
        };

        for (ch_idx, ch) in self.channels.iter_mut().enumerate() {
            if !ch.active {
                continue;
            }
            let bndt = ch.cbr1 & BNDT_MASK;
            if bndt == 0 {
                ch.active = false;
                continue;
            }

            // One source data unit per tick (a byte-width unit mirrors the
            // classic-DMA byte pacing). The bus executes the Copy after
            // this tick, applying the CTR1 data-handling transform: width
            // conversion (PAM zero-pad / sign-extend / truncate) and the
            // SBX / DBX / DHX exchanges — pinned by the DMA_DataHandling
            // HAL example's expected vectors + its on-board run.
            let src_w = 1u32 << ((ch.ctr1 >> CTR1_SDW_SHIFT) & 0x3).min(2);
            let dst_w = 1u32 << ((ch.ctr1 >> CTR1_DDW_SHIFT) & 0x3).min(2);
            dma_requests.get_or_insert_with(Vec::new).push(DmaRequest {
                src_addr: ch.csar as u64,
                addr: ch.cdar as u64,
                val: 0,
                direction: DmaDirection::Copy,
                transform: Some(crate::DmaUnitTransform {
                    src_width: src_w as u8,
                    dst_width: dst_w as u8,
                    pam: ((ch.ctr1 >> CTR1_PAM_SHIFT) & 0x3) as u8,
                    sbx: ch.ctr1 & CTR1_SBX != 0,
                    dbx: ch.ctr1 & CTR1_DBX != 0,
                    dhx: ch.ctr1 & CTR1_DHX != 0,
                }),
            });

            // Pinned: the H5 GPDMA advances the user-visible CSAR / CDAR
            // registers as the transfer runs (post-TC they read base + 16
            // for a 16-byte block) — unlike the classic STM32 DMA, which
            // keeps CPAR / CMAR at the programmed base.
            if ch.ctr1 & CTR1_SINC != 0 {
                ch.csar = ch.csar.wrapping_add(src_w);
            }
            if ch.ctr1 & CTR1_DINC != 0 {
                ch.cdar = ch.cdar.wrapping_add(dst_w);
            }

            // BNDT counts SOURCE bytes (RM0481): one unit drains src_w.
            let bndt = bndt.saturating_sub(src_w);
            ch.cbr1 = (ch.cbr1 & !BNDT_MASK) | bndt;

            // HTF: latches at the half-transfer point and remains set
            // (pinned: post-TC CSR is 0x301 = IDLEF | TCF | HTF).
            if ch.bndt_initial >= 2 && bndt <= ch.bndt_initial / 2 && ch.flags & CSR_HTF == 0 {
                ch.flags |= CSR_HTF;
                if ch.ccr & CCR_HTIE != 0 {
                    pend(ch_idx, &mut irq);
                }
            }

            if bndt == 0 {
                // Transfer complete: TCF set, EN auto-clears (pinned:
                // CCR reads 0 after TC for an EN-only programming),
                // channel returns to idle (IDLEF).
                ch.flags |= CSR_TCF;
                ch.ccr &= !CCR_EN;
                ch.active = false;
                if ch.ccr & CCR_TCIE != 0 {
                    pend(ch_idx, &mut irq);
                }
            }
        }

        PeripheralTickResult {
            irq,
            cycles: if dma_requests.is_none() { 0 } else { 1 },
            dma_requests,
            explicit_irqs,
            ..Default::default()
        }
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::Bus;

    // Absolute channel-0 register addresses, GPDMA1 base 0x4002_0000
    // (RM0481 memory map).
    const GPDMA_BASE: u64 = 0x4002_0000;
    const GPDMA_IRQ: u32 = 27; // GPDMA1_Channel0 NVIC position (stm32h563xx.h)
    const CH0_CFCR: u64 = GPDMA_BASE + 0x5C;
    const CH0_CSR: u64 = GPDMA_BASE + 0x60;
    const CH0_CCR: u64 = GPDMA_BASE + 0x64;
    const CH0_CTR1: u64 = GPDMA_BASE + 0x90;
    const CH0_CTR2: u64 = GPDMA_BASE + 0x94;
    const CH0_CBR1: u64 = GPDMA_BASE + 0x98;
    const CH0_CSAR: u64 = GPDMA_BASE + 0x9C;
    const CH0_CDAR: u64 = GPDMA_BASE + 0xA0;

    const SRC: u64 = 0x2000_0100;
    const DST: u64 = 0x2000_0200;

    /// Same harness shape as the classic-DMA end-to-end test: a SystemBus
    /// with RAM backing, the controller registered as a peripheral, and
    /// `tick_peripherals_fully` executing the emitted Copy requests.
    fn bus_with_gpdma() -> SystemBus {
        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "gpdma1",
            GPDMA_BASE,
            0x1000,
            Some(GPDMA_IRQ),
            Box::new(Gpdma::new()),
        );
        bus
    }

    /// Program channel 0 for a SWREQ mem-to-mem byte transfer of `len`
    /// bytes (the pinned bench flow), with `extra_ccr` OR'd into CCR.
    fn program_ch0_m2m(bus: &mut SystemBus, len: u32, extra_ccr: u32) {
        bus.write_u32(CH0_CTR1, CTR1_SINC | CTR1_DINC).unwrap();
        bus.write_u32(CH0_CTR2, CTR2_SWREQ).unwrap();
        bus.write_u32(CH0_CBR1, len).unwrap();
        bus.write_u32(CH0_CSAR, SRC as u32).unwrap();
        bus.write_u32(CH0_CDAR, DST as u32).unwrap();
        bus.write_u32(CH0_CCR, CCR_EN | extra_ccr).unwrap();
    }

    fn fill_src(bus: &mut SystemBus, len: u32) {
        for i in 0..len {
            bus.write_u8(SRC + i as u64, 0xA0u8.wrapping_add(i as u8))
                .unwrap();
            bus.write_u8(DST + i as u64, 0x00).unwrap();
        }
    }

    // ---- Reset / register-map tests ----

    #[test]
    fn test_reset_csr_reads_idlef_on_all_channels() {
        // Pinned: CSR = 0x0000_0001 (IDLEF) for every channel at reset;
        // all other channel registers read 0.
        let dma = Gpdma::new();
        for x in 0..NUM_CHANNELS as u64 {
            let base = CHAN_BASE + x * CHAN_STRIDE;
            assert_eq!(dma.read_reg(base + OFF_CSR), 0x0000_0001, "ch{x} CSR");
            for off in [
                OFF_CLBAR, OFF_CFCR, OFF_CCR, OFF_CTR1, OFF_CTR2, OFF_CBR1, OFF_CSAR, OFF_CDAR,
                OFF_CTR3, OFF_CBR2, OFF_CLLR,
            ] {
                assert_eq!(dma.read_reg(base + off), 0, "ch{x} reg +{off:#x}");
            }
        }
        // Top-level registers all reset 0 (silicon-pinned).
        for off in [0x00, 0x04, 0x08, 0x0C, 0x10] {
            assert_eq!(dma.read_reg(off), 0, "top reg {off:#x}");
        }
    }

    #[test]
    fn test_top_regs_rw_storage_and_readonly_status() {
        let mut dma = Gpdma::new();
        // SECCFGR / PRIVCFGR / RCFGLOCKR: plain R/W storage.
        dma.write_reg(0x00, 0xDEAD_BEEF);
        dma.write_reg(0x04, 0x1234_5678);
        dma.write_reg(0x08, 0x0000_00FF);
        assert_eq!(dma.read_reg(0x00), 0xDEAD_BEEF);
        assert_eq!(dma.read_reg(0x04), 0x1234_5678);
        assert_eq!(dma.read_reg(0x08), 0x0000_00FF);
        // MISR / SMISR: read-only, writes ignored, read 0 (KISS model).
        dma.write_reg(0x0C, 0xFFFF_FFFF);
        dma.write_reg(0x10, 0xFFFF_FFFF);
        assert_eq!(dma.read_reg(0x0C), 0);
        assert_eq!(dma.read_reg(0x10), 0);
    }

    #[test]
    fn test_channel_regs_round_trip_storage() {
        let mut dma = Gpdma::new();
        // Channel 3 plain-storage registers (incl. CLBAR/CLLR, which the
        // engine ignores — linked-list mode not modeled).
        let base = CHAN_BASE + 3 * CHAN_STRIDE;
        for (off, val) in [
            (OFF_CLBAR, 0x2000_0000u32),
            (OFF_CTR1, 0x0008_0008),
            (OFF_CTR2, 0x0000_0200),
            (OFF_CBR1, 0x0000_0010),
            (OFF_CSAR, 0x2000_0100),
            (OFF_CDAR, 0x2000_0200),
            (OFF_CTR3, 0x0001_0001),
            (OFF_CBR2, 0x0002_0002),
            (OFF_CLLR, 0x0000_00F0),
        ] {
            dma.write_reg(base + off, val);
            assert_eq!(dma.read_reg(base + off), val, "ch3 reg +{off:#x}");
        }
        // Neighboring channels untouched.
        assert_eq!(dma.read_reg(CHAN_BASE + 2 * CHAN_STRIDE + OFF_CBR1), 0);
        assert_eq!(dma.read_reg(CHAN_BASE + 4 * CHAN_STRIDE + OFF_CBR1), 0);
    }

    // ---- Pinned mem-to-mem flow (silicon capture 2026-06-11) ----

    #[test]
    fn test_mem_to_mem_completes_with_pinned_register_state() {
        let mut bus = bus_with_gpdma();
        fill_src(&mut bus, 16);
        program_ch0_m2m(&mut bus, 16, 0);

        // One byte per tick: 16 ticks drains the block.
        for _ in 0..16 {
            bus.tick_peripherals_fully();
        }

        // Destination bytes are byte-exact copies of the source.
        for i in 0..16u64 {
            assert_eq!(
                bus.read_u8(DST + i).unwrap(),
                bus.read_u8(SRC + i).unwrap(),
                "byte {i}"
            );
        }
        // Pinned completion state: CSR = IDLEF | TCF | HTF = 0x301,
        // BNDT = 0, CSAR/CDAR advanced by 16, CCR.EN auto-cleared.
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0x0000_0301);
        assert_eq!(bus.read_u32(CH0_CBR1).unwrap() & BNDT_MASK, 0);
        assert_eq!(bus.read_u32(CH0_CSAR).unwrap(), SRC as u32 + 16);
        assert_eq!(bus.read_u32(CH0_CDAR).unwrap(), DST as u32 + 16);
        assert_eq!(bus.read_u32(CH0_CCR).unwrap(), 0);
    }

    #[test]
    fn test_htf_latches_at_half_transfer_and_remains() {
        let mut bus = bus_with_gpdma();
        fill_src(&mut bus, 16);
        program_ch0_m2m(&mut bus, 16, 0);

        // After 7 ticks: remaining 9 > 8 — no flags yet, channel busy.
        for _ in 0..7 {
            bus.tick_peripherals_fully();
        }
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0);
        // 8th tick crosses the half mark: HTF latches, no TCF, not idle.
        bus.tick_peripherals_fully();
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), CSR_HTF);
        // Finish: HTF remains alongside TCF | IDLEF (pinned 0x301).
        for _ in 0..8 {
            bus.tick_peripherals_fully();
        }
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0x0000_0301);
    }

    #[test]
    fn test_cfcr_write_one_clears_flags_back_to_idle() {
        let mut bus = bus_with_gpdma();
        fill_src(&mut bus, 16);
        program_ch0_m2m(&mut bus, 16, 0);
        for _ in 0..16 {
            bus.tick_peripherals_fully();
        }
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0x0000_0301);

        // Pinned: CFCR = 0xFFFF_FFFF returns CSR to 0x0000_0001.
        bus.write_u32(CH0_CFCR, 0xFFFF_FFFF).unwrap();
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0x0000_0001);
        // CFCR is write-only and reads as 0.
        assert_eq!(bus.read_u32(CH0_CFCR).unwrap(), 0);
    }

    #[test]
    fn test_second_transfer_after_reprogram_works_identically() {
        let mut bus = bus_with_gpdma();
        fill_src(&mut bus, 16);
        program_ch0_m2m(&mut bus, 16, 0);
        for _ in 0..16 {
            bus.tick_peripherals_fully();
        }
        bus.write_u32(CH0_CFCR, 0xFFFF_FFFF).unwrap();

        // Pinned: re-programming CBR1/CSAR/CDAR and setting EN again runs
        // a second, identical transfer.
        const SRC2: u64 = 0x2000_0300;
        const DST2: u64 = 0x2000_0400;
        for i in 0..16u64 {
            bus.write_u8(SRC2 + i, 0x50 + i as u8).unwrap();
            bus.write_u8(DST2 + i, 0).unwrap();
        }
        bus.write_u32(CH0_CBR1, 16).unwrap();
        bus.write_u32(CH0_CSAR, SRC2 as u32).unwrap();
        bus.write_u32(CH0_CDAR, DST2 as u32).unwrap();
        bus.write_u32(CH0_CCR, CCR_EN).unwrap();
        for _ in 0..16 {
            bus.tick_peripherals_fully();
        }

        for i in 0..16u64 {
            assert_eq!(bus.read_u8(DST2 + i).unwrap(), 0x50 + i as u8);
        }
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0x0000_0301);
        assert_eq!(bus.read_u32(CH0_CSAR).unwrap(), SRC2 as u32 + 16);
        assert_eq!(bus.read_u32(CH0_CDAR).unwrap(), DST2 as u32 + 16);
        assert_eq!(bus.read_u32(CH0_CCR).unwrap(), 0);
    }

    #[test]
    fn test_ccr_reset_self_clears_and_does_not_clear_csr_flags() {
        let mut bus = bus_with_gpdma();
        fill_src(&mut bus, 16);
        program_ch0_m2m(&mut bus, 16, 0);
        for _ in 0..16 {
            bus.tick_peripherals_fully();
        }
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0x0000_0301);

        // Pinned: CCR.RESET self-clears (CCR reads 0 afterwards) and does
        // NOT clear the CSR flags — those only clear via CFCR.
        bus.write_u32(CH0_CCR, CCR_RESET).unwrap();
        assert_eq!(bus.read_u32(CH0_CCR).unwrap(), 0);
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0x0000_0301);
    }

    #[test]
    fn test_ccr_reset_aborts_in_flight_transfer() {
        let mut bus = bus_with_gpdma();
        fill_src(&mut bus, 16);
        program_ch0_m2m(&mut bus, 16, 0);
        for _ in 0..4 {
            bus.tick_peripherals_fully();
        }
        bus.write_u32(CH0_CCR, CCR_RESET).unwrap();
        assert_eq!(bus.read_u32(CH0_CCR).unwrap(), 0);
        // Channel idle again, remaining count frozen, no further copies.
        assert_eq!(bus.read_u32(CH0_CSR).unwrap() & CSR_IDLEF, CSR_IDLEF);
        let frozen = bus.read_u32(CH0_CBR1).unwrap() & BNDT_MASK;
        assert_eq!(frozen, 12);
        bus.tick_peripherals_fully();
        assert_eq!(bus.read_u32(CH0_CBR1).unwrap() & BNDT_MASK, frozen);
    }

    #[test]
    fn test_swreq_zero_peripheral_mode_does_not_start() {
        // SWREQ = 0 (peripheral-request mode) is not yet modeled: the
        // transfer must not start and CSR stays IDLEF.
        let mut bus = bus_with_gpdma();
        fill_src(&mut bus, 16);
        bus.write_u32(CH0_CTR1, CTR1_SINC | CTR1_DINC).unwrap();
        bus.write_u32(CH0_CTR2, 0).unwrap(); // SWREQ = 0
        bus.write_u32(CH0_CBR1, 16).unwrap();
        bus.write_u32(CH0_CSAR, SRC as u32).unwrap();
        bus.write_u32(CH0_CDAR, DST as u32).unwrap();
        bus.write_u32(CH0_CCR, CCR_EN).unwrap();

        for _ in 0..4 {
            bus.tick_peripherals_fully();
        }
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), CSR_IDLEF);
        assert_eq!(bus.read_u32(CH0_CBR1).unwrap() & BNDT_MASK, 16);
        assert_eq!(bus.read_u8(DST).unwrap(), 0);
    }

    #[test]
    fn test_no_increment_mode_repeats_same_address() {
        // DINC = 0: every source byte lands on the same destination
        // address; the destination ends up holding the LAST source byte.
        let mut bus = bus_with_gpdma();
        fill_src(&mut bus, 4);
        bus.write_u32(CH0_CTR1, CTR1_SINC).unwrap(); // SINC only, DINC = 0
        bus.write_u32(CH0_CTR2, CTR2_SWREQ).unwrap();
        bus.write_u32(CH0_CBR1, 4).unwrap();
        bus.write_u32(CH0_CSAR, SRC as u32).unwrap();
        bus.write_u32(CH0_CDAR, DST as u32).unwrap();
        bus.write_u32(CH0_CCR, CCR_EN).unwrap();
        for _ in 0..4 {
            bus.tick_peripherals_fully();
        }
        assert_eq!(bus.read_u8(DST).unwrap(), 0xA3); // last source byte
        assert_eq!(bus.read_u8(DST + 1).unwrap(), 0); // never touched
        assert_eq!(bus.read_u32(CH0_CSAR).unwrap(), SRC as u32 + 4);
        assert_eq!(bus.read_u32(CH0_CDAR).unwrap(), DST as u32); // unmoved
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0x0000_0301);
    }

    #[test]
    fn test_tcie_pends_nvic_irq_on_completion() {
        let mut bus = bus_with_gpdma();
        fill_src(&mut bus, 2);
        program_ch0_m2m(&mut bus, 2, CCR_TCIE);

        let (interrupts, _) = bus.tick_peripherals_fully();
        assert!(!interrupts.contains(&GPDMA_IRQ), "no IRQ before TC");
        let (interrupts, _) = bus.tick_peripherals_fully();
        assert!(interrupts.contains(&GPDMA_IRQ), "TCIE should pend the IRQ");
    }

    // ---- Data-handling tests (RM0481 §15) ----
    //
    // Vectors lifted verbatim from ST's DMA_DataHandling NUCLEO-H563ZI HAL
    // example (expected-result buffers): source = B0..B7, eight bytes. The
    // same firmware passes on the bench board, so these are silicon-anchored.

    const HAL_SRC: [u8; 8] = [0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7];

    fn run_handling(ctr1_extra: u32, src_w: u32, dst_w: u32, bndt: u32) -> Vec<u8> {
        let mut bus = bus_with_gpdma();
        for (i, b) in HAL_SRC.iter().enumerate() {
            bus.write_u8(SRC + i as u64, *b).unwrap();
        }
        for i in 0..16u64 {
            bus.write_u8(DST + i, 0).unwrap();
        }
        let sdw = src_w.trailing_zeros();
        let ddw = dst_w.trailing_zeros();
        bus.write_u32(
            CH0_CTR1,
            CTR1_SINC | CTR1_DINC | (sdw << CTR1_SDW_SHIFT) | (ddw << CTR1_DDW_SHIFT) | ctr1_extra,
        )
        .unwrap();
        bus.write_u32(CH0_CTR2, CTR2_SWREQ).unwrap();
        bus.write_u32(CH0_CBR1, bndt).unwrap();
        bus.write_u32(CH0_CSAR, SRC as u32).unwrap();
        bus.write_u32(CH0_CDAR, DST as u32).unwrap();
        bus.write_u32(CH0_CCR, CCR_EN).unwrap();
        for _ in 0..32 {
            bus.tick_peripherals_fully();
        }
        (0..8u64).map(|i| bus.read_u8(DST + i).unwrap()).collect()
    }

    #[test]
    fn test_handling_byte_to_half_zero_pad() {
        let out = run_handling(0, 1, 2, 4);
        assert_eq!(out, [0xB0, 0x00, 0xB1, 0x00, 0xB2, 0x00, 0xB3, 0x00]);
    }

    #[test]
    fn test_handling_byte_to_half_sign_extend() {
        let out = run_handling(1 << CTR1_PAM_SHIFT, 1, 2, 4);
        assert_eq!(out, [0xB0, 0xFF, 0xB1, 0xFF, 0xB2, 0xFF, 0xB3, 0xFF]);
    }

    #[test]
    fn test_handling_half_to_byte_left_trunc() {
        let out = run_handling(0, 2, 1, 8);
        assert_eq!(out, [0xB0, 0xB2, 0xB4, 0xB6, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_handling_half_to_byte_right_trunc() {
        let out = run_handling(1 << CTR1_PAM_SHIFT, 2, 1, 8);
        assert_eq!(out, [0xB1, 0xB3, 0xB5, 0xB7, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_handling_src_byte_exchange() {
        let out = run_handling(CTR1_SBX, 4, 4, 8);
        assert_eq!(out, [0xB0, 0xB2, 0xB1, 0xB3, 0xB4, 0xB6, 0xB5, 0xB7]);
    }

    #[test]
    fn test_handling_dest_byte_exchange() {
        let out = run_handling(CTR1_DBX, 4, 4, 8);
        assert_eq!(out, [0xB1, 0xB0, 0xB3, 0xB2, 0xB5, 0xB4, 0xB7, 0xB6]);
    }

    #[test]
    fn test_handling_dest_halfword_exchange() {
        let out = run_handling(CTR1_DHX, 4, 4, 8);
        assert_eq!(out, [0xB2, 0xB3, 0xB0, 0xB1, 0xB6, 0xB7, 0xB4, 0xB5]);
    }
}
