// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32 GPDMA — general-purpose DMA (RM0481 §16, RM0456 §16).
//!
//! Register map follows CMSIS `stm32h563xx.h` / `stm32u575xx.h`: top-level
//! block at offsets 0x00..0x10 (SECCFGR / PRIVCFGR / RCFGLOCKR / MISR /
//! SMISR), then channel blocks of 0x80 bytes starting at 0x50. The H563
//! GPDMA1 populates 8 of those channel slots; the U575 GPDMA1 populates 16
//! (last register at 0x84C), which is the widest map the model carries.
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
//! - Transfers move one source data unit per arbitration slot (one slot per
//!   tick/event for the whole controller) and are modeled byte-wise
//!   internally; `CBR1.BNDT` counts bytes (RM0481 §16.4.6), so SDW/DDW
//!   widths wider than a byte still produce byte-exact destination data and
//!   correct final addresses.
//! - **Arbitration**: when several channels have work, the slot goes to a
//!   high-priority channel (`CCR.PRIO` = 11) first, then to the highest
//!   low-priority `PRIO` (10, 01, 00), then to the lowest channel number.
//!   Silicon shares the bus among low-priority channels by weighted round
//!   robin (the `PRIO` weight); the model serves them in strict weight order
//!   instead (documented approximation).
//! - **Data transfer error**: an item that faults on the bus (an unmapped
//!   address) sets `CSR.DTEF`, clears `CCR.EN` (the channel is back to
//!   IDLEF) and, with `CCR.DTEIE`, pends the channel's line at the next slot.
//!
//! ## Drive modes (walk-free H5 campaign)
//!
//! * **Scheduler mode** (`event-scheduler` + bus [`CycleClock`]): the
//!   arbitration slots ride ONE arbiter event (Dma1 pattern). Enabling a
//!   channel (CCR.EN 0→1 with SWREQ / linked-list) arms a delay-0 event;
//!   `on_event` runs one slot and self-perpetuates at delay 1 while any
//!   channel has work remaining (including linked-list node fetch slots).
//! * **Legacy mode** (feature off / no clock): per-cycle `tick()` runs the
//!   same slot.

use crate::{
    CycleClock, DmaDirection, DmaRequest, DmaRoute, Peripheral, PeripheralTickResult, SimResult,
};
use std::any::Any;
use std::collections::BTreeMap;

// ---- Channel-relative register offsets (CMSIS stm32h563xx.h) ----
const CHAN_BASE: u64 = 0x50;
const CHAN_STRIDE: u64 = 0x80;
/// Widest GPDMA map modeled: the STM32U575's 16 channels (the H563 populates
/// the first 8). Channel slots 8..15 stay addressable on both parts.
const NUM_CHANNELS: usize = 16;

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
const CCR_DTEIE: u32 = 1 << 10;
/// CCR.PRIO[1:0] (H563 SVD, bits 23:22): 11 high priority, 00..10 low priority.
const CCR_PRIO_SHIFT: u32 = 22;

const CSR_IDLEF: u32 = 1 << 0;
const CSR_TCF: u32 = 1 << 8;
const CSR_HTF: u32 = 1 << 9;
const CSR_DTEF: u32 = 1 << 10;

/// The scheduler token of the controller's arbiter event.
const ARBITER_TOKEN: u32 = 0;

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
    /// A data transfer error latched after its item ran (DTEIE), pended at
    /// the next slot.
    irq_deferred: bool,
    /// Peripheral-request mode: one hardware request is outstanding for this
    /// channel. A request line is a level, so this is set by
    /// [`Peripheral::dma_request`] and consumed by one data beat.
    #[serde(default)]
    request_pending: bool,
}

impl GpdmaChannel {
    fn csr(&self) -> u32 {
        self.flags | if self.active { 0 } else { CSR_IDLEF }
    }
}

/// STM32 GPDMA controller — up to 16 channels.
///
/// Pinned against RM0481/RM0456 and silicon capture 2026-06-11 (NUCLEO-H563ZI);
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
    /// Own bus base address (for linked-list node-fetch self-copies).
    #[serde(default)]
    base: u32,
    /// `<peripheral id>_tx` / `<peripheral id>_rx` -> GPDMA request-select
    /// number (RM0456 Table 136, the `GPDMA1_REQUEST_*` constants). Only
    /// channels in peripheral-request mode (`CxTR2.SWREQ = 0`) are matched.
    #[serde(default)]
    request_table: BTreeMap<String, u32>,
    /// NVIC base of channels 8..15 on the 16-channel U5 GPDMA1 (80..87).
    /// Absent keeps the historic `irq_base + n` routing for every channel.
    #[serde(default)]
    irq_high_base: Option<u32>,
    /// The channel served by the most recent slot: the issuer of the item a
    /// bus error reports ([`Peripheral::dma_transfer_error`]).
    #[serde(skip)]
    last_served: Option<usize>,
    /// Scheduler mode only: the arbiter event is live in the heap.
    #[serde(skip)]
    arbiter_live: bool,
    /// Bus-published cycle clock. `Some` once registration attaches it;
    /// `None` keeps the model on the legacy walk path.
    #[serde(skip)]
    clock: Option<CycleClock>,
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

    /// The block's own bus base address — needed by linked-list mode,
    /// whose node fetches are modeled as memory -> own-register DMA
    /// copies (the bus routes them back through `write_reg`).
    pub fn with_base(mut self, base: u32) -> Self {
        self.base = base;
        self
    }

    /// Route hardware-request names (`usart1_rx`, `spi1_tx`, …) to the
    /// GPDMA request-select numbers in `CxTR2.REQSEL`.
    pub fn with_request_table(mut self, table: BTreeMap<String, u32>) -> Self {
        self.request_table = table;
        self
    }

    /// NVIC base of channels 8..15 (the U5 GPDMA1 splits 0..7 at `irq_base`
    /// and 8..15 at this base; H563 has only 8 channels).
    pub fn with_irq_high_base(mut self, base: u32) -> Self {
        self.irq_high_base = Some(base);
        self
    }

    fn channel_irq(&self, ch_idx: usize) -> Option<u32> {
        match (self.irq_base, self.irq_high_base) {
            (Some(_base), Some(high)) if ch_idx >= 8 => Some(high + (ch_idx as u32 - 8)),
            (Some(base), _) => Some(base + ch_idx as u32),
            _ => None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    /// Test/differential knob: detach the cycle clock so the model stays
    /// on the legacy walk (`uses_scheduler() == false`).
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// True when channel `i` has work the walk/event path must service.
    #[inline]
    fn channel_needs_service(ch: &GpdmaChannel) -> bool {
        if !ch.active {
            return false;
        }
        // Peripheral-request mode moves nothing until the request line is
        // asserted (RM0456 §16.4.2); software-request mode keeps the old
        // free-running block behaviour.
        if ch.ctr2 & CTR2_SWREQ == 0 && ch.cllr == 0 {
            return ch.request_pending;
        }
        let bndt = ch.cbr1 & BNDT_MASK;
        bndt > 0 || ch.cllr != 0
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
                    _ => {
                        crate::census_reg!("gpdma:Gpdma", reg_off, "read");
                        0
                    } // reserved channel offsets read 0
                }
            }
            _ => {
                crate::census_reg!("gpdma:Gpdma", offset, "read");
                0
            } // reserved top-level offsets read 0
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
                        // Start on EN rise for software-request transfers
                        // and for linked-list starts (CLLR set, the first
                        // node delivers CTR2/CBR1 from memory).
                        if !old_en && new_en {
                            // A software-request channel starts its block at
                            // once; a peripheral-request channel becomes
                            // active and waits for its request line.
                            ch.active = true;
                            ch.bndt_initial = ch.cbr1 & BNDT_MASK;
                        }
                    }
                    OFF_CTR1 => ch.ctr1 = value,
                    OFF_CTR2 => ch.ctr2 = value,
                    OFF_CBR1 => {
                        ch.cbr1 = value;
                        // A linked-list node fetch rewrites CBR1 while the
                        // channel is live: the new block's HTF midpoint
                        // derives from the fetched size.
                        if ch.active {
                            ch.bndt_initial = value & BNDT_MASK;
                        }
                    }
                    OFF_CSAR => ch.csar = value,
                    OFF_CDAR => ch.cdar = value,
                    OFF_CTR3 => ch.ctr3 = value,
                    OFF_CBR2 => ch.cbr2 = value,
                    OFF_CLLR => ch.cllr = value, // linked-list link: storage only
                    _ => {
                        crate::census_reg!("gpdma:Gpdma", reg_off, "write");
                    } // reserved channel offsets ignore writes
                }
            }
            _ => {
                crate::census_reg!("gpdma:Gpdma", offset, "write");
            } // reserved top-level offsets ignore writes
        }
    }

    /// Service one active channel once (element transfer or linked-list
    /// node fetch). Shared by the legacy walk and the scheduler path.
    fn service_channel_once(
        &mut self,
        ch_idx: usize,
    ) -> (Option<Vec<DmaRequest>>, bool, Option<Vec<u32>>) {
        let mut dma_requests: Option<Vec<DmaRequest>> = None;
        let mut irq = false;
        let mut explicit_irqs: Option<Vec<u32>> = None;
        let channel_irq: Vec<Option<u32>> =
            (0..NUM_CHANNELS).map(|i| self.channel_irq(i)).collect();
        let mut pend = |ch_idx: usize, irq_flag: &mut bool| match channel_irq[ch_idx] {
            Some(line) => explicit_irqs.get_or_insert_with(Vec::new).push(line),
            None => *irq_flag = true,
        };

        if ch_idx >= NUM_CHANNELS {
            return (None, false, None);
        }
        let ch = &mut self.channels[ch_idx];
        if !ch.active {
            return (None, false, None);
        }
        let peripheral_mode = ch.ctr2 & CTR2_SWREQ == 0 && ch.cllr == 0;
        if peripheral_mode {
            if !ch.request_pending {
                return (None, false, None);
            }
            ch.request_pending = false;
        }
        let bndt = ch.cbr1 & BNDT_MASK;
        if bndt == 0 {
            if ch.cllr != 0 {
                // Linked-list: fetch the next node (memory → own-register
                // copies the bus executes after this service).
                let node = ((ch.clbar & 0xFFFF_0000) | (ch.cllr & 0xFFFC)) as u64;
                let regs: [(u32, u64); 8] = [
                    (1 << 31, OFF_CTR1),
                    (1 << 30, OFF_CTR2),
                    (1 << 29, OFF_CBR1),
                    (1 << 28, OFF_CSAR),
                    (1 << 27, OFF_CDAR),
                    (1 << 26, OFF_CTR3),
                    (1 << 25, OFF_CBR2),
                    (1 << 16, OFF_CLLR),
                ];
                let ch_base = self.base as u64 + CHAN_BASE + ch_idx as u64 * CHAN_STRIDE;
                let mut word = 0u64;
                for (flag, reg_off) in regs {
                    if ch.cllr & flag != 0 {
                        for k in 0..4u64 {
                            dma_requests.get_or_insert_with(Vec::new).push(DmaRequest {
                                src_addr: node + word * 4 + k,
                                addr: ch_base + reg_off + k,
                                val: 0,
                                direction: DmaDirection::Copy,
                                transform: None,
                                remaining: None,
                            });
                        }
                        word += 1;
                    }
                }
            } else {
                ch.active = false;
            }
            return (dma_requests, false, None);
        }

        // One source data unit (classic-DMA byte pacing). The bus applies
        // CTR1 data-handling after this service.
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
            remaining: None,
        });

        // H5 GPDMA advances user-visible CSAR/CDAR as the transfer runs.
        if ch.ctr1 & CTR1_SINC != 0 {
            ch.csar = ch.csar.wrapping_add(src_w);
        }
        if ch.ctr1 & CTR1_DINC != 0 {
            ch.cdar = ch.cdar.wrapping_add(dst_w);
        }

        let bndt = bndt.saturating_sub(src_w);
        ch.cbr1 = (ch.cbr1 & !BNDT_MASK) | bndt;

        if ch.bndt_initial >= 2 && bndt <= ch.bndt_initial / 2 && ch.flags & CSR_HTF == 0 {
            ch.flags |= CSR_HTF;
            if ch.ccr & CCR_HTIE != 0 {
                pend(ch_idx, &mut irq);
            }
        }

        if bndt == 0 {
            ch.flags |= CSR_TCF;
            if ch.ccr & CCR_TCIE != 0 {
                pend(ch_idx, &mut irq);
            }
            if ch.cllr == 0 {
                ch.ccr &= !CCR_EN;
                ch.active = false;
            }
        }

        (dma_requests, irq, explicit_irqs)
    }

    /// The channel the arbiter serves this slot: high-priority channels
    /// (`CCR.PRIO` = 11) before low-priority ones, a higher `PRIO` first
    /// among those, then the lowest channel number.
    fn arbiter_winner(&self) -> Option<usize> {
        let prio = |ch: &GpdmaChannel| (ch.ccr >> CCR_PRIO_SHIFT) & 3;
        let mut best: Option<usize> = None;
        for (i, ch) in self.channels.iter().enumerate() {
            if !Self::channel_needs_service(ch) {
                continue;
            }
            if best.is_none_or(|b| prio(ch) > prio(&self.channels[b])) {
                best = Some(i);
            }
        }
        best
    }

    fn has_work(&self) -> bool {
        self.channels.iter().any(|c| c.irq_deferred) || self.arbiter_winner().is_some()
    }

    /// One arbitration slot — the ONE body both drive modes run: deferred
    /// error interrupts, then the winning channel's element (or node fetch).
    fn tick_channels_once(&mut self) -> PeripheralTickResult {
        let mut dma_requests: Option<Vec<DmaRequest>> = None;
        let mut irq = false;
        let mut explicit_irqs: Option<Vec<u32>> = None;
        for ch_idx in 0..NUM_CHANNELS {
            if std::mem::take(&mut self.channels[ch_idx].irq_deferred) {
                match self.channel_irq(ch_idx) {
                    Some(line) => explicit_irqs.get_or_insert_with(Vec::new).push(line),
                    None => irq = true,
                }
            }
        }
        self.last_served = self.arbiter_winner();
        if let Some(ch_idx) = self.last_served {
            let (reqs, chan_irq, chan_explicit) = self.service_channel_once(ch_idx);
            if let Some(r) = reqs {
                dma_requests.get_or_insert_with(Vec::new).extend(r);
            }
            irq |= chan_irq;
            if let Some(e) = chan_explicit {
                explicit_irqs.get_or_insert_with(Vec::new).extend(e);
            }
        }
        PeripheralTickResult {
            irq,
            // Walk-free normalization (Dma1 B4): charge zero so walk-on
            // reference and scheduler path agree cycle-for-cycle.
            cycles: 0,
            dma_requests,
            explicit_irqs,
            ..Default::default()
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
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.tick_channels_once()
    }

    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        // Oracle settle path: one element transition, even under scheduler mode.
        self.tick_channels_once()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    /// A hardware request line is asserted by a peripheral (`spi1_tx`,
    /// `usart1_rx`, …). Route it to every enabled channel whose
    /// `CxTR2.REQSEL` selects that request in peripheral-request mode.
    fn dma_route_request(&self, request: &str, out: &mut Vec<DmaRoute>) {
        let Some(&reqsel) = self.request_table.get(request) else {
            return;
        };
        for (idx, ch) in self.channels.iter().enumerate() {
            if ch.ccr & CCR_EN == 0 || ch.ctr2 & CTR2_SWREQ != 0 {
                continue;
            }
            if ch.ctr2 & 0x7F == reqsel {
                out.push(DmaRoute {
                    controller: None,
                    channel: idx as u32,
                });
            }
        }
    }

    /// One routed request: arm one data beat on the channel. The peripheral
    /// request is a level, so the beat runs at the next arbitration slot.
    fn dma_request(&mut self, channel: u32) {
        let idx = channel as usize;
        if let Some(ch) = self.channels.get_mut(idx) {
            if ch.active && ch.ctr2 & CTR2_SWREQ == 0 {
                ch.request_pending = true;
            }
        }
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn sync_to(&mut self, _now_cycle: u64) {
        // Readable registers mutate only at transfer events.
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        if self.has_work() && !self.arbiter_live {
            self.arbiter_live = true;
            return vec![(0, ARBITER_TOKEN)];
        }
        Vec::new()
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != ARBITER_TOKEN {
            return crate::sched::EventResult::default();
        }
        let res = self.tick_channels_once();
        let reschedule = self.has_work();
        self.arbiter_live = reschedule;
        crate::sched::EventResult {
            raise_own_irq: res.irq,
            explicit_irqs: res.explicit_irqs.unwrap_or_default(),
            reschedule_delay: reschedule.then_some(1),
            dma_requests: res.dma_requests.unwrap_or_default(),
            ..Default::default()
        }
    }

    fn dma_transfer_error(&mut self, _item: &DmaRequest) {
        // RM0481 (GPDMA error management): a data transfer error sets DTEF, and the channel
        // is disabled (EN cleared, CSR back to IDLEF).
        let Some(i) = self.last_served else {
            return;
        };
        let ch = &mut self.channels[i];
        if ch.flags & CSR_DTEF != 0 && !ch.active {
            return; // a later item of the same faulting slot
        }
        ch.flags |= CSR_DTEF;
        ch.ccr &= !CCR_EN;
        ch.active = false;
        ch.irq_deferred |= ch.ccr & CCR_DTEIE != 0;
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
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
            Box::new(Gpdma::new().with_base(GPDMA_BASE as u32)),
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
            bus.tick_peripherals_fully_forced();
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
            bus.tick_peripherals_fully_forced();
        }
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0);
        // 8th tick crosses the half mark: HTF latches, no TCF, not idle.
        bus.tick_peripherals_fully_forced();
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), CSR_HTF);
        // Finish: HTF remains alongside TCF | IDLEF (pinned 0x301).
        for _ in 0..8 {
            bus.tick_peripherals_fully_forced();
        }
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0x0000_0301);
    }

    #[test]
    fn test_cfcr_write_one_clears_flags_back_to_idle() {
        let mut bus = bus_with_gpdma();
        fill_src(&mut bus, 16);
        program_ch0_m2m(&mut bus, 16, 0);
        for _ in 0..16 {
            bus.tick_peripherals_fully_forced();
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
            bus.tick_peripherals_fully_forced();
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
            bus.tick_peripherals_fully_forced();
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
            bus.tick_peripherals_fully_forced();
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
            bus.tick_peripherals_fully_forced();
        }
        bus.write_u32(CH0_CCR, CCR_RESET).unwrap();
        assert_eq!(bus.read_u32(CH0_CCR).unwrap(), 0);
        // Channel idle again, remaining count frozen, no further copies.
        assert_eq!(bus.read_u32(CH0_CSR).unwrap() & CSR_IDLEF, CSR_IDLEF);
        let frozen = bus.read_u32(CH0_CBR1).unwrap() & BNDT_MASK;
        assert_eq!(frozen, 12);
        bus.tick_peripherals_fully_forced();
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
            bus.tick_peripherals_fully_forced();
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
            bus.tick_peripherals_fully_forced();
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

        let (interrupts, _) = bus.tick_peripherals_fully_forced();
        assert!(!interrupts.contains(&GPDMA_IRQ), "no IRQ before TC");
        let (interrupts, _) = bus.tick_peripherals_fully_forced();
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
            bus.tick_peripherals_fully_forced();
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

    /// Two-block linked-list chain, mirroring the 2026-06-12 silicon probe
    /// (capture12): block A copies 8 bytes, then the node at CLBAR|LA
    /// rewrites CTR1/CTR2/CBR1/CSAR/CDAR/CLLR and block B copies 8 more.
    /// Final state pinned: both destinations byte-exact, CLLR = 0,
    /// BNDT = 0, CSR = 0x301, CCR keeps TCIE with EN cleared.
    #[test]
    fn test_linked_list_two_block_chain_matches_silicon() {
        let mut bus = bus_with_gpdma();
        for i in 0..8u64 {
            bus.write_u8(SRC + i, 0x10 + i as u8).unwrap();
            bus.write_u8(SRC + 0x40 + i, 0x30 + i as u8).unwrap();
            bus.write_u8(DST + i, 0).unwrap();
            bus.write_u8(DST + 0x40 + i, 0).unwrap();
        }
        // Node at SRC + 0x200: CTR1, CTR2, CBR1, CSAR, CDAR, CLLR(=0).
        let node = SRC + 0x200;
        bus.write_u32(node, CTR1_SINC | CTR1_DINC).unwrap();
        bus.write_u32(node + 4, CTR2_SWREQ).unwrap();
        bus.write_u32(node + 8, 8).unwrap();
        bus.write_u32(node + 12, (SRC + 0x40) as u32).unwrap();
        bus.write_u32(node + 16, (DST + 0x40) as u32).unwrap();
        bus.write_u32(node + 20, 0).unwrap();

        bus.write_u32(GPDMA_BASE + 0x50, (SRC & 0xFFFF_0000) as u32)
            .unwrap(); // CLBAR
        bus.write_u32(CH0_CTR1, CTR1_SINC | CTR1_DINC).unwrap();
        bus.write_u32(CH0_CTR2, CTR2_SWREQ).unwrap();
        bus.write_u32(CH0_CBR1, 8).unwrap();
        bus.write_u32(CH0_CSAR, SRC as u32).unwrap();
        bus.write_u32(CH0_CDAR, DST as u32).unwrap();
        // CLLR: UT1|UT2|UB1|USA|UDA|ULL + LA of the node.
        let ull: u32 = (0x1F << 27) | (1 << 16) | ((node as u32) & 0xFFFC);
        bus.write_u32(GPDMA_BASE + 0xCC, ull).unwrap();
        bus.write_u32(CH0_CCR, CCR_EN | CCR_TCIE).unwrap();

        for _ in 0..40 {
            bus.tick_peripherals_fully_forced();
        }

        for i in 0..8u64 {
            assert_eq!(bus.read_u8(DST + i).unwrap(), 0x10 + i as u8, "block A");
            assert_eq!(
                bus.read_u8(DST + 0x40 + i).unwrap(),
                0x30 + i as u8,
                "block B"
            );
        }
        assert_eq!(bus.read_u32(GPDMA_BASE + 0xCC).unwrap(), 0, "CLLR");
        assert_eq!(bus.read_u32(CH0_CBR1).unwrap() & 0xFFFF, 0, "BNDT");
        assert_eq!(bus.read_u32(CH0_CSR).unwrap(), 0x301, "CSR");
        assert_eq!(
            bus.read_u32(CH0_CCR).unwrap(),
            CCR_TCIE,
            "CCR keeps TCIE, EN cleared at list end"
        );
    }

    // ---- 16-channel map (STM32U575; RM0456 §16, SVD GPDMA1_CH0..CH15) ----

    #[test]
    fn test_channels_14_and_15_register_access() {
        assert_eq!(NUM_CHANNELS, 16, "U5 GPDMA has 16 channels");
        assert_eq!(CHAN_STRIDE, 0x80, "channel stride");
        let mut dma = Gpdma::new();
        for idx in [14u64, 15] {
            let base = CHAN_BASE + idx * CHAN_STRIDE;
            assert_eq!(
                dma.read_reg(base + OFF_CSR),
                0x0000_0001,
                "ch{idx} reset CSR"
            );
            dma.write_reg(base + OFF_CBR1, 0x0000_0040);
            dma.write_reg(base + OFF_CSAR, 0x2000_3000);
            dma.write_reg(base + OFF_CDAR, 0x2000_4000);
            assert_eq!(dma.read_reg(base + OFF_CBR1), 0x40, "ch{idx} CBR1");
            assert_eq!(dma.read_reg(base + OFF_CSAR), 0x2000_3000, "ch{idx} CSAR");
            assert_eq!(dma.read_reg(base + OFF_CDAR), 0x2000_4000, "ch{idx} CDAR");
        }
    }

    #[test]
    fn test_channel_15_mem_to_mem_completes() {
        let mut bus = bus_with_gpdma();
        fill_src(&mut bus, 8);
        let c15 = GPDMA_BASE + CHAN_BASE + 15 * CHAN_STRIDE;
        bus.write_u32(c15 + OFF_CTR1, CTR1_SINC | CTR1_DINC)
            .unwrap();
        bus.write_u32(c15 + OFF_CTR2, CTR2_SWREQ).unwrap();
        bus.write_u32(c15 + OFF_CBR1, 8).unwrap();
        bus.write_u32(c15 + OFF_CSAR, SRC as u32).unwrap();
        bus.write_u32(c15 + OFF_CDAR, DST as u32).unwrap();
        bus.write_u32(c15 + OFF_CCR, CCR_EN).unwrap();

        for _ in 0..8 {
            bus.tick_peripherals_fully_forced();
        }

        for i in 0..8u64 {
            assert_eq!(
                bus.read_u8(DST + i).unwrap(),
                bus.read_u8(SRC + i).unwrap(),
                "byte {i}"
            );
        }
        assert_eq!(bus.read_u32(c15 + OFF_CSR).unwrap(), 0x0000_0301);
        assert_eq!(bus.read_u32(c15 + OFF_CBR1).unwrap() & BNDT_MASK, 0);
        assert_eq!(bus.read_u32(c15 + OFF_CCR).unwrap(), 0);
    }

    /// Program channel `n` for an 8-byte SWREQ mem-to-mem copy at `prio`.
    fn program_m2m(bus: &mut SystemBus, n: u64, src: u64, dst: u64, prio: u32, extra: u32) {
        let c = GPDMA_BASE + CHAN_BASE + n * CHAN_STRIDE;
        bus.write_u32(c + OFF_CTR1, CTR1_SINC | CTR1_DINC).unwrap();
        bus.write_u32(c + OFF_CTR2, CTR2_SWREQ).unwrap();
        bus.write_u32(c + OFF_CBR1, 8).unwrap();
        bus.write_u32(c + OFF_CSAR, src as u32).unwrap();
        bus.write_u32(c + OFF_CDAR, dst as u32).unwrap();
        bus.write_u32(c + OFF_CCR, CCR_EN | (prio << CCR_PRIO_SHIFT) | extra)
            .unwrap();
    }

    /// One unit per slot for the whole controller: CCR.PRIO = 11 (high)
    /// runs before a low-priority channel, a lower channel number wins
    /// between equals.
    #[test]
    fn test_arbiter_serves_by_prio_then_channel_number() {
        fn order(p0: u32, p3: u32) -> Vec<u64> {
            let mut bus = bus_with_gpdma();
            fill_src(&mut bus, 8);
            program_m2m(&mut bus, 0, SRC, DST, p0, 0);
            program_m2m(&mut bus, 3, SRC, DST + 0x40, p3, 0);
            let mut done = Vec::new();
            for _ in 0..20 {
                bus.tick_peripherals_fully_forced();
                for n in [0u64, 3] {
                    let csr = GPDMA_BASE + CHAN_BASE + n * CHAN_STRIDE + OFF_CSR;
                    if bus.read_u32(csr).unwrap() & CSR_TCF != 0 && !done.contains(&n) {
                        done.push(n);
                    }
                }
            }
            done
        }
        assert_eq!(order(0, 3), vec![3, 0], "high priority first");
        assert_eq!(order(2, 1), vec![0, 3], "higher low-priority weight first");
        assert_eq!(order(1, 1), vec![0, 3], "equal: lower channel number");
    }

    /// A unit that faults on the bus: DTEF, EN cleared (IDLEF), DTEIE pends
    /// the channel's line.
    #[test]
    fn test_bus_error_sets_dtef_and_disables_the_channel() {
        let mut bus = bus_with_gpdma();
        program_m2m(&mut bus, 0, 0x5FFF_0000, DST, 0, CCR_DTEIE);
        bus.tick_peripherals_fully_forced();
        let csr = bus.read_u32(CH0_CSR).unwrap();
        assert_eq!(csr & (CSR_DTEF | CSR_IDLEF), CSR_DTEF | CSR_IDLEF);
        assert_eq!(bus.read_u32(CH0_CCR).unwrap() & CCR_EN, 0);
        // MISR mirrors DTEF & DTEIE; the per-channel line pends next slot.
        assert_eq!(bus.read_u32(GPDMA_BASE + 0x0C).unwrap() & 1, 1);
        let mut g = Gpdma::new().with_irq_base(GPDMA_IRQ);
        g.write_reg(0x50 + OFF_CTR2, CTR2_SWREQ);
        g.write_reg(0x50 + OFF_CBR1, 4);
        g.write_reg(0x50 + OFF_CCR, CCR_EN | CCR_DTEIE);
        let item = g.tick().dma_requests.unwrap()[0];
        g.dma_transfer_error(&item);
        assert_eq!(g.tick().explicit_irqs, Some(vec![GPDMA_IRQ]));
    }
}
