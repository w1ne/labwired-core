// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{CycleClock, SimResult};
use std::cell::Cell;

/// ARMv7-M SysTick timer. Standard address: 0xE000_E010.
///
/// ## Drive modes (walk-free plan Part 2, batch B1)
///
/// Two mutually exclusive time sources, selected by ONE predicate
/// (`scheduler_mode`), exactly like the ESP32-C3 RTC main timer exemplar:
///
/// * **Scheduler mode** (`event-scheduler` feature + a [`CycleClock`] attached
///   at bus registration): `uses_scheduler()` is true, the per-cycle walk skips
///   this peripheral entirely, and
///   - `SYST_CVR` is **derived lazily** from the bus-published cycle clock with
///     exact modular arithmetic (`advance_counter`) — a `&self` read advances
///     `Cell`-held state to "now", so firmware delay loops polling `CVR` or
///     `CSR.COUNTFLAG` observe fresh time without any walk;
///   - the periodic exception (SysTick, system exception **15** — NOT an NVIC
///     line) is delivered by **scheduled events**: arming writes hand the bus a
///     `(delay, token)` via `take_scheduled_events`, and `on_event` claims the
///     wrap and reschedules the next one at its exact cycle, so there is no
///     cumulative drift at any tick interval. `Machine::apply_event_result`
///     routes `EventResult::system_exception` straight to
///     `Cpu::set_exception_pending(15)` — the same choke the legacy walk's
///     `PeripheralTickResult::system_exception` uses.
///
/// * **Legacy mode** (feature off, or no clock attached — hand-built test buses
///   that bypass the bus registration chokes): the per-cycle walk drives
///   `tick()` and the countdown advances eagerly, byte-identical to the
///   historical model.
///
/// ### Timing contract
///
/// Deadlines follow the run-loop convention (`SystemBus::collect_scheduled_events`
/// converts a post-write `(delay)` to `current_cycle + 1 + delay`): at tick
/// interval 1 on the batched `Machine::run` path, exception-15 pend cycles,
/// `CVR`/`COUNTFLAG` reads, `total_cycles` and register state are
/// **byte-identical** to the walk-driven reference. At interval N > 1 the
/// exception pend and lazy reads are quantised to the batch grid — at most one
/// interval late/stale, the same documented bound the write-path `sync_to`
/// ships (and strictly better than the legacy walk at interval N, which under
/// the default `tick_elapsed` slows the countdown by a factor of N).
///
/// ### Preserved semantics (differentially pinned)
///
/// - **Edge-triggered fire**: only a count-down transition to zero raises
///   COUNTFLAG/exception; a held or software-written zero reloads silently
///   (the Zephyr tickless-idle regression — see the legacy-mode tests).
/// - **COUNTFLAG read-clear**: a word read of `SYST_CSR` returns COUNTFLAG and
///   clears it; byte reads do not clear.
/// - **CVR write**: clears the counter to 0 AND COUNTFLAG; never fires.
/// - **RVR = 0**: counter parks at zero after the next zero-hit and never
///   fires again (matches ARMv7-M "the counter is disabled on the next wrap").
/// - **CALIB**: implementation-defined per chip, supplied by the chip yaml.
///
/// ### Tick-cost normalization (B1)
///
/// The legacy model charged `cycles: 1` into the peripheral tick-cost channel
/// on every enabled tick, inflating `total_cycles` by one extra cycle per tick
/// interval while armed — a sim artifact (real SysTick consumes zero core
/// cycles) that is structurally incompatible with deleting the walk. Both
/// modes now charge zero cost, so the walk-on reference and the scheduler
/// path agree cycle-for-cycle. (The logic-capture cost-path coverage that
/// piggybacked on this artifact now uses a dedicated test peripheral.)
#[derive(Debug, Default, serde::Serialize)]
pub struct Systick {
    /// SYST_CSR config bits (ENABLE/TICKINT/CLKSOURCE). COUNTFLAG (bit 16) is
    /// NOT stored here — it is held in `countflag` so a read of CSR can clear
    /// it (the ARMv7-M "reads clear COUNTFLAG" rule, which `cortex_m_systick`
    /// relies on; see `read_u32`).
    csr: u32,
    rvr: u32,
    /// Current counter value. `Cell` so the scheduler-mode `&self` read path
    /// can lazily advance it to the bus-published clock (walk-free plan Part 1
    /// read-side freshness). In legacy mode only `tick()` mutates it.
    cvr: Cell<u32>,
    calib: u32,
    /// SYST_CSR.COUNTFLAG. Set when the counter wraps to 0; cleared on a read
    /// of SYST_CSR or a write of SYST_CVR. `Cell` so the `&self` read path can
    /// clear it.
    countflag: Cell<bool>,
    /// Lazy-path anchor: the absolute published cycle `cvr` was last advanced
    /// to. Owned exclusively by `advance_to` (scheduler mode); the legacy walk
    /// never touches it.
    #[serde(skip)]
    anchor: Cell<u64>,
    /// Zero-hits that occurred with TICKINT set and have not yet been claimed
    /// by the event drain (`on_event` translates them into ONE exception-15
    /// pend — pends merge in the CPU's pending bitmap exactly as multiple
    /// wraps merge in silicon's single pend bit).
    #[serde(skip)]
    pending_fires: Cell<u32>,
    /// Arming-sequence token: bumped on every `take_scheduled_events` so an
    /// in-flight event chain scheduled under an older configuration dies on
    /// arrival (token mismatch) instead of racing the fresh chain.
    #[serde(skip)]
    arm_seq: u32,
    /// Bus-published cycle clock (walk-free plan Part 1). `Some` once the bus
    /// registration choke attaches it; `None` keeps the model on the legacy
    /// walk path.
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl Systick {
    pub fn new() -> Self {
        Self {
            csr: 0,
            rvr: 0,
            cvr: Cell::new(0),
            calib: 0x4000_0000, // No reference clock, no skew
            countflag: Cell::new(false),
            anchor: Cell::new(0),
            pending_fires: Cell::new(0),
            arm_seq: 0,
            clock: None,
        }
    }

    /// CALIB is implementation-defined per chip (TENMS/SKEW/NOREF). The chip
    /// yaml can supply the silicon value via `config: { calib: ... }`.
    pub fn with_calib(calib: u32) -> Self {
        Self {
            calib,
            ..Self::new()
        }
    }

    /// True when the event scheduler owns this timer's time base (feature on
    /// AND bus clock attached). Everything time-related branches on this ONE
    /// predicate so the two drive modes can never mix.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to
    /// the legacy walk path (`uses_scheduler() == false`). Used by the
    /// walk-on-vs-scheduler differential gates to build the reference config
    /// from the same bus assembly.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// Exact closed-form replay of `e` walk ticks from counter value `v` with
    /// reload `r`: returns `(new_value, zero_hits)`. Mirrors `tick()`'s
    /// edge-triggered sequence — from 0 the next tick reloads `r` (no fire);
    /// from `v > 0` the counter hits zero after `v` ticks (fire), then repeats
    /// with period `r + 1`; `r == 0` parks at zero after the first hit.
    fn advance_counter(v: u64, r: u64, e: u64) -> (u64, u64) {
        if e == 0 {
            return (v, 0);
        }
        if v == 0 {
            if r == 0 {
                return (0, 0);
            }
            let p = r + 1; // reload tick + r count-down ticks
            let k = e % p;
            let val = if k == 0 { 0 } else { r - (k - 1) };
            (val, e / p)
        } else if e < v {
            (v - e, 0)
        } else if r == 0 {
            // Hits zero at tick `v` (one fire), then reloads 0 forever.
            (0, 1)
        } else {
            let p = r + 1;
            let e2 = e - v; // ticks after the first zero-hit
            let k = e2 % p;
            let val = if k == 0 { 0 } else { r - (k - 1) };
            (val, 1 + e2 / p)
        }
    }

    /// Lazy advance to absolute published cycle `now` — callable from `&self`
    /// (all mutated state is in `Cell`). Idempotent; a `now` older than the
    /// anchor is ignored (the clock is monotonic within a run; a stale read
    /// must never rewind). The advanced window always has constant CSR/RVR:
    /// every MMIO write syncs first (bus `sync_to` choke), so settings changes
    /// never straddle a window.
    fn advance_to(&self, now: u64) {
        let anchor = self.anchor.get();
        if now <= anchor {
            return;
        }
        self.anchor.set(now);
        if (self.csr & 0x1) == 0 {
            // ENABLE=0: the counter is frozen; the window elapses unobserved.
            return;
        }
        let (val, wraps) = Self::advance_counter(
            self.cvr.get() as u64,
            (self.rvr & 0x00FF_FFFF) as u64,
            now - anchor,
        );
        self.cvr.set(val as u32);
        if wraps > 0 {
            self.countflag.set(true);
            // TICKINT gates whether a wrap pends the exception, evaluated over
            // the window (CSR is constant across it — see above). COUNTFLAG is
            // set regardless, exactly like the walk.
            if (self.csr & 0x2) != 0 {
                self.pending_fires.set(
                    self.pending_fires
                        .get()
                        .saturating_add(wraps.min(u32::MAX as u64) as u32),
                );
            }
        }
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

    /// Walk ticks until the next zero-hit from the CURRENT (just-synced)
    /// state, when one is armed to fire: requires ENABLE and TICKINT, and a
    /// reachable wrap (`v > 0`, or `v == 0` with `r > 0`).
    fn ticks_until_fire(&self) -> Option<u64> {
        if (self.csr & 0x3) != 0x3 {
            return None;
        }
        let v = self.cvr.get() as u64;
        let r = (self.rvr & 0x00FF_FFFF) as u64;
        if v > 0 {
            Some(v)
        } else if r > 0 {
            Some(r + 1) // reload tick + r count-down ticks
        } else {
            None
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            // COUNTFLAG (bit 16) folded in from `countflag`. Byte reads do not
            // clear it; the word read (`read_u32`) does, matching how the
            // driver accesses CSR.
            0x00 => self.csr | if self.countflag.get() { 0x1_0000 } else { 0 },
            0x04 => self.rvr,
            0x08 => self.cvr.get(),
            0x0C => self.calib,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                self.csr = value & 0x7;
                if std::env::var("LABWIRED_TRACE_SYSTICK").is_ok() {
                    eprintln!(
                        "SYSTICK CSR <- 0x{:08X} (enable={} tickint={} clksrc={})",
                        value,
                        value & 1,
                        (value >> 1) & 1,
                        (value >> 2) & 1
                    );
                }
            }
            0x04 => {
                self.rvr = value & 0x00FF_FFFF;
                if std::env::var("LABWIRED_TRACE_SYSTICK").is_ok() {
                    eprintln!(
                        "SYSTICK RVR <- 0x{:08X} ({})",
                        value & 0x00FF_FFFF,
                        value & 0x00FF_FFFF
                    );
                }
            }
            0x08 => {
                // Writing SYST_CVR clears the counter and COUNTFLAG. It does
                // NOT cancel an already-latched exception pend (pending_fires):
                // silicon keeps a wrap that already pended.
                self.cvr.set(0);
                self.countflag.set(false);
            }
            _ => {}
        }
    }
}

impl crate::Peripheral for Systick {
    fn read(&self, offset: u64) -> SimResult<u8> {
        self.sync_from_clock();
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        // Scheduler mode: advance the lazy counter to the published "now"
        // first, so polled CVR/COUNTFLAG reads observe fresh time (batch-
        // boundary freshness; exact at interval 1 on the run path).
        self.sync_from_clock();
        let val = self.read_reg(offset);
        // ARMv7-M: a read of SYST_CSR returns COUNTFLAG and then clears it.
        // `cortex_m_systick`'s elapsed() depends on this to detect a wrap, so
        // it must happen on the word read the driver issues.
        if offset == 0x00 {
            self.countflag.set(false);
        }
        Ok(val)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);

        // Modify byte
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        // Never runs in scheduler mode (the walk skips `uses_scheduler()`
        // peripherals; the guard keeps a stray direct call from corrupting the
        // lazily-anchored state).
        if self.scheduler_mode() {
            return crate::PeripheralTickResult::default();
        }
        if (self.csr & 0x1) == 0 {
            return crate::PeripheralTickResult {
                cycles: 0,
                ..Default::default()
            };
        }

        // ARMv7-M SysTick is EDGE-triggered: COUNTFLAG/interrupt are raised only
        // when the counter COUNTS DOWN to zero, never when it merely holds zero.
        // A zero counter (after reset, or after software writes SYST_CVR — which
        // hardware clears to 0) reloads from RVR on the next clock WITHOUT firing.
        // Modelling "cvr == 0 ⇒ fire" instead would make every `SYST_CVR <- 0`
        // the tickless-idle path issues each ISR re-pend SysTick immediately, so
        // the handler is re-entered before the interrupted thread can run.
        if self.cvr.get() == 0 {
            // Reload only; the previous wrap already fired (or this is the
            // initial/software-cleared zero).
            self.cvr.set(self.rvr);
            return crate::PeripheralTickResult {
                irq: false,
                cycles: 0,
                ..Default::default()
            };
        }

        self.cvr.set(self.cvr.get() - 1);
        if self.cvr.get() != 0 {
            return crate::PeripheralTickResult {
                irq: false,
                cycles: 0,
                ..Default::default()
            };
        }

        // Counter just transitioned to zero: set COUNTFLAG and raise the tick.
        self.countflag.set(true);
        // SysTick raises system exception 15 — NOT an NVIC IRQ. The bus
        // dispatches `system_exception` directly to the CPU's pending_exceptions
        // bitmap, bypassing NVIC ISER/ISPR. (Routing through NVIC would interpret
        // 15 as NVIC IRQ 15 = exception 31, which has no vector in standard
        // STM32 firmware.)
        let fire = (self.csr & 0x2) != 0;
        crate::PeripheralTickResult {
            irq: false,
            cycles: 0,
            dma_requests: None,
            system_exception: if fire { Some(15) } else { None },
            ..Default::default()
        }
    }

    fn uses_scheduler(&self) -> bool {
        // True once the bus attached its cycle clock (event-scheduler builds):
        // reads stay fresh through the lazy `advance_to` path and the periodic
        // exception rides scheduled events, so the walk is unnecessary.
        // Without a clock (feature off / hand-built buses) stay on the legacy
        // walk with exact historical semantics.
        self.scheduler_mode()
    }

    fn sync_to(&mut self, now_cycle: u64) {
        if self.scheduler_mode() {
            self.advance_to(now_cycle);
        }
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        // Kill any in-flight chain: the configuration (or counter) may have
        // just changed under this write, so its deadline is stale. The fresh
        // chain below carries the new token.
        self.arm_seq = self.arm_seq.wrapping_add(1);
        if self.pending_fires.get() > 0 {
            // A wrap already materialised (mid-batch write after the expiry
            // whose event chain we just killed): deliver at the next drain.
            return vec![(0, self.arm_seq)];
        }
        // `collect_scheduled_events` converts to `current_cycle + 1 + delay`;
        // the fire lands `d` walk ticks after the just-synced state, i.e. at
        // absolute cycle `current_cycle + d` — hence `d - 1` (d >= 1 always).
        self.ticks_until_fire()
            .map(|d| vec![(d - 1, self.arm_seq)])
            .unwrap_or_default()
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != self.arm_seq {
            // Stale chain (re-armed since this event was scheduled): die.
            return crate::sched::EventResult::default();
        }
        // Bring the lazy counter up to the drain cycle; this is what
        // materialises the wrap(s) this event was scheduled for.
        self.advance_to(sched.now());
        let fires = self.pending_fires.replace(0);
        crate::sched::EventResult {
            // One pend covers any number of accumulated wraps — pends merge in
            // the CPU pending bitmap exactly as they merge in silicon's single
            // PENDSTSET bit.
            system_exception: if fires > 0 { Some(15) } else { None },
            // Perpetuate the chain from the just-synced state: the delay is
            // measured from `sched.now()`, so the next expiry lands at its
            // exact absolute cycle — no cumulative drift at any interval.
            reschedule_delay: self.ticks_until_fire(),
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        // Anchor at the clock's current value so cycles that elapsed before
        // attach (normally zero — attach happens at bus assembly) are not
        // retroactively replayed into the counter.
        self.anchor.set(clock.now());
        self.clock = Some(clock);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        // Sync first so the serialized CVR reflects "now" (scheduler mode);
        // no-op in legacy mode. Keeps the snapshot shape identical across
        // drive modes for the determinism gates.
        self.sync_from_clock();
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    /// Enable SysTick with the given reload, leaving the counter cleared to 0
    /// (as hardware does on a SYST_CVR write).
    fn armed(reload: u8) -> Systick {
        let mut st = Systick::new();
        st.write(0x04, reload).unwrap(); // RVR
        st.write(0x08, 0x00).unwrap(); // CVR write → clears counter to 0
        st.write(0x00, 0x07).unwrap(); // CSR: ENABLE | TICKINT | CLKSOURCE
        st
    }

    /// Regression: a counter sitting at zero (freshly reloaded, or just cleared
    /// by a software SYST_CVR write) must RELOAD without firing. Only a count-
    /// down transition to zero raises the tick. Modelling "cvr == 0 ⇒ fire"
    /// makes the tickless-idle SYST_CVR write re-pend SysTick every step, so the
    /// ISR is re-entered before the interrupted thread can run (STM32 + Zephyr
    /// never reached main).
    #[test]
    fn fires_only_on_countdown_to_zero_not_on_loaded_zero() {
        let mut st = armed(4);

        // tick 1: counter is 0 (software-cleared) → reload to RVR, NO fire.
        assert_eq!(
            st.tick().system_exception,
            None,
            "reload tick must not fire"
        );
        // ticks 2..=4: 4→3→2→1, no fire yet.
        for _ in 0..3 {
            assert_eq!(st.tick().system_exception, None);
        }
        // tick 5: 1→0, the count-down edge → fire SysTick (exception 15).
        assert_eq!(
            st.tick().system_exception,
            Some(15),
            "count-down to zero must raise SysTick"
        );
        // tick 6: counter is 0 again → reload, NO fire.
        assert_eq!(
            st.tick().system_exception,
            None,
            "post-fire reload must not fire"
        );
    }

    /// A software write to SYST_CVR clears the counter; the immediately following
    /// tick must reload from RVR rather than fire (the exact path the Zephyr
    /// tickless idle exercises on every ISR).
    #[test]
    fn software_cvr_write_does_not_fire() {
        let mut st = armed(3);
        // Let it run partway down.
        st.tick(); // reload to 3
        st.tick(); // 3→2
                   // Software clears the counter.
        st.write(0x08, 0x00).unwrap();
        // Next tick reloads, does not fire.
        assert_eq!(st.tick().system_exception, None);
    }

    /// COUNTFLAG (SYST_CSR bit 16) reads as set after a wrap and is cleared by
    /// the word read, per ARMv7-M. `cortex_m_systick`'s elapsed() relies on this.
    #[test]
    fn countflag_sets_on_wrap_and_clears_on_read() {
        let mut st = armed(2);
        st.tick(); // reload to 2
        st.tick(); // 2→1
        assert_eq!(st.tick().system_exception, Some(15)); // 1→0, fire

        let csr = st.read_u32(0x00).unwrap();
        assert_eq!(csr & 0x1_0000, 0x1_0000, "COUNTFLAG set after wrap");
        let csr2 = st.read_u32(0x00).unwrap();
        assert_eq!(csr2 & 0x1_0000, 0, "COUNTFLAG cleared by the read");
    }

    /// A disabled SysTick (ENABLE=0) never ticks down or fires.
    #[test]
    fn disabled_systick_is_inert() {
        let mut st = Systick::new();
        st.write(0x04, 1).unwrap();
        st.write(0x08, 0x00).unwrap();
        // ENABLE not set.
        for _ in 0..10 {
            let r = st.tick();
            assert_eq!(r.system_exception, None);
            assert_eq!(r.cycles, 0);
        }
    }

    #[test]
    fn without_clock_stays_on_legacy_tick_path() {
        let st = Systick::new();
        assert!(
            !st.uses_scheduler(),
            "no cycle clock attached → the model must stay on the legacy walk \
             (hand-built buses that bypass the registration chokes keep exact semantics)"
        );
    }

    /// The closed-form lazy advance must replay the walk EXACTLY: property-check
    /// `advance_counter` against a literal tick-by-tick walk over a grid of
    /// (initial value, reload, elapsed) states, comparing both the final counter
    /// value and the number of zero-hits (fires).
    #[test]
    fn advance_counter_matches_iterated_walk_exactly() {
        fn walk(mut v: u64, r: u64, e: u64) -> (u64, u64) {
            let mut fires = 0;
            for _ in 0..e {
                if v == 0 {
                    v = r; // reload, no fire (edge-triggered)
                } else {
                    v -= 1;
                    if v == 0 {
                        fires += 1;
                    }
                }
            }
            (v, fires)
        }
        for r in [0u64, 1, 2, 3, 7, 99] {
            // `v > r` is reachable (firmware may shrink RVR mid-count), so the
            // grid deliberately includes it.
            for v in [0u64, 1, 2, 3, 5, 7, 99, 100] {
                for e in 0..=(3 * (r + 2) + v) {
                    assert_eq!(
                        Systick::advance_counter(v, r, e),
                        walk(v, r, e),
                        "advance_counter(v={v}, r={r}, e={e}) diverged from the walk"
                    );
                }
            }
        }
    }

    #[cfg(feature = "event-scheduler")]
    mod scheduler_mode {
        use super::*;
        use crate::CycleClock;

        fn armed_scheduler(reload: u32) -> (Systick, CycleClock) {
            let clock = CycleClock::default();
            let mut st = Systick::new();
            st.attach_cycle_clock(clock.clone());
            st.write(0x04, (reload & 0xFF) as u8).unwrap(); // RVR (small reloads)
            st.write(0x08, 0x00).unwrap(); // CVR clear
            st.write(0x00, 0x07).unwrap(); // ENABLE | TICKINT | CLKSOURCE
            (st, clock)
        }

        #[test]
        fn clock_attach_flips_to_scheduler_and_walk_tick_is_inert() {
            let (mut st, _clock) = armed_scheduler(4);
            assert!(st.uses_scheduler(), "clock attached → walk-independent");
            // A stray walk tick must not double-count against the lazy anchor.
            let r = st.tick();
            assert_eq!(r.system_exception, None);
            assert_eq!(
                st.read_u32(0x08).unwrap(),
                0,
                "tick inert in scheduler mode"
            );
        }

        #[test]
        fn lazy_cvr_read_tracks_published_clock_exactly() {
            let (st, clock) = armed_scheduler(4);
            // From v=0, r=4: tick 1 reloads to 4, ticks 2..5 count 3,2,1,0.
            clock.publish(1);
            assert_eq!(st.read_u32(0x08).unwrap(), 4, "reload tick");
            clock.publish(3);
            assert_eq!(st.read_u32(0x08).unwrap(), 2);
            clock.publish(5);
            assert_eq!(st.read_u32(0x08).unwrap(), 0, "zero-hit at tick 5");
            let csr = st.read_u32(0x00).unwrap();
            assert_eq!(csr & 0x1_0000, 0x1_0000, "COUNTFLAG folded in lazily");
            assert_eq!(
                st.read_u32(0x00).unwrap() & 0x1_0000,
                0,
                "word read clears COUNTFLAG"
            );
            // Next period: reload at 6, counts down to zero-hit at 10.
            clock.publish(10);
            assert_eq!(st.read_u32(0x08).unwrap(), 0);
            assert_eq!(
                st.read_u32(0x00).unwrap() & 0x1_0000,
                0x1_0000,
                "second wrap re-latches COUNTFLAG"
            );
        }

        #[test]
        fn disable_freezes_the_lazy_counter() {
            let (mut st, clock) = armed_scheduler(9);
            clock.publish(3);
            assert_eq!(st.read_u32(0x08).unwrap(), 7); // 0→9 (reload), 8, 7
                                                       // Disable; the window while disabled must not advance the counter.
            st.sync_to(3);
            st.write(0x00, 0x00).unwrap();
            clock.publish(100);
            assert_eq!(st.read_u32(0x08).unwrap(), 7, "frozen while ENABLE=0");
            // Re-enable; counting resumes from the frozen value.
            st.sync_to(100);
            st.write(0x00, 0x07).unwrap();
            clock.publish(102);
            assert_eq!(st.read_u32(0x08).unwrap(), 5);
        }

        #[test]
        fn arming_write_schedules_the_exact_expiry() {
            let (mut st, _clock) = armed_scheduler(99);
            // v=0, r=99 → first zero-hit d = r+1 = 100 ticks after the synced
            // state; the bus adds current_cycle + 1, so the peripheral hands
            // out d-1 = 99.
            let evs = st.take_scheduled_events();
            assert_eq!(evs.len(), 1);
            assert_eq!(evs[0].0, 99, "delay must be ticks-to-fire minus one");
        }

        #[test]
        fn on_event_claims_fire_and_reschedules_next_period() {
            let (mut st, clock) = armed_scheduler(99);
            let token = st.take_scheduled_events()[0].1;
            // Drain at the expiry cycle (fire at tick 100).
            clock.publish(100);
            let mut sched = crate::sched::EventScheduler::new();
            sched.advance_to(100);
            let mut bus = crate::bus::SystemBus::new();
            let res = st.on_event(token, &mut sched, &mut bus);
            assert_eq!(res.system_exception, Some(15), "wrap claims exception 15");
            assert_eq!(
                res.reschedule_delay,
                Some(100),
                "next expiry rescheduled at the exact period"
            );
            // A second drain at the same cycle claims nothing.
            let res2 = st.on_event(token, &mut sched, &mut bus);
            assert_eq!(res2.system_exception, None, "no double-claim");
        }

        #[test]
        fn stale_event_chain_dies_on_token_mismatch() {
            let (mut st, clock) = armed_scheduler(99);
            let old_token = st.take_scheduled_events()[0].1;
            // Re-arm (e.g. tickless idle rewrites CVR): kills the old chain.
            st.write(0x08, 0x00).unwrap();
            let new_token = st.take_scheduled_events()[0].1;
            assert_ne!(old_token, new_token);
            clock.publish(500);
            let mut sched = crate::sched::EventScheduler::new();
            sched.advance_to(500);
            let mut bus = crate::bus::SystemBus::new();
            let res = st.on_event(old_token, &mut sched, &mut bus);
            assert_eq!(res.system_exception, None, "stale chain must be inert");
            assert_eq!(res.reschedule_delay, None, "stale chain must not respawn");
        }

        #[test]
        fn tickint_off_wraps_set_countflag_but_never_pend() {
            let clock = CycleClock::default();
            let mut st = Systick::new();
            st.attach_cycle_clock(clock.clone());
            st.write(0x04, 4).unwrap();
            st.write(0x08, 0).unwrap();
            st.write(0x00, 0x05).unwrap(); // ENABLE | CLKSOURCE, no TICKINT
            assert!(
                st.take_scheduled_events().is_empty(),
                "no TICKINT → no exception events to schedule"
            );
            clock.publish(50); // many wraps elapse
            assert_eq!(
                st.read_u32(0x00).unwrap() & 0x1_0000,
                0x1_0000,
                "COUNTFLAG still latches without TICKINT"
            );
            assert_eq!(st.pending_fires.get(), 0, "no pends accumulate");
        }

        #[test]
        fn cvr_write_does_not_cancel_latched_pend() {
            let (mut st, clock) = armed_scheduler(4);
            let _ = st.take_scheduled_events();
            clock.publish(5); // wrap at tick 5 materialises on the next sync
            st.sync_to(5); // bus write-choke sync
            st.write(0x08, 0).unwrap(); // CVR clear: countflag cleared, pend kept
            assert_eq!(
                st.read_u32(0x00).unwrap() & 0x1_0000,
                0,
                "COUNTFLAG cleared"
            );
            let evs = st.take_scheduled_events();
            assert_eq!(
                evs.first().map(|e| e.0),
                Some(0),
                "already-latched pend must be delivered at the next drain (delay 0)"
            );
        }
    }
}
