// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 SAR ADC controller (`APB_SARADC`, `0x6004_0000`) — behavioral
//! one-shot (RTC) conversion engine.
//!
//! This is the controller the IDF `adc_oneshot` driver drives for ad-hoc
//! reads, DISTINCT from the cal-completion helper in `sar_adc.rs` (which only
//! makes the IDF's PHY/clock-bring-up self-calibration terminate). The chip
//! YAML wires THIS model for `apb_saradc` so a one-shot conversion produces a
//! genuine, channel-dependent result rather than a constant.
//!
//! ## One-shot handshake (register offsets + fields from
//! `configs/peripherals/esp32c3/apb_saradc.yaml`)
//!
//! The `adc_oneshot_read` flow:
//!   1. Program `ONETIME_SAMPLE` (0x20): `SARADC_ONETIME_CHANNEL` [28:25],
//!      `SARADC_ONETIME_ATTEN` [24:23], and the unit-select bits
//!      `SARADC1_ONETIME_SAMPLE` [31] / `SARADC2_ONETIME_SAMPLE` [30].
//!   2. Set `SARADC_ONETIME_START` [29] → trigger one conversion.
//!   3. Busy-poll `INT_RAW` (0x44) for `APB_SARADC1_DONE` [31] (SAR1) /
//!      `APB_SARADC2_DONE` [30] (SAR2).
//!   4. Read the result from `SAR1DATA_STATUS` (0x2C) / `SAR2DATA_STATUS`
//!      (0x30): `APB_SARADC{1,2}_DATA` [16:0]. The C3 packs the source channel
//!      into bits [16:13] and the 12-bit sample into [12:0] (the IDF macros
//!      `ADC_GET_CHANNEL`/`ADC_GET_DATA` decode it that way).
//!   5. Acknowledge via `INT_CLR` (0x4C, W1C).
//!
//! ## Conversion model — deterministic, channel-dependent
//!
//! There is no real analog front-end, so each channel is assigned a FIXED
//! 12-bit source code via [`channel_sample`]: an injective ramp
//! (`0x100 + channel * 0x111`, saturated to 12 bits). On `ONETIME_START` we
//! latch `(channel << 13) | code` into the selected unit's data register and
//! set its DONE bit; `ONETIME_START` self-clears. Because the result is a
//! function of the SELECTED channel — reading channel 3 yields a different,
//! predictable value than channel 5 — a declarative register file (which would
//! return a constant and never raise DONE) cannot reproduce this behavior.
//! Resolution is fixed at the C3's native 12 bits.

use std::cell::Cell;

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

pub const APB_SARADC_BASE: u32 = 0x6004_0000;
pub const APB_SARADC_SIZE: u64 = 0x1000;

/// `APB_SARADC` interrupt-matrix source on the C3 (`APB_ADC_INT_MAP` at
/// `interrupt_core0.yaml` offset 172 = `4 * 43`).
pub const APB_SARADC_INTR_SOURCE_ID: u32 = 43;

const SAR1DATA_STATUS: u64 = 0x2C;
const SAR2DATA_STATUS: u64 = 0x30;
const ONETIME_SAMPLE: u64 = 0x20;
const INT_ENA: u64 = 0x40;
const INT_RAW: u64 = 0x44;
const INT_ST: u64 = 0x48;
const INT_CLR: u64 = 0x4C;

/// `SARADC_ONETIME_START` (ONETIME_SAMPLE bit 29) — conversion trigger.
const ONETIME_START: u32 = 1 << 29;
/// `SARADC2_ONETIME_SAMPLE` (bit 30) — select SAR2 for the one-shot.
const SAR2_ONETIME_SAMPLE: u32 = 1 << 30;
/// `SARADC1_ONETIME_SAMPLE` (bit 31) — select SAR1 for the one-shot.
const SAR1_ONETIME_SAMPLE: u32 = 1 << 31;
/// `SARADC_ONETIME_CHANNEL` (bits [28:25]) — channel index for the one-shot.
const ONETIME_CHANNEL_SHIFT: u32 = 25;
const ONETIME_CHANNEL_MASK: u32 = 0xF;

/// `APB_SARADC2_DONE_INT` (INT bit 30).
const SAR2_DONE_INT: u32 = 1 << 30;
/// `APB_SARADC1_DONE_INT` (INT bit 31).
const SAR1_DONE_INT: u32 = 1 << 31;
/// All architected interrupt bits ([31:26]).
const INT_MASK: u32 = 0xFC00_0000;

/// `APB_SARADC{1,2}_DATA` field width in the data-status registers (bits[16:0]).
const DATA_MASK: u32 = 0x0001_FFFF;
/// The 12-bit ADC sample occupies bits [12:0]; the source channel is packed
/// into bits [16:13] (the IDF `ADC_GET_CHANNEL`/`ADC_GET_DATA` layout).
const DATA_CHANNEL_SHIFT: u32 = 13;

/// Silicon cold-reset values of the register-backed `APB_SARADC` config
/// registers (`(offset, reset_value)`), corroborated against a live ESP32-C3
/// (rev v0.4) in `crates/hw-oracle/tests/esp32c3_reset_conformance.rs`. These
/// power up non-zero; seeding them keeps the behavioral model's cold-reset
/// readback identical to the demoted declarative descriptor. The behavioral
/// one-shot logic only reacts to writes, so these defaults do not affect it.
const RESET_REGS: &[(u64, u32)] = &[
    (0x00, 0x4003_8240), // CTRL
    (0x04, 0x0000_A1FE), // CTRL2
    (0x0C, 0x00FF_0808), // FSM_WAIT
    (0x20, 0x1A00_0000), // ONETIME_SAMPLE
    (0x24, 0x0000_0900), // ARB_CTRL
    (0x50, 0x0000_00FF), // DMA_CONF
    (0x54, 0x0000_0004), // CLKM_CONF
    (0x60, 0x0000_8000), // CALI
];

/// Deterministic fixed 12-bit source code for `channel`. The ramp is injective
/// over the C3's five ADC1 channels so a reader can prove the result tracks the
/// SELECTED channel, not a constant.
fn channel_sample(channel: u32) -> u32 {
    (0x100 + channel * 0x111) & 0x0FFF
}

pub struct Esp32c3ApbSarAdc {
    /// Interrupt-matrix source ID (`APB_SARADC` = 43 on the C3).
    source_id: u32,
    /// Register-backed storage for the whole 0x400-byte window (word indexed).
    regs: Vec<u32>,
    /// Latched raw interrupt bits (`INT_RAW`). Separate from `regs` so the
    /// W1C / RO semantics of the INT block are explicit.
    int_raw: Cell<u32>,
    /// Latched conversion results for SAR1 / SAR2 (the `*DATA_STATUS` regs).
    sar1_data: u32,
    sar2_data: u32,
    /// Bus-published cycle clock (walk-free plan). `Some` once
    /// `SystemBus::push_peripheral`/`add_peripheral` attaches it. Its presence
    /// (under the `event-scheduler` feature) flips the model onto the event
    /// scheduler: the per-cycle walk skips this peripheral and the bus derives
    /// its level-sensitive matrix IRQ from [`Self::matrix_irq_sources`] instead
    /// of the walk's `explicit_irqs`. This one-shot controller has NO
    /// free-running counter — `int_raw` is write-armed by `convert()` on the
    /// ONETIME_START strobe — so there are no scheduled events: the migration is
    /// level-export only. `None` (feature off, a hand-built bus, or the
    /// differential's `force_legacy_walk`) keeps the legacy per-cycle walk. Not
    /// serialized — re-attached by the bus.
    clock: Option<CycleClock>,
}

impl Default for Esp32c3ApbSarAdc {
    fn default() -> Self {
        Self::new(APB_SARADC_INTR_SOURCE_ID)
    }
}

impl std::fmt::Debug for Esp32c3ApbSarAdc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32c3ApbSarAdc(src={}, int_raw=0x{:08x}, sar1=0x{:05x}, sar2=0x{:05x})",
            self.source_id,
            self.int_raw.get(),
            self.sar1_data,
            self.sar2_data
        )
    }
}

impl Esp32c3ApbSarAdc {
    pub fn new(source_id: u32) -> Self {
        // Seed the register-backed window with the silicon cold-reset state so
        // the model's reset readback matches the captured oracle; the remaining
        // registers power up at 0.
        let mut regs = vec![0u32; (APB_SARADC_SIZE / 4) as usize];
        for &(off, val) in RESET_REGS {
            regs[(off / 4) as usize] = val;
        }
        Self {
            source_id,
            regs,
            int_raw: Cell::new(0),
            sar1_data: 0,
            sar2_data: 0,
            clock: None,
        }
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to the
    /// legacy per-cycle walk (`uses_scheduler() == false`). Used by the
    /// walk-on-vs-scheduler differential gate to build the reference config from
    /// the same bus assembly (mirrors `Esp32c3I2c::force_legacy_walk`).
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    fn int_st(&self) -> u32 {
        self.int_raw.get() & self.reg(INT_ENA)
    }

    fn reg(&self, off: u64) -> u32 {
        *self.regs.get((off / 4) as usize).unwrap_or(&0)
    }

    /// Run one conversion of `channel` on the selected SAR unit(s): latch the
    /// channel-dependent packed result and raise the matching DONE bit.
    fn convert(&mut self, sample_word: u32) {
        let channel = (sample_word >> ONETIME_CHANNEL_SHIFT) & ONETIME_CHANNEL_MASK;
        let packed = ((channel << DATA_CHANNEL_SHIFT) | channel_sample(channel)) & DATA_MASK;
        let mut raw = self.int_raw.get();
        if sample_word & SAR1_ONETIME_SAMPLE != 0 {
            self.sar1_data = packed;
            raw |= SAR1_DONE_INT;
        }
        if sample_word & SAR2_ONETIME_SAMPLE != 0 {
            self.sar2_data = packed;
            raw |= SAR2_DONE_INT;
        }
        // If neither unit-select bit is set the IDF still expects SAR1 (the
        // default one-shot unit) to complete.
        if sample_word & (SAR1_ONETIME_SAMPLE | SAR2_ONETIME_SAMPLE) == 0 {
            self.sar1_data = packed;
            raw |= SAR1_DONE_INT;
        }
        self.int_raw.set(raw);
    }
}

impl Peripheral for Esp32c3ApbSarAdc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_u32(offset & !3)?;
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let sh = (offset & 3) * 8;
        let cur = self.read_u32(aligned)?;
        self.write_u32(aligned, (cur & !(0xFFu32 << sh)) | ((value as u32) << sh))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset & !3 {
            SAR1DATA_STATUS => self.sar1_data,
            SAR2DATA_STATUS => self.sar2_data,
            INT_RAW => self.int_raw.get(),
            INT_ST => self.int_st(),
            INT_CLR => 0, // write-only
            o => self.reg(o),
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            ONETIME_SAMPLE => {
                // Store the config, then run a conversion on the START strobe.
                if let Some(slot) = self.regs.get_mut((ONETIME_SAMPLE / 4) as usize) {
                    *slot = value & !ONETIME_START; // START self-clears
                }
                if value & ONETIME_START != 0 {
                    self.convert(value);
                }
            }
            // W1C: clear latched raw bits where INT_CLR has a 1.
            INT_CLR => {
                self.int_raw.set(self.int_raw.get() & !(value & INT_MASK));
            }
            // R/WTC: writing 1s clears those latched bits.
            INT_RAW => {
                self.int_raw.set(self.int_raw.get() & !(value & INT_MASK));
            }
            INT_ST => {} // read-only
            // Data-status registers are read-only (latched by `convert`).
            SAR1DATA_STATUS | SAR2DATA_STATUS => {}
            o => {
                if let Some(slot) = self.regs.get_mut((o / 4) as usize) {
                    *slot = value;
                }
            }
        }
        Ok(())
    }

    /// LEGACY per-cycle walk path: re-assert the level interrupt source while
    /// any enabled INT bit is set. In scheduler mode ([`Self::uses_scheduler`]
    /// true) the walk skips this peripheral entirely and the bus re-derives the
    /// level from [`Self::matrix_irq_sources`] instead; this reporter is a pure
    /// no-op on state, so a stray call is harmless.
    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult {
            explicit_irqs: if self.int_st() != 0 {
                Some(vec![self.source_id])
            } else {
                None
            },
            ..PeripheralTickResult::default()
        }
    }

    fn legacy_tick_active(&self) -> bool {
        self.int_st() != 0
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    /// Walk-free plan: driven by the event scheduler once the bus has attached
    /// its cycle clock (production `push_peripheral`/`add_peripheral` always do,
    /// under the `event-scheduler` feature). The per-cycle walk then skips this
    /// peripheral; its `int_raw` is write-armed by `convert()` on the
    /// ONETIME_START strobe (no free-running counter), so there is nothing to
    /// advance and no event to schedule — only the level export via
    /// `matrix_irq_sources` is needed. Without a clock (feature off, a
    /// hand-built bus, or `force_legacy_walk`) it stays on the legacy walk so
    /// those callers keep the old exact semantics.
    fn uses_scheduler(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    /// C3 interrupt-matrix level: the `APB_SARADC` source while any enabled INT
    /// bit is set — the exact condition `tick` pushes on the legacy walk. In
    /// scheduler mode the walk no longer re-emits it, so the bus re-derives the
    /// level from here (`refresh_esp32c3_sched_sources`, polled on the event
    /// path and the walk-tick aggregation) so the level-sensitive IRQ stays
    /// routed and de-asserts the tick after firmware writes INT_CLR.
    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.int_st() != 0 {
            out.push(self.source_id);
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode an ONETIME_SAMPLE word: SAR1-selected one-shot of `channel`.
    fn sar1_oneshot(channel: u32) -> u32 {
        SAR1_ONETIME_SAMPLE
            | ONETIME_START
            | ((channel & ONETIME_CHANNEL_MASK) << ONETIME_CHANNEL_SHIFT)
    }

    #[test]
    fn done_not_set_before_conversion() {
        let a = Esp32c3ApbSarAdc::new(APB_SARADC_INTR_SOURCE_ID);
        assert_eq!(a.read_u32(INT_RAW).unwrap() & SAR1_DONE_INT, 0);
    }

    #[test]
    fn oneshot_sets_done_and_self_clears_start() {
        let mut a = Esp32c3ApbSarAdc::new(APB_SARADC_INTR_SOURCE_ID);
        a.write_u32(ONETIME_SAMPLE, sar1_oneshot(3)).unwrap();
        assert_ne!(
            a.read_u32(INT_RAW).unwrap() & SAR1_DONE_INT,
            0,
            "SAR1 DONE raised"
        );
        assert_eq!(
            a.read_u32(ONETIME_SAMPLE).unwrap() & ONETIME_START,
            0,
            "ONETIME_START self-clears"
        );
    }

    #[test]
    fn result_tracks_selected_channel() {
        // The headline truthfulness property: distinct channels give distinct,
        // predictable codes — proof the conversion reflects the selected input.
        let mut a = Esp32c3ApbSarAdc::new(APB_SARADC_INTR_SOURCE_ID);
        a.write_u32(ONETIME_SAMPLE, sar1_oneshot(3)).unwrap();
        let d3 = a.read_u32(SAR1DATA_STATUS).unwrap();
        assert_eq!(d3 & 0x0FFF, channel_sample(3), "ch3 12-bit sample");
        assert_eq!((d3 >> DATA_CHANNEL_SHIFT) & 0xF, 3, "ch3 packed channel");

        a.write_u32(INT_CLR, SAR1_DONE_INT).unwrap();
        a.write_u32(ONETIME_SAMPLE, sar1_oneshot(5)).unwrap();
        let d5 = a.read_u32(SAR1DATA_STATUS).unwrap();
        assert_eq!(d5 & 0x0FFF, channel_sample(5), "ch5 12-bit sample");
        assert_ne!(d3, d5, "different channels yield different results");
    }

    #[test]
    fn int_clr_is_w1c() {
        let mut a = Esp32c3ApbSarAdc::new(APB_SARADC_INTR_SOURCE_ID);
        a.write_u32(ONETIME_SAMPLE, sar1_oneshot(1)).unwrap();
        assert_ne!(a.read_u32(INT_RAW).unwrap() & SAR1_DONE_INT, 0);
        a.write_u32(INT_CLR, SAR1_DONE_INT).unwrap();
        assert_eq!(
            a.read_u32(INT_RAW).unwrap() & SAR1_DONE_INT,
            0,
            "W1C clears DONE"
        );
    }

    #[test]
    fn int_st_masks_with_ena_and_emits_source() {
        let mut a = Esp32c3ApbSarAdc::new(APB_SARADC_INTR_SOURCE_ID);
        assert!(
            !a.legacy_tick_active(),
            "idle level-IRQ SARADC must stay out of the legacy tick walk"
        );
        assert!(
            a.legacy_tick_dynamic(),
            "writes that assert/clear INT_ST must refresh tick membership"
        );
        a.write_u32(ONETIME_SAMPLE, sar1_oneshot(2)).unwrap();
        assert_eq!(a.read_u32(INT_ST).unwrap(), 0, "ST gated by ENA");
        assert_eq!(a.tick().explicit_irqs, None);
        a.write_u32(INT_ENA, SAR1_DONE_INT).unwrap();
        assert_eq!(a.read_u32(INT_ST).unwrap() & SAR1_DONE_INT, SAR1_DONE_INT);
        assert!(a.legacy_tick_active(), "asserted INT_ST needs level ticks");
        assert_eq!(
            a.tick().explicit_irqs,
            Some(vec![APB_SARADC_INTR_SOURCE_ID])
        );
        a.write_u32(INT_CLR, SAR1_DONE_INT).unwrap();
        assert!(
            !a.legacy_tick_active(),
            "cleared INT_ST can leave tick walk"
        );
    }

    #[test]
    fn other_registers_are_register_backed() {
        let mut a = Esp32c3ApbSarAdc::new(APB_SARADC_INTR_SOURCE_ID);
        a.write_u32(0x00, 0xDEAD_0000).unwrap(); // CTRL
        assert_eq!(a.read_u32(0x00).unwrap(), 0xDEAD_0000);
    }
}
