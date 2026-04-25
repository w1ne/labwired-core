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
use labwired_core::cpu::xtensa_sr::{EXCCAUSE as EXCCAUSE_ID, SAR as SAR_ID};
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
/// Return PC = call_site_PC + 3.
#[test]
fn test_exec_call4_writes_return_pc_to_a4() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_call(1, TEST_PC, TEST_PC + 12), // CALL4 at TEST_PC; a4 ← TEST_PC+3
    ]);
    write_insns(&mut bus, (TEST_PC + 12) as u64, &[
        st0(4, 1, 0xF), // BREAK at subroutine entry
    ]);

    run_until_error(&mut cpu, &mut bus);
    assert_eq!(
        cpu.get_register(4),
        TEST_PC + 3,
        "a4 should hold return PC (call site + 3)"
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
    assert_eq!(cpu.get_register(8), TEST_PC + 3, "a8 should hold return PC");
    assert_eq!(cpu.ps.callinc(), 2, "PS.CALLINC should be 2 for CALLX8");
}

/// RETW must return NotImplemented (deferred to Phase F2).
#[test]
fn test_exec_retw_returns_not_implemented() {
    let mut cpu = XtensaLx7::new();
    let mut bus = SystemBus::new();
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(TEST_PC);

    write_insns(&mut bus, TEST_PC as u64, &[
        enc_retw(), // RETW — should error immediately
    ]);

    let err = cpu.step(&mut bus, &[]).unwrap_err();
    assert!(
        matches!(err, SimulationError::NotImplemented(_)),
        "RETW must return NotImplemented, got {:?}", err
    );
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
