// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `EventScheduler`: O(log P) min-heap of upcoming peripheral wakeups.
//!
//! Quantum: `SimCycle = u64` is CPU-CCOUNT-equivalent. Floor-truncate at
//! clock-domain conversion. Peripherals model sub-cycle phase internally
//! and schedule at the CPU-cycle boundary that matters.
//!
//! Ordering is `(deadline asc, event_id asc)`. `peripheral_idx` and
//! `event_token` never participate, so reordering peripherals on the bus
//! never changes event order.
//!
//! Reentrancy: an `on_event` handler may call `schedule()` mid-drain. The
//! new event gets a higher `event_id`; if its deadline equals `now`, it
//! lands at the end of the current drain batch via the same ordering rule.
//!
//! Cancel: lazy via per-peripheral `generation: u32`. `cancel_all_for`
//! bumps the generation on the peripheral side; `drain_due` drops events
//! whose generation snapshot no longer matches.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

pub type SimCycle = u64;

/// Reserved `peripheral_idx` for bus-subsystem pseudo-peripherals that are NOT
/// entries in `SystemBus::peripherals` and therefore have no generation slot —
/// currently the HC-SR04 echo-edge scheduler (`SystemBus::hcsr04`). Events
/// tagged with this idx are never generation-stale (there is nothing to cancel
/// them against) and are dispatched by `Machine::drain_scheduler_events` to a
/// dedicated bus handler rather than `peripherals[idx].on_event`.
pub const SUBSYSTEM_PERIPHERAL_IDX: u32 = u32::MAX;

#[derive(Debug, Default, Clone)]
pub struct SchedulerStats {
    /// Count of `schedule()` calls in release mode whose `deadline < now`
    /// was clamped to `now`. Debug mode panics via `debug_assert!`.
    pub past_schedule_clamps: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ScheduledEvent {
    pub deadline: SimCycle,
    pub event_id: u64,
    pub peripheral_idx: u32,
    pub event_token: u32,
    pub generation: u32,
}

impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deadline
            .cmp(&other.deadline)
            .then_with(|| self.event_id.cmp(&other.event_id))
    }
}

impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Default)]
pub struct EventScheduler {
    now: SimCycle,
    heap: BinaryHeap<Reverse<ScheduledEvent>>,
    next_event_id: u64,
    stats: SchedulerStats,
    /// Membership index for identical-event de-duplication, keyed by
    /// `(peripheral_idx, event_token, generation, deadline)`. Kept in exact sync
    /// with `heap`: a key is present here iff a live heap entry with that key
    /// exists. Lets `schedule` reject a byte-for-byte duplicate in O(1).
    ///
    /// Level-triggered peripherals re-arm the *same* wake on every MMIO poll.
    /// The ESP32 SYSTIMER is the pathological case: Arduino `millis()`/`micros()`
    /// polls it every loop iteration (an UPDATE-write that runs the scheduler
    /// harvest), each poll re-emitting the identical alarm wake at the same
    /// deadline. Nothing generation-cancels them, so they piled into `heap`
    /// without bound — every per-batch `next_event_deadline` / `drain_due` /
    /// push-pop then cost O(heap), degrading a run to O(cycles²). Collapsing
    /// byte-identical duplicates keeps `heap` bounded while preserving delivery:
    /// a genuinely distinct wake (any different key component — most importantly
    /// a different deadline, e.g. the initial bootstrap arm vs a write-path arm,
    /// or a period rollover) is still enqueued and still fires at its exact
    /// cycle. Only exact duplicates of an already-queued wake are dropped.
    queued: HashSet<(u32, u32, u32, SimCycle)>,
}

impl EventScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn now(&self) -> SimCycle {
        self.now
    }

    pub fn advance_to(&mut self, target: SimCycle) {
        if target > self.now {
            self.now = target;
        }
    }

    pub fn stats(&self) -> &SchedulerStats {
        &self.stats
    }

    /// Schedule an opaque token to fire at `deadline` for peripheral `peripheral_idx`.
    /// The peripheral interprets `event_token` however it wishes; the scheduler
    /// has zero knowledge of token semantics.
    ///
    /// `debug_assert!(deadline >= now)`. In release builds past deadlines are
    /// clamped to `now` and `stats.past_schedule_clamps` is incremented.
    pub fn schedule(
        &mut self,
        deadline: SimCycle,
        peripheral_idx: u32,
        event_token: u32,
        generation: u32,
    ) -> u64 {
        debug_assert!(
            deadline >= self.now,
            "schedule deadline {} < now {}",
            deadline,
            self.now
        );
        let clamped = if deadline < self.now {
            self.stats.past_schedule_clamps += 1;
            self.now
        } else {
            deadline
        };
        // Reject a byte-for-byte duplicate of an event already queued. A
        // level-triggered peripheral re-arming the identical wake every poll
        // would otherwise pile redundant entries into `heap` unbounded (see the
        // `queued` field). The retained entry fires at the identical cycle, so
        // delivery is unchanged; only the redundant copies are dropped.
        if !self
            .queued
            .insert((peripheral_idx, event_token, generation, clamped))
        {
            // Already queued — return the id the caller ignores anyway.
            return self.next_event_id;
        }
        let event_id = self.next_event_id;
        self.next_event_id += 1;
        self.heap.push(Reverse(ScheduledEvent {
            deadline: clamped,
            event_id,
            peripheral_idx,
            event_token,
            generation,
        }));
        event_id
    }

    /// Earliest deadline currently scheduled, skipping events whose generation
    /// no longer matches the peripheral's current generation (stale, lazily
    /// cancelled). Returns `None` if the heap is empty or only contains stale
    /// entries. Does not mutate the heap.
    ///
    /// Hot path: `BinaryHeap<Reverse<_>>` peeks the minimum deadline in O(1).
    /// When that top entry is live (the common case — lazy cancel is rare),
    /// return it immediately. Only if the top is stale do we fall back to a
    /// full scan for the next live deadline.
    pub fn next_event_deadline(&self, peripheral_generations: &[u32]) -> Option<SimCycle> {
        let Reverse(top) = self.heap.peek()?;
        if !Self::is_stale(top, peripheral_generations) {
            return Some(top.deadline);
        }
        // Top is stale: find the earliest live entry (iteration order is not
        // sorted — must scan).
        let mut best: Option<SimCycle> = None;
        for Reverse(ev) in self.heap.iter() {
            if Self::is_stale(ev, peripheral_generations) {
                continue;
            }
            best = Some(match best {
                Some(b) => b.min(ev.deadline),
                None => ev.deadline,
            });
        }
        best
    }

    /// Pop all events whose deadline is `<= now` AND whose generation matches
    /// the peripheral's current generation. Stale entries (mismatched
    /// generation) are popped and silently discarded. Returned in
    /// `(deadline asc, event_id asc)` order.
    pub fn drain_due(&mut self, peripheral_generations: &[u32]) -> Vec<ScheduledEvent> {
        let mut out = Vec::new();
        self.drain_due_into(peripheral_generations, &mut out);
        out
    }

    /// Push-based twin of [`Self::drain_due`]: append the due events into a
    /// CALLER-OWNED buffer instead of returning a freshly-allocated `Vec`. The
    /// per-batch drain (`Machine::drain_scheduler_events`) passes retained
    /// scratch so the steady-state SYSTIMER tick — which drains at least one
    /// event nearly every batch — no longer allocates. `out` is cleared first.
    pub fn drain_due_into(
        &mut self,
        peripheral_generations: &[u32],
        out: &mut Vec<ScheduledEvent>,
    ) {
        out.clear();
        // Nothing due: return without touching the heap loop.
        match self.heap.peek() {
            None => return,
            Some(Reverse(top)) if top.deadline > self.now => return,
            _ => {}
        }
        while let Some(Reverse(top)) = self.heap.peek() {
            if top.deadline > self.now {
                break;
            }
            let Reverse(ev) = self.heap.pop().unwrap();
            // Keep the dedup index in lockstep with the heap: this key leaves the
            // heap now, so an identical wake may be re-armed after it fires.
            self.queued.remove(&(
                ev.peripheral_idx,
                ev.event_token,
                ev.generation,
                ev.deadline,
            ));
            if Self::is_stale(&ev, peripheral_generations) {
                continue;
            }
            out.push(ev);
        }
    }

    /// True once no events remain queued. Lets the per-step drain skip its
    /// generation snapshot + heap scan entirely when nothing is scheduled.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    fn is_stale(ev: &ScheduledEvent, peripheral_generations: &[u32]) -> bool {
        // Bus-subsystem pseudo-peripherals (e.g. HC-SR04) have no generation
        // slot in `peripheral_generations`; they are never generation-cancelled.
        if ev.peripheral_idx == SUBSYSTEM_PERIPHERAL_IDX {
            return false;
        }
        match peripheral_generations.get(ev.peripheral_idx as usize) {
            Some(cur) => *cur != ev.generation,
            // Out-of-range idx (peripheral removed) → treat as stale.
            None => true,
        }
    }
}

#[cfg(test)]
mod dedup_tests {
    use super::*;

    const GENS: &[u32] = &[0, 0, 0, 0];

    #[test]
    fn identical_wakes_are_deduped_but_the_heap_still_fires_once() {
        // Regression for the O(n²) slowdown: a level-triggered peripheral
        // (e.g. the SYSTIMER polled by Arduino millis()) re-arms the identical
        // wake on every poll. Those byte-for-byte duplicates must NOT pile up.
        let mut s = EventScheduler::new();
        for _ in 0..1000 {
            s.schedule(100, 3, 0, 0);
        }
        assert_eq!(
            s.heap.len(),
            1,
            "identical wakes must collapse to one entry"
        );

        s.advance_to(100);
        let due = s.drain_due(GENS);
        assert_eq!(due.len(), 1, "the single retained wake fires exactly once");
        assert!(s.is_empty());
        // After it fires, the same key may be armed again.
        s.schedule(200, 3, 0, 0);
        assert_eq!(s.heap.len(), 1);
    }

    #[test]
    fn distinct_wakes_are_all_kept() {
        // Only EXACT duplicates collapse. A different deadline (the bootstrap-vs
        // write-path +1, or a period rollover), token, peripheral, or generation
        // is a distinct wake that must still be enqueued and fire at its cycle.
        let mut s = EventScheduler::new();
        s.schedule(100, 3, 0, 0); // baseline
        s.schedule(101, 3, 0, 0); // different deadline
        s.schedule(100, 3, 1, 0); // different token
        s.schedule(100, 4, 0, 0); // different peripheral
        s.schedule(100, 3, 0, 7); // different generation
        s.schedule(100, 3, 0, 0); // exact dup of baseline → dropped
        assert_eq!(
            s.heap.len(),
            5,
            "five distinct wakes, one duplicate dropped"
        );
    }

    #[test]
    fn requeue_after_drain_is_allowed() {
        // The dedup index must stay in lockstep with the heap: once an event is
        // drained, an identical wake can be armed again (steady-state re-arm).
        let mut s = EventScheduler::new();
        s.schedule(10, 2, 0, 0);
        s.advance_to(10);
        assert_eq!(s.drain_due(GENS).len(), 1);
        // Same key again at a later deadline: not suppressed.
        s.schedule(20, 2, 0, 0);
        s.advance_to(20);
        assert_eq!(s.drain_due(GENS).len(), 1);
    }
}
