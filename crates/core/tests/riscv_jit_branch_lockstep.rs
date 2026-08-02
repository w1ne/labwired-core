// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential equivalence gate for the RV32IMC branch/jump JIT codegen
//! (framework chunk D).
//!
//! Chunk D turns the block terminator — conditional branches, `JAL`/`JALR`,
//! and the compressed `C.J`/`C.JR`/`C.JALR`/`C.BEQZ`/`C.BNEZ` — into real
//! wasm. Each terminator resolves the block's next PC *in wasm*, writes it to
//! the register memory's dynamic next-PC slot, and returns the single
//! [`WIRE_CHAIN_DYNAMIC`](../src/cpu/jit_framework/riscv/emit.rs) wire code;
//! the runtime chains to it. This gate proves that path is byte-identical to
//! [`RiscV::step`]:
//!
//!   1. [`branch_hot_loop_is_byte_identical_and_chains`] — a real backward
//!      branch drives a hot loop whose body block chains back to itself,
//!      re-executing the *same* compiled block; state is compared byte-for-
//!      byte at every aligned boundary and the run is asserted non-vacuous
//!      (compiled blocks, block runs, a retired-instruction floor).
//!   2. [`every_terminator_matches_interpreter`] — a per-terminator
//!      differential: every branch/jump kind, taken AND not-taken, over
//!      varied operands, covering `JALR`/`C.JR` low-bit masking, `rd == x0`
//!      (no link), `rd == rs1` `JALR`, backward vs forward targets, and
//!      signed vs unsigned compares.
//!
//! Gated behind `jit-framework` (the module) and `jit` (the native wasmtime
//! executor), mirroring the chunk-C ALU gate.

#![cfg(all(feature = "jit-framework", feature = "jit"))]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::jit_framework::differential::{compare, DiffPolicy};
use labwired_core::cpu::jit_framework::frontend::IsaFrontend;
use labwired_core::cpu::jit_framework::riscv::{
    differential_cycle_ignore_indices, snapshot_state, RiscVFrontend, RiscvJitEngine, RiscvWasmJit,
};
use labwired_core::cpu::jit_framework::side_exit::SideExit;
use labwired_core::cpu::jit_framework::CodeView;
use labwired_core::cpu::RiscV;
use labwired_core::decoder::riscv::{decode_rv32, Instruction};
use labwired_core::Machine;

// ── Encoders (standard RISC-V field layouts) ──────────────────────────────

fn enc_i(rd: u32, rs1: u32, funct3: u32, imm: i32, opcode: u32) -> u32 {
    ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b000, imm, 0x13)
}
fn slli(rd: u32, rs1: u32, sh: u32) -> u32 {
    enc_i(rd, rs1, 0b001, sh as i32, 0x13)
}
fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
    (rs2 << 20) | (rs1 << 15) | (0b100 << 12) | (rd << 7) | 0x33
}
fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
    // funct3 = 0b000 for ADD (omitted — it contributes no bits).
    (rs2 << 20) | (rs1 << 15) | (rd << 7) | 0x33
}
/// B-type conditional branch.
fn enc_b(rs1: u32, rs2: u32, funct3: u32, imm: i32) -> u32 {
    let u = imm as u32;
    ((u >> 12) & 1) << 31
        | ((u >> 5) & 0x3F) << 25
        | (rs2 << 20)
        | (rs1 << 15)
        | (funct3 << 12)
        | ((u >> 1) & 0xF) << 8
        | ((u >> 11) & 1) << 7
        | 0x63
}
fn beq(rs1: u32, rs2: u32, imm: i32) -> u32 {
    enc_b(rs1, rs2, 0b000, imm)
}
fn jal(rd: u32, imm: i32) -> u32 {
    let u = imm as u32;
    let imm20 = (u >> 20) & 1;
    let imm10_1 = (u >> 1) & 0x3FF;
    let imm11 = (u >> 11) & 1;
    let imm19_12 = (u >> 12) & 0xFF;
    (imm20 << 31) | (imm10_1 << 21) | (imm11 << 20) | (imm19_12 << 12) | (rd << 7) | 0x6F
}
fn jalr(rd: u32, rs1: u32, imm: i32) -> u32 {
    enc_i(rd, rs1, 0b000, imm, 0x67)
}

// Compressed (16-bit) terminators. Verified against `decode_rv32` in the test.
fn c_jr(rs1: u32) -> u16 {
    0x8002 | ((rs1 as u16) << 7)
}
fn c_jalr(rs1: u32) -> u16 {
    0x9002 | ((rs1 as u16) << 7)
}
/// C.J with a (small) CJ-format offset.
fn c_j(off: i32) -> u16 {
    let u = off as u32;
    let bit = |o: u32| ((u >> o) & 1) as u16;
    0xA001
        | (bit(11) << 12)
        | (bit(4) << 11)
        | ((((u >> 8) & 0x3) as u16) << 9)
        | (bit(10) << 8)
        | (bit(6) << 7)
        | (bit(7) << 6)
        | ((((u >> 1) & 0x7) as u16) << 3)
        | (bit(5) << 2)
}
/// CB-format compare-with-zero branch. `rs1p` is the actual register 8..=15.
fn c_cb(funct3: u16, rs1p: u32, off: i32) -> u16 {
    let u = off as u32;
    let bit = |o: u32| ((u >> o) & 1) as u16;
    let rp = ((rs1p - 8) & 0x7) as u16;
    0x0001
        | (funct3 << 13)
        | (bit(8) << 12)
        | ((((u >> 3) & 0x3) as u16) << 10)
        | (rp << 7)
        | ((((u >> 6) & 0x3) as u16) << 5)
        | ((((u >> 1) & 0x3) as u16) << 3)
        | (bit(5) << 2)
}
fn c_beqz(rs1p: u32, off: i32) -> u16 {
    c_cb(0b110, rs1p, off)
}
fn c_bnez(rs1p: u32, off: i32) -> u16 {
    c_cb(0b111, rs1p, off)
}

// ── Machine builders ──────────────────────────────────────────────────────

fn machine_from_bytes(bytes: &[u8]) -> Machine<RiscV> {
    let mut bus = SystemBus::new();
    bus.flash.data = bytes.to_vec();
    bus.flash.base_addr = 0;
    let mut cpu = RiscV::new();
    cpu.pc = 0;
    cpu.mtimecmp = u64::MAX; // keep the CLINT timer from ever firing
    Machine::new(cpu, bus)
}

fn words_to_bytes(words: &[u32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(words.len() * 4);
    for w in words {
        b.extend_from_slice(&w.to_le_bytes());
    }
    b
}

// ══════════════════════════════════════════════════════════════════════════
// (1) Backward-branch hot loop: chains compiled blocks, byte-identical.
// ══════════════════════════════════════════════════════════════════════════

/// A hot loop whose body block ends in a backward conditional branch, laid at
/// flash 0:
///
/// ```text
/// 0x00:  addi x2, x0, 500    ; loop limit (runs once, in the entry block)
/// 0x04:  addi x1, x1, 1      ; <- loop top: counter++
/// 0x08:  slli x3, x1, 2
/// 0x0c:  add  x4, x3, x1
/// 0x10:  xor  x5, x4, x2
/// 0x14:  blt  x1, x2, -16    ; if x1 < x2 goto 0x04 (backward, taken 500x)
/// 0x18:  jal  x0, 0          ; self-loop once the branch falls through
/// ```
///
/// After warmup, the block at `0x04` compiles and its taken branch chains
/// straight back to `0x04` — re-executing the *same* compiled block every
/// iteration — which is the property the dynamic-chain wire must get right.
fn branch_loop_program() -> Vec<u32> {
    vec![
        addi(2, 0, 500), // x2 = limit
        addi(1, 1, 1),   // 0x04 loop top: x1++
        slli(3, 1, 2),   // x3 = x1 << 2
        add(4, 3, 1),    // x4 = x3 + x1
        xor(5, 4, 2),    // x5 = x4 ^ x2
        beq(0, 0, 0),    // placeholder, replaced below with blt
        jal(0, 0),       // 0x18 self-loop terminator
    ]
}

#[test]
fn branch_hot_loop_is_byte_identical_and_chains() {
    const MAX_UNITS: u64 = 20_000;
    const INSTR_FLOOR: u64 = 1_500;

    let mut prog = branch_loop_program();
    // blt x1, x2, -16  (backward to 0x04); funct3=4 (BLT).
    prog[5] = enc_b(1, 2, 0b100, -16);

    let bytes = words_to_bytes(&prog);
    let mut interp = machine_from_bytes(&bytes);
    let mut jit = machine_from_bytes(&bytes);

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
            interp
                .step()
                .expect("interpreter must not fault on this loop");
        }
        retired += n as u64;

        let si = snapshot_state(&interp.cpu);
        let sj = snapshot_state(&jit.cpu);
        if let Some(d) = compare(units, &si, &sj, &policy) {
            panic!(
                "JIT diverged from interpreter at unit {units} (retired {retired}): {d:?}\n\
                 interp pc={:#x} x={:?}\n jit    pc={:#x} x={:?}",
                interp.cpu.pc, &interp.cpu.x, jit.cpu.pc, &jit.cpu.x
            );
        }
    }

    let stats = engine.stats();
    assert!(
        stats.compiled > 0,
        "no block ever crossed the hot threshold / compiled"
    );
    assert!(
        stats.block_runs > 0,
        "no compiled block was ever executed (JIT path not engaged)"
    );
    assert!(
        stats.block_instrs >= INSTR_FLOOR,
        "compiled blocks retired only {} instructions (floor {INSTR_FLOOR})",
        stats.block_instrs
    );
    // The loop counter reached the limit — the backward branch really cycled.
    assert_eq!(
        interp.cpu.x[1], 500,
        "loop counter x1={} did not reach the limit",
        interp.cpu.x[1]
    );
    println!(
        "branch_hot_loop: retired={retired} compiled_blocks={} block_runs={} block_instrs={} interpreted={}",
        stats.compiled, stats.block_runs, stats.block_instrs, stats.interpreted
    );
}

// ══════════════════════════════════════════════════════════════════════════
// (2) Per-terminator differential (taken + not-taken, edge cases).
// ══════════════════════════════════════════════════════════════════════════

/// Run one terminator block (`flash`) against both engines with the given
/// register preset and assert the resolved next PC and full register file
/// match the interpreter byte-for-byte.
fn assert_terminator_matches(
    label: &str,
    flash: &[u8],
    expect_kind: impl Fn(&Instruction),
    setup: &[(usize, u32)],
) {
    // The bytes must decode to the terminator kind we intend to exercise.
    let inst = if (flash[0] & 0x3) == 0x3 {
        decode_rv32(u32::from_le_bytes([flash[0], flash[1], flash[2], flash[3]]))
    } else {
        decode_rv32(u16::from_le_bytes([flash[0], flash[1]]) as u32)
    };
    expect_kind(&inst);

    // Interpreter reference.
    let mut im = machine_from_bytes(flash);
    for &(r, v) in setup {
        im.cpu.x[r] = v;
    }
    im.step().expect("interpreter terminator step");
    let want_pc = im.cpu.pc;
    let want_x = im.cpu.x;

    // Compiled block.
    let frontend = RiscVFrontend::new();
    let jit = RiscvWasmJit::new();
    let plan = {
        let view = CodeView::new(0, flash);
        frontend.translate_block(0, &view).expect("translate")
    };
    assert!(
        !plan.is_stub(),
        "{label}: expected a compiled terminator block"
    );
    assert_eq!(plan.instr_count, 1, "{label}: one-instruction block");
    // A branch/jump block touches no RAM, so it has no memory binding and syncs
    // no guest-RAM window (chunk-E `run` signature; `ram` is unused here).
    let mut block = jit.compile(&plan, None).expect("compile");

    let mut x = [0u32; 32];
    for &(r, v) in setup {
        x[r] = v;
    }
    let (exit, n, _clear) = block.run(&mut x, &mut []);
    assert_eq!(n, 1, "{label}: retires one instruction");
    let got_pc = match exit {
        SideExit::Chain { next_pc } => next_pc as u32,
        other => panic!("{label}: expected Chain, got {other:?}"),
    };
    assert_eq!(
        got_pc, want_pc,
        "{label}: next-PC mismatch jit={got_pc:#x} interp={want_pc:#x} (setup {setup:?})"
    );
    assert_eq!(
        x, want_x,
        "{label}: register mismatch (setup {setup:?})\n jit={x:?}\n int={want_x:?}"
    );
}

/// 32-bit terminator block = the instruction word followed by a padding word
/// (so the view always has room to decode, and the not-taken fall-through
/// lands on a valid slot).
fn flash32(word: u32) -> Vec<u8> {
    words_to_bytes(&[word, 0x0000_0013 /* nop */])
}
/// Compressed terminator block = the 2-byte instruction plus padding.
fn flash16(half: u16) -> Vec<u8> {
    let mut b = half.to_le_bytes().to_vec();
    b.extend_from_slice(&[0x01, 0x00, 0x01, 0x00]); // c.nop padding
    b
}

#[test]
fn every_terminator_matches_interpreter() {
    // ── conditional branches (B-type): taken + not-taken, both target dirs,
    //    signed vs unsigned operands ──────────────────────────────────────
    // (funct3, name, matches).
    let branches: &[(u32, &str)] = &[
        (0b000, "beq"),
        (0b001, "bne"),
        (0b100, "blt"),
        (0b101, "bge"),
        (0b110, "bltu"),
        (0b111, "bgeu"),
    ];
    // Operand pairs chosen so signed and unsigned orderings disagree, plus
    // equal / ordered cases — every branch sees taken and not-taken.
    let pairs: &[(u32, u32)] = &[
        (5, 5),
        (5, 7),
        (7, 5),
        (0xFFFF_FFFF, 1), // -1 vs 1: signed lt, unsigned gt
        (1, 0xFFFF_FFFF),
        (0x8000_0000, 0x7FFF_FFFF), // INT_MIN vs INT_MAX
    ];
    let offsets: &[i32] = &[16, -16, 0x40];
    for &(f3, name) in branches {
        for &off in offsets {
            for &(a, b) in pairs {
                let flash = flash32(enc_b(1, 2, f3, off));
                assert_terminator_matches(
                    name,
                    &flash,
                    |i| {
                        assert!(
                            matches!(
                                i,
                                Instruction::Beq { .. }
                                    | Instruction::Bne { .. }
                                    | Instruction::Blt { .. }
                                    | Instruction::Bge { .. }
                                    | Instruction::Bltu { .. }
                                    | Instruction::Bgeu { .. }
                            ),
                            "expected a branch, got {i:?}"
                        )
                    },
                    &[(1, a), (2, b)],
                );
            }
        }
    }

    // ── JAL: link (rd=1) and no-link (rd=0), forward + backward ───────────
    for &rd in &[0u32, 1, 5] {
        for &off in &[8i32, -8, 0x400] {
            let flash = flash32(jal(rd, off));
            assert_terminator_matches(
                "jal",
                &flash,
                |i| assert!(matches!(i, Instruction::Jal { .. }), "got {i:?}"),
                &[],
            );
        }
    }

    // ── JALR: low-bit masking, rd=x0, and the rd==rs1 aliasing case ───────
    // (rd, rs1, imm, base_value)
    let jalr_cases: &[(u32, u32, i32, u32)] = &[
        (1, 2, 0, 0x1000),
        (1, 2, 0, 0x1001),  // base low bit set -> masked off
        (1, 2, 3, 0x1000),  // imm low bit set -> masked off after add
        (1, 2, -4, 0x2000), // negative offset
        (0, 2, 0, 0x1000),  // rd = x0: no link
        (2, 2, 8, 0x1000),  // rd == rs1: reads pre-write rs1
        (2, 2, 8, 0x1001),  // rd == rs1 with masking
    ];
    for &(rd, rs1, imm, base) in jalr_cases {
        let flash = flash32(jalr(rd, rs1, imm));
        assert_terminator_matches(
            "jalr",
            &flash,
            |i| assert!(matches!(i, Instruction::Jalr { .. }), "got {i:?}"),
            &[(rs1 as usize, base)],
        );
    }

    // ── C.J ───────────────────────────────────────────────────────────────
    for &off in &[8i32, -8, 20] {
        let flash = flash16(c_j(off));
        assert_terminator_matches(
            "c.j",
            &flash,
            |i| assert!(matches!(i, Instruction::CJ { .. }), "got {i:?}"),
            &[],
        );
    }

    // ── C.JR / C.JALR: low-bit masking; C.JALR links x1 = pc+2 ────────────
    for &base in &[0x1000u32, 0x1001, 0x2003] {
        let flash = flash16(c_jr(5));
        assert_terminator_matches(
            "c.jr",
            &flash,
            |i| assert!(matches!(i, Instruction::CJr { .. }), "got {i:?}"),
            &[(5, base)],
        );
        let flash = flash16(c_jalr(6));
        assert_terminator_matches(
            "c.jalr",
            &flash,
            |i| assert!(matches!(i, Instruction::CJalr { .. }), "got {i:?}"),
            &[(6, base)],
        );
    }

    // ── C.BEQZ / C.BNEZ: taken (rs1'==0 / !=0) and not-taken ──────────────
    for &val in &[0u32, 1, 0xFFFF_FFFF] {
        let flash = flash16(c_beqz(8, 12));
        assert_terminator_matches(
            "c.beqz",
            &flash,
            |i| assert!(matches!(i, Instruction::CBeqz { .. }), "got {i:?}"),
            &[(8, val)],
        );
        let flash = flash16(c_bnez(9, 12));
        assert_terminator_matches(
            "c.bnez",
            &flash,
            |i| assert!(matches!(i, Instruction::CBnez { .. }), "got {i:?}"),
            &[(9, val)],
        );
    }
}
