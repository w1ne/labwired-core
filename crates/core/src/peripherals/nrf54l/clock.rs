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
    /// INTEN/INTENSET enabled a source whose event is already latched. The
    /// line is a level (INTEN & EVENTS), so it must rise without a new event.
    pending_irq_eval: bool,

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

    /// Attached at bus assembly on an event-scheduler build. Its presence is
    /// what [`Self::scheduler_mode`] reads: it proves the bus actually wired
    /// the scheduler, so a hand-built bus stays on the legacy walk instead of
    /// arming events nothing will ever deliver.
    #[serde(skip)]
    clock: Option<crate::cycle_clock::CycleClock>,

    /// One-shot events armed by a TASKS write, waiting for
    /// `take_scheduled_events` to hand them to the scheduler. A peripheral
    /// cannot reach the scheduler from `write`, so this is the bootstrap path.
    #[serde(skip)]
    armed: Vec<(u64, u32)>,
}

/// Event tokens. One per deferred STARTED/DONE latch, so each settles
/// independently exactly as the walk settled them independently.
const EV_XOSTARTED: u32 = 0;
const EV_PLLSTARTED: u32 = 1;
const EV_LFCLKSTARTED: u32 = 2;
const EV_DONE: u32 = 3;
/// No event latch: re-evaluate the IRQ line after an INTEN/INTENSET write.
const EV_IRQ_EVAL: u32 = 4;

/// Relative delay handed to the bus. `collect_scheduled_events` stores
/// `current_cycle + 1 + delay`, and the walk settles on the next `tick()`,
/// so this must be 0. A 1 here is one instruction late even at interval 1.
const SETTLE_DELAY: u64 = 0;

impl Nrf54lClock {
    pub fn new() -> Self {
        Self::default()
    }

    crate::cycle_clock::scheduler_mode!();

    /// Arm a deferred STARTED/DONE latch.
    ///
    /// The `pending_*` flag is set in BOTH modes, and the scheduler event is
    /// queued on top of it. That looks redundant and is not:
    ///
    /// `take_scheduled_events()` drains `armed`, and the bus calls it after
    /// EVERY MMIO write — including on a bare `SystemBus` with no `Machine`
    /// behind it, where the harvested events land in `pending_schedule` and
    /// are never drained by anyone. A bare-CPU oracle harness
    /// (`tick_peripherals_fully_forced`) would then watch HFCLKSTARTED never
    /// arrive, which is exactly what Zephyr's clock_control spins on. Keeping
    /// the legacy flag means the forced walk settles the event through the
    /// path it always used, with no special case.
    ///
    /// On a real `Machine` the scheduler wins the race and `on_event` clears
    /// the flag as it settles, so the two representations never disagree and
    /// the event is latched exactly once.
    fn arm(&mut self, token: u32, pending: fn(&mut Self) -> &mut bool) {
        *pending(self) = true;
        if self.scheduler_mode() {
            self.armed.push((SETTLE_DELAY, token));
        }
    }

    /// Clear the legacy twin of an event the scheduler just settled, so
    /// `legacy_tick_active` stops reporting work and `tick()` cannot latch it
    /// a second time.
    fn clear_pending(&mut self, token: u32) {
        match token {
            EV_XOSTARTED => self.pending_xostarted = false,
            EV_PLLSTARTED => self.pending_pllstarted = false,
            EV_LFCLKSTARTED => self.pending_lfclkstarted = false,
            EV_DONE => self.pending_done = false,
            EV_IRQ_EVAL => self.pending_irq_eval = false,
            _ => {}
        }
    }

    /// Latch one settled event and report whether it should raise the line.
    /// The IRQ reflects the WHOLE event bitmap, not just the event that fired
    /// — the walk computed it that way and a driver that enabled two sources
    /// must not see one of them swallowed.
    fn settle(&mut self, token: u32) -> bool {
        match token {
            EV_XOSTARTED => self.events_xostarted = 1,
            EV_PLLSTARTED => self.events_pllstarted = 1,
            EV_LFCLKSTARTED => self.events_lfclkstarted = 1,
            EV_DONE => self.events_done = 1,
            EV_IRQ_EVAL => {}
            _ => return false,
        }
        self.inten & self.event_bitmap() != 0
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
            _ => {
                crate::census_reg!("nrf54l.clock:Nrf54lClock", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_XOSTART if value & 1 != 0 => {
                self.xo_run = RUN_TRIGGERED;
                self.xo_stat = STAT_STATE;
                self.arm(EV_XOSTARTED, |c| &mut c.pending_xostarted);
            }
            OFF_TASKS_XOSTOP if value & 1 != 0 => {
                self.xo_run = 0;
                self.xo_stat = 0;
            }
            OFF_TASKS_PLLSTART if value & 1 != 0 => {
                self.pll_run = RUN_TRIGGERED;
                self.pll_stat = STAT_STATE;
                self.arm(EV_PLLSTARTED, |c| &mut c.pending_pllstarted);
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
                self.arm(EV_LFCLKSTARTED, |c| &mut c.pending_lfclkstarted);
            }
            OFF_TASKS_LFCLKSTOP if value & 1 != 0 => {
                self.lfclk_run = 0;
                self.lfclk_stat = 0;
            }
            OFF_TASKS_CAL if value & 1 != 0 => {
                self.arm(EV_DONE, |c| &mut c.pending_done);
            }
            // Tasks written with 0 are no-ops (level-triggered on non-zero).
            OFF_TASKS_XOSTART | OFF_TASKS_XOSTOP | OFF_TASKS_PLLSTART | OFF_TASKS_PLLSTOP
            | OFF_TASKS_LFCLKSTART | OFF_TASKS_LFCLKSTOP | OFF_TASKS_CAL => {}

            // EVENTS: hardware-generated. SW write-1 ignored, write-0 clears.
            OFF_EVENTS_XOSTARTED if value == 0 => self.events_xostarted = 0,
            OFF_EVENTS_PLLSTARTED if value == 0 => self.events_pllstarted = 0,
            OFF_EVENTS_LFCLKSTARTED if value == 0 => self.events_lfclkstarted = 0,
            OFF_EVENTS_DONE if value == 0 => self.events_done = 0,
            OFF_EVENTS_XOSTARTED
            | OFF_EVENTS_PLLSTARTED
            | OFF_EVENTS_LFCLKSTARTED
            | OFF_EVENTS_DONE => {}

            OFF_INTEN | OFF_INTENSET => {
                let previous = self.inten;
                self.inten = if offset == OFF_INTEN {
                    value
                } else {
                    previous | value
                };
                // nrfx `nrfx_clock_lfclk_start` triggers LFCLKSTART and only
                // then writes INTENSET. The event can already be latched, and
                // the level line must still reach the NVIC.
                if self.inten & !previous & self.event_bitmap() != 0 {
                    self.arm(EV_IRQ_EVAL, |c| &mut c.pending_irq_eval);
                }
            }
            OFF_INTENCLR => self.inten &= !value,

            OFF_LFCLK_SRC => self.lfclk_src = value & LFCLK_SRC_MASK,

            // STAT/RUN/SRCCOPY are read-only status; writes are ignored, as is
            // everything else in the window.
            _ => {
                crate::census_reg!("nrf54l.clock:Nrf54lClock", offset, "write");
            }
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
        self.pending_irq_eval = false;

        res.irq = self.inten & self.event_bitmap() != 0;
        res
    }

    /// Only in the per-cycle walk while a start is settling. Outside that
    /// window `tick()` has nothing to do, and a firmware write to a start task
    /// re-arms the entry via `refresh_legacy_tick_index()`.
    fn attach_cycle_clock(&mut self, clock: crate::cycle_clock::CycleClock) {
        self.clock = Some(clock);
    }

    fn uses_scheduler(&self) -> bool {
        // True once the bus attached its cycle clock on an event-scheduler
        // build. Without one (feature off, or a hand-built bus) stay on the
        // legacy walk with exact historical semantics.
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        // In scheduler mode the only thing the walk did here — convert an
        // armed `pending_*` into its `events_*` one tick later — rides a
        // scheduled event instead, so the walk is deletable.
        //
        // This matters beyond this peripheral: `derive_walk_deletable` is an
        // ALL over the bus, and while ANY peripheral answers true the whole
        // board is pinned to `max_safe_tick_interval() == 1`. On nrf54l15 that
        // clamp costs ~38x (2119.5 Ir/step against nrf52840's 54.7 on the same
        // fixture and ISA), because at a one-instruction window the Cortex-M
        // hot-loop fast path never engages. See `tick_interval_inventory`,
        // which names the remaining forcers: uart20, uart30, twi21, twi22.
        !self.scheduler_mode()
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        std::mem::take(&mut self.armed)
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let irq = self.settle(event_token);
        self.clear_pending(event_token);
        crate::sched::EventResult {
            // `raise_own_irq`, not `raise_irq`: this model does not know its
            // own NVIC line — the bus maps it from `PeripheralEntry::irq`,
            // exactly as it mapped the legacy `PeripheralTickResult::irq`.
            raise_own_irq: irq,
            ..Default::default()
        }
    }

    /// Also selects membership of the bare-CPU oracle's forced walk, which is
    /// why `arm()` sets `pending_*` in scheduler mode too — see its docs.
    fn legacy_tick_active(&self) -> bool {
        self.pending_xostarted
            || self.pending_pllstarted
            || self.pending_lfclkstarted
            || self.pending_done
            || self.pending_irq_eval
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

    /// nrfx `nrfx_clock_lfclk_start` order: LFCLKSTART first, INTENSET after.
    /// The event is latched before the source is enabled; the level line
    /// must still rise.
    #[test]
    fn intenset_after_a_latched_event_raises_the_irq() {
        let mut c = Nrf54lClock::new();
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();
        assert!(!c.tick().irq, "INTEN is clear");
        assert_eq!(c.read_u32(OFF_EVENTS_LFCLKSTARTED).unwrap(), 1);
        assert!(!c.legacy_tick_active(), "nothing left to settle");

        c.write_u32(OFF_INTENSET, INTEN_LFCLKSTARTED).unwrap();
        assert!(
            c.legacy_tick_active(),
            "the INTENSET write must schedule a line re-evaluation"
        );
        assert!(c.tick().irq, "INTEN & EVENTS is non-zero");
        assert!(!c.legacy_tick_active());

        // Enabling a source with no latched event schedules nothing.
        c.write_u32(OFF_INTENSET, INTEN_XOSTARTED).unwrap();
        assert!(!c.legacy_tick_active());
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

/// The scheduler path this model gained when it left the legacy walk.
///
/// ⚠️ These are NOT covered by the tests above. Every one of those builds the
/// clock with `Nrf54lClock::new()` and never attaches a `CycleClock`, so
/// `scheduler_mode()` is false and they all exercise the LEGACY path — which
/// is exactly what makes them a useful control, and exactly why they cannot
/// witness a regression on the new one.
#[cfg(all(test, feature = "event-scheduler"))]
mod scheduler_mode_tests {
    use super::*;
    use crate::cycle_clock::CycleClock;
    use crate::Peripheral;

    fn armed() -> Nrf54lClock {
        let mut c = Nrf54lClock::new();
        c.attach_cycle_clock(CycleClock::default());
        c
    }

    #[test]
    fn attaching_a_clock_leaves_the_walk() {
        let c = armed();
        assert!(c.uses_scheduler(), "clock attached → scheduler drives it");
        assert!(
            !c.needs_legacy_walk(),
            "and the per-cycle walk is deletable — which is the whole point: \
             `derive_walk_deletable` is an ALL, so one holdout pins the board \
             to max_safe_tick_interval() == 1"
        );
    }

    #[test]
    fn no_clock_stays_on_the_legacy_walk() {
        let c = Nrf54lClock::new();
        assert!(!c.uses_scheduler());
        assert!(
            c.needs_legacy_walk(),
            "a hand-built bus never wired the scheduler, so arming events \
             nothing will deliver would strand the STARTED latch forever"
        );
    }

    #[test]
    fn a_task_write_arms_a_scheduled_event_instead_of_a_pending_flag() {
        let mut c = armed();
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();

        // The walk flag IS set in scheduler mode, deliberately: it is what
        // lets a bare-bus oracle harness settle the event when no `Machine`
        // exists to drain `pending_schedule`. See `arm`. Settling twice is
        // prevented by `on_event` clearing it, not by never setting it --
        // and `settle` is idempotent anyway (it assigns 1, not +=1).
        assert!(
            c.pending_lfclkstarted,
            "the legacy twin must be armed too, so the forced walk can settle it"
        );
        let armed_events = c.take_scheduled_events();
        assert_eq!(
            armed_events,
            vec![(0, EV_LFCLKSTARTED)],
            "delay 0: the bus adds the extra cycle itself"
        );
        assert!(
            c.take_scheduled_events().is_empty(),
            "the buffer drains on read"
        );
    }

    /// The walk settles STARTED on the instruction after the task write.
    /// The bus stores `current_cycle + 1 + delay`, so a delay of 1 is the
    /// instruction after that — one cycle later than `tick()`, even at
    /// interval 1.
    #[test]
    fn lfclk_started_is_due_on_the_instruction_after_the_task_write() {
        // A local, not `const BASE`: the yaml-owned-base gate flags that
        // name, and this window only has to be somewhere the write lands.
        let base: u64 = 0x5010_E000;
        let mut bus = crate::bus::SystemBus::new();
        bus.add_peripheral(
            "clock",
            base,
            0x1000,
            Some(261),
            Box::new(Nrf54lClock::new()),
        );
        bus.current_cycle = 10;
        crate::Bus::write_u32(&mut bus, base + OFF_TASKS_LFCLKSTART, 1).unwrap();

        assert_eq!(bus.pending_schedule.len(), 1);
        let (_idx, deadline, _token) = bus.pending_schedule[0];
        assert_eq!(
            deadline, 11,
            "STARTED must be due on the next instruction, matching tick()"
        );
        assert_eq!(
            crate::Bus::read_u32(&bus, base + OFF_EVENTS_LFCLKSTARTED).unwrap(),
            0,
            "not latched in the same access as the task write"
        );
    }

    #[test]
    fn the_event_settles_on_delivery_and_raises_the_line_through_inten() {
        let mut c = armed();
        c.write_u32(OFF_INTEN, INTEN_LFCLKSTARTED).unwrap();
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();
        let token = c.take_scheduled_events()[0].1;

        assert_eq!(
            c.read_u32(OFF_EVENTS_LFCLKSTARTED).unwrap(),
            0,
            "not settled before the event is delivered — the deferred start is \
             the reason drivers spin rather than read once"
        );

        let mut sched = crate::sched::EventScheduler::new();
        let mut bus = crate::bus::SystemBus::new();
        let res = c.on_event(token, &mut sched, &mut bus);

        assert_eq!(c.read_u32(OFF_EVENTS_LFCLKSTARTED).unwrap(), 1);
        assert!(
            res.raise_own_irq,
            "INTEN enabled this source, so the bus must pend the line — \
             `raise_own_irq`, because this model does not know its NVIC number"
        );
    }

    /// nrfx `nrfx_clock_lfclk_start` order on the scheduler path: the event
    /// is delivered first, INTENSET arrives after it.
    #[test]
    fn intenset_after_a_delivered_event_schedules_a_line_evaluation() {
        let mut c = armed();
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();
        let token = c.take_scheduled_events()[0].1;
        let mut sched = crate::sched::EventScheduler::new();
        let mut bus = crate::bus::SystemBus::new();
        assert!(!c.on_event(token, &mut sched, &mut bus).raise_own_irq);

        c.write_u32(OFF_INTENSET, INTEN_LFCLKSTARTED).unwrap();
        assert_eq!(
            c.take_scheduled_events(),
            vec![(SETTLE_DELAY, EV_IRQ_EVAL)],
            "the INTENSET write must queue one line evaluation"
        );
        let res = c.on_event(EV_IRQ_EVAL, &mut sched, &mut bus);
        assert!(res.raise_own_irq, "INTEN & EVENTS is non-zero");
        assert!(!c.legacy_tick_active());

        // Writing the same enable again queues nothing.
        c.write_u32(OFF_INTENSET, INTEN_LFCLKSTARTED).unwrap();
        assert!(c.take_scheduled_events().is_empty());
    }

    #[test]
    fn the_line_stays_down_when_inten_does_not_enable_the_source() {
        let mut c = armed();
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();
        let token = c.take_scheduled_events()[0].1;

        let mut sched = crate::sched::EventScheduler::new();
        let mut bus = crate::bus::SystemBus::new();
        let res = c.on_event(token, &mut sched, &mut bus);

        assert_eq!(
            c.read_u32(OFF_EVENTS_LFCLKSTARTED).unwrap(),
            1,
            "the EVENT latches regardless — only the IRQ is gated"
        );
        assert!(!res.raise_own_irq);
    }

    /// The IRQ reflects the WHOLE event bitmap, not just the token that fired.
    /// The walk computed it that way; a driver with two sources enabled must
    /// not have one swallowed because the other settled first.
    #[test]
    fn a_second_source_still_raises_while_an_earlier_event_is_latched() {
        let mut c = armed();
        c.write_u32(OFF_INTEN, INTEN_LFCLKSTARTED | INTEN_XOSTARTED)
            .unwrap();
        c.write_u32(OFF_TASKS_LFCLKSTART, 1).unwrap();
        c.write_u32(OFF_TASKS_XOSTART, 1).unwrap();
        let tokens: Vec<u32> = c
            .take_scheduled_events()
            .into_iter()
            .map(|(_, t)| t)
            .collect();
        assert_eq!(tokens, vec![EV_LFCLKSTARTED, EV_XOSTARTED]);

        let mut sched = crate::sched::EventScheduler::new();
        let mut bus = crate::bus::SystemBus::new();
        for token in tokens {
            assert!(
                c.on_event(token, &mut sched, &mut bus).raise_own_irq,
                "both settlements raise — neither is swallowed"
            );
        }
    }
}
