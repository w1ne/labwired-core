// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! JIT superblock trace fusion (v2) lockstep gate.
//!
//! Exercises a hot loop whose body spans **three** basic blocks joined by
//! direct jumps (an unconditional `jal` chaining block A -> B, block B's
//! straight-line tail falling into block C, and block C ending in a backward
//! conditional `bne` that either re-enters the loop at A or falls through to
//! halt). With `LABWIRED_RISCV_JIT_TRACE=1` the frontend fuses all three
//! blocks into one wasm function instead of chaining three separate
//! host<->guest block runs.
//!
//! This binary sets the fusion env var **before any frontend call** (the
//! toggle is cached process-wide via `OnceLock`), so it must stay in its own
//! test file/process rather than sharing one with fusion-off gates.
//!
//! Asserts:
//! 1. byte-identical guest state (registers + RAM) against the interpreter,
//!    every dispatch unit, for the whole run;
//! 2. `EngineStats::fused_traces` > 0 — the loop body actually got fused, not
//!    silently chained block-by-block (an anti-vacuity check).

#![cfg(all(feature = "jit-framework", feature = "jit"))]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::jit_framework::differential::{compare, DiffPolicy};
use labwired_core::cpu::jit_framework::riscv::{
    differential_cycle_ignore_indices, snapshot_state, RiscvJitEngine,
};
use labwired_core::cpu::RiscV;
use labwired_core::Machine;

const RAM_BASE: u32 = 0x2000_0000;
const RAM_LEN: usize = 0x1000;

fn enc_i(rd: u32, rs1: u32, funct3: u32, imm: i32, opcode: u32) -> u32 {
    ((imm as u32) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b000, imm, 0x13)
}
fn jal(rd: u32, imm: i32) -> u32 {
    let u = imm as u32;
    let imm20 = (u >> 20) & 1;
    let imm10_1 = (u >> 1) & 0x3FF;
    let imm11 = (u >> 11) & 1;
    let imm19_12 = (u >> 12) & 0xFF;
    (imm20 << 31) | (imm10_1 << 21) | (imm11 << 20) | (imm19_12 << 12) | (rd << 7) | 0x6F
}
fn enc_b(rs1: u32, rs2: u32, funct3: u32, imm: i32) -> u32 {
    let u = imm as u32;
    let b12 = (u >> 12) & 1;
    let b11 = (u >> 11) & 1;
    let b10_5 = (u >> 5) & 0x3F;
    let b4_1 = (u >> 1) & 0xF;
    (b12 << 31)
        | (b10_5 << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (funct3 << 12)
        | (b4_1 << 8)
        | (b11 << 7)
        | 0x63
}
fn bne(rs1: u32, rs2: u32, imm: i32) -> u32 {
    enc_b(rs1, rs2, 0b001, imm)
}
fn ecall() -> u32 {
    0x0000_0073
}

fn flash_of(prog: &[u32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(prog.len() * 4);
    for w in prog {
        b.extend_from_slice(&w.to_le_bytes());
    }
    b
}

fn build_machine(prog: &[u32]) -> Machine<RiscV> {
    let mut bus = SystemBus::new();
    bus.flash.data = flash_of(prog);
    bus.flash.base_addr = 0;
    bus.ram.base_addr = RAM_BASE as u64;
    bus.ram.data = vec![0u8; RAM_LEN];
    let mut cpu = RiscV::new();
    cpu.pc = 0;
    cpu.mtimecmp = u64::MAX; // keep the CLINT timer from ever firing
    Machine::new(cpu, bus)
}

/// A 40-iteration counting loop whose HOT body spans **three** basic blocks,
/// re-entered every iteration, joined by two direct (unconditional) `jal`s
/// plus one backward conditional branch:
///
///   block A @ pc=8   (loop head): x3 += 1 -- ends in `jal` to block B.
///   block B @ pc=20  (direct-jumped target of A): x4 += 1 -- ends in
///                     another `jal` to block C.
///   block C @ pc=32  (direct-jumped target of B): x2 += 1, then
///                     `bne x2,x6,-28` -- backward conditional branch to the
///                     loop head (block A, pc=8) if the loop is not done;
///                     falls through to `ecall` (halt) once `x2 == x6`.
///
/// Each `ecall` right after a `jal` (pc=16, pc=28) is dead code the fused
/// trace must never execute — a canary: control only reaches it if a `jal`
/// target was miscomputed. A->B and B->C are genuine cross-block direct
/// jumps (fusable only under trace fusion); C->A is the fused conditional
/// loop-back, whose TAKEN target lands back inside the trace's own visited
/// set (block A), so the walker's cycle guard must side-exit it dynamically
/// rather than fusing an unbounded self-loop. Prologue (`x2`/`x6` init) runs
/// once and is never part of the hot trace.
fn three_block_loop_program() -> Vec<u32> {
    vec![
        addi(2, 0, 0),  // x2 = 0 (counter)                 [pc=0]
        addi(6, 0, 40), // x6 = 40 (loop bound)              [pc=4]
        addi(3, 3, 1),  // x3++ (loop head, block A)        [pc=8]
        jal(0, 8),      // jal -> pc=20 (block B)            [pc=12]
        ecall(),        // dead code canary                  [pc=16]
        addi(4, 4, 1),  // x4++ (block B)                    [pc=20]
        jal(0, 8),      // jal -> pc=32 (block C)            [pc=24]
        ecall(),        // dead code canary                  [pc=28]
        addi(2, 2, 1),  // x2++ (block C)                    [pc=32]
        bne(2, 6, -28), // if x2 != x6 goto pc=8 (block A)   [pc=36]
        ecall(),        // halt once the loop falls through  [pc=40]
    ]
}

#[test]
fn three_block_direct_jump_loop_is_byte_identical_and_fuses() {
    // Must be set before the frontend's first call: `trace_fusion_enabled()`
    // caches the env read in a process-wide `OnceLock`, and every test in
    // this binary shares that cache -- hence this program having its own
    // test file/process.
    std::env::set_var("LABWIRED_RISCV_JIT_TRACE", "1");

    const MAX_UNITS: u64 = 2_000;

    let prog = three_block_loop_program();
    let mut interp = build_machine(&prog);
    let mut jit = build_machine(&prog);

    let mut engine = RiscvJitEngine::new(4);
    let policy = DiffPolicy {
        ignore_indices: differential_cycle_ignore_indices(),
        block_boundary_only: false,
    };

    let mut retired: u64 = 0;
    let mut units: u64 = 0;
    while units < MAX_UNITS && jit.cpu.x[2] < 40 {
        units += 1;
        let n = engine.step_unit(&mut jit);
        if n == 0 {
            break; // genuinely halted/faulted before the loop finished
        }
        for _ in 0..n {
            interp.step().expect("interpreter must not fault");
        }
        retired += n as u64;

        let si = snapshot_state(&interp.cpu);
        let sj = snapshot_state(&jit.cpu);
        if let Some(d) = compare(units, &si, &sj, &policy) {
            panic!(
                "JIT diverged from interpreter at unit {units} (retired {retired}): {d:?}\n\
                 interp x={:?}\n jit    x={:?}",
                &interp.cpu.x, &jit.cpu.x
            );
        }
        assert_eq!(
            jit.bus.ram.data, interp.bus.ram.data,
            "guest RAM diverged at unit {units}"
        );
    }

    assert_eq!(
        interp.cpu.x[2], 40,
        "interpreter loop counter must reach 40"
    );
    assert_eq!(jit.cpu.x[2], 40, "jit loop counter must reach 40");
    assert_eq!(
        interp.cpu.x[3], 40,
        "interpreter block-A counter must reach 40"
    );
    assert_eq!(jit.cpu.x[3], 40, "jit block-A counter must reach 40");
    assert_eq!(
        interp.cpu.x[4], 40,
        "interpreter block-B counter must reach 40"
    );
    assert_eq!(jit.cpu.x[4], 40, "jit block-B counter must reach 40");

    let stats = engine.stats();
    assert!(stats.compiled > 0, "no block compiled");
    assert!(
        stats.fused_traces > 0,
        "trace fusion enabled but no fused (multi-block) trace was installed \
         -- the A->B direct jal never got fused: {stats:?}"
    );
    assert!(
        stats.fused_blocks >= 2 * stats.fused_traces,
        "a fused trace must span at least 2 blocks: {stats:?}"
    );
    println!(
        "three_block_fusion: retired={retired} compiled={} fused_traces={} fused_blocks={}",
        stats.compiled, stats.fused_traces, stats.fused_blocks
    );
}
