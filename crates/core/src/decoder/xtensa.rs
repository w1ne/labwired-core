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
        0x0 => {
            match op2 {
            0x0 => decode_st0(w, r, s, t),
            0x1 => Instruction::And { ar: r, as_: s, at: t },
            0x2 => Instruction::Or  { ar: r, as_: s, at: t },
            0x3 => Instruction::Xor { ar: r, as_: s, at: t },
            // op2=4 covers both shift-setup (r=0..4) and NSA/NSAU (r=0xE/0xF).
            //
            // HW-oracle (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
            //   nsa  a3, a4 → 0x40E430: op2=4, r=0xE, s=as_=4, t=ar=3
            //   nsau a3, a4 → 0x40F430: op2=4, r=0xF, s=as_=4, t=ar=3
            // NSA/NSAU: op2=4 (constant sub-group selector), r=0xE/0xF (instruction
            // discriminator), t=ar (destination register), s=as_ (source register).
            //
            // SUBX4/SUBX8 use op2=0xE/0xF (not op2=4), so they are routed to the
            // 0xE/0xF match arms below and are never confused with NSA/NSAU.
            0x4 => match r {
                0xE => Instruction::Nsa  { ar: t, as_: s },
                0xF => Instruction::Nsau { ar: t, as_: s },
                _   => decode_st3_shiftsetup(w, r, s, t),
            },
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
        }}
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
            // MUL16 family (16-bit multiply).
            // HW-oracle (xtensa-esp32s3-elf-as + objdump, mul16u a3,a4,a5 → 0x5034c1):
            //   MUL16U op1=0x1 op2=0xC  MUL16S op1=0x1 op2=0xD
            0xC => Instruction::Mul16u { ar: r, as_: s, at: t },
            0xD => Instruction::Mul16s { ar: r, as_: s, at: t },
            _ => Instruction::Unknown(w),
        },
        // op1=0x2: MUL/DIV 32×32 family.
        // HW-oracle (xtensa-esp32s3-elf-as + objdump, mull a3,a4,a5 → 0x503482):
        //   MULL  op1=0x2 op2=0x8  MULUH op1=0x2 op2=0xA  MULSH op1=0x2 op2=0xB
        //   QUOU  op1=0x2 op2=0xC  QUOS  op1=0x2 op2=0xD
        //   REMU  op1=0x2 op2=0xE  REMS  op1=0x2 op2=0xF
        // Source: xtensa-esp32s3-elf-as + objdump (esp-15.2.0_20250920):
        //   quos a3,a4,a5 → 0xD23450  quou a3,a4,a5 → 0xC23450
        //   rems a3,a4,a5 → 0xF23450  remu a3,a4,a5 → 0xE23450
        0x2 => match op2 {
            0x8 => Instruction::Mull  { ar: r, as_: s, at: t },
            0xA => Instruction::Muluh { ar: r, as_: s, at: t },
            0xB => Instruction::Mulsh { ar: r, as_: s, at: t },
            0xC => Instruction::Quou  { ar: r, as_: s, at: t },
            0xD => Instruction::Quos  { ar: r, as_: s, at: t },
            0xE => Instruction::Remu  { ar: r, as_: s, at: t },
            0xF => Instruction::Rems  { ar: r, as_: s, at: t },
            _ => Instruction::Unknown(w),
        },
        // op1 = 0x3 — fill in later tasks.
        // op1 = 0x4..=0xF — fill in later tasks.
        _ => Instruction::Unknown(w),
    }
}

/// ST0 group — miscellaneous single-operand / zero-operand instructions.
///
/// Encoding: op0=0, op1=0, op2=0. Fields: r at bits[15:12], s at bits[11:8], t at bits[7:4].
///
/// HW-oracle verified encoding table (xtensa-esp-elf-objdump):
///
/// r=0, t=8              → RET           (s field ignored per ISA RM)
/// r=0, t=9              → RETW          (s field ignored)
/// r=0, s=<as>, t=0xA    → JX as_
/// r=0, s=<as>, t=0xC    → CALLX0 as_
/// r=0, s=<as>, t=0xD    → CALLX4 as_
/// r=0, s=<as>, t=0xE    → CALLX8 as_
/// r=0, s=<as>, t=0xF    → CALLX12 as_
/// r=2, s=0, t=0          → ISYNC
/// r=2, s=0, t=1          → RSYNC
/// r=2, s=0, t=2          → ESYNC
/// r=2, s=0, t=3          → DSYNC
/// r=2, s=0, t=0xC        → MEMW
/// r=2, s=0, t=0xD        → EXTW
/// r=2, s=0, t=0xF        → NOP
/// r=3, s=0, t=0          → RFE
/// r=3, s=2, t=0          → RFDE
/// r=3, s=4, t=0          → RFWO
/// r=3, s=5, t=0          → RFWU
/// r=3, s=<level>, t=1    → RFI level
/// r=4, s=<imm_s>, t=<imm_t> → BREAK
/// r=5, s=0, t=0          → SYSCALL
fn decode_st0(w: u32, r: u8, s: u8, t: u8) -> Instruction {
    match r {
        0x0 => match t {
            0x8 => Instruction::Ret,
            0x9 => Instruction::Retw,
            0xA => Instruction::Jx { as_: s },
            0xC => Instruction::Callx0  { as_: s },
            0xD => Instruction::Callx4  { as_: s },
            0xE => Instruction::Callx8  { as_: s },
            0xF => Instruction::Callx12 { as_: s },
            _   => Instruction::Unknown(w),
        },
        0x2 => match (s, t) {
            (0, 0x0) => Instruction::Isync,
            (0, 0x1) => Instruction::Rsync,
            (0, 0x2) => Instruction::Esync,
            (0, 0x3) => Instruction::Dsync,
            (0, 0xC) => Instruction::Memw,
            (0, 0xD) => Instruction::Extw,
            (0, 0xF) => Instruction::Nop,
            _        => Instruction::Unknown(w),
        },
        0x3 => match (t, s) {
            (0x0, 0) => Instruction::Rfe,
            (0x0, 2) => Instruction::Rfde,
            (0x0, 4) => Instruction::Rfwo,
            (0x0, 5) => Instruction::Rfwu,
            (0x1, _) => Instruction::Rfi { level: s },
            _        => Instruction::Unknown(w),
        },
        0x4 => Instruction::Break { imm_s: s, imm_t: t },
        0x5 => match (s, t) {
            (0, 0) => Instruction::Syscall,
            _      => Instruction::Unknown(w),
        },
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
        // MOVI at, imm12: 12-bit signed immediate; imm12 = {s[3:0], imm8[7:0]}.
        // HW-oracle verified: `movi a3, -100` → 0x9caf32, s=0xf, imm8=0x9c → 0xf9c → sext12=-100.
        0xA => {
            let imm12 = ((s as u32) << 8) | imm8;
            let sext = ((imm12 ^ 0x800).wrapping_sub(0x800)) as i32;
            Instruction::Movi { at: t, imm: sext }
        }
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
/// LSCI group (op0=0x3): MIN, MAX, MINU, MAXU, SEXT, CLAMPS.
///
/// HW-oracle (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
///   min  a3,a4,a5 → 0x503443: op0=3, t=4(MIN),  r=ar=3, s=as_=4, op2=at=5
///   max  a3,a4,a5 → 0x503453: op0=3, t=5(MAX),  r=ar=3, s=as_=4, op2=at=5
///   minu a3,a4,a5 → 0x503463: op0=3, t=6(MINU), r=ar=3, s=as_=4, op2=at=5
///   maxu a3,a4,a5 → 0x503473: op0=3, t=7(MAXU), r=ar=3, s=as_=4, op2=at=5
///   sext a3,a4,7  → 0x003423: op0=3, t=2(SEXT),   r=ar=3, s=as_=4, op2=sa-7=0, sa=7
///   clamps a3,a4,7 → 0x003433: op0=3, t=3(CLAMPS), r=ar=3, s=as_=4, op2=sa-7=0, sa=7
///
/// Field layout: op2=bits[23:20], op1=bits[19:16], r=bits[15:12], s=bits[11:8], t=bits[7:4].
///   - MIN/MAX/MINU/MAXU: r=ar, s=as_, op2=at, op1=0, t=subop(4,5,6,7)
///   - SEXT/CLAMPS:       r=ar, s=as_, op2=sa-7 (raw immediate, 0..=15), op1=0, t=subop(2,3)
fn decode_lsci(w: u32) -> Instruction {
    let op2 = ((w >> 20) & 0xF) as u8;
    let r   = ((w >> 12) & 0xF) as u8;
    let s   = ((w >> 8)  & 0xF) as u8;
    let t   = ((w >> 4)  & 0xF) as u8;

    match t {
        // SEXT ar, as_, sa: sign-extend as_ from bit position sa (7..=22).
        // sa = op2 + 7.  Stored in Instruction as the `t` immediate field.
        0x2 => Instruction::Sext   { ar: r, as_: s, t: op2 + 7 },
        // CLAMPS ar, as_, sa: saturate signed as_ into (sa+1)-bit range.
        // sa = op2 + 7.  Stored in Instruction as the `t` immediate field.
        0x3 => Instruction::Clamps { ar: r, as_: s, t: op2 + 7 },
        // MIN ar, as_, at: ar = signed min(as_, at).
        0x4 => Instruction::Min    { ar: r, as_: s, at: op2 },
        // MAX ar, as_, at: ar = signed max(as_, at).
        0x5 => Instruction::Max    { ar: r, as_: s, at: op2 },
        // MINU ar, as_, at: ar = unsigned min(as_, at).
        0x6 => Instruction::Minu   { ar: r, as_: s, at: op2 },
        // MAXU ar, as_, at: ar = unsigned max(as_, at).
        0x7 => Instruction::Maxu   { ar: r, as_: s, at: op2 },
        _ => Instruction::Unknown(w),
    }
}
fn decode_mac16(w: u32) -> Instruction { Instruction::Unknown(w) }

/// CALLN family (op0=5): CALL0, CALL4, CALL8, CALL12.
///
/// Encoding (ISA RM §8 CALL format):
///   bits[3:0]  = op0 = 0x5
///   bits[5:4]  = n   (selects CALL0/4/8/12)
///   bits[23:6] = imm18 (signed 18-bit word offset from (PC+3)&~3 per ISA RM §4.4)
///
/// HW-oracle verified: imm18=0 → target = (PC+3)&~3 (i.e. PC itself when PC is 4-aligned).
/// Decoder convention: `offset` is signed byte displacement from (PC+3)&~3,
/// i.e. offset = sign_extend18(imm18) * 4.  The executor applies the base as
/// ((pc+3)&!3) + offset.
fn decode_calln(w: u32) -> Instruction {
    let n = ((w >> 4) & 0x3) as u8;
    let imm18 = (w >> 6) & 0x3_FFFF;
    // Sign-extend 18-bit: XOR with 0x2_0000 (bit 17) then subtract.
    let off = (((imm18 ^ 0x2_0000).wrapping_sub(0x2_0000)) as i32) * 4;
    match n {
        0 => Instruction::Call0  { offset: off },
        1 => Instruction::Call4  { offset: off },
        2 => Instruction::Call8  { offset: off },
        3 => Instruction::Call12 { offset: off },
        _ => unreachable!(),
    }
}

/// Decode op0=0x7 — BR format: two-register conditional branches and bit-test branches.
///
/// Encoding (ISA RM §3.2 BR format / RRI8):
///   bits [3:0]  = op0 = 0x7
///   bits [7:4]  = t   (second source register, or low 4 bits of bit-index for BBCI/BBSI)
///   bits [11:8] = s   (first source register / as_)
///   bits [15:12]= r   (sub-op selector; also high bit of bit-index for BBCI/BBSI)
///   bits [23:16]= imm8 (signed 8-bit branch byte offset, added to PC+4)
///
/// Dispatch is on r (bits[15:12]), NOT on the high nibble of imm8.
/// For BBCI/BBSI the 5-bit bit-index is: ((r & 0x1) << 4) | (t & 0xF).
fn decode_b(w: u32) -> Instruction {
    let imm8 = ((w >> 16) & 0xFF) as u32;
    let r    = ((w >> 12) & 0xF) as u8;
    let s    = ((w >> 8)  & 0xF) as u8;
    let t    = ((w >> 4)  & 0xF) as u8;
    let offset = sext8(imm8) + 4;

    match r {
        0x0 => Instruction::Bnone { as_: s, at: t, offset },
        0x1 => Instruction::Beq   { as_: s, at: t, offset },
        0x2 => Instruction::Blt   { as_: s, at: t, offset },
        0x3 => Instruction::Bltu  { as_: s, at: t, offset },
        0x4 => Instruction::Ball  { as_: s, at: t, offset },
        0x5 => Instruction::Bbc   { as_: s, at: t, offset },
        0x6 | 0x7 => Instruction::Bbci {
            as_: s,
            bit: ((r & 0x1) << 4) | (t & 0xF),
            offset,
        },
        0x8 => Instruction::Bany  { as_: s, at: t, offset },
        0x9 => Instruction::Bne   { as_: s, at: t, offset },
        0xA => Instruction::Bge   { as_: s, at: t, offset },
        0xB => Instruction::Bgeu  { as_: s, at: t, offset },
        0xC => Instruction::Bnall { as_: s, at: t, offset },
        0xD => Instruction::Bbs   { as_: s, at: t, offset },
        0xE | 0xF => Instruction::Bbsi {
            as_: s,
            bit: ((r & 0x1) << 4) | (t & 0xF),
            offset,
        },
        _ => Instruction::Unknown(w),
    }
}

/// Decode op0=0x6 — SI format: J (unconditional jump), BZ family (BEQZ/BNEZ/BLTZ/BGEZ),
/// BI family (BEQI/BNEI/BLTI/BGEI), and BIU family (BLTUI/BGEUI).
///
/// Field layout (ISA RM §3.2):
///   bits [3:0]  = op0 = 0x6
///   bits [5:4]  = n   (2-bit sub-format selector)
///   bits [7:6]  = m   (2-bit sub-op selector within BZ/BI/BIU families)
///   bits [11:8] = s   (source register index)
///   bits [15:12]= r   (B4CONST/B4CONSTU table index for BI/BIU families)
///   bits [23:12]= imm12 (12-bit signed offset for BZ family, n=1)
///   bits [23:16]= imm8  (8-bit signed offset for BI/BIU families, n=2/3)
///
/// Dispatch by n:
///   n=0 → J (CALL format, imm18 at bits[23:6])
///   n=1 → BZ family (BRI12; m selects: 0=BEQZ, 1=BNEZ, 2=BLTZ, 3=BGEZ)
///   n=2 → BI family (RRI8; m selects: 0=BEQI, 1=BNEI, 2=BLTI, 3=BGEI; r→B4CONST)
///   n=3 → BIU family (RRI8; m=0,1 reserved; m=2=BLTUI, m=3=BGEUI; r→B4CONSTU)
fn decode_si(w: u32) -> Instruction {
    let n = ((w >> 4) & 0x3) as u8;
    let m = ((w >> 6) & 0x3) as u8;
    let s = ((w >> 8) & 0xF) as u8;
    let r = ((w >> 12) & 0xF) as u8;

    match n {
        0 => {
            // J: imm18 at bits[23:6], sign-extended, +4 pre-baked bias.
            let imm18 = (w >> 6) & 0x3_FFFF;
            let off = ((imm18 ^ 0x2_0000).wrapping_sub(0x2_0000)) as i32;
            Instruction::J { offset: off + 4 }
        }
        1 => {
            // BZ family (BRI12): imm12 at bits[23:12], sign-extended.
            let imm12 = (w >> 12) & 0xFFF;
            let off12 = ((imm12 ^ 0x800).wrapping_sub(0x800)) as i32 + 4;
            match m {
                0 => Instruction::Beqz { as_: s, offset: off12 },
                1 => Instruction::Bnez { as_: s, offset: off12 },
                2 => Instruction::Bltz { as_: s, offset: off12 },
                3 => Instruction::Bgez { as_: s, offset: off12 },
                _ => Instruction::Unknown(w),
            }
        }
        2 => {
            // BI family (RRI8): imm8 at bits[23:16] is the offset; r indexes B4CONST.
            let imm8 = ((w >> 16) & 0xFF) as u32;
            let off = sext8(imm8) + 4;
            match m {
                0 => Instruction::Beqi { as_: s, imm: b4const(r), offset: off },
                1 => Instruction::Bnei { as_: s, imm: b4const(r), offset: off },
                2 => Instruction::Blti { as_: s, imm: b4const(r), offset: off },
                3 => Instruction::Bgei { as_: s, imm: b4const(r), offset: off },
                _ => Instruction::Unknown(w),
            }
        }
        3 => {
            // n=3, m=0: ENTRY as_, imm12
            //
            // HW-oracle (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
            //   entry a1, 32  → 004136 → w=0x004136: op0=6, n=3, m=0, s=1, imm12=4
            //   entry a1, 256 → 020136 → w=0x020136: op0=6, n=3, m=0, s=1, imm12=32
            //   entry sp, 16  → 002136 → w=0x002136: op0=6, n=3, m=0, s=1, imm12=2
            //
            // Field layout: op0=bits[3:0]=6, n=bits[5:4]=3, m=bits[7:6]=0,
            //   as_=bits[11:8], imm12=bits[23:12].
            // Stack decrement = imm12 * 8 bytes (8-byte-aligned frames per ISA RM §4.4).
            // Instruction::Entry stores raw imm12 (not scaled).
            //
            // n=3, m=1: also reserved (Unknown).
            // n=3, m=2: BLTUI; n=3, m=3: BGEUI (BIU family, unchanged).
            match m {
                0 => {
                    // ENTRY as_, imm12
                    let imm12 = (w >> 12) & 0xFFF;
                    Instruction::Entry { as_: s, imm: imm12 }
                }
                1 => Instruction::Unknown(w), // reserved per ISA RM
                2 => {
                    // BLTUI as_, imm, offset (BIU family)
                    let imm8 = (w >> 16) & 0xFF;
                    let off = sext8(imm8) + 4;
                    Instruction::Bltui { as_: s, imm: b4constu(r), offset: off }
                }
                3 => {
                    // BGEUI as_, imm, offset (BIU family)
                    let imm8 = (w >> 16) & 0xFF;
                    let off = sext8(imm8) + 4;
                    Instruction::Bgeui { as_: s, imm: b4constu(r), offset: off }
                }
                _ => Instruction::Unknown(w),
            }
        }
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

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self) // adequate for Plan 1; proper disassembly format later
    }
}
