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
//! The corpus declaration is `errata:` on the `watchdog` instance in
//! `configs/chips/rp2040.yaml`; its load-bearing integration check is
//! `crates/core/tests/rp2040_errata_watchdog_e1.rs`.
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
    /// Bus cycle the stored `counter`/`tick_count`/`bitten`/`reason` are exact
    /// at. Scheduler mode only: the countdown is not stepped every cycle, so
    /// the ticks of cycles `anchor+1..=now` are applied in closed form by
    /// [`Self::advance`] when a write or the expiry wake needs the state, and
    /// projected on a read by [`Self::projected`].
    anchor: u64,
}

/// The countdown state that moves with time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Live {
    counter: u32,
    tick_count: u32,
    bitten: bool,
    reason: u32,
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
            anchor: 0,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    /// True while the generator + dog can still change observable state.
    fn countdown_active(&self) -> bool {
        self.tick_running() && self.enabled && !self.bitten
    }

    fn live(&self) -> Live {
        Live {
            counter: self.counter,
            tick_count: self.tick_count,
            bitten: self.bitten,
            reason: self.reason,
        }
    }

    /// 24-bit counter value a stored `counter` of `value` needs this many
    /// watchdog ticks to expire: each tick takes [`DECREMENT_PER_TICK`] off and
    /// the bite is the tick that lands on zero, so `ceil(value / 2)`, at
    /// least one (a zero counter bites on the first tick).
    fn ticks_to_expire(value: u32) -> u64 {
        u64::from(value.div_ceil(DECREMENT_PER_TICK)).max(1)
    }

    /// Reference-clock cycles from the stored state to the cycle the dog
    /// bites: the generator fires on its `tick_count + 1`-th cycle and then
    /// every `tick_cycles`-th, and the bite is the
    /// [`Self::ticks_to_expire`]-th firing. `None` while the countdown is not
    /// running.
    fn cycles_to_bite(&self) -> Option<u64> {
        if !self.countdown_active() {
            return None;
        }
        let first = u64::from(self.tick_count) + 1;
        Some(first + (Self::ticks_to_expire(self.counter) - 1) * u64::from(self.tick_cycles))
    }

    /// The state `n` more reference-clock cycles on: exactly what `n` calls of
    /// [`Self::step_one`] leave behind, without the loop. Nothing moves
    /// unless the countdown is running, and a bite stops it.
    fn advanced(&self, n: u64) -> Live {
        let mut live = self.live();
        if n == 0 || !self.countdown_active() {
            return live;
        }
        let period = u64::from(self.tick_cycles);
        let phase = u64::from(self.tick_count);
        if n <= phase {
            live.tick_count = (phase - n) as u32;
            return live;
        }
        // Firings in `n` cycles: the first at cycle `phase + 1`, then one per
        // `period`.
        let fires = 1 + (n - phase - 1) / period;
        if fires >= Self::ticks_to_expire(self.counter) {
            live.counter = 0;
            live.bitten = true;
            live.reason |= REASON_TIMER;
            live.tick_count = (period - 1) as u32;
        } else {
            live.counter = self.counter - (fires * u64::from(DECREMENT_PER_TICK)) as u32;
            let last_fire = phase + 1 + (fires - 1) * period;
            live.tick_count = (period - 1 - (n - last_fire)) as u32;
        }
        live
    }

    /// Bring the stored state to bus cycle `now`.
    fn settle(&mut self, now: u64) {
        if now <= self.anchor {
            return;
        }
        let live = self.advanced(now - self.anchor);
        self.counter = live.counter;
        self.tick_count = live.tick_count;
        self.bitten = live.bitten;
        self.reason = live.reason;
        self.anchor = now;
    }

    /// The state at the bus-published cycle, for a `&self` read.
    fn projected(&self) -> Live {
        match (&self.clock, self.scheduler_mode()) {
            (Some(clock), true) => self.advanced(clock.now().saturating_sub(self.anchor)),
            _ => self.live(),
        }
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
                    | (self.projected().counter & CTRL_TIME_MASK)
            }
            LOAD => 0, // write-only
            REASON => self.projected().reason,
            SCRATCH0..=SCRATCH7 => self.scratch[((offset - SCRATCH0) / 4) as usize],
            TICK => {
                (self.projected().tick_count << TICK_COUNT_SHIFT)
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

    /// Bring the countdown up to the write's cycle before the write changes
    /// its rate, its load or its enable.
    fn sync_to(&mut self, now_cycle: u64) {
        if self.scheduler_mode() {
            self.settle(now_cycle);
        }
    }

    /// One wake, at the cycle the dog bites. Nothing observable moves before
    /// it: the counter and divider phase are projected on a read, so a running
    /// watchdog costs no per-cycle event. Every write re-arms (and so
    /// supersedes) it from the freshly settled state.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        match self.cycles_to_bite() {
            // delay-0 -> the next cycle, so a bite `k` cycles out is delay `k - 1`.
            Some(k) => vec![(k - 1, self.arm_seq)],
            None => Vec::new(),
        }
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != self.arm_seq {
            return crate::sched::EventResult::default();
        }
        self.settle(sched.now());
        // The RP2040 watchdog has no interrupt line: expiry is visible through
        // REASON only. A wake that finds the dog still counting (it cannot, but
        // a stale anchor must not lose the bite) sleeps to the real expiry.
        crate::sched::EventResult {
            reschedule_delay: self.cycles_to_bite(),
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

    /// The closed-form projection is `n` calls of `step_one`, for every
    /// divider ratio, divider phase, counter parity and elapsed time that
    /// straddles a firing, the bite, and the cycles after it.
    #[test]
    fn advanced_equals_stepping_cycle_by_cycle() {
        for cycles in [1u32, 2, 3, 12] {
            for phase in 0..cycles.min(4) {
                for counter in [0u32, 1, 2, 5, 6, 11] {
                    let mut base = Rp2040Watchdog::new();
                    base.tick_cycles = cycles;
                    base.tick_enable = true;
                    base.enabled = true;
                    base.counter = counter;
                    base.tick_count = phase;
                    let mut stepped = Rp2040Watchdog::new();
                    stepped.tick_cycles = cycles;
                    stepped.tick_enable = true;
                    stepped.enabled = true;
                    stepped.counter = counter;
                    stepped.tick_count = phase;
                    for n in 0..=(cycles as u64 * 8 + 6) {
                        assert_eq!(
                            base.advanced(n),
                            stepped.live(),
                            "cycles={cycles} phase={phase} counter={counter} n={n}"
                        );
                        // The scheduler chain stops at the bite, so the
                        // divider freezes there too.
                        if stepped.countdown_active() {
                            stepped.step_one();
                        }
                    }
                }
            }
        }
    }

    /// Drive one watchdog on a bus clock the way `Machine` does.
    struct Drive {
        wd: Rp2040Watchdog,
        clock: CycleClock,
        events: Vec<(u64, u32)>,
        now: u64,
        fired: u64,
    }

    impl Drive {
        fn new() -> Self {
            let clock = CycleClock::default();
            let mut wd = Rp2040Watchdog::new();
            wd.attach_cycle_clock(clock.clone());
            Self {
                wd,
                clock,
                events: Vec::new(),
                now: 0,
                fired: 0,
            }
        }

        fn write(&mut self, off: u64, value: u32) {
            self.wd.sync_to(self.now);
            self.wd.write_u32(off, value).unwrap();
            for (delay, token) in self.wd.take_scheduled_events() {
                self.events.push((self.now + 1 + delay, token));
            }
        }

        fn run_to(&mut self, cycle: u64) {
            while self.now < cycle {
                self.now += 1;
                self.clock.publish(self.now);
                let mut sched = crate::sched::EventScheduler::new();
                sched.advance_to(self.now);
                let due: Vec<_> = self
                    .events
                    .iter()
                    .copied()
                    .filter(|(d, _)| *d <= self.now)
                    .collect();
                self.events.retain(|(d, _)| *d > self.now);
                for (_, token) in due {
                    self.fired += 1;
                    let mut bus = crate::bus::SystemBus::new();
                    let res = self.wd.on_event(token, &mut sched, &mut bus);
                    if let Some(delay) = res.reschedule_delay {
                        self.events.push((self.now + delay, token));
                    }
                }
            }
        }

        fn time(&self) -> u32 {
            self.wd.read_u32(CTRL).unwrap() & CTRL_TIME_MASK
        }
    }

    /// An armed watchdog costs one wake (at the bite), reads the exact
    /// countdown in between, and latches `REASON.TIMER` on the exact cycle.
    #[test]
    fn an_armed_dog_sleeps_to_the_bite_and_reads_exactly() {
        if !crate::cycle_clock::event_scheduler_enabled() {
            return;
        }
        let mut d = Drive::new();
        d.write(TICK, 4 | TICK_ENABLE);
        d.write(LOAD, 100);
        d.write(CTRL, CTRL_PAUSE_MASK | CTRL_ENABLE); // armed at cycle 0
        assert_eq!(d.events.len(), 1, "exactly one wake for the whole countdown");
        // Fires on cycles 1, 5, 9, ... each taking 2: the 50th firing is cycle 197.
        d.run_to(1);
        assert_eq!(d.time(), 98);
        d.run_to(8);
        assert_eq!(d.time(), 96);
        d.run_to(9);
        assert_eq!(d.time(), 94);
        d.run_to(196);
        assert_eq!(d.time(), 2);
        assert_eq!(d.wd.read_u32(REASON).unwrap(), 0);
        d.run_to(197);
        assert_eq!(d.time(), 0);
        assert_eq!(d.wd.read_u32(REASON).unwrap(), REASON_TIMER);
        assert_eq!(d.fired, 1, "one wake, at the bite");
        d.run_to(5000);
        assert_eq!(d.fired, 1);
    }

    /// Feeding the dog (LOAD) supersedes the pending wake: the old bite never
    /// happens, the new one lands at the new cycle, and the stale token is
    /// ignored.
    #[test]
    fn feeding_supersedes_the_pending_wake() {
        if !crate::cycle_clock::event_scheduler_enabled() {
            return;
        }
        let mut d = Drive::new();
        d.write(TICK, 1 | TICK_ENABLE);
        d.write(LOAD, 20); // bites on the 10th firing: cycle 10
        d.write(CTRL, CTRL_ENABLE);
        d.run_to(7);
        assert_eq!(d.time(), 20 - 2 * 7);
        d.write(LOAD, 20); // fed at cycle 7: bite moves to cycle 17
        d.run_to(16);
        assert_eq!(d.time(), 20 - 2 * 9);
        assert_eq!(d.wd.read_u32(REASON).unwrap(), 0, "the old bite is gone");
        d.run_to(17);
        assert_eq!(d.wd.read_u32(REASON).unwrap(), REASON_TIMER);
        assert_eq!(d.time(), 0);
    }

    /// Reprogramming the tick generator mid-count settles the elapsed
    /// countdown at the old rate first.
    #[test]
    fn a_rate_change_settles_the_old_rate_first() {
        if !crate::cycle_clock::event_scheduler_enabled() {
            return;
        }
        let mut d = Drive::new();
        d.write(TICK, 3 | TICK_ENABLE);
        d.write(LOAD, 1000);
        d.write(CTRL, CTRL_ENABLE);
        d.run_to(31); // firings on cycles 1, 4, ..., 31 -> 11 firings
        assert_eq!(d.time(), 1000 - 22);
        d.write(TICK, 1 | TICK_ENABLE); // fires every cycle from now (phase restarts)
        d.run_to(41);
        assert_eq!(d.time(), 1000 - 22 - 2 * 10);
    }
}
