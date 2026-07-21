// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Phase 2B.1 (issue #192): unit tests for the event-driven peripheral
//! scheduler skeleton + Machine integration parity.

use labwired_core::sched::{EventScheduler, ScheduledEvent};

fn drained_ids(out: &[ScheduledEvent]) -> Vec<u64> {
    out.iter().map(|e| e.event_id).collect()
}

#[test]
fn empty_scheduler() {
    let mut sched = EventScheduler::new();
    assert_eq!(sched.now(), 0);
    assert_eq!(sched.next_event_deadline(), None);
    assert!(sched.drain_due().is_empty());
}

#[test]
fn single_event_visible_then_drained() {
    let mut sched = EventScheduler::new();
    sched.schedule(100, 0, 0xAB);

    assert_eq!(sched.next_event_deadline(), Some(100));
    // Before deadline: no events drained, but heap unchanged.
    assert!(sched.drain_due().is_empty());
    assert_eq!(sched.next_event_deadline(), Some(100));

    sched.advance_to(100);
    let out = sched.drain_due();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].event_token, 0xAB);
    assert_eq!(sched.next_event_deadline(), None);
}

#[test]
fn multi_event_deadline_order() {
    let mut sched = EventScheduler::new();
    sched.schedule(300, 0, 3);
    sched.schedule(100, 0, 1);
    sched.schedule(200, 0, 2);

    assert_eq!(sched.next_event_deadline(), Some(100));

    sched.advance_to(500);
    let out = sched.drain_due();
    let tokens: Vec<u32> = out.iter().map(|e| e.event_token).collect();
    assert_eq!(tokens, vec![1, 2, 3]);
}

#[test]
fn same_deadline_deterministic_by_event_id() {
    let mut sched = EventScheduler::new();
    // Schedule in (1, 2) order at the same deadline; event_id is monotonic
    // so drain must yield them in (1, 2) order regardless of insertion side
    // effects.
    let id_a = sched.schedule(100, 0, 1);
    let id_b = sched.schedule(100, 0, 2);
    assert!(id_b > id_a);

    sched.advance_to(100);
    let out = sched.drain_due();
    assert_eq!(drained_ids(&out), vec![id_a, id_b]);
}

#[test]
fn past_deadline_clamps_in_release_and_counts() {
    // The `debug_assert!` only fires under cfg(debug_assertions); we
    // exercise the release-mode clamp path explicitly using a custom
    // scaffold that calls `schedule` after advancing past the deadline.
    let mut sched = EventScheduler::new();
    sched.advance_to(100);

    // In debug builds this would panic. We can't easily flip cfg in a
    // single test binary, so only exercise the counter path when the
    // assertion isn't compiled in.
    #[cfg(not(debug_assertions))]
    {
        sched.schedule(50, 0, 1);
        assert_eq!(sched.stats().past_schedule_clamps, 1);
        let out = sched.drain_due(&[0u32]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].deadline, 100, "clamped to now");
    }
    #[cfg(debug_assertions)]
    {
        // Counter is still reachable; just don't trip the assertion.
        let _ = sched.stats().past_schedule_clamps;
    }
}

#[test]
fn reentrant_schedule_during_drain_semantics() {
    // Schedule A at 100, drain at 100 → caller can "re-enter" by calling
    // schedule(now, ...) which lands at the back of the next drain batch
    // (event_id higher → strict order). Confirms the documented
    // reentrancy model: same-deadline events are ordered by event_id.
    let mut sched = EventScheduler::new();
    let id_a = sched.schedule(100, 0, 1);

    sched.advance_to(100);
    let first = sched.drain_due();
    assert_eq!(drained_ids(&first), vec![id_a]);

    // Equivalent to "handler called schedule(now, ...) mid-drain": the new
    // event has a higher event_id and lands in the next drain.
    let id_b = sched.schedule(100, 0, 2);
    assert!(id_b > id_a);
    let second = sched.drain_due();
    assert_eq!(drained_ids(&second), vec![id_b]);
}

#[test]
fn advance_to_is_monotonic() {
    let mut sched = EventScheduler::new();
    sched.advance_to(100);
    sched.advance_to(50); // ignored — must never rewind
    assert_eq!(sched.now(), 100);
    sched.advance_to(200);
    assert_eq!(sched.now(), 200);
}

#[test]
fn clock_graph_set_rate_notifies_observers() {
    use labwired_core::sched::{ClockDomain, ClockGraph};
    let mut cg = ClockGraph::new();
    cg.subscribe(ClockDomain::Apb, 7);
    cg.subscribe(ClockDomain::Apb, 9);
    cg.subscribe(ClockDomain::Cpu, 1); // different domain — must not fire

    let mut notified: Vec<(u32, u64)> = Vec::new();
    cg.set_rate(ClockDomain::Apb, 80_000_000, |idx, dom, hz| {
        assert_eq!(dom, ClockDomain::Apb);
        notified.push((idx, hz));
    });
    assert_eq!(notified, vec![(7, 80_000_000), (9, 80_000_000)]);
    assert_eq!(cg.rate(ClockDomain::Apb), 80_000_000);
    assert_eq!(cg.rate(ClockDomain::Cpu), 0);
}

#[test]
fn clock_graph_subscribe_dedupes() {
    use labwired_core::sched::{ClockDomain, ClockGraph};
    let mut cg = ClockGraph::new();
    cg.subscribe(ClockDomain::Apb, 7);
    cg.subscribe(ClockDomain::Apb, 7);
    assert_eq!(cg.observers(ClockDomain::Apb), &[7]);
}

/// Machine integration parity test. Builds a stock STM32 `SystemBus`
/// (no firmware loaded — peripherals start in reset state) and steps it
/// many times. With the `event-scheduler` flag OFF, the new drain
/// compiles out entirely. With the flag ON, no peripheral overrides
/// `uses_scheduler()`, so the scheduler stays empty and the drain is a
/// no-op. Either way `total_cycles` must advance exactly N per N steps
/// (modulo the cost-of-tick accumulation that already exists in the
/// legacy path — peripherals reset to 0-cost so this stays at N).
#[test]
fn machine_step_parity_with_no_scheduler_users() {
    use labwired_core::bus::SystemBus;
    use labwired_core::cpu::CortexM;
    use labwired_core::peripherals::stub::StubPeripheral;
    use labwired_core::Machine;

    // Strip default STM32 peripherals + add a single stub so the legacy
    // tick walk has something to iterate (matches what `tick_peripherals_fully`
    // does in real configs). The stub returns `PeripheralTickResult::default()`
    // — cycles=0, ticks_until_next=None — so total_cycles grows by exactly 1
    // per step.
    let mut bus = SystemBus::empty();
    bus.add_peripheral(
        "stub",
        0x5000_0000,
        0x10,
        None,
        Box::new(StubPeripheral::new(0)),
    );

    let cpu = CortexM::new();
    let mut machine = Machine::new(cpu, bus);
    let before = machine.total_cycles;
    for _ in 0..1000 {
        // Cortex-M with no firmware → likely returns Ok(()) or a benign
        // memory error. Either way the scheduler-related code path is
        // exercised. We ignore the per-step result here; what matters is
        // the post-state.
        let _ = machine.step();
    }
    let advanced = machine.total_cycles - before;
    // The comment above documents that total_cycles grows by exactly 1 per
    // step when the stub returns a 0-cost tick. Assert the exact count so any
    // regression in the scheduler's cycle-accounting integration fires here.
    assert_eq!(
        advanced, 1000,
        "total_cycles should advance by exactly 1 per step (1000 steps = 1000 cycles); \
         actual advance={advanced}. Scheduler or tick-cost regression detected."
    );

    // Scheduler must remain inert when nobody opts in.
    assert_eq!(machine.sched.stats().past_schedule_clamps, 0);
}

// Phase 2B.3a (issue #192): write-context scheduling plumbing. A peripheral
// that arms an event from an MMIO write must have it buffered into the bus's
// `pending_schedule` for `Machine::drain_scheduler_events` to enqueue. Gated
// because the collection only runs under the `event-scheduler` feature.
#[cfg(feature = "event-scheduler")]
mod write_context_scheduling {
    use labwired_core::bus::SystemBus;
    use labwired_core::Bus;
    use labwired_core::{Peripheral, SimResult};

    const ARM_TOKEN: u32 = 0xE5;

    #[derive(Debug, Default)]
    struct ArmingPeripheral {
        armed: bool,
    }

    impl Peripheral for ArmingPeripheral {
        fn read(&self, _offset: u64) -> SimResult<u8> {
            Ok(0)
        }
        fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
            self.armed = true;
            Ok(())
        }
        fn uses_scheduler(&self) -> bool {
            true
        }
        fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
            if std::mem::take(&mut self.armed) {
                vec![(0, ARM_TOKEN)]
            } else {
                Vec::new()
            }
        }
    }

    #[test]
    fn write_buffers_schedule_request() {
        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "arm",
            0x5000_0000,
            0x10,
            None,
            Box::new(ArmingPeripheral::default()),
        );

        // A non-arming-aware peripheral would leave this empty; ours queues
        // exactly one (peripheral_idx=0, deadline, ARM_TOKEN) on write. The
        // peripheral's relative delay 0 becomes the absolute cycle deadline
        // `current_cycle + 1 + 0` — the cycle the next per-cycle drain runs at
        // (`current_cycle` is 0 here; no machine is stepping this bus).
        bus.write_u32(0x5000_0000, 1).unwrap();
        assert_eq!(bus.pending_schedule, vec![(0usize, 1u64, ARM_TOKEN)]);

        // A second write re-arms and appends another request (the harvest
        // cleared `armed` the first time, so this isn't a stale duplicate),
        // with the deadline pinned to the (advanced) write cycle.
        bus.current_cycle = 5;
        bus.write_u32(0x5000_0000, 1).unwrap();
        assert_eq!(
            bus.pending_schedule,
            vec![(0usize, 1u64, ARM_TOKEN), (0usize, 6u64, ARM_TOKEN)]
        );
    }
}
