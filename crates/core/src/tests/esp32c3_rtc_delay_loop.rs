// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential gate for the ESP32-C3 RTC main-timer scheduler migration
//! (`perf/c3-walk-free`, Task B): the EXACT scenario the old
//! `uses_scheduler() == false` comment feared — *"firmware delay loops observe
//! stale time and spin forever"* — proven terminating and byte-identical to
//! the legacy walk.
//!
//! The firmware is the IDF `rtc_time_get` poll shape: write `TIME_UPDATE`
//! (latch), read `TIME0`, loop until the deadline passes. Run on the legacy
//! per-cycle walk (reference — `force_legacy_walk()` detaches the cycle
//! clock) and on the scheduler path (counter advanced lazily from the
//! bus-published `CycleClock`), at tick interval 1 and 64:
//!
//! * both paths TERMINATE (no stale-spin), and
//! * they terminate at the SAME cycle at interval 1 AND 64, and
//! * the scheduler's counter READ is now cycle-EXACT and thus tick-interval
//!   INDEPENDENT: `sched(1) == sched(64)` for both the termination cycle and
//!   the final counter value.
//!
//! Fidelity note (event-scheduler exact-cycle clock): a lazily-derived counter
//! read is anchored to `batch_start + retired` (the CPU republishes the exact
//! cycle before each interpreted instruction — see `Cpu::step_batch`), so a
//! mid-batch read returns the SAME value at any tick interval. On real silicon
//! the RTC counter advances every cycle and a read returns the live value, so
//! this exact read is the faithful behaviour. The LEGACY WALK, by contrast,
//! only refreshes the counter at tick boundaries, so at interval > 1 its read
//! is QUANTISED to the tick grid (it reports the last grid value, up to one
//! interval stale). The two therefore agree at interval 1 (grid == every
//! cycle) but the walk's final read lags the scheduler's exact read by up to
//! one interval at interval 64 — the scheduler is the more faithful path, so
//! this test pins scheduler interval-independence rather than equality to the
//! coarser walk read at interval > 1. The termination CYCLE still matches the
//! walk at both intervals (the deadline is crossed at the same cycle).

#![cfg(all(test, feature = "event-scheduler"))]

use crate::cpu::RiscV;
use crate::peripherals::esp32c3::rtc_timer::Esp32c3RtcTimer;
use crate::{Bus, Cpu, DebugControl, Machine};

const RTC_BASE: u64 = 0x6000_8000;
/// The delay-loop deadline in RTC counter ticks (== CPU cycles in this model).
const DEADLINE: u32 = 0x2000; // 8192
/// PC of the terminal `jal x0, 0` self-loop.
const DONE_PC: u32 = 0x18;

/// Build a RISC-V machine whose firmware busy-polls the RTC counter:
///
/// ```text
///   lui  x1, 0x60008        ; x1 = RTC_CNTL base
///   lui  x2, 0x80000        ; x2 = TIME_UPDATE latch bit (bit31)
///   lui  x4, 0x2            ; x4 = deadline (0x2000 counter ticks)
/// loop:
///   sw   x2, 0x0C(x1)       ; TIME_UPDATE ← latch the live counter
///   lw   x3, 0x10(x1)       ; TIME0      → latched counter (low word)
///   bltu x3, x4, loop       ; spin until the deadline passes
/// done:
///   jal  x0, 0              ; park
/// ```
fn build_machine(tick_interval: u32, legacy_walk: bool) -> Machine<RiscV> {
    let mut bus = crate::bus::SystemBus::new();
    bus.flash.data = vec![0; 0x100].into();
    // add_peripheral attaches the bus cycle clock → scheduler mode under the
    // event-scheduler feature. `force_legacy_walk` detaches it, restoring the
    // reference per-cycle-walk drive.
    bus.add_peripheral(
        "rtc_cntl_timer",
        RTC_BASE,
        0x100,
        None,
        Box::new(Esp32c3RtcTimer::new()),
    );
    if legacy_walk {
        let idx = bus.find_peripheral_index_by_name("rtc_cntl_timer").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32c3RtcTimer>()
            .unwrap()
            .force_legacy_walk();
    }

    bus.write_u32(0x00, 0x6000_80B7).unwrap(); // lui x1, 0x60008
    bus.write_u32(0x04, 0x8000_0137).unwrap(); // lui x2, 0x80000
    bus.write_u32(0x08, 0x0000_2237).unwrap(); // lui x4, 0x2
    bus.write_u32(0x0C, 0x0020_A623).unwrap(); // sw  x2, 12(x1)
    bus.write_u32(0x10, 0x0100_A183).unwrap(); // lw  x3, 16(x1)
    bus.write_u32(0x14, 0xFE41_ECE3).unwrap(); // bltu x3, x4, -8
    bus.write_u32(0x18, 0x0000_006F).unwrap(); // jal x0, 0 (done)

    let mut cpu = RiscV::new();
    cpu.pc = 0x0;
    cpu.mtimecmp = u64::MAX;
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    machine
}

/// Run until the firmware parks at `done` (or a generous step budget runs
/// out) and return `(terminated, total_cycles, final_counter_read)`.
fn run_delay_loop(mut machine: Machine<RiscV>) -> (bool, u64, u32) {
    // The loop needs ~DEADLINE cycles to satisfy its deadline; 40x headroom.
    const BUDGET: u64 = (DEADLINE as u64) * 40;
    while machine.total_cycles < BUDGET {
        machine.run(Some(1_000)).unwrap();
        if machine.cpu.get_pc() == DONE_PC {
            return (true, machine.total_cycles, machine.cpu.get_register(3));
        }
    }
    (false, machine.total_cycles, machine.cpu.get_register(3))
}

/// THE gate: the busy-poll delay loop terminates on both drive modes at
/// interval 1 AND 64 (no stale-spin); the scheduler crosses the deadline at the
/// SAME cycle as the legacy walk; the scheduler is byte-identical to the walk at
/// interval 1; and — the property the exact-cycle clock buys — the scheduler is
/// tick-interval INDEPENDENT (its exact counter read at interval 64 equals its
/// read at interval 1), whereas the walk's read quantises to the tick grid.
#[test]
fn rtc_delay_loop_terminates_and_matches_walk() {
    let mut sched_by_interval: Vec<(u64, u32)> = Vec::new();
    for tick_interval in [1u32, 64] {
        let (walk_done, walk_cycles, walk_cnt) = run_delay_loop(build_machine(tick_interval, true));
        let (sched_done, sched_cycles, sched_cnt) =
            run_delay_loop(build_machine(tick_interval, false));

        assert!(
            walk_done,
            "interval {tick_interval}: legacy-walk delay loop must terminate"
        );
        assert!(
            sched_done,
            "interval {tick_interval}: scheduler delay loop must terminate — \
             this is the stale-read spin-forever case the migration must prevent"
        );
        assert!(
            sched_cnt >= DEADLINE,
            "interval {tick_interval}: loop exited before the RTC deadline \
             (read {sched_cnt:#x} < {DEADLINE:#x})"
        );
        // Termination CYCLE is byte-identical to the walk at every interval (the
        // deadline is crossed at the same cycle; only the reported read value
        // quantises under the walk at interval > 1).
        assert_eq!(
            walk_cycles, sched_cycles,
            "interval {tick_interval}: scheduler must cross the RTC deadline at \
             the same cycle as the legacy walk"
        );
        if tick_interval == 1 {
            // At interval 1 the walk grid IS every cycle, so the reads match too.
            assert_eq!(
                walk_cnt, sched_cnt,
                "interval 1: scheduler counter read must equal the walk (grid == \
                 every cycle)"
            );
        }
        sched_by_interval.push((sched_cycles, sched_cnt));
    }

    // The payoff: the scheduler's exact-cycle read makes the WHOLE observation
    // (termination cycle + final counter value) tick-interval INDEPENDENT.
    assert_eq!(
        sched_by_interval[0], sched_by_interval[1],
        "scheduler-driven RTC must be tick-interval independent \
         (interval-1 {:?} != interval-64 {:?})",
        sched_by_interval[0], sched_by_interval[1]
    );
}

/// The scheduler path must actually be scheduler-driven (walk-independent) and
/// the reference must actually be on the walk — guard the knob itself so the
/// differential above can't silently compare like against like.
#[test]
fn differential_knob_selects_distinct_paths() {
    let sched = build_machine(1, false);
    let idx = sched
        .bus
        .find_peripheral_index_by_name("rtc_cntl_timer")
        .unwrap();
    assert!(sched.bus.peripherals[idx].dev.uses_scheduler());

    let walk = build_machine(1, true);
    let idx = walk
        .bus
        .find_peripheral_index_by_name("rtc_cntl_timer")
        .unwrap();
    assert!(!walk.bus.peripherals[idx].dev.uses_scheduler());
}
