// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential equivalence + non-vacuity gate for the Cortex-M ALU JIT.
//!
//! The interpreter is the spec. Compiled Thumb blocks must retire the same
//! architectural state at every block/instruction boundary.

#![cfg(all(feature = "jit-framework", feature = "jit"))]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::jit_framework::cortex_m::{
    differential_cycle_ignore_indices, snapshot_state, CortexMFrontend, CortexMJitEngine,
    CortexMWasmJit,
};
use labwired_core::cpu::jit_framework::differential::{compare, DiffPolicy};
use labwired_core::cpu::jit_framework::frontend::IsaFrontend;
use labwired_core::cpu::jit_framework::CodeView;
use labwired_core::cpu::CortexM;
use labwired_core::Machine;

fn h(bytes: &mut Vec<u8>, half: u16) {
    bytes.extend_from_slice(&half.to_le_bytes());
}

fn movs(rd: u8, imm: u8) -> u16 {
    0x2000 | ((rd as u16) << 8) | imm as u16
}
fn adds_imm8(rd: u8, imm: u8) -> u16 {
    0x3000 | ((rd as u16) << 8) | imm as u16
}
fn add_reg(rd: u8, rn: u8, rm: u8) -> u16 {
    0x1800 | ((rm as u16) << 6) | ((rn as u16) << 3) | rd as u16
}
fn sub_reg(rd: u8, rn: u8, rm: u8) -> u16 {
    0x1A00 | ((rm as u16) << 6) | ((rn as u16) << 3) | rd as u16
}
fn ands(rd: u8, rm: u8) -> u16 {
    0x4000 | ((rm as u16) << 3) | rd as u16
}
fn eors(rd: u8, rm: u8) -> u16 {
    0x4040 | ((rm as u16) << 3) | rd as u16
}
fn orrs(rd: u8, rm: u8) -> u16 {
    0x4300 | ((rm as u16) << 3) | rd as u16
}
fn lsls_imm(rd: u8, rm: u8, imm: u8) -> u16 {
    ((imm as u16) << 6) | ((rm as u16) << 3) | rd as u16
}
fn b_to(from_pc: i32, to_pc: i32) -> u16 {
    let offset = to_pc - (from_pc + 4);
    let imm11 = ((offset / 2) as i32) as u16 & 0x7FF;
    0xE000 | imm11
}

fn alu_loop_program() -> Vec<u8> {
    // 20 sequential 16-bit ALU ops + an unconditional branch back to 0.
    let mut p = Vec::new();
    h(&mut p, movs(0, 0));
    h(&mut p, adds_imm8(0, 1));
    h(&mut p, add_reg(1, 0, 0));
    h(&mut p, add_reg(2, 1, 0));
    h(&mut p, sub_reg(3, 2, 1));
    h(&mut p, eors(3, 0));
    h(&mut p, orrs(1, 0));
    h(&mut p, ands(1, 2));
    h(&mut p, lsls_imm(4, 0, 3));
    h(&mut p, adds_imm8(2, 3));
    h(&mut p, add_reg(5, 4, 2));
    h(&mut p, sub_reg(6, 5, 4));
    h(&mut p, eors(6, 1));
    h(&mut p, orrs(5, 6));
    h(&mut p, ands(5, 0));
    h(&mut p, adds_imm8(4, 1));
    h(&mut p, add_reg(7, 4, 5));
    h(&mut p, sub_reg(7, 7, 0));
    h(&mut p, eors(2, 7));
    h(&mut p, adds_imm8(0, 0)); // still ALU; r0 unchanged add #0
                                // 20 insns so far (index 0..19), branch at byte 40.
    let from = 20 * 2;
    h(&mut p, b_to(from, 2)); // skip the initial movs; loop at adds r0,#1
    p
}

fn build_machine(prog: &[u8]) -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    bus.flash.data = prog.to_vec();
    bus.flash.data.resize(1024 * 1024, 0xFF);
    bus.flash.base_addr = 0;
    let mut cpu = CortexM::new();
    cpu.pc = 0;
    cpu.sp = 0x2000_1000;
    cpu.xpsr = 0x0100_0000;
    Machine::new(cpu, bus)
}

#[test]
fn alu_hot_loop_is_byte_identical_and_compiles() {
    const MAX_UNITS: u64 = 8_000;
    const INSTR_FLOOR: u64 = 3_000;

    let prog = alu_loop_program();
    let mut interp = build_machine(&prog);
    let mut jit = build_machine(&prog);
    let mut engine = CortexMJitEngine::new(4);
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
            interp
                .step()
                .expect("interpreter must not fault on pure ALU");
        }
        retired += n as u64;
        let si = snapshot_state(&interp.cpu);
        let sj = snapshot_state(&jit.cpu);
        if let Some(d) = compare(units, &si, &sj, &policy) {
            panic!(
                "JIT diverged from interpreter at unit {units} (retired {retired}): {d:?}\n\
                 interp pc={:#x} r0={} xpsr={:#x}\n jit    pc={:#x} r0={} xpsr={:#x}",
                interp.cpu.pc, interp.cpu.r0, interp.cpu.xpsr, jit.cpu.pc, jit.cpu.r0, jit.cpu.xpsr
            );
        }
    }

    let stats = engine.stats();
    assert!(stats.compiled > 0, "no ALU block compiled");
    assert!(stats.block_runs > 0, "no compiled block executed");
    assert!(
        stats.block_instrs >= INSTR_FLOOR,
        "compiled blocks retired only {} (floor {INSTR_FLOOR})",
        stats.block_instrs
    );
    println!(
        "cortex_m alu_hot_loop: retired={retired} compiled={} block_runs={} block_instrs={} interpreted={}",
        stats.compiled, stats.block_runs, stats.block_instrs, stats.interpreted
    );
}

#[test]
fn every_alu_op_matches_interpreter() {
    let mut seed: u64 = 0x1234_5678_9abc_def0;
    let mut rng = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as u32
    };

    let frontend = CortexMFrontend::new();
    let jit = CortexMWasmJit::new();

    // One-instruction blocks: adds r0, #imm ; b .
    for _ in 0..64 {
        let imm = (rng() & 0xFF) as u8;
        let mut prog = Vec::new();
        h(&mut prog, adds_imm8(0, imm));
        h(&mut prog, 0xE7FE); // b .
        let view = CodeView::new(0, &prog);
        let plan = frontend.translate_block(0, &view).expect("translate");
        assert!(!plan.is_stub(), "adds r0, #{imm} should compile");
        let mut block = jit.compile(&plan, None).expect("compile");

        let mut interp = build_machine(&prog);
        interp.cpu.r0 = rng();
        interp.cpu.r1 = rng();
        let mut x = [0u32; 16];
        labwired_core::cpu::jit_framework::cortex_m::host::pack_regs(&interp.cpu, &mut x);
        let start_r0 = interp.cpu.r0;
        interp.step().unwrap();
        let (exit, n, _) = block.run(&mut x, &mut []);
        assert_eq!(n, 2, "adds + branch");
        assert_eq!(x[0], interp.cpu.r0, "r0 imm={imm} start={start_r0:#x}");
        assert_eq!(x[15] & 0xF000_0000, interp.cpu.xpsr & 0xF000_0000, "NZCV");
        let _ = exit;
    }
}

fn str_imm(rt: u8, rn: u8, imm_bytes: u8) -> u16 {
    let imm5 = (imm_bytes / 4) as u16;
    0x6000 | (imm5 << 6) | ((rn as u16) << 3) | rt as u16
}
fn ldr_imm(rt: u8, rn: u8, imm_bytes: u8) -> u16 {
    let imm5 = (imm_bytes / 4) as u16;
    0x6800 | (imm5 << 6) | ((rn as u16) << 3) | rt as u16
}

#[test]
fn ram_load_store_loop_matches_interpreter() {
    // r1 = RAM base. Body: str r0,[r1,#0] ; ldr r2,[r1,#0] ; adds r0,#1 ; b back.
    // Pad with nops so the compiled block clears MIN_PROFITABLE.
    let mut prog = Vec::new();
    h(&mut prog, movs(0, 0));
    for _ in 0..16 {
        h(&mut prog, 0xBF00); // nop
    }
    h(&mut prog, str_imm(0, 1, 0));
    h(&mut prog, ldr_imm(2, 1, 0));
    h(&mut prog, adds_imm8(0, 1));
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 2));

    let mut interp = build_machine(&prog);
    let mut jit = build_machine(&prog);
    interp.cpu.r1 = 0x2000_0000;
    jit.cpu.r1 = 0x2000_0000;
    let mut engine = CortexMJitEngine::new(4);
    let policy = DiffPolicy {
        ignore_indices: differential_cycle_ignore_indices(),
        block_boundary_only: false,
    };
    let mut retired = 0u64;
    for units in 1..=4_000u64 {
        let n = engine.step_unit(&mut jit);
        assert!(n > 0, "halt at unit {units}");
        for _ in 0..n {
            interp.step().expect("interp ram loop");
        }
        retired += n as u64;
        let d = compare(
            units,
            &snapshot_state(&interp.cpu),
            &snapshot_state(&jit.cpu),
            &policy,
        );
        if let Some(d) = d {
            panic!("RAM loop diverged at unit {units}: {d:?}");
        }
        if retired > 2_000 {
            break;
        }
    }
    let stats = engine.stats();
    assert!(stats.block_runs > 0, "RAM loop never compiled: {stats:?}");
    assert_eq!(interp.cpu.r0, jit.cpu.r0);
}
