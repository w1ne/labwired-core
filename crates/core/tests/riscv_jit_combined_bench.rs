// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Whole-program MIPS gate for the composed RV32IMC JIT (chunks C+D+E).
//!
//! The per-chunk micro-benchmarks isolate one surface; this measures a
//! realistic firmware-style hot loop that mixes ALU, in-window loads/stores,
//! and a conditional-branch terminator — the shape a real embedded main loop
//! takes — and reports the compiled-vs-interpreter ratio. This is MERGE BAR #2
//! for the RISC-V frontend: the loop must clear the interpreter by a healthy
//! margin (target >=3x) to justify wiring the JIT into production dispatch.
//!
//! Ignored by default (timing-sensitive); run with `--ignored --nocapture`.

#![cfg(all(feature = "jit-framework", feature = "jit"))]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::jit_framework::riscv::RiscvJitEngine;
use labwired_core::cpu::RiscV;
use labwired_core::Machine;

const RAM_BASE: u32 = 0x2000_0000;
const RAM_LEN: usize = 0x100;

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

fn build_machine(prog: &[u32]) -> Machine<RiscV> {
    let mut bus = SystemBus::new();
    let mut flash = Vec::with_capacity(prog.len() * 4);
    for w in prog {
        flash.extend_from_slice(&w.to_le_bytes());
    }
    bus.flash.data = flash;
    bus.flash.base_addr = 0;
    bus.ram.base_addr = RAM_BASE as u64;
    bus.ram.data = vec![0u8; RAM_LEN];
    let mut cpu = RiscV::new();
    cpu.pc = 0;
    cpu.mtimecmp = u64::MAX;
    Machine::new(cpu, bus)
}

/// Prologue sets x1 = RAM base, x2 = counter, x6 = 0 (branch always taken).
/// The loop body does four (load, alu, store) triples over distinct RAM slots
/// plus a counter bump, then a backward `bne` terminator — one compiled block
/// per iteration exercising ALU + memory + dynamic-chain together.
fn combined_hot_program() -> Vec<u32> {
    let mut prog = vec![
        lui(1, 0x2_0000), // x1 = RAM base
        addi(2, 0, 0),    // x2 = 0 counter
        addi(6, 0, 0),    // x6 = 0
    ];
    let head = prog.len();
    // 4 slots × (lw, addi, xor, sw)
    for slot in 0..4i32 {
        let off = slot * 4;
        prog.push(lw(3, 1, off)); // x3 = mem[slot]
        prog.push(addi(3, 3, 1)); // x3++
        prog.push(xor(4, 3, 2)); // x4 = x3 ^ counter
        prog.push(add(3, 3, 4)); // x3 += x4
        prog.push(sw(1, 3, off)); // mem[slot] = x3
    }
    prog.push(addi(2, 2, 1)); // counter++
    let body_words = (prog.len() - head + 1) as i32; // include the bne itself
    prog.push(bne(2, 6, -(body_words - 1) * 4)); // back to head
    prog
}

#[test]
#[ignore = "micro-benchmark; run with --ignored --nocapture"]
fn bench_combined_loop_beats_interpreter() {
    use std::time::Instant;

    let prog = combined_hot_program();
    const ITERS: u64 = 400_000;

    // Interpreter-only timing.
    let mut im = build_machine(&prog);
    let t0 = Instant::now();
    let mut ic = 0u64;
    while ic < ITERS {
        im.step().unwrap();
        ic += 1;
    }
    let interp_dt = t0.elapsed();

    // JIT timing (warm up so the loop block is compiled first).
    let mut jm = build_machine(&prog);
    let mut engine = RiscvJitEngine::new(4);
    for _ in 0..4_000 {
        engine.step_unit(&mut jm);
    }
    let warm = engine.stats();
    let t1 = Instant::now();
    let mut jc = 0u64;
    while jc < ITERS {
        jc += engine.step_unit(&mut jm) as u64;
    }
    let jit_dt = t1.elapsed();
    let stats = engine.stats();

    let interp_mips = ITERS as f64 / interp_dt.as_secs_f64() / 1e6;
    let jit_mips = ITERS as f64 / jit_dt.as_secs_f64() / 1e6;
    let ratio = interp_dt.as_secs_f64() / jit_dt.as_secs_f64();

    println!("--- RV32IMC combined (ALU+mem+branch) whole-loop bench ({ITERS} guest instr) ---");
    println!("interp: {interp_dt:?}  ({interp_mips:.1} MIPS)");
    println!("jit   : {jit_dt:?}  ({jit_mips:.1} MIPS)");
    println!("speedup: {ratio:.2}x");
    println!(
        "jit path: compiled_blocks={} block_runs={} block_instrs={} interpreted={} (warmup block_runs={})",
        stats.compiled, stats.block_runs, stats.block_instrs, stats.interpreted, warm.block_runs
    );

    assert!(
        stats.block_runs > warm.block_runs,
        "compiled blocks must run during the timed phase"
    );
    assert!(
        ratio > 1.0,
        "compiled combined loop ({jit_mips:.1} MIPS) did not beat the interpreter \
         ({interp_mips:.1} MIPS): {ratio:.2}x"
    );
}
