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
    CortexMWasmJit, MIN_PROFITABLE_BLOCK_INSTRS,
};
use labwired_core::cpu::jit_framework::differential::{compare, compare_memory, DiffPolicy};
use labwired_core::cpu::jit_framework::frontend::IsaFrontend;
use labwired_core::cpu::jit_framework::CodeView;
use labwired_core::cpu::CortexM;
use labwired_core::memory::LinearMemory;
use labwired_core::{Bus, DebugControl, Machine};

fn h(bytes: &mut Vec<u8>, half: u16) {
    bytes.extend_from_slice(&half.to_le_bytes());
}

fn movs(rd: u8, imm: u8) -> u16 {
    0x2000 | ((rd as u16) << 8) | imm as u16
}
fn mov_reg(rd: u8, rm: u8) -> u16 {
    0x4600 | ((rm as u16) << 3) | rd as u16
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
        if let Some(d) = compare_memory(units, interp.cpu.pc, &interp.bus, &jit.bus) {
            panic!(
                "JIT diverged from interpreter in RAM at unit {units} (retired {retired}): \
                 addr={:#x} interp={:#04x} jit={:#04x} pc={:#x}",
                d.address, d.interp, d.jit, d.pc
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
fn default_compile_floor_is_four() {
    assert_eq!(MIN_PROFITABLE_BLOCK_INSTRS, 4);
}

#[test]
fn eight_insn_loop_compiles_when_min_profitable_is_4() {
    let mut prog = Vec::new();
    for _ in 0..7 {
        h(&mut prog, adds_imm8(0, 1));
    }
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let mut interp = build_machine(&prog);
    let mut jit = build_machine(&prog);
    let mut engine = CortexMJitEngine::new(4);
    engine.set_min_profitable(4);
    engine.try_compile_from_bus(0, &jit.bus);
    assert!(
        engine.stats().compiled > 0,
        "8-insn loop must compile at min_profitable=4: {:?}",
        engine.stats()
    );
    assert_eq!(engine.ready_instr_count(0), Some(8));

    let policy = DiffPolicy {
        ignore_indices: differential_cycle_ignore_indices(),
        block_boundary_only: false,
    };
    let mut retired = 0u64;
    for units in 1..=2_000u64 {
        let n = engine.step_unit(&mut jit);
        assert!(n > 0, "halt at unit {units}");
        for _ in 0..n {
            interp.step().expect("interp 8-insn loop");
        }
        retired += n as u64;
        if let Some(d) = compare(
            units,
            &snapshot_state(&interp.cpu),
            &snapshot_state(&jit.cpu),
            &policy,
        ) {
            panic!("8-insn loop diverged at unit {units}: {d:?}");
        }
        if let Some(d) = compare_memory(units, interp.cpu.pc, &interp.bus, &jit.bus) {
            panic!(
                "8-insn loop diverged in RAM at unit {units}: addr={:#x} interp={:#04x} \
                 jit={:#04x} pc={:#x}",
                d.address, d.interp, d.jit, d.pc
            );
        }
        if retired > 400 {
            break;
        }
    }
    assert!(engine.stats().block_runs > 0);
    assert_eq!(engine.stats().ram_bytes_synced, 0);
}

#[test]
fn machine_run_chains_compiled_self_loop() {
    let mut prog = Vec::new();
    for _ in 0..7 {
        h(&mut prog, adds_imm8(0, 1));
    }
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let mut machine = build_machine(&prog);
    machine.config.cortex_m_jit_enabled = true;
    machine.bus.config.cortex_m_jit_enabled = true;
    machine.config.cortex_m_jit_min_block_instrs = 4;
    machine.config.peripheral_tick_interval = 64;
    machine.bus.config.peripheral_tick_interval = 64;
    for _ in 0..200 {
        machine.run(Some(64)).expect("heat");
        if machine.cpu.jit_stats().map(|s| s.chained).unwrap_or(0) > 0 {
            break;
        }
    }
    let stats = machine.cpu.jit_stats().expect("engine");
    assert!(
        stats.chained > 0,
        "self-loop should chain compiled blocks: {stats:?}"
    );
    assert!(stats.block_runs > 1);
}

#[test]
fn it_does_not_disable_compiled_remainder_of_batch() {
    let mut prog = Vec::new();
    for _ in 0..7 {
        h(&mut prog, adds_imm8(0, 1));
    }
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let mut machine = build_machine(&prog);
    machine.config.cortex_m_jit_enabled = true;
    machine.bus.config.cortex_m_jit_enabled = true;
    machine.config.cortex_m_jit_min_block_instrs = 4;
    machine.config.peripheral_tick_interval = 64;
    machine.bus.config.peripheral_tick_interval = 64;
    for _ in 0..200 {
        machine.run(Some(64)).expect("heat");
        if machine.cpu.jit_stats().map(|s| s.block_runs).unwrap_or(0) > 0 {
            break;
        }
    }
    let runs_before = machine.cpu.jit_stats().map(|s| s.block_runs).unwrap_or(0);
    assert!(runs_before > 0, "loop must compile before IT probe");
    // One remaining AL-predicated instruction: interpret that one, then
    // keep dispatching compiled blocks in the same batch.
    machine.cpu.it_state = 0xE8;
    machine.run(Some(64)).expect("post-it");
    let stats = machine.cpu.jit_stats().expect("engine");
    assert_eq!(machine.cpu.it_state, 0, "IT must be consumed");
    assert!(
        stats.block_runs > runs_before,
        "IT must not disable the rest of the batch: before={runs_before} after={stats:?}"
    );
}

fn vadd_s0() -> (u16, u16) {
    (0xEE30, 0x0A00)
}

#[test]
fn vadd_f32_matches_interpreter() {
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    let (a, b) = vadd_s0();
    h(&mut prog, a);
    h(&mut prog, b);
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let probe = build_machine(&prog);
    let frontend = CortexMFrontend::with_ram_window(
        probe.bus.ram.base_addr as u32,
        probe.bus.ram.data.len() as u32,
    );
    let view = CodeView::new(0, &prog);
    let (plan, _binding) = frontend
        .translate_block_thumb(0, &view)
        .expect("translate VADD");
    assert!(
        !plan.code.is_empty(),
        "VADD block must emit wasm, instrs={}",
        plan.instr_count
    );
    wasmtime::Module::new(&wasmtime::Engine::default(), &plan.code)
        .expect("VADD wasm module must validate");
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &probe.bus);
    assert!(
        engine.stats().compiled > 0,
        "VADD must instantiate: {:?}",
        engine.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.fpu_s[0] = 1.0f32.to_bits();
    });
    assert!(
        engine.stats().block_runs > 0,
        "VADD must compile: {:?}",
        engine.stats()
    );
    assert_eq!(interp.cpu.fpu_s[0], jit.cpu.fpu_s[0]);
}

fn it_eq() -> u16 {
    0xBF08
}

fn it_eq_adds_loop() -> Vec<u8> {
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, it_eq());
    h(&mut prog, adds_imm8(0, 1));
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));
    prog
}

#[test]
fn it_eq_adds_compiles_as_one_block() {
    let prog = it_eq_adds_loop();
    let jit = build_machine(&prog);
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &jit.bus);
    assert_eq!(
        engine.ready_instr_count(0),
        Some(7),
        "IT EQ + ADDS must stay in the compiled block: {:?}",
        engine.stats()
    );
}

#[test]
fn it_eq_adds_taken_matches_interpreter() {
    let prog = it_eq_adds_loop();
    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.xpsr |= 1 << 30; // Z=1 so EQ is taken
        m.cpu.r0 = 0;
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(
        engine.ready_instr_count(0),
        Some(7),
        "taken path must run the compiled IT block: {:?}",
        engine.stats()
    );
    assert_eq!(interp.cpu.r0, jit.cpu.r0);
    assert_ne!(interp.cpu.r0, 0, "EQ-taken ADDS must increment r0");
    assert_eq!(
        interp.cpu.xpsr & 0xF000_0000,
        jit.cpu.xpsr & 0xF000_0000,
        "ADDS inside IT must not leak flags"
    );
}

#[test]
fn it_eq_adds_skipped_matches_interpreter() {
    let prog = it_eq_adds_loop();
    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.xpsr &= !(1 << 30); // Z=0 so EQ is skipped
        m.cpu.r0 = 5;
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(
        engine.ready_instr_count(0),
        Some(7),
        "skipped path must run the compiled IT block: {:?}",
        engine.stats()
    );
    assert_eq!(interp.cpu.r0, jit.cpu.r0);
    assert_eq!(interp.cpu.r0, 5, "EQ-skipped ADDS must leave r0");
}

fn ite_eq() -> u16 {
    0xBF0C
}

#[test]
fn ite_eq_adds_matches_interpreter() {
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, ite_eq());
    h(&mut prog, adds_imm8(0, 1));
    h(&mut prog, adds_imm8(1, 1));
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let jit = build_machine(&prog);
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &jit.bus);
    assert_eq!(
        engine.ready_instr_count(0),
        Some(8),
        "ITE EQ + two ADDS must compile: {:?}",
        engine.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.xpsr |= 1 << 30;
        m.cpu.r0 = 0;
        m.cpu.r1 = 0;
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.r0, jit.cpu.r0);
    assert_eq!(interp.cpu.r1, jit.cpu.r1);
    assert_ne!(interp.cpu.r0, 0, "ITE THEN (EQ) must run");
    assert_eq!(interp.cpu.r1, 0, "ITE ELSE (NE) must skip when Z=1");
}

fn itt_eq() -> u16 {
    0xBF04
}

#[test]
fn itt_eq_ands_adds_does_not_leak_flags() {
    // ITT EQ; ANDS r0, r1; ADDS r2, #1
    // r0=r1=1 → ANDS result 1. If JIT writes Z=0, the ADDS is skipped.
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, itt_eq());
    h(&mut prog, ands(0, 1));
    h(&mut prog, adds_imm8(2, 1));
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let probe = build_machine(&prog);
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine.ready_instr_count(0),
        Some(8),
        "ITT EQ + ANDS + ADDS must compile: {:?}",
        engine.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.xpsr |= 1 << 30;
        m.cpu.r0 = 1;
        m.cpu.r1 = 1;
        m.cpu.r2 = 0;
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.r2, jit.cpu.r2);
    assert_ne!(interp.cpu.r2, 0, "EQ-taken ADDS after ANDS must run");
    assert_eq!(
        interp.cpu.xpsr & (1 << 30),
        1 << 30,
        "ANDS inside IT must leave Z set"
    );
    assert_eq!(interp.cpu.xpsr & 0xF000_0000, jit.cpu.xpsr & 0xF000_0000);
}

/// Bus-resident memory the JIT's RAM binding does not cover: every access to
/// it side-exits to the interpreter, but the write still lands on a real
/// `LinearMemory`, so the bus write counter and read-back both observe it.
const OFF_WINDOW_MMIO: u32 = 0x4000_0000;

fn attach_off_window_mem(m: &mut Machine<CortexM>) {
    m.bus
        .extra_mem
        .push(LinearMemory::new(0x1000, u64::from(OFF_WINDOW_MMIO)));
}

fn off_window_word(m: &Machine<CortexM>) -> u32 {
    u32::from_le_bytes(m.bus.extra_mem[0].data[0..4].try_into().unwrap())
}

fn itt_eq_adds_str_loop() -> Vec<u8> {
    // ITT EQ; ADDS r2, #1; STR r0, [r1]. The store is the second (last)
    // predicated instruction: a side-exit there must resume with the IT state
    // as of BEFORE the store, or the interpreter replays it unpredicated.
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, itt_eq());
    h(&mut prog, adds_imm8(2, 1));
    h(&mut prog, str_imm(0, 1, 0));
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));
    prog
}

#[test]
fn itt_eq_adds_str_out_of_window_compiles() {
    let prog = itt_eq_adds_str_loop();
    let probe = build_machine(&prog);
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine.ready_instr_count(0),
        Some(8),
        "ITT EQ + ADDS + out-of-window STR must compile as one block: {:?}",
        engine.stats()
    );
}

#[test]
fn itt_eq_adds_str_out_of_window_taken_resumes_predicated() {
    let prog = itt_eq_adds_str_loop();
    let seed = |m: &mut Machine<CortexM>| {
        attach_off_window_mem(m);
        m.cpu.xpsr |= 1 << 30; // Z=1 → EQ taken
        m.cpu.r0 = 0xA0A0_0000;
        m.cpu.r1 = OFF_WINDOW_MMIO;
        m.cpu.r2 = 0;
    };
    let (interp, jit, engine) = lockstep_until_compiled(&prog, seed);
    assert!(
        engine.stats().block_runs > 0,
        "out-of-window IT block must run: {:?}",
        engine.stats()
    );
    assert_eq!(interp.cpu.r2, jit.cpu.r2, "ADDS inside the IT body");
    assert_ne!(
        interp.cpu.r2, 0,
        "EQ-taken ADDS before the faulting STR must retire"
    );
    assert_eq!(
        snapshot_state(&interp.cpu),
        snapshot_state(&jit.cpu),
        "full arch state after resuming the predicated STR"
    );
    let (_, iw, _) = interp.bus.access_counts();
    let (_, jw, _) = jit.bus.access_counts();
    assert_eq!(iw, jw, "each lane must land the STR exactly once per pass");
    assert_eq!(off_window_word(&interp), 0xA0A0_0000);
    assert_eq!(off_window_word(&jit), 0xA0A0_0000);
}

#[test]
fn itt_eq_adds_str_out_of_window_skipped_does_not_store() {
    let prog = itt_eq_adds_str_loop();
    let seed = |m: &mut Machine<CortexM>| {
        attach_off_window_mem(m);
        m.cpu.xpsr &= !(1 << 30); // Z=0 → EQ false
        m.cpu.r0 = 0xA0A0_0000;
        m.cpu.r1 = OFF_WINDOW_MMIO;
        m.cpu.r2 = 0;
    };
    let (interp, jit, engine) = lockstep_until_compiled(&prog, seed);
    assert!(
        engine.stats().block_runs > 0,
        "skipped-predicate IT block must still compile and run: {:?}",
        engine.stats()
    );
    assert_eq!(interp.cpu.r2, 0, "EQ-false ADDS must be skipped");
    assert_eq!(jit.cpu.r2, 0, "EQ-false ADDS must be skipped");
    assert_eq!(
        snapshot_state(&interp.cpu),
        snapshot_state(&jit.cpu),
        "full arch state after the skipped predicated STR"
    );
    assert_eq!(
        off_window_word(&jit),
        0,
        "EQ-false STR must not execute in either lane"
    );
    assert_eq!(off_window_word(&interp), 0);
}

#[test]
fn itt_eq_adds_str_in_window_runs_to_completion() {
    // Control: the same IT body with an in-window target stays entirely in
    // compiled code (no side-exit) and must store once.
    let prog = itt_eq_adds_str_loop();
    let seed = |m: &mut Machine<CortexM>| {
        m.cpu.xpsr |= 1 << 30;
        m.cpu.r0 = 0xB0B0_0001;
        m.cpu.r1 = m.bus.ram.base_addr as u32 + 0x100;
        m.cpu.r2 = 0;
    };
    let (interp, jit, engine) = lockstep_until_compiled(&prog, seed);
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.r2, jit.cpu.r2);
    assert_ne!(interp.cpu.r2, 0, "EQ-taken ADDS must retire");
    assert_eq!(
        snapshot_state(&interp.cpu),
        snapshot_state(&jit.cpu),
        "full arch state after an in-window predicated store"
    );
    let word_at =
        |m: &Machine<CortexM>| u32::from_le_bytes(m.bus.ram.data[0x100..0x104].try_into().unwrap());
    assert_eq!(word_at(&interp), 0xB0B0_0001);
    assert_eq!(word_at(&jit), 0xB0B0_0001);
}

#[test]
fn ite_eq_str_then_adds_keeps_else_predicated() {
    // ITE EQ; STR r0, [r1] (out of window → side-exit); ADDS r2, #1 (ELSE).
    // With Z=1 the THEN store runs and the ELSE ADDS must stay skipped. A
    // resume that resets it_state to 0 would run the ADDS unconditionally:
    // this is the case the IT-state restore exists for.
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, ite_eq());
    h(&mut prog, str_imm(0, 1, 0));
    h(&mut prog, adds_imm8(2, 1));
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let probe = build_machine(&prog);
    let mut engine_probe = CortexMJitEngine::new(4);
    engine_probe.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine_probe.ready_instr_count(0),
        Some(8),
        "ITE EQ + STR + ADDS must compile as one block: {:?}",
        engine_probe.stats()
    );

    let seed = |m: &mut Machine<CortexM>| {
        attach_off_window_mem(m);
        m.cpu.xpsr |= 1 << 30; // Z=1 → THEN (EQ) taken, ELSE (NE) skipped
        m.cpu.r0 = 0xC0C0_0002;
        m.cpu.r1 = OFF_WINDOW_MMIO;
        m.cpu.r2 = 0;
    };
    let (interp, jit, engine) = lockstep_until_compiled(&prog, seed);
    assert!(engine.stats().block_runs > 0);
    assert_eq!(
        interp.cpu.r2, 0,
        "ELSE after the faulting THEN must stay skipped"
    );
    assert_eq!(
        jit.cpu.r2, 0,
        "ELSE after the faulting THEN must stay skipped"
    );
    assert_eq!(
        snapshot_state(&interp.cpu),
        snapshot_state(&jit.cpu),
        "full arch state after the ITE side-exit"
    );
    let (_, iw, _) = interp.bus.access_counts();
    let (_, jw, _) = jit.bus.access_counts();
    assert_eq!(iw, jw, "each lane must land the STR exactly once per pass");
    assert_eq!(off_window_word(&jit), 0xC0C0_0002);
}

#[test]
fn itt_eq_ldr_out_of_window_resumes_predicated() {
    // ITT EQ; LDR r2, [r1]; ADDS r3, #1 — a predicated load that side-exits.
    // The interpreter resumption must load the value and still run the ADDS.
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, itt_eq());
    h(&mut prog, ldr_imm(2, 1, 0));
    h(&mut prog, adds_imm8(3, 1));
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let probe = build_machine(&prog);
    let mut engine_probe = CortexMJitEngine::new(4);
    engine_probe.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine_probe.ready_instr_count(0),
        Some(8),
        "ITT EQ + LDR + ADDS must compile as one block: {:?}",
        engine_probe.stats()
    );

    let seed = |m: &mut Machine<CortexM>| {
        attach_off_window_mem(m);
        m.bus
            .write_u32(u64::from(OFF_WINDOW_MMIO), 0xDEAD_BEEF)
            .expect("seed off-window word");
        m.cpu.xpsr |= 1 << 30; // Z=1 → EQ taken
        m.cpu.r1 = OFF_WINDOW_MMIO;
        m.cpu.r2 = 0;
        m.cpu.r3 = 0;
    };
    let (interp, jit, engine) = lockstep_until_compiled(&prog, seed);
    assert!(engine.stats().block_runs > 0);
    assert_eq!(
        interp.cpu.r2, 0xDEAD_BEEF,
        "resumed predicated LDR must load the off-window word"
    );
    assert_eq!(interp.cpu.r2, jit.cpu.r2);
    assert_ne!(interp.cpu.r3, 0, "the ADDS after the LDR must still run");
    assert_eq!(interp.cpu.r3, jit.cpu.r3);
    assert_eq!(
        snapshot_state(&interp.cpu),
        snapshot_state(&jit.cpu),
        "full arch state after the predicated LDR side-exit"
    );
}

#[test]
fn itt_with_store_locksteps_with_it_intact() {
    // Same program with an in-window target: the whole IT body (store
    // included) runs inside the compiled block, so `it_state` must still be
    // consumed (0) at the block boundary and the store must land predicated —
    // exactly what the interpreter does.
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, itt_eq());
    h(&mut prog, adds_imm8(0, 1));
    h(&mut prog, str_imm(0, 1, 0));
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.xpsr |= 1 << 30; // EQ taken
        m.cpu.r0 = 0;
        m.cpu.r1 = 0x2000_0100;
    });
    assert!(
        engine.stats().block_runs > 0,
        "IT+store must still run compiled nops"
    );
    assert_eq!(interp.cpu.r0, jit.cpu.r0);
    assert_eq!(interp.cpu.it_state, 0, "IT must be consumed");
    assert_eq!(jit.cpu.it_state, 0, "compiled batch must leave IT consumed");
    assert_eq!(
        interp.bus.read_u32(0x2000_0100).expect("interp store"),
        jit.bus.read_u32(0x2000_0100).expect("jit store"),
        "predicated store must land once, identically"
    );
}

#[test]
fn itt_eq_neq_orrs_adds_does_not_leak_flags() {
    // ITT EQ; ORRS r0, r1; ADDS r2, #1 — the pattern the interpreter had to
    // fix for `strls`: a logic op inside IT must not write APSR. r0=r1=1
    // makes ORRS yield 1; a leaked Z=0 would clear the EQ predicate and skip
    // the ADDS.
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, itt_eq());
    h(&mut prog, orrs(0, 1));
    h(&mut prog, adds_imm8(2, 1));
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let probe = build_machine(&prog);
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine.ready_instr_count(0),
        Some(8),
        "ITT EQ + ORRS + ADDS must compile: {:?}",
        engine.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.xpsr |= 1 << 30;
        m.cpu.r0 = 1;
        m.cpu.r1 = 1;
        m.cpu.r2 = 0;
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.r0, 1, "ORRS must still write its result");
    assert_eq!(interp.cpu.r2, jit.cpu.r2);
    assert_ne!(interp.cpu.r2, 0, "EQ-taken ADDS after ORRS must run");
    assert_eq!(
        interp.cpu.xpsr & (1 << 30),
        1 << 30,
        "ORRS inside IT must leave Z set"
    );
    assert_eq!(interp.cpu.xpsr & 0xF000_0000, jit.cpu.xpsr & 0xF000_0000);
}

#[test]
fn itt_eq_movs_adds_does_not_leak_flags() {
    // ITT EQ; MOVS r0, #7; ADDS r2, #1 — MOVS is the other T1 writer the
    // interpreter suppresses inside IT. Leaked flags would clear EQ before
    // the second instruction.
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, itt_eq());
    h(&mut prog, movs(0, 7));
    h(&mut prog, adds_imm8(2, 1));
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.xpsr |= 1 << 30;
        m.cpu.r0 = 0;
        m.cpu.r2 = 0;
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.r0, 7, "MOVS must still write its result");
    assert_eq!(interp.cpu.r2, jit.cpu.r2);
    assert_ne!(interp.cpu.r2, 0, "EQ-taken ADDS after MOVS must run");
    assert_eq!(
        interp.cpu.xpsr & (1 << 30),
        1 << 30,
        "MOVS inside IT must leave Z set"
    );
    assert_eq!(interp.cpu.xpsr & 0xF000_0000, jit.cpu.xpsr & 0xF000_0000);
}

fn vldr_s0_r0() -> (u16, u16) {
    (0xED90, 0x0A00)
}

fn vstr_s0_r0() -> (u16, u16) {
    (0xED80, 0x0A00)
}

#[test]
fn vldr_f32_compiles_and_matches_interpreter() {
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    let (a, b) = vldr_s0_r0();
    h(&mut prog, a);
    h(&mut prog, b);
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let bits = 1.0f32.to_bits();
    let mut probe = build_machine(&prog);
    probe.cpu.r0 = 0x2000_0000;
    probe
        .bus
        .write_u32(0x2000_0000, bits)
        .expect("seed VLDR word");
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine.ready_instr_count(0),
        Some(6),
        "VLDR must compile into the block: {:?}",
        engine.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.r0 = 0x2000_0000;
        m.bus.write_u32(0x2000_0000, bits).expect("seed");
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.fpu_s[0], jit.cpu.fpu_s[0]);
    assert_eq!(interp.cpu.fpu_s[0], bits);
}

#[test]
fn vstr_f32_compiles_and_matches_interpreter() {
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    let (a, b) = vstr_s0_r0();
    h(&mut prog, a);
    h(&mut prog, b);
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let bits = 2.0f32.to_bits();
    let mut probe = build_machine(&prog);
    probe.cpu.r0 = 0x2000_0000;
    probe.cpu.fpu_s[0] = bits;
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine.ready_instr_count(0),
        Some(6),
        "VSTR must compile into the block: {:?}",
        engine.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.r0 = 0x2000_0000;
        m.cpu.fpu_s[0] = bits;
    });
    assert!(engine.stats().block_runs > 0);
    let got_i = u32::from_le_bytes(interp.bus.ram.data[0..4].try_into().unwrap());
    let got_j = u32::from_le_bytes(jit.bus.ram.data[0..4].try_into().unwrap());
    assert_eq!(got_i, got_j);
    assert_eq!(got_i, bits);
}

// S2 <- S0 op S1: the destination is NOT a source, so the hot loop
// re-computes the same value and the expected result is stable.
fn vadd_s2_s0_s1() -> (u16, u16) {
    (0xEE30, 0x1A20)
}

fn vsub_s2_s0_s1() -> (u16, u16) {
    (0xEE30, 0x1A60)
}

fn vmul_s2_s0_s1() -> (u16, u16) {
    (0xEE20, 0x1A20)
}

fn vdiv_s2_s0_s1() -> (u16, u16) {
    (0xEE80, 0x1A20)
}

fn vmov_s1_s0() -> (u16, u16) {
    (0xEEF0, 0x0A40)
}

/// Shared harness for the S-ALU coverage: a hot loop whose only float work is
/// `op`, seeded with two non-zero operands (0/0 would hand the assert a NaN
/// payload, and wasm f32 vs Rust f32 do not promise NaN bits). `snapshot_state`
/// carries `fpu_s`, so `lockstep_until_compiled` compares the FULL register
/// file at every unit, not just the destination the test names.
fn vfp_binop_lockstep(op: (u16, u16), a: f32, b: f32, expect: f32) -> (f32, f32) {
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, op.0);
    h(&mut prog, op.1);
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let probe = build_machine(&prog);
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &probe.bus);
    assert!(
        engine.ready_instr_count(0) == Some(6),
        "S-ALU op must compile into the block: {:?}",
        engine.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.fpu_s[0] = a.to_bits();
        m.cpu.fpu_s[1] = b.to_bits();
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(
        f32::from_bits(jit.cpu.fpu_s[2]),
        expect,
        "compiled result must be the IEEE-754 op result"
    );
    assert_eq!(jit.cpu.fpu_s[2], interp.cpu.fpu_s[2]);
    (
        f32::from_bits(interp.cpu.fpu_s[2]),
        f32::from_bits(jit.cpu.fpu_s[2]),
    )
}

#[test]
fn vsub_f32_matches_interpreter() {
    let (i, j) = vfp_binop_lockstep(vsub_s2_s0_s1(), 3.5, 1.25, 2.25);
    assert_eq!(i, 2.25);
    assert_eq!(j, 2.25);
}

#[test]
fn vmul_f32_matches_interpreter() {
    let (i, j) = vfp_binop_lockstep(vmul_s2_s0_s1(), 3.5, 1.25, 4.375);
    assert_eq!(i, 4.375);
    assert_eq!(j, 4.375);
}

#[test]
fn vdiv_f32_matches_interpreter() {
    let (i, j) = vfp_binop_lockstep(vdiv_s2_s0_s1(), 3.5, 1.25, 2.8);
    assert_eq!(i, 2.8);
    assert_eq!(j, 2.8);
}

#[test]
fn vmov_f32_reg_matches_interpreter() {
    // VMOV.F32 S1, S0 — a pure register move; the differential snapshot
    // carries every S register, so a wrong destination is caught even when
    // the source stays intact.
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    let (a, b) = vmov_s1_s0();
    h(&mut prog, a);
    h(&mut prog, b);
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let bits = 3.5f32.to_bits();
    let probe = build_machine(&prog);
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &probe.bus);
    assert!(
        engine.ready_instr_count(0) == Some(6),
        "VMOV must compile into the block: {:?}",
        engine.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.fpu_s[0] = bits;
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.fpu_s[1], jit.cpu.fpu_s[1]);
    assert_eq!(jit.cpu.fpu_s[1], bits, "S1 must receive S0's bits");
}

/// VADD/VSUB/VMUL/VDIV S2 <- S0 op S1 hot loop under a seeded FPSCR, run
/// through the full lockstep harness (which compares `fpu_s` at every unit).
/// Returns the interpreter's and the compiled lane's raw S2 bits.
fn vfp_fpscr_lockstep(op: (u16, u16), a_bits: u32, b_bits: u32, fpscr: u32) -> (u32, u32) {
    let mut prog = Vec::new();
    for _ in 0..4 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, op.0);
    h(&mut prog, op.1);
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let probe = build_machine(&prog);
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine.ready_instr_count(0),
        Some(6),
        "S-ALU op must compile into the block: {:?}",
        engine.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.fpu_s[0] = a_bits;
        m.cpu.fpu_s[1] = b_bits;
        m.cpu.fpscr = fpscr;
    });
    assert!(engine.stats().block_runs > 0);
    (interp.cpu.fpu_s[2], jit.cpu.fpu_s[2])
}

#[test]
fn vfp_fz_flushes_denormal_inputs_lockstep() {
    // 2^-149 + 2^-148 = 0x0000_0003 with FZ off; both inputs are denormal
    // and flush to +0 with FZ on. VADD S2, S0, S1.
    let denorm_min = 0x0000_0001u32;
    let denorm_two = 0x0000_0002u32;
    let (i_off, j_off) = vfp_fpscr_lockstep(vadd_s2_s0_s1(), denorm_min, denorm_two, 0);
    assert_eq!(i_off, 0x0000_0003, "interpreter, FZ off");
    assert_eq!(j_off, 0x0000_0003, "compiled lane, FZ off");
    assert_eq!(i_off, j_off);

    let (i_on, j_on) = vfp_fpscr_lockstep(vadd_s2_s0_s1(), denorm_min, denorm_two, 1 << 24);
    assert_eq!(i_on, 0x0000_0000, "interpreter must flush denormal inputs");
    assert_eq!(
        j_on, 0x0000_0000,
        "compiled lane must flush denormal inputs"
    );
}

#[test]
fn vfp_fz_flushes_denormal_results_lockstep() {
    // 2^-64 * 2^-85 = 2^-149 = 0x0000_0001 (denormal) with FZ off; the
    // denormal result flushes to +0 with FZ on. VMUL S2, S0, S1.
    let two_pow_m64 = 0x1F80_0000u32;
    let two_pow_m85 = 0x1500_0000u32;
    let (i_off, j_off) = vfp_fpscr_lockstep(vmul_s2_s0_s1(), two_pow_m64, two_pow_m85, 0);
    assert_eq!(i_off, 0x0000_0001, "interpreter keeps the denormal result");
    assert_eq!(
        j_off, 0x0000_0001,
        "compiled lane keeps the denormal result"
    );
    assert_eq!(i_off, j_off);

    let (i_on, j_on) = vfp_fpscr_lockstep(vmul_s2_s0_s1(), two_pow_m64, two_pow_m85, 1 << 24);
    assert_eq!(i_on, 0x0000_0000, "interpreter must flush denormal results");
    assert_eq!(
        j_on, 0x0000_0000,
        "compiled lane must flush denormal results"
    );
}

#[test]
fn vfp_dn_defaults_nan_results_lockstep() {
    // VADD with a quiet NaN carrying a payload. DN off: the payload
    // propagates. DN on: the ARM default NaN, payload discarded.
    let qnan = 0x7FC0_AAAAu32;
    let one = (1.0f32).to_bits();
    let (i_off, j_off) = vfp_fpscr_lockstep(vadd_s2_s0_s1(), qnan, one, 0);
    assert_eq!(i_off, qnan, "interpreter propagates the NaN payload");
    assert_eq!(j_off, qnan, "compiled lane propagates the NaN payload");
    assert_eq!(i_off, j_off);

    let (i_on, j_on) = vfp_fpscr_lockstep(vadd_s2_s0_s1(), qnan, one, 1 << 25);
    assert_eq!(i_on, 0x7FC0_0000, "interpreter must emit the default NaN");
    assert_eq!(j_on, 0x7FC0_0000, "compiled lane must emit the default NaN");
}

#[test]
fn vfp_nan_payload_is_identical_across_lanes() {
    // Signaling NaN operand quieted, payload preserved.
    let snan = 0x7F80_0001u32;
    let (i, j) = vfp_fpscr_lockstep(vadd_s2_s0_s1(), snan, (1.0f32).to_bits(), 0);
    assert_eq!(i, 0x7FC0_0001);
    assert_eq!(j, 0x7FC0_0001);

    // Invalid operation with no NaN operand: both lanes land on the ARM
    // default NaN, not the host FPU's synthesized payload.
    let (i, j) = vfp_fpscr_lockstep(
        vmul_s2_s0_s1(),
        (0.0f32).to_bits(),
        f32::INFINITY.to_bits(),
        0,
    );
    assert_eq!(i, 0x7FC0_0000, "0 * inf is the default NaN");
    assert_eq!(j, 0x7FC0_0000);

    // Two NaN operands: the first one wins, deterministically, in both lanes.
    let (i, j) = vfp_fpscr_lockstep(vadd_s2_s0_s1(), 0x7FC0_1111, 0x7FC0_2222, 0);
    assert_eq!(i, 0x7FC0_1111);
    assert_eq!(j, 0x7FC0_1111);
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
        let mut fpu = [0u32; 32];
        let (exit, n, _, _) = block.run(&mut x, &mut [], &mut fpu);
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
    assert_eq!(
        stats.ram_bytes_synced, 0,
        "mem blocks must not memcpy guest RAM: {stats:?}"
    );
    assert_eq!(interp.cpu.r0, jit.cpu.r0);
}

#[test]
fn ldr_only_block_does_not_need_writeback_lockstep() {
    // Store-free mem block: 16 nops + in-window LDR + b. Seed still happens
    // (loads read wasm RAM); writeback is skipped when `!has_store`. Host RAM
    // matching a pre-run clone is true with or without the skip (writeback of
    // identical bytes is a no-op).
    let mut prog = Vec::new();
    for _ in 0..16 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, ldr_imm(2, 1, 0));
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let probe = build_machine(&prog);
    let ram_base = probe.bus.ram.base_addr as u32;
    let ram_len = probe.bus.ram.data.len() as u32;
    let frontend = CortexMFrontend::with_ram_window(ram_base, ram_len);
    let view = CodeView::new(0, &prog);
    let (plan, binding) = frontend
        .translate_block_thumb(0, &view)
        .expect("translate LDR-only");
    assert_eq!(plan.instr_count, 18, "16 nops + LDR + b");
    assert!(
        !binding.expect("LDR binds RAM").has_store,
        "LDR-only block must not set has_store"
    );

    let mut engine_probe = CortexMJitEngine::new(4);
    engine_probe.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine_probe.ready_instr_count(0),
        Some(18),
        "LDR-only must compile: {:?}",
        engine_probe.stats()
    );

    let seed = |m: &mut Machine<CortexM>| {
        m.cpu.r1 = m.bus.ram.base_addr as u32;
        m.bus
            .write_u32(u64::from(m.cpu.r1), 0xA5A5_5A5A)
            .expect("seed RAM word");
    };

    let mut jit = build_machine(&prog);
    seed(&mut jit);
    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &jit.bus);
    let orig_ram = jit.bus.ram.data.clone();
    let n = engine.step_unit(&mut jit);
    assert!(
        engine.stats().block_runs > 0,
        "LDR-only never ran compiled: {:?}",
        engine.stats()
    );
    assert_eq!(n, 18, "compiled LDR-only block retired {n}");
    assert_eq!(
        jit.bus.ram.data, orig_ram,
        "store-free compiled block must leave host RAM unchanged"
    );
    assert_eq!(jit.cpu.r2, 0xA5A5_5A5A, "LDR must observe seeded RAM");

    let (interp, jit, engine) = lockstep_until_compiled(&prog, seed);
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.r2, jit.cpu.r2);
    assert_eq!(interp.cpu.r2, 0xA5A5_5A5A);
    assert_eq!(interp.cpu.pc, jit.cpu.pc);
}

fn ldr_imm32(rt: u8, rn: u8, imm12: u16) -> (u16, u16) {
    (
        0xF8D0 | (rn as u16 & 0xF),
        ((rt as u16 & 0xF) << 12) | (imm12 & 0xFFF),
    )
}

fn lockstep_until_compiled(
    prog: &[u8],
    seed: impl Fn(&mut Machine<CortexM>),
) -> (Machine<CortexM>, Machine<CortexM>, CortexMJitEngine) {
    let mut interp = build_machine(prog);
    let mut jit = build_machine(prog);
    seed(&mut interp);
    seed(&mut jit);
    let mut engine = CortexMJitEngine::new(4);
    let policy = DiffPolicy {
        ignore_indices: differential_cycle_ignore_indices(),
        block_boundary_only: false,
    };
    let mut retired = 0u64;
    for units in 1..=8_000u64 {
        let n = engine.step_unit(&mut jit);
        assert!(n > 0, "halt at unit {units}");
        for _ in 0..n {
            interp.step().expect("interpreter must not fault");
        }
        retired += n as u64;
        if let Some(d) = compare(
            units,
            &snapshot_state(&interp.cpu),
            &snapshot_state(&jit.cpu),
            &policy,
        ) {
            panic!(
                "JIT diverged from interpreter at unit {units} (retired {retired}): {d:?}\n\
                 interp pc={:#x} r0={:#x} r1={:#x} xpsr={:#x}\n\
                 jit    pc={:#x} r0={:#x} r1={:#x} xpsr={:#x}",
                interp.cpu.pc,
                interp.cpu.r0,
                interp.cpu.r1,
                interp.cpu.xpsr,
                jit.cpu.pc,
                jit.cpu.r0,
                jit.cpu.r1,
                jit.cpu.xpsr
            );
        }
        if let Some(d) = compare_memory(units, interp.cpu.pc, &interp.bus, &jit.bus) {
            panic!(
                "JIT diverged from interpreter in RAM at unit {units} (retired {retired}): \
                 addr={:#x} interp={:#04x} jit={:#04x} pc={:#x}",
                d.address, d.interp, d.jit, d.pc
            );
        }
        if engine.stats().block_runs > 0 && retired > 32 {
            break;
        }
    }
    assert!(
        engine.stats().block_runs > 0,
        "block never compiled: {:?}",
        engine.stats()
    );
    (interp, jit, engine)
}

fn pad_alu_then(op: u16, nops: usize) -> Vec<u8> {
    let mut prog = Vec::new();
    h(&mut prog, mov_reg(0, 1));
    for _ in 0..nops {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, op);
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));
    prog
}

#[test]
fn adds_rd_eq_rm_overflow_matches_interpreter() {
    // Reload r0 from r1 each loop so the compiled block's last flag-setter
    // is `adds r0, r0, r0` on 0x4000_0000 (signed overflow), not a later 0+0.
    let prog = pad_alu_then(add_reg(0, 0, 0), 14);
    let (interp, jit, _engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.r1 = 0x4000_0000;
    });
    assert_ne!(
        interp.cpu.xpsr & (1 << 28),
        0,
        "test must actually set V (signed overflow)"
    );
    assert_eq!(
        interp.cpu.xpsr & 0xF000_0000,
        jit.cpu.xpsr & 0xF000_0000,
        "NZCV after adds r0, r0, r0 overflow"
    );
}

#[test]
fn subs_rd_eq_rm_matches_interpreter() {
    let prog = pad_alu_then(sub_reg(0, 2, 0), 14);
    let (interp, jit, _engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.r1 = 0x8000_0000;
        m.cpu.r2 = 0x8000_0000;
    });
    assert_eq!(
        interp.cpu.xpsr & 0xF000_0000,
        jit.cpu.xpsr & 0xF000_0000,
        "NZCV after subs r0, r2, r0 with rd==rm"
    );
}

#[test]
fn add_high_from_pc_matches_interpreter() {
    let mut prog = Vec::new();
    for _ in 0..16 {
        h(&mut prog, adds_imm8(1, 1));
    }
    h(&mut prog, 0x4478); // ADD r0, r15
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let (interp, jit, _engine) = lockstep_until_compiled(&prog, |_| {});
    assert_eq!(
        interp.cpu.r0, jit.cpu.r0,
        "ADD r0, pc must use raw insn PC, not PC+4"
    );
}

#[test]
fn ldr_imm32_to_pc_is_compiled_terminator() {
    use labwired_core::cpu::jit_framework::cortex_m::emit::{
        is_mem_emittable, is_terminator_emittable,
    };
    use labwired_core::decoder::arm::Instruction;

    assert!(
        !is_mem_emittable(&Instruction::LdrImm32 {
            rt: 15,
            rn: 0,
            imm12: 0
        }),
        "LDR.W PC must not be mem-emittable (it is a terminator)"
    );
    assert!(
        is_terminator_emittable(&Instruction::LdrImm32 {
            rt: 15,
            rn: 0,
            imm12: 0
        }),
        "LDR.W PC must be a compiled terminator"
    );

    let mut prog = Vec::new();
    for _ in 0..16 {
        h(&mut prog, adds_imm8(1, 1));
    }
    let (h1, h2) = ldr_imm32(15, 0, 0);
    h(&mut prog, h1);
    h(&mut prog, h2);

    let (interp, jit, _engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.r0 = 0x2000_0000;
        m.bus.write_u32(0x2000_0000, 1).expect("thumb target at 0");
    });
    assert_eq!(
        interp.cpu.pc, jit.cpu.pc,
        "LDR.W PC must match interpreter (interworking branch, not write(15))"
    );
}

fn push_regs(registers: u8, m: bool) -> u16 {
    0xB400 | ((u16::from(m)) << 8) | u16::from(registers)
}

#[test]
fn push_out_of_window_matches_interpreter() {
    // 16 ALU nops + PUSH {r0-r7} compiles as one block (PUSH is mem-emittable).
    // SP sits at ram.base+16 so the first PUSH slots would be in-window if
    // stored incrementally, but the last of the 8 words is below RAM.
    let mut prog = Vec::new();
    for _ in 0..16 {
        h(&mut prog, 0xBF00); // nop
    }
    h(&mut prog, push_regs(0xFF, false)); // PUSH {r0-r7}

    let mut interp = build_machine(&prog);
    let mut jit = build_machine(&prog);
    let ram_base = jit.bus.ram.base_addr as u32;
    let orig_sp = ram_base + 16;
    let seed = |m: &mut Machine<CortexM>, sp: u32| {
        m.cpu.sp = sp;
        m.cpu.r0 = 0xA0A0_0000;
        m.cpu.r1 = 0xA1A1_0001;
        m.cpu.r2 = 0xA2A2_0002;
        m.cpu.r3 = 0xA3A3_0003;
        m.cpu.r4 = 0xA4A4_0004;
        m.cpu.r5 = 0xA5A5_0005;
        m.cpu.r6 = 0xA6A6_0006;
        m.cpu.r7 = 0xA7A7_0007;
    };
    seed(&mut interp, orig_sp);
    seed(&mut jit, orig_sp);

    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &jit.bus);
    assert!(
        engine.stats().compiled > 0,
        "16 ALU + PUSH must compile: {:?}",
        engine.stats()
    );

    let orig_ram = jit.bus.ram.data.clone();
    let n = engine.step_unit(&mut jit);
    assert!(
        engine.stats().block_runs > 0,
        "PUSH was not in a compiled block: {:?}",
        engine.stats()
    );
    assert!(
        engine.stats().block_instrs > 0,
        "compiled block retired nothing: {:?}",
        engine.stats()
    );
    assert_eq!(
        n, 16,
        "PUSH must mem-fault after the 16-ALU prefix (retired={n})"
    );

    for _ in 0..n {
        interp.step().expect("ALU prefix must not fault");
    }

    assert_eq!(interp.cpu.pc, 32, "resume PC is the PUSH");
    assert_eq!(
        jit.cpu.pc, interp.cpu.pc,
        "JIT resume PC must match interpreter"
    );
    assert_eq!(
        interp.cpu.sp, orig_sp,
        "interpreter SP is unchanged; PUSH has not committed r13"
    );
    assert_eq!(
        jit.cpu.sp, orig_sp,
        "JIT must not leave SP decremented on an out-of-window PUSH side-exit"
    );
    assert_eq!(
        jit.cpu.sp, interp.cpu.sp,
        "SP must match after PUSH side-exit"
    );
    assert_eq!(
        jit.bus.ram.data, interp.bus.ram.data,
        "JIT must not store any PUSH slot before the out-of-window side-exit"
    );
    assert_eq!(
        jit.bus.ram.data, orig_ram,
        "faulting compiled PUSH must leave RAM unchanged"
    );
}

fn pop_regs(registers: u8, p: bool) -> u16 {
    0xBC00 | ((u16::from(p)) << 8) | u16::from(registers)
}

#[test]
fn pop_out_of_window_matches_interpreter() {
    // Mirror of push_out_of_window: 16 nops + POP {r0-r7}. SP sits 16 bytes
    // below RAM end so the first slots are in-window if loaded incrementally,
    // but the last of the 8 words is past ram_end. JIT must not increment SP
    // (or commit dest regs) before the side-exit.
    let mut prog = Vec::new();
    for _ in 0..16 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, pop_regs(0xFF, false)); // POP {r0-r7}

    let mut interp = build_machine(&prog);
    let mut jit = build_machine(&prog);
    let ram_base = jit.bus.ram.base_addr as u32;
    let ram_len = jit.bus.ram.data.len() as u32;
    let orig_sp = ram_base + ram_len - 16;
    let seed = |m: &mut Machine<CortexM>, sp: u32| {
        m.cpu.sp = sp;
        m.cpu.r0 = 0;
        m.cpu.r1 = 0;
        m.cpu.r2 = 0;
        m.cpu.r3 = 0;
        m.cpu.r4 = 0;
        m.cpu.r5 = 0;
        m.cpu.r6 = 0;
        m.cpu.r7 = 0;
    };
    seed(&mut interp, orig_sp);
    seed(&mut jit, orig_sp);

    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &jit.bus);
    assert!(
        engine.stats().compiled > 0,
        "16 ALU + POP must compile: {:?}",
        engine.stats()
    );

    let r0_before = jit.cpu.r0;
    let n = engine.step_unit(&mut jit);
    assert!(
        engine.stats().block_runs > 0,
        "POP was not in a compiled block: {:?}",
        engine.stats()
    );
    assert_eq!(
        n, 16,
        "POP must mem-fault after the 16-ALU prefix (retired={n})"
    );

    for _ in 0..n {
        interp.step().expect("ALU prefix must not fault");
    }

    assert_eq!(interp.cpu.pc, 32, "resume PC is the POP");
    assert_eq!(
        jit.cpu.pc, interp.cpu.pc,
        "JIT resume PC must match interpreter"
    );
    assert_eq!(
        interp.cpu.sp, orig_sp,
        "interpreter SP is unchanged; POP has not committed r13"
    );
    assert_eq!(
        jit.cpu.sp, orig_sp,
        "JIT must not leave SP incremented on an out-of-window POP side-exit"
    );
    assert_eq!(
        jit.cpu.r0, r0_before,
        "JIT must not commit dest regs before the out-of-window POP side-exit"
    );
    assert_eq!(
        snapshot_state(&interp.cpu),
        snapshot_state(&jit.cpu),
        "arch state after POP side-exit"
    );
}

#[test]
fn pop_pc_terminator_matches_interpreter() {
    // 16 nops + POP {PC}. Without the terminator in is_terminator_emittable
    // the compiled block is 16 nops (fall-through); with it, 17 insns chain
    // to the stacked Thumb return address.
    let mut prog = Vec::new();
    for _ in 0..16 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, pop_regs(0, true)); // POP {PC} = 0xBD00

    let mut interp = build_machine(&prog);
    let mut jit = build_machine(&prog);
    let ram_base = jit.bus.ram.base_addr as u32;
    let stacked = 1u32; // Thumb return to PC=0
    let seed = |m: &mut Machine<CortexM>| {
        m.cpu.sp = ram_base;
        for i in 0..256u32 {
            m.bus
                .write_u32(u64::from(ram_base + i * 4), stacked)
                .expect("stack a Thumb return address");
        }
    };
    seed(&mut interp);
    seed(&mut jit);

    let mut engine = CortexMJitEngine::new(4);
    engine.try_compile_from_bus(0, &jit.bus);
    assert_eq!(
        engine.ready_instr_count(0),
        Some(17),
        "POP {{PC}} must be a compiled terminator: {:?}",
        engine.stats()
    );

    let policy = DiffPolicy {
        ignore_indices: differential_cycle_ignore_indices(),
        block_boundary_only: false,
    };
    let mut retired = 0u64;
    for units in 1..=8_000u64 {
        let n = engine.step_unit(&mut jit);
        assert!(n > 0, "halt at unit {units}");
        for _ in 0..n {
            interp.step().expect("interpreter must not fault");
        }
        retired += n as u64;
        if let Some(d) = compare(
            units,
            &snapshot_state(&interp.cpu),
            &snapshot_state(&jit.cpu),
            &policy,
        ) {
            panic!(
                "POP PC diverged at unit {units} (retired {retired}): {d:?}\n\
                 interp pc={:#x} sp={:#x}\n jit    pc={:#x} sp={:#x}",
                interp.cpu.pc, interp.cpu.sp, jit.cpu.pc, jit.cpu.sp
            );
        }
        if let Some(d) = compare_memory(units, interp.cpu.pc, &interp.bus, &jit.bus) {
            panic!(
                "POP PC diverged in RAM at unit {units} (retired {retired}): addr={:#x} \
                 interp={:#04x} jit={:#04x} pc={:#x}",
                d.address, d.interp, d.jit, d.pc
            );
        }
        if engine.stats().block_runs > 0 && retired > 32 {
            break;
        }
    }
    assert!(
        engine.stats().block_runs > 0,
        "POP PC block never ran: {:?}",
        engine.stats()
    );
    assert_eq!(interp.cpu.pc, jit.cpu.pc, "JIT PC must match interpreter");
    assert_eq!(interp.cpu.sp, jit.cpu.sp, "SP after POP PC");
}

fn stm(rn: u8, registers: u8) -> u16 {
    0xC000 | ((rn as u16) << 8) | u16::from(registers)
}
fn ldm(rn: u8, registers: u8) -> u16 {
    0xC800 | ((rn as u16) << 8) | u16::from(registers)
}

#[test]
fn ldm_stm_roundtrip_matches_interpreter() {
    // 16 nops + STM r1!, {r0,r2,r3} + LDM r4!, {r5,r6,r7} + b back.
    // r1 and r4 start at the same RAM buffer; roundtrip r5..r7 == r0,r2,r3.
    let mut prog = Vec::new();
    for _ in 0..16 {
        h(&mut prog, 0xBF00);
    }
    h(&mut prog, stm(1, 0b0000_1101)); // STM r1!, {r0, r2, r3}
    h(&mut prog, ldm(4, 0b1110_0000)); // LDM r4!, {r5, r6, r7}
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let mut engine_probe = CortexMJitEngine::new(4);
    let probe = build_machine(&prog);
    engine_probe.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine_probe.ready_instr_count(0),
        Some(19),
        "STM/LDM must be inside the compiled block: {:?}",
        engine_probe.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        let ram_base = m.bus.ram.base_addr as u32;
        m.cpu.r1 = ram_base;
        m.cpu.r4 = ram_base;
        m.cpu.r0 = 0xA0A0_0000;
        m.cpu.r2 = 0xA2A2_0002;
        m.cpu.r3 = 0xA3A3_0003;
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.r5, jit.cpu.r5);
    assert_eq!(interp.cpu.r6, jit.cpu.r6);
    assert_eq!(interp.cpu.r7, jit.cpu.r7);
    assert_eq!(interp.cpu.r1, jit.cpu.r1, "STM writeback");
    assert_eq!(interp.cpu.r4, jit.cpu.r4, "LDM writeback");
    assert_eq!(interp.cpu.sp, jit.cpu.sp);
    assert_eq!(interp.cpu.r5, interp.cpu.r0);
    assert_eq!(interp.cpu.r6, interp.cpu.r2);
    assert_eq!(interp.cpu.r7, interp.cpu.r3);
}

fn stmdb_w(rn: u8, reg_list: u16, writeback: bool) -> (u16, u16) {
    let mut h1 = 0xE800 | u16::from(rn & 0xF);
    if writeback {
        h1 |= 0x20;
    }
    (h1, reg_list)
}
fn ldmia_w(rn: u8, reg_list: u16, writeback: bool) -> (u16, u16) {
    let mut h1 = 0xE890 | u16::from(rn & 0xF);
    if writeback {
        h1 |= 0x20;
    }
    (h1, reg_list)
}

#[test]
fn ldmia_w_stmdb_w_roundtrip_matches_interpreter() {
    // 16 nops + STMDB.W r1!, {r0,r2} + LDMIA.W r1!, {r3,r4} + b back.
    let mut prog = Vec::new();
    for _ in 0..16 {
        h(&mut prog, 0xBF00);
    }
    let (s1, s2) = stmdb_w(1, 0x0005, true);
    h(&mut prog, s1);
    h(&mut prog, s2);
    let (l1, l2) = ldmia_w(1, 0x0018, true);
    h(&mut prog, l1);
    h(&mut prog, l2);
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let probe = build_machine(&prog);
    let mut engine_probe = CortexMJitEngine::new(4);
    engine_probe.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine_probe.ready_instr_count(0),
        Some(19),
        "LDMIA.W/STMDB.W must be inside the compiled block: {:?}",
        engine_probe.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.r1 = m.bus.ram.base_addr as u32 + 64;
        m.cpu.r0 = 0x1111_0000;
        m.cpu.r2 = 0x2222_0002;
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.r3, jit.cpu.r3);
    assert_eq!(interp.cpu.r4, jit.cpu.r4);
    assert_eq!(interp.cpu.r1, jit.cpu.r1);
    assert_eq!(interp.cpu.r3, interp.cpu.r0);
    assert_eq!(interp.cpu.r4, interp.cpu.r2);
}

fn adc(rd: u8, rm: u8) -> u16 {
    0x4140 | ((rm as u16) << 3) | rd as u16
}
fn sbc(rd: u8, rm: u8) -> u16 {
    0x4180 | ((rm as u16) << 3) | rd as u16
}
fn rsbs(rd: u8, rn: u8) -> u16 {
    0x4240 | ((rn as u16) << 3) | rd as u16
}
fn lsl_reg(rd: u8, rm: u8) -> u16 {
    0x4080 | ((rm as u16) << 3) | rd as u16
}
fn lsr_reg(rd: u8, rm: u8) -> u16 {
    0x40C0 | ((rm as u16) << 3) | rd as u16
}
fn asr_reg(rd: u8, rm: u8) -> u16 {
    0x4100 | ((rm as u16) << 3) | rd as u16
}
fn ror_reg(rd: u8, rm: u8) -> u16 {
    0x41C0 | ((rm as u16) << 3) | rd as u16
}
fn mul32(rd: u8, rn: u8, rm: u8) -> (u16, u16) {
    (
        0xFB00 | u16::from(rn & 0xF),
        0xF000 | ((rd as u16 & 0xF) << 8) | u16::from(rm & 0xF),
    )
}

fn assert_nzcv(interp: &Machine<CortexM>, jit: &Machine<CortexM>, what: &str) {
    assert_eq!(
        interp.cpu.xpsr & 0xF000_0000,
        jit.cpu.xpsr & 0xF000_0000,
        "NZCV after {what}"
    );
}

#[test]
fn adc_sbc_rsbs_shift_mul32_match_interpreter() {
    let cases: &[(&str, Vec<u8>, fn(&mut Machine<CortexM>))] = &[
        (
            "adcs r0, r2",
            pad_alu_then(adc(0, 2), 14),
            (|m| {
                m.cpu.r1 = 0x7FFF_FFFF;
                m.cpu.r2 = 0;
                m.cpu.xpsr |= 1 << 29;
            }) as fn(&mut Machine<CortexM>),
        ),
        ("sbcs r0, r2", pad_alu_then(sbc(0, 2), 14), |m| {
            m.cpu.r1 = 0x8000_0000;
            m.cpu.r2 = 1;
            m.cpu.xpsr &= !(1 << 29);
        }),
        (
            "rsbs r0, r2",
            {
                let mut p = Vec::new();
                h(&mut p, mov_reg(2, 1));
                for _ in 0..14 {
                    h(&mut p, 0xBF00);
                }
                h(&mut p, rsbs(0, 2));
                let from = p.len() as i32;
                h(&mut p, b_to(from, 0));
                p
            },
            |m| {
                m.cpu.r1 = 1;
            },
        ),
        ("lsls r0, r2", pad_alu_then(lsl_reg(0, 2), 14), |m| {
            m.cpu.r1 = 0x8000_0001;
            m.cpu.r2 = 1;
        }),
        ("lsrs r0, r2", pad_alu_then(lsr_reg(0, 2), 14), |m| {
            m.cpu.r1 = 0x8000_0001;
            m.cpu.r2 = 1;
        }),
        ("asrs r0, r2", pad_alu_then(asr_reg(0, 2), 14), |m| {
            m.cpu.r1 = 0x8000_0000;
            m.cpu.r2 = 1;
        }),
        ("rors r0, r2", pad_alu_then(ror_reg(0, 2), 14), |m| {
            m.cpu.r1 = 0x0000_0001;
            m.cpu.r2 = 1;
        }),
    ];

    for (name, prog, seed) in cases {
        let probe = build_machine(prog);
        let mut engine_probe = CortexMJitEngine::new(4);
        engine_probe.try_compile_from_bus(0, &probe.bus);
        assert!(
            engine_probe.ready_instr_count(0).unwrap_or(0) >= 16,
            "{name} must compile a profitable block: {:?}",
            engine_probe.stats()
        );
        let (interp, jit, engine) = lockstep_until_compiled(prog, *seed);
        assert!(
            engine.stats().block_runs > 0,
            "{name} never ran compiled: {:?}",
            engine.stats()
        );
        assert_nzcv(&interp, &jit, name);
        assert_eq!(interp.cpu.r0, jit.cpu.r0, "r0 after {name}");
    }

    let mut prog = Vec::new();
    h(&mut prog, mov_reg(0, 1));
    for _ in 0..14 {
        h(&mut prog, 0xBF00);
    }
    let (h1, h2) = mul32(0, 0, 2);
    h(&mut prog, h1);
    h(&mut prog, h2);
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));
    let probe = build_machine(&prog);
    let mut engine_probe = CortexMJitEngine::new(4);
    engine_probe.try_compile_from_bus(0, &probe.bus);
    assert!(
        engine_probe.ready_instr_count(0).unwrap_or(0) >= 16,
        "MUL.W must compile: {:?}",
        engine_probe.stats()
    );
    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.r1 = 7;
        m.cpu.r2 = 9;
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.r0, jit.cpu.r0, "r0 after MUL.W");
}

fn ands_w(rd: u8, rn: u8, imm8: u8) -> (u16, u16) {
    // ANDS.W rd, rn, #imm8 (ThumbExpandImm of 00:000:imm8)
    (
        0xF010 | u16::from(rn & 0xF),
        ((rd as u16 & 0xF) << 8) | u16::from(imm8),
    )
}

#[test]
fn ands_w_imm_matches_interpreter() {
    let mut prog = Vec::new();
    for _ in 0..16 {
        h(&mut prog, 0xBF00);
    }
    let (h1, h2) = ands_w(0, 0, 0xFF);
    h(&mut prog, h1);
    h(&mut prog, h2);
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let probe = build_machine(&prog);
    let mut engine_probe = CortexMJitEngine::new(4);
    engine_probe.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine_probe.ready_instr_count(0),
        Some(18),
        "ANDS.W must be inside the compiled block: {:?}",
        engine_probe.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        m.cpu.r0 = 0xFFFF_00F0;
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.r0, jit.cpu.r0);
    assert_nzcv(&interp, &jit, "ands.w r0, r0, #0xff");
    assert_eq!(interp.cpu.r0, 0xF0);
}

fn ldr_w_post(rt: u8, rn: u8, imm8: u8) -> (u16, u16) {
    // LDR.W rt, [rn], #imm8  (P=0, U=1, W=1, bit11=1)
    (
        0xF850 | u16::from(rn & 0xF),
        ((rt as u16 & 0xF) << 12) | 0x0800 | 0x0200 | 0x0100 | u16::from(imm8),
    )
}

#[test]
fn ldr_w_postindex_sp_matches_interpreter() {
    let mut prog = Vec::new();
    for _ in 0..16 {
        h(&mut prog, 0xBF00);
    }
    let (h1, h2) = ldr_w_post(0, 13, 4);
    h(&mut prog, h1);
    h(&mut prog, h2);
    let from = prog.len() as i32;
    h(&mut prog, b_to(from, 0));

    let probe = build_machine(&prog);
    let mut engine_probe = CortexMJitEngine::new(4);
    engine_probe.try_compile_from_bus(0, &probe.bus);
    assert_eq!(
        engine_probe.ready_instr_count(0),
        Some(18),
        "LDR.W [sp], #4 must be inside the compiled block: {:?}",
        engine_probe.stats()
    );

    let (interp, jit, engine) = lockstep_until_compiled(&prog, |m| {
        let ram_base = m.bus.ram.base_addr as u32;
        m.cpu.sp = ram_base;
        for i in 0..64u32 {
            m.bus
                .write_u32(u64::from(ram_base + i * 4), 0xA000_0000 + i)
                .expect("seed RAM");
        }
    });
    assert!(engine.stats().block_runs > 0);
    assert_eq!(interp.cpu.r0, jit.cpu.r0);
    assert_eq!(interp.cpu.sp, jit.cpu.sp);
}

/// Negative control: prove the RAM diff actually bites. Registers/flags can
/// agree while a store still lands at the wrong address or with the wrong
/// value — that class of bug is invisible to `compare(..)` on the
/// [`StateVec`] alone, because RAM is not part of it. Corrupt one byte in
/// the JIT lane's RAM after a comparison point and confirm `compare_memory`
/// reports the exact address, even though CPU state is untouched.
#[test]
fn compare_memory_catches_a_corrupted_jit_ram_byte() {
    let prog = alu_loop_program();
    let mut interp = build_machine(&prog);
    let mut jit = build_machine(&prog);
    let mut engine = CortexMJitEngine::new(4);

    // Advance both lanes past one comparison point where they still agree.
    let n = engine.step_unit(&mut jit);
    assert!(n > 0, "jit machine halted unexpectedly");
    for _ in 0..n {
        interp.step().expect("interpreter must not fault");
    }
    assert!(
        compare_memory(1, interp.cpu.pc, &interp.bus, &jit.bus).is_none(),
        "lanes must agree before the injected corruption"
    );

    // A wrong store under the JIT: corrupt one byte deep in RAM. Registers,
    // flags and FPU state are all untouched, so `compare(..)` on the
    // StateVec alone would see nothing wrong.
    let ram_base = jit.bus.ram.base_addr;
    let corrupt_offset: u64 = 0x100;
    let corrupt_addr = ram_base + corrupt_offset;
    jit.bus.ram.data[corrupt_offset as usize] ^= 0xFF;

    assert_eq!(
        snapshot_state(&interp.cpu),
        snapshot_state(&jit.cpu),
        "corrupting RAM must not perturb CPU state; this proves the StateVec \
         compare alone cannot see the bug"
    );

    let d = compare_memory(2, interp.cpu.pc, &interp.bus, &jit.bus)
        .expect("compare_memory must catch the corrupted RAM byte");
    assert_eq!(
        d.address, corrupt_addr,
        "must report the exact corrupted address"
    );
    assert_ne!(d.interp, d.jit, "must report differing byte values");
}
