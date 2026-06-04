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
#[allow(
    dead_code,
    reason = "variants are used in later Plan 1 tasks B3-B8/D1-D8"
)]
/// Single-precision FP compare predicate (FP1 group, op1=0xB, op2 selects).
/// "u*" variants are unordered (true if either operand is NaN); "o*" are
/// ordered (false if either operand is NaN). HW-oracle op2 values:
///   un=1 oeq=2 ueq=3 olt=4 ult=5 ole=6 ule=7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpCmp {
    Un,
    Oeq,
    Ueq,
    Olt,
    Ult,
    Ole,
    Ule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    // -- ALU reg-reg (RRR) --
    Add {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Sub {
        ar: u8,
        as_: u8,
        at: u8,
    },
    And {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Or {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Xor {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Neg {
        ar: u8,
        at: u8,
    },
    Abs {
        ar: u8,
        at: u8,
    },
    // -- Shift --
    Sll {
        ar: u8,
        as_: u8,
    },
    Srl {
        ar: u8,
        at: u8,
    },
    Sra {
        ar: u8,
        at: u8,
    },
    Src {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Slli {
        ar: u8,
        as_: u8,
        shamt: u8,
    },
    Srli {
        ar: u8,
        at: u8,
        shamt: u8,
    },
    Srai {
        ar: u8,
        at: u8,
        shamt: u8,
    },
    Ssl {
        as_: u8,
    },
    Ssr {
        as_: u8,
    },
    Ssa8l {
        as_: u8,
    },
    Ssa8b {
        as_: u8,
    },
    Ssai {
        shamt: u8,
    },
    /// RER — Read External Register: `AT = ext_read(AS)`. ESP32 uses this for
    /// RF/PHY/config registers outside the normal address space.
    Rer {
        at: u8,
        as_: u8,
    },
    /// WER — Write External Register: `ext_write(AS, AT)`.
    Wer {
        at: u8,
        as_: u8,
    },
    // -- Arith immediate --
    Addi {
        at: u8,
        as_: u8,
        imm8: i32,
    },
    Addmi {
        at: u8,
        as_: u8,
        imm: i32,
    },
    Movi {
        at: u8,
        imm: i32,
    },
    // -- Loads / stores (RRI8 / LSAI) --
    L8ui {
        at: u8,
        as_: u8,
        imm: u32,
    },
    L16ui {
        at: u8,
        as_: u8,
        imm: u32,
    },
    L16si {
        at: u8,
        as_: u8,
        imm: u32,
    },
    L32i {
        at: u8,
        as_: u8,
        imm: u32,
    },
    S8i {
        at: u8,
        as_: u8,
        imm: u32,
    },
    S16i {
        at: u8,
        as_: u8,
        imm: u32,
    },
    S32i {
        at: u8,
        as_: u8,
        imm: u32,
    },
    L32r {
        at: u8,
        pc_rel_byte_offset: i32,
    },
    // -- Branches (BRI8/BRI12/BR) --
    Beq {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Bne {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Blt {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Bge {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Bltu {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Bgeu {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Beqz {
        as_: u8,
        offset: i32,
    },
    Bnez {
        as_: u8,
        offset: i32,
    },
    Bltz {
        as_: u8,
        offset: i32,
    },
    Bgez {
        as_: u8,
        offset: i32,
    },
    Beqi {
        as_: u8,
        imm: i32,
        offset: i32,
    },
    Bnei {
        as_: u8,
        imm: i32,
        offset: i32,
    },
    Blti {
        as_: u8,
        imm: i32,
        offset: i32,
    },
    Bgei {
        as_: u8,
        imm: i32,
        offset: i32,
    },
    Bltui {
        as_: u8,
        imm: u32,
        offset: i32,
    },
    Bgeui {
        as_: u8,
        imm: u32,
        offset: i32,
    },
    Bany {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Ball {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Bnone {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Bnall {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Bbc {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Bbs {
        as_: u8,
        at: u8,
        offset: i32,
    },
    Bbci {
        as_: u8,
        bit: u8,
        offset: i32,
    },
    Bbsi {
        as_: u8,
        bit: u8,
        offset: i32,
    },
    // -- Jumps and calls --
    J {
        offset: i32,
    },
    Jx {
        as_: u8,
    },
    Call0 {
        offset: i32,
    },
    Callx0 {
        as_: u8,
    },
    Call4 {
        offset: i32,
    },
    Callx4 {
        as_: u8,
    },
    Call8 {
        offset: i32,
    },
    Callx8 {
        as_: u8,
    },
    Call12 {
        offset: i32,
    },
    Callx12 {
        as_: u8,
    },
    Ret,
    Retw,
    // -- Windowed-only --
    Entry {
        as_: u8,
        imm: u32,
    },
    Movsp {
        at: u8,
        as_: u8,
    },
    Rotw {
        n: i8,
    },
    S32e {
        at: u8,
        as_: u8,
        imm: u32,
    },
    L32e {
        at: u8,
        as_: u8,
        imm: u32,
    },
    Rfwo,
    Rfwu,
    // -- Exception/interrupt return --
    Rfe,
    Rfde,
    Rfi {
        level: u8,
    },
    // -- Atomic / memory-order --
    S32c1i {
        at: u8,
        as_: u8,
        imm: u32,
    },
    L32ai {
        at: u8,
        as_: u8,
        imm: u32,
    },
    S32ri {
        at: u8,
        as_: u8,
        imm: u32,
    },
    // -- SALT / SALTU: set 1 if AR[s] < AR[t] (signed / unsigned), else 0 --
    Salt {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Saltu {
        ar: u8,
        as_: u8,
        at: u8,
    },
    // -- MUL / DIV --
    Mull {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Muluh {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Mulsh {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Quos {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Quou {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Rems {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Remu {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Mul16s {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Mul16u {
        ar: u8,
        as_: u8,
        at: u8,
    },
    // -- Bit-manip --
    Nsa {
        ar: u8,
        as_: u8,
    },
    Nsau {
        ar: u8,
        as_: u8,
    },
    Min {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Max {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Minu {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Maxu {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Sext {
        ar: u8,
        as_: u8,
        t: u8,
    },
    Clamps {
        ar: u8,
        as_: u8,
        t: u8,
    },
    Addx2 {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Addx4 {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Addx8 {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Subx2 {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Subx4 {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Subx8 {
        ar: u8,
        as_: u8,
        at: u8,
    },
    // -- CSR / SR --
    Rsr {
        at: u8,
        sr: u16,
    },
    Wsr {
        at: u8,
        sr: u16,
    },
    Xsr {
        at: u8,
        sr: u16,
    },
    Rur {
        ar: u8,
        ur: u16,
    },
    Wur {
        at: u8,
        ur: u16,
    },
    // -- Loop (stubbed; decoded so SRs latch) --
    Loop {
        as_: u8,
        offset: i32,
    },
    Loopnez {
        as_: u8,
        offset: i32,
    },
    Loopgtz {
        as_: u8,
        offset: i32,
    },
    // Conditional moves (op0=0, op1=3, op2=8..=B). All take 3 registers.
    Moveqz {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Movnez {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Movltz {
        ar: u8,
        as_: u8,
        at: u8,
    },
    Movgez {
        ar: u8,
        as_: u8,
        at: u8,
    },
    // -- Misc --
    Nop,
    Break {
        imm_s: u8,
        imm_t: u8,
    },
    Syscall,
    /// WAITI level — set PS.INTLEVEL=level and halt until an interrupt
    /// at a higher level fires. In the sim we don't model interrupts
    /// for the e-paper labs, so the CPU sits at the WAITI instruction
    /// (PC doesn't advance), and the test loop's wfi-streak detector
    /// breaks out cleanly.
    Waiti {
        level: u8,
    },
    Ill,
    Memw,
    Extw,
    Isync,
    Rsync,
    Esync,
    Dsync,
    /// RSIL at, level: read PS into at, then set PS.INTLEVEL = level.
    Rsil {
        at: u8,
        level: u8,
    },
    /// EXTUI ar, at, shift, bits: ar = (at >> shift) & ((1 << bits) - 1).
    Extui {
        ar: u8,
        at: u8,
        shift: u8,
        bits: u8,
    },
    // ── Single-precision FPU (FP0 op1=0xA, FP1 op1=0xB) ────────────────────
    // `fr`/`fs`/`ft` index the 16-entry FP register file; `ar`/`as_`/`at`
    // index the AR file. Encodings HW-oracle confirmed against
    // xtensa-esp32s3-elf-as + objdump (esp-15.2.0_20250920) — see decode_fp0.
    /// add.s fr, fs, ft : f[fr] = f[fs] + f[ft]
    AddS {
        fr: u8,
        fs: u8,
        ft: u8,
    },
    /// sub.s fr, fs, ft : f[fr] = f[fs] - f[ft]
    SubS {
        fr: u8,
        fs: u8,
        ft: u8,
    },
    /// mul.s fr, fs, ft : f[fr] = f[fs] * f[ft]
    MulS {
        fr: u8,
        fs: u8,
        ft: u8,
    },
    /// madd.s fr, fs, ft : f[fr] = f[fr] + f[fs] * f[ft]
    MaddS {
        fr: u8,
        fs: u8,
        ft: u8,
    },
    /// msub.s fr, fs, ft : f[fr] = f[fr] - f[fs] * f[ft]
    MsubS {
        fr: u8,
        fs: u8,
        ft: u8,
    },
    /// abs.s fr, fs : f[fr] = |f[fs]|
    AbsS {
        fr: u8,
        fs: u8,
    },
    /// neg.s fr, fs : f[fr] = -f[fs]
    NegS {
        fr: u8,
        fs: u8,
    },
    /// mov.s fr, fs : f[fr] = f[fs]
    MovS {
        fr: u8,
        fs: u8,
    },
    /// rfr ar, fs : AR[ar] = bits(f[fs])  (move FR → AR)
    Rfr {
        ar: u8,
        fs: u8,
    },
    /// wfr fr, as_ : f[fr] = bits(AR[as_])  (move AR → FR)
    Wfr {
        fr: u8,
        as_: u8,
    },
    /// float.s fr, as_, imm : f[fr] = (f32)(i32)AR[as_] / 2^imm
    FloatS {
        fr: u8,
        as_: u8,
        imm: u8,
    },
    /// ufloat.s fr, as_, imm : f[fr] = (f32)(u32)AR[as_] / 2^imm
    UfloatS {
        fr: u8,
        as_: u8,
        imm: u8,
    },
    /// trunc.s ar, fs, imm : AR[ar] = (i32)(f[fs] * 2^imm)  (toward zero)
    TruncS {
        ar: u8,
        fs: u8,
        imm: u8,
    },
    /// utrunc.s ar, fs, imm : AR[ar] = (u32)(f[fs] * 2^imm)  (toward zero)
    UtruncS {
        ar: u8,
        fs: u8,
        imm: u8,
    },
    /// round.s ar, fs, imm : AR[ar] = (i32) round-to-nearest(f[fs] * 2^imm)
    RoundS {
        ar: u8,
        fs: u8,
        imm: u8,
    },
    /// ceil.s ar, fs, imm : AR[ar] = (i32) ceil(f[fs] * 2^imm)
    CeilS {
        ar: u8,
        fs: u8,
        imm: u8,
    },
    /// floor.s ar, fs, imm : AR[ar] = (i32) floor(f[fs] * 2^imm)
    FloorS {
        ar: u8,
        fs: u8,
        imm: u8,
    },
    /// moveqz.s fr, fs, at : if AR[at] == 0 then f[fr] = f[fs]
    MoveqzS {
        fr: u8,
        fs: u8,
        at: u8,
    },
    /// movnez.s fr, fs, at : if AR[at] != 0 then f[fr] = f[fs]
    MovnezS {
        fr: u8,
        fs: u8,
        at: u8,
    },
    /// movltz.s fr, fs, at : if (i32)AR[at]  < 0 then f[fr] = f[fs]
    MovltzS {
        fr: u8,
        fs: u8,
        at: u8,
    },
    /// movgez.s fr, fs, at : if (i32)AR[at] >= 0 then f[fr] = f[fs]
    MovgezS {
        fr: u8,
        fs: u8,
        at: u8,
    },
    /// movf.s fr, fs, bt : if BR[bt] == 0 then f[fr] = f[fs]
    MovfS {
        fr: u8,
        fs: u8,
        bt: u8,
    },
    /// movt.s fr, fs, bt : if BR[bt] == 1 then f[fr] = f[fs]
    MovtS {
        fr: u8,
        fs: u8,
        bt: u8,
    },
    /// FP compares (op1=0xB). Result written to boolean register BR[br].
    /// `kind` selects the predicate (ordered/unordered + relation).
    CmpS {
        br: u8,
        fs: u8,
        ft: u8,
        kind: FpCmp,
    },
    /// lsi ft, as_, imm : f[ft] = mem32[AR[as_] + imm]   (imm = imm8<<2)
    Lsi {
        ft: u8,
        as_: u8,
        imm: u32,
    },
    /// lsip ft, as_, imm : load then AR[as_] += imm (update form)
    Lsiu {
        ft: u8,
        as_: u8,
        imm: u32,
    },
    /// ssi ft, as_, imm : mem32[AR[as_] + imm] = f[ft]
    Ssi {
        ft: u8,
        as_: u8,
        imm: u32,
    },
    /// ssip ft, as_, imm : store then AR[as_] += imm (update form)
    Ssiu {
        ft: u8,
        as_: u8,
        imm: u32,
    },
    /// lsx fr, as_, at : f[fr] = mem32[AR[as_] + AR[at]]
    Lsx {
        fr: u8,
        as_: u8,
        at: u8,
    },
    /// lsxp fr, as_, at : load then AR[as_] += AR[at] (update form)
    Lsxu {
        fr: u8,
        as_: u8,
        at: u8,
    },
    /// ssx fr, as_, at : mem32[AR[as_] + AR[at]] = f[fr]
    Ssx {
        fr: u8,
        as_: u8,
        at: u8,
    },
    /// ssxp fr, as_, at : store then AR[as_] += AR[at] (update form)
    Ssxu {
        fr: u8,
        as_: u8,
        at: u8,
    },
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
        // op0=0x9 is a 2-byte NARROW S32I.N (Code Density), never reached as
        // a wide instruction here — the dispatcher in xtensa_lx7::step routes
        // length-2 to xtensa_narrow::decode_narrow before calling us. Earlier
        // drafts mistakenly routed op0=0x9 here to a fake S32E/L32E decoder
        // that worked only on hand-crafted test inputs (see Plan 3 Task 10
        // case study). Real S32E/L32E live in QRST op1=9 and are decoded
        // there.
        _ => Instruction::Unknown(w),
    }
}

// Each `decode_*` is stubbed to `Unknown(w)` in this task; filled in by
// subsequent tasks B3..B8.
fn decode_qrst(w: u32) -> Instruction {
    let op1 = ((w >> 16) & 0xF) as u8;
    let op2 = ((w >> 20) & 0xF) as u8;
    let r = ((w >> 12) & 0xF) as u8;
    let s = ((w >> 8) & 0xF) as u8;
    let t = ((w >> 4) & 0xF) as u8;

    match op1 {
        0x0 => {
            match op2 {
                0x0 => decode_st0(w, r, s, t),
                0x1 => Instruction::And {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0x2 => Instruction::Or {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0x3 => Instruction::Xor {
                    ar: r,
                    as_: s,
                    at: t,
                },
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
                // ROTW n: op1=0, op2=4, r=8, s=0, t=n (4-bit signed).
                //
                // HW-oracle (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
                //   rotw  1 → 0x408010: op2=4, r=8, s=0, t=1 → n=+1
                //   rotw -1 → 0x4080f0: op2=4, r=8, s=0, t=0xF → n=-1 (4-bit two's complement)
                //   rotw  7 → 0x408070: op2=4, r=8, s=0, t=7   → n=+7
                //   rotw -8 → 0x408080: op2=4, r=8, s=0, t=8   → n=-8 (4-bit two's complement)
                //
                // n is sign-extended from the 4-bit t field: values 8..=15 → -8..=-1.
                0x4 => match r {
                    0xE => Instruction::Nsa { ar: t, as_: s },
                    0xF => Instruction::Nsau { ar: t, as_: s },
                    0x8 => {
                        // Sign-extend 4-bit t field to i8: if bit3 set, subtract 16.
                        let n = if t & 0x8 != 0 {
                            (t as i8).wrapping_sub(16)
                        } else {
                            t as i8
                        };
                        Instruction::Rotw { n }
                    }
                    _ => decode_st3_shiftsetup(w, r, s, t),
                },
                0x5 => {
                    // RST5 group: MMU / TLB instructions (RITLB / WITLB / WDTLB /
                    // PITLB / PDTLB / etc per ISA RM §A.6, MMU Option). We don't
                    // model the MMU; treat the whole sub-group as no-op so that
                    // Arduino-ESP32's mpu_hal_set_region_access doesn't fault
                    // mid-boot. Real silicon executes them to populate the
                    // address-translation TLBs — irrelevant to our flat memory.
                    Instruction::Nop
                }
                0x6 => match s {
                    0x0 => Instruction::Neg { ar: r, at: t },
                    0x1 => Instruction::Abs { ar: r, at: t },
                    _ => Instruction::Unknown(w),
                },
                0x8 => Instruction::Add {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0x9 => Instruction::Addx2 {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0xA => Instruction::Addx4 {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0xB => Instruction::Addx8 {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0xC => Instruction::Sub {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0xD => Instruction::Subx2 {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0xE => Instruction::Subx4 {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0xF => Instruction::Subx8 {
                    ar: r,
                    as_: s,
                    at: t,
                },
                _ => Instruction::Unknown(w),
            }
        }
        0x1 => match op2 {
            // SLLI: 5-bit shift amount split across op2[0] (high bit) and t (low 4 bits).
            // ISA RM §8: encodes 1_sa = 32 - sa, so shamt = 32 - raw.
            0x0 | 0x1 => {
                let raw = ((op2 & 0x1) << 4) | t;
                let shamt = 32u8.wrapping_sub(raw);
                Instruction::Slli {
                    ar: r,
                    as_: s,
                    shamt,
                }
            }
            // SRAI ar, at, sa: arithmetic right shift immediate.
            // Per Xtensa ISA RM §4.3 SRAI encoding (RRR variant):
            //   r (15:12) = ar          — destination
            //   s (11:8)  = sa[3:0]     — shift amount low 4 bits
            //   t (7:4)   = at          — source register
            //   op2 bit 0 = sa[4]
            // HW-oracle: `srai a12, a12, 3` → 0x21c3c0 (s=3, t=12, r=12, op2=2).
            // Earlier commit e96c2f4 swapped source<->shamt fields — both fields
            // came out wrong (source from `s`, shamt from `t`). For drawPixel's
            // `srai a12, a12, 3` that mis-execed as `a12 = a3 >> 12` instead of
            // `a12 = a12 >> 3`, producing wrong byte indices into the framebuffer.
            0x2 | 0x3 => {
                let shamt = ((op2 & 0x1) << 4) | s;
                Instruction::Srai {
                    ar: r,
                    at: t,
                    shamt,
                }
            }
            // SRLI: 4-bit shift amount in `s` field (bits[11:8]); source register
            // in `t` field (bits[7:4]). HW-oracle (xtensa-esp32s3-elf-as):
            //   srli a8, a8, 13 → 0x418D80: t=8 (at), s=D=13 (shamt), r=8 (ar)
            //   srli a3, a4, 5  → 0x413540: t=4, s=5, r=3
            //   srli a5, a6, 0  → 0x415060: t=6, s=0, r=5
            // Earlier draft of this decoder (and the matching xtensa_exec
            // unit test) had `shamt = t = at`, treating the shift amount as
            // colocated with the source register. That worked for hand-
            // crafted tests but mis-decoded esp-hal's `(prid >> 13) & 1`
            // CPU-discrimination check inside `__level_1_interrupt`, which
            // crosses CPU0/CPU1 paths and routed every interrupt status
            // read to the CPU1 INTR_STATUS bank — the alarm source IDs were
            // stale (Plan 3 Task 10 case study).
            0x4 => Instruction::Srli {
                ar: r,
                at: t,
                shamt: s,
            },
            0x8 => Instruction::Src {
                ar: r,
                as_: s,
                at: t,
            },
            0x9 => Instruction::Srl { ar: r, at: t },
            0xA => Instruction::Sll { ar: r, as_: s },
            0xB => Instruction::Sra { ar: r, at: t },
            // MUL16 family (16-bit multiply).
            // HW-oracle (xtensa-esp32s3-elf-as + objdump, mul16u a3,a4,a5 → 0x5034c1):
            //   MUL16U op1=0x1 op2=0xC  MUL16S op1=0x1 op2=0xD
            0xC => Instruction::Mul16u {
                ar: r,
                as_: s,
                at: t,
            },
            0xD => Instruction::Mul16s {
                ar: r,
                as_: s,
                at: t,
            },
            // XSR — atomic SR swap. Despite RSR/WSR living in op1=3, XSR is
            // op1=1, op2=6. HW-oracle:
            //   xsr.sar      a3 → 0x610330: op0=0,op1=1,op2=6,r=0,s=3,t=3
            //   xsr.intenable a13→ 0x61e4d0: op0=0,op1=1,op2=6,r=0xE,s=4,t=0xD; sr=0xE4
            // SR ID is bits[15:8] = (r<<4)|s; at = t.
            0x6 => {
                let sr = ((r as u16) << 4) | (s as u16);
                Instruction::Xsr { at: t, sr }
            }
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
            // SALT/SALTU: AR[r] = (AR[s] {< | <u} AR[t]) ? 1 : 0.
            // HW-oracle (xtensa-esp32s3-elf objdump): `saltu a7,a5,a4` → 0x627540
            // (op1=2,op2=6); `salt a2,a9,a8` → 0x722980 (op1=2,op2=7).
            0x6 => Instruction::Saltu {
                ar: r,
                as_: s,
                at: t,
            },
            0x7 => Instruction::Salt {
                ar: r,
                as_: s,
                at: t,
            },
            0x8 => Instruction::Mull {
                ar: r,
                as_: s,
                at: t,
            },
            0xA => Instruction::Muluh {
                ar: r,
                as_: s,
                at: t,
            },
            0xB => Instruction::Mulsh {
                ar: r,
                as_: s,
                at: t,
            },
            0xC => Instruction::Quou {
                ar: r,
                as_: s,
                at: t,
            },
            0xD => Instruction::Quos {
                ar: r,
                as_: s,
                at: t,
            },
            0xE => Instruction::Remu {
                ar: r,
                as_: s,
                at: t,
            },
            0xF => Instruction::Rems {
                ar: r,
                as_: s,
                at: t,
            },
            _ => Instruction::Unknown(w),
        },
        // op1 = 0x3: RSR / WSR / RUR / WUR.
        //
        // HW-oracle (xtensa-esp-elf-as + objdump, esp-15.2.0_20250920):
        //   rsr.sar a3      → 0x030330: op0=0,op1=3,op2=0; sr=bits[15:8]=0x03; at=t=3
        //   wsr.sar a3      → 0x130330: op0=0,op1=3,op2=1; sr=bits[15:8]=0x03; at=t=3
        //   rur.threadptr a3→ 0xe33e70: op0=0,op1=3,op2=0xe; ar=r=3; ur=(s<<4)|t=0xe7=231
        //   wur.threadptr a3→ 0xf3e730: op0=0,op1=3,op2=0xf; at=t=3; ur=bits[15:8]=0xe7=231
        //
        // For RSR/WSR: SR ID is bits[15:8] = (r << 4) | s; at = t (bits[7:4]).
        // For RUR:     ar = r (bits[15:12]); UR ID = (s << 4) | t.
        // For WUR:     at = t (bits[7:4]); UR ID = bits[15:8] = (r << 4) | s.
        //
        // XSR is NOT in op1=3: see op1=1 below.
        0x3 => {
            let sr = ((r as u16) << 4) | (s as u16);
            match op2 {
                0x0 => Instruction::Rsr { at: t, sr },
                0x1 => Instruction::Wsr { at: t, sr },
                // SEXT / CLAMPS — sign-extend / saturate at op0=0, op1=3,
                // op2=2/3. The SEXT/CLAMPS instructions also have a
                // mirror at op0=3, op1=0 in `decode_lsci`; the Xtensa LX7
                // ISA tolerates both encodings. esp-hal-1.1's compiled
                // sign-extend sequence (`sext aN, aM, 7`, bytes `00 8M 23`)
                // uses the QRST encoding, so we must decode it here too —
                // omitting this slot caused IllegalInstruction faults
                // mid-handler when running real esp-hal firmware.
                //
                // Encoding: r=ar, s=as_, sa = t + 7 (range 7..=22).
                0x2 => Instruction::Sext {
                    ar: r,
                    as_: s,
                    t: t + 7,
                },
                0x3 => Instruction::Clamps {
                    ar: r,
                    as_: s,
                    t: t + 7,
                },
                // MIN/MAX/MINU/MAXU live in op1=3, op2=4..=7 — three-operand
                // RRR encoding, not the SR-access slot. HW-oracle:
                //   min  a3, a4, a5 → 0x433450: op2=4
                //   max  a3, a4, a5 → 0x533450: op2=5
                //   minu a3, a4, a5 → 0x633450: op2=6
                //   maxu a3, a4, a5 → 0x733450: op2=7
                0x4 => Instruction::Min {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0x5 => Instruction::Max {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0x6 => Instruction::Minu {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0x7 => Instruction::Maxu {
                    ar: r,
                    as_: s,
                    at: t,
                },
                // Conditional moves (op0=0, op1=3, op2=8..=B). RRR encoding,
                // ar = bits[15:12], as_ = bits[11:8], at = bits[7:4].
                // MOVEQZ ar, as, at: if at == 0 then ar = as.
                // MOVNEZ ar, as, at: if at != 0 then ar = as.
                // MOVLTZ ar, as, at: if (i32) at  < 0 then ar = as.
                // MOVGEZ ar, as, at: if (i32) at >= 0 then ar = as.
                0x8 => Instruction::Moveqz {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0x9 => Instruction::Movnez {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0xA => Instruction::Movltz {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0xB => Instruction::Movgez {
                    ar: r,
                    as_: s,
                    at: t,
                },
                0xE => {
                    let ur = ((s as u16) << 4) | (t as u16);
                    Instruction::Rur { ar: r, ur }
                }
                0xF => Instruction::Wur { at: t, ur: sr },
                _ => Instruction::Unknown(w),
            }
        }
        // op1 = 0x4 / 0x5: EXTUI ar, at, shift, bits.
        //
        // EXTUI extracts `bits` consecutive bits from `at` starting at bit
        // `shift`, zero-extending into `ar`. The 5-bit shift is split:
        //   shift[3:0] = s field
        //   shift[4]   = op1 LSB (so op1 ∈ {4, 5} both select EXTUI)
        // The 4-bit bits-1 (range 1..=16) lives in op2.
        //
        // HW-oracle (xtensa-esp32s3-elf-as):
        //   extui a5, a8, 21, 11 → 0xa55580: op0=0, op1=5, op2=0xa, r=5, s=5, t=8
        //                          → shift=(1<<4)|5=21, bits=op2+1=11. ✓
        //   extui a3, a4, 0, 1   → 0x043040: op1=4, op2=0 → shift=0, bits=1. ✓
        //   extui a3, a4, 31, 1  → 0x053f40: op1=5, s=0xf, op2=0 → shift=31, bits=1. ✓
        0x4 | 0x5 => {
            let shift = ((op1 & 0x1) << 4) | s;
            let bits = op2 + 1;
            Instruction::Extui {
                ar: r,
                at: t,
                shift,
                bits,
            }
        }
        // op1 = 0x9 — LSC4 group: S32E / L32E (windowed exception store/load).
        //
        // HW-oracle (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
        //   s32e a3, a4, -16 → bytes 30 C4 49 → LE u32 0x49C430:
        //     op0=0, op1=9 (bits[19:16]), op2=4 (bits[23:20]) → S32E
        //     at = bits[7:4] = 3, as_ = bits[11:8] = 4, imm4 = bits[15:12] = 12
        //     imm_byte = 12*4 - 64 = -16  ✓
        //   s32e a0, a1, -12 → bytes 00 D1 49 → LE u32 0x49D100:
        //     op2=4 → S32E; at=0, as_=1, imm4=13 → imm_byte = -12  ✓
        //   l32e a3, a4, -16 → bytes 30 C4 09 → LE u32 0x09C430: op2=0 → L32E
        //
        // Field layout for op0=0, op1=9:
        //   bits[3:0]   = op0 = 0
        //   bits[7:4]   = at (value / destination register)
        //   bits[11:8]  = as_ (base register)
        //   bits[15:12] = imm4 (range 0..15; imm_byte = imm4*4 - 64, range -64..-4)
        //   bits[19:16] = op1 = 9
        //   bits[23:20] = op2: 0 = L32E, 4 = S32E
        //
        // Earlier drafts dispatched these via a top-level `op0 == 9` arm with
        // swapped field positions — that worked only on hand-crafted test
        // inputs and missed real-firmware S32E (e.g. esp-hal's
        // __default_naked_exception spill at PC 0x40379049). The real
        // assembler emits op0=0, so the QRST routing is canonical.
        0x9 => {
            // imm_byte = imm4 * 4 - 64  (range -64..-4), stored as two's-complement u32.
            let imm4 = (w >> 12) & 0xF;
            let imm = imm4.wrapping_mul(4).wrapping_sub(64);
            let at = t; // bits[7:4]
            let as_ = s; // bits[11:8]
            match op2 {
                0x0 => Instruction::L32e { at, as_, imm },
                0x4 => Instruction::S32e { at, as_, imm },
                _ => Instruction::Unknown(w),
            }
        }
        // op1 = 0x8 — LSCX group: indexed FP loads/stores (LSX/LSXU/SSX/SSXU).
        // RRR-shaped: op2 selects, r=fr, s=as_, t=at (index register).
        // HW-oracle (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
        //   lsx  f2, a1, a3 → 0x082130: op2=0, r=2, s=1, t=3
        //   lsxp f2, a1, a3 → 0x182130: op2=1
        //   ssx  f2, a1, a3 → 0x482130: op2=4
        //   ssxp f2, a1, a3 → 0x582130: op2=5
        0x8 => match op2 {
            0x0 => Instruction::Lsx {
                fr: r,
                as_: s,
                at: t,
            },
            0x1 => Instruction::Lsxu {
                fr: r,
                as_: s,
                at: t,
            },
            0x4 => Instruction::Ssx {
                fr: r,
                as_: s,
                at: t,
            },
            0x5 => Instruction::Ssxu {
                fr: r,
                as_: s,
                at: t,
            },
            _ => Instruction::Unknown(w),
        },
        // op1 = 0xA / 0xB — single-precision FPU. See decode_fp0 / decode_fp1.
        0xA => decode_fp0(w, op2, r, s, t),
        0xB => decode_fp1(w, op2, r, s, t),
        // op1 = 0x6..=0x7, 0xC..=0xF — fill in as needed.
        _ => Instruction::Unknown(w),
    }
}

/// FP0 group (op0=0, op1=0xA): single-precision arithmetic, conversions, and
/// the op2=0xF sub-group (mov/abs/neg/rfr/wfr).
///
/// HW-oracle confirmed (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
///   add.s f3,f4,f5 → 0x0a3450 (op2=0)   sub.s → 0x1a3450 (op2=1)
///   mul.s          → 0x2a3450 (op2=2)   madd.s → 0x4a3450 (op2=4)
///   msub.s         → 0x5a3450 (op2=5)
///   round.s a3,f4,n → 0x8a34n0 (op2=8)  trunc.s → 0x9a34n0 (op2=9)
///   floor.s        → 0xaa34n0 (op2=0xA) ceil.s → 0xba34n0 (op2=0xB)
///   utrunc.s       → 0xea34n0 (op2=0xE)
///   float.s f3,a4,n → 0xca34n0 (op2=0xC) ufloat.s → 0xda34n0 (op2=0xD)
///   op2=0xF: t selects — mov.s(t=0) abs.s(t=1) rfr(t=4) wfr(t=5) neg.s(t=6).
/// For float/ufloat: r=fr, s=as_, t=imm. For the int-result conversions
/// (round/trunc/utrunc/ceil/floor): r=ar, s=fs, t=imm.
fn decode_fp0(w: u32, op2: u8, r: u8, s: u8, t: u8) -> Instruction {
    match op2 {
        0x0 => Instruction::AddS {
            fr: r,
            fs: s,
            ft: t,
        },
        0x1 => Instruction::SubS {
            fr: r,
            fs: s,
            ft: t,
        },
        0x2 => Instruction::MulS {
            fr: r,
            fs: s,
            ft: t,
        },
        0x4 => Instruction::MaddS {
            fr: r,
            fs: s,
            ft: t,
        },
        0x5 => Instruction::MsubS {
            fr: r,
            fs: s,
            ft: t,
        },
        0x8 => Instruction::RoundS {
            ar: r,
            fs: s,
            imm: t,
        },
        0x9 => Instruction::TruncS {
            ar: r,
            fs: s,
            imm: t,
        },
        0xA => Instruction::FloorS {
            ar: r,
            fs: s,
            imm: t,
        },
        0xB => Instruction::CeilS {
            ar: r,
            fs: s,
            imm: t,
        },
        0xC => Instruction::FloatS {
            fr: r,
            as_: s,
            imm: t,
        },
        0xD => Instruction::UfloatS {
            fr: r,
            as_: s,
            imm: t,
        },
        0xE => Instruction::UtruncS {
            ar: r,
            fs: s,
            imm: t,
        },
        0xF => match t {
            0x0 => Instruction::MovS { fr: r, fs: s },
            0x1 => Instruction::AbsS { fr: r, fs: s },
            0x4 => Instruction::Rfr { ar: r, fs: s },
            0x5 => Instruction::Wfr { fr: r, as_: s },
            0x6 => Instruction::NegS { fr: r, fs: s },
            _ => Instruction::Unknown(w),
        },
        _ => Instruction::Unknown(w),
    }
}

/// FP1 group (op0=0, op1=0xB): compares (write boolean BR[r]) and the
/// floating-point conditional moves (moveqz/movnez/movltz/movgez/movf/movt).
///
/// HW-oracle confirmed (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
///   un.s  b0,f4,f5 → 0x1b0450 (op2=1)   oeq.s → 0x2b0450 (op2=2)
///   ueq.s          → 0x3b0450 (op2=3)   olt.s → 0x4b0450 (op2=4)
///   ult.s          → 0x5b0450 (op2=5)   ole.s → 0x6b0450 (op2=6)
///   ule.s          → 0x7b0450 (op2=7)
///   moveqz.s f3,f4,a5 → 0x8b3450 (op2=8) movnez.s → 0x9b3450 (op2=9)
///   movltz.s          → 0xab3450 (op2=0xA) movgez.s → 0xbb3450 (op2=0xB)
///   movf.s f3,f4,b5   → 0xcb3450 (op2=0xC) movt.s → 0xdb3450 (op2=0xD)
/// Compares: r=br (boolean dest), s=fs, t=ft. Moves: r=fr, s=fs, t=at/bt.
fn decode_fp1(w: u32, op2: u8, r: u8, s: u8, t: u8) -> Instruction {
    use FpCmp::*;
    let cmp = |kind| Instruction::CmpS {
        br: r,
        fs: s,
        ft: t,
        kind,
    };
    match op2 {
        0x1 => cmp(Un),
        0x2 => cmp(Oeq),
        0x3 => cmp(Ueq),
        0x4 => cmp(Olt),
        0x5 => cmp(Ult),
        0x6 => cmp(Ole),
        0x7 => cmp(Ule),
        0x8 => Instruction::MoveqzS {
            fr: r,
            fs: s,
            at: t,
        },
        0x9 => Instruction::MovnezS {
            fr: r,
            fs: s,
            at: t,
        },
        0xA => Instruction::MovltzS {
            fr: r,
            fs: s,
            at: t,
        },
        0xB => Instruction::MovgezS {
            fr: r,
            fs: s,
            at: t,
        },
        0xC => Instruction::MovfS {
            fr: r,
            fs: s,
            bt: t,
        },
        0xD => Instruction::MovtS {
            fr: r,
            fs: s,
            bt: t,
        },
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
            0xC => Instruction::Callx0 { as_: s },
            0xD => Instruction::Callx4 { as_: s },
            0xE => Instruction::Callx8 { as_: s },
            0xF => Instruction::Callx12 { as_: s },
            _ => Instruction::Unknown(w),
        },
        // MOVSP at, as_: move stack pointer between adjacent windowed frames safely.
        //
        // HW-oracle (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
        //   movsp a3, a4 → 0x001430: op0=0, op1=0, op2=0 (ST0 group), r=1, s=as_=4, t=at=3.
        //
        // Field layout (op0=0, op1=0, op2=0, r=1): s=as_ (source), t=at (destination).
        0x1 => Instruction::Movsp { at: t, as_: s },
        0x2 => match (s, t) {
            (0, 0x0) => Instruction::Isync,
            (0, 0x1) => Instruction::Rsync,
            (0, 0x2) => Instruction::Esync,
            (0, 0x3) => Instruction::Dsync,
            (0, 0xC) => Instruction::Memw,
            (0, 0xD) => Instruction::Extw,
            (0, 0xF) => Instruction::Nop,
            _ => Instruction::Unknown(w),
        },
        0x3 => match (t, s) {
            (0x0, 0) => Instruction::Rfe,
            (0x0, 2) => Instruction::Rfde,
            (0x0, 4) => Instruction::Rfwo,
            (0x0, 5) => Instruction::Rfwu,
            (0x1, _) => Instruction::Rfi { level: s },
            _ => Instruction::Unknown(w),
        },
        0x4 => Instruction::Break { imm_s: s, imm_t: t },
        0x5 => match (s, t) {
            (0, 0) => Instruction::Syscall,
            _ => Instruction::Unknown(w),
        },
        // RSIL at, level: read PS into at, set PS.INTLEVEL = level.
        // ST0 group: op0=0, op1=0, op2=0, r=6.
        // s = level (4-bit immediate, typically 0..7), t = at.
        // HW-oracle (xtensa-esp32s3-elf-as):
        //   rsil a8, 5 → 0x006580: r=6, s=5 (level), t=8 (at).
        0x6 => Instruction::Rsil { at: t, level: s },
        // WAITI imm4 — ST0 group, op2=0, r=7, s=0, t=imm4 (interrupt level).
        // ISA RM §7.4: WAITI sets PS.INTLEVEL=imm4 and waits for an
        // interrupt of higher level. We don't model interrupts in the
        // ESP32-classic e-paper labs, so the CPU stays parked on the
        // instruction — see exec arm in xtensa_lx7.rs.
        0x7 => match s {
            0 => Instruction::Waiti { level: t },
            _ => Instruction::Unknown(w),
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
        0x4 => Instruction::Ssai {
            shamt: ((t & 0x1) << 4) | s,
        },
        // RER (r=6) / WER (r=7): external-register access (ESP32 RF/PHY/config).
        0x6 => Instruction::Rer { at: t, as_: s },
        0x7 => Instruction::Wer { at: t, as_: s },
        _ => Instruction::Unknown(w),
    }
}

fn decode_l32r(w: u32) -> Instruction {
    let at = ((w >> 4) & 0xF) as u8;
    let imm16 = (w >> 8) & 0xFFFF;
    // Per Xtensa ISA RM §A.2 (L32R Format): the 16-bit immediate is NOT
    // a regular two's-complement value. The literal pool always lives
    // BELOW the instruction, and the encoding sign-extends imm16 by
    // forcing bits[31:16]=0xFFFF before the *4 shift. So a raw imm16
    // value 0x770E produces a negative byte offset, not positive.
    //
    // EA = ((PC + 3) & ~3) + ((imm16 | 0xFFFF_0000) << 2)
    //
    // The wrapping-shift produces a 32-bit two's-complement byte offset.
    // Earlier this routine sign-extended via (imm16 ^ 0x8000)-0x8000,
    // which treats imm16 < 0x8000 as positive — breaking every l32r whose
    // literal sits more than 64 KiB above the instruction. Discovered
    // booting Arduino-ESP32 firmware: rtc_init's callx8 went to PC=0
    // because the literal-pool load resolved to the wrong address.
    let offset_bytes = ((imm16 | 0xFFFF_0000) << 2) as i32;
    Instruction::L32r {
        at,
        pc_rel_byte_offset: offset_bytes,
    }
}
fn decode_lsai(w: u32) -> Instruction {
    let imm8 = (w >> 16) & 0xFF;
    let r = ((w >> 12) & 0xF) as u8;
    let s = ((w >> 8) & 0xF) as u8;
    let t = ((w >> 4) & 0xF) as u8;

    match r {
        0x0 => Instruction::L8ui {
            at: t,
            as_: s,
            imm: imm8,
        },
        0x1 => Instruction::L16ui {
            at: t,
            as_: s,
            imm: imm8 << 1,
        },
        0x2 => Instruction::L32i {
            at: t,
            as_: s,
            imm: imm8 << 2,
        },
        0x4 => Instruction::S8i {
            at: t,
            as_: s,
            imm: imm8,
        },
        0x5 => Instruction::S16i {
            at: t,
            as_: s,
            imm: imm8 << 1,
        },
        0x6 => Instruction::S32i {
            at: t,
            as_: s,
            imm: imm8 << 2,
        },
        0x9 => Instruction::L16si {
            at: t,
            as_: s,
            imm: imm8 << 1,
        },
        0xB => Instruction::L32ai {
            at: t,
            as_: s,
            imm: imm8 << 2,
        },
        // MOVI at, imm12: 12-bit signed immediate; imm12 = {s[3:0], imm8[7:0]}.
        // HW-oracle verified: `movi a3, -100` → 0x9caf32, s=0xf, imm8=0x9c → 0xf9c → sext12=-100.
        0xA => {
            let imm12 = ((s as u32) << 8) | imm8;
            let sext = ((imm12 ^ 0x800).wrapping_sub(0x800)) as i32;
            Instruction::Movi { at: t, imm: sext }
        }
        0xC => Instruction::Addi {
            at: t,
            as_: s,
            imm8: sext8(imm8),
        },
        0xD => Instruction::Addmi {
            at: t,
            as_: s,
            imm: sext8(imm8) << 8,
        },
        0xE => Instruction::S32c1i {
            at: t,
            as_: s,
            imm: imm8 << 2,
        },
        0xF => Instruction::S32ri {
            at: t,
            as_: s,
            imm: imm8 << 2,
        },
        _ => Instruction::Unknown(w),
    }
}

// (decode_s32e_l32e removed — S32E/L32E are decoded inline in QRST/op1=9
//  at the canonical bit positions verified against real esp-hal firmware.)

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
    let r = ((w >> 12) & 0xF) as u8;
    let s = ((w >> 8) & 0xF) as u8;
    let t = ((w >> 4) & 0xF) as u8;
    let imm8 = (w >> 16) & 0xFF;

    // op0=3 is the LSCI format: single-precision FP loads/stores with an
    // 8-bit immediate (LSI/LSIU/SSI/SSIU). The `r` field is the sub-opcode;
    // imm8 at bits[23:16] is the WHOLE top byte (scaled <<2 → 0..1020), so
    // there is NO separate op1/op2 here. s=as_ (base AR), t=ft (FP reg).
    //
    // HW-oracle (xtensa-esp32s3-elf-as + objdump, esp-15.2.0_20250920):
    //   lsi  f2, a1, 0 → 0x000123   lsi f2,a1,4 → 0x010123   lsi f2,a1,1020 → 0xff0123
    //   lsip f2, a1, 0 → 0x008123 (r=8)   ssi → 0x004123 (r=4)   ssip → 0x00c123 (r=0xC)
    // So r=0 LSI, r=8 LSIU, r=4 SSI, r=0xC SSIU; imm = imm8 << 2.
    //
    // (SEXT/CLAMPS/MIN/MAX/MINU/MAXU are NOT in op0=3 — the real assembler
    // emits them at op0=0, op1=3 (see decode_qrst). The previous draft of this
    // routine decoded them here from hand-crafted nibble-swapped inputs; they
    // are kept as the `_` fallback below only so any such legacy byte pattern
    // still resolves rather than faulting, but no real firmware reaches it.)
    let imm = imm8 << 2;
    match r {
        0x0 => return Instruction::Lsi { ft: t, as_: s, imm },
        0x8 => return Instruction::Lsiu { ft: t, as_: s, imm },
        0x4 => return Instruction::Ssi { ft: t, as_: s, imm },
        0xC => return Instruction::Ssiu { ft: t, as_: s, imm },
        _ => {}
    }

    match t {
        // SEXT ar, as_, sa: sign-extend as_ from bit position sa (7..=22).
        // sa = op2 + 7.  Stored in Instruction as the `t` immediate field.
        0x2 => Instruction::Sext {
            ar: r,
            as_: s,
            t: op2 + 7,
        },
        // CLAMPS ar, as_, sa: saturate signed as_ into (sa+1)-bit range.
        // sa = op2 + 7.  Stored in Instruction as the `t` immediate field.
        0x3 => Instruction::Clamps {
            ar: r,
            as_: s,
            t: op2 + 7,
        },
        // MIN ar, as_, at: ar = signed min(as_, at).
        0x4 => Instruction::Min {
            ar: r,
            as_: s,
            at: op2,
        },
        // MAX ar, as_, at: ar = signed max(as_, at).
        0x5 => Instruction::Max {
            ar: r,
            as_: s,
            at: op2,
        },
        // MINU ar, as_, at: ar = unsigned min(as_, at).
        0x6 => Instruction::Minu {
            ar: r,
            as_: s,
            at: op2,
        },
        // MAXU ar, as_, at: ar = unsigned max(as_, at).
        0x7 => Instruction::Maxu {
            ar: r,
            as_: s,
            at: op2,
        },
        _ => Instruction::Nop,
    }
}
fn decode_mac16(w: u32) -> Instruction {
    Instruction::Unknown(w)
}

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
        0 => Instruction::Call0 { offset: off },
        1 => Instruction::Call4 { offset: off },
        2 => Instruction::Call8 { offset: off },
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
    let imm8 = (w >> 16) & 0xFF;
    let r = ((w >> 12) & 0xF) as u8;
    let s = ((w >> 8) & 0xF) as u8;
    let t = ((w >> 4) & 0xF) as u8;
    let offset = sext8(imm8) + 4;

    match r {
        0x0 => Instruction::Bnone {
            as_: s,
            at: t,
            offset,
        },
        0x1 => Instruction::Beq {
            as_: s,
            at: t,
            offset,
        },
        0x2 => Instruction::Blt {
            as_: s,
            at: t,
            offset,
        },
        0x3 => Instruction::Bltu {
            as_: s,
            at: t,
            offset,
        },
        0x4 => Instruction::Ball {
            as_: s,
            at: t,
            offset,
        },
        0x5 => Instruction::Bbc {
            as_: s,
            at: t,
            offset,
        },
        0x6 | 0x7 => Instruction::Bbci {
            as_: s,
            bit: ((r & 0x1) << 4) | (t & 0xF),
            offset,
        },
        0x8 => Instruction::Bany {
            as_: s,
            at: t,
            offset,
        },
        0x9 => Instruction::Bne {
            as_: s,
            at: t,
            offset,
        },
        0xA => Instruction::Bge {
            as_: s,
            at: t,
            offset,
        },
        0xB => Instruction::Bgeu {
            as_: s,
            at: t,
            offset,
        },
        0xC => Instruction::Bnall {
            as_: s,
            at: t,
            offset,
        },
        0xD => Instruction::Bbs {
            as_: s,
            at: t,
            offset,
        },
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
                0 => Instruction::Beqz {
                    as_: s,
                    offset: off12,
                },
                1 => Instruction::Bnez {
                    as_: s,
                    offset: off12,
                },
                2 => Instruction::Bltz {
                    as_: s,
                    offset: off12,
                },
                3 => Instruction::Bgez {
                    as_: s,
                    offset: off12,
                },
                _ => Instruction::Unknown(w),
            }
        }
        2 => {
            // BI family (RRI8): imm8 at bits[23:16] is the offset; r indexes B4CONST.
            let imm8 = (w >> 16) & 0xFF;
            let off = sext8(imm8) + 4;
            match m {
                0 => Instruction::Beqi {
                    as_: s,
                    imm: b4const(r),
                    offset: off,
                },
                1 => Instruction::Bnei {
                    as_: s,
                    imm: b4const(r),
                    offset: off,
                },
                2 => Instruction::Blti {
                    as_: s,
                    imm: b4const(r),
                    offset: off,
                },
                3 => Instruction::Bgei {
                    as_: s,
                    imm: b4const(r),
                    offset: off,
                },
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
                1 => {
                    // n=3, m=1: LOOP family (BRI8-shaped, r selects variant).
                    // Per Xtensa ISA RM §7.4 Zero-Overhead Loop Option:
                    //   r=8  → LOOP   as_, imm8 (always taken)
                    //   r=9  → LOOPNEZ as_, imm8 (skip body if as_==0)
                    //   r=10 → LOOPGTZ as_, imm8 (skip body if as_<=0 signed)
                    // imm8 in bits[23:16], offset relative to PC+4.
                    let imm8 = (w >> 16) & 0xFF;
                    let offset = imm8 as i32 + 4;
                    match r {
                        8 => Instruction::Loop { as_: s, offset },
                        9 => Instruction::Loopnez { as_: s, offset },
                        10 => Instruction::Loopgtz { as_: s, offset },
                        _ => Instruction::Unknown(w),
                    }
                }
                2 => {
                    // BLTUI as_, imm, offset (BIU family)
                    let imm8 = (w >> 16) & 0xFF;
                    let off = sext8(imm8) + 4;
                    Instruction::Bltui {
                        as_: s,
                        imm: b4constu(r),
                        offset: off,
                    }
                }
                3 => {
                    // BGEUI as_, imm, offset (BIU family)
                    let imm8 = (w >> 16) & 0xFF;
                    let off = sext8(imm8) + 4;
                    Instruction::Bgeui {
                        as_: s,
                        imm: b4constu(r),
                        offset: off,
                    }
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
        0 => -1,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 5,
        6 => 6,
        7 => 7,
        8 => 8,
        9 => 10,
        10 => 12,
        11 => 16,
        12 => 32,
        13 => 64,
        14 => 128,
        15 => 256,
        _ => unreachable!(),
    }
}

/// Look up the B4CONSTU table (ISA RM Appendix B4CONSTU).
///
/// Maps a 4-bit register-field index r to the unsigned immediate constant used
/// by BLTUI/BGEUI.
fn b4constu(r: u8) -> u32 {
    match r & 0xF {
        0 => 32768,
        1 => 65536,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 5,
        6 => 6,
        7 => 7,
        8 => 8,
        9 => 10,
        10 => 12,
        11 => 16,
        12 => 32,
        13 => 64,
        14 => 128,
        15 => 256,
        _ => unreachable!(),
    }
}

impl Instruction {
    /// Returns the highest logical AR register number this instruction reads
    /// or writes. Used by the windowed-register exception model: if accessing
    /// a logical register that aliases to a phys reg owned by a different live
    /// frame, the hardware fires a Window Overflow exception.
    ///
    /// Per Xtensa LX ISA RM §4.7: every instruction's effective register access
    /// must check `WindowStart[(WindowBase + max_reg/4 + 1) MOD 16]`. If set,
    /// raise WindowOverflow with cause based on the rotation distance.
    ///
    /// Returns 0 for instructions that only access fixed low registers (a0,
    /// system regs, etc.) — these never trigger a window overflow.
    pub fn max_logical_reg(&self) -> u8 {
        use Instruction::*;
        match *self {
            // RRR: 3-reg ops
            Add { ar, as_, at }
            | Sub { ar, as_, at }
            | And { ar, as_, at }
            | Or { ar, as_, at }
            | Xor { ar, as_, at }
            | Src { ar, as_, at }
            | Salt { ar, as_, at }
            | Saltu { ar, as_, at }
            | Mull { ar, as_, at }
            | Muluh { ar, as_, at }
            | Mulsh { ar, as_, at }
            | Quos { ar, as_, at }
            | Quou { ar, as_, at }
            | Rems { ar, as_, at }
            | Remu { ar, as_, at }
            | Mul16s { ar, as_, at }
            | Mul16u { ar, as_, at }
            | Min { ar, as_, at }
            | Max { ar, as_, at }
            | Minu { ar, as_, at }
            | Maxu { ar, as_, at }
            | Addx2 { ar, as_, at }
            | Addx4 { ar, as_, at }
            | Addx8 { ar, as_, at }
            | Subx2 { ar, as_, at }
            | Subx4 { ar, as_, at }
            | Subx8 { ar, as_, at } => ar.max(as_).max(at),
            // 2-reg ops
            Neg { ar, at } | Abs { ar, at } | Srl { ar, at } | Sra { ar, at } => ar.max(at),
            Sll { ar, as_ } => ar.max(as_),
            Slli { ar, as_, .. } => ar.max(as_),
            Srli { ar, at, .. } | Srai { ar, at, .. } => ar.max(at),
            // SAR/shift setup — use one source reg
            Ssl { as_ } | Ssr { as_ } | Ssa8l { as_ } | Ssa8b { as_ } => as_,
            Ssai { .. } => 0,
            // ALU + immediate
            Addi { at, as_, .. } | Addmi { at, as_, .. } => at.max(as_),
            Movi { at, .. } => at,
            // Loads/stores
            L8ui { at, as_, .. }
            | L16ui { at, as_, .. }
            | L16si { at, as_, .. }
            | L32i { at, as_, .. }
            | S8i { at, as_, .. }
            | S16i { at, as_, .. }
            | S32i { at, as_, .. }
            | S32c1i { at, as_, .. }
            | L32ai { at, as_, .. }
            | S32ri { at, as_, .. }
            | S32e { at, as_, .. }
            | L32e { at, as_, .. } => at.max(as_),
            L32r { at, .. } => at,
            // Branches
            Beq { as_, at, .. }
            | Bne { as_, at, .. }
            | Blt { as_, at, .. }
            | Bge { as_, at, .. }
            | Bltu { as_, at, .. }
            | Bgeu { as_, at, .. }
            | Bany { as_, at, .. }
            | Ball { as_, at, .. }
            | Bnone { as_, at, .. }
            | Bnall { as_, at, .. }
            | Bbc { as_, at, .. }
            | Bbs { as_, at, .. } => as_.max(at),
            Beqz { as_, .. }
            | Bnez { as_, .. }
            | Bltz { as_, .. }
            | Bgez { as_, .. }
            | Beqi { as_, .. }
            | Bnei { as_, .. }
            | Blti { as_, .. }
            | Bgei { as_, .. }
            | Bltui { as_, .. }
            | Bgeui { as_, .. }
            | Bbci { as_, .. }
            | Bbsi { as_, .. } => as_,
            // Jumps
            J { .. } => 0,
            Jx { as_ } => as_,
            // Calls — CALL{N} and CALLX{N} both write to a[N] (CALLINC*4) of
            // the OLD window. Indirect Callx variants also read a[as_]. The
            // window check should account for the highest reg accessed.
            Call0 { .. } => 0,
            Callx0 { as_ } => as_,
            // CALL4 writes to a4 (logical 4 in caller's window).
            Call4 { .. } => 4,
            Callx4 { as_ } => as_.max(4),
            // CALL8 writes to a8.
            Call8 { .. } => 8,
            Callx8 { as_ } => as_.max(8),
            // CALL12 writes to a12.
            Call12 { .. } => 12,
            Callx12 { as_ } => as_.max(12),
            // Windowed flow
            Entry { as_, .. } => as_,
            Movsp { at, as_ } => at.max(as_),
            Rer { at, as_ } | Wer { at, as_ } => at.max(as_),
            Rotw { .. } => 0,
            Ret => 0,
            Retw => 0,
            Rfwo | Rfwu | Rfe | Rfde => 0,
            Rfi { .. } => 0,
            // Bit/sign manipulation
            Nsa { ar, as_ } | Nsau { ar, as_ } => ar.max(as_),
            Sext { ar, as_, .. } | Clamps { ar, as_, .. } => ar.max(as_),
            // SR/UR access
            Rsr { at, .. } | Wsr { at, .. } | Xsr { at, .. } | Wur { at, .. } => at,
            Rur { ar, .. } => ar,
            // Loop / misc
            Loop { as_, .. } | Loopnez { as_, .. } | Loopgtz { as_, .. } => as_,
            Nop
            | Break { .. }
            | Syscall
            | Waiti { .. }
            | Ill
            | Memw
            | Extw
            | Isync
            | Rsync
            | Esync
            | Dsync => 0,
            Moveqz { ar, as_, at }
            | Movnez { ar, as_, at }
            | Movltz { ar, as_, at }
            | Movgez { ar, as_, at } => ar.max(as_).max(at),
            Rsil { at, .. } => at,
            Extui { ar, at, .. } => ar.max(at),
            // FP ops: the FR file is separate from the AR window, so only the
            // AR operands count toward the window-overflow check.
            AddS { .. }
            | SubS { .. }
            | MulS { .. }
            | MaddS { .. }
            | MsubS { .. }
            | AbsS { .. }
            | NegS { .. }
            | MovS { .. }
            | MoveqzS { .. }
            | MovnezS { .. }
            | MovltzS { .. }
            | MovgezS { .. }
            | MovfS { .. }
            | MovtS { .. }
            | CmpS { .. } => 0,
            Rfr { ar, .. } => ar,
            Wfr { as_, .. } => as_,
            FloatS { as_, .. } | UfloatS { as_, .. } => as_,
            TruncS { ar, .. }
            | UtruncS { ar, .. }
            | RoundS { ar, .. }
            | CeilS { ar, .. }
            | FloorS { ar, .. } => ar,
            Lsi { as_, .. } | Lsiu { as_, .. } | Ssi { as_, .. } | Ssiu { as_, .. } => as_,
            Lsx { as_, at, .. }
            | Lsxu { as_, at, .. }
            | Ssx { as_, at, .. }
            | Ssxu { as_, at, .. } => as_.max(at),
            Unknown(_) => 0,
        }
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self) // adequate for Plan 1; proper disassembly format later
    }
}

#[cfg(test)]
mod salt_saltu_tests {
    use super::{decode, Instruction};

    // Encodings captured from xtensa-esp32s3-elf objdump of a real ESP-IDF
    // image (the firmware that exposed the missing instruction).
    #[test]
    fn decodes_saltu() {
        // saltu a7, a5, a4  → 0x627540   (op0=0, op1=2, op2=6)
        assert_eq!(
            decode(0x627540),
            Instruction::Saltu {
                ar: 7,
                as_: 5,
                at: 4
            }
        );
        // saltu a8, a8, a9  → 0x628890
        assert_eq!(
            decode(0x628890),
            Instruction::Saltu {
                ar: 8,
                as_: 8,
                at: 9
            }
        );
    }

    #[test]
    fn decodes_salt() {
        // salt a2, a9, a8   → 0x722980   (op0=0, op1=2, op2=7)
        assert_eq!(
            decode(0x722980),
            Instruction::Salt {
                ar: 2,
                as_: 9,
                at: 8
            }
        );
    }
}
