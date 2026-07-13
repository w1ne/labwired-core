// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential equivalence gate for the **composition** of RV32IMC JIT chunks
//! C (integer ALU), D (branches/jumps), and E (loads/stores).
//!
//! The three sibling gates each prove one emittable surface in isolation
//! (`riscv_jit_alu_lockstep`, `riscv_jit_branch_lockstep`,
//! `riscv_jit_mem_lockstep`). This gate is the first proof they *compose*: a
//! single compiled block whose straight-line body mixes ALU ops, an in-window
//! **load**, and an in-window **store**, and which then **terminates at a
//! backward conditional branch** (chunk D) that chains the block back to
//! itself. All three wire codes are in play — `0` fall-through, `1`
//! dynamic-chain, `2` memory-fault.
//!
//!   1. [`combined_block_is_one_alu_mem_branch_block`] — a static assertion on
//!      the emitted plan: the loop body translates to ONE block that retires
//!      the load, the ALU ops, the store, AND the branch terminator, carries a
//!      RAM binding with a store, and exposes both the dynamic-chain and the
//!      memory-fault exit edges.
//!   2. [`combined_hot_loop_is_byte_identical`] — that block driven as a hot
//!      loop through the compiled-block dispatcher, state compared word-for-
//!      word against the interpreter at every dispatch boundary and guest RAM
//!      compared byte-for-byte, asserted non-vacuous (compiled blocks, block
//!      runs, a retired-instruction floor).
//!   3. [`combined_mmio_fault_is_byte_identical`] — a loop whose combined block
//!      does an in-window load + ALU + in-window store then an **out-of-window
//!      (MMIO) load**: the load faults mid-block, the block commits the prior
//!      instructions, and the engine resumes the interpreter to perform the
//!      real access — byte-identical to a pure-interpreter run.
//!
//! Gated behind `jit-framework` (the module) and `jit` (the native wasmtime
//! executor), mirroring the three per-chunk gates.

#![cfg(all(feature = "jit-framework", feature = "jit"))]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::jit_framework::differential::{compare, DiffPolicy};
use labwired_core::cpu::jit_framework::riscv::emit::{WIRE_CHAIN_DYNAMIC, WIRE_MEM_FAULT};
use labwired_core::cpu::jit_framework::riscv::{
    differential_cycle_ignore_indices, snapshot_state, RiscVFrontend, RiscvJitEngine,
};
use labwired_core::cpu::jit_framework::CodeView;
use labwired_core::cpu::RiscV;
use labwired_core::Machine;

/// Guest-RAM window the JIT binds against. Base is the default SRAM base so the
/// interpreter's bus routes it to `bus.ram`; length kept small so the per-block
/// RAM sync is cheap.
const RAM_BASE: u32 = 0x2000_0000;
const RAM_LEN: usize = 0x1000;

// ── encoders (standard RISC-V field layouts) ────────────────────────────────

fn enc_i(rd: u32, rs1: u32, funct3: u32, imm: i32, opcode: u32) -> u32 {
    ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}
fn enc_s(rs1: u32, rs2: u32, funct3: u32, imm: i32, opcode: u32) -> u32 {
    let u = imm as u32 & 0xFFF;
    ((u >> 5) << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | ((u & 0x1F) << 7) | opcode
}
fn lui(rd: u32, imm20: u32) -> u32 {
    (imm20 << 12) | (rd << 7) | 0x37
}
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b000, imm, 0x13)
}
fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
    (rs2 << 20) | (rs1 << 15) | (0b100 << 12) | (rd << 7) | 0x33
}
fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
    (rs2 << 20) | (rs1 << 15) | (rd << 7) | 0x33
}
fn lw(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b010, imm, 0x03)
}
fn sw(rs1: u32, rs2: u32, imm: i32) -> u32 {
    enc_s(rs1, rs2, 0b010, imm, 0x23)
}
/// B-type conditional branch (`bne` funct3 = 0b001).
fn bne(rs1: u32, rs2: u32, imm: i32) -> u32 {
    let u = imm as u32;
    ((u >> 12) & 1) << 31
        | ((u >> 5) & 0x3F) << 25
        | (rs2 << 20)
        | (rs1 << 15)
        | (0b001 << 12)
        | ((u >> 1) & 0xF) << 8
        | ((u >> 11) & 1) << 7
        | 0x63
}

// ── machine / flash helpers ─────────────────────────────────────────────────

fn flash_of(prog: &[u32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(prog.len() * 4);
    for w in prog {
        b.extend_from_slice(&w.to_le_bytes());
    }
    b
}

fn build_machine(prog: &[u32], ram: &[u8]) -> Machine<RiscV> {
    let mut bus = SystemBus::new();
    bus.flash.data = flash_of(prog);
    bus.flash.base_addr = 0;
    bus.ram.base_addr = RAM_BASE as u64;
    bus.ram.data = vec![0u8; RAM_LEN];
    bus.ram.data[..ram.len()].copy_from_slice(ram);
    let mut cpu = RiscV::new();
    cpu.pc = 0;
    cpu.mtimecmp = u64::MAX; // keep the CLINT timer from ever firing
    Machine::new(cpu, bus)
}

/// A hot loop whose body — the block starting at the loop head (word index 3,
/// `pc = 12`) — mixes an in-window load, ALU, an in-window store, more ALU, and
/// ends at a backward `bne` that chains the block back to itself. `x6 == 0` and
/// `x2` only increments, so the branch is always taken (the loop never falls
/// out) and the same compiled block re-runs every iteration.
fn combined_loop_program() -> Vec<u32> {
    vec![
        lui(1, 0x2_0000), // x1 = 0x2000_0000 (RAM base)     [0]
        addi(2, 0, 0),    // x2 = 0 (counter)                [1]
        addi(6, 0, 0),    // x6 = 0 (loop never terminates)  [2]
        // loop head @ pc = 12 (word index 3):
        lw(3, 1, 0),         //  x3 = mem[base+0]      (load)   [3]
        addi(3, 3, 1),       //  x3++                  (alu)    [4]
        sw(1, 3, 0),         //  mem[base+0] = x3      (store)  [5]  accumulates
        xor(4, 3, 2),        //  x4 = x3 ^ x2          (alu)    [6]
        sw(1, 4, 4),         //  mem[base+4] = x4      (store)  [7]
        addi(2, 2, 1),       //  x2++                  (alu)    [8]
        bne(2, 6, -(6 * 4)), //  if x2 != x6 goto head (branch) [9]
    ]
}

/// Word index of the loop head and its guest PC.
const HEAD_IDX: usize = 3;
const HEAD_PC: u64 = (HEAD_IDX as u64) * 4;
/// Instructions in the loop body block: lw, addi, sw, xor, sw, addi, bne.
const BODY_INSTRS: u32 = 7;

// ── (1) the plan is ONE combined ALU+load+store+branch block ─────────────────

#[test]
fn combined_block_is_one_alu_mem_branch_block() {
    let prog = combined_loop_program();
    let flash = flash_of(&prog);
    let frontend = RiscVFrontend::with_ram_window(RAM_BASE, RAM_LEN as u32);

    let (plan, binding) = frontend
        .translate_block_riscv(HEAD_PC, &CodeView::new(0, &flash))
        .expect("loop-head block must translate");

    assert!(!plan.is_stub(), "combined block must emit real wasm");
    assert_eq!(
        plan.instr_count, BODY_INSTRS,
        "one block subsumes load + ALU + store + the branch terminator"
    );

    // Ends AT the branch (its next PC is dynamic, so end_pc is metadata only).
    assert_eq!(plan.entry_pc, HEAD_PC);
    assert_eq!(plan.end_pc, HEAD_PC + (BODY_INSTRS as u64) * 4);

    // Chunk-E facts: the block touches RAM and contains a store.
    let binding = binding.expect("a load/store block carries a RAM binding");
    assert_eq!(binding.ram_len, RAM_LEN);
    assert!(binding.has_store, "the block contains a store");

    // Chunk-D + chunk-E edges coexist: the primary clean exit is the dynamic
    // chain (the branch terminator), plus the memory-fault edge from the
    // load/store body.
    assert!(
        plan.exits.iter().any(|e| e.wire_code == WIRE_CHAIN_DYNAMIC),
        "combined block must expose the dynamic-chain (branch) edge: {:?}",
        plan.exits
    );
    assert!(
        plan.exits.iter().any(|e| e.wire_code == WIRE_MEM_FAULT),
        "combined block must expose the memory-fault edge: {:?}",
        plan.exits
    );
}

// ── (2) hot loop: byte-identical, non-vacuous ────────────────────────────────

#[test]
fn combined_hot_loop_is_byte_identical() {
    const MAX_UNITS: u64 = 4_000;
    const INSTR_FLOOR: u64 = 3_000;

    let prog = combined_loop_program();
    let seed = vec![0u8; RAM_LEN];
    let mut interp = build_machine(&prog, &seed);
    let mut jit = build_machine(&prog, &seed);

    let mut engine = RiscvJitEngine::new(4);
    let policy = DiffPolicy {
        ignore_indices: differential_cycle_ignore_indices(),
        block_boundary_only: false,
    };

    let mut retired: u64 = 0;
    let mut units: u64 = 0;
    while retired < INSTR_FLOOR * 2 && units < MAX_UNITS {
        units += 1;
        let n = engine.step_unit(&mut jit);
        assert!(n > 0, "jit machine halted unexpectedly at unit {units}");
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
        // RAM is not part of the StateVec, so compare it explicitly.
        assert_eq!(
            jit.bus.ram.data, interp.bus.ram.data,
            "guest RAM diverged at unit {units}"
        );
    }

    let stats = engine.stats();
    assert!(stats.compiled > 0, "no combined block compiled");
    assert!(stats.block_runs > 0, "no compiled block executed");
    assert!(
        stats.block_instrs >= INSTR_FLOOR,
        "compiled blocks retired only {} instructions (floor {INSTR_FLOOR})",
        stats.block_instrs
    );
    // The loop actually mutated RAM (non-vacuous memory traffic).
    let slot0 = u32::from_le_bytes(interp.bus.ram.data[0..4].try_into().unwrap());
    let slot1 = u32::from_le_bytes(interp.bus.ram.data[4..8].try_into().unwrap());
    assert!(slot0 >= 100, "RAM slot 0 = {slot0}, loop under-ran");
    println!(
        "combined_hot_loop: retired={retired} compiled={} block_runs={} block_instrs={} interpreted={} slot0={slot0} slot1={slot1}",
        stats.compiled, stats.block_runs, stats.block_instrs, stats.interpreted
    );
}

// ── (3) MMIO fault: out-of-window access resumes on the interpreter ──────────

/// A hot loop whose combined block does an in-window load + ALU + in-window
/// store, then an **out-of-window (MMIO/flash) load**. The out-of-window load
/// faults mid-block: the block commits the earlier instructions, publishes the
/// faulting PC + retired count, and the engine resumes the interpreter for the
/// real access. The remaining ALU + counter + backward branch are interpreted,
/// and the whole run stays byte-identical to a pure-interpreter execution.
fn combined_fault_loop_program() -> Vec<u32> {
    vec![
        lui(1, 0x2_0000), // x1 = RAM base                    [0]
        addi(2, 0, 0),    // x2 = counter                     [1]
        addi(6, 0, 0),    // x6 = 0                           [2]
        // loop head @ pc = 12 (index 3):
        lw(3, 1, 0),         //  x3 = mem[base]   (in window)   [3]
        addi(3, 3, 1),       //  x3++                           [4]
        sw(1, 3, 0),         //  mem[base] = x3   (in window)   [5]
        lw(4, 0, 0),         //  x4 = mem[0]      (FLASH→FAULT) [6]
        add(4, 4, 3),        //  x4 += x3                       [7]
        addi(2, 2, 1),       //  x2++                           [8]
        bne(2, 6, -(6 * 4)), //  loop (backward branch)         [9]
    ]
}

#[test]
fn combined_mmio_fault_is_byte_identical() {
    const MAX_UNITS: u64 = 3_000;
    const FLOOR: u64 = 1_500;

    let prog = combined_fault_loop_program();
    let seed = vec![0u8; RAM_LEN];
    let mut interp = build_machine(&prog, &seed);
    let mut jit = build_machine(&prog, &seed);

    let mut engine = RiscvJitEngine::new(4);
    let policy = DiffPolicy {
        ignore_indices: differential_cycle_ignore_indices(),
        block_boundary_only: false,
    };

    let mut retired: u64 = 0;
    let mut units: u64 = 0;
    while retired < FLOOR && units < MAX_UNITS {
        units += 1;
        let n = engine.step_unit(&mut jit);
        assert!(n > 0, "jit halted at unit {units}");
        for _ in 0..n {
            interp.step().expect("interpreter step");
        }
        retired += n as u64;

        let si = snapshot_state(&interp.cpu);
        let sj = snapshot_state(&jit.cpu);
        if let Some(d) = compare(units, &si, &sj, &policy) {
            panic!(
                "combined fault-path divergence at unit {units} (retired {retired}): {d:?}\n\
                 interp x={:?}\n jit    x={:?}",
                &interp.cpu.x, &jit.cpu.x
            );
        }
        assert_eq!(
            jit.bus.ram.data, interp.bus.ram.data,
            "guest RAM diverged at unit {units}"
        );
    }

    let stats = engine.stats();
    assert!(stats.compiled > 0, "no block compiled");
    assert!(
        stats.block_runs > 0,
        "no compiled block executed (fault path never engaged)"
    );
    // The out-of-window load is retired on the interpreter every iteration.
    assert!(
        stats.interpreted > 0,
        "the faulting load never fell to the interpreter"
    );
    println!(
        "combined_mmio_fault: retired={retired} compiled={} block_runs={} block_instrs={} interpreted={}",
        stats.compiled, stats.block_runs, stats.block_instrs, stats.interpreted
    );
}
