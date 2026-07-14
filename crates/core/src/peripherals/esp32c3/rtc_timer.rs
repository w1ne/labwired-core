// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 RTC_CNTL main timer (`0x6000_8000`) — the free-running RTC
//! slow-clock counter the IDF reads via `rtc_time_get`.
//!
//! `rtc_time_get` does: set `RTC_CNTL_TIME_UPDATE` (offset `0x0C`, bit31) to
//! latch the 48-bit counter into the readout registers, then read the low word
//! (`RTC_CNTL_TIME0`, `0x10`) and high word (`RTC_CNTL_TIME1`, `0x14`).
//!
//! A plain register-backed stub returns a *constant* timer, so every IDF loop
//! that waits on an RTC deadline spins forever. The most load-bearing example
//! is `calibrate_ocode` (RTC bandgap offset-code calibration): it polls a
//! regi2c comparator and exits either when the comparator settles *or* when a
//! ~10 ms RTC deadline expires. With no real RF/analog the comparator never
//! settles, so the loop relies entirely on the timeout — which never fires if
//! the timer is frozen. Modelling the counter as a real advancing timer lets
//! that loop (and every other RTC-deadline wait: clock cal, PHY cal, delays)
//! reach its designed timeout and continue, exactly as silicon does when the
//! calibration can't converge.
//!
//! ## Time source (walk-free plan Part 1 — first production user)
//!
//! The counter advances one tick per simulated CPU cycle; by default the
//! absolute slow-clock rate is not modelled — only that time *advances*
//! monotonically, which is all the deadline comparisons observe (and what the
//! C3 boot budget + every existing test rely on). The real silicon rate — the
//! internal RC_SLOW oscillator, measured on this board at ~136.7 kHz over the
//! built-in USB-JTAG (see [`RTC_SLOW_HZ_MEASURED`]) — can be opted into via
//! [`Esp32c3RtcTimer::set_slow_clock_hz`], which scales the firmware-visible
//! time down to that rate at readout for firmware that needs absolute RTC
//! wall-time. (Trade-off: RTC busy-wait delays then span ~1170x more simulated
//! cycles, since one RTC tick becomes ~1170 CPU cycles.) Two coexisting drive
//! modes:
//!
//! * **Scheduler mode** (`event-scheduler` feature + a [`crate::CycleClock`]
//!   attached by `SystemBus::add_peripheral`): `uses_scheduler()` is true, the
//!   per-cycle walk skips this peripheral entirely, and the counter advances
//!   **lazily** — `advance_to(now)` runs from the write-path `sync_to` choke
//!   and from the `TIME_UPDATE` latch itself, which pulls "now" from the
//!   bus-published clock. Freshness contract: the latched value is exact at
//!   batch boundaries and trails the true cycle by < one
//!   `peripheral_tick_interval` mid-batch — the same quantisation the legacy
//!   walk itself exhibits at that interval, so firmware delay loops (which
//!   re-latch every poll iteration) terminate exactly as before. This is what
//!   un-pins the walk: the old `uses_scheduler() == false` existed purely
//!   because a `&self` read could not sync (the historical comment feared
//!   "firmware delay loops observe stale time and spin forever").
//!
//! * **Legacy mode** (feature off, or no clock attached — e.g. hand-built
//!   test buses that bypass `add_peripheral`): the per-cycle walk drives
//!   `tick_elapsed(cycles)` and the counter advances eagerly, byte-identical
//!   to the historical behaviour.
//!
//! The two modes are mutually exclusive by construction: `tick_elapsed` is a
//! no-op while scheduler mode is active (the walk never calls it there — the
//! guard is defensive), and the lazy `advance_to` path is anchored so repeated
//! syncs to the same cycle are idempotent. The old code kept a parallel
//! `anchor_tick` bump inside `tick_elapsed` to feed a then-dead `sync_to`;
//! that was a double-count trap (relative walk anchor vs absolute cycle
//! anchor) and is gone — the anchor now belongs exclusively to the lazy path.
//!
//! All other registers in the window are register-backed (writes stored,
//! reads return the last value) so the rest of RTC_CNTL bring-up (reset-cause
//! seed at `0x38`, ANA config, …) behaves like the previous declarative stub.

use crate::{CycleClock, Peripheral, SimResult};
use std::cell::Cell;

const TIME_UPDATE: u64 = 0x0C; // bit31 = latch request
const TIME_LOW: u64 = 0x10;
const TIME_HIGH: u64 = 0x14;
const TIME_UPDATE_BIT: u32 = 1 << 31;

/// ESP32-C3 CPU clock the internal counter is anchored to (the cycle base the
/// bus `CycleClock` publishes). Used as the denominator when scaling the
/// free-running cycle count down to the RTC slow-clock rate.
pub const CPU_HZ: u64 = 160_000_000;

/// RTC slow-clock rate MEASURED on real silicon (this board, over the built-in
/// USB-JTAG: SYSTIMER-referenced 16.0 MHz time base gave 136.7 kHz for the
/// `RTC_CNTL` TIME counter — the internal RC_SLOW oscillator, nominal 150 kHz,
/// calibrated ~136–137 kHz). At 160 MHz CPU this is ~1170 CPU cycles per RTC
/// tick. See `Esp32c3RtcTimer::set_slow_clock_hz`.
pub const RTC_SLOW_HZ_MEASURED: u64 = 136_700;

#[derive(Debug)]
pub struct Esp32c3RtcTimer {
    /// Register-backed storage for the whole window (non-timer registers).
    regs: Vec<u32>,
    /// Free-running 48-bit counter tracking RAW elapsed CPU cycles (one step
    /// per cycle). The firmware-visible slow-clock time is this value scaled by
    /// `slow_num/slow_den` at readout (default 1:1).
    counter: Cell<u64>,
    /// Counter value latched by the most recent TIME_UPDATE write; what the
    /// TIME0/TIME1 readout registers return.
    latched: Cell<u64>,
    /// Lazy-path anchor: the absolute CPU cycle `counter` was last advanced
    /// to. Owned exclusively by `advance_to` (scheduler mode); the legacy
    /// walk never touches it.
    anchor_tick: Cell<u64>,
    /// Bus-published cycle clock (walk-free plan Part 1). `Some` once
    /// `SystemBus::add_peripheral` attaches it; `None` keeps the model on
    /// the legacy walk path.
    clock: Option<CycleClock>,
    /// Slow-clock scale applied AT READOUT: the latched (firmware-visible) time
    /// is `counter * slow_num / slow_den`. `counter` itself keeps tracking raw
    /// elapsed CPU cycles (so all the monotonic/anchor logic is unchanged); only
    /// the observable value is divided down to the RTC slow-clock rate.
    ///
    /// Default `(1, 1)` = one RTC tick per CPU cycle — the historical model
    /// contract ("slow-clock rate is not modelled; time advances monotonically")
    /// that every existing test and the C3 boot budget rely on. Call
    /// [`Self::set_slow_clock_hz`] with [`RTC_SLOW_HZ_MEASURED`] to opt into the
    /// silicon-faithful rate for firmware that needs absolute RTC wall-time
    /// (note: RTC busy-wait delays then take ~1170x more simulated cycles).
    slow_num: u64,
    slow_den: u64,
}

impl Default for Esp32c3RtcTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c3RtcTimer {
    /// `size_bytes` is the window size (rounded up to whole 32-bit words).
    pub fn new_sized(size_bytes: usize) -> Self {
        Self {
            regs: vec![0u32; size_bytes.div_ceil(4)],
            counter: Cell::new(0),
            latched: Cell::new(0),
            anchor_tick: Cell::new(0),
            clock: None,
            slow_num: 1,
            slow_den: 1,
        }
    }

    /// Opt into a modelled RTC slow-clock rate of `hz` (relative to the
    /// [`CPU_HZ`] cycle base). Pass [`RTC_SLOW_HZ_MEASURED`] for the
    /// silicon-measured ~136.7 kHz. `hz == CPU_HZ` (or leaving the default)
    /// keeps the 1:1 "monotonic-only" contract. `hz == 0` is ignored.
    pub fn set_slow_clock_hz(&mut self, hz: u64) {
        if hz == 0 {
            return;
        }
        self.slow_num = hz;
        self.slow_den = CPU_HZ;
    }

    /// Scale a raw elapsed-cycle count down to the modelled RTC slow-clock rate.
    #[inline]
    fn to_slow_ticks(&self, cycles: u64) -> u64 {
        if self.slow_num == self.slow_den {
            return cycles; // 1:1 fast path — byte-identical to the old model.
        }
        (cycles as u128 * self.slow_num as u128 / self.slow_den as u128) as u64
    }

    pub fn new() -> Self {
        Self::new_sized(0x100)
    }

    /// True when the event scheduler owns this timer's time base (feature
    /// on AND bus clock attached). Everything time-related branches on this
    /// ONE predicate so the two drive modes can never mix.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Lazy advance to absolute CPU cycle `now` — callable from `&self`
    /// (all mutated state is in `Cell`). Idempotent: repeated calls with the
    /// same `now` add nothing; `now` older than the anchor is ignored (the
    /// clock is monotonic within a run; a stale read must never rewind).
    fn advance_to(&self, now: u64) {
        let anchor = self.anchor_tick.get();
        if now <= anchor {
            return;
        }
        self.counter
            .set(self.counter.get().wrapping_add(now - anchor));
        self.anchor_tick.set(now);
    }

    /// Pull "now" from the bus-published clock and advance. No-op without an
    /// attached clock (legacy mode — the walk advances the counter instead).
    fn sync_from_clock(&self) {
        if let Some(clock) = &self.clock {
            if self.scheduler_mode() {
                self.advance_to(clock.now());
            }
        }
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to
    /// the legacy walk path (`uses_scheduler() == false`). Used by the
    /// walk-on-vs-scheduler differential gates to build the reference config
    /// from the same bus assembly.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }
}

impl Peripheral for Esp32c3RtcTimer {
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
        Ok(match offset {
            TIME_LOW => self.latched.get() as u32,
            TIME_HIGH => (self.latched.get() >> 32) as u32,
            _ => *self.regs.get((offset / 4) as usize).unwrap_or(&0),
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset == TIME_UPDATE && (value & TIME_UPDATE_BIT) != 0 {
            // Latch the current counter into the readout registers. In
            // scheduler mode, first advance to the bus-published "now" so the
            // latch is fresh-to-batch-start even though the walk no longer
            // ticks this model. (The bus write path already ran `sync_to`
            // before this write; the explicit sync keeps direct/unit-test
            // writes correct too, and is idempotent.)
            self.sync_from_clock();
            // Scale the raw elapsed-cycle counter down to the modelled RTC
            // slow-clock rate at readout (default 1:1 → byte-identical).
            self.latched.set(self.to_slow_ticks(self.counter.get()));
        }
        if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
            *slot = value;
        }
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        self.tick_elapsed(1)
    }

    /// Legacy walk drive: one slow-clock tick per elapsed CPU cycle. Never
    /// runs in scheduler mode (the walk skips `uses_scheduler()` peripherals;
    /// the guard below keeps a stray direct call from double-counting against
    /// the lazy anchor).
    fn tick_elapsed(&mut self, cycles: u64) -> crate::PeripheralTickResult {
        if !self.scheduler_mode() {
            self.counter.set(self.counter.get().wrapping_add(cycles));
        }
        crate::PeripheralTickResult::default()
    }

    fn uses_scheduler(&self) -> bool {
        // True once the bus attached its cycle clock (event-scheduler builds):
        // reads stay fresh through the lazy `advance_to` path, so the old
        // "stale time → delay loops spin forever" blocker is gone. Without a
        // clock (feature off / hand-built buses) stay on the legacy walk.
        self.scheduler_mode()
    }

    fn sync_to(&mut self, now_cycle: u64) {
        self.advance_to(now_cycle);
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        // Anchor at the clock's current value so cycles that elapsed before
        // attach (normally zero — attach happens at bus assembly) are not
        // retroactively credited to the counter.
        self.anchor_tick.set(clock.now());
        self.clock = Some(clock);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    /// Capture the free-running RTC counter + latched readout + register
    /// window. Boot-critical for a rom-boot resume: IDF RTC-deadline loops
    /// (e.g. `calibrate_ocode`) exit on a ~10 ms RTC timeout, so the number of
    /// calibration retries — and hence the exact boot-log lines — depends on
    /// the counter value carrying across the snapshot.
    fn runtime_snapshot(&self) -> Vec<u8> {
        let snap = RtcTimerSnapshot {
            regs: self.regs.clone(),
            counter: self.counter.get(),
            latched: self.latched.get(),
            anchor_tick: self.anchor_tick.get(),
        };
        bincode::serialize(&snap).expect("bincode serialize Esp32c3RtcTimer")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        let snap: RtcTimerSnapshot = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Esp32c3RtcTimer snapshot decode: {e}"))
        })?;
        self.regs = snap.regs;
        self.counter.set(snap.counter);
        self.latched.set(snap.latched);
        // Re-anchor rather than trusting the persisted anchor: a snapshot is
        // typically resumed on a FRESH machine whose cycle count restarts at
        // ~0, so a large persisted anchor would make `advance_to(now)` see
        // `now <= anchor` and freeze the counter for millions of cycles —
        // silently re-introducing the spin-forever failure the model exists
        // to prevent. Anchoring to the live clock keeps the restored counter
        // value (the part boot-log determinism depends on) and resumes
        // monotonic advance from the resuming machine's "now". The persisted
        // field is kept in the blob for format stability / legacy readers.
        match &self.clock {
            Some(clock) => self.anchor_tick.set(clock.now()),
            None => self.anchor_tick.set(snap.anchor_tick),
        }
        Ok(())
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RtcTimerSnapshot {
    regs: Vec<u32>,
    counter: u64,
    latched: u64,
    #[serde(default)]
    anchor_tick: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtc_time_get(t: &mut Esp32c3RtcTimer) -> u64 {
        // Mirror the IDF sequence: set TIME_UPDATE, read low then high.
        t.write_u32(TIME_UPDATE, TIME_UPDATE_BIT).unwrap();
        let lo = t.read_u32(TIME_LOW).unwrap() as u64;
        let hi = t.read_u32(TIME_HIGH).unwrap() as u64;
        lo | (hi << 32)
    }

    #[test]
    fn timer_advances_and_latches() {
        let mut t = Esp32c3RtcTimer::new();
        let t0 = rtc_time_get(&mut t);
        for _ in 0..1000 {
            t.tick();
        }
        let t1 = rtc_time_get(&mut t);
        assert!(t1 > t0, "RTC timer must advance ({t0} -> {t1})");
        assert_eq!(t1 - t0, 1000, "one tick per step");
    }

    #[test]
    fn readout_frozen_until_next_update() {
        let mut t = Esp32c3RtcTimer::new();
        t.write_u32(TIME_UPDATE, TIME_UPDATE_BIT).unwrap();
        let snap = t.read_u32(TIME_LOW).unwrap();
        for _ in 0..50 {
            t.tick();
        }
        // No new latch: readout is unchanged even though the counter advanced.
        assert_eq!(t.read_u32(TIME_LOW).unwrap(), snap);
        t.write_u32(TIME_UPDATE, TIME_UPDATE_BIT).unwrap();
        assert_eq!(t.read_u32(TIME_LOW).unwrap(), snap + 50);
    }

    #[test]
    fn tick_elapsed_matches_repeated_tick() {
        let mut repeated = Esp32c3RtcTimer::new();
        let mut elapsed = Esp32c3RtcTimer::new();

        for _ in 0..1000 {
            repeated.tick();
        }
        elapsed.tick_elapsed(1000);

        assert_eq!(rtc_time_get(&mut elapsed), rtc_time_get(&mut repeated));
    }

    #[test]
    fn without_clock_stays_on_legacy_tick_path() {
        let t = Esp32c3RtcTimer::new();
        assert!(
            !t.uses_scheduler(),
            "no cycle clock attached → the model must stay on the legacy walk \
             (hand-built buses that bypass add_peripheral keep exact semantics)"
        );
    }

    #[cfg(feature = "event-scheduler")]
    #[test]
    fn clock_attach_flips_to_scheduler_and_latch_tracks_published_clock() {
        let clock = CycleClock::default();
        let mut t = Esp32c3RtcTimer::new();
        t.attach_cycle_clock(clock.clone());
        assert!(
            t.uses_scheduler(),
            "clock attached under event-scheduler → walk-independent"
        );

        // The walk no longer drives it; the latch must pull time from the
        // published clock — this is the exact firmware delay-loop shape the
        // old comment feared (poll = TIME_UPDATE write + TIME0/1 read).
        clock.publish(1234);
        assert_eq!(rtc_time_get(&mut t), 1234, "latch synced to published now");
        clock.publish(1234 + 4096);
        assert_eq!(rtc_time_get(&mut t), 1234 + 4096, "monotonic re-latch");

        // Idempotent: re-latching at the same published cycle adds nothing.
        assert_eq!(rtc_time_get(&mut t), 1234 + 4096);
    }

    /// The opt-in silicon-faithful slow-clock rate: after
    /// `set_slow_clock_hz(RTC_SLOW_HZ_MEASURED)`, the firmware-visible RTC time
    /// advances at the HW-measured ~136.7 kHz relative to the CPU cycle base —
    /// NOT 1:1. Locks in the rate captured on real hardware (this board, over
    /// the built-in USB-JTAG). The default path stays 1:1 (asserted above), so
    /// this is purely additive and breaks no existing timing.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn faithful_slow_clock_rate_matches_measured_silicon() {
        let clock = CycleClock::default();
        let mut t = Esp32c3RtcTimer::new();
        t.attach_cycle_clock(clock.clone());
        t.set_slow_clock_hz(RTC_SLOW_HZ_MEASURED);

        // Advance the CPU cycle base by exactly one second's worth of cycles.
        clock.publish(CPU_HZ);
        let ticks = rtc_time_get(&mut t);

        // One CPU-second must read as ~RTC_SLOW_HZ_MEASURED RTC ticks (exact
        // integer division of CPU_HZ * hz / CPU_HZ == hz here).
        assert_eq!(
            ticks, RTC_SLOW_HZ_MEASURED,
            "faithful RTC rate: 1 CPU-second must read {RTC_SLOW_HZ_MEASURED} ticks, got {ticks}"
        );

        // Half a second → half the ticks (the scale is linear in elapsed cycles).
        clock.publish(CPU_HZ + CPU_HZ / 2);
        let ticks2 = rtc_time_get(&mut t);
        assert_eq!(
            ticks2,
            RTC_SLOW_HZ_MEASURED + RTC_SLOW_HZ_MEASURED / 2,
            "1.5 CPU-seconds must read 1.5x the RTC ticks"
        );

        // Sanity: the raw ~1170 CPU-cycles-per-RTC-tick ratio the capture found.
        let cycles_per_tick = CPU_HZ / RTC_SLOW_HZ_MEASURED;
        assert!(
            (1160..=1180).contains(&cycles_per_tick),
            "measured ratio ~1170 CPU cycles per RTC tick, got {cycles_per_tick}"
        );
    }

    #[cfg(feature = "event-scheduler")]
    #[test]
    fn scheduler_mode_write_sync_and_clock_sync_do_not_double_count() {
        let clock = CycleClock::default();
        let mut t = Esp32c3RtcTimer::new();
        t.attach_cycle_clock(clock.clone());

        clock.publish(500);
        // Bus write path: sync_to(current_cycle) runs before the MMIO write…
        t.sync_to(500);
        // …then the TIME_UPDATE latch syncs from the clock again. Same cycle,
        // so the counter must be advanced exactly once.
        assert_eq!(rtc_time_get(&mut t), 500);

        // A stray legacy tick in scheduler mode must not double-count either.
        t.tick_elapsed(64);
        assert_eq!(
            rtc_time_get(&mut t),
            500,
            "tick_elapsed inert in scheduler mode"
        );
    }

    #[cfg(feature = "event-scheduler")]
    #[test]
    fn resume_re_anchors_and_keeps_counting() {
        // Cold machine ran to cycle 150M and snapshotted.
        let cold_clock = CycleClock::default();
        let mut cold = Esp32c3RtcTimer::new();
        cold.attach_cycle_clock(cold_clock.clone());
        cold_clock.publish(150_000_000);
        let cold_time = rtc_time_get(&mut cold);
        assert_eq!(cold_time, 150_000_000);
        let blob = cold.runtime_snapshot();

        // Resume on a FRESH machine whose cycle count restarts near zero.
        let warm_clock = CycleClock::default();
        let mut warm = Esp32c3RtcTimer::new();
        warm.attach_cycle_clock(warm_clock.clone());
        warm.restore_runtime_snapshot(&blob).unwrap();

        // The restored counter value carries over…
        warm_clock.publish(0);
        assert_eq!(
            rtc_time_get(&mut warm),
            cold_time,
            "counter survives resume"
        );
        // …and time keeps advancing from the resuming machine's clock instead
        // of freezing until it catches up to the persisted 150M anchor (the
        // stale-anchor spin-forever trap).
        warm_clock.publish(1_000);
        assert_eq!(
            rtc_time_get(&mut warm),
            cold_time + 1_000,
            "counter must keep advancing immediately after resume"
        );
    }

    #[test]
    fn other_registers_are_register_backed() {
        let mut t = Esp32c3RtcTimer::new();
        // e.g. the reset-cause seed at offset 0x38.
        t.write_u32(0x38, 0x1).unwrap();
        assert_eq!(t.read_u32(0x38).unwrap(), 0x1);
    }
}
