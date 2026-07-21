// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential (lockstep) equivalence gate for the RV32IMC JIT frontend.
//!
//! This is the universal-dispatch framework's **merge gate #1** applied to
//! RISC-V: run the *same* hand-assembled RV32IMC hot loop twice from the
//! same reset — once on the pure interpreter, once through the
//! [`DispatchLoop`] driving the [`RiscVFrontend`] + `InterpreterRuntime`
//! over a [`RiscVJitHost`] — and assert the architectural state is
//! byte-identical at *every* retired instruction.
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
use labwired_core::cpu::jit_framework::riscv::{
    differential_cycle_ignore_indices, snapshot_state, RiscVFrontend, RiscVJitHost,
};
use labwired_core::cpu::jit_framework::runtime::{InterpreterRuntime, MemoryBinding};
use labwired_core::cpu::RiscV;
use labwired_core::Machine;

// ── RV32I instruction encoders (standard field layouts) ──────────────────

fn enc_addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (rd << 7) | 0x13
}

fn enc_xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
    (rs2 << 20) | (rs1 << 15) | (0b100 << 12) | (rd << 7) | 0x33
}

fn enc_jal(rd: u32, imm: i32) -> u32 {
    let u = imm as u32;
    let imm20 = (u >> 20) & 1;
    let imm10_1 = (u >> 1) & 0x3FF;
    let imm11 = (u >> 11) & 1;
    let imm19_12 = (u >> 12) & 0xFF;
    (imm20 << 31) | (imm10_1 << 21) | (imm11 << 20) | (imm19_12 << 12) | (rd << 7) | 0x6F
}

/// A self-contained RV32IMC hot loop laid at flash PC 0:
///
/// ```text
/// 0x00:  addi x1, x1, 1     ; counter++
/// 0x04:  xor  x3, x1, x2    ; some real ALU work depending on the counter
/// 0x08:  jal  x0, -8        ; branch back to 0x00 (infinite loop)
/// ```
///
/// Pure register + control-flow work: no memory, no CSRs, no traps — so the
/// interpreter never errors and both runs stay perfectly PC-aligned.
fn loop_program() -> Vec<u8> {
    let words = [
        enc_addi(1, 1, 1), // addi x1,x1,1
        enc_xor(3, 1, 2),  // xor  x3,x1,x2
        enc_jal(0, -8),    // jal  x0,-8  (back to 0x00)
    ];
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for w in words {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    bytes
}

/// Build a fresh `Machine<RiscV>` with the hot loop in flash at base 0 and
/// the reset PC at the loop head. Two calls produce two independent,
/// identically-initialised machines.
fn build_loop_machine() -> Machine<RiscV> {
    let mut bus = SystemBus::new();
    bus.flash.data = loop_program();
    bus.flash.base_addr = 0;

    let mut cpu = RiscV::new();
    cpu.pc = 0;
    // Keep the CLINT timer out of the way: with mtimecmp maxed, the
    // free-running mtime never raises MTIP, so `mip` stays 0 throughout and
    // no timer interrupt is ever pending (mie/mstatus.MIE are 0 anyway).
    cpu.mtimecmp = u64::MAX;

    Machine::new(cpu, bus)
}

#[test]
fn riscv_all_bail_frontend_is_byte_identical_to_interpreter() {
    // Comparison budget — well above any no-op floor.
    const MAX_COMPARES: u64 = 3_000;
    const FLOOR: u64 = 2_000;

    let mut interp_machine = build_loop_machine();
    let mut jit_machine = build_loop_machine();

    // JIT side: the all-bail RV32IMC frontend on the interpreter runtime,
    // driven by the universal dispatch loop. A low hot threshold makes the
    // recurring loop PCs promote + compile (to body-less stubs) quickly, so
    // the compiled-block dispatch path is genuinely exercised.
    let flash_len = jit_machine.bus.flash.data.len();
    let mem = MemoryBinding::NativeLinear {
        guest_base: 0,
        len: flash_len,
    };
    let mut jit_loop =
        DispatchLoop::new(RiscVFrontend::new(), InterpreterRuntime, mem).with_hot_threshold(4);

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
            let mut host = RiscVJitHost::new(&mut jit_machine);
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
        interp_machine.cpu.x[1] as u64 >= FLOOR / 3,
        "counter x1={} too low — the loop body did not run enough",
        interp_machine.cpu.x[1]
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
