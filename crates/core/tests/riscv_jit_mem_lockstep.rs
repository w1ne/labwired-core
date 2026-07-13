// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential equivalence gate for the RV32IMC load/store JIT codegen
//! (framework chunk E), the sibling of `riscv_jit_alu_lockstep.rs`.
//!
//! It proves the compiled **memory** path is byte-identical to the
//! interpreter:
//!
//!   1. [`mem_hot_loop_is_byte_identical_and_compiles`] — a hot loop doing
//!      in-window loads/stores to RAM: run in lockstep, state compared word-
//!      for-word at every dispatch boundary AND the guest RAM compared
//!      byte-for-byte, asserted non-vacuous (real compiled-block executions,
//!      a retired-instruction floor).
//!   2. [`every_mem_op_matches_interpreter`] — a per-op differential: each
//!      base load/store width, aligned addresses, positive/negative offsets,
//!      sign vs zero extension, vs the interpreter.
//!   3. [`compressed_mem_ops_match_interpreter`] — the same for the
//!      RVC forms `C.LW / C.SW / C.LWSP / C.SWSP`.
//!   4. [`mem_fault_resume_is_byte_identical`] — a block whose access falls
//!      outside the RAM window (a fault) resumes on the interpreter and
//!      produces the identical architectural result as a pure-interpreter run.
//!
//! Gated behind `jit-framework` + `jit`.

#![cfg(all(feature = "jit-framework", feature = "jit"))]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::jit_framework::differential::{compare, DiffPolicy};
use labwired_core::cpu::jit_framework::riscv::{
    differential_cycle_ignore_indices, snapshot_state, RiscVFrontend, RiscvJitEngine, RiscvWasmJit,
};
use labwired_core::cpu::jit_framework::CodeView;
use labwired_core::cpu::RiscV;
use labwired_core::decoder::riscv::{decode_rv32, Instruction};
use labwired_core::Machine;

/// Guest-RAM window the JIT binds against. Base is the default SRAM base so
/// the interpreter's bus unambiguously routes it to `bus.ram`; length is kept
/// small so the per-block RAM sync in the runtime is cheap in the test.
const RAM_BASE: u32 = 0x2000_0000;
const RAM_LEN: usize = 0x1000;
const ECALL: u32 = 0x0000_0073;

// ── encoders ───────────────────────────────────────────────────────────────

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
fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
    (rs2 << 20) | (rs1 << 15) | (rd << 7) | 0x33
}
fn lb(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b000, imm, 0x03)
}
fn lh(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b001, imm, 0x03)
}
fn lw(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b010, imm, 0x03)
}
fn lbu(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b100, imm, 0x03)
}
fn lhu(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b101, imm, 0x03)
}
fn sb(rs1: u32, rs2: u32, imm: i32) -> u32 {
    enc_s(rs1, rs2, 0b000, imm, 0x23)
}
fn sh(rs1: u32, rs2: u32, imm: i32) -> u32 {
    enc_s(rs1, rs2, 0b001, imm, 0x23)
}
fn sw(rs1: u32, rs2: u32, imm: i32) -> u32 {
    enc_s(rs1, rs2, 0b010, imm, 0x23)
}
fn bne(rs1: u32, rs2: u32, imm: i32) -> u32 {
    let u = imm as u32;
    let imm12 = (u >> 12) & 1;
    let imm10_5 = (u >> 5) & 0x3F;
    let imm4_1 = (u >> 1) & 0xF;
    let imm11 = (u >> 11) & 1;
    (imm12 << 31)
        | (imm10_5 << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (0b001 << 12)
        | (imm4_1 << 8)
        | (imm11 << 7)
        | 0x63
}

// ── RVC encoders (inverses of the decoder; asserted on decode below) ───────

fn c_lw(rd: u32, rs1: u32, imm: u32) -> u16 {
    let imm5_3 = (imm >> 3) & 0x7;
    let imm2 = (imm >> 2) & 0x1;
    let imm6 = (imm >> 6) & 0x1;
    ((0b010 << 13)
        | (imm5_3 << 10)
        | ((rs1 - 8) << 7)
        | (imm2 << 6)
        | (imm6 << 5)
        | ((rd - 8) << 2)) as u16
}
fn c_sw(rs1: u32, rs2: u32, imm: u32) -> u16 {
    let imm5_3 = (imm >> 3) & 0x7;
    let imm2 = (imm >> 2) & 0x1;
    let imm6 = (imm >> 6) & 0x1;
    ((0b110 << 13)
        | (imm5_3 << 10)
        | ((rs1 - 8) << 7)
        | (imm2 << 6)
        | (imm6 << 5)
        | ((rs2 - 8) << 2)) as u16
}
fn c_lwsp(rd: u32, imm: u32) -> u16 {
    let imm5 = (imm >> 5) & 1;
    let imm4_2 = (imm >> 2) & 0x7;
    let imm7_6 = (imm >> 6) & 0x3;
    ((0b010 << 13) | (imm5 << 12) | (rd << 7) | (imm4_2 << 4) | (imm7_6 << 2) | 0b10) as u16
}
fn c_swsp(rs2: u32, imm: u32) -> u16 {
    let imm5_2 = (imm >> 2) & 0xF;
    let imm7_6 = (imm >> 6) & 0x3;
    ((0b110 << 13) | (imm5_2 << 9) | (imm7_6 << 7) | (rs2 << 2) | 0b10) as u16
}

// ── machine / flash helpers ────────────────────────────────────────────────

fn flash_of(prog: &[u32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(prog.len() * 4);
    for w in prog {
        b.extend_from_slice(&w.to_le_bytes());
    }
    b
}

/// A machine whose flash holds `prog` and whose `bus.ram` is a small window at
/// [`RAM_BASE`] seeded from `ram`.
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

/// A deterministic RAM pattern with high bytes set so sign/zero extension is
/// actually exercised.
fn ram_pattern() -> Vec<u8> {
    let mut r = vec![0u8; RAM_LEN];
    // A signed-negative byte, halfword, and word at aligned offsets.
    r[0x40] = 0x80; // Lb → 0xFFFFFF80 ; Lbu → 0x00000080
    r[0x44] = 0x00;
    r[0x45] = 0x80; // Lh @0x44 → 0xFFFF8000 ; Lhu → 0x00008000
    r[0x48..0x4C].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    r[0x4C..0x50].copy_from_slice(&0x0000_007Fu32.to_le_bytes());
    r
}

// ── (1) hot loop: in-window loads/stores ────────────────────────────────────

/// A hot loop that loads two RAM slots, mutates them, and stores them back.
/// The `bne` is chunk-D territory, so each iteration runs the compiled ALU +
/// load/store block and interprets the single branch.
fn mem_loop_program() -> Vec<u32> {
    vec![
        lui(1, 0x2_0000), // x1 = 0x2000_0000 (RAM base)   [0]
        addi(2, 0, 0),    // x2 = 0 (counter)              [1]
        addi(6, 0, 0),    // x6 = 0 (loop never terminates)[2]
        // loop head @ pc = 12 (word index 3):
        lw(3, 1, 0),         //  x3 = mem[base+0]             [3]
        addi(3, 3, 1),       //  x3++                          [4]
        sw(1, 3, 0),         //  mem[base+0] = x3              [5]
        lw(4, 1, 4),         //  x4 = mem[base+4]             [6]
        add(4, 4, 3),        //  x4 += x3                      [7]
        sw(1, 4, 4),         //  mem[base+4] = x4              [8]
        addi(2, 2, 1),       //  x2++                          [9]
        bne(2, 6, -(7 * 4)), // if x2 != x6 goto loop head  [10]
    ]
}

#[test]
fn mem_hot_loop_is_byte_identical_and_compiles() {
    const MAX_UNITS: u64 = 4_000;
    const INSTR_FLOOR: u64 = 3_000;

    let prog = mem_loop_program();
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
        // RAM is NOT part of the StateVec, so compare it explicitly.
        assert_eq!(
            jit.bus.ram.data, interp.bus.ram.data,
            "guest RAM diverged at unit {units}"
        );
    }

    let stats = engine.stats();
    assert!(stats.compiled > 0, "no memory block compiled");
    assert!(stats.block_runs > 0, "no compiled block executed");
    assert!(
        stats.block_instrs >= INSTR_FLOOR,
        "compiled blocks retired only {} instructions (floor {INSTR_FLOOR})",
        stats.block_instrs
    );
    // The loop actually mutated RAM (non-vacuous memory traffic).
    let slot0 = u32::from_le_bytes(interp.bus.ram.data[0..4].try_into().unwrap());
    assert!(slot0 >= 100, "RAM slot 0 = {slot0}, loop under-ran");
    println!(
        "mem_hot_loop: retired={retired} compiled={} block_runs={} block_instrs={} interpreted={} slot0={slot0}",
        stats.compiled, stats.block_runs, stats.block_instrs, stats.interpreted
    );
}

// ── (2) per-op differential (base loads/stores) ─────────────────────────────

/// Run a single memory op through both the interpreter and a compiled block
/// and assert the full register file + guest RAM match. `setup` seeds the
/// pre-state registers (base pointer / store value); the op is the only
/// instruction, followed by an `ecall` terminator.
fn assert_op_matches(op: u32, setup: &[(usize, u32)], ram: &[u8]) {
    // Interpreter reference.
    let prog = [op, ECALL];
    let mut im = build_machine(&prog, ram);
    for &(r, v) in setup {
        im.cpu.x[r] = v;
    }
    im.step().expect("interpreter op step");

    // Compiled block run directly against a register file + RAM buffer.
    let frontend = RiscVFrontend::with_ram_window(RAM_BASE, RAM_LEN as u32);
    let jit = RiscvWasmJit::new();
    let flash = flash_of(&prog);
    let (plan, binding) = frontend
        .translate_block_riscv(0, &CodeView::new(0, &flash))
        .expect("translate");
    assert!(!plan.is_stub(), "expected a compiled load/store block");
    assert_eq!(plan.instr_count, 1, "one-instruction block");
    let mut block = jit.compile(&plan, binding).expect("compile");

    let mut x = [0u32; 32];
    for &(r, v) in setup {
        x[r] = v;
    }
    let mut rambuf = vec![0u8; RAM_LEN];
    rambuf[..ram.len()].copy_from_slice(ram);
    let (_exit, n, _clear) = block.run(&mut x, &mut rambuf);
    assert_eq!(n, 1, "retires one instruction");

    assert_eq!(
        x, im.cpu.x,
        "register mismatch for op {op:#010x}: jit={x:?} interp={:?}",
        im.cpu.x
    );
    assert_eq!(rambuf, im.bus.ram.data, "RAM mismatch for op {op:#010x}");
}

#[test]
fn every_mem_op_matches_interpreter() {
    let ram = ram_pattern();
    let base = RAM_BASE + 0x40; // rs1 holds an in-window pointer
    let val = 0xCAFE_F00Du32; // store source
                              // rs1 = x10, rs2 = x11, rd = x12.

    // Loads: cover every width + sign/zero extension, +/- offsets.
    for &imm in &[0i32, 4, 8, -4, 12] {
        assert_op_matches(lb(12, 10, imm), &[(10, base)], &ram);
        assert_op_matches(lbu(12, 10, imm), &[(10, base)], &ram);
        assert_op_matches(lh(12, 10, imm), &[(10, base)], &ram);
        assert_op_matches(lhu(12, 10, imm), &[(10, base)], &ram);
        assert_op_matches(lw(12, 10, imm), &[(10, base)], &ram);
    }

    // Stores: each width, +/- offsets. Compare the whole RAM buffer.
    for &imm in &[0i32, 2, 8, -8, 16] {
        assert_op_matches(sb(10, 11, imm), &[(10, base), (11, val)], &ram);
        assert_op_matches(sh(10, 11, imm), &[(10, base), (11, val)], &ram);
        assert_op_matches(sw(10, 11, imm), &[(10, base), (11, val)], &ram);
    }
}

#[test]
fn compressed_mem_ops_match_interpreter() {
    let ram = ram_pattern();
    let base = RAM_BASE + 0x40;
    let sp = RAM_BASE + 0x80;
    let val = 0x1234_5678u32;

    // Decode each RVC word to confirm the hand encoding is what we intend.
    let clw = c_lw(8, 9, 8) as u32; // C.LW x8, 8(x9)
    assert!(matches!(
        decode_rv32(clw),
        Instruction::CLw {
            rd: 8,
            rs1: 9,
            imm: 8
        }
    ));
    let csw = c_sw(9, 8, 12) as u32; // C.SW x8, 12(x9)
    assert!(matches!(
        decode_rv32(csw),
        Instruction::CSw {
            rs2: 8,
            rs1: 9,
            imm: 12
        }
    ));
    let clwsp = c_lwsp(12, 8) as u32; // C.LWSP x12, 8(sp)
    assert!(matches!(
        decode_rv32(clwsp),
        Instruction::CLwsp { rd: 12, imm: 8 }
    ));
    let cswsp = c_swsp(11, 16) as u32; // C.SWSP x11, 16(sp)
    assert!(matches!(
        decode_rv32(cswsp),
        Instruction::CSwsp { rs2: 11, imm: 16 }
    ));

    // Each RVC op is a 2-byte instruction; pad the flash word's high half with
    // a compressed `nop` (c.addi x0,0 == 0x0001) so the block walk sees a full
    // second instruction, then an ecall terminator in the next word.
    let c_nop: u32 = 0x0001;
    assert_op_matches_words(&[clw | (c_nop << 16), ECALL], &[(9, base)], &ram);
    assert_op_matches_words(&[csw | (c_nop << 16), ECALL], &[(9, base), (8, val)], &ram);
    assert_op_matches_words(&[clwsp | (c_nop << 16), ECALL], &[(2, sp)], &ram);
    assert_op_matches_words(&[cswsp | (c_nop << 16), ECALL], &[(2, sp), (11, val)], &ram);
}

/// Like [`assert_op_matches`] but for a hand-assembled multi-word program
/// whose first instruction is the op under test (used for the 2-byte RVC
/// forms). Runs the whole compiled block to its terminator vs the interpreter.
fn assert_op_matches_words(prog: &[u32], setup: &[(usize, u32)], ram: &[u8]) {
    let mut im = build_machine(prog, ram);
    for &(r, v) in setup {
        im.cpu.x[r] = v;
    }
    // Step the interpreter across exactly the compiled block's instruction
    // count (all instructions up to, but not including, the ecall).
    let frontend = RiscVFrontend::with_ram_window(RAM_BASE, RAM_LEN as u32);
    let jit = RiscvWasmJit::new();
    let flash = flash_of(prog);
    let (plan, binding) = frontend
        .translate_block_riscv(0, &CodeView::new(0, &flash))
        .expect("translate");
    assert!(!plan.is_stub(), "expected a compiled block");
    let n = plan.instr_count;
    for _ in 0..n {
        im.step().expect("interpreter step");
    }

    let mut block = jit.compile(&plan, binding).expect("compile");
    let mut x = [0u32; 32];
    for &(r, v) in setup {
        x[r] = v;
    }
    let mut rambuf = vec![0u8; RAM_LEN];
    rambuf[..ram.len()].copy_from_slice(ram);
    let (_exit, got, _clear) = block.run(&mut x, &mut rambuf);
    assert_eq!(got, n, "retired count");
    assert_eq!(
        x, im.cpu.x,
        "register mismatch: jit={x:?} interp={:?}",
        im.cpu.x
    );
    assert_eq!(rambuf, im.bus.ram.data, "RAM mismatch");
}

// ── (4) fault path: out-of-window access resumes on the interpreter ─────────

/// A hot loop whose block does an in-window store and then an **out-of-window**
/// load (from flash address 0). The load faults mid-block: the block commits
/// the prior instructions, publishes the faulting PC + retired count, and the
/// engine resumes the interpreter to perform the real (flash) access — which
/// must produce the identical architectural result as a pure-interpreter run.
fn fault_loop_program() -> Vec<u32> {
    vec![
        lui(1, 0x2_0000), // x1 = RAM base                 [0]
        addi(2, 0, 0),    // x2 = counter                  [1]
        addi(6, 0, 0),    // x6 = 0                        [2]
        // loop head @ pc = 12 (index 3):
        lw(3, 1, 0),         //  x3 = mem[base]  (in window)  [3]
        addi(3, 3, 1),       //  x3++                          [4]
        sw(1, 3, 0),         //  mem[base] = x3  (in window)  [5]
        lw(4, 0, 0),         //  x4 = mem[0]  (FLASH → FAULT) [6]
        add(4, 4, 3),        //  x4 += x3                      [7]
        addi(2, 2, 1),       //  x2++                          [8]
        bne(2, 6, -(6 * 4)), //  loop                          [9]
    ]
}

#[test]
fn mem_fault_resume_is_byte_identical() {
    const MAX_UNITS: u64 = 3_000;
    const FLOOR: u64 = 1_500;

    let prog = fault_loop_program();
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
                "fault-path divergence at unit {units} (retired {retired}): {d:?}\n\
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
        "mem_fault: retired={retired} compiled={} block_runs={} block_instrs={} interpreted={}",
        stats.compiled, stats.block_runs, stats.block_instrs, stats.interpreted
    );
}
