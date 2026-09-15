// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential (lockstep) equivalence gate for the Thumb-2 JIT frontend.
//!
//! Mirror of [`riscv_jit_lockstep`]: run the *same* hand-assembled Thumb
//! hot loop twice from the same reset — once on the pure interpreter, once
//! through the [`DispatchLoop`] driving the [`ThumbFrontend`] +
//! `InterpreterRuntime` over a [`CortexMJitHost`] — and assert the
//! architectural state is byte-identical at *every* retired instruction.
//!
//! In this foundation milestone the frontend is **all-bail**: every block
//! walks and classifies but emits no wasm, so it side-exits to the
//! interpreter. The test therefore proves the entire dispatch / host /
//! snapshot / cache / side-exit / fallback plumbing is correct and
//! equivalence-preserving *before* any codegen exists. Each later codegen
//! chunk re-runs this exact gate.
//!
//! Gated behind `jit-framework` (the module it exercises is), so it compiles
//! away under the default and `jit` builds.

#![cfg(feature = "jit-framework")]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::jit_framework::differential::{DiffPolicy, DifferentialHarness};
use labwired_core::cpu::jit_framework::dispatch::DispatchLoop;
use labwired_core::cpu::jit_framework::runtime::{InterpreterRuntime, MemoryBinding};
use labwired_core::cpu::jit_framework::thumb::{
    differential_cycle_ignore_indices, snapshot_state, CortexMJitHost, ThumbFrontend,
};
use labwired_core::cpu::CortexM;
use labwired_core::Machine;

/// Reset-handler PC: vector table occupies 8 bytes at flash 0, then the
/// three-instruction loop. Same encodings as `thumb_jit_walk`.
const LOOP_PC: u32 = 0x08;
const MSP: u32 = 0x2000_1000;

/// A self-contained Thumb-16 hot loop laid just past the vector table:
///
/// ```text
/// 0x00:  .word MSP          ; vector[0] initial SP
/// 0x04:  .word LOOP_PC|1    ; vector[1] reset (Thumb)
/// 0x08:  movs r0, #1        ; r0 = 1
/// 0x0A:  adds r0, r0, #1    ; r0 = 2
/// 0x0C:  b    0x08          ; branch back to the MOV
/// ```
///
/// Pure register + control-flow work: no memory, no exceptions, no MMIO —
/// so the interpreter never errors and both runs stay perfectly PC-aligned.
/// `0xE7FC` is B with imm11 = -4 → byte offset -8; from the branch at 0x0C
/// (PC = 0x10) that lands back at 0x08.
fn loop_program() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + 6);
    bytes.extend_from_slice(&MSP.to_le_bytes());
    bytes.extend_from_slice(&(LOOP_PC | 1).to_le_bytes());
    bytes.extend_from_slice(&0x2001u16.to_le_bytes()); // MOV r0, #1
    bytes.extend_from_slice(&0x1C40u16.to_le_bytes()); // ADDS r0, r0, #1
    bytes.extend_from_slice(&0xE7FCu16.to_le_bytes()); // B .-4 (back to LOOP_PC)
    bytes
}

/// Build a fresh `Machine<CortexM>` with the vector table + hot loop in
/// flash at base 0, then reset so SP/PC come from the table. Two calls
/// produce two independent, identically-initialised machines.
fn build_loop_machine() -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    let prog = loop_program();
    bus.flash.data[..prog.len()].copy_from_slice(&prog);
    bus.flash.base_addr = 0;

    let cpu = CortexM::new();
    let mut machine = Machine::new(cpu, bus);
    machine
        .reset()
        .expect("vector table at flash 0 must be readable on reset");
    machine
}

#[test]
fn thumb_all_bail_frontend_is_byte_identical_to_interpreter() {
    // Comparison budget — a short hot loop is enough to promote + dispatch.
    const MAX_COMPARES: u64 = 64;
    const FLOOR: u64 = 64;

    let mut interp_machine = build_loop_machine();
    let mut jit_machine = build_loop_machine();

    assert_eq!(interp_machine.cpu.pc, LOOP_PC, "reset PC from vector[1]");
    assert_eq!(interp_machine.cpu.sp, MSP, "reset SP from vector[0]");

    // JIT side: the all-bail Thumb frontend on the interpreter runtime,
    // driven by the universal dispatch loop. A low hot threshold makes the
    // recurring loop PCs promote + compile (to body-less stubs) quickly, so
    // the compiled-block dispatch path is genuinely exercised.
    let flash_len = jit_machine.bus.flash.data.len();
    let mem = MemoryBinding::NativeLinear {
        guest_base: 0,
        len: flash_len,
    };
    let mut jit_loop =
        DispatchLoop::new(ThumbFrontend::new(), InterpreterRuntime, mem).with_hot_threshold(4);

    let policy = DiffPolicy {
        ignore_indices: differential_cycle_ignore_indices(),
        block_boundary_only: false, // compare after every retired instruction
    };
    let harness = DifferentialHarness::new(MAX_COMPARES).with_policy(policy);

    // Each closure advances its machine by exactly one retired instruction
    // and returns the flattened architectural state, or `None` if the
    // machine failed to advance (halt / trap the interpreter cannot service).
    let interp_step = || match interp_machine.step() {
        Ok(()) => Some(snapshot_state(&interp_machine.cpu)),
        Err(_) => None,
    };

    let jit_step = || {
        let before = jit_machine.total_cycles;
        {
            let mut host = CortexMJitHost::new(&mut jit_machine);
            // One dispatch iteration retires exactly one guest instruction:
            // the all-bail stub side-exits to the interpreter, which steps
            // once (or a cold PC is interpreted directly).
            jit_loop.run(&mut host, 1);
        }
        if jit_machine.total_cycles == before {
            None // machine did not advance — treat as halt
        } else {
            Some(snapshot_state(&jit_machine.cpu))
        }
    };

    let report = harness.run(interp_step, jit_step);

    // (1) Byte-for-byte equivalence at every comparison point.
    assert!(
        report.is_equivalent(),
        "JIT diverged from interpreter: {:?}",
        report.divergence
    );

    // (2) Non-vacuous: the firmware really executed a hot loop, not a no-op.
    assert!(
        report.compares >= FLOOR,
        "only {} instructions compared (expected >= {}); the run halted early",
        report.compares,
        FLOOR
    );
    assert!(
        interp_machine.cpu.r0 == 1 || interp_machine.cpu.r0 == 2,
        "r0={} — MOV/ADD body did not run",
        interp_machine.cpu.r0
    );

    // (3) The JIT plumbing was genuinely engaged (not pure refusal): hot PCs
    // were compiled and their (body-less) blocks dispatched + side-exited.
    let stats = jit_loop.stats();
    assert!(
        stats.compiled > 0,
        "no block ever crossed the hot threshold"
    );
    assert!(
        stats.block_runs > 0,
        "no compiled block was ever dispatched (JIT path not exercised)"
    );
    assert!(
        stats.interpreted >= FLOOR,
        "interpreter fallback retired only {} instructions",
        stats.interpreted
    );
    // All-bail frontend never chains (every block side-exits to interp).
    assert_eq!(stats.chained, 0, "an all-bail block must not chain");
}
