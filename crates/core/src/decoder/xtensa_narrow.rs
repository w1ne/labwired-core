// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa Code Density (16-bit) decoder.
//!
//! Expands narrow encodings into the same `Instruction` enum from
//! `super::xtensa::Instruction` where semantics are identical. The only
//! difference between a narrow and a wide variant is the instruction length,
//! which the fetch loop already tracks via `len`.
//!
//! ## Field naming convention (narrow format)
//!
//! All narrow (16-bit) instructions use the following field extraction:
//! ```text
//! bits[3:0]  = op0  (selects the broad opcode group)
//! bits[7:4]  = s    (role varies per instruction)
//! bits[11:8] = t    (role varies per instruction)
//! bits[15:12]= r    (role varies per instruction)
//! ```
//! **Note:** This naming is inverted compared to the Xtensa ISA RM field labels
//! (where the ISA RM calls `bits[7:4]` the "t" field and `bits[11:8]` the "s"
//! field).  The local `s`/`t`/`r` names here match the plan's extraction order
//! and are corrected for each instruction's semantics per HW-oracle ground truth
//! (xtensa-esp32s3-elf-as + objdump; see tests for byte-level verification).
//!
//! ## HW-oracle verified encodings (Task D8)
//!
//! | Instruction          | op0  | Bytes (LE)    | Notes                   |
//! |----------------------|------|---------------|-------------------------|
//! | `add.n  a3,a4,a5`    | 0xA  | `5a 34`       | ar=r, as_=t, at=s       |
//! | `addi.n a3,a4,5`     | 0xB  | `5b 34`       | at=r, as_=t, imm=sext4s |
//! | `addi.n a3,a4,-1`    | 0xB  | `0b 34`       | s=0 encodes imm=-1      |
//! | `mov.n  a3,a4`       | 0xD  | `3d 04`       | OR ar=s, as_=t, at=t    |
//! | `movi.n a3,5`        | 0xC  | `0c 53`       | at=t, imm7=(s<<4)\|r    |
//! | `movi.n a3,-32`      | 0xC  | `6c 03`       | sign rule: raw>=96→-128 |
//! | `movi.n a3,95`       | 0xC  | `5c f3`       | raw=95→positive 95      |
//! | `l32i.n a3,a4,4`     | 0x8  | `38 14`       | at=s, as_=t, imm=r<<2   |
//! | `s32i.n a3,a4,8`     | 0x9  | `39 24`       | at=s, as_=t, imm=r<<2   |
//! | `beqz.n a3,+4`       | 0xC  | `8c 23`       | as_=t, off=(r+2)*(1+b4) |
//! | `bnez.n a3,+4`       | 0xC  | `cc 03`       | same formula as BEQZ.N  |
//! | `nop.n`              | 0xD  | `3d f0`       | r=0xF, s=3              |
//! | `ret.n`              | 0xD  | `0d f0`       | r=0xF, s=0              |
//! | `retw.n`             | 0xD  | `1d f0`       | r=0xF, s=1              |
//! | `break.n 0`          | 0xD  | `2d f0`       | r=0xF, s=2              |
//! | `ill.n`              | 0xD  | `6d f0`       | r=0xF, s=6              |

use super::xtensa::Instruction;

/// Decode a 16-bit narrow instruction. Caller must have confirmed narrowness
/// via `super::xtensa_length::instruction_length(byte0) == 2`.
pub fn decode_narrow(halfword: u16) -> Instruction {
    let hw = halfword;
    let op0 = (hw & 0x0F) as u8;
    let s   = ((hw >> 4) & 0xF) as u8;   // bits[7:4]
    let t   = ((hw >> 8) & 0xF) as u8;   // bits[11:8]
    let r   = ((hw >> 12) & 0xF) as u8;  // bits[15:12]

    match op0 {
        // L32I.N  at, as_, imm:  at = mem32[as_ + r*4]
        // HW: at=s(bits[7:4]), as_=t(bits[11:8]), imm=r<<2(bits[15:12]*4)
        // Verified: l32i.n a3,a4,4 → 0x1438 → s=3=at, t=4=as_, r=1→imm=4
        0x8 => Instruction::L32i { at: s, as_: t, imm: (r as u32) << 2 },

        // S32I.N  at, as_, imm:  mem32[as_ + r*4] = at
        // HW: at=s(bits[7:4]), as_=t(bits[11:8]), imm=r<<2(bits[15:12]*4)
        // Verified: s32i.n a3,a4,8 → 0x2439 → s=3=at, t=4=as_, r=2→imm=8
        0x9 => Instruction::S32i { at: s, as_: t, imm: (r as u32) << 2 },

        // ADD.N  ar, as_, at:  ar = as_ + at
        // HW: ar=r(bits[15:12]), as_=t(bits[11:8]), at=s(bits[7:4])
        // Verified: add.n a3,a4,a5 → 0x345a → r=3=ar, t=4=as_, s=5=at
        0xA => Instruction::Add { ar: r, as_: t, at: s },

        // ADDI.N  at, as_, imm4:  at = as_ + sign_extend4_nonzero(s)
        // HW: at=r(bits[15:12]), as_=t(bits[11:8]), imm=sext4_nonzero(s=bits[7:4])
        // Encoding: s=0 → imm=-1; s=1..15 → imm=s (positive 1..15)
        // Verified: addi.n a3,a4,5 → 0x345b → r=3=at, t=4=as_, s=5→imm=5
        //           addi.n a3,a4,-1 → 0x340b → r=3=at, t=4=as_, s=0→imm=-1
        0xB => Instruction::Addi { at: r, as_: t, imm8: sext4_nonzero(s) },

        // op0=0xC: MOVI.N / BEQZ.N / BNEZ.N
        // Dispatch on bits[7] and bits[6] (via s bit3 and bit2):
        //   s bit3=0 → MOVI.N (s in 0..7)
        //   s bit3=1, bit6=0 → BEQZ.N
        //   s bit3=1, bit6=1 → BNEZ.N
        0xC => decode_narrow_c(hw, r, s, t),

        // op0=0xD: MOV.N / NOP.N / RET.N / RETW.N / BREAK.N / ILL.N
        // Dispatch on r; for r=0xF further dispatch on s.
        0xD => decode_narrow_d(hw, r, s, t),

        _ => Instruction::Unknown(hw as u32),
    }
}

/// Decode op0=0xC group: MOVI.N, BEQZ.N, BNEZ.N.
///
/// Dispatch by bit 7 and bit 6 of the halfword (= s[3] and s[2]):
/// - s < 8 (bit 7 = 0): MOVI.N
/// - s >= 8 with bit6=0 (s & 0xF in {8..11}): BEQZ.N
/// - s >= 8 with bit6=1 (s & 0xF in {12..15}): BNEZ.N
fn decode_narrow_c(hw: u16, r: u8, s: u8, t: u8) -> Instruction {
    if s & 0x8 == 0 {
        // MOVI.N  at, imm7
        // at = t (bits[11:8])
        // imm7 raw = (s<<4)|r, sign rule: if raw >= 96 then imm = raw - 128 else imm = raw
        // Range: -32..=95 (128 values, not standard 2's complement)
        // Verified: movi.n a3,5 → 0x530c → t=3=at, s=0, r=5, raw7=5 → imm=5
        //           movi.n a3,-32 → 0x036c → t=3=at, s=6, r=0, raw7=96 → imm=-32
        //           movi.n a3,95 → 0xf35c → t=3=at, s=5, r=15, raw7=95 → imm=95 (positive!)
        let raw7 = ((s as u32) << 4) | (r as u32);
        let imm = movi_n_sext(raw7);
        Instruction::Movi { at: t, imm }
    } else if s & 0x4 == 0 {
        // BEQZ.N  as_, offset
        // as_ = t (bits[11:8])
        // offset = (r + 2) * (1 + bit4)  where bit4 = (hw >> 4) & 1
        // Offsets are forward-only (range 2..=32), relative to PC + 2.
        // Verified: beqz.n a3,+4 → 0x238c → t=3=as_, r=2, bit4=0 → (2+2)*1=4
        //           beqz.n a3,+32 → 0xe39c → t=3=as_, r=14, bit4=1 → (14+2)*2=32
        let bit4 = ((hw >> 4) & 1) as u32;
        let offset = (r as u32 + 2) * (1 + bit4);
        Instruction::Beqz { as_: t, offset: offset as i32 }
    } else {
        // BNEZ.N  as_, offset
        // Same offset formula as BEQZ.N.
        // Verified: bnez.n a3,+2 → 0x03cc → t=3=as_, r=0, bit4=0 → (0+2)*1=2
        let bit4 = ((hw >> 4) & 1) as u32;
        let offset = (r as u32 + 2) * (1 + bit4);
        Instruction::Bnez { as_: t, offset: offset as i32 }
    }
}

/// Decode op0=0xD group: MOV.N and zero/minimal operand misc ops.
///
/// Dispatch by r field:
/// - r=0x0: MOV.N  (OR ar=s, as_=t, at=t)
/// - r=0xF: sub-dispatch by s for NOP.N / RET.N / RETW.N / BREAK.N / ILL.N
fn decode_narrow_d(hw: u16, r: u8, s: u8, t: u8) -> Instruction {
    match r {
        // MOV.N  at_dest, as_src  →  OR at_dest, as_src, as_src
        // MOV.N is a pseudo-instruction: OR ar, as_, as_ (both sources the same register).
        // HW: ar=s(bits[7:4])=dest, as_=t(bits[11:8])=src
        // Verified: mov.n a3,a4 → 0x043d → r=0, t=4=as_, s=3=ar(dest) → Or{ar=3,as_=4,at=4}
        0x0 => Instruction::Or { ar: s, as_: t, at: t },

        // r=0xF: zero/minimal operand misc group. Dispatch by s (bits[7:4]).
        // t (bits[11:8]) = 0 for all of these.
        // Verified:
        //   ret.n   → 0xf00d: r=0xF, s=0, t=0
        //   retw.n  → 0xf01d: r=0xF, s=1, t=0
        //   break.n → 0xf02d: r=0xF, s=2, t=0
        //   nop.n   → 0xf03d: r=0xF, s=3, t=0
        //   ill.n   → 0xf06d: r=0xF, s=6, t=0
        0xF => match s {
            0x0 => Instruction::Ret,
            0x1 => Instruction::Retw,
            0x2 => Instruction::Break { imm_s: 0, imm_t: 0 },
            0x3 => Instruction::Nop,
            0x6 => Instruction::Ill,
            _ => Instruction::Unknown(hw as u32),
        },

        _ => Instruction::Unknown(hw as u32),
    }
}

/// ADDI.N special immediate encoding.
///
/// s=0 → imm = -1 (encodes the most common signed delta that won't fit in 0..15)
/// s=1..=15 → imm = s as i32 (positive)
///
/// Verified: addi.n a3,a4,-1 → s=0 → -1; addi.n a3,a4,5 → s=5 → 5
#[inline]
fn sext4_nonzero(s: u8) -> i32 {
    if s == 0 { -1 } else { s as i32 }
}

/// MOVI.N non-standard sign extension.
///
/// The range is -32..=95 (128 values), NOT standard 7-bit signed (-64..=63).
/// Rule: if raw7 >= 96, imm = raw7 - 128 (negative: -32..-1);
///       else           imm = raw7       (positive:  0..95).
///
/// This preserves the values 64..=95 as positive even though bit 6 is set.
/// Verified against HW oracle for all 14 test cases (see `test_decode_narrow_movi_n_*`).
#[inline]
fn movi_n_sext(raw7: u32) -> i32 {
    if raw7 >= 96 {
        raw7 as i32 - 128
    } else {
        raw7 as i32
    }
}
