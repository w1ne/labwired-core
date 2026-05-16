// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! STM32F4 / ARM Cortex-M4F oracle bank — initial slice (15 instruction
//! tests covering Thumb-1 and a few Thumb-2 instructions).
//!
//! Each `#[thumb_oracle_test]` function expands into three tests:
//!
//! * `*_sim`  — always compiled; runs against the software simulator.
//! * `*_hw`   — gated on `feature = "hw-oracle-stm32"`, `#[ignore]`;
//!   requires an SWD-attached STM32 board.
//! * `*_diff` — gated on `feature = "hw-oracle-stm32"`, `#[ignore]`;
//!   runs both and diffs.
//!
//! Run sim only:
//! ```text
//! cargo test -p labwired-hw-oracle --test thumb_oracles
//! ```
//!
//! Run hw / diff (board connected, OpenOCD installed, feature enabled):
//! ```text
//! cargo test -p labwired-hw-oracle --test thumb_oracles --features hw-oracle-stm32 -- --ignored
//! ```
//!
//! Encodings are produced by helpers in `labwired_hw_oracle::arm_thumb`,
//! cross-checked by `mod encoder_tests` against the ARMv7-M Architecture
//! Reference Manual (DDI 0403E.e), Chapter A6 "Thumb instruction set
//! encoding".  16-bit Thumb-1 instructions (most of the bank) follow the
//! T1 encoding tables; 32-bit Thumb-2 (MOV.W, UDIV, SDIV) use the T3 / T1
//! encodings as cited in the relevant helper.

use labwired_hw_oracle::arm_thumb::{
    adds_imm3, adds_imm8, adds_reg, ands, asrs_imm, b_uncond, beq, cmp_reg, eors, ldr_imm5,
    lsls_imm, lsrs_imm, movs_imm8, movw_imm16, muls, orrs, sdiv, str_imm5, subs_reg, udiv,
    ThumbOracleCase, DATA_BASE,
};
use labwired_hw_oracle::thumb_oracle_test;

// ── 1. MOVS Rd, #imm8 ─────────────────────────────────────────────────────────
#[thumb_oracle_test]
fn movs_imm() -> ThumbOracleCase {
    ThumbOracleCase::halfwords(&[movs_imm8(0, 0x42)]).expect(|st| {
        st.assert_reg("r0", 0x42);
    })
}

// ── 2. ADDS Rd, Rn, Rm ────────────────────────────────────────────────────────
#[thumb_oracle_test]
fn adds_reg_basic() -> ThumbOracleCase {
    ThumbOracleCase::halfwords(&[
        movs_imm8(0, 0x11),
        movs_imm8(1, 0x22),
        adds_reg(2, 0, 1),
    ])
    .expect(|st| {
        st.assert_reg("r0", 0x11);
        st.assert_reg("r1", 0x22);
        st.assert_reg("r2", 0x33);
    })
}

// ── 3. SUBS Rd, Rn, Rm (signed wrap) ──────────────────────────────────────────
#[thumb_oracle_test]
fn subs_reg_wrap() -> ThumbOracleCase {
    ThumbOracleCase::halfwords(&[
        movs_imm8(0, 5),
        movs_imm8(1, 7),
        subs_reg(2, 0, 1),
    ])
    .expect(|st| {
        // 5 - 7 = -2 = 0xFFFFFFFE
        st.assert_reg("r2", 0xFFFF_FFFE);
    })
}

// ── 4. ADDS Rd, Rn, #imm3 + ADDS Rd, Rd, #imm8 ────────────────────────────────
#[thumb_oracle_test]
fn adds_immediates() -> ThumbOracleCase {
    ThumbOracleCase::halfwords(&[
        movs_imm8(0, 100),
        adds_imm3(1, 0, 7),  // r1 = r0 + 7 = 107
        adds_imm8(0, 0x80),  // r0 = r0 + 0x80 = 100 + 128 = 228
    ])
    .expect(|st| {
        st.assert_reg("r0", 228);
        st.assert_reg("r1", 107);
    })
}

// ── 5. ANDS / ORRS / EORS ─────────────────────────────────────────────────────
#[thumb_oracle_test]
fn logical_reg() -> ThumbOracleCase {
    ThumbOracleCase::halfwords(&[
        movs_imm8(0, 0x55),
        movs_imm8(1, 0x0F),
        movs_imm8(2, 0x55),
        movs_imm8(3, 0x55),
        ands(0, 1), // r0 = r0 & r1 = 0x05
        orrs(2, 1), // r2 = r2 | r1 = 0x5F
        eors(3, 1), // r3 = r3 ^ r1 = 0x5A
    ])
    .expect(|st| {
        st.assert_reg("r0", 0x05);
        st.assert_reg("r2", 0x5F);
        st.assert_reg("r3", 0x5A);
    })
}

// ── 6. LSLS Rd, Rm, #imm5 ─────────────────────────────────────────────────────
#[thumb_oracle_test]
fn lsls_immediate() -> ThumbOracleCase {
    ThumbOracleCase::halfwords(&[
        movs_imm8(0, 1),
        lsls_imm(1, 0, 31), // r1 = 1 << 31 = 0x8000_0000
    ])
    .expect(|st| {
        st.assert_reg("r1", 0x8000_0000);
    })
}

// ── 7. LSRS Rd, Rm, #imm5 ─────────────────────────────────────────────────────
#[thumb_oracle_test]
fn lsrs_immediate() -> ThumbOracleCase {
    ThumbOracleCase::halfwords(&[
        movs_imm8(0, 0xFF),
        lsrs_imm(1, 0, 4), // r1 = 0xFF >> 4 = 0x0F
    ])
    .expect(|st| {
        st.assert_reg("r1", 0x0F);
    })
}

// ── 8. ASRS Rd, Rm, #imm5 (arithmetic right shift on negative) ────────────────
#[thumb_oracle_test]
fn asrs_negative() -> ThumbOracleCase {
    // r0 = 0xFFFF_FFFF (build by MOVS imm + LSLS).  Simpler: MOVS r0, #0
    // then SUBS r0, r0, #imm — but a no-Rd-Rn-imm SUBS T2 form needs r0
    // as both Rn and Rd.  Easier: synthesise via MOVS r0, #1; LSLS r0, r0, #31
    // to get 0x8000_0000, then ASR by 24 to get 0xFFFF_FF80.  Or just
    // start from 0xFFFFFF80 via MOVW (Thumb-2) — let's do the long way
    // with Thumb-1 only to keep this test 16-bit-only.
    //
    // Use: r0 = 0x80; LSLS r0, r0, #24 → r0 = 0x80000000;
    // r1 = LSLS by 0 (copy); ASRS r2, r1, #1 → r2 = 0xC0000000 (sign-extended).
    ThumbOracleCase::halfwords(&[
        movs_imm8(0, 0x80),
        lsls_imm(0, 0, 24),  // r0 = 0x8000_0000
        asrs_imm(1, 0, 1),    // r1 = (i32)(0x80000000) >> 1 = 0xC0000000
        asrs_imm(2, 0, 31),   // r2 = (i32)(0x80000000) >> 31 = 0xFFFFFFFF
    ])
    .expect(|st| {
        st.assert_reg("r0", 0x8000_0000);
        st.assert_reg("r1", 0xC000_0000);
        st.assert_reg("r2", 0xFFFF_FFFF);
    })
}

// ── 9. MULS Rd, Rm, Rd (Rd = Rm * Rd, two-arg form) ───────────────────────────
#[thumb_oracle_test]
fn muls_basic() -> ThumbOracleCase {
    ThumbOracleCase::halfwords(&[
        movs_imm8(0, 0x12), // r0 = 0x12
        movs_imm8(1, 0x10), // r1 = 0x10
        muls(0, 1),         // r0 = r1 * r0 = 0x10 * 0x12 = 0x120
    ])
    .expect(|st| {
        st.assert_reg("r0", 0x120);
    })
}

// ── 10. CMP + BEQ taken: skip a MOVS ──────────────────────────────────────────
#[thumb_oracle_test]
fn cmp_beq_taken() -> ThumbOracleCase {
    // r0 == r1 → BEQ taken → skip the next MOVS that would clobber r2.
    // Layout (each 16-bit Thumb instruction = 2 bytes from start):
    //   off  insn
    //   +0   movs r0, #7
    //   +2   movs r1, #7
    //   +4   movs r2, #0x55     // sentinel "branch did NOT execute" marker
    //   +6   cmp  r0, r1
    //   +8   beq  +6  (skip the next two MOVS; lands at +14 [the B-self])
    //   +10  movs r2, #0xAA     // skipped if branch taken
    //   +12  movs r2, #0xBB     // skipped if branch taken
    //   +14  (B-self terminator inserted by harness)
    //
    // BEQ offset from-self = 6 → encoder subtracts 4 → imm = +2 halfwords
    // → lands at "+8 + 4 + 2 = +14" per ARM PC+4 semantics.
    ThumbOracleCase::halfwords(&[
        movs_imm8(0, 7),
        movs_imm8(1, 7),
        movs_imm8(2, 0x55),
        cmp_reg(0, 1),
        beq(6),
        movs_imm8(2, 0xAA),
        movs_imm8(2, 0xBB),
    ])
    .expect(|st| {
        // BEQ taken — r2 retains the 0x55 sentinel.
        st.assert_reg("r2", 0x55);
    })
}

// ── 11. STR + LDR round-trip via DATA_BASE ────────────────────────────────────
#[thumb_oracle_test]
fn str_ldr_roundtrip() -> ThumbOracleCase {
    // Build DATA_BASE (0x2000_0000) in r1 using MOVS+LSLS:
    //   r1 = 0x20; r1 = r1 << 24 → r1 = 0x2000_0000
    // r2 = the value to store (0x7A)
    // STR r2, [r1, #0]; LDR r3, [r1, #0]
    let addr = DATA_BASE;
    ThumbOracleCase::halfwords(&[
        movs_imm8(1, 0x20),
        lsls_imm(1, 1, 24),
        movs_imm8(2, 0x7A),
        str_imm5(2, 1, 0),  // mem[r1] = r2
        ldr_imm5(3, 1, 0),  // r3 = mem[r1]
    ])
    .capture_mem(&[addr])
    .expect(|st| {
        st.assert_reg("r2", 0x7A);
        st.assert_reg("r3", 0x7A);
        st.assert_mem(DATA_BASE, 0x7A);
    })
}

// ── 12. B unconditional forward: skip a MOVS ──────────────────────────────────
#[thumb_oracle_test]
fn b_uncond_forward() -> ThumbOracleCase {
    // Layout (per instruction):
    //   +0  movs r0, #0x11
    //   +2  b    +6  (skip the next instruction, land on second MOVS)
    //   +4  movs r0, #0x22  (skipped)
    //   +6  movs r0, #0x33  (executes)
    //   +8  (B-self terminator)
    ThumbOracleCase::halfwords(&[
        movs_imm8(0, 0x11),
        b_uncond(4), // skip the next instruction; b_uncond's arg is offset from-self
        movs_imm8(0, 0x22),
        movs_imm8(0, 0x33),
    ])
    .expect(|st| {
        st.assert_reg("r0", 0x33);
    })
}

// ── 13. MOV.W Rd, #imm16 (Thumb-2 T3 encoding) ────────────────────────────────
#[thumb_oracle_test]
fn movw_imm16_thumb2() -> ThumbOracleCase {
    // MOV.W r0, #0xBEEF — load a 16-bit immediate that wouldn't fit in
    // any Thumb-1 form.  This exercises the 32-bit Thumb-2 decoder path
    // and the runner's halfword-pair emission ordering.
    ThumbOracleCase::t2_words(&[movw_imm16(0, 0xBEEF)]).expect(|st| {
        st.assert_reg("r0", 0xBEEF);
    })
}

// ── 14. UDIV Rd, Rn, Rm (Thumb-2 T1) ──────────────────────────────────────────
#[thumb_oracle_test]
fn udiv_basic() -> ThumbOracleCase {
    // r0 / r1 = 100 / 7 = 14 (unsigned).
    // Setup uses the setup closure rather than asm-MOVS so each operand
    // can be > 255 in a future variant.
    ThumbOracleCase::t2_words(&[udiv(2, 0, 1)])
        .setup(|st| {
            st.write_reg("r0", 100);
            st.write_reg("r1", 7);
        })
        .expect(|st| {
            st.assert_reg("r2", 14);
        })
}

// ── 15. SDIV Rd, Rn, Rm (Thumb-2 T1, signed with negative dividend) ───────────
#[thumb_oracle_test]
fn sdiv_signed_negative() -> ThumbOracleCase {
    // r0 / r1 = -100 / 7 = -14 (truncates toward zero per ARM SDIV spec).
    ThumbOracleCase::t2_words(&[sdiv(2, 0, 1)])
        .setup(|st| {
            st.write_reg("r0", (-100_i32) as u32);
            st.write_reg("r1", 7);
        })
        .expect(|st| {
            st.assert_reg("r2", (-14_i32) as u32);
        })
}
