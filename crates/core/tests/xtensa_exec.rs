// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! D1 execution tests: ALU reg-reg (ADD/SUB/AND/OR/XOR/NEG/ABS/ADDX*/SUBX*),
//! MOVI, NOP/fence instructions, and BREAK.
//!
//! All instruction byte encodings verified via xtensa-esp-elf-objdump.
//! Bus: default SystemBus::new() provides RAM at 0x2000_0000..0x2010_0000.

use labwired_core::bus::SystemBus;
use labwired_core::cpu::xtensa_lx7::XtensaLx7;
use labwired_core::{Bus, Cpu, SimulationError};

const TEST_PC: u32 = 0x2000_0000;

// ── Encoding helpers (all HW-oracle verified via xtensa-esp-elf-objdump) ────

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
