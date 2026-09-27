// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Snapshot/restore and reverse step on the adapter the headless debug
//! sessions run, on the committed nRF54L15 smart-ring probe (never skips).

use labwired_dap::adapter::LabwiredAdapter;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load() -> LabwiredAdapter {
    let a = LabwiredAdapter::new();
    a.load_firmware(
        root().join("tests/fixtures/nrf54l15-smart-ring.elf"),
        Some(root().join("examples/nrf54l15-smart-ring/system.yaml")),
    )
    .unwrap();
    a
}

/// PC, every core register, cycles.
fn state(a: &LabwiredAdapter) -> (u32, Vec<u32>, u64) {
    let regs = (0..16).map(|i| a.get_register(i).unwrap()).collect();
    let cycles = a
        .machine
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .get_cycle_count();
    (a.get_pc().unwrap(), regs, cycles)
}

fn drive(a: &LabwiredAdapter) {
    for _ in 0..25 {
        a.step().unwrap();
    }
    a.continue_execution_chunk(4_000).unwrap();
    for _ in 0..10 {
        a.step().unwrap();
    }
}

#[test]
fn restore_then_run_matches_a_straight_run() {
    let straight = load();
    for _ in 0..40 {
        straight.step().unwrap();
    }
    straight.continue_execution_chunk(2_000).unwrap();
    straight.poll_uart();
    drive(&straight);
    let expected = (state(&straight), straight.poll_uart());
    assert!(!expected.1.is_empty(), "the probe prints in this window");

    let a = load();
    for _ in 0..40 {
        a.step().unwrap();
    }
    a.continue_execution_chunk(2_000).unwrap();
    let saved = a.snapshot_save(Some("mid-probe".into())).unwrap();
    a.poll_uart();
    drive(&a);
    a.continue_execution_chunk(3_000).unwrap();
    assert_ne!(state(&a).2, saved.cycles);

    let back = a.snapshot_restore(saved.id).unwrap();
    assert_eq!(back.cycles, saved.cycles);
    assert_eq!(state(&a).2, saved.cycles);
    assert!(
        a.poll_uart().is_empty(),
        "output before the point is not re-sent"
    );
    drive(&a);
    assert_eq!(
        (state(&a), a.poll_uart()),
        expected,
        "restore then run == straight run"
    );
    assert_eq!(a.snapshot_list().len(), 1);
}

#[test]
fn reverse_step_undoes_single_steps_headlessly() {
    let a = load();
    a.continue_execution_chunk(1_000).unwrap();
    let before = state(&a);
    for _ in 0..8 {
        a.step().unwrap();
    }
    assert_ne!(state(&a).0, before.0);
    for _ in 0..8 {
        a.step_back().unwrap();
    }
    let after = state(&a);
    assert_eq!(
        (after.0, after.1),
        (before.0, before.1),
        "PC and registers restored"
    );
    let e = a.step_back().unwrap_err().to_string();
    assert!(e.contains("No history"), "{e}");
}

#[test]
fn a_continue_clears_the_reverse_history() {
    // Before: the steps recorded before a continue stayed in the history, so a
    // step back after the continue "undid" an instruction from before it and
    // landed on a state the firmware was never in.
    let a = load();
    for _ in 0..5 {
        a.step().unwrap();
    }
    a.continue_execution_chunk(500).unwrap();
    let e = a.step_back().unwrap_err().to_string();
    assert!(e.contains("No history"), "{e}");
}

#[test]
fn restore_is_refused_when_the_replay_cannot_reproduce_the_state() {
    let a = load();
    a.continue_execution_chunk(1_000).unwrap();
    // Change a register behind the journal's back.
    a.machine
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .write_core_reg(5, 0xDEAD_BEEF);
    let saved = a.snapshot_save(None).unwrap();
    a.continue_execution_chunk(500).unwrap();
    let before = state(&a);
    let e = a.snapshot_restore(saved.id).unwrap_err().to_string();
    assert!(e.contains("restore refused"), "{e}");
    assert_eq!(state(&a), before, "the previous state is unchanged");
}
