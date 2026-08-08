// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 WATCHDOG + tick generator (datasheet §4.7, base `0x40058000`).
//!
//! Twelve registers: `CTRL`, `LOAD`, `REASON`, `SCRATCH0..7`, `TICK`. Offsets,
//! field positions and reset values are taken from the vendored SVD
//! (`tests/fixtures/real_world/rp2040.svd`): `CTRL` resets to `0x0700_0000`
//! (all three PAUSE bits set) and `TICK` to `0x0000_0200` (`ENABLE` set,
//! `CYCLES` zero).
//!
//! ## What silicon does, and what this models
//!
//! * **The ×2 decrement.** This is the block's defining quirk. Errata RP2040-E1:
//!   the watchdog counter is decremented **twice** per tick, so a `LOAD` value
//!   is in units of 0.5 µs and the effective timeout is half the naive figure.
//!   It is why `pico-sdk`'s `watchdog_enable()` computes `delay_ms * 1000 * 2`.
//!   Firmware that assumes 1 µs per count is wrong on hardware, so it must be
//!   wrong here too — `CTRL.TIME` really does step down in twos.
//! * **The tick generator.** `TICK.CYCLES` divides the reference clock down to
//!   the watchdog/timer tick. `TICK.RUNNING` reads set only when `ENABLE` is
//!   set *and* `CYCLES` is non-zero — which is exactly why the reset state
//!   (`ENABLE=1, CYCLES=0`) reads back `0x200` and not `0x600`. Nothing counts
//!   until firmware calls the equivalent of `watchdog_start_tick()`.
//!   `TICK.COUNT` is the live down-counter within the current tick period, so
//!   it is a real divider, not a stored constant.
//! * **`LOAD` reloads the counter** whether or not the watchdog is enabled —
//!   `pico-sdk` writes `LOAD` before setting `CTRL.ENABLE`, and again on every
//!   `watchdog_update()` feed.
//! * **`REASON` latches** `TIMER` on a countdown expiry and `FORCE` on a
//!   `CTRL.TRIGGER` write, and is read-only.
//! * **`SCRATCH0..7`** are plain 32-bit storage that survives a watchdog reset
//!   on silicon; the bootrom reboot path (`SCRATCH4..7`) depends on that, and
//!   round-tripping them is the whole of their behaviour.
//!
//! ## Deliberately not modelled
//!
//! * **The reset itself.** An expired watchdog resets the chip on silicon. This
//!   model latches `REASON.TIMER`, stops the counter and leaves the CPU alone —
//!   the same convention the Nordic WDT model uses
//!   ([`crate::peripherals::nrf52::wdt`]): surface the timeout signal so a test
//!   can observe it, rather than restarting a simulation that would then never
//!   terminate. Once bitten the dog stays down; a later `LOAD` reloads the
//!   counter but `REASON` keeps its latched bit, because on silicon that bit is
//!   what survives to tell the next boot why it happened.
//! * **`PAUSE_JTAG` / `PAUSE_DBG0` / `PAUSE_DBG1`.** Stored and read back, but
//!   they gate the countdown on debugger/core-halt state and there is no
//!   debugger attached in simulation, so the countdown is never paused. That is
//!   the honest reading of "no debugger present", not a shortcut.
//! * **A common time base with [`crate::peripherals::rp2040::timer`].** On
//!   silicon both blocks are driven by *this* tick generator, so `CYCLES`
//!   simultaneously sets the timer's microsecond and the watchdog's half-
//!   microsecond. The TIMER model derives its microsecond straight from the
//!   peripheral tick instead (its own header says the absolute rate is
//!   arbitrary — the simulator has no wall clock), so with the usual
//!   `CYCLES = 12` the two blocks are 12× apart in sim time. Neither block can
//!   fix this alone; it is recorded here rather than papered over by ignoring
//!   `CYCLES`.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

// Register offsets (relative to the WATCHDOG base) — SVD-verified.
const CTRL: u64 = 0x00;
const LOAD: u64 = 0x04;
const REASON: u64 = 0x08;
const SCRATCH0: u64 = 0x0C;
const SCRATCH7: u64 = 0x28;
const TICK: u64 = 0x2C;

// CTRL fields.
const CTRL_TIME_MASK: u32 = 0x00FF_FFFF; // TIME[23:0], read-only
const CTRL_PAUSE_MASK: u32 = 0x0700_0000; // PAUSE_JTAG[24] | PAUSE_DBG0[25] | PAUSE_DBG1[26]
const CTRL_ENABLE: u32 = 1 << 30;
const CTRL_TRIGGER: u32 = 1 << 31; // write-only

// REASON fields.
const REASON_TIMER: u32 = 1 << 0;
const REASON_FORCE: u32 = 1 << 1;

// TICK fields.
const TICK_CYCLES_MASK: u32 = 0x1FF; // CYCLES[8:0]
const TICK_ENABLE: u32 = 1 << 9;
const TICK_RUNNING: u32 = 1 << 10;
const TICK_COUNT_SHIFT: u32 = 11; // COUNT[19:11]

/// Errata RP2040-E1: the counter decrements twice per tick.
const DECREMENT_PER_TICK: u32 = 2;

#[derive(Debug)]
pub struct Rp2040Watchdog {
    /// Live 24-bit down-counter, read through `CTRL.TIME`.
    counter: u32,
    /// `CTRL.ENABLE`.
    enabled: bool,
    /// `CTRL.PAUSE_*` — stored, never consulted (no debugger in simulation).
    pause: u32,
    /// `REASON`, latched and read-only.
    reason: u32,
    scratch: [u32; 8],
    /// `TICK.CYCLES` — reference-clock cycles per generated tick.
    tick_cycles: u32,
    /// `TICK.ENABLE`.
    tick_enable: bool,
    /// `TICK.COUNT` — live down-counter inside the current tick period.
    tick_count: u32,
    /// The dog has bitten: the countdown stops until firmware reloads. Silicon
    /// would have reset the chip here; see the module header.
    bitten: bool,
    /// Bus-published cycle clock (event-scheduler). When present the model is
    /// walk-independent: countdown rides scheduled events.
    clock: Option<CycleClock>,
    /// Bumped each arm so stale countdown events die on arrival.
    arm_seq: u32,
}

impl Default for Rp2040Watchdog {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040Watchdog {
    pub fn new() -> Self {
        Self {
            counter: 0,
            enabled: false,
            // CTRL reset value 0x0700_0000: all three PAUSE bits set.
            pause: CTRL_PAUSE_MASK,
            reason: 0,
            scratch: [0; 8],
            // TICK reset value 0x0000_0200: ENABLE set, CYCLES zero.
            tick_cycles: 0,
            tick_enable: true,
            tick_count: 0,
            bitten: false,
            clock: None,
            arm_seq: 0,
        }
    }

    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// True while the generator + dog can still change observable state.
    fn countdown_active(&self) -> bool {
        self.tick_running() && self.enabled && !self.bitten
    }

    /// One reference-clock edge of work (legacy `tick()` body).
    fn step_one(&mut self) {
        if !self.tick_generator_fires() {
            return;
        }
        if !self.enabled || self.bitten {
            return;
        }
        // Errata RP2040-E1: two counts per tick, so an odd LOAD lands on 0
        // rather than stepping past it.
        self.counter = self.counter.saturating_sub(DECREMENT_PER_TICK);
        if self.counter == 0 {
            self.bitten = true;
            self.reason |= REASON_TIMER;
        }
    }

    /// `TICK.RUNNING` — the generator only produces ticks with a non-zero
    /// divide ratio, which is what makes the reset state (`ENABLE=1`,
    /// `CYCLES=0`) read back `0x200` rather than `0x600`.
    fn tick_running(&self) -> bool {
        self.tick_enable && self.tick_cycles != 0
    }

    /// Advance the tick generator by one reference-clock cycle. Returns `true`
    /// on the cycle that produces a watchdog tick.
    fn tick_generator_fires(&mut self) -> bool {
        if !self.tick_running() {
            return false;
        }
        if self.tick_count == 0 {
            self.tick_count = self.tick_cycles - 1;
            true
        } else {
            self.tick_count -= 1;
            false
        }
    }
}

impl Peripheral for Rp2040Watchdog {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // TRIGGER is write-only and never reads back set.
            CTRL => {
                self.pause
                    | if self.enabled { CTRL_ENABLE } else { 0 }
                    | (self.counter & CTRL_TIME_MASK)
            }
            LOAD => 0, // write-only
            REASON => self.reason,
            SCRATCH0..=SCRATCH7 => self.scratch[((offset - SCRATCH0) / 4) as usize],
            TICK => {
                (self.tick_count << TICK_COUNT_SHIFT)
                    | if self.tick_running() { TICK_RUNNING } else { 0 }
                    | if self.tick_enable { TICK_ENABLE } else { 0 }
                    | (self.tick_cycles & TICK_CYCLES_MASK)
            }
            _ => {
                crate::census_reg!("rp2040.watchdog:Rp2040Watchdog", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            CTRL => {
                self.pause = value & CTRL_PAUSE_MASK;
                self.enabled = value & CTRL_ENABLE != 0;
                // TRIGGER forces an immediate watchdog reset on silicon. Here
                // it latches REASON.FORCE and stops the dog; the CPU keeps
                // running (see the module header).
                if value & CTRL_TRIGGER != 0 {
                    self.counter = 0;
                    self.reason |= REASON_FORCE;
                    self.bitten = true;
                }
            }
            // Write-only, 24-bit. Reloads the counter whether or not the
            // watchdog is enabled, and re-arms a dog that has already bitten.
            LOAD => {
                self.counter = value & CTRL_TIME_MASK;
                self.bitten = false;
            }
            // REASON is read-only.
            REASON => {}
            SCRATCH0..=SCRATCH7 => self.scratch[((offset - SCRATCH0) / 4) as usize] = value,
            TICK => {
                // Reprogramming the generator restarts the current period.
                self.tick_cycles = value & TICK_CYCLES_MASK;
                self.tick_enable = value & TICK_ENABLE != 0;
                self.tick_count = 0;
            }
            _ => {
                crate::census_reg!("rp2040.watchdog:Rp2040Watchdog", offset, "write");
            }
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        let shift = (offset & 0x3) * 8;
        let cur = self.read_u32(aligned)?;
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Legacy / feature-off path. Scheduler mode skips the walk.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.step_one();
        // The RP2040 watchdog has no interrupt line — there is no
        // WATCHDOG_IRQ in the SVD's interrupt table. Expiry is observable
        // through REASON only.
        PeripheralTickResult::default()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() || !self.countdown_active() {
            return Vec::new();
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        // delay-0 → next cycle, matching one legacy walk tick.
        vec![(0, self.arm_seq)]
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != self.arm_seq {
            return crate::sched::EventResult::default();
        }
        self.step_one();
        crate::sched::EventResult {
            reschedule_delay: self.countdown_active().then_some(1),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Start the tick generator with a 1:1 divide so one peripheral tick is one
    /// watchdog tick, then load `load` counts.
    fn armed(load: u32) -> Rp2040Watchdog {
        let mut w = Rp2040Watchdog::new();
        w.write_u32(TICK, 1 | TICK_ENABLE).unwrap();
        w.write_u32(LOAD, load).unwrap();
        w.write_u32(CTRL, CTRL_PAUSE_MASK | CTRL_ENABLE).unwrap();
        w
    }

    /// Reset values must match the SVD exactly: CTRL 0x0700_0000, TICK 0x200.
    #[test]
    fn reset_values_match_the_svd() {
        let w = Rp2040Watchdog::new();
        assert_eq!(w.read_u32(CTRL).unwrap(), 0x0700_0000);
        assert_eq!(w.read_u32(TICK).unwrap(), 0x0000_0200);
        assert_eq!(w.read_u32(REASON).unwrap(), 0);
    }

    /// The tick generator does not run with CYCLES = 0, even though ENABLE is
    /// set out of reset — hence TICK reading 0x200 and not 0x600.
    #[test]
    fn tick_generator_needs_a_nonzero_divide_ratio() {
        let mut w = Rp2040Watchdog::new();
        w.write_u32(LOAD, 100).unwrap();
        w.write_u32(CTRL, CTRL_ENABLE).unwrap();
        for _ in 0..50 {
            w.tick();
        }
        assert_eq!(
            w.read_u32(CTRL).unwrap() & CTRL_TIME_MASK,
            100,
            "no ticks generated, so no countdown"
        );
        assert_eq!(w.read_u32(TICK).unwrap() & TICK_RUNNING, 0);

        // Programming a ratio starts it.
        w.write_u32(TICK, 4 | TICK_ENABLE).unwrap();
        assert_ne!(w.read_u32(TICK).unwrap() & TICK_RUNNING, 0);
    }

    /// Errata RP2040-E1: two counts per tick, so a LOAD is in half-microseconds.
    #[test]
    fn counter_decrements_twice_per_tick() {
        let mut w = armed(100);
        for _ in 0..10 {
            w.tick();
        }
        assert_eq!(w.read_u32(CTRL).unwrap() & CTRL_TIME_MASK, 80);
    }

    /// The tick generator really divides: CYCLES = 4 costs four peripheral
    /// ticks per watchdog tick (so 8 counts per 4 ticks, not 8 per tick).
    #[test]
    fn tick_cycles_divides_the_countdown() {
        let mut w = Rp2040Watchdog::new();
        w.write_u32(TICK, 4 | TICK_ENABLE).unwrap();
        w.write_u32(LOAD, 100).unwrap();
        w.write_u32(CTRL, CTRL_ENABLE).unwrap();
        for _ in 0..20 {
            w.tick();
        }
        assert_eq!(
            w.read_u32(CTRL).unwrap() & CTRL_TIME_MASK,
            90,
            "20 ticks / 4 = 5 watchdog ticks x 2 counts"
        );
    }

    /// A disabled watchdog holds its counter even while the tick generator runs.
    #[test]
    fn disabled_watchdog_does_not_count() {
        let mut w = Rp2040Watchdog::new();
        w.write_u32(TICK, 1 | TICK_ENABLE).unwrap();
        w.write_u32(LOAD, 50).unwrap();
        for _ in 0..100 {
            w.tick();
        }
        assert_eq!(w.read_u32(CTRL).unwrap() & CTRL_TIME_MASK, 50);
    }

    /// Expiry latches REASON.TIMER and stops the dog — it does not reset the CPU.
    #[test]
    fn expiry_latches_reason_timer_and_stops() {
        let mut w = armed(6);
        for _ in 0..3 {
            w.tick();
        }
        assert_eq!(w.read_u32(CTRL).unwrap() & CTRL_TIME_MASK, 0);
        assert_eq!(w.read_u32(REASON).unwrap(), REASON_TIMER);
        // Further ticks change nothing; the counter stays at the floor.
        for _ in 0..20 {
            w.tick();
        }
        assert_eq!(w.read_u32(CTRL).unwrap() & CTRL_TIME_MASK, 0);
    }

    /// An odd LOAD lands on zero rather than wrapping past it.
    #[test]
    fn odd_load_saturates_at_zero() {
        let mut w = armed(5);
        for _ in 0..3 {
            w.tick();
        }
        assert_eq!(w.read_u32(CTRL).unwrap() & CTRL_TIME_MASK, 0);
        assert_eq!(w.read_u32(REASON).unwrap() & REASON_TIMER, REASON_TIMER);
    }

    /// Feeding the dog (a LOAD write) postpones the bite indefinitely.
    #[test]
    fn feeding_prevents_the_bite() {
        let mut w = armed(10);
        for _ in 0..20 {
            w.tick();
            w.write_u32(LOAD, 10).unwrap();
        }
        assert_eq!(w.read_u32(REASON).unwrap(), 0, "a fed dog never bites");
        assert_eq!(w.read_u32(CTRL).unwrap() & CTRL_TIME_MASK, 10);
    }

    /// CTRL.TRIGGER latches REASON.FORCE, is write-only, and stops the counter.
    #[test]
    fn trigger_forces_a_bite_and_never_reads_back() {
        let mut w = armed(1000);
        w.write_u32(CTRL, CTRL_TRIGGER | CTRL_ENABLE).unwrap();
        assert_eq!(w.read_u32(REASON).unwrap(), REASON_FORCE);
        assert_eq!(w.read_u32(CTRL).unwrap() & CTRL_TRIGGER, 0);
        assert_eq!(w.read_u32(CTRL).unwrap() & CTRL_TIME_MASK, 0);
    }

    /// REASON is read-only: firmware cannot clear the record of why it died.
    #[test]
    fn reason_is_read_only() {
        let mut w = armed(2);
        w.tick();
        assert_eq!(w.read_u32(REASON).unwrap(), REASON_TIMER);
        w.write_u32(REASON, 0xFFFF_FFFF).unwrap();
        assert_eq!(w.read_u32(REASON).unwrap(), REASON_TIMER);
    }

    /// All eight scratch words are independent 32-bit storage (the bootrom
    /// reboot path uses SCRATCH4..7).
    #[test]
    fn scratch_registers_round_trip_independently() {
        let mut w = Rp2040Watchdog::new();
        for i in 0..8u64 {
            w.write_u32(SCRATCH0 + i * 4, 0xA5A5_0000 | i as u32)
                .unwrap();
        }
        for i in 0..8u64 {
            assert_eq!(
                w.read_u32(SCRATCH0 + i * 4).unwrap(),
                0xA5A5_0000 | i as u32
            );
        }
    }

    /// LOAD is write-only and the PAUSE bits round-trip.
    #[test]
    fn load_is_write_only_and_pause_bits_round_trip() {
        let mut w = Rp2040Watchdog::new();
        w.write_u32(LOAD, 0x00AB_CDEF).unwrap();
        assert_eq!(w.read_u32(LOAD).unwrap(), 0, "LOAD is write-only");
        assert_eq!(w.read_u32(CTRL).unwrap() & CTRL_TIME_MASK, 0x00AB_CDEF);

        w.write_u32(CTRL, 1 << 25).unwrap(); // PAUSE_DBG0 only
        assert_eq!(w.read_u32(CTRL).unwrap() & CTRL_PAUSE_MASK, 1 << 25);
    }

    /// TICK.COUNT is the live divider phase, not a stored constant.
    #[test]
    fn tick_count_reports_the_live_divider_phase() {
        let mut w = Rp2040Watchdog::new();
        w.write_u32(TICK, 5 | TICK_ENABLE).unwrap();
        let phase = |w: &Rp2040Watchdog| w.read_u32(TICK).unwrap() >> TICK_COUNT_SHIFT;
        let mut seen = Vec::new();
        for _ in 0..6 {
            w.tick();
            seen.push(phase(&w));
        }
        // Fires on the first tick (count 0), reloads to CYCLES-1, counts down.
        assert_eq!(seen, vec![4, 3, 2, 1, 0, 4]);
    }
}
