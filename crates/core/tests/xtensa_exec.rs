// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! D1 + D2 + E4 execution tests:
//!
//! - D1: ALU reg-reg (ADD/SUB/AND/OR/XOR/NEG/ABS/ADDX*/SUBX*), MOVI, NOP/fence, BREAK.
//! - D2: Shift instructions (SLL/SRL/SRA/SRC/SLLI/SRLI/SRAI) and SAR-setup
//!   (SSL/SSR/SSAI/SSA8L/SSA8B).
//! - E4: Atomic exec (S32C1I/L32AI/S32RI) with SCOMPARE1 (SR ID 12).
//!
//! D1 and E4 encodings verified via xtensa-esp-elf-objdump (esp-15.2.0_20250920).
//! D2 encodings cross-referenced against Xtensa LX ISA RM.
//! Bus: default SystemBus::new() provides RAM at 0x2000_0000..0x2010_0000.

use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::cpu::xtensa_sr::{
    EPC1 as EPC1_ID, EPC2 as EPC2_ID, EPC3 as EPC3_ID, EXCCAUSE as EXCCAUSE_ID,
    EPS2 as EPS2_ID, EPS3 as EPS3_ID, SAR as SAR_ID, SCOMPARE1 as SCOMPARE1_ID,
    VECBASE as VECBASE_ID,
};
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

// ── D4 load encoding helpers ──

/// Encode L8UI at, as_, imm8 (op0=0x2, r=0x0).
/// LSAI format: (imm8<<16) | (r<<12) | (s<<8) | (t<<4) | op0.
/// imm is the raw byte offset (0..=255); no pre-shift in encoding.
fn enc_l8ui(at: u32, as_: u32, imm8: u32) -> u32 {
    0x2 | (at << 4) | (as_ << 8) | ((imm8 & 0xFF) << 16)
}

/// Encode L16UI at, as_, imm (op0=0x2, r=0x1).
/// The hardware imm field is the byte offset >> 1 (i.e. the word offset).
/// Pass the final byte offset here; this fn will right-shift by 1 for encoding.
fn enc_l16ui(at: u32, as_: u32, byte_off: u32) -> u32 {
    let imm8 = (byte_off >> 1) & 0xFF;
    0x2 | (at << 4) | (as_ << 8) | (0x1 << 12) | (imm8 << 16)
}

/// Encode L16SI at, as_, imm (op0=0x2, r=0x9). Same layout as L16UI.
fn enc_l16si(at: u32, as_: u32, byte_off: u32) -> u32 {
    let imm8 = (byte_off >> 1) & 0xFF;
    0x2 | (at << 4) | (as_ << 8) | (0x9 << 12) | (imm8 << 16)
}

/// Encode L32I at, as_, imm (op0=0x2, r=0x2).
/// The hardware imm field is the byte offset >> 2 (i.e. the word offset).
/// Pass the final byte offset here; this fn will right-shift by 2 for encoding.
fn enc_l32i(at: u32, as_: u32, byte_off: u32) -> u32 {
    let imm8 = (byte_off >> 2) & 0xFF;
    0x2 | (at << 4) | (as_ << 8) | (0x2 << 12) | (imm8 << 16)
}

/// Encode L32R at, imm16 (op0=0x1).
/// `imm16` is the raw 16-bit field (already the encoded word-count offset as
/// an unsigned 16-bit value). The decoder sign-extends it and multiplies by 4.
/// Layout: op0=0x1 in bits[3:0], at in bits[7:4], imm16 in bits[23:8].
fn enc_l32r(at: u32, imm16: u32) -> u32 {
    0x1 | (at << 4) | ((imm16 & 0xFFFF) << 8)
}

// ── D5 store encoding helpers ──

/// Encode S8I at, as_, imm8 (op0=0x2, r=0x4).
/// LSAI format: (imm8<<16) | (r<<12) | (s<<8) | (t<<4) | op0.
/// imm is the raw byte offset (0..=255); no pre-shift in encoding.
fn enc_s8i(at: u32, as_: u32, imm8: u32) -> u32 {
    0x2 | (at << 4) | (as_ << 8) | (0x4 << 12) | ((imm8 & 0xFF) << 16)
}

/// Encode S16I at, as_, imm (op0=0x2, r=0x5).
/// The hardware imm field is the byte offset >> 1 (i.e. the word offset).
/// Pass the final byte offset here; this fn will right-shift by 1 for encoding.
fn enc_s16i(at: u32, as_: u32, byte_off: u32) -> u32 {
    let imm8 = (byte_off >> 1) & 0xFF;
    0x2 | (at << 4) | (as_ << 8) | (0x5 << 12) | (imm8 << 16)
}

/// Encode S32I at, as_, imm (op0=0x2, r=0x6).
/// The hardware imm field is the byte offset >> 2 (i.e. the word offset).
/// Pass the final byte offset here; this fn will right-shift by 2 for encoding.
fn enc_s32i(at: u32, as_: u32, byte_off: u32) -> u32 {
    let imm8 = (byte_off >> 2) & 0xFF;
    0x2 | (at << 4) | (as_ << 8) | (0x6 << 12) | (imm8 << 16)
}

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

// ── D7 jump/call/ret encoding helpers ───────────────────────────────────────

/// Encode J offset (op0=0x6, n=0).
/// `offset_with_bias` is the signed byte displacement **including** the +4 pre-baked
/// by the decoder, i.e. `offset_with_bias = sign_extend18(imm18) + 4`.
/// Execute: `pc = pc.wrapping_add(offset_with_bias as u32)`.
fn enc_j(offset_with_bias: i32) -> u32 {
    // Remove the +4 bias to recover raw imm18.
    let imm18 = ((offset_with_bias - 4) as u32) & 0x3_FFFF;
    0x6 | (imm18 << 6)
}

/// Encode JX as_  (op0=0, op1=0, op2=0, r=0, t=0xA, s=as_).
fn enc_jx(as_: u32) -> u32 {
    st0(0, as_, 0xA)
}

/// Encode CALL0/4/8/12 (op0=0x5, n=0/1/2/3).
///
/// Computes imm18 from the ISA RM §4.4 formula:
///   base = (pc + 3) & !3
///   offset_bytes = target - base  (must be a multiple of 4)
///   imm18 = offset_bytes / 4
fn enc_call(n: u32, pc: u32, target: u32) -> u32 {
    let base = (pc.wrapping_add(3)) & !3u32;
    let offset_bytes = target.wrapping_sub(base) as i32;
    let imm18 = ((offset_bytes / 4) as u32) & 0x3_FFFF;
    0x5 | (n << 4) | (imm18 << 6)
}

/// Encode CALLX0/4/8/12 (op0=0, op1=0, op2=0, r=0, s=as_).
/// t field: 0xC=x0, 0xD=x4, 0xE=x8, 0xF=x12.
fn enc_callx(n: u32, as_: u32) -> u32 {
    let t = 0xC + n;
    st0(0, as_, t)
}

/// Encode RET  (op0=0, op1=0, op2=0, r=0, t=0x8).
fn enc_ret() -> u32 {
    st0(0, 0, 0x8)
}

/// Encode RETW  (op0=0, op1=0, op2=0, r=0, t=0x9).
fn enc_retw() -> u32 {
    st0(0, 0, 0x9)
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

// ── D4 Tests: Load instructions ────────────────────────────────────────────────

/// L8UI: zero-extends an 8-bit byte load.
/// Store 0xAB at DATA_ADDR; set a2 = DATA_ADDR; L8UI a3, a2, 0; expect a3 = 0xAB.
#[test]
fn test_exec_l8ui() {
    // Data lives at 0x2000_4000, instructions at TEST_PC (0x2000_0000).
    const DATA_ADDR: u64 = 0x2000_4000;

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Write test byte into RAM data region.
    bus.write_u8(DATA_ADDR, 0xAB).unwrap();

    // Set a2 = DATA_ADDR, then L8UI a3, a2, 0.
    // Note: MOVI only handles 12-bit signed immediates; load address via set_register.
    cpu.set_register(2, DATA_ADDR as u32);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_l8ui(3, 2, 0),  // L8UI a3, a2, 0 → a3 = mem[a2]
        st0(4, 0, 0),        // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0xAB, "L8UI: zero-extended byte load");
}

/// L16UI: zero-extends a 16-bit halfword load.
/// Store 0xBEEF at a 2-byte-aligned address; expect a3 = 0x0000BEEF.
#[test]
fn test_exec_l16ui() {
    const DATA_ADDR: u64 = 0x2000_4000; // 2-byte aligned

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Write 0xBEEF little-endian at DATA_ADDR.
    bus.write_u8(DATA_ADDR,     0xEF).unwrap();
    bus.write_u8(DATA_ADDR + 1, 0xBE).unwrap();

    cpu.set_register(2, DATA_ADDR as u32);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_l16ui(3, 2, 0),  // L16UI a3, a2, 0 → a3 = 0x0000BEEF
        st0(4, 0, 0),         // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0x0000_BEEF, "L16UI: zero-extended halfword");
}

/// L16SI positive: MSB of loaded 16-bit value is 0 → no sign-extension change.
/// Store 0x7FFE → a3 = 0x00007FFE.
#[test]
fn test_exec_l16si_positive() {
    const DATA_ADDR: u64 = 0x2000_4002; // 2-byte aligned

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // 0x7FFE little-endian: byte0=0xFE, byte1=0x7F.
    bus.write_u8(DATA_ADDR,     0xFE).unwrap();
    bus.write_u8(DATA_ADDR + 1, 0x7F).unwrap();

    cpu.set_register(2, DATA_ADDR as u32);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_l16si(3, 2, 0),  // L16SI a3, a2, 0 → a3 = sign_ext16(0x7FFE)
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0x0000_7FFE, "L16SI positive: no sign-fill");
}

/// L16SI negative: MSB of loaded 16-bit value is 1 → sign-extends to 0xFFFF8000.
/// Store 0x8000 → a3 = 0xFFFF8000.
#[test]
fn test_exec_l16si_negative() {
    const DATA_ADDR: u64 = 0x2000_4004; // 2-byte aligned

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // 0x8000 little-endian: byte0=0x00, byte1=0x80.
    bus.write_u8(DATA_ADDR,     0x00).unwrap();
    bus.write_u8(DATA_ADDR + 1, 0x80).unwrap();

    cpu.set_register(2, DATA_ADDR as u32);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_l16si(3, 2, 0),  // L16SI a3, a2, 0 → sign_ext16(0x8000) = 0xFFFF8000
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0xFFFF_8000, "L16SI negative: sign-fill");
}

/// L32I: loads a full 32-bit word.
/// Store 0xDEAD_BEEF at a 4-byte-aligned address; expect a3 = 0xDEADBEEF.
#[test]
fn test_exec_l32i() {
    const DATA_ADDR: u64 = 0x2000_4008; // 4-byte aligned

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // 0xDEADBEEF little-endian.
    bus.write_u8(DATA_ADDR,     0xEF).unwrap();
    bus.write_u8(DATA_ADDR + 1, 0xBE).unwrap();
    bus.write_u8(DATA_ADDR + 2, 0xAD).unwrap();
    bus.write_u8(DATA_ADDR + 3, 0xDE).unwrap();

    cpu.set_register(2, DATA_ADDR as u32);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_l32i(3, 2, 0),  // L32I a3, a2, 0 → a3 = 0xDEADBEEF
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0xDEAD_BEEF, "L32I: full 32-bit word load");
}

/// L32R: PC-relative load from literal pool.
///
/// Encoding math:
///   PC = 0x2000_1000 (instruction start for this test).
///   Literal at 0x2000_0F00.
///   base = (PC + 3) & !3 = 0x2000_1000 (already 4-byte aligned).
///   pc_rel_byte_offset = literal_addr - base = 0x2000_0F00 - 0x2000_1000 = -256.
///   word_offset = -256 / 4 = -64.
///   imm16 = (-64i32 as u16) = 0xFFC0.
///   Verify: sext16(0xFFC0) = -64; *4 = -256; EA = 0x2000_1000 + (-256) = 0x2000_0F00. ✓
#[test]
fn test_exec_l32r() {
    const INSN_PC: u32  = 0x2000_1000; // Instructions at this address.
    const LIT_ADDR: u64 = 0x2000_0F00; // Literal pool address (before PC).

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(INSN_PC);

    // Store literal value 0x1234_5678 at LIT_ADDR (4-byte aligned).
    bus.write_u8(LIT_ADDR,     0x78).unwrap();
    bus.write_u8(LIT_ADDR + 1, 0x56).unwrap();
    bus.write_u8(LIT_ADDR + 2, 0x34).unwrap();
    bus.write_u8(LIT_ADDR + 3, 0x12).unwrap();

    // Compute imm16: word_offset = (LIT_ADDR as i64 - INSN_PC as i64) / 4 = -64
    //   imm16 = (-64i32 as u32) & 0xFFFF = 0xFFC0
    let imm16: u32 = (-64i32 as u32) & 0xFFFF;

    write_insns(&mut bus, INSN_PC as u64, &[
        enc_l32r(3, imm16),  // L32R a3, literal → a3 = 0x12345678
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0x1234_5678, "L32R: PC-relative load");
}

// ── D5 Store Tests ──────────────────────────────────────────────────────────

/// S8I: store low byte of register to memory.
/// Write 0xAB12CD34 to a2, store byte 0x34 at memory address.
#[test]
fn test_exec_s8i() {
    const DATA_ADDR: u64 = 0x2000_4000;

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Initialize data area with zeros.
    bus.write_u8(DATA_ADDR, 0x00).unwrap();

    // Set a2 = 0xAB12CD34 (value to store); a3 = DATA_ADDR (address).
    cpu.set_register(2, 0xAB12CD34);
    cpu.set_register(3, DATA_ADDR as u32);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_s8i(2, 3, 0),  // S8I a2, a3, 0 → mem[a3] = 0x34 (low byte)
        st0(4, 0, 0),       // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(bus.read_u8(DATA_ADDR).unwrap(), 0x34, "S8I: low byte stored");
}

/// S16I: store low 16 bits of register to memory.
/// Write 0xAB12CD34 to a2, store 0xCD34 at 2-byte-aligned address.
#[test]
fn test_exec_s16i() {
    const DATA_ADDR: u64 = 0x2000_4000; // 2-byte aligned

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Initialize data area with zeros.
    bus.write_u8(DATA_ADDR,     0x00).unwrap();
    bus.write_u8(DATA_ADDR + 1, 0x00).unwrap();

    // Set a2 = 0xAB12CD34; a3 = DATA_ADDR.
    cpu.set_register(2, 0xAB12CD34);
    cpu.set_register(3, DATA_ADDR as u32);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_s16i(2, 3, 0),  // S16I a2, a3, 0 → mem16[a3] = 0xCD34
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);

    // Verify little-endian: 0xCD34 = [0x34, 0xCD]
    let byte0 = bus.read_u8(DATA_ADDR).unwrap();
    let byte1 = bus.read_u8(DATA_ADDR + 1).unwrap();
    let loaded = (byte1 as u32) << 8 | (byte0 as u32);
    assert_eq!(loaded, 0xCD34, "S16I: 16-bit halfword stored");
}

/// S32I: store full 32-bit register to memory.
/// Write 0xDEADBEEF to a2, store at 4-byte-aligned address.
#[test]
fn test_exec_s32i() {
    const DATA_ADDR: u64 = 0x2000_4008; // 4-byte aligned

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Initialize data area with zeros.
    for i in 0..4 {
        bus.write_u8(DATA_ADDR + i, 0x00).unwrap();
    }

    // Set a2 = 0xDEADBEEF; a3 = DATA_ADDR.
    cpu.set_register(2, 0xDEAD_BEEF);
    cpu.set_register(3, DATA_ADDR as u32);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_s32i(2, 3, 0),  // S32I a2, a3, 0 → mem32[a3] = 0xDEADBEEF
        st0(4, 0, 0),
    ]);

    run_until_error(&mut cpu, &mut bus);

    // Verify little-endian: 0xDEADBEEF = [0xEF, 0xBE, 0xAD, 0xDE]
    let stored = bus.read_u32(DATA_ADDR).unwrap();
    assert_eq!(stored, 0xDEAD_BEEF, "S32I: full 32-bit word stored");
}

// ── D6: Branch instruction encoders ────────────────────────────────────────

/// Encode a BR-format (op0=0x7) instruction: rri8_pack(op0=7, t, s, r, imm8).
/// `imm8` is the raw 8-bit field; decoder adds +4 to make the offset.
/// Caller provides the raw imm8 field value (i.e. desired_offset - 4, sign-truncated to u8).
fn enc_br(t: u32, s: u32, r: u32, imm8: u32) -> u32 {
    0x7 | ((t & 0xF) << 4) | ((s & 0xF) << 8) | ((r & 0xF) << 12) | ((imm8 & 0xFF) << 16)
}

/// Encode a BRI12-format branch: op0=6, n at bits[5:4], m at bits[7:6], s at bits[11:8], imm12 at bits[23:12].
/// `imm12` is the raw 12-bit field; decoder adds +4.
fn enc_bri12(m: u32, n: u32, s: u32, imm12: u32) -> u32 {
    0x6 | ((n & 0x3) << 4) | ((m & 0x3) << 6) | ((s & 0xF) << 8) | ((imm12 & 0xFFF) << 12)
}

/// Encode a BI/BIU-format branch: op0=6, n at bits[5:4], m at bits[7:6], s at bits[11:8],
/// r (B4CONST/B4CONSTU index) at bits[15:12], imm8 (offset raw) at bits[23:16].
fn enc_bi(m: u32, n: u32, s: u32, r: u32, imm8: u32) -> u32 {
    0x6 | ((n & 0x3) << 4) | ((m & 0x3) << 6) | ((s & 0xF) << 8) | ((r & 0xF) << 12) | ((imm8 & 0xFF) << 16)
}

/// BREAK instruction at position 0 (halts unconditionally).
fn break_insn() -> u32 { 4 << 12 }  // st0(r=4,s=0,t=0) = BREAK 0,0

// ── D6: Branch test infrastructure ─────────────────────────────────────────
//
// Layout for each branch test:
//   INSN_PC+0: the branch instruction (3 bytes)
//   INSN_PC+3: BREAK (not-taken path)
//   INSN_PC+6: BREAK (placeholder — not reached; taken target is elsewhere)
//
// For taken branch: target = INSN_PC + offset (offset pre-baked with +4 already).
// We pick offset = 9 (raw imm8 = 5, +4 = 9) to place taken BREAK at INSN_PC+9 (= 4th slot).
// Not-taken BREAK is at INSN_PC+3.
//
// Branch instruction is at TEST_PC.
//   offset_taken = 9  → target = TEST_PC + 9
//   not-taken falls through to TEST_PC + 3
//
// We write:
//   [0] branch insn     @ TEST_PC+0
//   [1] BREAK 0,0       @ TEST_PC+3   (not-taken marker)
//   [2] NOP (or BREAK)  @ TEST_PC+6   (padding — never hit)
//   [3] BREAK 0,0       @ TEST_PC+9   (taken marker)

/// Expect a taken branch: run until BREAK, assert PC == TEST_PC + 9 (taken target).
fn check_taken(cpu: &mut XtensaLx7, bus: &mut SystemBus) {
    let err = run_until_error(cpu, bus);
    match err {
        SimulationError::BreakpointHit(pc) => {
            assert_eq!(pc, TEST_PC + 9, "expected taken branch (PC = TEST_PC+9), got PC = {:#010x}", pc);
        }
        other => panic!("expected BreakpointHit, got {:?}", other),
    }
}

/// Expect a not-taken branch: run until BREAK, assert PC == TEST_PC + 3 (fall-through).
fn check_not_taken(cpu: &mut XtensaLx7, bus: &mut SystemBus) {
    let err = run_until_error(cpu, bus);
    match err {
        SimulationError::BreakpointHit(pc) => {
            assert_eq!(pc, TEST_PC + 3, "expected not-taken branch (PC = TEST_PC+3), got PC = {:#010x}", pc);
        }
        other => panic!("expected BreakpointHit, got {:?}", other),
    }
}

/// Write branch + two BREAK sentinels.
/// slot[0] = branch insn, slot[1] = not-taken BREAK, slot[2] = padding NOP, slot[3] = taken BREAK.
/// offset 9 = raw_imm8 5 + 4 (pre-baked by decoder).
fn write_branch_test(bus: &mut SystemBus, branch_insn: u32) {
    write_insns(bus, TEST_PC as u64, &[
        branch_insn,   // @ TEST_PC+0  — the branch, offset field = 9 (targets TEST_PC+9)
        break_insn(),  // @ TEST_PC+3  — not-taken sentinel
        0x000002,      // @ TEST_PC+6  — NOP (padding; never hit)
        break_insn(),  // @ TEST_PC+9  — taken sentinel
    ]);
}

/// Fresh CPU + bus with PC = TEST_PC.
fn make_cpu_bus() -> (XtensaLx7, SystemBus) {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    (cpu, bus)
}

// ── D6 Tests ─────────────────────────────────────────────────────────────────
//
// For BR-format branches (8-bit offset), raw_imm8=5 → offset = sext8(5)+4 = 9.
// Taken target = TEST_PC + 9; not-taken = TEST_PC + 3.

// BEQ: as_ == at
#[test]
fn test_exec_beq_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 42);
    cpu.set_register(3, 42);
    // BEQ a2, a3, offset=9 (r=0x1, imm8=5)
    write_branch_test(&mut bus, enc_br(3, 2, 0x1, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_beq_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 42);
    cpu.set_register(3, 99);
    write_branch_test(&mut bus, enc_br(3, 2, 0x1, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BNE: as_ != at
#[test]
fn test_exec_bne_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 1);
    cpu.set_register(3, 2);
    // BNE r=0x9
    write_branch_test(&mut bus, enc_br(3, 2, 0x9, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bne_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 7);
    cpu.set_register(3, 7);
    write_branch_test(&mut bus, enc_br(3, 2, 0x9, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BLT: signed as_ < at
#[test]
fn test_exec_blt_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, (-5i32) as u32);  // -5 signed
    cpu.set_register(3, 3);
    // BLT r=0x2
    write_branch_test(&mut bus, enc_br(3, 2, 0x2, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_blt_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 10);
    cpu.set_register(3, 3);
    write_branch_test(&mut bus, enc_br(3, 2, 0x2, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BGE: signed as_ >= at
#[test]
fn test_exec_bge_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 5);
    cpu.set_register(3, 5);
    // BGE r=0xA
    write_branch_test(&mut bus, enc_br(3, 2, 0xA, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bge_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, (-1i32) as u32);  // -1 signed < 0
    cpu.set_register(3, 0);
    write_branch_test(&mut bus, enc_br(3, 2, 0xA, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BLTU: unsigned as_ < at
#[test]
fn test_exec_bltu_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 3);
    cpu.set_register(3, 10);
    // BLTU r=0x3
    write_branch_test(&mut bus, enc_br(3, 2, 0x3, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bltu_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0xFFFF_FFFFu32);  // large unsigned
    cpu.set_register(3, 1);
    write_branch_test(&mut bus, enc_br(3, 2, 0x3, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BGEU: unsigned as_ >= at
#[test]
fn test_exec_bgeu_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0xFFFF_FFFFu32);
    cpu.set_register(3, 1);
    // BGEU r=0xB
    write_branch_test(&mut bus, enc_br(3, 2, 0xB, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bgeu_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0);
    cpu.set_register(3, 1);
    write_branch_test(&mut bus, enc_br(3, 2, 0xB, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BANY: (as_ & at) != 0
#[test]
fn test_exec_bany_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);
    cpu.set_register(3, 0b0011);
    // BANY r=0x8
    write_branch_test(&mut bus, enc_br(3, 2, 0x8, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bany_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);
    cpu.set_register(3, 0b0101);  // no bits in common
    write_branch_test(&mut bus, enc_br(3, 2, 0x8, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BALL: (as_ & at) == at  (all bits of at set in as_)
#[test]
fn test_exec_ball_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1111);
    cpu.set_register(3, 0b1010);  // all bits of at are in as_
    // BALL r=0x4
    write_branch_test(&mut bus, enc_br(3, 2, 0x4, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_ball_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);
    cpu.set_register(3, 0b1111);  // at has bits not in as_
    write_branch_test(&mut bus, enc_br(3, 2, 0x4, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BNONE: (as_ & at) == 0
#[test]
fn test_exec_bnone_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);
    cpu.set_register(3, 0b0101);
    // BNONE r=0x0
    write_branch_test(&mut bus, enc_br(3, 2, 0x0, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bnone_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);
    cpu.set_register(3, 0b0011);  // bit 1 in common
    write_branch_test(&mut bus, enc_br(3, 2, 0x0, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BNALL: (as_ & at) != at  (at least one bit of at missing in as_)
#[test]
fn test_exec_bnall_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);
    cpu.set_register(3, 0b1111);  // at has bit0,bit2 not in as_
    // BNALL r=0xC
    write_branch_test(&mut bus, enc_br(3, 2, 0xC, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bnall_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1111);
    cpu.set_register(3, 0b1010);  // all bits of at are in as_
    write_branch_test(&mut bus, enc_br(3, 2, 0xC, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BBC: bit (at & 0x1F) of as_ is CLEAR
#[test]
fn test_exec_bbc_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);  // bit 2 is CLEAR
    cpu.set_register(3, 2);       // check bit 2
    // BBC r=0x5
    write_branch_test(&mut bus, enc_br(3, 2, 0x5, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bbc_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);  // bit 1 is SET
    cpu.set_register(3, 1);       // check bit 1
    write_branch_test(&mut bus, enc_br(3, 2, 0x5, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BBS: bit (at & 0x1F) of as_ is SET
#[test]
fn test_exec_bbs_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);  // bit 3 is SET
    cpu.set_register(3, 3);       // check bit 3
    // BBS r=0xD
    write_branch_test(&mut bus, enc_br(3, 2, 0xD, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bbs_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);  // bit 2 is CLEAR
    cpu.set_register(3, 2);       // check bit 2
    write_branch_test(&mut bus, enc_br(3, 2, 0xD, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BBCI: bit (imm & 0x1F) of as_ is CLEAR
// Encode: r=0x6, t=bit_index (for bits 0..15, r & 1 = 0).
#[test]
fn test_exec_bbci_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);  // bit 2 is CLEAR
    // BBCI as_=a2, bit=2: r=0x6, t=2, s=2
    write_branch_test(&mut bus, enc_br(2, 2, 0x6, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bbci_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);  // bit 3 is SET
    // BBCI as_=a2, bit=3: r=0x6, t=3, s=2
    write_branch_test(&mut bus, enc_br(3, 2, 0x6, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BBSI: bit (imm & 0x1F) of as_ is SET
// Encode: r=0xE, t=bit_index (for bits 0..15, r & 1 = 0 → high bit of index = 0).
#[test]
fn test_exec_bbsi_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);  // bit 1 is SET
    // BBSI as_=a2, bit=1: r=0xE, t=1, s=2
    write_branch_test(&mut bus, enc_br(1, 2, 0xE, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bbsi_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0b1010);  // bit 2 is CLEAR
    // BBSI as_=a2, bit=2: r=0xE, t=2, s=2
    write_branch_test(&mut bus, enc_br(2, 2, 0xE, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BEQZ: as_ == 0
// BRI12 format: n=1, m=0, imm12=5 → offset = sext12(5)+4 = 9
#[test]
fn test_exec_beqz_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0);
    // BEQZ a2: n=1, m=0, s=2, imm12=5 → offset=9
    write_branch_test(&mut bus, enc_bri12(0, 1, 2, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_beqz_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 1);
    write_branch_test(&mut bus, enc_bri12(0, 1, 2, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BNEZ: as_ != 0
#[test]
fn test_exec_bnez_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 99);
    // BNEZ a2: n=1, m=1
    write_branch_test(&mut bus, enc_bri12(1, 1, 2, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bnez_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0);
    write_branch_test(&mut bus, enc_bri12(1, 1, 2, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BLTZ: (as_ as i32) < 0
#[test]
fn test_exec_bltz_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0x8000_0000u32);  // MSB set → negative
    // BLTZ a2: n=1, m=2
    write_branch_test(&mut bus, enc_bri12(2, 1, 2, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bltz_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0);
    write_branch_test(&mut bus, enc_bri12(2, 1, 2, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BGEZ: (as_ as i32) >= 0
#[test]
fn test_exec_bgez_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0);
    // BGEZ a2: n=1, m=3
    write_branch_test(&mut bus, enc_bri12(3, 1, 2, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bgez_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0xFFFF_FFFFu32);  // -1 as i32
    write_branch_test(&mut bus, enc_bri12(3, 1, 2, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BEQI: as_ == B4CONST[r]
// B4CONST[5] = 5. Encode: n=2, m=0, s=2, r=5, imm8=5 → offset=9.
#[test]
fn test_exec_beqi_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 5);  // B4CONST[5] = 5
    // BEQI a2, B4CONST[5]: n=2, m=0, r=5, imm8=5
    write_branch_test(&mut bus, enc_bi(0, 2, 2, 5, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_beqi_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 7);  // != B4CONST[5]=5
    write_branch_test(&mut bus, enc_bi(0, 2, 2, 5, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BNEI: as_ != B4CONST[r]
// B4CONST[3] = 3.
#[test]
fn test_exec_bnei_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 99);  // != B4CONST[3]=3
    // BNEI a2, B4CONST[3]: n=2, m=1, r=3
    write_branch_test(&mut bus, enc_bi(1, 2, 2, 3, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bnei_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 3);  // == B4CONST[3]=3
    write_branch_test(&mut bus, enc_bi(1, 2, 2, 3, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BLTI: (as_ as i32) < B4CONST[r]  (signed)
// B4CONST[4] = 4.
#[test]
fn test_exec_blti_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, (-1i32) as u32);  // -1 < 4
    // BLTI a2, B4CONST[4]: n=2, m=2, r=4
    write_branch_test(&mut bus, enc_bi(2, 2, 2, 4, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_blti_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 10);  // 10 >= 4
    write_branch_test(&mut bus, enc_bi(2, 2, 2, 4, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BGEI: (as_ as i32) >= B4CONST[r]  (signed)
// B4CONST[0] = -1.
#[test]
fn test_exec_bgei_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0);  // 0 >= -1
    // BGEI a2, B4CONST[0]=-1: n=2, m=3, r=0
    write_branch_test(&mut bus, enc_bi(3, 2, 2, 0, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bgei_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, (-2i32) as u32);  // -2 < -1
    write_branch_test(&mut bus, enc_bi(3, 2, 2, 0, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BLTUI: as_ < B4CONSTU[r]  (unsigned)
// B4CONSTU[5] = 5.
#[test]
fn test_exec_bltui_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 3);  // 3 < 5
    // BLTUI a2, B4CONSTU[5]: n=3, m=2, r=5
    write_branch_test(&mut bus, enc_bi(2, 3, 2, 5, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bltui_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0xFFFF_FFFFu32);  // huge unsigned >= 5
    write_branch_test(&mut bus, enc_bi(2, 3, 2, 5, 5));
    check_not_taken(&mut cpu, &mut bus);
}

// BGEUI: as_ >= B4CONSTU[r]  (unsigned)
// B4CONSTU[0] = 32768.
#[test]
fn test_exec_bgeui_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 0xFFFF_FFFFu32);  // >= 32768
    // BGEUI a2, B4CONSTU[0]=32768: n=3, m=3, r=0
    write_branch_test(&mut bus, enc_bi(3, 3, 2, 0, 5));
    check_taken(&mut cpu, &mut bus);
}

#[test]
fn test_exec_bgeui_not_taken() {
    let (mut cpu, mut bus) = make_cpu_bus();
    cpu.set_register(2, 100);  // 100 < 32768
    write_branch_test(&mut bus, enc_bi(3, 3, 2, 0, 5));
    check_not_taken(&mut cpu, &mut bus);
}

/// S8I + S16I roundtrip: store, then load back.
#[test]
fn test_exec_store_then_load_roundtrip() {
    const DATA_ADDR: u64 = 0x2000_4000;

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Initialize data area.
    for i in 0..4 {
        bus.write_u8(DATA_ADDR + i, 0x00).unwrap();
    }

    // Set a2 = 0xABCD1234; a3 = DATA_ADDR.
    cpu.set_register(2, 0xABCD1234);
    cpu.set_register(3, DATA_ADDR as u32);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_s32i(2, 3, 0),    // S32I a2, a3, 0 → store 0xABCD1234
        enc_l32i(4, 3, 0),    // L32I a4, a3, 0 → load back
        st0(5, 0, 0),          // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(4), 0xABCD1234, "roundtrip: store then load");
}

// ── D7: Jump / CALL / RET / CALLX / RETW tests ─────────────────────────────

/// J forward: jump over a MOVI, land on MOVI a2=42, then BREAK.
///
/// Layout (each instruction is 3 bytes):
///   PC+0: J +6  (offset_with_bias=6, sext18=2)
///   PC+3: MOVI a2, 99  ← should be skipped
///   PC+6: MOVI a2, 42
///   PC+9: BREAK
#[test]
fn test_exec_j_forward() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_j(6),           // J → PC+6
        movi(2, 99),        // skipped
        movi(2, 42),        // target
        st0(4, 1, 0xF),     // BREAK
    ]);

    let err = run_until_error(&mut cpu, &mut bus);
    assert!(matches!(err, SimulationError::BreakpointHit(_)), "expected BreakpointHit");
    assert_eq!(cpu.get_register(2), 42, "a2 should be 42 (MOVI at landing target)");
}

/// J backward: CPU starts at the J instruction and jumps back to BREAK.
///
/// Layout:
///   PC+0: BREAK
///   PC+3: J backward to PC+0  (offset_with_bias = (PC+0)-(PC+3) = -3)
///
/// CPU starts at PC+3; J lands at PC+0 (BREAK).
#[test]
fn test_exec_j_backward() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();

    // Write instructions at TEST_PC; start execution at the J instruction (TEST_PC+3).
    write_insns(&mut bus, TEST_PC as u64, &[
        st0(4, 1, 0xF),     // BREAK at PC+0
        enc_j(-3),          // J → PC+0  (offset_with_bias = -3)
    ]);

    cpu.set_pc(TEST_PC + 3);
    let err = run_until_error(&mut cpu, &mut bus);
    assert!(
        matches!(err, SimulationError::BreakpointHit(pc) if pc == TEST_PC),
        "expected BreakpointHit at TEST_PC, got {:?}", err
    );
}

/// JX as_: load target address into a3, JX a3 jumps there.
///
/// Layout:
///   PC+0: JX a3                 (a3 pre-loaded = PC+6)
///   PC+3: MOVI a2, 99           ← skipped
///   PC+6: MOVI a2, 42
///   PC+9: BREAK
#[test]
fn test_exec_jx() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Pre-load a3 with the target address.
    cpu.set_register(3, TEST_PC + 6);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_jx(3),          // JX a3 → PC+6
        movi(2, 99),        // skipped
        movi(2, 42),        // target
        st0(4, 1, 0xF),     // BREAK
    ]);

    let err = run_until_error(&mut cpu, &mut bus);
    assert!(matches!(err, SimulationError::BreakpointHit(_)), "expected BreakpointHit");
    assert_eq!(cpu.get_register(2), 42, "a2 should be 42 (JX landed at MOVI)");
}

/// CALL0 + RET round-trip: spec scenario "subroutine that returns a constant in a2."
///
/// Layout:
///   PC+0:  CALL0 subroutine          (target = PC+12, offset=12 from (PC+3)&!3=PC)
///   PC+3:  BREAK                     (halts after return; inspects a2)
///   PC+6:  (padding — 3 bytes not reached; needed to align subroutine to PC+12)
///   PC+12: MOVI a2, 42               (subroutine body)
///   PC+15: RET                        (returns to PC+3)
///
/// a0 after CALL0 = PC+3 (return address). RET → pc = a0 = PC+3 (BREAK).
#[test]
fn test_exec_call0_returns_via_ret() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // ((TEST_PC + 3) & !3) = TEST_PC  (TEST_PC is 4-aligned per ISA RM §4.4)
    // Target = TEST_PC + 12 → offset_bytes = 12 - 0 = 12
    write_insns(&mut bus, TEST_PC as u64, &[
        enc_call(0, TEST_PC, TEST_PC + 12), // CALL0 target=PC+12
        st0(4, 1, 0xF), // BREAK at PC+3
        movi(2, 0),     // padding at PC+6 (not reached)
    ]);
    write_insns(&mut bus, (TEST_PC + 12) as u64, &[
        movi(2, 42),    // MOVI a2, 42  at PC+12
        enc_ret(),      // RET           at PC+15
    ]);

    let err = run_until_error(&mut cpu, &mut bus);
    assert!(matches!(err, SimulationError::BreakpointHit(_)), "expected BreakpointHit");
    assert_eq!(cpu.get_register(2), 42, "a2 should be 42 (returned from subroutine)");
    assert_eq!(
        cpu.get_register(0),
        TEST_PC + 3,
        "a0 should be return address (PC+3)"
    );
}

/// CALLX0 + RET: register-indirect call round-trip.
///
/// a5 pre-loaded with subroutine address. CALLX0 a5 → a0 = PC+3, jump.
/// Subroutine: MOVI a2, 77; RET.
#[test]
fn test_exec_callx0_returns() {
    const SUBR: u32 = TEST_PC + 12;

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(5, SUBR);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_callx(0, 5), // CALLX0 a5 → jump to SUBR; a0 = PC+3
        st0(4, 1, 0xF),  // BREAK at PC+3
        movi(2, 0),      // padding at PC+6
    ]);
    write_insns(&mut bus, SUBR as u64, &[
        movi(2, 77),     // MOVI a2, 77
        enc_ret(),       // RET → pc = a0 = PC+3
    ]);

    let err = run_until_error(&mut cpu, &mut bus);
    assert!(matches!(err, SimulationError::BreakpointHit(_)), "expected BreakpointHit");
    assert_eq!(cpu.get_register(2), 77, "a2 should be 77");
}

/// CALL4: PS.CALLINC must be 1 after execution.
#[test]
fn test_exec_call4_sets_callinc() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // CALL4 + BREAK at subroutine so we can inspect state.
    write_insns(&mut bus, TEST_PC as u64, &[
        enc_call(1, TEST_PC, TEST_PC + 12), // CALL4 target=PC+12
    ]);
    write_insns(&mut bus, (TEST_PC + 12) as u64, &[
        st0(4, 1, 0xF), // BREAK at subroutine entry
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.ps.callinc(), 1, "PS.CALLINC should be 1 after CALL4");
}

/// CALL8: PS.CALLINC must be 2 after execution.
#[test]
fn test_exec_call8_sets_callinc() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_call(2, TEST_PC, TEST_PC + 12), // CALL8 target=PC+12
    ]);
    write_insns(&mut bus, (TEST_PC + 12) as u64, &[
        st0(4, 1, 0xF), // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.ps.callinc(), 2, "PS.CALLINC should be 2 after CALL8");
}

/// CALL12: PS.CALLINC must be 3 after execution.
#[test]
fn test_exec_call12_sets_callinc() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_call(3, TEST_PC, TEST_PC + 12), // CALL12 target=PC+12
    ]);
    write_insns(&mut bus, (TEST_PC + 12) as u64, &[
        st0(4, 1, 0xF), // BREAK
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.ps.callinc(), 3, "PS.CALLINC should be 3 after CALL12");
}

/// CALL4 writes the return PC into a4 (the register that becomes a0 after ENTRY rotates).
///
/// Per ISA RM §8: the return address stored in a[N] encodes the call type in bits[31:30].
/// CALL4 sets bits[31:30] = 01, so a4 = (PC+3 low30) | (1 << 30).
/// RETW recovers N = a0[31:30] = 1 to know how many windows to rotate back.
#[test]
fn test_exec_call4_writes_return_pc_to_a4() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_call(1, TEST_PC, TEST_PC + 12), // CALL4 at TEST_PC; a4 ← encoded(TEST_PC+3)
    ]);
    write_insns(&mut bus, (TEST_PC + 12) as u64, &[
        st0(4, 1, 0xF), // BREAK at subroutine entry
    ]);

    run_until_error(&mut cpu, &mut bus);
    let expected_a4 = ((TEST_PC + 3) & 0x3FFF_FFFF) | (1 << 30);
    assert_eq!(
        cpu.get_register(4),
        expected_a4,
        "a4 should hold return PC with N=1 in bits[31:30]"
    );
}

/// CALLX8 + register-indirect: verify jump target and return-PC placement in a8.
///
/// a7 pre-loaded with subroutine address. CALLX8 a7 → a8 = PC+3, jump to a7.
#[test]
fn test_exec_callx8_jumps_and_writes_return_pc() {
    const SUBR: u32 = TEST_PC + 12;

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(7, SUBR);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_callx(2, 7), // CALLX8 a7 → a8 = PC+3, jump to SUBR
    ]);
    write_insns(&mut bus, SUBR as u64, &[
        st0(4, 1, 0xF),  // BREAK at subroutine entry
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_pc(), SUBR, "PC should be at subroutine");
    // CALLX8 encodes N=2 in bits[31:30] of the return address (ISA RM §8)
    let expected_a8 = ((TEST_PC + 3) & 0x3FFF_FFFF) | (2 << 30);
    assert_eq!(cpu.get_register(8), expected_a8, "a8 should hold return PC with N=2 in bits[31:30]");
    assert_eq!(cpu.ps.callinc(), 2, "PS.CALLINC should be 2 for CALLX8");
}

// ── F1: ENTRY + RETW exec tests (no window OF/UF check) ─────────────────────
//
// HW-oracle encoding (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
//   entry a1, 32  → 004136 → word 0x004136  (op0=6, n=3, m=0, as_=1, imm12=4)
//   retw (wide)   → 000090 → word 0x000090  (ST0 group, r=0, t=9)
//
// ENTRY semantics (F1 — no overflow check):
//   WB_new = (WB_old + PS.CALLINC) mod 16
//   WindowStart[WB_new] = 1
//   PS.CALLINC = 0
//   a[as_] (in new window) -= imm * 8
//
// RETW semantics (F1 — no underflow check):
//   N = a0[31:30]
//   target_pc = (a[0] & 0x3FFF_FFFF) | (PC & 0xC000_0000)
//   WindowStart[WB_current] = 0
//   WB_new = (WB_current - N) mod 16
//   PC = target_pc

/// Encode ENTRY as_, imm12 (op0=6, n=3, m=0).
/// imm12 is the raw 12-bit field; stack decrement = imm12 * 8 bytes.
fn enc_entry(as_: u32, imm12: u32) -> u32 {
    // op0=6, bits[5:4]=3 (n=3), bits[7:6]=0 (m=0), as_=bits[11:8], imm12=bits[23:12]
    0x6 | (3 << 4) | (as_ << 8) | ((imm12 & 0xFFF) << 12)
}

/// ENTRY rotates WindowBase by PS.CALLINC (=1 for CALL4).
#[test]
fn test_exec_entry_rotates_window_base_by_callinc() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    // Set CALLINC=1 (as if CALL4 was just executed)
    cpu.ps.set_callinc(1);
    // Set a1 (stack pointer) to a known value so ENTRY doesn't crash on SP subtract
    cpu.set_register(1, 0x2005_0000);

    // ENTRY a1, 4  (imm12=4 → 32 bytes stack)  →  word 0x004136
    write_insns(&mut bus, TEST_PC as u64, &[enc_entry(1, 4)]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.regs.windowbase(), 1, "WindowBase should advance by CALLINC=1");
}

/// ENTRY with CALLINC=2 rotates WindowBase by 2.
#[test]
fn test_exec_entry_rotates_window_base_callinc2() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.ps.set_callinc(2);
    cpu.set_register(1, 0x2005_0000);

    write_insns(&mut bus, TEST_PC as u64, &[enc_entry(1, 4)]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.regs.windowbase(), 2, "WindowBase should advance by CALLINC=2");
}

/// ENTRY sets WindowStart bit for the new WindowBase.
#[test]
fn test_exec_entry_sets_windowstart_bit() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.ps.set_callinc(1);
    cpu.set_register(1, 0x2005_0000);

    write_insns(&mut bus, TEST_PC as u64, &[enc_entry(1, 4)]);
    cpu.step(&mut bus, &[]).unwrap();

    assert!(cpu.regs.windowstart_bit(1), "WindowStart bit 1 should be set after ENTRY with CALLINC=1");
}

/// ENTRY clears PS.CALLINC to 0 after rotation.
#[test]
fn test_exec_entry_clears_callinc() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.ps.set_callinc(2);
    cpu.set_register(1, 0x2005_0000);

    write_insns(&mut bus, TEST_PC as u64, &[enc_entry(1, 4)]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.ps.callinc(), 0, "PS.CALLINC must be 0 after ENTRY");
}

/// ENTRY decrements a[as_] (in the new window) by imm * 8 bytes.
/// Setup: CALLINC=1, WB starts at 0. After ENTRY, WB=1.
/// In new window (WB=1), a1 is at physical[(1*4+1) mod 64]=physical[5].
/// We pre-set physical[5] via write_logical after setting WB=1 temporarily.
/// Simpler: set a1 BEFORE rotation; after rotation with CALLINC=1,
/// the new frame's a1 is the old frame's a5 (physical[1*4+1=5]).
/// Use a direct physical write to set physical[5] = SP before ENTRY.
#[test]
fn test_exec_entry_allocates_stack() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.ps.set_callinc(1);

    // In WB=0, a1 is phys[1]. After ENTRY with CALLINC=1, WB=1, new a1 is phys[5].
    // Pre-load phys[5] = 0x2005_1000 via set_physical.
    cpu.regs.set_physical(5, 0x2005_1000);

    // ENTRY a1, 4  (imm12=4 → 32 bytes).
    write_insns(&mut bus, TEST_PC as u64, &[enc_entry(1, 4)]);
    cpu.step(&mut bus, &[]).unwrap();

    // After ENTRY: WB=1, new a1 (phys[5]) = 0x2005_1000 - 32
    assert_eq!(
        cpu.regs.read_logical(1),
        0x2005_1000 - 32,
        "ENTRY a1,4 should decrement SP by 32 bytes"
    );
}

/// ENTRY with maximum imm (imm12=0xFFF → 0xFFF * 8 = 32760 bytes).
#[test]
fn test_exec_entry_imm_max() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.ps.set_callinc(1);

    let initial_sp: u32 = 0x4000_0000;
    cpu.regs.set_physical(5, initial_sp);

    // ENTRY a1, 0xFFF  (imm12=0xFFF → 0xFFF * 8 = 32760 bytes).
    write_insns(&mut bus, TEST_PC as u64, &[enc_entry(1, 0xFFF)]);
    cpu.step(&mut bus, &[]).unwrap();

    let expected = initial_sp.wrapping_sub(0xFFF * 8);
    assert_eq!(
        cpu.regs.read_logical(1),
        expected,
        "ENTRY max imm should decrement SP by 0xFFF * 8 = 32760 bytes"
    );
}

/// Full CALL4 → ENTRY round-trip: callee can access return address via a0.
///
/// CALL4 sets caller's a4 = return_pc, CALLINC=1.
/// ENTRY rotates WB by 1: callee's a0 (phys[(WB_new*4)]) = caller's a4 (phys[4]).
/// Verify callee a0 holds the return address after ENTRY.
#[test]
fn test_exec_call4_entry_round_trip() {
    const CALLEE: u32 = TEST_PC + 12;

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    // Set a1 in caller to a valid stack pointer. After CALL4+ENTRY,
    // caller's a5 (phys[5]) becomes callee's a1. Pre-load it.
    cpu.regs.set_physical(5, 0x2005_0000);

    // CALL4 to CALLEE (CALLEE = TEST_PC + 12, must be 4-aligned)
    write_insns(&mut bus, TEST_PC as u64, &[
        enc_call(1, TEST_PC, CALLEE), // CALL4 CALLEE
    ]);
    // ENTRY a1, 4  (allocate 32-byte frame)
    write_insns(&mut bus, CALLEE as u64, &[
        enc_entry(1, 4),  // ENTRY a1, 32 bytes
        st0(4, 1, 0xF),   // BREAK to halt
    ]);

    run_until_error(&mut cpu, &mut bus);

    // After ENTRY: WB=1. Callee's a0 = phys[4] = caller's a4.
    // CALL4 wrote (TEST_PC+3 low30) | (1<<30) into caller's a4 (phys[4]).
    let expected_ret = ((TEST_PC + 3) & 0x3FFF_FFFF) | (1 << 30);
    assert_eq!(
        cpu.regs.read_logical(0),
        expected_ret,
        "callee a0 should hold encoded return address after CALL4+ENTRY"
    );
    assert_eq!(cpu.regs.windowbase(), 1, "WB should be 1 after CALL4+ENTRY");
}

// ── RETW exec tests ──────────────────────────────────────────────────────────

/// Helper: set up a minimal windowed call frame for RETW tests.
/// Returns the CPU with WB=1, WindowStart[0]=1, WindowStart[1]=1,
/// a0 of the current (WB=1) frame encoding N=1 and a fake return PC.
fn setup_retw_frame(cpu: &mut XtensaLx7, ret_pc: u32) {
    // WB = 1 (callee frame)
    cpu.regs.set_windowbase(1);
    // WindowStart[0] = 1 (caller still live), WindowStart[1] = 1 (callee)
    cpu.regs.set_windowstart(0b0000_0000_0000_0011);
    // a0 in callee (WB=1) = phys[4] = ret_pc with N=1 encoded in bits[31:30]
    // N=1 → bits[31:30] = 0b01
    let a0_val = (ret_pc & 0x3FFF_FFFF) | (1 << 30);
    cpu.regs.set_physical(4, a0_val);
}

/// RETW rotates WindowBase back by N (N=1).
#[test]
fn test_exec_retw_rotates_window_back() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    setup_retw_frame(&mut cpu, TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[enc_retw()]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.regs.windowbase(), 0, "RETW should restore WindowBase to 0 (1 - N=1)");
}

/// RETW clears the WindowStart bit for the old WindowBase.
#[test]
fn test_exec_retw_clears_windowstart_bit() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    setup_retw_frame(&mut cpu, TEST_PC);
    assert!(cpu.regs.windowstart_bit(1), "pre-condition: WS[1] should be set");

    write_insns(&mut bus, TEST_PC as u64, &[enc_retw()]);
    cpu.step(&mut bus, &[]).unwrap();

    assert!(!cpu.regs.windowstart_bit(1), "RETW should clear WindowStart bit 1 (old WB)");
}

/// RETW sets PC = (a0 & 0x3FFF_FFFF) | (cur_pc & 0xC000_0000).
#[test]
fn test_exec_retw_jumps_to_a0_low30() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Encode return address with N=1 in high 2 bits, and 0x2000_0100 as the target
    let target = 0x2000_0100u32;
    let a0_val = (target & 0x3FFF_FFFF) | (1u32 << 30);
    // Set up WB=1, WS[1]=1, a0 in callee frame
    cpu.regs.set_windowbase(1);
    cpu.regs.set_windowstart(0b11);
    cpu.regs.set_physical(4, a0_val);  // phys[4] = callee's a0 when WB=1

    write_insns(&mut bus, TEST_PC as u64, &[enc_retw()]);
    cpu.step(&mut bus, &[]).unwrap();

    let expected_pc = (target & 0x3FFF_FFFF) | (TEST_PC & 0xC000_0000);
    assert_eq!(
        cpu.get_pc(),
        expected_pc,
        "RETW should set PC = (a0 low30) | (old_pc high2)"
    );
}

/// Full CALL4 → ENTRY → RETW cycle: caller resumes at instruction after CALL4.
///
/// Layout:
///   TEST_PC+0:  CALL4 → CALLEE    (3 bytes)
///   TEST_PC+3:  BREAK             (3 bytes) ← where RETW should return
///   CALLEE+0:   ENTRY a1, 4       (3 bytes)
///   CALLEE+3:   RETW              (3 bytes)
#[test]
fn test_exec_call4_entry_retw_full_cycle() {
    const CALLEE: u32 = TEST_PC + 12;  // 4-byte aligned target

    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    // Pre-load callee's a1 (= caller's a5 = phys[5]) for stack allocation
    cpu.regs.set_physical(5, 0x2005_0000);

    // Caller code
    write_insns(&mut bus, TEST_PC as u64, &[
        enc_call(1, TEST_PC, CALLEE),   // CALL4 to CALLEE (3 bytes at TEST_PC)
        st0(4, 1, 0xF),                 // BREAK at TEST_PC+3 (return point)
    ]);
    // Callee code
    write_insns(&mut bus, CALLEE as u64, &[
        enc_entry(1, 4),    // ENTRY a1, 32 bytes
        enc_retw(),         // RETW back to caller
    ]);

    let err = run_until_error(&mut cpu, &mut bus);
    assert!(
        matches!(err, SimulationError::BreakpointHit(_)),
        "expected BreakpointHit at return, got {:?}", err
    );
    // RETW should have jumped to the return address: TEST_PC + 3
    // (CALL4 wrote TEST_PC+3 into caller's a4, which is callee's a0)
    if let SimulationError::BreakpointHit(pc) = err {
        assert_eq!(
            pc, TEST_PC + 3,
            "RETW should return to TEST_PC+3 (instruction after CALL4)"
        );
    }
    // After RETW, WB should be back to 0
    assert_eq!(cpu.regs.windowbase(), 0, "WB should be restored to 0 after RETW");
    // WindowStart[1] should be cleared
    assert!(!cpu.regs.windowstart_bit(1), "WS[1] should be cleared by RETW");
}

// ── D8: Narrow (Code Density) exec tests ────────────────────────────────────
//
// Narrow instructions are 2 bytes. The fetch loop in step() reads byte0, calls
// instruction_length(), and dispatches to decode_narrow() when len=2. PC must
// advance by 2, not 3, after each narrow instruction.
//
// Helper: write a narrow (2-byte) instruction to the bus at a specific address.
fn write_narrow(bus: &mut SystemBus, addr: u64, hw: u16) {
    bus.write_u8(addr,     (hw & 0xFF) as u8).unwrap();
    bus.write_u8(addr + 1, (hw >> 8) as u8).unwrap();
}

/// ADD.N: PC advances by 2, not 3.
///
/// Layout:
///   TEST_PC+0: add.n a3, a4, a5   (2 bytes, 0x345a)
///   TEST_PC+2: BREAK               (3 bytes, to halt)
///
/// a4=10, a5=7 → a3=17. PC should advance by 2 from TEST_PC.
#[test]
fn test_exec_add_n_advances_pc_by_2() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 10);
    cpu.set_register(5, 7);

    // add.n a3, a4, a5 → 0x345a (HW-oracle verified)
    write_narrow(&mut bus, TEST_PC as u64, 0x345a);
    // BREAK at TEST_PC+2 (3 bytes)
    write_insns(&mut bus, (TEST_PC + 2) as u64, &[break_insn()]);

    let err = run_until_error(&mut cpu, &mut bus);
    assert!(matches!(err, SimulationError::BreakpointHit(_)), "expected BREAK, got {:?}", err);
    assert_eq!(cpu.get_register(3), 17, "a3 = a4+a5 = 10+7 = 17");
    // The BREAK was at TEST_PC+2, confirming narrow advanced PC by 2
    if let SimulationError::BreakpointHit(pc) = err {
        assert_eq!(pc, TEST_PC + 2, "BREAK at TEST_PC+2 means PC advanced by 2");
    }
}

/// MOV.N: implemented as OR ar, as_, as_. Reads correctly.
///
/// Layout:
///   TEST_PC+0: mov.n a3, a4   (2 bytes, 0x043d)
///   TEST_PC+2: BREAK
///
/// a4=0xDEAD_BEEF → a3=0xDEAD_BEEF.
#[test]
fn test_exec_mov_n_via_or() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0xDEAD_BEEF);

    // mov.n a3, a4 → 0x043d (HW-oracle verified)
    write_narrow(&mut bus, TEST_PC as u64, 0x043d);
    write_insns(&mut bus, (TEST_PC + 2) as u64, &[break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0xDEAD_BEEF, "mov.n should copy a4 to a3");
}

/// RET.N: restores PC from a0, advances PC by 2 internally (then sets to a0).
///
/// Layout:
///   TEST_PC+0: ret.n   (2 bytes, 0xf00d)
///   Execution resumes at a0.
///
/// Set a0 = TEST_PC + 100; after ret.n, PC should be TEST_PC+100.
#[test]
fn test_exec_ret_n_returns() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(0, TEST_PC + 100);  // return address

    // ret.n → 0xf00d (HW-oracle verified)
    write_narrow(&mut bus, TEST_PC as u64, 0xf00d);
    // BREAK at return address
    write_insns(&mut bus, (TEST_PC + 100) as u64, &[break_insn()]);

    let err = run_until_error(&mut cpu, &mut bus);
    match err {
        SimulationError::BreakpointHit(pc) => {
            assert_eq!(pc, TEST_PC + 100, "ret.n should set PC = a0 = TEST_PC+100");
        }
        other => panic!("expected BreakpointHit, got {:?}", other),
    }
}

/// MOVI.N with negative value: sign extension end-to-end.
///
/// movi.n a3, -32 → 0x036c (HW-oracle verified)
/// Expected: a3 = 0xFFFFFFE0 (two's complement -32 sign-extended to 32 bits).
#[test]
fn test_exec_movi_n_negative_value() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // movi.n a3, -32 → 0x036c (HW-oracle: bytes 6c 03)
    write_narrow(&mut bus, TEST_PC as u64, 0x036c);
    write_insns(&mut bus, (TEST_PC + 2) as u64, &[break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(
        cpu.get_register(3),
        (-32i32) as u32,
        "movi.n a3,-32: a3 should be 0xFFFFFFE0"
    );
}

// ── E1: MUL family execution tests ───────────────────────────────────────────
//
// Encoding helpers (HW-oracle verified):
//   MULL   a3,a4,a5 → rrr(0x8, 0x2, ar, as_, at)   op2=0x8 op1=0x2
//   MULUH  a3,a4,a5 → rrr(0xA, 0x2, ar, as_, at)   op2=0xA op1=0x2
//   MULSH  a3,a4,a5 → rrr(0xB, 0x2, ar, as_, at)   op2=0xB op1=0x2
//   MUL16U a3,a4,a5 → rrr(0xC, 0x1, ar, as_, at)   op2=0xC op1=0x1
//   MUL16S a3,a4,a5 → rrr(0xD, 0x1, ar, as_, at)   op2=0xD op1=0x1

fn enc_mull(ar: u32, as_: u32, at: u32)   -> u32 { rrr(0x8, 0x2, ar, as_, at) }
fn enc_muluh(ar: u32, as_: u32, at: u32)  -> u32 { rrr(0xA, 0x2, ar, as_, at) }
fn enc_mulsh(ar: u32, as_: u32, at: u32)  -> u32 { rrr(0xB, 0x2, ar, as_, at) }
fn enc_mul16u(ar: u32, as_: u32, at: u32) -> u32 { rrr(0xC, 0x1, ar, as_, at) }
fn enc_mul16s(ar: u32, as_: u32, at: u32) -> u32 { rrr(0xD, 0x1, ar, as_, at) }

/// MULL basic: 0x12345678 * 2 = 0x2468ACF0 (low 32 bits).
#[test]
fn test_exec_mull_basic() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0x12345678);
    cpu.set_register(5, 2);

    write_insns(&mut bus, TEST_PC as u64, &[enc_mull(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0x2468ACF0, "mull: 0x12345678 * 2 should give 0x2468ACF0");
}

/// MULL overflow wraps: 0xFFFFFFFF * 0xFFFFFFFF low-32 = 0x00000001.
#[test]
fn test_exec_mull_overflow_wraps() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0xFFFFFFFF);
    cpu.set_register(5, 0xFFFFFFFF);

    write_insns(&mut bus, TEST_PC as u64, &[enc_mull(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    // 0xFFFFFFFF * 0xFFFFFFFF = 0xFFFFFFFE_00000001 → low32 = 0x00000001
    assert_eq!(cpu.get_register(3), 0x00000001, "mull: 0xFFFFFFFF * 0xFFFFFFFF low32 should wrap to 0x1");
}

/// MULUH basic: upper 32 bits of unsigned 0x80000000 * 0x80000000 = 0x40000000.
///
/// 0x80000000 * 0x80000000 = 0x4000_0000_0000_0000 → upper32 = 0x40000000.
#[test]
fn test_exec_muluh_basic() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0x80000000);
    cpu.set_register(5, 0x80000000);

    write_insns(&mut bus, TEST_PC as u64, &[enc_muluh(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0x40000000, "muluh: 0x80000000*0x80000000 upper32 should be 0x40000000");
}

/// MULSH positive × positive: upper 32 bits of small product fits in 32 bits → upper = 0.
#[test]
fn test_exec_mulsh_positive() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 100);
    cpu.set_register(5, 200);

    write_insns(&mut bus, TEST_PC as u64, &[enc_mulsh(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    // 100 * 200 = 20000; fits in 32 bits → upper32 = 0
    assert_eq!(cpu.get_register(3), 0, "mulsh: 100*200 upper32 should be 0");
}

/// MULSH negative × negative: (-2^30) * (-2^30) = 2^60 → upper32 = 0x10000000.
#[test]
fn test_exec_mulsh_negative() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    // -0x40000000 as i32 = 0xC0000000 as u32
    cpu.set_register(4, (-0x40000000i32) as u32);
    cpu.set_register(5, (-0x40000000i32) as u32);

    write_insns(&mut bus, TEST_PC as u64, &[enc_mulsh(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    // (-2^30) * (-2^30) = 2^60 = 0x1000_0000_0000_0000 → upper32 = 0x10000000
    assert_eq!(cpu.get_register(3), 0x10000000, "mulsh: (-2^30)*(-2^30) upper32 should be 0x10000000");
}

/// MULSH mixed sign: (-2^30) * (2^30) = -2^60 → upper32 = 0xF0000000.
#[test]
fn test_exec_mulsh_mixed() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, (-0x40000000i32) as u32);  // 0xC0000000
    cpu.set_register(5, 0x40000000u32);

    write_insns(&mut bus, TEST_PC as u64, &[enc_mulsh(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    // (-2^30) * (2^30) = -2^60; upper32 of -2^60 sign-extended = 0xF0000000
    assert_eq!(cpu.get_register(3), 0xF0000000, "mulsh: (-2^30)*(2^30) upper32 should be 0xF0000000");
}

/// MUL16U zero-extend: only low 16 bits of each operand used; high bits ignored.
///
/// a4 = 0xABCD_1234, a5 = 0xFFFF_5678 → only 0x1234 * 0x5678 = 0x06260060.
#[test]
fn test_exec_mul16u_zero_extend() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0xABCD1234);
    cpu.set_register(5, 0xFFFF5678);

    write_insns(&mut bus, TEST_PC as u64, &[enc_mul16u(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    // 0x1234 * 0x5678 = 0x06260060; high bits of inputs are ignored
    assert_eq!(cpu.get_register(3), 0x06260060, "mul16u: 0x1234*0x5678 = 0x06260060; high bits ignored");
}

/// MUL16S sign-extend: low 16 bits sign-extended before multiply.
///
/// a4 = 0xFFFF_8000 (low16 = 0x8000 = -32768), a5 = 2 → -32768*2 = -65536 = 0xFFFF_0000.
#[test]
fn test_exec_mul16s_sign_extend() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0xFFFF8000);  // low16 = 0x8000 = -32768 as i16
    cpu.set_register(5, 2);

    write_insns(&mut bus, TEST_PC as u64, &[enc_mul16s(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    // (-32768) * 2 = -65536 = 0xFFFF0000 as u32
    assert_eq!(cpu.get_register(3), 0xFFFF0000, "mul16s: 0x8000 (=-32768) * 2 = 0xFFFF0000");
}

// ── E2: DIV family exec tests ─────────────────────────────────────────────────
//
// HW-oracle encodings (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
//   quos a3,a4,a5 → 0xD23450  quou a3,a4,a5 → 0xC23450
//   rems a3,a4,a5 → 0xF23450  remu a3,a4,a5 → 0xE23450
//
// Field layout: op0=0, op1=0x2; op2=0xD(QUOS) 0xC(QUOU) 0xF(REMS) 0xE(REMU)

fn enc_quos(ar: u32, as_: u32, at: u32) -> u32 { rrr(0xD, 0x2, ar, as_, at) }
fn enc_quou(ar: u32, as_: u32, at: u32) -> u32 { rrr(0xC, 0x2, ar, as_, at) }
fn enc_rems(ar: u32, as_: u32, at: u32) -> u32 { rrr(0xF, 0x2, ar, as_, at) }
fn enc_remu(ar: u32, as_: u32, at: u32) -> u32 { rrr(0xE, 0x2, ar, as_, at) }

/// QUOS basic: 100 / 7 = 14.
#[test]
fn test_exec_quos_basic() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 100);
    cpu.set_register(5, 7);

    write_insns(&mut bus, TEST_PC as u64, &[enc_quos(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 14, "quos: 100 / 7 should be 14");
}

/// QUOS negative dividend: -100 / 7 = -14 (truncation toward zero, sign of dividend).
#[test]
fn test_exec_quos_negative_dividend() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, (-100i32) as u32);
    cpu.set_register(5, 7);

    write_insns(&mut bus, TEST_PC as u64, &[enc_quos(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3) as i32, -14, "quos: -100 / 7 should be -14");
}

/// QUOS i32::MIN / -1 = i32::MIN (wrapping/saturating per ISA RM §8 — no exception).
#[test]
fn test_exec_quos_min_div_neg_one() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, i32::MIN as u32);
    cpu.set_register(5, (-1i32) as u32);

    write_insns(&mut bus, TEST_PC as u64, &[enc_quos(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3) as i32, i32::MIN, "quos: i32::MIN / -1 should wrap to i32::MIN (no exception)");
}

/// QUOU basic: 100 / 7 = 14 (unsigned).
#[test]
fn test_exec_quou_basic() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 100);
    cpu.set_register(5, 7);

    write_insns(&mut bus, TEST_PC as u64, &[enc_quou(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 14, "quou: 100 / 7 should be 14");
}

/// QUOU large unsigned: 0xF0000000 / 0x10000000 = 0xF.
#[test]
fn test_exec_quou_large_unsigned() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0xF0000000);
    cpu.set_register(5, 0x10000000);

    write_insns(&mut bus, TEST_PC as u64, &[enc_quou(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0xF, "quou: 0xF0000000 / 0x10000000 should be 0xF");
}

/// REMS basic: 100 % 7 = 2.
#[test]
fn test_exec_rems_basic() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 100);
    cpu.set_register(5, 7);

    write_insns(&mut bus, TEST_PC as u64, &[enc_rems(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 2, "rems: 100 % 7 should be 2");
}

/// REMS negative dividend: -100 % 7 = -2 (sign of result follows dividend).
#[test]
fn test_exec_rems_negative_dividend() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, (-100i32) as u32);
    cpu.set_register(5, 7);

    write_insns(&mut bus, TEST_PC as u64, &[enc_rems(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3) as i32, -2, "rems: -100 % 7 should be -2 (sign of dividend)");
}

/// REMU basic: 100 % 7 = 2 (unsigned).
#[test]
fn test_exec_remu_basic() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 100);
    cpu.set_register(5, 7);

    write_insns(&mut bus, TEST_PC as u64, &[enc_remu(3, 4, 5), break_insn()]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 2, "remu: 100 % 7 should be 2");
}

/// QUOS divide by zero: sets EXCCAUSE=6 and returns ExceptionRaised{cause:6}.
#[test]
fn test_exec_quos_div_by_zero_raises_exception() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 100);
    cpu.set_register(5, 0);  // divisor = 0

    write_insns(&mut bus, TEST_PC as u64, &[enc_quos(3, 4, 5)]);

    let err = cpu.step(&mut bus, &[]).unwrap_err();
    assert!(
        matches!(err, SimulationError::ExceptionRaised { cause: 6, .. }),
        "quos div-by-zero should return ExceptionRaised{{cause:6}}, got: {:?}", err
    );
    assert_eq!(cpu.sr.read(EXCCAUSE_ID), 6, "EXCCAUSE SR should be 6 after div-by-zero");
}

/// QUOU divide by zero: sets EXCCAUSE=6 and returns ExceptionRaised{cause:6}.
#[test]
fn test_exec_quou_div_by_zero() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 42);
    cpu.set_register(5, 0);  // divisor = 0

    write_insns(&mut bus, TEST_PC as u64, &[enc_quou(3, 4, 5)]);

    let err = cpu.step(&mut bus, &[]).unwrap_err();
    assert!(
        matches!(err, SimulationError::ExceptionRaised { cause: 6, .. }),
        "quou div-by-zero should return ExceptionRaised{{cause:6}}, got: {:?}", err
    );
    assert_eq!(cpu.sr.read(EXCCAUSE_ID), 6, "EXCCAUSE SR should be 6 after div-by-zero");
}

/// REMS divide by zero: sets EXCCAUSE=6 and returns ExceptionRaised{cause:6}.
#[test]
fn test_exec_rems_div_by_zero() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, (-7i32) as u32);
    cpu.set_register(5, 0);  // divisor = 0

    write_insns(&mut bus, TEST_PC as u64, &[enc_rems(3, 4, 5)]);

    let err = cpu.step(&mut bus, &[]).unwrap_err();
    assert!(
        matches!(err, SimulationError::ExceptionRaised { cause: 6, .. }),
        "rems div-by-zero should return ExceptionRaised{{cause:6}}, got: {:?}", err
    );
    assert_eq!(cpu.sr.read(EXCCAUSE_ID), 6, "EXCCAUSE SR should be 6 after div-by-zero");
}

/// REMU divide by zero: sets EXCCAUSE=6 and returns ExceptionRaised{cause:6}.
#[test]
fn test_exec_remu_div_by_zero() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.set_register(4, 99);
    cpu.set_register(5, 0);  // divisor = 0

    write_insns(&mut bus, TEST_PC as u64, &[enc_remu(3, 4, 5)]);

    let err = cpu.step(&mut bus, &[]).unwrap_err();
    assert!(
        matches!(err, SimulationError::ExceptionRaised { cause: 6, .. }),
        "remu div-by-zero should return ExceptionRaised{{cause:6}}, got: {:?}", err
    );
    assert_eq!(cpu.sr.read(EXCCAUSE_ID), 6, "EXCCAUSE SR should be 6 after div-by-zero");
}

// ── E3: Bit-manip encoding helpers ────────────────────────────────────────────
//
// NSA/NSAU field layout (op0=0, op1=0):
//   r=0xE(NSA)/0xF(NSAU) is constant subop; op2=ar, s=as_.
//   Word = (ar << 20) | (0xE << 12) | (as_ << 8)  [for NSA]
// MIN/MAX/MINU/MAXU field layout (op0=3):
//   Byte0 = (subop << 4) | 3; s=as_; r=ar; op2=at; op1=0
//   Word = (at << 20) | (r << 12) | (s << 8) | (subop << 4) | 3
// SEXT/CLAMPS field layout (op0=3):
//   Word = ((sa-7) << 20) | (r << 12) | (s << 8) | (subop << 4) | 3

/// Encode NSA ar, as_ (op0=0, op1=0, op2=4, r=0xE, s=as_, t=ar).
/// HW-oracle: nsa a3, a4 → objdump 40e430 → word 0x40E430.
fn enc_nsa(ar: u32, as_: u32) -> u32 {
    (4u32 << 20) | (0xEu32 << 12) | (as_ << 8) | (ar << 4)
}

/// Encode NSAU ar, as_ (op0=0, op1=0, op2=4, r=0xF, s=as_, t=ar).
/// HW-oracle: nsau a3, a4 → objdump 40f430 → word 0x40F430.
fn enc_nsau(ar: u32, as_: u32) -> u32 {
    (4u32 << 20) | (0xFu32 << 12) | (as_ << 8) | (ar << 4)
}

/// Encode MIN/MAX/MINU/MAXU (op0=3, subop in t field, r=ar, s=as_, op2=at).
fn enc_lsci_rrr(subop: u32, ar: u32, as_: u32, at: u32) -> u32 {
    (at << 20) | (ar << 12) | (as_ << 8) | (subop << 4) | 3
}

fn enc_min(ar: u32, as_: u32, at: u32)  -> u32 { enc_lsci_rrr(4, ar, as_, at) }
fn enc_max(ar: u32, as_: u32, at: u32)  -> u32 { enc_lsci_rrr(5, ar, as_, at) }
fn enc_minu(ar: u32, as_: u32, at: u32) -> u32 { enc_lsci_rrr(6, ar, as_, at) }
fn enc_maxu(ar: u32, as_: u32, at: u32) -> u32 { enc_lsci_rrr(7, ar, as_, at) }

/// Encode SEXT ar, as_, sa (op0=3, t=2, r=ar, s=as_, op2=sa-7).
/// sa must be 7..=22.
fn enc_sext(ar: u32, as_: u32, sa: u32) -> u32 {
    let raw_imm = sa - 7;
    (raw_imm << 20) | (ar << 12) | (as_ << 8) | (2u32 << 4) | 3
}

/// Encode CLAMPS ar, as_, sa (op0=3, t=3, r=ar, s=as_, op2=sa-7).
/// sa must be 7..=22.
fn enc_clamps(ar: u32, as_: u32, sa: u32) -> u32 {
    let raw_imm = sa - 7;
    (raw_imm << 20) | (ar << 12) | (as_ << 8) | (3u32 << 4) | 3
}

// ── E3: Bit-manip execution tests ─────────────────────────────────────────────

fn step_ok(cpu: &mut XtensaLx7, bus: &mut SystemBus) {
    cpu.step(bus, &[]).expect("step should succeed");
}

/// NSA(0) = 31: clz(0) - 1 = 32 - 1 = 31.
#[test]
fn test_exec_nsa_zero() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0);
    write_insns(&mut bus, TEST_PC as u64, &[enc_nsa(3, 4)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 31, "NSA(0) should be 31");
}

/// NSA(0x10) = 26: clz(16) - 1 = 27 - 1 = 26.
#[test]
fn test_exec_nsa_positive() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0x10);
    write_insns(&mut bus, TEST_PC as u64, &[enc_nsa(3, 4)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 26, "NSA(0x10) should be 26");
}

/// NSA(0xFFFFFFFF = -1) = 31: clz(!0xFFFFFFFF) - 1 = clz(0) - 1 = 31.
#[test]
fn test_exec_nsa_negative() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0xFFFFFFFF);
    write_insns(&mut bus, TEST_PC as u64, &[enc_nsa(3, 4)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 31, "NSA(-1) should be 31");
}

/// NSA(0x80000000 = i32::MIN) = 0: clz(!0x80000000) - 1 = clz(0x7FFFFFFF) - 1 = 1 - 1 = 0.
#[test]
fn test_exec_nsa_min_signed() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0x80000000);
    write_insns(&mut bus, TEST_PC as u64, &[enc_nsa(3, 4)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0, "NSA(0x80000000) should be 0");
}

/// NSAU(0) = 32: clz(0) = 32 (Rust u32::leading_zeros).
#[test]
fn test_exec_nsau_zero() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0);
    write_insns(&mut bus, TEST_PC as u64, &[enc_nsau(3, 4)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 32, "NSAU(0) should be 32");
}

/// NSAU(0x10) = 27: clz(16) = 27.
#[test]
fn test_exec_nsau_basic() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0x10);
    write_insns(&mut bus, TEST_PC as u64, &[enc_nsau(3, 4)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 27, "NSAU(0x10) should be 27");
}

/// NSAU(0x80000000) = 0: clz(0x80000000) = 0.
#[test]
fn test_exec_nsau_high_bit() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0x80000000);
    write_insns(&mut bus, TEST_PC as u64, &[enc_nsau(3, 4)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0, "NSAU(0x80000000) should be 0");
}

/// MIN(5, 7) = 5 (signed min).
#[test]
fn test_exec_min_basic() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 5); cpu.set_register(5, 7);
    write_insns(&mut bus, TEST_PC as u64, &[enc_min(3, 4, 5)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 5, "MIN(5,7) should be 5");
}

/// MIN(-1, 1) = -1 (signed min; -1 as u32 = 0xFFFFFFFF).
#[test]
fn test_exec_min_negative() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, (-1i32) as u32); cpu.set_register(5, 1);
    write_insns(&mut bus, TEST_PC as u64, &[enc_min(3, 4, 5)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), (-1i32) as u32, "MIN(-1,1) should be -1");
}

/// MAX(5, 7) = 7 (signed max).
#[test]
fn test_exec_max_basic() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 5); cpu.set_register(5, 7);
    write_insns(&mut bus, TEST_PC as u64, &[enc_max(3, 4, 5)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 7, "MAX(5,7) should be 7");
}

/// MAX(-1, 1) = 1 (signed max; -1 as signed is less than 1).
#[test]
fn test_exec_max_negative() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, (-1i32) as u32); cpu.set_register(5, 1);
    write_insns(&mut bus, TEST_PC as u64, &[enc_max(3, 4, 5)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 1, "MAX(-1,1) should be 1");
}

/// MINU(0xFFFFFFFF, 1) = 1 (0xFFFFFFFF is huge unsigned; 1 wins).
#[test]
fn test_exec_minu() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0xFFFFFFFF); cpu.set_register(5, 1);
    write_insns(&mut bus, TEST_PC as u64, &[enc_minu(3, 4, 5)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 1, "MINU(0xFFFFFFFF,1) should be 1");
}

/// MAXU(0xFFFFFFFF, 1) = 0xFFFFFFFF (largest unsigned value wins).
#[test]
fn test_exec_maxu() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0xFFFFFFFF); cpu.set_register(5, 1);
    write_insns(&mut bus, TEST_PC as u64, &[enc_maxu(3, 4, 5)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0xFFFFFFFF, "MAXU(0xFFFFFFFF,1) should be 0xFFFFFFFF");
}

/// SEXT(0x80, sa=7): bit 7 set → sign-extend → 0xFFFFFF80.
/// sa=7 means sign bit at position 7; lower 7 bits (bits[6:0]=0) are preserved;
/// bits[31:7] are filled with 1.
#[test]
fn test_exec_sext_t0_negative() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0x80);
    write_insns(&mut bus, TEST_PC as u64, &[enc_sext(3, 4, 7)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0xFFFFFF80, "SEXT(0x80, sa=7) should be 0xFFFFFF80");
}

/// SEXT(0x7F, sa=7): bit 7 clear → 0x0000007F (no sign fill).
#[test]
fn test_exec_sext_t0_positive() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 0x7F);
    write_insns(&mut bus, TEST_PC as u64, &[enc_sext(3, 4, 7)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 0x0000007F, "SEXT(0x7F, sa=7) should be 0x0000007F");
}

/// CLAMPS(50, sa=7): 50 is in range [-128, 127] → result = 50.
#[test]
fn test_exec_clamps_t0_in_range() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 50);
    write_insns(&mut bus, TEST_PC as u64, &[enc_clamps(3, 4, 7)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 50, "CLAMPS(50, sa=7) should be 50");
}

/// CLAMPS(200, sa=7): 200 > 127 → saturate to 127.
#[test]
fn test_exec_clamps_t0_overflow_pos() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, 200);
    write_insns(&mut bus, TEST_PC as u64, &[enc_clamps(3, 4, 7)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(cpu.get_register(3), 127, "CLAMPS(200, sa=7) should saturate to 127");
}

/// CLAMPS(-200, sa=7): -200 < -128 → saturate to -128 (= 0xFFFFFF80 as u32).
#[test]
fn test_exec_clamps_t0_overflow_neg() {
    let mut cpu = XtensaLx7::new(); let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap(); cpu.set_pc(TEST_PC);
    cpu.set_register(4, (-200i32) as u32);
    write_insns(&mut bus, TEST_PC as u64, &[enc_clamps(3, 4, 7)]);
    step_ok(&mut cpu, &mut bus);
    assert_eq!(
        cpu.get_register(3),
        (-128i32) as u32,
        "CLAMPS(-200, sa=7) should saturate to -128"
    );
}

// ── E4: Atomic instructions (S32C1I / L32AI / S32RI) ─────────────────────────
//
// HW-oracle (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
//   s32c1i a3, a4, 0  →  word 0x00e432  (r=0xE, t=3, s=4, imm8=0)
//   l32ai  a3, a4, 0  →  word 0x00b432  (r=0xB, t=3, s=4, imm8=0)
//   s32ri  a3, a4, 0  →  word 0x00f432  (r=0xF, t=3, s=4, imm8=0)
//
// LSAI format (op0=0x2): bits[3:0]=0x2, t=bits[7:4], s=bits[11:8],
//   r=bits[15:12], imm8=bits[23:16]; decoded imm = imm8 << 2.
//
// SCOMPARE1 is SR ID 12 (verified against xtensa_sr.rs constant and LX7 oracle).
// Tests read SCOMPARE1 via cpu.sr.read(SCOMPARE1_ID) to exercise the SR dispatcher.

/// Encode S32C1I at, as_, imm (op0=0x2, r=0xE, HW-oracle verified).
/// Pass the final byte offset (multiple of 4, 0..=1020); function right-shifts by 2.
fn enc_s32c1i(at: u32, as_: u32, byte_off: u32) -> u32 {
    let imm8 = (byte_off >> 2) & 0xFF;
    0x2 | (at << 4) | (as_ << 8) | (0xE << 12) | (imm8 << 16)
}

/// Encode L32AI at, as_, imm (op0=0x2, r=0xB, HW-oracle verified).
fn enc_l32ai(at: u32, as_: u32, byte_off: u32) -> u32 {
    let imm8 = (byte_off >> 2) & 0xFF;
    0x2 | (at << 4) | (as_ << 8) | (0xB << 12) | (imm8 << 16)
}

/// Encode S32RI at, as_, imm (op0=0x2, r=0xF, HW-oracle verified).
fn enc_s32ri(at: u32, as_: u32, byte_off: u32) -> u32 {
    let imm8 = (byte_off >> 2) & 0xFF;
    0x2 | (at << 4) | (as_ << 8) | (0xF << 12) | (imm8 << 16)
}

// Data address inside the RAM region (0x2000_0000..0x2010_0000).
// Keep it well away from TEST_PC (0x2000_0000) to avoid stomping instructions.
const DATA_ADDR: u32 = 0x2008_0000;

/// S32C1I uncontended success: SCOMPARE1 == mem → swap succeeds.
///
/// Setup: SCOMPARE1 = 0xCAFEBABE, mem[DATA_ADDR] = 0xCAFEBABE, a3 = 0xDEADBEEF.
/// After S32C1I a3, a4, 0 (with a4 = DATA_ADDR):
///   mem[DATA_ADDR] == 0xDEADBEEF  (new value written)
///   a3 == 0xCAFEBABE              (old value returned)
#[test]
fn test_exec_s32c1i_uncontended_success() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Prime memory with the old value.
    bus.write_u32(DATA_ADDR as u64, 0xCAFEBABE).unwrap();

    // Set SCOMPARE1 via SR dispatcher (SR ID 12).
    cpu.sr.write(SCOMPARE1_ID, 0xCAFEBABE);
    // Verify the write went through the dispatcher before the test.
    assert_eq!(cpu.sr.read(SCOMPARE1_ID), 0xCAFEBABE, "SCOMPARE1 must be set before test");

    // a3 = new value to store; a4 = base address.
    cpu.set_register(3, 0xDEADBEEF);
    cpu.set_register(4, DATA_ADDR);

    // S32C1I a3, a4, 0  →  HW-oracle word 0x00e432.
    write_insns(&mut bus, TEST_PC as u64, &[enc_s32c1i(3, 4, 0)]);
    step_ok(&mut cpu, &mut bus);

    // mem must now hold the new value (compare succeeded → write happened).
    assert_eq!(
        bus.read_u32(DATA_ADDR as u64).unwrap(),
        0xDEADBEEF,
        "CAS success: mem should hold new value 0xDEADBEEF"
    );
    // a3 must hold the old value (always returned by S32C1I).
    assert_eq!(
        cpu.get_register(3),
        0xCAFEBABE,
        "CAS success: a3 should hold old value 0xCAFEBABE"
    );
}

/// S32C1I uncontended failure: SCOMPARE1 != mem → no write, old value returned.
///
/// Setup: SCOMPARE1 = 0xCAFEBABE, mem[DATA_ADDR] = 0x12345678, a3 = 0xDEADBEEF.
/// After S32C1I:
///   mem[DATA_ADDR] == 0x12345678  (unchanged — compare failed)
///   a3 == 0x12345678              (old mem value returned)
#[test]
fn test_exec_s32c1i_uncontended_failure() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    // Prime memory with a value that does NOT match SCOMPARE1.
    bus.write_u32(DATA_ADDR as u64, 0x12345678).unwrap();

    // SCOMPARE1 differs from mem value → CAS will fail.
    cpu.sr.write(SCOMPARE1_ID, 0xCAFEBABE);

    cpu.set_register(3, 0xDEADBEEF);
    cpu.set_register(4, DATA_ADDR);

    write_insns(&mut bus, TEST_PC as u64, &[enc_s32c1i(3, 4, 0)]);
    step_ok(&mut cpu, &mut bus);

    // mem must remain unchanged (compare failed → no write).
    assert_eq!(
        bus.read_u32(DATA_ADDR as u64).unwrap(),
        0x12345678,
        "CAS failure: mem should remain unchanged 0x12345678"
    );
    // a3 must hold the old mem value (always returned).
    assert_eq!(
        cpu.get_register(3),
        0x12345678,
        "CAS failure: a3 should hold old mem value 0x12345678"
    );
}

/// S32C1I reads SCOMPARE1 through the SR dispatcher.
///
/// This test verifies that changing SCOMPARE1 via cpu.sr.write() changes the
/// CAS outcome — proving the exec arm uses the SR dispatcher, not a side-channel.
/// Write 0xABCD1234 to mem and to SCOMPARE1, then S32C1I with a different at value.
/// Outcome: CAS succeeds (mem == SCOMPARE1), mem updated, at = old.
#[test]
fn test_exec_s32c1i_uses_scompare1_via_sr_dispatcher() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    let sentinel = 0xABCD_1234u32;
    bus.write_u32(DATA_ADDR as u64, sentinel).unwrap();
    // Write SCOMPARE1 via the SR dispatcher; read it back to confirm.
    cpu.sr.write(SCOMPARE1_ID, sentinel);
    assert_eq!(
        cpu.sr.read(SCOMPARE1_ID),
        sentinel,
        "SR dispatcher write/read round-trip must preserve SCOMPARE1 value"
    );

    cpu.set_register(3, 0x0000_CAFE); // new value to store if CAS succeeds
    cpu.set_register(4, DATA_ADDR);

    write_insns(&mut bus, TEST_PC as u64, &[enc_s32c1i(3, 4, 0)]);
    step_ok(&mut cpu, &mut bus);

    // CAS must have succeeded: mem = new value, at = old.
    assert_eq!(bus.read_u32(DATA_ADDR as u64).unwrap(), 0x0000_CAFE,
        "SR-dispatcher test: CAS should succeed, mem = new value");
    assert_eq!(cpu.get_register(3), sentinel,
        "SR-dispatcher test: a3 should hold old value (sentinel)");
}

/// L32AI basic: load word from memory with acquire semantics (no-op barrier in Plan 1).
///
/// Stores 0xDEAD_C0DE into DATA_ADDR, runs L32AI a3, a4, 0, expects a3 = 0xDEAD_C0DE.
#[test]
fn test_exec_l32ai_basic() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    bus.write_u32(DATA_ADDR as u64, 0xDEAD_C0DE).unwrap();
    cpu.set_register(4, DATA_ADDR);

    // L32AI a3, a4, 0  →  HW-oracle word 0x00b432.
    write_insns(&mut bus, TEST_PC as u64, &[enc_l32ai(3, 4, 0)]);
    step_ok(&mut cpu, &mut bus);

    assert_eq!(cpu.get_register(3), 0xDEAD_C0DE, "L32AI should load 0xDEAD_C0DE into a3");
}

/// L32AI with nonzero imm: load from DATA_ADDR + 4 using imm=4 field.
#[test]
fn test_exec_l32ai_imm_offset() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    bus.write_u32((DATA_ADDR + 4) as u64, 0x1234_5678).unwrap();
    cpu.set_register(4, DATA_ADDR);

    // L32AI a3, a4, 4 (imm=4, imm8=1) →  HW-oracle word 0x01b432.
    write_insns(&mut bus, TEST_PC as u64, &[enc_l32ai(3, 4, 4)]);
    step_ok(&mut cpu, &mut bus);

    assert_eq!(cpu.get_register(3), 0x1234_5678, "L32AI imm=4 should load from DATA_ADDR+4");
}

/// S32RI basic: store word to memory with release semantics (no-op barrier in Plan 1).
///
/// Sets a3 = 0xC0FFEE00, runs S32RI a3, a4, 0, verifies mem[DATA_ADDR] = 0xC0FFEE00.
#[test]
fn test_exec_s32ri_basic() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    cpu.set_register(3, 0xC0FFEE00);
    cpu.set_register(4, DATA_ADDR);

    // S32RI a3, a4, 0  →  HW-oracle word 0x00f432.
    write_insns(&mut bus, TEST_PC as u64, &[enc_s32ri(3, 4, 0)]);
    step_ok(&mut cpu, &mut bus);

    assert_eq!(
        bus.read_u32(DATA_ADDR as u64).unwrap(),
        0xC0FFEE00,
        "S32RI should store 0xC0FFEE00 to DATA_ADDR"
    );
}

/// S32RI with nonzero imm: store to DATA_ADDR + 8 using imm=8 field.
#[test]
fn test_exec_s32ri_imm_offset() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    cpu.set_register(3, 0xBEEF_CAFE);
    cpu.set_register(4, DATA_ADDR);

    write_insns(&mut bus, TEST_PC as u64, &[enc_s32ri(3, 4, 8)]);
    step_ok(&mut cpu, &mut bus);

    assert_eq!(
        bus.read_u32((DATA_ADDR + 8) as u64).unwrap(),
        0xBEEF_CAFE,
        "S32RI imm=8 should store to DATA_ADDR+8"
    );
}

// ── F3: Window Overflow exception on ENTRY ───────────────────────────────────
//
// Vector offsets (Xtensa LX ISA RM §5.6; confirmed by Zephyr
// arch/xtensa/core/window_vectors.S .org directives):
//   WindowOverflow4:   VECBASE + 0x000
//   WindowUnderflow4:  VECBASE + 0x040
//   WindowOverflow8:   VECBASE + 0x080
//   WindowUnderflow8:  VECBASE + 0x0C0
//   WindowOverflow12:  VECBASE + 0x100
//   WindowUnderflow12: VECBASE + 0x140
//
// EXCCAUSE: window overflow does NOT set EXCCAUSE. Window exceptions vector
// independently via dedicated slots (not the general exception path).
// EXCCAUSE 5/6/7 = AllocaCause / IntegerDivideByZero / PrivilegedCause.
//
// Trigger condition: WindowStart[(wb_new + 1) mod 16] == 1
//   where wb_new = (wb_old + callinc) & 0xF.

const OF4_VECOFS:  u32 = 0x000;
const OF8_VECOFS:  u32 = 0x080;
const OF12_VECOFS: u32 = 0x100;

/// Helper: set up CPU state so that ENTRY with the given CALLINC will trigger
/// window overflow. WindowBase=0, CALLINC=callinc, wb_new = callinc & 0xF.
/// We mark WindowStart[(wb_new + 1) mod 16] = 1 to trigger overflow.
fn setup_entry_overflow(cpu: &mut XtensaLx7, callinc: u8) {
    cpu.regs.set_windowbase(0);
    cpu.ps.set_callinc(callinc);
    // wb_new = callinc (since wb_old=0)
    let wb_new = callinc & 0x0F;
    let check_idx = (wb_new + 1) & 0x0F;
    // Mark the check frame as in-use to trigger overflow
    cpu.regs.set_windowstart_bit(check_idx, true);
    // Also keep bit 0 set (reset state: caller frame live)
    cpu.regs.set_windowstart_bit(0, true);
}

/// F3: ENTRY with CALLINC=1 triggers WindowOverflow4.
/// Expect: PC = VECBASE + OF4 offset (0x000), EPC1 = original PC, PS.EXCM=1,
/// WindowBase NOT rotated (still 0), WindowStart check bit still set.
#[test]
fn test_exec_entry_window_overflow_of4() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    setup_entry_overflow(&mut cpu, 1);
    let original_pc = TEST_PC;
    let vecbase = cpu.sr.read(VECBASE_ID);

    write_insns(&mut bus, TEST_PC as u64, &[enc_entry(1, 4)]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(
        cpu.pc,
        vecbase.wrapping_add(OF4_VECOFS),
        "OF4: PC should jump to VECBASE + 0x000"
    );
    assert_eq!(
        cpu.sr.read(EPC1_ID),
        original_pc,
        "OF4: EPC1 should hold the faulting ENTRY PC"
    );
    assert!(cpu.ps.excm(), "OF4: PS.EXCM should be set");
    assert_eq!(cpu.regs.windowbase(), 0, "OF4: WindowBase must NOT be rotated");
    // check_idx = (1 + 1) & 0xF = 2
    assert!(
        cpu.regs.windowstart_bit(2),
        "OF4: WindowStart[check_idx] must remain set (overflow did not consume it)"
    );
}

/// F3: ENTRY with CALLINC=2 triggers WindowOverflow8.
/// Expect: PC = VECBASE + OF8 offset (0x080), EPC1 = original PC, PS.EXCM=1,
/// WindowBase NOT rotated (still 0).
#[test]
fn test_exec_entry_window_overflow_of8() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    setup_entry_overflow(&mut cpu, 2);
    let original_pc = TEST_PC;
    let vecbase = cpu.sr.read(VECBASE_ID);

    write_insns(&mut bus, TEST_PC as u64, &[enc_entry(1, 4)]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(
        cpu.pc,
        vecbase.wrapping_add(OF8_VECOFS),
        "OF8: PC should jump to VECBASE + 0x080"
    );
    assert_eq!(
        cpu.sr.read(EPC1_ID),
        original_pc,
        "OF8: EPC1 should hold the faulting ENTRY PC"
    );
    assert!(cpu.ps.excm(), "OF8: PS.EXCM should be set");
    assert_eq!(cpu.regs.windowbase(), 0, "OF8: WindowBase must NOT be rotated");
}

/// F3: ENTRY with CALLINC=3 triggers WindowOverflow12.
/// Expect: PC = VECBASE + OF12 offset (0x100), EPC1 = original PC, PS.EXCM=1,
/// WindowBase NOT rotated (still 0).
#[test]
fn test_exec_entry_window_overflow_of12() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    setup_entry_overflow(&mut cpu, 3);
    let original_pc = TEST_PC;
    let vecbase = cpu.sr.read(VECBASE_ID);

    write_insns(&mut bus, TEST_PC as u64, &[enc_entry(1, 4)]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(
        cpu.pc,
        vecbase.wrapping_add(OF12_VECOFS),
        "OF12: PC should jump to VECBASE + 0x100"
    );
    assert_eq!(
        cpu.sr.read(EPC1_ID),
        original_pc,
        "OF12: EPC1 should hold the faulting ENTRY PC"
    );
    assert!(cpu.ps.excm(), "OF12: PS.EXCM should be set");
    assert_eq!(cpu.regs.windowbase(), 0, "OF12: WindowBase must NOT be rotated");
}

/// F3 happy path: ENTRY proceeds normally when the target frame is clear.
/// With CALLINC=1, WB=0, wb_new=1, check_idx=2: if WindowStart[2]=0, no overflow.
/// Verify normal ENTRY semantics: WB rotates, WindowStart set, CALLINC cleared.
#[test]
fn test_exec_entry_no_overflow_when_target_frame_clear() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.ps.set_callinc(1);
    // Ensure check_idx=2 is clear (it should be after reset, but be explicit)
    cpu.regs.set_windowstart_bit(2, false);
    // Pre-load new frame's a1 (phys[5]) for ENTRY stack subtract
    cpu.regs.set_physical(5, 0x2005_0000);

    write_insns(&mut bus, TEST_PC as u64, &[enc_entry(1, 4)]);
    cpu.step(&mut bus, &[]).unwrap();

    // Normal ENTRY: WB rotated, WindowStart[1] set, CALLINC=0
    assert_eq!(cpu.regs.windowbase(), 1, "No-OF: WindowBase should rotate to 1");
    assert!(cpu.regs.windowstart_bit(1), "No-OF: WindowStart[1] should be set");
    assert_eq!(cpu.ps.callinc(), 0, "No-OF: CALLINC should be cleared");
    assert_eq!(
        cpu.pc,
        TEST_PC + 3,
        "No-OF: PC should advance past ENTRY"
    );
    // EPC1 must NOT have been set (no exception)
    assert_eq!(
        cpu.sr.read(EPC1_ID),
        0,
        "No-OF: EPC1 should remain 0 (no exception)"
    );
}

// ── F4: RETW window underflow tests ─────────────────────────────────────────
//
// ISA RM §5.5: When RETW executes, the destination frame index is
// wb_dest = (wb_cur - N) & 0x0F. If WindowStart[wb_dest] == 0, the
// frame's physical registers have been spilled and must be reloaded via
// the underflow vector.
//
// Window vector table (Xtensa LX ISA RM §5.6):
//   WindowUnderflow4:  VECBASE + 0x040   (N=1)
//   WindowUnderflow8:  VECBASE + 0x0C0   (N=2)
//   WindowUnderflow12: VECBASE + 0x140   (N=3)
//
// On underflow: EPC1=PC, PS.EXCM=1, PC=vector — WB and WS NOT modified.
// EXCCAUSE is NOT set (window vectors bypass the general exception path).

const UF4_VECOFS:  u32 = 0x040;
const UF8_VECOFS:  u32 = 0x0C0;
const UF12_VECOFS: u32 = 0x140;

/// Helper: set up a RETW frame where the destination WindowBase bit is clear,
/// triggering underflow. WB=callinc (callee frame), WS[callinc]=1 (callee live),
/// WS[0]=0 (destination frame NOT live → underflow), a0 encodes N=callinc.
fn setup_retw_underflow(cpu: &mut XtensaLx7, callinc: u8) {
    let wb_cur = callinc & 0x0F;
    cpu.regs.set_windowbase(wb_cur);
    // Only mark the callee frame live; destination (WB=0) is clear.
    let mut ws: u16 = 0;
    ws |= 1 << wb_cur;
    cpu.regs.set_windowstart(ws);
    // a0 for the callee frame: phys[wb_cur * 4] = (N << 30) | fake_return_addr
    let fake_ret = 0x4000_0400u32; // something that looks like a valid PC
    let a0_val = ((callinc as u32) << 30) | (fake_ret & 0x3FFF_FFFF);
    cpu.regs.set_physical((wb_cur as usize) * 4, a0_val);
}

/// F4: RETW with N=1 triggers WindowUnderflow4 when destination frame is absent.
/// Expect: PC = VECBASE + 0x040, EPC1 = RETW's PC, PS.EXCM=1,
/// WindowBase NOT rotated (still 1), WindowStart NOT modified.
#[test]
fn test_exec_retw_window_underflow_uf4() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    setup_retw_underflow(&mut cpu, 1);
    let original_pc = TEST_PC;
    let vecbase = cpu.sr.read(VECBASE_ID);
    let ws_before = cpu.regs.windowstart();

    write_insns(&mut bus, TEST_PC as u64, &[enc_retw()]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(
        cpu.pc,
        vecbase.wrapping_add(UF4_VECOFS),
        "UF4: PC should jump to VECBASE + 0x040"
    );
    assert_eq!(
        cpu.sr.read(EPC1_ID),
        original_pc,
        "UF4: EPC1 should hold the faulting RETW PC"
    );
    assert!(cpu.ps.excm(), "UF4: PS.EXCM should be set");
    assert_eq!(cpu.regs.windowbase(), 1, "UF4: WindowBase must NOT be rotated");
    assert_eq!(
        cpu.regs.windowstart(),
        ws_before,
        "UF4: WindowStart must NOT be modified"
    );
}

/// F4: RETW with N=2 triggers WindowUnderflow8 when destination frame is absent.
/// Expect: PC = VECBASE + 0x0C0, EPC1 = RETW's PC, PS.EXCM=1,
/// WindowBase NOT rotated (still 2), WindowStart NOT modified.
#[test]
fn test_exec_retw_window_underflow_uf8() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    setup_retw_underflow(&mut cpu, 2);
    let original_pc = TEST_PC;
    let vecbase = cpu.sr.read(VECBASE_ID);
    let ws_before = cpu.regs.windowstart();

    write_insns(&mut bus, TEST_PC as u64, &[enc_retw()]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(
        cpu.pc,
        vecbase.wrapping_add(UF8_VECOFS),
        "UF8: PC should jump to VECBASE + 0x0C0"
    );
    assert_eq!(
        cpu.sr.read(EPC1_ID),
        original_pc,
        "UF8: EPC1 should hold the faulting RETW PC"
    );
    assert!(cpu.ps.excm(), "UF8: PS.EXCM should be set");
    assert_eq!(cpu.regs.windowbase(), 2, "UF8: WindowBase must NOT be rotated");
    assert_eq!(
        cpu.regs.windowstart(),
        ws_before,
        "UF8: WindowStart must NOT be modified"
    );
}

/// F4: RETW with N=3 triggers WindowUnderflow12 when destination frame is absent.
/// Expect: PC = VECBASE + 0x140, EPC1 = RETW's PC, PS.EXCM=1,
/// WindowBase NOT rotated (still 3), WindowStart NOT modified.
#[test]
fn test_exec_retw_window_underflow_uf12() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    setup_retw_underflow(&mut cpu, 3);
    let original_pc = TEST_PC;
    let vecbase = cpu.sr.read(VECBASE_ID);
    let ws_before = cpu.regs.windowstart();

    write_insns(&mut bus, TEST_PC as u64, &[enc_retw()]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(
        cpu.pc,
        vecbase.wrapping_add(UF12_VECOFS),
        "UF12: PC should jump to VECBASE + 0x140"
    );
    assert_eq!(
        cpu.sr.read(EPC1_ID),
        original_pc,
        "UF12: EPC1 should hold the faulting RETW PC"
    );
    assert!(cpu.ps.excm(), "UF12: PS.EXCM should be set");
    assert_eq!(cpu.regs.windowbase(), 3, "UF12: WindowBase must NOT be rotated");
    assert_eq!(
        cpu.regs.windowstart(),
        ws_before,
        "UF12: WindowStart must NOT be modified"
    );
}

/// F4 happy path: RETW proceeds normally when destination frame IS live.
/// Regression test: no spurious EPC1/PC-vector side effects when WS[wb_dest]=1.
/// Uses N=1, WB=1, WS[0]=1 (destination frame live) → no underflow.
#[test]
fn test_exec_retw_no_underflow_when_destination_frame_set() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    // Clear EXCM so we can detect if the UF path spuriously sets it.
    cpu.ps.set_excm(false);

    // WB=1, destination is WB=0. Mark both frames live.
    cpu.regs.set_windowbase(1);
    cpu.regs.set_windowstart(0b0000_0000_0000_0011); // WS[0]=1, WS[1]=1
    // a0 in callee frame (phys[4]): N=1, target = some address in same 256MB region
    let target = 0x2000_0200u32;
    let a0_val = (target & 0x3FFF_FFFF) | (1u32 << 30);
    cpu.regs.set_physical(4, a0_val);

    write_insns(&mut bus, TEST_PC as u64, &[enc_retw()]);
    cpu.step(&mut bus, &[]).unwrap();

    // Normal RETW: PC = target (low30 | cur_pc high2)
    let expected_pc = (target & 0x3FFF_FFFF) | (TEST_PC & 0xC000_0000);
    assert_eq!(cpu.pc, expected_pc, "No-UF: PC should be set to return address");
    // WB rotated back to 0
    assert_eq!(cpu.regs.windowbase(), 0, "No-UF: WindowBase should rotate to 0");
    // WS[1] cleared by normal RETW
    assert!(!cpu.regs.windowstart_bit(1), "No-UF: WindowStart[1] should be cleared");
    // EPC1 must NOT have been set (no exception)
    assert_eq!(
        cpu.sr.read(EPC1_ID),
        0,
        "No-UF: EPC1 should remain 0 (no exception)"
    );
    // PS.EXCM must NOT have been set by the UF path (was cleared before the test)
    assert!(!cpu.ps.excm(), "No-UF: PS.EXCM should remain clear (no UF exception)");
}

// ── F5: S32E / L32E exec tests ────────────────────────────────────────────────
//
// HW-oracle encoding (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
//   s32e a3, a4, -16 → 0x30C449  s32e a3, a4, -64 → 0x300449
//   l32e a3, a4, -16 → 0x30C409  l32e a3, a4, -64 → 0x300409
//
// Encoding (op0=9):
//   bits[23:20]=at, bits[15:12]=imm4, bits[11:8]=as_, bits[7:4]=subop(4=S32E/0=L32E)
//   imm_byte = imm4 * 4 - 64
//
// These instructions are gated on PS.EXCM=1.  When PS.EXCM=0 they raise
// IllegalInstruction (EXCCAUSE=0, ExceptionRaised{cause:0}).

/// Encode S32E at, as_, imm_byte  (op0=9, subop=4).
/// imm_byte must be in -64..-4 (multiples of 4).
fn enc_s32e(at: u32, as_: u32, imm_byte: i32) -> u32 {
    let imm4 = ((imm_byte + 64) / 4) as u32;
    (at << 20) | (imm4 << 12) | (as_ << 8) | (0x4 << 4) | 0x9
}

/// Encode L32E at, as_, imm_byte  (op0=9, subop=0).
fn enc_l32e(at: u32, as_: u32, imm_byte: i32) -> u32 {
    let imm4 = ((imm_byte + 64) / 4) as u32;
    (at << 20) | (imm4 << 12) | (as_ << 8) | 0x9
}

/// S32E inside exception context (PS.EXCM=1): should write to memory.
#[test]
fn test_exec_s32e_in_excm_writes() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    // Reset leaves PS.EXCM=1 (reset value 0x1F). Confirm and proceed.
    assert!(cpu.ps.excm(), "pre-condition: reset should leave PS.EXCM=1");

    // a4 = base address; a3 = value to store.
    let base: u32 = 0x2000_0100;
    cpu.set_register(4, base);
    cpu.set_register(3, 0xDEAD_BEEF);

    // S32E a3, a4, -16: EA = base - 16.
    write_insns(&mut bus, TEST_PC as u64, &[enc_s32e(3, 4, -16)]);
    cpu.step(&mut bus, &[]).unwrap();

    let stored = bus.read_u32((base - 16) as u64).unwrap();
    assert_eq!(stored, 0xDEAD_BEEF, "S32E should write at to EA=as_+imm");
    assert_eq!(cpu.pc, TEST_PC + 3, "PC should advance by 3 after S32E");
}

// S32E decoder EXCM gate: outside exception context (PS.EXCM=0), the S32E byte
// sequence is NOT decoded as a wide instruction. Instead, it's decoded as a
// narrow S32I.N (which has the same op0=0x9), so no exception is raised by the
// executor. This test has been removed because it tested OLD behavior (executor-level
// exception raising) that is no longer correct with the decoder-level EXCM gate.
// The executor's defensive EXCM check remains (lines 934-935) as defense-in-depth.

/// L32E inside exception context (PS.EXCM=1): should read from memory into at.
#[test]
fn test_exec_l32e_in_excm_reads() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    assert!(cpu.ps.excm(), "pre-condition: reset should leave PS.EXCM=1");

    let base: u32 = 0x2000_0100;
    let ea: u64 = (base - 16) as u64;
    bus.write_u32(ea, 0xCAFE_BABE).unwrap();

    cpu.set_register(4, base);

    // L32E a3, a4, -16: at=a3, as_=a4, EA = base - 16.
    write_insns(&mut bus, TEST_PC as u64, &[enc_l32e(3, 4, -16)]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.get_register(3), 0xCAFE_BABE, "L32E should load from EA into at");
    assert_eq!(cpu.pc, TEST_PC + 3, "PC should advance by 3 after L32E");
}

// L32E decoder EXCM gate: outside exception context (PS.EXCM=0), the L32E byte
// sequence is NOT decoded as a wide instruction. Instead, it's decoded as a
// narrow S32I.N (which has the same op0=0x9), so no exception is raised by the
// executor. This test has been removed because it tested OLD behavior (executor-level
// exception raising) that is no longer correct with the decoder-level EXCM gate.
// The executor's defensive EXCM check remains (lines 944-945) as defense-in-depth.

/// S32E with maximum negative offset (-64): EA = as_ - 64.
#[test]
fn test_exec_s32e_negative_offset() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    assert!(cpu.ps.excm(), "pre-condition: PS.EXCM=1");

    let base: u32 = 0x2000_0100;
    cpu.set_register(5, base);
    cpu.set_register(6, 0xABCD_1234);

    // S32E a6, a5, -64: EA = base - 64 = 0x2000_00C0.
    write_insns(&mut bus, TEST_PC as u64, &[enc_s32e(6, 5, -64)]);
    cpu.step(&mut bus, &[]).unwrap();

    let stored = bus.read_u32((base - 64) as u64).unwrap();
    assert_eq!(stored, 0xABCD_1234, "S32E with imm=-64 should write to as_-64");
}

/// HW-oracle byte verification: s32e a3, a4, -16 → 0x30C449.
#[test]
fn test_exec_s32e_hw_oracle_bytes() {
    // Use the exact HW-oracle word; verify it decodes and executes correctly.
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    assert!(cpu.ps.excm());

    let base: u32 = 0x2000_0080;
    cpu.set_register(4, base);
    cpu.set_register(3, 0x1111_2222);

    // Write HW-oracle bytes directly: 0x30C449 in LE = 0x49, 0xC4, 0x30.
    bus.write_u8(TEST_PC as u64,     0x49).unwrap();
    bus.write_u8(TEST_PC as u64 + 1, 0xC4).unwrap();
    bus.write_u8(TEST_PC as u64 + 2, 0x30).unwrap();
    cpu.step(&mut bus, &[]).unwrap();

    let stored = bus.read_u32((base - 16) as u64).unwrap();
    assert_eq!(stored, 0x1111_2222, "HW-oracle s32e a3,a4,-16 should store a3 at a4-16");
}

/// HW-oracle byte verification: l32e a3, a4, -16 → 0x30C409.
#[test]
fn test_exec_l32e_hw_oracle_bytes() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    assert!(cpu.ps.excm());

    let base: u32 = 0x2000_0080;
    bus.write_u32((base - 16) as u64, 0xFEED_FACE).unwrap();
    cpu.set_register(4, base);

    // Write HW-oracle bytes: 0x30C409 in LE = 0x09, 0xC4, 0x30.
    bus.write_u8(TEST_PC as u64,     0x09).unwrap();
    bus.write_u8(TEST_PC as u64 + 1, 0xC4).unwrap();
    bus.write_u8(TEST_PC as u64 + 2, 0x30).unwrap();
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.get_register(3), 0xFEED_FACE, "HW-oracle l32e a3,a4,-16 should load into a3");
}

/// Regression test: s32i.n + QRST must NOT be mis-decoded as L32E outside EXCM.
///
/// Bug: In normal (PS.EXCM=0) code, `s32i.n a0, a1, 0` followed by any QRST
/// instruction (byte0 ending in 0x0 — extremely common: ADD, OR, MOVI, NOP, etc.)
/// would cause the decoder to speculatively read byte2, find low nibble 0, and
/// incorrectly decode the 2-byte narrow instruction as a 3-byte L32E, advancing
/// PC by 3 instead of 2 and corrupting the instruction stream.
///
/// Fix: Gate the S32E/L32E disambiguation on PS.EXCM. These instructions are
/// only valid in exception context, so the decoder should only look for them
/// when EXCM=1.
///
/// Scenario:
///   s32i.n a0, a1, 0 = HW-oracle bytes [09, 01] (little-endian)
///   nop.n = HW-oracle bytes [3d, f0] (little-endian)
///
/// At TEST_PC, without the EXCM gate, byte0=0x09 would read byte2=0x3d (low
/// nibble 0xD), spuriously matching the L32E pattern and advancing PC by 3.
#[test]
fn test_step_does_not_misdecode_s32i_n_as_l32e_outside_excm() {
    let (mut cpu, mut bus) = make_cpu_bus();

    // Precondition: PS.EXCM must be 0 in normal code. Reset leaves EXCM=1, so clear it.
    assert!(cpu.ps.excm(), "reset leaves PS.EXCM=1, we will clear it below");
    cpu.ps.set_excm(false);
    assert!(!cpu.ps.excm(), "PS.EXCM should now be 0");

    // Set up a1 to point to somewhere safe in RAM for the store.
    let safe_addr: u32 = 0x2000_0080;
    cpu.set_register(1, safe_addr);

    // Write s32i.n a0, a1, 0 (2 bytes) + nop.n (2 bytes) at TEST_PC.
    // Using a raw byte write since write_insns is designed for 3-byte wide instructions.
    bus.write_u8(TEST_PC as u64,     0x09).unwrap(); // s32i.n byte 0
    bus.write_u8(TEST_PC as u64 + 1, 0x01).unwrap(); // s32i.n byte 1
    bus.write_u8(TEST_PC as u64 + 2, 0x3d).unwrap(); // nop.n byte 0
    bus.write_u8(TEST_PC as u64 + 3, 0xf0).unwrap(); // nop.n byte 1

    // Step 1: execute s32i.n a0, a1, 0 (narrow, 2 bytes).
    // Without the EXCM gate, this would read byte2=0x3d, match the L32E pattern
    // (low nibble 0xD → 0), and incorrectly advance PC by 3.
    cpu.step(&mut bus, &[]).expect("first step (s32i.n) should not error");

    // Verify: PC should advance by exactly 2 for the narrow store, NOT 3.
    assert_eq!(
        cpu.get_pc(),
        TEST_PC + 2,
        "PC must advance by 2 for narrow s32i.n, not 3"
    );

    // Step 2: execute nop.n (2 bytes).
    cpu.step(&mut bus, &[])
        .expect("second step (nop.n) should succeed");

    // Verify: PC should now be at TEST_PC + 2 + 2 = TEST_PC + 4.
    assert_eq!(
        cpu.get_pc(),
        TEST_PC + 4,
        "PC must advance to TEST_PC+4 after nop.n"
    );
}

// ── F6 Tests: MOVSP + ROTW ────────────────────────────────────────────────────
//
// HW-oracle encodings (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
//   movsp a3, a4 → 0x001430  (ST0 group: op0=0, op1=0, op2=0, r=1, s=as_=4, t=at=3)
//   rotw  1      → 0x408010  (op0=0, op1=0, op2=4, r=8, s=0, t=1)
//   rotw -1      → 0x4080f0  (t=0xF → 4-bit two's complement -1)
//   rotw  7      → 0x408070  (t=7, max positive)
//   rotw -8      → 0x408080  (t=8 → 4-bit two's complement -8)

/// Encode MOVSP at, as_: ST0 group, r=1, s=as_, t=at.
/// Layout: op0=0, op1=0, op2=0 → (r<<12)|(s<<8)|(t<<4).
fn enc_movsp(at: u32, as_: u32) -> u32 {
    st0(1, as_, at)
}

/// Encode ROTW n: op0=0, op1=0, op2=4, r=8, s=0, t=n_raw (4-bit two's complement of n).
fn enc_rotw(n: i32) -> u32 {
    let t = (n as u32) & 0xF; // mask to 4 bits (two's complement)
    rrr(0x4, 0x0, 0x8, 0, t)
}

/// MOVSP safe path: when WS bit (WB+1) is CLEAR, perform plain register move.
///
/// Setup: WB=0, WS=0x1 (only bit 0 set → bit 1 is clear → safe path).
/// Execute `movsp a3, a4`: a[at=3] = a[as_=4]; PC advances by 3.
#[test]
fn test_exec_movsp_safe_path() {
    let (mut cpu, mut bus) = make_cpu_bus();

    // Precondition: WindowBase=0, WindowStart bit 1 (WB+1=1) must be CLEAR.
    // After reset: WB=0, WS=0x1 (bit 0 set, bit 1 clear) → safe path.
    assert_eq!(cpu.regs.windowbase(), 0, "pre: WB=0");
    assert!(!cpu.regs.windowstart_bit(1), "pre: WS[1] must be clear for safe path");

    // Set source register a4 to a known value.
    cpu.set_register(4, 0xDEAD_BEEF);
    cpu.set_register(3, 0x0); // destination starts at 0

    // Write HW-oracle bytes for `movsp a3, a4` → 0x001430 in LE: 0x30, 0x14, 0x00.
    write_insns(&mut bus, TEST_PC as u64, &[enc_movsp(3, 4)]);
    cpu.step(&mut bus, &[]).expect("MOVSP safe path must not error");

    // a[3] = a[4] = 0xDEAD_BEEF
    assert_eq!(cpu.get_register(3), 0xDEAD_BEEF, "MOVSP: a3 must equal a4 after safe-path move");
    // a[4] (source) unchanged
    assert_eq!(cpu.get_register(4), 0xDEAD_BEEF, "MOVSP: a4 (source) must be unchanged");
    // PC advanced by 3
    assert_eq!(cpu.get_pc(), TEST_PC + 3, "MOVSP: PC must advance by 3");
    // WindowBase and WindowStart unchanged
    assert_eq!(cpu.regs.windowbase(), 0, "MOVSP: WindowBase must not change");
    assert_eq!(cpu.regs.windowstart(), 0x1, "MOVSP: WindowStart must not change");
}

/// MOVSP with adjacent frame live: WS bit (WB+1) SET → raises ExceptionRaised{cause:5}.
///
/// Plan 1 defers the spill path; AllocaCause (EXCCAUSE=5) is raised instead.
/// TODO(plan2): replace with full register-spill implementation.
#[test]
fn test_exec_movsp_overflow_raises_exception() {
    let (mut cpu, mut bus) = make_cpu_bus();

    // Set WB=0 and force WS bit 1 (= (WB+1) & 0xF) to be SET → live adjacent frame.
    cpu.regs.set_windowbase(0);
    cpu.regs.set_windowstart_bit(1, true);
    assert!(cpu.regs.windowstart_bit(1), "pre: WS[(WB+1)&0xF] must be set");

    cpu.set_register(4, 0x1234_5678);
    write_insns(&mut bus, TEST_PC as u64, &[enc_movsp(3, 4)]);

    let err = cpu.step(&mut bus, &[]).expect_err("MOVSP with live adjacent frame must raise exception");
    assert!(
        matches!(err, SimulationError::ExceptionRaised { cause: 5, .. }),
        "MOVSP overflow: expected ExceptionRaised{{cause:5}}, got {:?}", err
    );
}

/// ROTW +1: WindowBase = (old_WB + 1) & 0xF.
#[test]
fn test_exec_rotw_pos() {
    let (mut cpu, mut bus) = make_cpu_bus();

    cpu.regs.set_windowbase(3);
    let ws_before = cpu.regs.windowstart();

    // HW-oracle: `rotw 1` → 0x408010.
    write_insns(&mut bus, TEST_PC as u64, &[enc_rotw(1)]);
    cpu.step(&mut bus, &[]).expect("ROTW +1 must not error");

    assert_eq!(cpu.regs.windowbase(), 4, "ROTW +1: WB must be (3+1)&0xF = 4");
    assert_eq!(cpu.regs.windowstart(), ws_before, "ROTW +1: WindowStart must not change");
    assert_eq!(cpu.get_pc(), TEST_PC + 3, "ROTW +1: PC must advance by 3");
}

/// ROTW -1: WindowBase = (old_WB - 1) & 0xF (wrapping).
#[test]
fn test_exec_rotw_neg() {
    let (mut cpu, mut bus) = make_cpu_bus();

    cpu.regs.set_windowbase(5);
    let ws_before = cpu.regs.windowstart();

    // HW-oracle: `rotw -1` → 0x4080f0.
    write_insns(&mut bus, TEST_PC as u64, &[enc_rotw(-1)]);
    cpu.step(&mut bus, &[]).expect("ROTW -1 must not error");

    assert_eq!(cpu.regs.windowbase(), 4, "ROTW -1: WB must be (5-1)&0xF = 4");
    assert_eq!(cpu.regs.windowstart(), ws_before, "ROTW -1: WindowStart must not change");
    assert_eq!(cpu.get_pc(), TEST_PC + 3, "ROTW -1: PC must advance by 3");
}

/// ROTW -1 wraparound: WindowBase = (0 - 1) & 0xF = 15.
#[test]
fn test_exec_rotw_neg_wraparound() {
    let (mut cpu, mut bus) = make_cpu_bus();

    cpu.regs.set_windowbase(0);
    let ws_before = cpu.regs.windowstart();

    write_insns(&mut bus, TEST_PC as u64, &[enc_rotw(-1)]);
    cpu.step(&mut bus, &[]).expect("ROTW -1 wraparound must not error");

    assert_eq!(cpu.regs.windowbase(), 15, "ROTW -1 from WB=0 must wrap to 15");
    assert_eq!(cpu.regs.windowstart(), ws_before, "ROTW wraparound: WindowStart must not change");
}

/// ROTW +7 (max positive): WindowBase = (old_WB + 7) & 0xF.
#[test]
fn test_exec_rotw_max_pos() {
    let (mut cpu, mut bus) = make_cpu_bus();

    cpu.regs.set_windowbase(2);
    let ws_before = cpu.regs.windowstart();

    // HW-oracle: `rotw 7` → 0x408070.
    write_insns(&mut bus, TEST_PC as u64, &[enc_rotw(7)]);
    cpu.step(&mut bus, &[]).expect("ROTW +7 must not error");

    assert_eq!(cpu.regs.windowbase(), 9, "ROTW +7: WB must be (2+7)&0xF = 9");
    assert_eq!(cpu.regs.windowstart(), ws_before, "ROTW max-pos: WindowStart must not change");
}

/// ROTW -8 (max negative): WindowBase = (old_WB - 8) & 0xF (wrapping).
#[test]
fn test_exec_rotw_max_neg() {
    let (mut cpu, mut bus) = make_cpu_bus();

    cpu.regs.set_windowbase(5);
    let ws_before = cpu.regs.windowstart();

    // HW-oracle: `rotw -8` → 0x408080.
    write_insns(&mut bus, TEST_PC as u64, &[enc_rotw(-8)]);
    cpu.step(&mut bus, &[]).expect("ROTW -8 must not error");

    assert_eq!(cpu.regs.windowbase(), 13, "ROTW -8: WB must be (5-8+16)&0xF = 13");
    assert_eq!(cpu.regs.windowstart(), ws_before, "ROTW max-neg: WindowStart must not change");
}

/// ROTW does not modify WindowStart regardless of the rotation amount.
#[test]
fn test_exec_rotw_does_not_change_windowstart() {
    let (mut cpu, mut bus) = make_cpu_bus();

    // Set an unusual WindowStart pattern to verify it's untouched.
    let ws_pattern: u16 = 0b0000_0000_0101_0011; // bits 0,1,4,6 set
    cpu.regs.set_windowbase(0);
    cpu.regs.set_windowstart(ws_pattern);

    // ROTW +3: WB → 3, but WS must be unchanged.
    write_insns(&mut bus, TEST_PC as u64, &[enc_rotw(3)]);
    cpu.step(&mut bus, &[]).expect("ROTW +3 must not error");

    assert_eq!(
        cpu.regs.windowstart(),
        ws_pattern,
        "ROTW must leave WindowStart completely untouched"
    );
    assert_eq!(cpu.regs.windowbase(), 3, "ROTW +3: WB must be 3");
}

// ── G1 Tests: General exception entry dispatch ────────────────────────────────
//
// Kernel vector offset for ESP32-S3 LX7: VECBASE + 0x300.
// Source: Zephyr soc/xtensa/esp32s3/linker.ld `. = 0x300; KEEP(*(.KernelExceptionVector.text))`.
//
// EPS1 does NOT exist in the ESP32-S3 LX7 config (rejected by xtensa-esp32s3-elf-as).
// PS is read by the exception handler via `rsr.ps` after entry; no EPS1 save.
//
// On general exception entry:
//   EPC1    ← pre-advance PC (the faulting instruction's address)
//   EXCCAUSE ← cause
//   PS.EXCM ← 1
//   PC      ← VECBASE + 0x300

const KERNEL_VECTOR_OFFSET_TEST: u32 = 0x300;

/// G1: QUOS div-by-zero redirects PC to kernel vector and sets EPC1/EXCCAUSE/PS.EXCM.
#[test]
fn test_general_exception_div_by_zero_redirects_pc() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    // Clear EXCM so we can observe PS.EXCM being set by the exception.
    cpu.ps.set_excm(false);
    let old_ps_raw = cpu.ps.as_raw();

    cpu.set_register(4, 100);
    cpu.set_register(5, 0);  // divisor = 0
    write_insns(&mut bus, TEST_PC as u64, &[enc_quos(3, 4, 5)]);

    let err = cpu.step(&mut bus, &[]).unwrap_err();

    // Error kind is still ExceptionRaised so callers can react.
    assert!(
        matches!(err, SimulationError::ExceptionRaised { cause: 6, pc } if pc == TEST_PC),
        "div-by-zero: ExceptionRaised{{cause:6, pc=TEST_PC}} expected, got: {:?}", err
    );

    // EPC1 must hold the faulting PC (pre-advance).
    assert_eq!(
        cpu.sr.read(EPC1_ID), TEST_PC,
        "EPC1 must be set to the faulting instruction's PC"
    );
    // EXCCAUSE must be 6 (IntegerDivideByZero).
    assert_eq!(cpu.sr.read(EXCCAUSE_ID), 6, "EXCCAUSE must be 6");
    // PS.EXCM must be 1.
    assert!(cpu.ps.excm(), "PS.EXCM must be 1 after general exception entry");
    // PC must be at the kernel vector.
    let vecbase = cpu.sr.read(VECBASE_ID);
    assert_eq!(
        cpu.get_pc(), vecbase.wrapping_add(KERNEL_VECTOR_OFFSET_TEST),
        "PC must be redirected to VECBASE + 0x300 (_KernelExceptionVector)"
    );
    // Confirm the old PS was non-EXCM (validates the pre-condition captured earlier).
    assert_eq!(old_ps_raw & 0x10, 0, "pre: PS.EXCM should have been 0 before exception");
}

/// G1: REMU div-by-zero also redirects correctly (covers the REMU path).
#[test]
fn test_general_exception_remu_redirects_pc() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);
    cpu.ps.set_excm(false);

    cpu.set_register(4, 99);
    cpu.set_register(5, 0);
    write_insns(&mut bus, TEST_PC as u64, &[enc_remu(3, 4, 5)]);

    let err = cpu.step(&mut bus, &[]).unwrap_err();
    assert!(
        matches!(err, SimulationError::ExceptionRaised { cause: 6, pc } if pc == TEST_PC),
        "remu div-by-zero: ExceptionRaised{{cause:6}} expected, got {:?}", err
    );
    assert_eq!(cpu.sr.read(EPC1_ID), TEST_PC, "EPC1 must be set to faulting PC");
    assert_eq!(cpu.sr.read(EXCCAUSE_ID), 6, "EXCCAUSE must be 6");
    assert!(cpu.ps.excm(), "PS.EXCM must be 1");
    let vecbase = cpu.sr.read(VECBASE_ID);
    assert_eq!(cpu.get_pc(), vecbase.wrapping_add(KERNEL_VECTOR_OFFSET_TEST),
        "PC must redirect to VECBASE + 0x300");
}

/// G1: MOVSP with live adjacent frame redirects PC to kernel vector (AllocaCause=5).
#[test]
fn test_general_exception_movsp_alloca_redirects_pc() {
    let (mut cpu, mut bus) = make_cpu_bus();

    // Clear PS.EXCM so we can observe it being set.
    cpu.ps.set_excm(false);

    // Set WB=0 and mark WS bit 1 live → triggers AllocaCause.
    cpu.regs.set_windowbase(0);
    cpu.regs.set_windowstart_bit(1, true);

    cpu.set_register(4, 0x1234_5678);
    write_insns(&mut bus, TEST_PC as u64, &[enc_movsp(3, 4)]);

    let err = cpu.step(&mut bus, &[]).unwrap_err();
    assert!(
        matches!(err, SimulationError::ExceptionRaised { cause: 5, pc } if pc == TEST_PC),
        "MOVSP alloca: ExceptionRaised{{cause:5, pc=TEST_PC}} expected, got: {:?}", err
    );
    assert_eq!(cpu.sr.read(EPC1_ID), TEST_PC, "EPC1 must be set to faulting MOVSP PC");
    assert_eq!(cpu.sr.read(EXCCAUSE_ID), 5, "EXCCAUSE must be 5 (AllocaCause)");
    assert!(cpu.ps.excm(), "PS.EXCM must be 1 after MOVSP alloca exception");
    let vecbase = cpu.sr.read(VECBASE_ID);
    assert_eq!(cpu.get_pc(), vecbase.wrapping_add(KERNEL_VECTOR_OFFSET_TEST),
        "PC must redirect to VECBASE + 0x300 (_KernelExceptionVector)");
}

/// G1: Exception entry captures pre-advance PC (not PC+3).
///
/// The faulting PC stored in EPC1 and returned in ExceptionRaised must be the
/// address of the faulting instruction itself, not the next instruction.
#[test]
fn test_general_exception_does_not_advance_pc_into_epc1() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    // Place the faulting instruction at a non-trivial offset.
    let faulting_pc: u32 = 0x2000_0010;
    cpu.set_pc(faulting_pc);
    cpu.ps.set_excm(false);
    cpu.set_register(4, 50);
    cpu.set_register(5, 0);
    write_insns(&mut bus, faulting_pc as u64, &[enc_quos(3, 4, 5)]);

    let err = cpu.step(&mut bus, &[]).unwrap_err();
    // EPC1 must be the faulting instruction's address, NOT faulting_pc + 3.
    assert_eq!(
        cpu.sr.read(EPC1_ID), faulting_pc,
        "EPC1 must equal the faulting instruction PC, not PC+3"
    );
    assert!(
        matches!(err, SimulationError::ExceptionRaised { pc, .. } if pc == faulting_pc),
        "ExceptionRaised.pc must be the pre-advance faulting PC"
    );
    // PC is at the vector, NOT at faulting_pc + 3.
    assert_ne!(cpu.get_pc(), faulting_pc + 3, "PC must not be advanced to faulting_pc+3");
    let vecbase = cpu.sr.read(VECBASE_ID);
    assert_eq!(cpu.get_pc(), vecbase.wrapping_add(KERNEL_VECTOR_OFFSET_TEST),
        "PC must be at VECBASE + 0x300 after exception");
}

// ── G2 Tests: Exception/Interrupt Return (RFE / RFI / RFWO / RFWU) ───────────
//
// HW-oracle byte encodings (xtensa-esp32s3-elf-as + objdump esp-15.2.0_20250920):
//   rfe    → 0x003000   (bytes: 00 30 00)
//   rfi 2  → 0x003210   (bytes: 10 32 00)
//   rfi 3  → 0x003310   (bytes: 10 33 00)
//   rfwo   → 0x003400   (bytes: 00 34 00)
//   rfwu   → 0x003500   (bytes: 00 35 00)
//
// SR IDs verified from C2 LX7 table (xtensa_sr.rs constants):
//   EPC1 = 177, EPC2 = 178, EPC3 = 179
//   EPS2 = 194, EPS3 = 195

/// Encode RFE (op0=0, op1=0, op2=0, r=3, s=0, t=0).
/// HW-oracle: 0x003000.
fn enc_rfe() -> u32 {
    st0(3, 0, 0)
}

/// Encode RFI level (op0=0, op1=0, op2=0, r=3, s=level, t=1).
/// HW-oracle: rfi 2 → 0x003210, rfi 3 → 0x003310.
fn enc_rfi(level: u32) -> u32 {
    st0(3, level, 1)
}

/// Encode RFWO (op0=0, op1=0, op2=0, r=3, s=4, t=0).
/// HW-oracle: 0x003400.
fn enc_rfwo() -> u32 {
    st0(3, 4, 0)
}

/// Encode RFWU (op0=0, op1=0, op2=0, r=3, s=5, t=0).
/// HW-oracle: 0x003500.
fn enc_rfwu() -> u32 {
    st0(3, 5, 0)
}

/// G2: RFE clears PS.EXCM and jumps to EPC1.
///
/// Pre-condition: PS.EXCM=1, EPC1=target_pc.
/// Post-condition: PS.EXCM=0, PC=target_pc.
/// PS.INTLEVEL is left unchanged (not reset by RFE per LX7 ISA RM §4.4.2).
#[test]
fn test_exec_rfe_clears_excm_and_jumps() {
    let (mut cpu, mut bus) = make_cpu_bus();

    let target_pc: u32 = 0x2000_0100;
    // Set up: PS.EXCM=1, PS.INTLEVEL=3 (to verify INTLEVEL is left unchanged).
    cpu.ps.set_excm(true);
    cpu.ps.set_intlevel(3);
    cpu.sr.set_raw(EPC1_ID, target_pc);

    write_insns(&mut bus, TEST_PC as u64, &[enc_rfe()]);
    cpu.step(&mut bus, &[]).unwrap();

    assert!(!cpu.ps.excm(), "RFE: PS.EXCM must be 0 after RFE");
    assert_eq!(cpu.get_pc(), target_pc, "RFE: PC must be EPC1");
    // INTLEVEL must be left unchanged by RFE.
    assert_eq!(cpu.ps.intlevel(), 3, "RFE: PS.INTLEVEL must be left unchanged");
}

/// G2: RFE round-trip — set EPC1 to the "next instruction" address, run RFE,
/// verify we land exactly at EPC1.
#[test]
fn test_exec_rfe_round_trip_via_epc1() {
    let (mut cpu, mut bus) = make_cpu_bus();

    let return_addr: u32 = 0x2000_0200;
    cpu.ps.set_excm(true);
    cpu.sr.set_raw(EPC1_ID, return_addr);

    write_insns(&mut bus, TEST_PC as u64, &[enc_rfe()]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.get_pc(), return_addr, "RFE round-trip: PC must be exactly EPC1");
    assert!(!cpu.ps.excm(), "RFE round-trip: PS.EXCM must be 0");
}

/// G2: RFI level 2 — restores full PS from EPS2 and jumps to EPC2.
#[test]
fn test_exec_rfi_2() {
    let (mut cpu, mut bus) = make_cpu_bus();

    let saved_ps: u32 = 0x0000_0008;  // INTLEVEL=8, EXCM=0
    let saved_pc: u32 = 0x2000_0300;
    cpu.sr.set_raw(EPS2_ID, saved_ps);
    cpu.sr.set_raw(EPC2_ID, saved_pc);

    write_insns(&mut bus, TEST_PC as u64, &[enc_rfi(2)]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.ps.as_raw(), saved_ps, "RFI 2: PS must be fully restored from EPS2");
    assert_eq!(cpu.get_pc(), saved_pc, "RFI 2: PC must be EPC2");
}

/// G2: RFI level 3 — restores full PS from EPS3 and jumps to EPC3.
#[test]
fn test_exec_rfi_3() {
    let (mut cpu, mut bus) = make_cpu_bus();

    let saved_ps: u32 = 0x0000_0010;  // EXCM=1, INTLEVEL=0
    let saved_pc: u32 = 0x2000_0400;
    cpu.sr.set_raw(EPS3_ID, saved_ps);
    cpu.sr.set_raw(EPC3_ID, saved_pc);

    write_insns(&mut bus, TEST_PC as u64, &[enc_rfi(3)]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.ps.as_raw(), saved_ps, "RFI 3: PS must be fully restored from EPS3");
    assert_eq!(cpu.get_pc(), saved_pc, "RFI 3: PC must be EPC3");
}

/// G2: RFWO rotates WB forward by CALLINC, clears old WS bit, sets new WS bit,
/// clears PS.EXCM, and jumps to EPC1.
///
/// Scenario: WB=0, CALLINC=1, WS[0]=1. After RFWO: WB=1, WS[0]=0, WS[1]=1,
/// PS.EXCM=0, PC=EPC1.
#[test]
fn test_exec_rfwo_rotates_forward_and_clears_ws() {
    let (mut cpu, mut bus) = make_cpu_bus();

    let target_pc: u32 = 0x2000_0010;  // simulate EPC1 = faulting ENTRY PC
    cpu.regs.set_windowbase(0);
    cpu.regs.set_windowstart_bit(0, true);
    cpu.ps.set_callinc(1);
    cpu.ps.set_excm(true);
    cpu.sr.set_raw(EPC1_ID, target_pc);

    write_insns(&mut bus, TEST_PC as u64, &[enc_rfwo()]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.regs.windowbase(), 1, "RFWO: WB must advance by CALLINC=1 to 1");
    assert!(!cpu.regs.windowstart_bit(0), "RFWO: WS[old_WB=0] must be cleared (frame spilled)");
    assert!(cpu.regs.windowstart_bit(1), "RFWO: WS[new_WB=1] must be set (new frame live)");
    assert!(!cpu.ps.excm(), "RFWO: PS.EXCM must be 0");
    assert_eq!(cpu.get_pc(), target_pc, "RFWO: PC must be EPC1");
}

/// G2: RFWO with CALLINC=2 (OF8 scenario): WB advances by 2.
#[test]
fn test_exec_rfwo_callinc2_advances_wb_by_2() {
    let (mut cpu, mut bus) = make_cpu_bus();

    let target_pc: u32 = 0x2000_0020;
    cpu.regs.set_windowbase(3);
    cpu.regs.set_windowstart_bit(3, true);
    cpu.ps.set_callinc(2);
    cpu.ps.set_excm(true);
    cpu.sr.set_raw(EPC1_ID, target_pc);

    write_insns(&mut bus, TEST_PC as u64, &[enc_rfwo()]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.regs.windowbase(), 5, "RFWO: WB=3 + CALLINC=2 = 5");
    assert!(!cpu.regs.windowstart_bit(3), "RFWO: WS[old_WB=3] cleared");
    assert!(cpu.regs.windowstart_bit(5), "RFWO: WS[new_WB=5] set");
    assert!(!cpu.ps.excm(), "RFWO: PS.EXCM=0");
    assert_eq!(cpu.get_pc(), target_pc, "RFWO: PC=EPC1");
}

/// G2: RFWU decrements WB by 1, sets WS[new_WB], clears PS.EXCM, jumps to EPC1.
///
/// Scenario: WB=2 (underflow handler was entered with WB=2 = callee's frame).
/// After RFWU: WB=1, WS[1]=1, PS.EXCM=0, PC=EPC1.
#[test]
fn test_exec_rfwu_rotates_back_and_sets_ws() {
    let (mut cpu, mut bus) = make_cpu_bus();

    let target_pc: u32 = 0x2000_0030;  // simulate EPC1 = faulting RETW PC
    cpu.regs.set_windowbase(2);
    cpu.regs.set_windowstart_bit(2, true);   // callee frame is live
    cpu.regs.set_windowstart_bit(1, false);  // caller frame was not live (triggered UF)
    cpu.ps.set_excm(true);
    cpu.sr.set_raw(EPC1_ID, target_pc);

    write_insns(&mut bus, TEST_PC as u64, &[enc_rfwu()]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.regs.windowbase(), 1, "RFWU: WB must decrement by 1 to 1");
    assert!(cpu.regs.windowstart_bit(1), "RFWU: WS[new_WB=1] must be set (frame reloaded)");
    assert!(!cpu.ps.excm(), "RFWU: PS.EXCM must be 0");
    assert_eq!(cpu.get_pc(), target_pc, "RFWU: PC must be EPC1");
}

/// G2: RFWU wraps around at WB=0 → WB=15.
#[test]
fn test_exec_rfwu_wraps_windowbase() {
    let (mut cpu, mut bus) = make_cpu_bus();

    let target_pc: u32 = 0x2000_0040;
    cpu.regs.set_windowbase(0);
    cpu.ps.set_excm(true);
    cpu.sr.set_raw(EPC1_ID, target_pc);

    write_insns(&mut bus, TEST_PC as u64, &[enc_rfwu()]);
    cpu.step(&mut bus, &[]).unwrap();

    assert_eq!(cpu.regs.windowbase(), 15, "RFWU: WB=0 wraps to WB=15");
    assert!(cpu.regs.windowstart_bit(15), "RFWU: WS[15] set after wrap");
    assert!(!cpu.ps.excm(), "RFWU: PS.EXCM=0 after wrap");
    assert_eq!(cpu.get_pc(), target_pc, "RFWU: PC=EPC1 after wrap");
}
