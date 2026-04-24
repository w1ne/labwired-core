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
            0x4 => decode_st3_shiftsetup(w, r, s, t),
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
        0x1 => match op2 {
            // SLLI: 5-bit shift amount split across op2[0] (high bit) and t (low 4 bits).
            // ISA RM §8: encodes 1_sa = 32 - sa, so shamt = 32 - raw.
            0x0 | 0x1 => {
                let raw = ((op2 & 0x1) << 4) | t;
                let shamt = 32u8.wrapping_sub(raw);
                Instruction::Slli { ar: r, as_: s, shamt }
            },
            // SRAI: 5-bit shift amount; direct encoding (no complement).
            // ISA RM §8: shamt = ((op2 & 1) << 4) | t.
            0x2 | 0x3 => {
                let shamt = ((op2 & 0x1) << 4) | t;
                Instruction::Srai { ar: r, at: t, shamt }
            },
            // SRLI: 4-bit shift amount; direct from t field (0..15).
            // ISA RM §8: shamt = t.
            0x4 => Instruction::Srli { ar: r, at: t, shamt: t },
            0x8 => Instruction::Src { ar: r, as_: s, at: t },
            0x9 => Instruction::Srl { ar: r, at: t },
            0xA => Instruction::Sll { ar: r, as_: s },
            0xB => Instruction::Sra { ar: r, at: t },
            _ => Instruction::Unknown(w),
        },
        // op1 = 0x2, 0x3 (shift immediates, extended range) — fill in later tasks.
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

/// ST3 shift-setup group (`op1=0x0, op2=0x4`): SSR, SSL, SSA8L, SSA8B, SSAI.
///
/// `r` selects the specific instruction. SSAI has a 5-bit shift amount encoded
/// as `{t[0], s[3:0]}` per ISA RM §8.
fn decode_st3_shiftsetup(w: u32, r: u8, s: u8, t: u8) -> Instruction {
    match r {
        0x0 => Instruction::Ssr { as_: s },
        0x1 => Instruction::Ssl { as_: s },
        0x2 => Instruction::Ssa8l { as_: s },
        0x3 => Instruction::Ssa8b { as_: s },
        // SSAI: ISA RM §8: 5-bit shamt = {t[0], s[3:0]}.
        0x4 => Instruction::Ssai { shamt: ((t & 0x1) << 4) | s },
        _ => Instruction::Unknown(w),
    }
}

fn decode_l32r(w: u32) -> Instruction {
    let at = ((w >> 4) & 0xF) as u8;
    let imm16 = (w >> 8) & 0xFFFF;
    // Sign-extend 16-bit value (2's complement), interpret as word-count.
    let sext = ((imm16 ^ 0x8000).wrapping_sub(0x8000)) as i32;
    let pc_rel_byte_offset = sext * 4;
    Instruction::L32r { at, pc_rel_byte_offset }
}
fn decode_lsai(w: u32) -> Instruction {
    let imm8 = ((w >> 16) & 0xFF) as u32;
    let r    = ((w >> 12) & 0xF) as u8;
    let s    = ((w >> 8)  & 0xF) as u8;
    let t    = ((w >> 4)  & 0xF) as u8;

    match r {
        0x0 => Instruction::L8ui  { at: t, as_: s, imm: imm8 },
        0x1 => Instruction::L16ui { at: t, as_: s, imm: imm8 << 1 },
        0x2 => Instruction::L32i  { at: t, as_: s, imm: imm8 << 2 },
        0x4 => Instruction::S8i   { at: t, as_: s, imm: imm8 },
        0x5 => Instruction::S16i  { at: t, as_: s, imm: imm8 << 1 },
        0x6 => Instruction::S32i  { at: t, as_: s, imm: imm8 << 2 },
        0x9 => Instruction::L16si { at: t, as_: s, imm: imm8 << 1 },
        0xB => Instruction::L32ai { at: t, as_: s, imm: imm8 << 2 },
        0xC => Instruction::Addi  { at: t, as_: s, imm8: sext8(imm8) },
        0xD => Instruction::Addmi { at: t, as_: s, imm: sext8(imm8) << 8 },
        0xE => Instruction::S32c1i { at: t, as_: s, imm: imm8 << 2 },
        0xF => Instruction::S32ri  { at: t, as_: s, imm: imm8 << 2 },
        _ => Instruction::Unknown(w),
    }
}

/// Sign-extend an 8-bit value in a u32 to i32, range [-128, 127].
#[inline]
fn sext8(v: u32) -> i32 {
    ((v ^ 0x80) as i32).wrapping_sub(0x80)
}
fn decode_lsci(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_mac16(w: u32) -> Instruction { Instruction::Unknown(w) }
fn decode_calln(w: u32) -> Instruction { Instruction::Unknown(w) }

/// Decode op0=0x7 — BR format: two-register conditional branches and bit-test branches.
///
/// Encoding (ISA RM §3.2 BR format):
///   bits [3:0]  = op0 = 0x7
///   bits [7:4]  = t   (second source register for register-register branches)
///   bits [11:8] = s   (first source register)
///   bits [15:12]= r   (bit-index for BBCI/BBSI)
///   bits [23:16]= imm8 (signed 8-bit branch offset, added to PC+4)
///   bits [23:20]= op2 (high-4 bits of imm8 field overlap with sub-opcode)
///
/// Note: bits[23:20] select the branch variant; bits[23:16] form the signed offset.
fn decode_b(w: u32) -> Instruction {
    let op2 = ((w >> 20) & 0xF) as u8;
    let r   = ((w >> 12) & 0xF) as u8;
    let s   = ((w >> 8)  & 0xF) as u8;
    let t   = ((w >> 4)  & 0xF) as u8;
    let imm8 = ((w >> 16) & 0xFF) as u32;
    let offset8 = sext8(imm8);

    match op2 {
        0x0 => Instruction::Bnone { as_: s, at: t, offset: offset8 + 4 },
        0x1 => Instruction::Beq   { as_: s, at: t, offset: offset8 + 4 },
        0x2 => Instruction::Blt   { as_: s, at: t, offset: offset8 + 4 },
        0x3 => Instruction::Bltu  { as_: s, at: t, offset: offset8 + 4 },
        0x4 => Instruction::Ball  { as_: s, at: t, offset: offset8 + 4 },
        0x5 => Instruction::Bbc   { as_: s, at: t, offset: offset8 + 4 },
        0x6 | 0x7 => Instruction::Bbci { as_: s, bit: (r & 0xF) | ((op2 & 0x1) << 4), offset: offset8 + 4 },
        0x8 => Instruction::Bany  { as_: s, at: t, offset: offset8 + 4 },
        0x9 => Instruction::Bne   { as_: s, at: t, offset: offset8 + 4 },
        0xA => Instruction::Bge   { as_: s, at: t, offset: offset8 + 4 },
        0xB => Instruction::Bgeu  { as_: s, at: t, offset: offset8 + 4 },
        0xC => Instruction::Bnall { as_: s, at: t, offset: offset8 + 4 },
        0xD => Instruction::Bbs   { as_: s, at: t, offset: offset8 + 4 },
        0xE | 0xF => Instruction::Bbsi { as_: s, bit: (r & 0xF) | ((op2 & 0x1) << 4), offset: offset8 + 4 },
        _ => Instruction::Unknown(w),
    }
}

/// Decode op0=0x6 — SI format: J (unconditional jump), BZ family (BEQZ/BNEZ),
/// BI family (BEQI/BNEI/BLTI/BGEI), and BIU family (BLTUI/BGEUI).
///
/// Field layout (ISA RM §3.2 BRI12 / SI format):
///   bits [3:0]  = op0 = 0x6
///   bits [5:4]  = n   (2-bit sub-family selector)
///   bits [7:6]  = m   (2-bit family selector)
///   bits [11:8] = s   (source register index)
///   bits [23:12]= imm12 (12-bit signed offset for BRI12 BZ family)
///   bits [23:16]= imm8  (8-bit signed offset for BI/BIU families)
///   bits [15:12]= r   (B4CONST/B4CONSTU table index for BI/BIU families)
///
/// Family dispatch by m:
///   m=0: J (n=0), reserved (n=1), BZ group (n=2,3)
///   m=1: BI  — BEQI/BNEI/BLTI/BGEI (n selects which; r → B4CONST; imm8 = offset)
///   m=2: BIU — BLTUI/BGEUI         (n selects which; r → B4CONSTU; imm8 = offset)
///   m=3: reserved / Unknown
///
/// BZ (BEQZ/BNEZ/BLTZ/BGEZ) encoding note — ISA RM §8:
///   The plan spec (lines 1264-1273) erroneously dispatches on `s` (the register
///   field) rather than on `n`. Based on widely available Xtensa ISA references,
///   the correct BRI12 BZ dispatch is: m=0, n=2 → BEQZ; m=0, n=3 → BNEZ.
///   BLTZ and BGEZ do NOT appear to exist as BRI12 variants in the LX7 base ISA;
///   they may appear in some Xtensa option sets or as BRI8 sub-ops in a different
///   position. Until verified against ISA RM §8 Table 3-24 and real hardware
///   (Task H oracle tests), BLTZ/BGEZ are emitted as Unknown.
///
///   TODO(Task H): verify BEQZ (m=0,n=2) and BNEZ (m=0,n=3) against HW traces;
///   locate BLTZ/BGEZ in the ISA RM encoding table and implement here.
fn decode_si(w: u32) -> Instruction {
    let n = ((w >> 4) & 0x3) as u8;
    let m = ((w >> 6) & 0x3) as u8;
    let s = ((w >> 8) & 0xF) as u8;
    let imm12 = (w >> 12) & 0xFFF;
    let offset12 = ((imm12 ^ 0x800).wrapping_sub(0x800)) as i32;
    let imm8 = ((w >> 16) & 0xFF) as u32;
    let offset8 = sext8(imm8);

    match m {
        0 => match n {
            0 => {
                // J: imm18 = bits[23:6]; 18-bit signed offset.
                let imm18 = (w >> 6) & 0x3_FFFF;
                let off = ((imm18 ^ 0x2_0000).wrapping_sub(0x2_0000)) as i32;
                Instruction::J { offset: off + 4 }
            }
            1 => Instruction::Unknown(w), // reserved
            2 => Instruction::Beqz { as_: s, offset: offset12 + 4 },
            3 => Instruction::Bnez { as_: s, offset: offset12 + 4 },
            // BLTZ/BGEZ: not confirmed for m=0 in LX7 base ISA — see doc comment above.
            _ => Instruction::Unknown(w),
        },
        1 => decode_bi(w, n, s, imm8 as i32, offset8),
        2 => decode_bi_u(w, n, s, imm8, offset8),
        _ => Instruction::Unknown(w),
    }
}

/// Look up the B4CONST table (ISA RM Appendix B4CONST).
///
/// Maps a 4-bit register-field index r to the immediate constant used by
/// BEQI/BNEI/BLTI/BGEI.
fn b4const(r: u8) -> i32 {
    match r & 0xF {
        0 => -1, 1 => 1,  2 => 2,   3 => 3,
        4 => 4,  5 => 5,  6 => 6,   7 => 7,
        8 => 8,  9 => 10, 10 => 12, 11 => 16,
        12 => 32, 13 => 64, 14 => 128, 15 => 256,
        _ => unreachable!(),
    }
}

/// Look up the B4CONSTU table (ISA RM Appendix B4CONSTU).
///
/// Maps a 4-bit register-field index r to the unsigned immediate constant used
/// by BLTUI/BGEUI.
fn b4constu(r: u8) -> u32 {
    match r & 0xF {
        0 => 32768, 1 => 65536, 2 => 2,  3 => 3,
        4 => 4,     5 => 5,     6 => 6,  7 => 7,
        8 => 8,     9 => 10,   10 => 12, 11 => 16,
        12 => 32,  13 => 64,   14 => 128, 15 => 256,
        _ => unreachable!(),
    }
}

/// Decode BI sub-family (m=1): BEQI/BNEI/BLTI/BGEI.
///
/// n selects the comparison; r (bits[15:12]) indexes into B4CONST for the immediate;
/// offset is the sign-extended imm8 field (bits[23:16]) + 4.
fn decode_bi(w: u32, n: u8, s: u8, _imm8: i32, offset: i32) -> Instruction {
    let r = ((w >> 12) & 0xF) as u8;
    match n {
        0 => Instruction::Beqi { as_: s, imm: b4const(r), offset: offset + 4 },
        1 => Instruction::Bnei { as_: s, imm: b4const(r), offset: offset + 4 },
        2 => Instruction::Blti { as_: s, imm: b4const(r), offset: offset + 4 },
        3 => Instruction::Bgei { as_: s, imm: b4const(r), offset: offset + 4 },
        _ => Instruction::Unknown(w),
    }
}

/// Decode BIU sub-family (m=2): BLTUI/BGEUI.
///
/// n selects the comparison; r (bits[15:12]) indexes into B4CONSTU for the unsigned
/// immediate; offset is the sign-extended imm8 field (bits[23:16]) + 4.
fn decode_bi_u(w: u32, n: u8, s: u8, _imm8: u32, offset: i32) -> Instruction {
    let r = ((w >> 12) & 0xF) as u8;
    match n {
        0 => Instruction::Bltui { as_: s, imm: b4constu(r), offset: offset + 4 },
        1 => Instruction::Bgeui { as_: s, imm: b4constu(r), offset: offset + 4 },
        _ => Instruction::Unknown(w),
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self) // adequate for Plan 1; proper disassembly format later
    }
}
