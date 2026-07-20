// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF54L CLOCK — oscillator control, the nRF54L-generation layout.
//!
//! Source: Nordic MDK SVD `nrf54l15_application.svd`, peripheral
//! `GLOBAL_CLOCK_S` (base 0x5010_E000), with bit positions taken from
//! `nrf54l15_application_peripherals.h` (`CLOCK_LFCLK_STAT_STATE_Pos = 16`,
//! `CLOCK_LFCLK_STAT_SRC_Pos = 0`, `CLOCK_LFCLK_STAT_ALWAYSRUNNING_Pos = 4`,
//! `CLOCK_XO_STAT_STATE_Pos = 16`). Cross-checked against the instruction
//! stream of a real Zephyr build spinning in `lfclk_spinwait()`.
//!
//! **This is NOT the nRF52 CLOCK with a different base address**, despite the
//! devicetree binding both as `compatible = "nordic,nrf-clock"`. The nRF54L
//! family replaced the HFCLK/LFCLK task pair with an XO/PLL/LFCLK trio and
//! moved every status register:
//!
//! | function             | nRF52 | nRF54L |
//! |----------------------|-------|--------|
//! | start high-freq osc  | 0x000 `TASKS_HFCLKSTART` | 0x000 `TASKS_XOSTART` |
//! | start low-freq osc   | 0x008 | 0x010 (`TASKS_LFCLKSTART`) |
//! | LF started event     | 0x104 | 0x108 (`EVENTS_LFCLKSTARTED`) |
//! | LF status            | 0x418 `LFCLKSTAT` | 0x44C (`LFCLK.STAT`) |
//! | LF source select     | 0x518 `LFCLKSRC`  | 0x440 (`LFCLK.SRC`) |
//!
//! `XO.STAT` landing on 0x40C — the same offset nRF52 uses for `HFCLKSTAT` —
//! is a coincidence, and reusing the nRF52 model here got far enough to look
//! like it worked before hanging in the LF spin-wait.
//!
//! Zephyr's `lfclk_spinwait()` (drivers/clock_control/clock_control_nrf.c)
//! polls `LFCLK.STAT` and tests STATE (bit 16) together with SRC (bits 1:0),
//! so both fields must be populated on start or the kernel never gets its
//! tick source and the boot stops there.
//!
//! Not modelled: XO tuning (`TASKS_XOTUNE`, `EVENTS_XOTUNED`/`XOTUNEERROR`/
//! `XOTUNEFAILED`), calibration timing, DPPI SUBSCRIBE routing, and the real
//! oscillator start latency. Starts settle on the next peripheral tick, which
//! is the same approximation the nRF52 CLOCK model makes.

use crate::{PeripheralTickResult, SimResult};

// ── Tasks ────────────────────────────────────────────────────────────────
const OFF_TASKS_XOSTART: u64 = 0x000;
const OFF_TASKS_XOSTOP: u64 = 0x004;
const OFF_TASKS_PLLSTART: u64 = 0x008;
const OFF_TASKS_PLLSTOP: u64 = 0x00C;
const OFF_TASKS_LFCLKSTART: u64 = 0x010;
const OFF_TASKS_LFCLKSTOP: u64 = 0x014;
const OFF_TASKS_CAL: u64 = 0x018;

// ── Events ───────────────────────────────────────────────────────────────
const OFF_EVENTS_XOSTARTED: u64 = 0x100;
const OFF_EVENTS_PLLSTARTED: u64 = 0x104;
const OFF_EVENTS_LFCLKSTARTED: u64 = 0x108;
const OFF_EVENTS_DONE: u64 = 0x10C;

// ── Interrupts ───────────────────────────────────────────────────────────
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_INTPEND: u64 = 0x30C;

// ── Status / config ──────────────────────────────────────────────────────
const OFF_XO_RUN: u64 = 0x408;
const OFF_XO_STAT: u64 = 0x40C;
const OFF_PLL_RUN: u64 = 0x428;
const OFF_PLL_STAT: u64 = 0x42C;
const OFF_LFCLK_SRC: u64 = 0x440;
const OFF_LFCLK_RUN: u64 = 0x448;
const OFF_LFCLK_STAT: u64 = 0x44C;
const OFF_LFCLK_SRCCOPY: u64 = 0x450;

/// STATE bit in XO.STAT / PLL.STAT / LFCLK.STAT (MDK `*_STAT_STATE_Pos`).
const STAT_STATE: u32 = 1 << 16;
/// SRC field in LFCLK.STAT / LFCLK.SRC (`CLOCK_LFCLK_STAT_SRC_Pos = 0`).
const LFCLK_SRC_MASK: u32 = 0x3;
/// RUN.STATUS "triggered".
const RUN_TRIGGERED: u32 = 1;

/// INTEN bit positions, from the SVD field order.
const INTEN_XOSTARTED: u32 = 1 << 0;
const INTEN_PLLSTARTED: u32 = 1 << 1;
const INTEN_LFCLKSTARTED: u32 = 1 << 2;
const INTEN_DONE: u32 = 1 << 3;

#[derive(Debug, Default, serde::Serialize)]
pub struct Nrf54lClock {
    // Events
    events_xostarted: u32,
    events_pllstarted: u32,
    events_lfclkstarted: u32,
    events_done: u32,

    // Deferred start: the STAT register reflects the oscillator immediately,
    // but the STARTED event settles on the next tick — the same few-cycle
    // delay silicon has, and the reason drivers spin rather than read once.
    pending_xostarted: bool,
    pending_pllstarted: bool,
    pending_lfclkstarted: bool,
    pending_done: bool,

    // Status / config
    xo_run: u32,
    xo_stat: u32,
    pll_run: u32,
    pll_stat: u32,
    lfclk_src: u32,
    lfclk_run: u32,
    lfclk_stat: u32,
    lfclk_srccopy: u32,

    inten: u32,
}

impl Nrf54lClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bitmap of currently-latched events, in INTEN bit positions.
    fn event_bitmap(&self) -> u32 {
        let mut b = 0;
        if self.events_xostarted != 0 {
            b |= INTEN_XOSTARTED;
        }
        if self.events_pllstarted != 0 {
            b |= INTEN_PLLSTARTED;
        }
        if self.events_lfclkstarted != 0 {
            b |= INTEN_LFCLKSTARTED;
        }
        if self.events_done != 0 {
            b |= INTEN_DONE;
        }
        b
    }
}

impl crate::Peripheral for Nrf54lClock {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // 32-bit register file; byte reads are not used by any nRF driver.
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // Tasks read as 0.
            OFF_TASKS_XOSTART | OFF_TASKS_XOSTOP | OFF_TASKS_PLLSTART | OFF_TASKS_PLLSTOP
            | OFF_TASKS_LFCLKSTART | OFF_TASKS_LFCLKSTOP | OFF_TASKS_CAL => 0,

            OFF_EVENTS_XOSTARTED => self.events_xostarted,
            OFF_EVENTS_PLLSTARTED => self.events_pllstarted,
            OFF_EVENTS_LFCLKSTARTED => self.events_lfclkstarted,
            OFF_EVENTS_DONE => self.events_done,

            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_INTPEND => self.inten & self.event_bitmap(),

            OFF_XO_RUN => self.xo_run,
            OFF_XO_STAT => self.xo_stat,
            OFF_PLL_RUN => self.pll_run,
            OFF_PLL_STAT => self.pll_stat,
            OFF_LFCLK_SRC => self.lfclk_src,
            OFF_LFCLK_RUN => self.lfclk_run,
            OFF_LFCLK_STAT => self.lfclk_stat,
            OFF_LFCLK_SRCCOPY => self.lfclk_srccopy,

            // Everything else in the 4 KB window reads as zero rather than
            // faulting the bus (SUBSCRIBE/PUBLISH windows, XO tune block).
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_XOSTART if value & 1 != 0 => {
                self.xo_run = RUN_TRIGGERED;
                self.xo_stat = STAT_STATE;
                self.pending_xostarted = true;
            }
            OFF_TASKS_XOSTOP if value & 1 != 0 => {
                self.xo_run = 0;
                self.xo_stat = 0;
            }
            OFF_TASKS_PLLSTART if value & 1 != 0 => {
                self.pll_run = RUN_TRIGGERED;
                self.pll_stat = STAT_STATE;
                self.pending_pllstarted = true;
            }
            OFF_TASKS_PLLSTOP if value & 1 != 0 => {
                self.pll_run = 0;
                self.pll_stat = 0;
            }
            OFF_TASKS_LFCLKSTART if value & 1 != 0 => {
                // Zephyr's lfclk_spinwait() tests STATE *and* SRC together, so
                // the selected source has to be latched into STAT here (and
                // copied to SRCCOPY, which is what the driver reads back to
                // confirm which source actually started).
                let src = self.lfclk_src & LFCLK_SRC_MASK;
                self.lfclk_run = RUN_TRIGGERED;
                self.lfclk_stat = STAT_STATE | src;
                self.lfclk_srccopy = src;
                self.pending_lfclkstarted = true;
            }
            OFF_TASKS_LFCLKSTOP if value & 1 != 0 => {
                self.lfclk_run = 0;
                self.lfclk_stat = 0;
            }
            OFF_TASKS_CAL if value & 1 != 0 => {
                self.pending_done = true;
            }
            // Tasks written with 0 are no-ops (level-triggered on non-zero).
            OFF_TASKS_XOSTART | OFF_TASKS_XOSTOP | OFF_TASKS_PLLSTART | OFF_TASKS_PLLSTOP
            | OFF_TASKS_LFCLKSTART | OFF_TASKS_LFCLKSTOP | OFF_TASKS_CAL => {}

            // EVENTS: hardware-generated. SW write-1 ignored, write-0 clears.
            OFF_EVENTS_XOSTARTED if value == 0 => self.events_xostarted = 0,
            OFF_EVENTS_PLLSTARTED if value == 0 => self.events_pllstarted = 0,
            OFF_EVENTS_LFCLKSTARTED if value == 0 => self.events_lfclkstarted = 0,
            OFF_EVENTS_DONE if value == 0 => self.events_done = 0,
            OFF_EVENTS_XOSTARTED | OFF_EVENTS_PLLSTARTED | OFF_EVENTS_LFCLKSTARTED
            | OFF_EVENTS_DONE => {}

            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,

            OFF_LFCLK_SRC => self.lfclk_src = value & LFCLK_SRC_MASK,

            // STAT/RUN/SRCCOPY are read-only status; writes are ignored, as is
            // everything else in the window.
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut res = PeripheralTickResult {
            cycles: 1,
            ..Default::default()
        };

        if self.pending_xostarted {
            self.pending_xostarted = false;
            self.events_xostarted = 1;
        }
        if self.pending_pllstarted {
            self.pending_pllstarted = false;
            self.events_pllstarted = 1;
        }
        if self.pending_lfclkstarted {
            self.pending_lfclkstarted = false;
            self.events_lfclkstarted = 1;
        }
        if self.pending_done {
            self.pending_done = false;
            self.events_done = 1;
        }

        res.irq = self.inten & self.event_bitmap() != 0;
        res
    }

    /// Only in the per-cycle walk while a start is settling. Outside that
    /// window `tick()` has nothing to do, and a firmware write to a start task
    /// re-arms the entry via `refresh_legacy_tick_index()`.
    fn legacy_tick_active(&self) -> bool {
        self.pending_xostarted
            || self.pending_pllstarted
            || self.pending_lfclkstarted
            || self.pending_done
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
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
    use crate::Peripheral;

    fn clock() -> Nrf54lClock {
        Nrf54lClock::new()
    }

    #[test]
    fn lfclk_start_latches_state_and_source_into_stat() {
        let mut c = clock();
        // Select source 1 (XTAL), then start.
        c.write_u32(OFF_LFCLK_SRC, 1).unwrap();
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();

        let stat = c.read_u32(OFF_LFCLK_STAT).unwrap();
        assert_ne!(stat & STAT_STATE, 0, "STATE (bit 16) must report running");
        assert_eq!(stat & LFCLK_SRC_MASK, 1, "SRC must be latched into STAT");
        assert_eq!(
            c.read_u32(OFF_LFCLK_SRCCOPY).unwrap(),
            1,
            "SRCCOPY must reflect the source that started"
        );
        assert_eq!(c.read_u32(OFF_LFCLK_RUN).unwrap(), RUN_TRIGGERED);
    }

    /// The exact condition Zephyr's `lfclk_spinwait()` tests.
    #[test]
    fn lfclk_started_event_settles_on_tick() {
        let mut c = clock();
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();
        assert_eq!(
            c.read_u32(OFF_EVENTS_LFCLKSTARTED).unwrap(),
            0,
            "event must not be set in the same access as the task write"
        );

        c.tick();
        assert_eq!(c.read_u32(OFF_EVENTS_LFCLKSTARTED).unwrap(), 1);
    }

    #[test]
    fn xo_start_reports_running_and_raises_event() {
        let mut c = clock();
        assert_eq!(c.read_u32(OFF_XO_STAT).unwrap(), 0);

        c.write_u32(OFF_TASKS_XOSTART, 1).unwrap();
        assert_ne!(c.read_u32(OFF_XO_STAT).unwrap() & STAT_STATE, 0);

        c.tick();
        assert_eq!(c.read_u32(OFF_EVENTS_XOSTARTED).unwrap(), 1);
    }

    #[test]
    fn stop_clears_status() {
        let mut c = clock();
        c.write_u32(OFF_TASKS_XOSTART, 1).unwrap();
        c.write_u32(OFF_TASKS_XOSTOP, 1).unwrap();
        assert_eq!(c.read_u32(OFF_XO_STAT).unwrap(), 0);

        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();
        c.write_u32(OFF_TASKS_LFCLKSTOP, 1).unwrap();
        assert_eq!(c.read_u32(OFF_LFCLK_STAT).unwrap(), 0);
    }

    #[test]
    fn events_are_write_zero_to_clear_and_write_one_ignored() {
        let mut c = clock();
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();
        c.tick();
        assert_eq!(c.read_u32(OFF_EVENTS_LFCLKSTARTED).unwrap(), 1);

        // Write 1: ignored (hardware owns the set).
        c.write_u32(OFF_EVENTS_XOSTARTED, 1).unwrap();
        assert_eq!(c.read_u32(OFF_EVENTS_XOSTARTED).unwrap(), 0);

        // Write 0: clears.
        c.write_u32(OFF_EVENTS_LFCLKSTARTED, 0).unwrap();
        assert_eq!(c.read_u32(OFF_EVENTS_LFCLKSTARTED).unwrap(), 0);
    }

    #[test]
    fn irq_is_gated_by_inten() {
        let mut c = clock();
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();
        assert!(!c.tick().irq, "IRQ must not fire while INTEN is clear");

        let mut c = clock();
        c.write_u32(OFF_INTENSET, INTEN_LFCLKSTARTED).unwrap();
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();
        assert!(c.tick().irq, "IRQ must fire once enabled");
        assert_ne!(c.read_u32(OFF_INTPEND).unwrap() & INTEN_LFCLKSTARTED, 0);
    }

    #[test]
    fn unmapped_offsets_read_zero_and_do_not_panic() {
        let mut c = clock();
        assert_eq!(c.read_u32(0xFFC).unwrap(), 0);
        c.write_u32(0xFFC, 0xDEAD_BEEF).unwrap();
        assert_eq!(c.read_u32(0xFFC).unwrap(), 0);
    }

    #[test]
    fn lfclk_src_is_masked_to_two_bits() {
        let mut c = clock();
        c.write_u32(OFF_LFCLK_SRC, 0xFFFF_FFFF).unwrap();
        assert_eq!(c.read_u32(OFF_LFCLK_SRC).unwrap(), LFCLK_SRC_MASK);
    }
}
