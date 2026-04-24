// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Thumb-2 arithmetic helpers shared across the CortexM executor.

/// PSR carry bit position in XPSR. Used by ADC/SBC-style paths that need
/// to fold the prior carry back into the new result.
pub(super) const PSR_C: u32 = 1 << 29;

/// Unsigned 32-bit add returning `(result, carry_out, signed_overflow)`.
#[inline]
pub(super) fn add_with_flags(op1: u32, op2: u32) -> (u32, bool, bool) {
    let (res, carry) = op1.overflowing_add(op2);
    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    let overflow = (neg_op1 == neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

/// Unsigned 32-bit subtract returning `(result, carry_out, signed_overflow)`.
/// Note: ARM uses "carry set on no-borrow"; we invert `overflowing_sub`'s
/// borrow flag to match that convention.
#[inline]
pub(super) fn sub_with_flags(op1: u32, op2: u32) -> (u32, bool, bool) {
    let (res, borrow) = op1.overflowing_sub(op2);
    let carry = !borrow;
    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    let overflow = (neg_op1 != neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

/// ARM Thumb-2 "expand immediate": reconstructs a 32-bit constant from the
/// 12-bit encoding used by T3/T4 data-processing instructions. See
/// ARMv7-M ARM A5.3.2 (ThumbExpandImm).
pub(super) fn thumb_expand_imm(imm12: u32) -> u32 {
    let i = (imm12 >> 11) & 1;
    let imm3 = (imm12 >> 8) & 7;
    let imm8 = imm12 & 0xFF;

    if i == 0 && (imm3 >> 2) == 0 {
        // i:imm3 ∈ {0000, 0001, 0010, 0011} — repetition patterns.
        match imm3 {
            0 => imm8,                                             // ........ ........ ........ abcdefgh
            1 => (imm8 << 16) | imm8,                              // ........ abcdefgh ........ abcdefgh
            2 => (imm8 << 24) | (imm8 << 8),                       // abcdefgh ........ abcdefgh ........
            3 => (imm8 << 24) | (imm8 << 16) | (imm8 << 8) | imm8, // abcdefgh abcdefgh abcdefgh abcdefgh
            _ => unreachable!(),
        }
    } else {
        // Rotated immediate: value is '1' || imm8[6:0], rotated right by i:imm3:imm8[7].
        let val = 0x80 | (imm8 & 0x7F);
        let n = (i << 4) | (imm3 << 1) | (imm8 >> 7);
        val.rotate_right(n)
    }
}
