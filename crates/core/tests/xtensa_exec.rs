// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! D1 + D2 execution tests:
//!
//! - D1: ALU reg-reg (ADD/SUB/AND/OR/XOR/NEG/ABS/ADDX*/SUBX*), MOVI, NOP/fence, BREAK.
//! - D2: Shift instructions (SLL/SRL/SRA/SRC/SLLI/SRLI/SRAI) and SAR-setup
//!   (SSL/SSR/SSAI/SSA8L/SSA8B).
//!
//! D1 encodings verified via xtensa-esp-elf-objdump; D2 encodings cross-referenced against
//! Xtensa LX ISA RM (assembler not available on this host — objdump verification TODO when
//! toolchain is accessible).
//! Bus: default SystemBus::new() provides RAM at 0x2000_0000..0x2010_0000.

use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::cpu::xtensa_sr::SAR as SAR_ID;
use labwired_core::{Bus, Cpu, SimulationError};

const TEST_PC: u32 = 0x2000_0000;

// ── D1 encoding helpers (HW-oracle verified via xtensa-esp-elf-objdump) ────

/// Pack a wide (3-byte) RRR instruction.
/// Layout: bits[3:0]=op0=0, bits[7:4]=t, bits[11:8]=s, bits[15:12]=r,
///         bits[19:16]=op1, bits[23:20]=op2.
fn rrr(op2: u32, op1: u32, r: u32, s: u32, t: u32) -> u32 {
    (op2 << 20) | (op1 << 16) | (r << 12) | (s << 8) | (t << 4)
}

/// Pack a wide ST0 instruction (op0=0, op1=0, op2=0).
fn st0(r: u32, s: u32, t: u32) -> u32 {
    (r << 12) | (s << 8) | (t << 4)
}

/// Pack a wide MOVI at, imm instruction (op0=0x2, r=0xA).
/// imm is sign-extended 12-bit; encoded as imm12 = {s[3:0], imm8[7:0]}.
fn movi(at: u32, imm: i32) -> u32 {
    let imm12 = (imm as u32) & 0xFFF;
    let s = (imm12 >> 8) & 0xF;
    let imm8 = imm12 & 0xFF;
    0x2 | (at << 4) | (s << 8) | (0xA << 12) | (imm8 << 16)
}

// ── D2 shift/SAR encoding helpers — ISA RM cross-referenced; objdump verification pending assembler availability ──

/// Encode SLL ar, as_  (op0=0, op1=0x1, op2=0xA, r=ar, s=as_, t=0).
fn enc_sll(ar: u32, as_: u32) -> u32 { rrr(0xA, 0x1, ar, as_, 0) }

/// Encode SRL ar, at  (op0=0, op1=0x1, op2=0x9, r=ar, s=0, t=at).
fn enc_srl(ar: u32, at: u32) -> u32 { rrr(0x9, 0x1, ar, 0, at) }

/// Encode SRA ar, at  (op0=0, op1=0x1, op2=0xB, r=ar, s=0, t=at).
fn enc_sra(ar: u32, at: u32) -> u32 { rrr(0xB, 0x1, ar, 0, at) }

/// Encode SRC ar, as_, at  (op0=0, op1=0x1, op2=0x8).
fn enc_src(ar: u32, as_: u32, at: u32) -> u32 { rrr(0x8, 0x1, ar, as_, at) }

/// Encode SLLI ar, as_, shamt (1..=31).
///
/// ISA encoding: raw = 32 - shamt; op2 = 0 | (raw >> 4), t = raw & 0xF.
fn enc_slli(ar: u32, as_: u32, shamt: u32) -> u32 {
    let raw = 32u32.wrapping_sub(shamt);
    let op2 = raw >> 4;       // 0 or 1
    let t   = raw & 0xF;
    rrr(op2, 0x1, ar, as_, t)
}

/// Encode SRLI ar, at, shamt (0..=15).
///
/// ISA encoding: op2=0x4, t=shamt.  The `t` field doubles as both the source
/// register number and the shift amount, so `at == shamt` is required.
/// Caller must place the value to shift in register `shamt`.
fn enc_srli(ar: u32, shamt: u32) -> u32 {
    // at == shamt: the source register IS the shift count (ISA encoding constraint)
    rrr(0x4, 0x1, ar, 0, shamt)
}

/// Encode SRAI ar, at, shamt (0..=31).
///
/// ISA encoding: shamt = (op2 & 1) << 4 | t; at = t (low nibble of shamt).
/// For shamt 0..=15: op2=0x2, t=shamt, at=shamt.
/// For shamt 16..=31: op2=0x3, t=shamt&0xF, at=shamt&0xF.
/// Caller must place the value to shift in register `shamt & 0xF`.
fn enc_srai(ar: u32, shamt: u32) -> u32 {
    let op2 = 0x2 | (shamt >> 4);  // 0x2 or 0x3
    let t   = shamt & 0xF;         // t == at (source register)
    rrr(op2, 0x1, ar, 0, t)
}

/// Encode SSL as_  (op0=0, op1=0, op2=0x4, r=0x1, s=as_, t=0).
fn enc_ssl(as_: u32) -> u32 { rrr(0x4, 0x0, 0x1, as_, 0) }

/// Encode SSR as_  (op0=0, op1=0, op2=0x4, r=0x0, s=as_, t=0).
fn enc_ssr(as_: u32) -> u32 { rrr(0x4, 0x0, 0x0, as_, 0) }

/// Encode SSA8L as_  (op0=0, op1=0, op2=0x4, r=0x2, s=as_, t=0).
fn enc_ssa8l(as_: u32) -> u32 { rrr(0x4, 0x0, 0x2, as_, 0) }

/// Encode SSA8B as_  (op0=0, op1=0, op2=0x4, r=0x3, s=as_, t=0).
fn enc_ssa8b(as_: u32) -> u32 { rrr(0x4, 0x0, 0x3, as_, 0) }

/// Encode SSAI shamt (0..=31).
///
/// ISA encoding: r=0x4, shamt = {t[0], s[3:0]}.
/// So s = shamt & 0xF, t = shamt >> 4 (0 or 1).
fn enc_ssai(shamt: u32) -> u32 { rrr(0x4, 0x0, 0x4, shamt & 0xF, shamt >> 4) }

// ── D3 LSAI encoding helpers (ADDI/ADDMI) ──

/// Encode ADDI at, as_, imm8 (op0=0x2, r=0xC).
/// LSAI format: (imm8<<16) | (r<<12) | (s<<8) | (t<<4) | op0.
fn enc_addi(at: u32, as_: u32, imm8: i32) -> u32 {
    let imm = (imm8 as u32) & 0xFF;
    0x2 | (at << 4) | (as_ << 8) | (0xC << 12) | (imm << 16)
}

/// Encode ADDMI at, as_, imm8 (op0=0x2, r=0xD).
/// LSAI format: (imm8<<16) | (r<<12) | (s<<8) | (t<<4) | op0.
fn enc_addmi(at: u32, as_: u32, imm8: i32) -> u32 {
    let imm = (imm8 as u32) & 0xFF;
    0x2 | (at << 4) | (as_ << 8) | (0xD << 12) | (imm << 16)
}

// ── Bus helpers ─────────────────────────────────────────────────────────────

/// Write a sequence of 3-byte wide instructions to the bus starting at `addr`.
fn write_insns(bus: &mut SystemBus, addr: u64, words: &[u32]) {
    for (i, &w) in words.iter().enumerate() {
        let off = (i as u64) * 3;
        bus.write_u8(addr + off,     (w & 0xFF) as u8).unwrap();
        bus.write_u8(addr + off + 1, ((w >> 8) & 0xFF) as u8).unwrap();
        bus.write_u8(addr + off + 2, ((w >> 16) & 0xFF) as u8).unwrap();
    }
}

/// Run the CPU until step() returns Err, then return that error.
/// Panics after 1000 steps to guard against infinite loops in tests.
fn run_until_error(cpu: &mut XtensaLx7, bus: &mut SystemBus) -> SimulationError {
    for _ in 0..1000 {
        match cpu.step(bus, &[]) {
            Ok(()) => {}
            Err(e) => return e,
        }
    }
    panic!("run_until_error: still running after 1000 steps — infinite loop or missing BREAK");
}

// ── D1 Tests ─────────────────────────────────────────────────────────────────

/// Main spec scenario: MOVI a2, 5; MOVI a3, 7; ADD a4, a2, a3; BREAK 1, 15 → a4 == 12.
#[test]
fn test_exec_add_movi_break_sequence() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 5),          // MOVI a2, 5
        movi(3, 7),          // MOVI a3, 7
        rrr(0x8, 0, 4, 2, 3), // ADD a4, a2, a3
        st0(4, 1, 0xF),      // BREAK 1, 15
    ]);

    let err = run_until_error(&mut cpu, &mut bus);
    assert!(
        matches!(err, SimulationError::BreakpointHit(_)),
        "expected BreakpointHit, got {:?}", err
    );
    assert_eq!(cpu.get_register(4), 12, "a4 should be 5+7=12");
    assert_eq!(cpu.get_register(2), 5,  "a2 unchanged");
    assert_eq!(cpu.get_register(3), 7,  "a3 unchanged");
}

#[test]
fn test_exec_sub() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // a2=20, a3=7 → SUB a4, a2, a3 → a4=13
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 20),
        movi(3, 7),
        rrr(0xC, 0, 4, 2, 3), // SUB a4, a2, a3
        st0(4, 0, 0),          // BREAK 0, 0
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 13);
}

/// Subtraction wrapping: 3 - 10 = 0xFFFFFFF9 (u32 wrapping).
#[test]
fn test_exec_sub_wrapping() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 3),
        movi(3, 10),
        rrr(0xC, 0, 4, 2, 3),
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 3u32.wrapping_sub(10));
}

#[test]
fn test_exec_and_or_xor() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // a2=0xFF, a3=0x0F
    // AND→0x0F, OR→0xFF, XOR→0xF0
    // We test each individually; use a2/a3 set once, then AND/OR/XOR into a4/a5/a6.
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 0xFF),
        movi(3, 0x0F),
        rrr(0x1, 0, 4, 2, 3),  // AND a4, a2, a3
        rrr(0x2, 0, 5, 2, 3),  // OR  a5, a2, a3
        rrr(0x3, 0, 6, 2, 3),  // XOR a6, a2, a3
        st0(4, 0, 0),           // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 0x0F, "AND");
    assert_eq!(cpu.get_register(5), 0xFF, "OR");
    assert_eq!(cpu.get_register(6), 0xF0, "XOR");
}

#[test]
fn test_exec_neg_abs() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // NEG a4, a2 with a2=5 → a4 = 0 - 5 = 0xFFFFFFFB
    // ABS a5, a3 with a3=-3 (0xFFFFFFFD) → a5 = 3
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 5),
        movi(3, -3),
        rrr(0x6, 0, 4, 0, 2),  // NEG a4, a2  (s=0 selects NEG)
        rrr(0x6, 0, 5, 1, 3),  // ABS a5, a3  (s=1 selects ABS)
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 0u32.wrapping_sub(5));
    assert_eq!(cpu.get_register(5), 3u32);
}

/// ABS(i32::MIN) edge case: Xtensa ISA RM specifies unsigned-abs result.
/// i32::MIN = 0x80000000; unsigned_abs = 0x80000000 (two's complement wraparound).
#[test]
fn test_exec_abs_i32_min_edge_case() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Load i32::MIN (0x80000000) into a2 via two MOVIs + ADD is complex.
    // Instead use set_register directly, then ABS a3, a2.
    cpu.set_register(2, 0x8000_0000u32);

    write_insns(&mut bus, TEST_PC as u64, &[
        rrr(0x6, 0, 3, 1, 2),  // ABS a3, a2  (s=1)
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    // ISA RM: ABS returns unsigned abs; for i32::MIN (0x80000000), result = 0x80000000.
    assert_eq!(cpu.get_register(3), 0x8000_0000u32);
}

#[test]
fn test_exec_addx_family() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // a2=3, a3=1
    // ADDX2 a4 = (3<<1)+1 = 7
    // ADDX4 a5 = (3<<2)+1 = 13
    // ADDX8 a6 = (3<<3)+1 = 25
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 3),
        movi(3, 1),
        rrr(0x9, 0, 4, 2, 3),  // ADDX2 a4, a2, a3
        rrr(0xA, 0, 5, 2, 3),  // ADDX4 a5, a2, a3
        rrr(0xB, 0, 6, 2, 3),  // ADDX8 a6, a2, a3
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 7,  "ADDX2: (3<<1)+1");
    assert_eq!(cpu.get_register(5), 13, "ADDX4: (3<<2)+1");
    assert_eq!(cpu.get_register(6), 25, "ADDX8: (3<<3)+1");
}

#[test]
fn test_exec_subx_family() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // a2=3, a3=1
    // SUBX2 a4 = (3<<1)-1 = 5
    // SUBX4 a5 = (3<<2)-1 = 11
    // SUBX8 a6 = (3<<3)-1 = 23
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 3),
        movi(3, 1),
        rrr(0xD, 0, 4, 2, 3),  // SUBX2 a4, a2, a3
        rrr(0xE, 0, 5, 2, 3),  // SUBX4 a5, a2, a3
        rrr(0xF, 0, 6, 2, 3),  // SUBX8 a6, a2, a3
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 5,  "SUBX2: (3<<1)-1");
    assert_eq!(cpu.get_register(5), 11, "SUBX4: (3<<2)-1");
    assert_eq!(cpu.get_register(6), 23, "SUBX8: (3<<3)-1");
}

/// NOP and MEMW must advance PC by 3 (wide instruction length) without error.
#[test]
fn test_exec_nop_memw_advance_pc() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // NOP (3 bytes) then MEMW (3 bytes) then BREAK → total advance = 6 before BREAK.
    write_insns(&mut bus, TEST_PC as u64, &[
        st0(2, 0, 0xF),  // NOP
        st0(2, 0, 0xC),  // MEMW
        st0(4, 0, 0),    // BREAK 0, 0
    ]);

    run_until_error(&mut cpu, &mut bus);
    // PC should be at the BREAK instruction (offset 6 from TEST_PC).
    assert_eq!(cpu.get_pc(), TEST_PC + 6, "PC after NOP+MEMW should be TEST_PC+6");
}

/// BREAK must return BreakpointHit with the PC value at the BREAK instruction (pre-advance).
#[test]
fn test_exec_break_returns_breakpoint_error() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Place BREAK at TEST_PC (offset 0).
    write_insns(&mut bus, TEST_PC as u64, &[
        st0(4, 2, 5),  // BREAK imm_s=2, imm_t=5
    ]);

    let err = cpu.step(&mut bus, &[]).unwrap_err();
    match err {
        SimulationError::BreakpointHit(pc) => {
            assert_eq!(pc, TEST_PC, "BreakpointHit must carry pre-advance PC");
        }
        other => panic!("expected BreakpointHit, got {:?}", other),
    }
    // PC must not have advanced.
    assert_eq!(cpu.get_pc(), TEST_PC, "PC must not advance on BREAK");
}

/// MOVI with negative immediate must sign-extend correctly.
#[test]
fn test_exec_movi_negative_immediate() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, -1),    // a2 = 0xFFFFFFFF
        movi(3, -100),  // a3 = 0xFFFFFF9C
        movi(4, -2048), // a4 = 0xFFFFF800 (min 12-bit signed)
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(2), 0xFFFF_FFFFu32, "MOVI -1");
    assert_eq!(cpu.get_register(3), (-100i32) as u32, "MOVI -100");
    assert_eq!(cpu.get_register(4), (-2048i32) as u32, "MOVI -2048");
}

// ── D2 Tests: Shift instructions ─────────────────────────────────────────────

/// SSL sets SAR = 32 - (as_ & 0x1F), then SLL a4, a2 → a4 = a2 << (32 - SAR).
/// With a2=1 and SSL a3 where a3=4: SAR = 32-4 = 28, SLL = 1 << 4 = 16.
#[test]
fn test_exec_sll() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // a2=1, a3=4. SSL a3 → SAR=28. SLL a4, a2 → a4 = a2 << (32-28) = 1 << 4 = 16.
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 1),
        movi(3, 4),
        enc_ssl(3),          // SSL a3 → SAR = 32 - 4 = 28
        enc_sll(4, 2),       // SLL a4, a2 → a4 = 1 << 4 = 16
        st0(4, 0, 0),        // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 16, "SLL: 1 << 4 = 16");
}

/// SSAI 0 → SAR=0 → SLL ar, as_ shifts by (32-0)=32 → result must be 0 (ISA RM §8).
/// Using u64 lift: (as_ as u64) << 32 as u32 = 0.
#[test]
fn test_exec_sll_with_sar_zero() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // a2 = 0xDEADBEEF. SSAI 0 → SAR=0. SLL a3, a2 → a3 = a2 << 32 = 0.
    // We also load a non-zero sentinel into a3 so that if the instructions
    // don't execute (e.g. fall through to NotImplemented early), the assert fails.
    cpu.set_register(2, 0xDEAD_BEEFu32);
    cpu.set_register(3, 0xCAFE_BABEu32); // sentinel: must be overwritten to 0
    write_insns(&mut bus, TEST_PC as u64, &[
        enc_ssai(0),         // SSAI 0 → SAR = 0
        enc_sll(3, 2),       // SLL a3, a2 → shift by 32 → 0 (must overwrite sentinel)
        st0(4, 0, 0),        // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0, "SLL with SAR=0 shifts by 32 → 0");
}

/// SRL ar, at: unsigned right shift. SSR a3 sets SAR, then SRL a4, a2.
/// a2=0x8000_0000, a3=4 → SSR a3 → SAR=4 → SRL a4,a2 → 0x0800_0000.
#[test]
fn test_exec_srl() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    cpu.set_register(2, 0x8000_0000u32);
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(3, 4),
        enc_ssr(3),          // SSR a3 → SAR = 4
        enc_srl(4, 2),       // SRL a4, a2 → 0x80000000 >> 4 = 0x08000000
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 0x0800_0000u32, "SRL: unsigned right shift");
}

/// SRA with positive value: no sign extension triggered.
/// a2=0x7FFF_FFFF, SAR=4 → SRA a3,a2 → 0x07FF_FFFF.
#[test]
fn test_exec_sra_positive() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    cpu.set_register(2, 0x7FFF_FFFFu32);
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(3, 4),
        enc_ssr(3),          // SSR a3 → SAR = 4
        enc_sra(4, 2),       // SRA a4, a2 (positive) → 0x07FFFFFF
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 0x07FF_FFFFu32, "SRA positive: no sign fill");
}

/// SRA with negative value: sign extension fills high bits with 1.
/// a2=0x8000_0000 (i32::MIN), SAR=4 → SRA → 0xF800_0000.
#[test]
fn test_exec_sra_negative() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    cpu.set_register(2, 0x8000_0000u32);
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(3, 4),
        enc_ssr(3),          // SSR a3 → SAR = 4
        enc_sra(4, 2),       // SRA a4, a2 (negative) → sign extends
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 0xF800_0000u32, "SRA negative: sign fill");
}

/// SRC ar, as_, at: concat (as_:at) as 64-bit, shift right by SAR, take low 32.
/// as_=0xABCD_EF01, at=0x2345_6789, SAR=8 →
/// concat = 0xABCDEF01_23456789, >> 8 = 0x00ABCDEF_01234567, low32 = 0x01234567.
#[test]
fn test_exec_src() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    cpu.set_register(2, 0xABCD_EF01u32); // as_
    cpu.set_register(3, 0x2345_6789u32); // at

    write_insns(&mut bus, TEST_PC as u64, &[
        movi(5, 8),
        enc_ssr(5),          // SSR a5 → SAR = 8
        enc_src(4, 2, 3),    // SRC a4, a2, a3
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    // Expected: (0xABCDEF01_23456789u64 >> 8) as u32 = 0x0123_4567
    assert_eq!(cpu.get_register(4), 0x0123_4567u32, "SRC bit-string extract");
}

/// SLLI a4, a2, 3: a2 << 3.  (Shift count is literal; no SAR involved.)
#[test]
fn test_exec_slli() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 7),
        enc_slli(4, 2, 3),   // SLLI a4, a2, 3 → 7 << 3 = 56
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 56, "SLLI 7 << 3 = 56");
}

/// SRLI a4, at, shamt: unsigned right shift by literal.
/// ISA encoding constraint: the source register number equals shamt (same t field).
/// We use shamt=4 → source register is a4. Load 0x80 into a4, SRLI a5, a4, 4 → 8.
/// But wait: enc_srli(ar=5, shamt=4) puts source as a4 (t=4).
#[test]
fn test_exec_srli() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // shamt=4 → source register = a4 (ISA constraint: t field = shamt = at).
    cpu.set_register(4, 0x80u32); // a4 = 0x80
    write_insns(&mut bus, TEST_PC as u64, &[
        enc_srli(5, 4),      // SRLI a5, a4, 4 → 0x80 >> 4 = 8
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(5), 8, "SRLI 0x80 >> 4 = 8");
}

/// SRAI positive: arithmetic right shift, no sign extension.
/// shamt=4 → source register a4. a4=0x7FFF_FFFF → 0x07FF_FFFF.
#[test]
fn test_exec_srai_positive() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // shamt=4 → at = a4 (ISA constraint).
    cpu.set_register(4, 0x7FFF_FFFFu32);
    write_insns(&mut bus, TEST_PC as u64, &[
        enc_srai(5, 4),      // SRAI a5, a4, 4 → 0x7FFFFFFF >> 4 = 0x07FFFFFF
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(5), 0x07FF_FFFFu32, "SRAI positive: no sign fill");
}

/// SRAI negative: arithmetic right shift sign-extends.
/// shamt=8 → source register a8. a8=0x8000_0000 → 0xFF80_0000.
#[test]
fn test_exec_srai_negative() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // shamt=8 → at = a8 (ISA constraint: t=8 & 0xF=8, op2=0x2).
    cpu.set_register(8, 0x8000_0000u32);
    write_insns(&mut bus, TEST_PC as u64, &[
        enc_srai(5, 8),      // SRAI a5, a8, 8 → sign-extended
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(5), 0xFF80_0000u32, "SRAI negative: sign fill");
}

/// SSL sets SAR = 32 - (as_ & 0x1F). Read it back via sr.read(SAR_ID).
#[test]
fn test_exec_ssl_sets_sar() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // a2=7 → SSL a2 → SAR = 32 - 7 = 25.
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 7),
        enc_ssl(2),          // SSL a2 → SAR = 32 - 7 = 25
        st0(4, 0, 0),        // BREAK so we can check SAR
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.sr.read(SAR_ID), 25, "SSL: SAR = 32 - 7 = 25");
}

/// SSR sets SAR = as_ & 0x1F.
#[test]
fn test_exec_ssr_sets_sar() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // a2=11 → SSR a2 → SAR = 11.
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 11),
        enc_ssr(2),          // SSR a2 → SAR = 11
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.sr.read(SAR_ID), 11, "SSR: SAR = 11");
}

/// SSAI sets SAR = shamt & 0x1F.
#[test]
fn test_exec_ssai_sets_sar() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_ssai(19),        // SSAI 19 → SAR = 19
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.sr.read(SAR_ID), 19, "SSAI: SAR = 19");
}

/// SSA8L: SAR = (as_ & 3) * 8. Little-endian byte selection.
/// as_=2 → SAR = 2*8 = 16.
#[test]
fn test_exec_ssa8l() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 2),
        enc_ssa8l(2),        // SSA8L a2 → SAR = (2 & 3) * 8 = 16
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.sr.read(SAR_ID), 16, "SSA8L: SAR = (2 & 3) * 8 = 16");
}

/// SSA8B: SAR = 32 - (as_ & 3) * 8. Big-endian byte selection.
/// as_=1 → SAR = 32 - 1*8 = 24.
/// as_=0 → SAR = 32 - 0 = 32 (valid 6-bit value).
#[test]
fn test_exec_ssa8b() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Test with as_=1 → SAR=24.
    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 1),
        enc_ssa8b(2),        // SSA8B a2 → SAR = 32 - (1 & 3)*8 = 24
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.sr.read(SAR_ID), 24, "SSA8B: SAR = 32 - 8 = 24");

    // Also test as_=0 → SAR=32 (valid 6-bit, masks to 32).
    let mut cpu2 = XtensaLx7::new();
    let mut bus2 = SystemBus::new();
    cpu2.reset(&mut bus2).unwrap();
    cpu2.set_pc(TEST_PC);

    write_insns(&mut bus2, TEST_PC as u64, &[
        movi(2, 0),
        enc_ssa8b(2),        // SSA8B a2 → SAR = 32 - 0 = 32
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu2, &mut bus2);
    // SAR is 6-bit (0..63), so 32 is valid.
    assert_eq!(cpu2.sr.read(SAR_ID), 32, "SSA8B: SAR = 32 when as_=0");
}

// ── D3 Tests: ADDI, ADDMI with sign-extension ───────────────────────────────

/// ADDI at, as_, imm8: at = as_ + sign_extend(imm8).
/// Positive immediate: a2=100, ADDI a3, a2, 50 → a3 = 150.
#[test]
fn test_exec_addi_positive() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 100),
        enc_addi(3, 2, 50),  // ADDI a3, a2, 50 → 100 + 50 = 150
        st0(4, 0, 0),        // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 150, "ADDI: 100 + 50 = 150");
}

/// ADDI with negative immediate: sign-extension.
/// a2=100, ADDI a3, a2, -50 → a3 = 50 (sign-extended -50 is 0xFFFFFFCE).
#[test]
fn test_exec_addi_negative() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 100),
        enc_addi(3, 2, -50),  // ADDI a3, a2, -50 → 100 + (-50) = 50
        st0(4, 0, 0),         // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 50, "ADDI: 100 + (-50) = 50");
}

/// ADDMI at, as_, imm8: at = as_ + (sign_extend(imm8) << 8).
/// Effective add is imm8 * 256. With imm8=5: adds 1280.
/// a2=1000, ADDMI a3, a2, 5 → a3 = 1000 + 1280 = 2280.
#[test]
fn test_exec_addmi_positive() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 1000),
        enc_addmi(3, 2, 5),  // ADDMI a3, a2, 5 → 1000 + (5 * 256) = 2280
        st0(4, 0, 0),        // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 2280, "ADDMI: 1000 + 1280 = 2280");
}

/// ADDMI with negative immediate: sign-extended then shifted.
/// imm8=-1 → sign-extended to 0xFFFFFFFF, << 8 → 0xFFFFFF00, add to a2.
/// a2=512, ADDMI a3, a2, -1 → a3 = 512 + ((-1) << 8) = 512 - 256 = 256.
#[test]
fn test_exec_addmi_negative() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        movi(2, 512),
        enc_addmi(3, 2, -1),  // ADDMI a3, a2, -1 → 512 + ((-1) << 8) = 512 - 256 = 256
        st0(4, 0, 0),         // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 256, "ADDMI: 512 + (-256) = 256");
}
