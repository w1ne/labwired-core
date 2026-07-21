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
//! # Cancellation contract
//!
//! There is NO scheduler-side cancel API. An event, once queued, is always
//! delivered to its peripheral at its deadline. Superseding a schedule is a
//! PERIPHERAL-side concern, and the platform relies on three cooperating
//! layers. A peripheral author must implement layer 2 or 3; layer 1 is free.
//!
//! 1. **Identical-wake dedup (scheduler, structural).** [`Self::schedule`]
//!    drops a wake byte-identical to one already queued (see the `queued`
//!    field). This is what bounds the heap for level-triggered peripherals
//!    that re-arm the same wake on every MMIO poll.
//! 2. **In-flight singleton guard (peripheral, most common).** A bool that
//!    refuses to arm a second event while one is live, cleared in `on_event`.
//!    Bounds the peripheral to one live event by construction.
//! 3. **Arming token (peripheral, for reconfigurable timers).** A counter
//!    bumped on every re-arm and carried in `event_token`; `on_event` returns
//!    early when `event_token` does not match the current value, so a
//!    superseded chain dies on arrival rather than being cancelled.
//!
//! Which mechanism each scheduler-participating peripheral uses:
//!
//! | Peripheral | Mechanism |
//! |---|---|
//! | `timer` | arming token (`arm_seq`) |
//! | `systick` | arming token (`arm_seq`) |
//! | `esp32c3::ledc` | arming token (`arm_seq`) |
//! | `esp32c3::i2c` | singleton (`scheduled`) + delta re-sync |
//! | `i2c` (Kinetis) | singleton (`chain_live`) |
//! | `uart` | singleton (`scheduled`) |
//! | `spi` | singleton (`scheduled`) + early-wakeup re-anchor |
//! | `scb` | singleton (`drain_chain_armed`) |
//! | `dma` | singleton per channel (`chain_live`); token is a channel index |
//! | `esp32s3::systimer` | none of its own — relies on layer 1 dedup |
//!
//! # Residual risk
//!
//! The dedup key includes the deadline. Identical re-arms at the SAME
//! deadline collapse — that is the SYSTIMER polling case and why layer 1
//! works. A peripheral that re-arms at a DIFFERENT (e.g. nearer) deadline
//! each time leaves the older entries resident until they fire; they are
//! discarded on arrival by layer 2/3, not on re-arm. Heap residency is
//! therefore bounded by the number of DISTINCT in-flight reconfigurations,
//! not unbounded — but it is not O(1) either. [`SchedulerStats`] and the
//! `debug_assert` in [`Self::schedule`] exist to catch a peripheral whose
//! distinct-reconfiguration count grows without bound; see
//! [`MAX_LIVE_EVENTS_PER_PERIPHERAL`].

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

pub type SimCycle = u64;

/// Reserved `peripheral_idx` for bus-subsystem pseudo-peripherals that are NOT
/// entries in `SystemBus::peripherals` — currently the HC-SR04 echo-edge
/// scheduler (`SystemBus::hcsr04`). Events tagged with this idx are dispatched
/// by `Machine::drain_scheduler_events` to a dedicated bus handler rather than
/// `peripherals[idx].on_event`, and are exempt from the per-peripheral live
/// event ceiling (the idx is a sentinel, not a real peripheral slot).
pub const SUBSYSTEM_PERIPHERAL_IDX: u32 = u32::MAX;

/// Ceiling on simultaneously-live events for a single `peripheral_idx`.
///
/// Every mechanism in the cancellation contract bounds a peripheral to a
/// SMALL constant number of in-flight events: layer 2 bounds it to 1 (per
/// channel for `dma`, whose 7 channels share one idx), layer 3 to the number
/// of distinct in-flight reconfigurations, layer 1 to the number of distinct
/// deadlines. Exceeding this ceiling means a peripheral is re-arming at
/// ever-changing deadlines without superseding its old ones — the unbounded
/// heap growth that degrades a run to O(cycles²).
///
/// Chosen as 8: comfortably above `dma`'s 7 channels (the legitimate maximum
/// across the peripheral set) and far below any pathological growth.
pub const MAX_LIVE_EVENTS_PER_PERIPHERAL: u32 = 8;

#[derive(Debug, Default, Clone)]
pub struct SchedulerStats {
    /// Count of `schedule()` calls in release mode whose `deadline < now`
    /// was clamped to `now`. Debug mode panics via `debug_assert!`.
    pub past_schedule_clamps: u64,
    /// High-water mark of simultaneously-live events held by any single
    /// `peripheral_idx`. Maintained in release builds too (one `HashMap`
    /// update per schedule/drain, alongside the dedup index that is already
    /// on that path). Compare against [`MAX_LIVE_EVENTS_PER_PERIPHERAL`].
    pub max_live_events_per_peripheral: u32,
    /// Count of `schedule()` calls that pushed a peripheral's live event count
    /// above [`MAX_LIVE_EVENTS_PER_PERIPHERAL`]. Non-zero means a peripheral
    /// is leaking wakes; debug builds panic via `debug_assert!` instead.
    pub live_event_ceiling_trips: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ScheduledEvent {
    pub deadline: SimCycle,
    pub event_id: u64,
    pub peripheral_idx: u32,
    pub event_token: u32,
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
    /// `(peripheral_idx, event_token, deadline)`. Kept in exact sync
    /// with `heap`: a key is present here iff a live heap entry with that key
    /// exists. Lets `schedule` reject a byte-for-byte duplicate in O(1).
    ///
    /// Level-triggered peripherals re-arm the *same* wake on every MMIO poll.
    /// The ESP32 SYSTIMER is the pathological case: Arduino `millis()`/`micros()`
    /// polls it every loop iteration (an UPDATE-write that runs the scheduler
    /// harvest), each poll re-emitting the identical alarm wake at the same
    /// deadline. Nothing supersedes them on re-arm, so they piled into `heap`
    /// without bound — every per-batch `next_event_deadline` / `drain_due` /
    /// push-pop then cost O(heap), degrading a run to O(cycles²). Collapsing
    /// byte-identical duplicates keeps `heap` bounded while preserving delivery:
    /// a genuinely distinct wake (any different key component — most importantly
    /// a different deadline, e.g. the initial bootstrap arm vs a write-path arm,
    /// or a period rollover) is still enqueued and still fires at its exact
    /// cycle. Only exact duplicates of an already-queued wake are dropped.
    queued: HashSet<(u32, u32, SimCycle)>,
    /// Live event count per `peripheral_idx`, kept in lockstep with `heap`.
    /// Backs the [`MAX_LIVE_EVENTS_PER_PERIPHERAL`] invariant. An idx is
    /// absent rather than zero once it drains to nothing.
    live_per_peripheral: HashMap<u32, u32>,
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
    pub fn schedule(&mut self, deadline: SimCycle, peripheral_idx: u32, event_token: u32) -> u64 {
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
        if !self.queued.insert((peripheral_idx, event_token, clamped)) {
            // Already queued — return the id the caller ignores anyway.
            return self.next_event_id;
        }
        // Track live events per peripheral and enforce the residency ceiling.
        // A peripheral past the ceiling is re-arming at ever-changing deadlines
        // without superseding its old wakes — the #570 unbounded-heap class.
        if peripheral_idx != SUBSYSTEM_PERIPHERAL_IDX {
            let live = self.live_per_peripheral.entry(peripheral_idx).or_insert(0);
            *live += 1;
            let live = *live;
            if live > self.stats.max_live_events_per_peripheral {
                self.stats.max_live_events_per_peripheral = live;
            }
            if live > MAX_LIVE_EVENTS_PER_PERIPHERAL {
                self.stats.live_event_ceiling_trips += 1;
                debug_assert!(
                    false,
                    "peripheral {} holds {} live events (ceiling {}): it re-arms \
                     without superseding prior wakes — see the cancellation \
                     contract in this module's docs",
                    peripheral_idx, live, MAX_LIVE_EVENTS_PER_PERIPHERAL
                );
            }
        }
        let event_id = self.next_event_id;
        self.next_event_id += 1;
        self.heap.push(Reverse(ScheduledEvent {
            deadline: clamped,
            event_id,
            peripheral_idx,
            event_token,
        }));
        event_id
    }

    /// Earliest deadline currently scheduled, or `None` if nothing is queued.
    /// Does not mutate the heap.
    ///
    /// Hot path: `BinaryHeap<Reverse<_>>` peeks the minimum deadline in O(1).
    /// Every queued event is live (there is no scheduler-side cancel — see the
    /// cancellation contract in this module's docs), so the peek is the answer.
    pub fn next_event_deadline(&self) -> Option<SimCycle> {
        let Reverse(top) = self.heap.peek()?;
        Some(top.deadline)
    }

    /// Pop all events whose deadline is `<= now`, in `(deadline asc,
    /// event_id asc)` order. Every popped event is delivered: a superseded
    /// wake is discarded by the PERIPHERAL on arrival, not here.
    pub fn drain_due(&mut self) -> Vec<ScheduledEvent> {
        let mut out = Vec::new();
        self.drain_due_into(&mut out);
        out
    }

    /// Push-based twin of [`Self::drain_due`]: append the due events into a
    /// CALLER-OWNED buffer instead of returning a freshly-allocated `Vec`. The
    /// per-batch drain (`Machine::drain_scheduler_events`) passes retained
    /// scratch so the steady-state SYSTIMER tick — which drains at least one
    /// event nearly every batch — no longer allocates. `out` is cleared first.
    pub fn drain_due_into(&mut self, out: &mut Vec<ScheduledEvent>) {
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
            self.queued
                .remove(&(ev.peripheral_idx, ev.event_token, ev.deadline));
            if ev.peripheral_idx != SUBSYSTEM_PERIPHERAL_IDX {
                if let Some(live) = self.live_per_peripheral.get_mut(&ev.peripheral_idx) {
                    *live -= 1;
                    if *live == 0 {
                        self.live_per_peripheral.remove(&ev.peripheral_idx);
                    }
                }
            }
            out.push(ev);
        }
    }

    /// True once no events remain queued. Lets the per-step drain skip the
    /// heap entirely when nothing is scheduled.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

#[cfg(test)]
mod dedup_tests {
    use super::*;

    #[test]
    fn identical_wakes_are_deduped_but_the_heap_still_fires_once() {
        // Regression for the O(n²) slowdown: a level-triggered peripheral
        // (e.g. the SYSTIMER polled by Arduino millis()) re-arms the identical
        // wake on every poll. Those byte-for-byte duplicates must NOT pile up.
        let mut s = EventScheduler::new();
        for _ in 0..1000 {
            s.schedule(100, 3, 0);
        }
        assert_eq!(
            s.heap.len(),
            1,
            "identical wakes must collapse to one entry"
        );

        s.advance_to(100);
        let due = s.drain_due();
        assert_eq!(due.len(), 1, "the single retained wake fires exactly once");
        assert!(s.is_empty());
        // After it fires, the same key may be armed again.
        s.schedule(200, 3, 0);
        assert_eq!(s.heap.len(), 1);
    }

    #[test]
    fn distinct_wakes_are_all_kept() {
        // Only EXACT duplicates collapse. A different deadline (the bootstrap-vs
        // write-path +1, or a period rollover), token, or peripheral is a
        // distinct wake that must still be enqueued and fire at its cycle.
        let mut s = EventScheduler::new();
        s.schedule(100, 3, 0); // baseline
        s.schedule(101, 3, 0); // different deadline
        s.schedule(100, 3, 1); // different token
        s.schedule(100, 4, 0); // different peripheral
        s.schedule(100, 3, 0); // exact dup of baseline → dropped
        assert_eq!(
            s.heap.len(),
            4,
            "four distinct wakes, one duplicate dropped"
        );
    }

    #[test]
    fn requeue_after_drain_is_allowed() {
        // The dedup index must stay in lockstep with the heap: once an event is
        // drained, an identical wake can be armed again (steady-state re-arm).
        let mut s = EventScheduler::new();
        s.schedule(10, 2, 0);
        s.advance_to(10);
        assert_eq!(s.drain_due().len(), 1);
        // Same key again at a later deadline: not suppressed.
        s.schedule(20, 2, 0);
        s.advance_to(20);
        assert_eq!(s.drain_due().len(), 1);
    }
}

#[cfg(test)]
mod residency_invariant_tests {
    use super::*;

    /// A well-behaved level-triggered peripheral: re-arms the SAME wake on
    /// every poll (the SYSTIMER `millis()` pattern). Layer-1 dedup collapses
    /// them, so the heap stays at one entry and the ceiling is never neared.
    #[test]
    fn repeated_identical_rearm_stays_bounded() {
        let mut s = EventScheduler::new();
        for _ in 0..100_000 {
            s.schedule(5_000, 3, 0);
        }
        assert_eq!(s.heap.len(), 1, "identical re-arms must not accumulate");
        assert_eq!(s.stats().max_live_events_per_peripheral, 1);
        assert_eq!(s.stats().live_event_ceiling_trips, 0);
    }

    /// A well-behaved peripheral re-arming at MOVING deadlines but superseding
    /// each prior wake as it fires (drain between arms) also stays bounded.
    #[test]
    fn moving_deadline_rearm_with_drain_stays_bounded() {
        let mut s = EventScheduler::new();
        for cycle in 1..10_000u64 {
            s.schedule(cycle, 3, 0);
            s.advance_to(cycle);
            s.drain_due();
        }
        assert!(s.is_empty());
        assert_eq!(s.stats().max_live_events_per_peripheral, 1);
        assert_eq!(s.stats().live_event_ceiling_trips, 0);
    }

    /// The invariant must BITE. A peripheral that re-arms at ever-nearer
    /// deadlines without ever superseding its prior wakes is exactly the #570
    /// unbounded-growth class. In debug builds `schedule` panics via
    /// `debug_assert!`; this asserts that panic actually fires.
    #[test]
    #[should_panic(expected = "live events")]
    #[cfg(debug_assertions)]
    fn unbounded_distinct_rearm_trips_the_ceiling() {
        let mut s = EventScheduler::new();
        // Distinct deadlines → dedup cannot help; nothing drains them.
        for cycle in (1..=1_000u64).rev() {
            s.schedule(cycle, 3, 0);
        }
    }

    /// Release builds cannot panic, so the same pathology must be observable
    /// as a counter. Exercised via the same distinct-deadline re-arm loop.
    #[test]
    #[cfg(not(debug_assertions))]
    fn unbounded_distinct_rearm_is_counted_in_release() {
        let mut s = EventScheduler::new();
        for cycle in (1..=1_000u64).rev() {
            s.schedule(cycle, 3, 0);
        }
        assert!(
            s.stats().live_event_ceiling_trips > 0,
            "release builds must count ceiling breaches"
        );
        assert!(s.stats().max_live_events_per_peripheral > MAX_LIVE_EVENTS_PER_PERIPHERAL);
    }

    /// The subsystem sentinel idx is not a real peripheral slot and must be
    /// exempt: HC-SR04 echo edges legitimately queue without a slot to bound.
    #[test]
    fn subsystem_pseudo_peripheral_is_exempt_from_the_ceiling() {
        let mut s = EventScheduler::new();
        for cycle in 1..=1_000u64 {
            s.schedule(cycle, SUBSYSTEM_PERIPHERAL_IDX, 0);
        }
        assert_eq!(s.stats().live_event_ceiling_trips, 0);
        assert_eq!(s.stats().max_live_events_per_peripheral, 0);
    }
}
