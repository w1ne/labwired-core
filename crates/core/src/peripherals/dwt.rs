// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ARMv7-M DWT (Data Watchpoint & Trace) — the free-running cycle counter
//! `CYCCNT` at `DWT_CYCCNT` (offset 0x04), gated by `DWT_CTRL.CYCCNTENA`
//! (offset 0x00, bit 0).
//!
//! ## Drive modes (walk-free plan Part 1, the purest lazy-read case)
//!
//! `CYCCNT` is a linear function of elapsed CPU cycles while `CYCCNTENA` is
//! set — no IRQ, no DMA, no compare (the comparators / CPI-LSU counters are
//! not modelled), so it is the simplest possible scheduler-migrated peripheral:
//! a bare lazily-derived counter with NO scheduled events. Two mutually
//! exclusive time sources, selected by ONE predicate (`scheduler_mode`),
//! exactly like [`crate::peripherals::systick::Systick`]'s lazy `SYST_CVR`:
//!
//! * **Scheduler mode** (`event-scheduler` feature + a [`CycleClock`] attached
//!   at bus registration): `uses_scheduler()` is true, the per-cycle walk skips
//!   this peripheral entirely, and `CYCCNT` is **derived lazily** from the
//!   bus-published cycle clock — a `&self` read advances the `Cell`-held counter
//!   to "now" (`advance_to`), so firmware polling `CYCCNT` for a busy-wait /
//!   micro-delay / cycle-profiling loop observes fresh cycles with NO walk. The
//!   counter counts TRUE CPU cycles (a frozen `CYCCNT` under walk-deletion would
//!   be a fidelity bug — this keeps it live).
//!
//! * **Legacy mode** (feature off, or no clock attached — hand-built test buses
//!   that bypass the bus registration chokes): the per-cycle walk drives
//!   `tick()` and the counter advances eagerly by one per enabled cycle,
//!   byte-identical to the historical model.
//!
//! ### Timing contract
//!
//! On the batched `Machine::run` path at tick interval 1, a `CYCCNT` read is
//! **byte-identical** to the walk-driven reference at every instruction
//! boundary: the batch is one instruction, so the bus-published clock is the
//! exact current cycle. At interval N > 1 lazy reads quantise to the batch-start
//! grid (≤ one interval stale, never frozen, no cumulative drift) — the same
//! documented bound the write-path [`Peripheral::sync_to`] ships, and strictly
//! better than the legacy walk at interval N (which under the default
//! `tick_elapsed` would slow the count by a factor of N).
//!
//! ### Preserved semantics (differentially pinned)
//!
//! - `CYCCNTENA` (CTRL bit 0) gates counting; clearing it freezes `CYCCNT`
//!   exactly where it stood. The window between any two MMIO writes has a
//!   constant CTRL (every write syncs first via the bus `sync_to` choke), so a
//!   settings change never straddles a lazy window.
//! - A software write to `CYCCNT` sets it verbatim; from then it advances from
//!   the written value (used by delay/profiling code that resets the counter).
//! - `CYCCNT` is 32-bit and wraps modulo 2^32, matching silicon.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;

const DWT_CTRL: u64 = 0x00;
const DWT_CYCCNT: u64 = 0x04;

/// CTRL.CYCCNTENA — enables the cycle counter.
const CTRL_CYCCNTENA: u32 = 1 << 0;

#[derive(Debug)]
pub struct Dwt {
    /// DWT_CTRL. Plain `u32`: only the `&mut self` write path mutates it, and
    /// the lazy `&self` read path merely reads it (the window between writes has
    /// constant CTRL, so `advance_to` can safely apply it across the window).
    ctrl: u32,
    /// `CYCCNT` as of `anchor`. `Cell` so the scheduler-mode `&self` read path
    /// can lazily advance it to the bus-published clock (walk-free plan Part 1
    /// read-side freshness). In legacy mode only `tick()` mutates it.
    cyccnt: Cell<u32>,
    /// Lazy-path anchor: the absolute published cycle `cyccnt` was last advanced
    /// to. Owned exclusively by `advance_to` (scheduler mode); the legacy walk
    /// never touches it.
    anchor: Cell<u64>,
    /// Bus-published cycle clock (walk-free plan Part 1). `Some` once the bus
    /// registration choke attaches it; `None` keeps the model on the legacy
    /// walk path.
    clock: Option<CycleClock>,
}

impl Dwt {
    pub fn new() -> Self {
        Self {
            ctrl: 0,
            cyccnt: Cell::new(0),
            anchor: Cell::new(0),
            clock: None,
        }
    }

    /// True when the event scheduler owns this counter's time base (feature on
    /// AND bus clock attached). Everything time-related branches on this ONE
    /// predicate so the two drive modes can never mix.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to the
    /// legacy walk path (`uses_scheduler() == false`). Used by the
    /// walk-on-vs-scheduler differential gate to build the reference lane from
    /// the same bus assembly.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// Lazily advance `cyccnt` to absolute published cycle `now` — callable from
    /// `&self` (all mutated state is in `Cell`). Idempotent; a `now` older than
    /// the anchor is ignored (the clock is monotonic within a run; a stale read
    /// must never rewind). `CYCCNT += (now - anchor)` while `CYCCNTENA` is set,
    /// wrapping modulo 2^32 exactly as the per-cycle walk's `fetch_add(1)` chain
    /// does. The advanced window always has a constant CTRL: every MMIO write
    /// syncs first (bus `sync_to` choke), so an enable/disable never straddles a
    /// window.
    fn advance_to(&self, now: u64) {
        let anchor = self.anchor.get();
        if now <= anchor {
            return;
        }
        let elapsed = now - anchor;
        self.anchor.set(now);
        if (self.ctrl & CTRL_CYCCNTENA) != 0 {
            self.cyccnt
                .set(self.cyccnt.get().wrapping_add(elapsed as u32));
        }
    }

    /// Pull "now" from the bus-published clock and advance. No-op without an
    /// attached clock (legacy mode — the walk advances the counter instead).
    fn sync_from_clock(&self) {
        if self.scheduler_mode() {
            if let Some(clock) = &self.clock {
                self.advance_to(clock.now());
            }
        }
    }

    /// The current register word, folding in the lazily-advanced `CYCCNT`.
    /// Assumes the counter has already been synced.
    fn read_reg(&self, aligned_offset: u64) -> u32 {
        match aligned_offset {
            DWT_CTRL => self.ctrl,
            DWT_CYCCNT => self.cyccnt.get(),
            _ => 0,
        }
    }
}

impl Default for Dwt {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Dwt {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Scheduler mode: advance the lazy counter to the published "now" first,
        // so polled CYCCNT reads observe fresh cycles (batch-boundary freshness;
        // exact at interval 1 on the run path). No-op in legacy mode.
        self.sync_from_clock();
        let val = self.read_reg(offset & !3);
        let byte_offset = (offset & 3) as u32;
        Ok(((val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.sync_from_clock();
        Ok(self.read_reg(offset & !3))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // `Bus` decomposes a u32 write into 4 byte writes; load-modify-store the
        // target register. In scheduler mode the bus `sync_to` choke has already
        // advanced the counter to the write cycle and re-anchored, so the
        // load-modify-store operates on the fresh value (and the post-write
        // anchor stays at the write cycle).
        let aligned_offset = offset & !3;
        let byte_shift = (offset & 3) * 8;
        let mask = 0xFF_u32 << byte_shift;
        let inserted = (value as u32) << byte_shift;

        match aligned_offset {
            DWT_CTRL => {
                self.ctrl = (self.ctrl & !mask) | inserted;
            }
            DWT_CYCCNT => {
                let cur = self.cyccnt.get();
                self.cyccnt.set((cur & !mask) | inserted);
            }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Never runs in scheduler mode (the walk skips `uses_scheduler()`
        // peripherals; the guard keeps a stray direct call from corrupting the
        // lazily-anchored state).
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        if (self.ctrl & CTRL_CYCCNTENA) != 0 {
            self.cyccnt.set(self.cyccnt.get().wrapping_add(1));
        }
        PeripheralTickResult::default()
    }

    fn uses_scheduler(&self) -> bool {
        // True once the bus attached its cycle clock (event-scheduler builds):
        // reads stay fresh through the lazy `advance_to` path, so the walk is
        // unnecessary. Without a clock (feature off / hand-built buses) stay on
        // the legacy walk with exact historical semantics.
        self.scheduler_mode()
    }

    fn sync_to(&mut self, now_cycle: u64) {
        if self.scheduler_mode() {
            self.advance_to(now_cycle);
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        // Anchor at the clock's current value so cycles that elapsed before
        // attach (normally zero — attach happens at bus assembly) are not
        // retroactively replayed into the counter.
        self.anchor.set(clock.now());
        self.clock = Some(clock);
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        // Side-effect-free: fold in the lazily-advanced counter WITHOUT mutating
        // the anchor, so a debug probe never perturbs the lazy state. Reads the
        // published clock directly.
        let cyccnt = if self.scheduler_mode() {
            match &self.clock {
                Some(clock) => {
                    let now = clock.now();
                    let anchor = self.anchor.get();
                    let base = self.cyccnt.get();
                    if now > anchor && (self.ctrl & CTRL_CYCCNTENA) != 0 {
                        base.wrapping_add((now - anchor) as u32)
                    } else {
                        base
                    }
                }
                None => self.cyccnt.get(),
            }
        } else {
            self.cyccnt.get()
        };
        let val = match offset & !3 {
            DWT_CTRL => self.ctrl,
            DWT_CYCCNT => cyccnt,
            _ => return None,
        };
        let byte_offset = (offset & 3) as u32;
        Some(((val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        // Sync first so the serialized CYCCNT reflects "now" (scheduler mode);
        // no-op in legacy mode. Keeps the snapshot shape identical across drive
        // modes for the determinism gates.
        self.sync_from_clock();
        serde_json::json!({
            "ctrl": self.ctrl,
            "cyccnt": self.cyccnt.get(),
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Some(obj) = state.as_object() {
            if let Some(ctrl) = obj.get("ctrl").and_then(|v| v.as_u64()) {
                self.ctrl = ctrl as u32;
            }
            if let Some(cyccnt) = obj.get("cyccnt").and_then(|v| v.as_u64()) {
                self.cyccnt.set(cyccnt as u32);
            }
        }
        // Re-anchor to "now" so a restored counter advances from the restored
        // value rather than replaying the pre-restore window.
        if let Some(clock) = &self.clock {
            self.anchor.set(clock.now());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_tick_counts_only_when_enabled() {
        let mut dwt = Dwt::new();
        // Disabled: no counting.
        for _ in 0..10 {
            dwt.tick();
        }
        assert_eq!(dwt.read_u32(DWT_CYCCNT).unwrap(), 0);
        // Enable CYCCNTENA.
        dwt.write(DWT_CTRL, 0x01).unwrap();
        for _ in 0..5 {
            dwt.tick();
        }
        assert_eq!(dwt.read_u32(DWT_CYCCNT).unwrap(), 5);
    }

    #[test]
    fn software_cyccnt_write_sets_value() {
        let mut dwt = Dwt::new();
        dwt.write(DWT_CTRL, 0x01).unwrap();
        // Byte-decomposed word write of 0x0000_1234.
        dwt.write(DWT_CYCCNT, 0x34).unwrap();
        dwt.write(DWT_CYCCNT + 1, 0x12).unwrap();
        dwt.write(DWT_CYCCNT + 2, 0x00).unwrap();
        dwt.write(DWT_CYCCNT + 3, 0x00).unwrap();
        assert_eq!(dwt.read_u32(DWT_CYCCNT).unwrap(), 0x1234);
        dwt.tick();
        assert_eq!(dwt.read_u32(DWT_CYCCNT).unwrap(), 0x1235);
    }

    #[test]
    fn without_clock_stays_on_legacy_tick_path() {
        let dwt = Dwt::new();
        assert!(
            !dwt.uses_scheduler(),
            "no cycle clock attached → the model must stay on the legacy walk \
             (hand-built buses that bypass the registration chokes keep exact semantics)"
        );
    }

    #[cfg(feature = "event-scheduler")]
    mod scheduler_mode {
        use super::*;
        use crate::CycleClock;

        fn enabled_scheduler() -> (Dwt, CycleClock) {
            let clock = CycleClock::default();
            let mut dwt = Dwt::new();
            dwt.attach_cycle_clock(clock.clone());
            dwt.write(DWT_CTRL, 0x01).unwrap(); // CYCCNTENA
            (dwt, clock)
        }

        #[test]
        fn clock_attach_flips_to_scheduler_and_walk_tick_is_inert() {
            let (mut dwt, _clock) = enabled_scheduler();
            assert!(dwt.uses_scheduler(), "clock attached → walk-independent");
            // A stray walk tick must not double-count against the lazy anchor.
            dwt.tick();
            assert_eq!(
                dwt.read_u32(DWT_CYCCNT).unwrap(),
                0,
                "tick inert in scheduler mode (clock still at 0)"
            );
        }

        #[test]
        fn lazy_cyccnt_read_tracks_published_clock_exactly() {
            let (dwt, clock) = enabled_scheduler();
            // CYCCNT == published cycle (enabled from cycle 0, base 0).
            for c in [1u64, 7, 8, 100, 12345] {
                clock.publish(c);
                assert_eq!(
                    dwt.read_u32(DWT_CYCCNT).unwrap() as u64,
                    c,
                    "CYCCNT must equal elapsed cycles at c={c}"
                );
            }
        }

        #[test]
        fn disable_freezes_the_counter() {
            let (mut dwt, clock) = enabled_scheduler();
            clock.publish(50);
            assert_eq!(dwt.read_u32(DWT_CYCCNT).unwrap(), 50);
            // Disable CYCCNTENA (bus syncs to 50 before the write).
            dwt.sync_to(50);
            dwt.write(DWT_CTRL, 0x00).unwrap();
            clock.publish(1000);
            assert_eq!(
                dwt.read_u32(DWT_CYCCNT).unwrap(),
                50,
                "frozen while CYCCNTENA=0"
            );
            // Re-enable; counting resumes from the frozen value.
            dwt.sync_to(1000);
            dwt.write(DWT_CTRL, 0x01).unwrap();
            clock.publish(1005);
            assert_eq!(dwt.read_u32(DWT_CYCCNT).unwrap(), 55);
        }

        #[test]
        fn software_write_reanchors_and_advances_from_written_value() {
            let (mut dwt, clock) = enabled_scheduler();
            clock.publish(100);
            assert_eq!(dwt.read_u32(DWT_CYCCNT).unwrap(), 100);
            // Reset CYCCNT to 0 (bus syncs to 100 first).
            dwt.sync_to(100);
            dwt.write(DWT_CYCCNT, 0).unwrap();
            dwt.write(DWT_CYCCNT + 1, 0).unwrap();
            dwt.write(DWT_CYCCNT + 2, 0).unwrap();
            dwt.write(DWT_CYCCNT + 3, 0).unwrap();
            clock.publish(130);
            assert_eq!(
                dwt.read_u32(DWT_CYCCNT).unwrap(),
                30,
                "counts from the written value"
            );
        }

        #[test]
        fn peek_is_side_effect_free() {
            let (dwt, clock) = enabled_scheduler();
            clock.publish(42);
            assert_eq!(dwt.peek(DWT_CYCCNT).unwrap(), 42);
            // Peek must not have advanced the anchor: a later real read at the
            // same clock still returns the same value.
            assert_eq!(dwt.read_u32(DWT_CYCCNT).unwrap(), 42);
        }

        #[test]
        fn wraps_modulo_2_32_like_the_walk() {
            let clock = CycleClock::default();
            let mut dwt = Dwt::new();
            dwt.attach_cycle_clock(clock.clone());
            dwt.write(DWT_CTRL, 0x01).unwrap();
            // Preset near the 32-bit boundary.
            dwt.sync_to(0);
            dwt.write(DWT_CYCCNT, 0xFF).unwrap();
            dwt.write(DWT_CYCCNT + 1, 0xFF).unwrap();
            dwt.write(DWT_CYCCNT + 2, 0xFF).unwrap();
            dwt.write(DWT_CYCCNT + 3, 0xFF).unwrap(); // 0xFFFF_FFFF
            clock.publish(5);
            assert_eq!(
                dwt.read_u32(DWT_CYCCNT).unwrap(),
                4,
                "0xFFFF_FFFF + 5 wraps to 4"
            );
        }
    }
}
