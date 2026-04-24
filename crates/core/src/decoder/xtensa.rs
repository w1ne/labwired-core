// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 wide (24-bit) instruction decoder.
//!
//! Entry: [`decode`] takes a 32-bit fetch word; only the low 24 bits matter.
//! Narrow (16-bit) instructions use [`super::xtensa_narrow::decode_narrow`].

use std::fmt;

/// Typed Xtensa instruction (covers MVP set: base ISA, windowed, density,
/// MUL, bit-manip, atomics). FP lands in a future plan's extension.
#[allow(dead_code, reason = "variants are used in later Plan 1 tasks B3-B8/D1-D8")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    // -- ALU reg-reg (RRR) --
    Add { ar: u8, as_: u8, at: u8 },
    Sub { ar: u8, as_: u8, at: u8 },
    And { ar: u8, as_: u8, at: u8 },
    Or  { ar: u8, as_: u8, at: u8 },
    Xor { ar: u8, as_: u8, at: u8 },
    Neg { ar: u8, at: u8 },
    Abs { ar: u8, at: u8 },
    // -- Shift --
    Sll { ar: u8, as_: u8 },
    Srl { ar: u8, at: u8 },
    Sra { ar: u8, at: u8 },
    Src { ar: u8, as_: u8, at: u8 },
    Slli { ar: u8, as_: u8, shamt: u8 },
    Srli { ar: u8, at: u8, shamt: u8 },
    Srai { ar: u8, at: u8, shamt: u8 },
    Ssl { as_: u8 }, Ssr { as_: u8 }, Ssa8l { as_: u8 }, Ssa8b { as_: u8 },
    Ssai { shamt: u8 },
    // -- Arith immediate --
    Addi { at: u8, as_: u8, imm8: i32 },
    Addmi { at: u8, as_: u8, imm: i32 },
    Movi { at: u8, imm: i32 },
    // -- Loads / stores (RRI8 / LSAI) --
    L8ui { at: u8, as_: u8, imm: u32 },
    L16ui { at: u8, as_: u8, imm: u32 },
    L16si { at: u8, as_: u8, imm: u32 },
    L32i { at: u8, as_: u8, imm: u32 },
    S8i  { at: u8, as_: u8, imm: u32 },
    S16i { at: u8, as_: u8, imm: u32 },
    S32i { at: u8, as_: u8, imm: u32 },
    L32r { at: u8, pc_rel_byte_offset: i32 },
    // -- Branches (BRI8/BRI12/BR) --
    Beq  { as_: u8, at: u8, offset: i32 },
    Bne  { as_: u8, at: u8, offset: i32 },
    Blt  { as_: u8, at: u8, offset: i32 },
    Bge  { as_: u8, at: u8, offset: i32 },
    Bltu { as_: u8, at: u8, offset: i32 },
    Bgeu { as_: u8, at: u8, offset: i32 },
    Beqz { as_: u8, offset: i32 },
    Bnez { as_: u8, offset: i32 },
    Bltz { as_: u8, offset: i32 },
    Bgez { as_: u8, offset: i32 },
    Beqi { as_: u8, imm: i32, offset: i32 },
    Bnei { as_: u8, imm: i32, offset: i32 },
    Blti { as_: u8, imm: i32, offset: i32 },
    Bgei { as_: u8, imm: i32, offset: i32 },
    Bltui { as_: u8, imm: u32, offset: i32 },
    Bgeui { as_: u8, imm: u32, offset: i32 },
    Bany { as_: u8, at: u8, offset: i32 },
    Ball { as_: u8, at: u8, offset: i32 },
    Bnone { as_: u8, at: u8, offset: i32 },
    Bnall { as_: u8, at: u8, offset: i32 },
    Bbc  { as_: u8, at: u8, offset: i32 },
    Bbs  { as_: u8, at: u8, offset: i32 },
    Bbci { as_: u8, bit: u8, offset: i32 },
    Bbsi { as_: u8, bit: u8, offset: i32 },
    // -- Jumps and calls --
    J { offset: i32 },
    Jx { as_: u8 },
    Call0 { offset: i32 },
    Callx0 { as_: u8 },
    Call4 { offset: i32 }, Callx4 { as_: u8 },
    Call8 { offset: i32 }, Callx8 { as_: u8 },
    Call12 { offset: i32 }, Callx12 { as_: u8 },
    Ret,
    Retw,
    // -- Windowed-only --
    Entry { as_: u8, imm: u32 },
    Movsp { at: u8, as_: u8 },
    Rotw { n: i8 },
    S32e { at: u8, as_: u8, imm: u32 },
    L32e { at: u8, as_: u8, imm: u32 },
    Rfwo, Rfwu,
    // -- Exception/interrupt return --
    Rfe, Rfde,
    Rfi { level: u8 },
    // -- Atomic / memory-order --
    S32c1i { at: u8, as_: u8, imm: u32 },
    L32ai  { at: u8, as_: u8, imm: u32 },
    S32ri  { at: u8, as_: u8, imm: u32 },
    // -- MUL / DIV --
    Mull { ar: u8, as_: u8, at: u8 },
    Muluh { ar: u8, as_: u8, at: u8 },
    Mulsh { ar: u8, as_: u8, at: u8 },
    Quos { ar: u8, as_: u8, at: u8 },
    Quou { ar: u8, as_: u8, at: u8 },
    Rems { ar: u8, as_: u8, at: u8 },
    Remu { ar: u8, as_: u8, at: u8 },
    Mul16s { ar: u8, as_: u8, at: u8 },
    Mul16u { ar: u8, as_: u8, at: u8 },
    // -- Bit-manip --
    Nsa { ar: u8, as_: u8 },
    Nsau { ar: u8, as_: u8 },
    Min { ar: u8, as_: u8, at: u8 },
    Max { ar: u8, as_: u8, at: u8 },
    Minu { ar: u8, as_: u8, at: u8 },
    Maxu { ar: u8, as_: u8, at: u8 },
    Sext { ar: u8, as_: u8, t: u8 },
    Clamps { ar: u8, as_: u8, t: u8 },
    Addx2 { ar: u8, as_: u8, at: u8 },
    Addx4 { ar: u8, as_: u8, at: u8 },
    Addx8 { ar: u8, as_: u8, at: u8 },
    Subx2 { ar: u8, as_: u8, at: u8 },
    Subx4 { ar: u8, as_: u8, at: u8 },
    Subx8 { ar: u8, as_: u8, at: u8 },
    // -- CSR / SR --
    Rsr { at: u8, sr: u16 },
    Wsr { at: u8, sr: u16 },
    Xsr { at: u8, sr: u16 },
    Rur { ar: u8, ur: u16 },
    Wur { at: u8, ur: u16 },
    // -- Loop (stubbed; decoded so SRs latch) --
    Loop { as_: u8, offset: i32 },
    Loopnez { as_: u8, offset: i32 },
    Loopgtz { as_: u8, offset: i32 },
    // -- Misc --
    Nop,
    Break { imm_s: u8, imm_t: u8 },
    Syscall,
    Ill,
    Memw, Extw, Isync, Rsync, Esync, Dsync,
    Unknown(u32),
}

/// Decode a 24-bit (wide) instruction. High byte of the 32-bit fetch word is
/// ignored; caller must use [`super::xtensa_length::instruction_length`] first
/// to confirm wideness.
pub fn decode(word: u32) -> Instruction {
    let w = word & 0x00FF_FFFF;
    let op0 = (w & 0x0F) as u8;
    match op0 {
        0x0 => decode_qrst(w),
        0x1 => decode_l32r(w),
        0x2 => decode_lsai(w),
        0x3 => decode_lsci(w),
        0x4 => decode_mac16(w),
        0x5 => decode_calln(w),
        0x6 => decode_si(w),
        0x7 => decode_b(w),
        _ => Instruction::Unknown(w),
    }
}

// Each `decode_*` is stubbed to `Unknown(w)` in this task; filled in by
// subsequent tasks B3..B8.
fn decode_qrst(w: u32) -> Instruction {
    let op1 = ((w >> 16) & 0xF) as u8;
    let op2 = ((w >> 20) & 0xF) as u8;
    let r   = ((w >> 12) & 0xF) as u8;
    let s   = ((w >> 8)  & 0xF) as u8;
    let t   = ((w >> 4)  & 0xF) as u8;

    match op1 {
        0x0 => match op2 {
            0x0 => decode_st0(w, r, s, t),
            0x1 => Instruction::And { ar: r, as_: s, at: t },
            0x2 => Instruction::Or  { ar: r, as_: s, at: t },
            0x3 => Instruction::Xor { ar: r, as_: s, at: t },
            0x6 => match s {
                0x0 => Instruction::Neg { ar: r, at: t },
                0x1 => Instruction::Abs { ar: r, at: t },
                _ => Instruction::Unknown(w),
            },
            0x8 => Instruction::Add { ar: r, as_: s, at: t },
            0x9 => Instruction::Addx2 { ar: r, as_: s, at: t },
            0xA => Instruction::Addx4 { ar: r, as_: s, at: t },
            0xB => Instruction::Addx8 { ar: r, as_: s, at: t },
            0xC => Instruction::Sub  { ar: r, as_: s, at: t },
            0xD => Instruction::Subx2 { ar: r, as_: s, at: t },
            0xE => Instruction::Subx4 { ar: r, as_: s, at: t },
            0xF => Instruction::Subx8 { ar: r, as_: s, at: t },
            _ => Instruction::Unknown(w),
        },
        // op1 = 0x1, 0x2, 0x3 (shifts) — fill in Task B4.
        // op1 = 0x4..=0xF — fill in later tasks.
        _ => Instruction::Unknown(w),
    }
}

/// ST0 group — miscellaneous single-operand / zero-operand instructions.
///
/// Covers RET, RETW, JX, CALLX*, NOP, ISYNC/RSYNC/ESYNC/DSYNC, MEMW/EXTW,
/// RFE/RFDE/RFI, BREAK, SYSCALL. RSR/WSR/XSR are in ST1 (not here).
/// This task implements NOP / BREAK / sync-barrier family; the rest are
/// stubbed as Unknown and filled in later tasks (B8 for CALLX/RET, G2 for
/// RFE/RFDE/RFI, D1 already consumes BREAK).
fn decode_st0(w: u32, r: u8, s: u8, t: u8) -> Instruction {
    match r {
        0x0 => match s {
            0x0 => match t {
                0x0 => Instruction::Isync,
                0x1 => Instruction::Rsync,
                0x2 => Instruction::Esync,
                0x3 => Instruction::Dsync,
                0xC => Instruction::Memw,
                0xD => Instruction::Extw,
                0xF => Instruction::Nop,
                _ => Instruction::Unknown(w),
            },
            _ => Instruction::Unknown(w),
        },
        0x4 => Instruction::Break { imm_s: s, imm_t: t },
        _ => Instruction::Unknown(w),
    }
}

fn decode_l32r(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_lsai(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_lsci(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_mac16(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_calln(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_si(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_b(w: u32) -> Instruction { Instruction::Unknown(w) }

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self) // adequate for Plan 1; proper disassembly format later
    }
}
