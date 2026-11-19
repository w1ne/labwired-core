// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The walk-differential harness, tested on itself.
//!
//! A differential oracle that cannot SEE a divergence is worse than no oracle:
//! it converts "nobody checked" into "the gate is green", which is exactly the
//! failure the harness exists to prevent. So before any migration leans on it,
//! prove each half:
//!
//! * it passes when the two runs really are identical,
//! * it FAILS when they differ — separately for architectural state and for
//!   the caller-supplied observables, because those travel different code
//!   paths through the comparison,
//! * and `assert_modes_differ` catches both directions of the vacuous pass.
//!
//! The last one is the important one. `assert_walk_differential` compares
//! whatever it is handed; if a migration is not in effect it compares the walk
//! against itself and reports success. That is a gate that ran and told you
//! nothing, and it is how eleven hand-rolled copies of this could each have
//! been quietly vacuous without anyone noticing.

use crate::common;
use common::walk_differential::{
    assert_modes_differ, assert_probes_identical, assert_walk_differential, Probe, WalkMode,
};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::Machine;

/// A bare Cortex-M machine with a two-instruction self-branch at `ENTRY`, so
/// stepping it is well-defined and identical every time.
const ENTRY: u32 = 0x2000_0000;

fn machine() -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    // `b .` — one Thumb instruction branching to itself. Steps forever with a
    // stable PC, which keeps the probe stream deterministic without needing
    // firmware.
    bus.write_u32(u64::from(ENTRY), 0xE7FE_E7FE).unwrap();
    let mut m = Machine::new(cpu, bus);
    m.config.peripheral_tick_interval = 1;
    m.bus.config.peripheral_tick_interval = 1;
    m
}

fn no_extra(_: &Machine<CortexM>) -> Vec<(&'static str, u64)> {
    Vec::new()
}

#[test]
fn identical_runs_pass() {
    assert_walk_differential("identical", ENTRY, 32, |_mode| machine(), no_extra);
}

/// The caller-supplied observables are what make a real gate bite — the
/// architectural state alone would not notice a peripheral that stopped moving
/// bytes. Prove a difference THERE is caught, not just one in the registers.
#[test]
#[should_panic(expected = "first divergence at step")]
fn a_divergence_in_caller_observables_is_caught() {
    let reference: Vec<Probe> = (1..=4)
        .map(|s| Probe {
            step: s,
            total_cycles: s,
            pc: ENTRY,
            regs: [0; 16],
            extra: vec![("bytes_sent", s)],
        })
        .collect();
    let mut candidate = reference.clone();
    // The scheduler side stopped moving bytes after step 2 — the exact shape
    // of a botched transfer migration.
    candidate[2].extra = vec![("bytes_sent", 2)];
    candidate[3].extra = vec![("bytes_sent", 2)];

    assert_probes_identical(&reference, &candidate, "observable divergence");
}

#[test]
#[should_panic(expected = "total_cycles")]
fn a_divergence_in_cycles_is_caught() {
    let reference: Vec<Probe> = (1..=4)
        .map(|s| Probe {
            step: s,
            total_cycles: s,
            pc: ENTRY,
            regs: [0; 16],
            extra: Vec::new(),
        })
        .collect();
    let mut candidate = reference.clone();
    candidate[3].total_cycles = 99;

    assert_probes_identical(&reference, &candidate, "cycle divergence");
}

#[test]
#[should_panic(expected = "probe counts differ")]
fn a_run_that_stops_early_is_a_divergence_not_a_pass() {
    let reference: Vec<Probe> = (1..=4)
        .map(|s| Probe {
            step: s,
            total_cycles: s,
            pc: ENTRY,
            regs: [0; 16],
            extra: Vec::new(),
        })
        .collect();
    let candidate = reference[..2].to_vec();

    assert_probes_identical(&reference, &candidate, "short run");
}

/// The vacuity guard, both directions.
#[test]
fn modes_differ_accepts_a_real_differential() {
    assert_modes_differ(false, true, "real");
}

#[test]
#[should_panic(expected = "CANDIDATE side does not report uses_scheduler")]
fn an_unmigrated_model_is_refused_rather_than_passed() {
    // Both sides walk-driven: the migration is not in effect, so comparing
    // them proves nothing. This is the pass that must never be silent.
    assert_modes_differ(false, false, "unmigrated");
}

#[test]
#[should_panic(expected = "REFERENCE side reports uses_scheduler")]
fn a_reference_that_did_not_pin_to_the_walk_is_refused() {
    // `force_legacy_walk()` did not take — both sides scheduler-driven.
    assert_modes_differ(true, true, "unpinned reference");
}

/// `WalkMode` is the builder's only input, so it has to be legible at the call
/// site: `if mode.is_legacy_walk() { p.force_legacy_walk() }`.
#[test]
fn walk_mode_reads_the_way_a_builder_uses_it() {
    assert!(WalkMode::LegacyWalk.is_legacy_walk());
    assert!(!WalkMode::Scheduler.is_legacy_walk());
}
