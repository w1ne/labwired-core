// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! ESP32-C3 / RV32IMC oracle bank — initial slice (15 instruction-level tests).
//!
//! Each `#[riscv_oracle_test]` function expands into three tests:
//!
//! * `*_sim`  — always compiled; runs against the software simulator.
//! * `*_hw`   — gated on `feature = "hw-oracle-c3"`, `#[ignore]`; requires
//!   a USB-JTAG-attached ESP32-C3.
//! * `*_diff` — gated on `feature = "hw-oracle-c3"`, `#[ignore]`; requires
//!   ESP32-C3 board.
//!
//! Run sim only:
//! ```text
//! cargo test -p labwired-hw-oracle --test riscv_oracles
//! ```
//!
//! Run hw / diff (board connected, OpenOCD installed, feature enabled):
//! ```text
//! cargo test -p labwired-hw-oracle --test riscv_oracles --features hw-oracle-c3 -- --ignored
//! ```
//!
//! Encodings are produced by the helpers in `labwired_hw_oracle::riscv`,
//! cross-checked by `mod encoder_tests` in that module against the
//! RISC-V Unprivileged ISA v2.2 spec.  Each test sets up a minimal sequence
//! of `ADDI`-based register loads followed by the instruction under test,
//! then asserts the destination register and (where applicable) the side
//! effect (memory store, branch-taken PC, etc.).

use labwired_hw_oracle::riscv::{
    add, addi, and, andi, beq, bne, div, jal, lui, lw, mul, or, ori, rem, sll, slli, sra, srai,
    srl, srli, sub, sw, xor, xori, RiscVOracleCase, DATA_BASE, PROG_BASE,
};
use labwired_hw_oracle::riscv_oracle_test;

// ── 1. ADDI: x10 = x0 + 0x123 ─────────────────────────────────────────────────
#[riscv_oracle_test]
fn addi_imm12() -> RiscVOracleCase {
    RiscVOracleCase::words(&[addi(10, 0, 0x123)]).expect(|st| {
        st.assert_reg("x10", 0x123);
    })
}

// ── 2. ADD: x12 = x10 + x11 ───────────────────────────────────────────────────
#[riscv_oracle_test]
fn add_reg() -> RiscVOracleCase {
    RiscVOracleCase::words(&[
        addi(10, 0, 0x11),
        addi(11, 0, 0x22),
        add(12, 10, 11),
    ])
    .expect(|st| {
        st.assert_reg("x10", 0x11);
        st.assert_reg("x11", 0x22);
        st.assert_reg("x12", 0x33);
    })
}

// ── 3. SUB: x12 = x10 - x11 (signed wrap) ─────────────────────────────────────
#[riscv_oracle_test]
fn sub_reg_wrap() -> RiscVOracleCase {
    RiscVOracleCase::words(&[
        addi(10, 0, 5),
        addi(11, 0, 7),
        sub(12, 10, 11),
    ])
    .expect(|st| {
        // 5 - 7 = -2 = 0xFFFFFFFE
        st.assert_reg("x12", 0xFFFF_FFFE);
    })
}

// ── 4. AND / OR / XOR via ANDI / ORI / XORI ───────────────────────────────────
#[riscv_oracle_test]
fn logical_imm() -> RiscVOracleCase {
    RiscVOracleCase::words(&[
        addi(10, 0, 0x0F0),     // x10 = 0x0F0
        andi(11, 10, 0x0FF),    // x11 = x10 & 0x0FF = 0x0F0
        ori(12, 10, 0x00F),     // x12 = x10 | 0x00F = 0x0FF
        xori(13, 10, 0x0FF),    // x13 = x10 ^ 0x0FF = 0x00F
    ])
    .expect(|st| {
        st.assert_reg("x11", 0x0F0);
        st.assert_reg("x12", 0x0FF);
        st.assert_reg("x13", 0x00F);
    })
}

// ── 5. AND / OR / XOR (register form) ─────────────────────────────────────────
#[riscv_oracle_test]
fn logical_reg() -> RiscVOracleCase {
    RiscVOracleCase::words(&[
        addi(10, 0, 0x55),
        addi(11, 0, 0x0F), // 0x0F so the sign-extension of imm12 doesn't bite
        and(12, 10, 11),
        or(13, 10, 11),
        xor(14, 10, 11),
    ])
    .expect(|st| {
        st.assert_reg("x12", 0x55 & 0x0F); // 0x05
        st.assert_reg("x13", 0x55 | 0x0F); // 0x5F
        st.assert_reg("x14", 0x55 ^ 0x0F); // 0x5A
    })
}

// ── 6. SLLI / SRLI / SRAI ─────────────────────────────────────────────────────
#[riscv_oracle_test]
fn shifts_immediate() -> RiscVOracleCase {
    RiscVOracleCase::words(&[
        // x10 = 0xFFFF_FF80 (negative)
        addi(10, 0, -128),
        slli(11, 10, 4),  // logical left shift
        srli(12, 10, 4),  // logical right (zero-fill)
        srai(13, 10, 4),  // arithmetic right (sign-extend)
    ])
    .expect(|st| {
        st.assert_reg("x10", 0xFFFF_FF80);
        st.assert_reg("x11", 0xFFFF_F800); // (-128) << 4
        st.assert_reg("x12", 0x0FFF_FFF8); // (u32)(-128) >> 4 = 0x0FFFFFF8
        st.assert_reg("x13", 0xFFFF_FFF8); // -128 >>> 4 = -8 = 0xFFFFFFF8
    })
}

// ── 7. SLL / SRL / SRA (register form) ────────────────────────────────────────
#[riscv_oracle_test]
fn shifts_register() -> RiscVOracleCase {
    RiscVOracleCase::words(&[
        addi(10, 0, 1),
        addi(11, 0, 31),
        sll(12, 10, 11),  // 1 << 31 = 0x8000_0000
        addi(13, 0, -1),
        srl(14, 13, 11),  // 0xFFFFFFFF >> 31 = 1
        sra(15, 13, 11),  // 0xFFFFFFFF >>> 31 = 0xFFFFFFFF
    ])
    .expect(|st| {
        st.assert_reg("x12", 0x8000_0000);
        st.assert_reg("x14", 0x0000_0001);
        st.assert_reg("x15", 0xFFFF_FFFF);
    })
}

// ── 8. LUI + ADDI synthesise a full 32-bit immediate ──────────────────────────
#[riscv_oracle_test]
fn lui_addi_full_imm() -> RiscVOracleCase {
    // Synthesize 0xDEAD_BEEF.  Because ADDI sign-extends, when the low 12
    // bits are >= 0x800 we have to *add 1* to the upper LUI value to
    // compensate.  0xDEAD_BEEF: upper=0xDEAD_C, lower=0xEEF (sign-extends
    // to 0xFFFFFEEF), so LUI 0xDEADC + ADDI 0xEEF lands on 0xDEAD_BEEF.
    RiscVOracleCase::words(&[
        lui(10, 0xDEAD_C000),
        addi(10, 10, 0xEEF - 0x1000), // = -0x111, sign-extended to 0xFFFF_FEEF
    ])
    .expect(|st| {
        st.assert_reg("x10", 0xDEAD_BEEF);
    })
}

// ── 9. SW / LW round-trip via DATA_BASE ───────────────────────────────────────
#[riscv_oracle_test]
fn sw_lw_roundtrip() -> RiscVOracleCase {
    // Build DATA_BASE address (0x3FC8_0000) in x10 via LUI + ADDI.  Low 12
    // bits are 0, so LUI alone is fine.
    let addr = DATA_BASE;
    RiscVOracleCase::words(&[
        lui(10, addr),              // x10 = DATA_BASE
        addi(11, 0, 0x7BC),         // x11 = 0x7BC (12-bit signed range)
        sw(11, 10, 0),              // mem[x10+0] = x11
        lw(12, 10, 0),              // x12 = mem[x10+0]
    ])
    .capture_mem(&[addr])
    .expect(|st| {
        st.assert_reg("x11", 0x7BC);
        st.assert_reg("x12", 0x7BC);
        st.assert_mem(DATA_BASE, 0x7BC);
    })
}

// ── 10. BEQ taken: skip an ADDI ───────────────────────────────────────────────
#[riscv_oracle_test]
fn beq_taken() -> RiscVOracleCase {
    // x10=1; x11=1; if equal, skip the next ADDI.  Immediates must fit in
    // 12 bits signed (-2048..2047), so we use small distinct values that
    // unambiguously prove which branch executed.
    RiscVOracleCase::words(&[
        addi(10, 0, 1),
        addi(11, 0, 1),
        beq(10, 11, 8),         // taken → branch to +8 from this insn
        addi(12, 0, 0x123),     // ← skipped
        addi(12, 0, 0x456),     // ← lands here
    ])
    .expect(|st| {
        st.assert_reg("x12", 0x456);
    })
}

// ── 11. BNE not taken: fall through ───────────────────────────────────────────
#[riscv_oracle_test]
fn bne_not_taken() -> RiscVOracleCase {
    // x10 == x11 → BNE NOT taken → both ADDIs execute.
    RiscVOracleCase::words(&[
        addi(10, 0, 7),
        addi(11, 0, 7),
        bne(10, 11, 8),         // not taken; fall through
        addi(12, 0, 0x123),     // executes (12-bit imm: fits)
        addi(12, 12, 0x010),    // x12 = 0x133
    ])
    .expect(|st| {
        st.assert_reg("x12", 0x133);
    })
}

// ── 12. JAL: link register receives PC+4 ──────────────────────────────────────
#[riscv_oracle_test]
fn jal_link_register() -> RiscVOracleCase {
    // JAL x1, +8 — skip the next ADDI; x1 should hold PC+4 of the JAL.
    // Program layout: insn 0 is addi(10,0,1), insn 1 is the JAL, insn 2 is
    // the skipped addi.  PC of JAL = PROG_BASE + 4.  Link = JAL_PC + 4
    // = PROG_BASE + 8.
    RiscVOracleCase::words(&[
        addi(10, 0, 1),
        jal(1, 8),              // jump over the next two insns
        addi(12, 0, 0x123),     // skipped
        addi(12, 0, 0x456),     // lands here
    ])
    .expect(|st| {
        st.assert_reg("x1", PROG_BASE + 8);
        st.assert_reg("x12", 0x456);
    })
}

// ── 13. MUL: x12 = x10 * x11 ──────────────────────────────────────────────────
#[riscv_oracle_test]
fn mul_basic() -> RiscVOracleCase {
    RiscVOracleCase::words(&[
        addi(10, 0, 0x100),
        addi(11, 0, 0x101),
        mul(12, 10, 11),
    ])
    .expect(|st| {
        // 0x100 * 0x101 = 0x10100
        st.assert_reg("x12", 0x1_0100);
    })
}

// ── 14. DIV: signed division with negative dividend ───────────────────────────
#[riscv_oracle_test]
fn div_signed_negative() -> RiscVOracleCase {
    RiscVOracleCase::words(&[
        addi(10, 0, -100),      // dividend = -100
        addi(11, 0, 7),         // divisor  = 7
        div(12, 10, 11),        // -100 / 7 = -14 (truncates toward zero)
        rem(13, 10, 11),        // -100 % 7 = -2
    ])
    .expect(|st| {
        st.assert_reg("x12", (-14_i32) as u32);
        st.assert_reg("x13", (-2_i32) as u32);
    })
}

// ── 15. x0 stays zero (architectural guarantee) ───────────────────────────────
#[riscv_oracle_test]
fn x0_hardwired_zero() -> RiscVOracleCase {
    // Any write to x0 must be silently discarded.  Both ADDI and the
    // setup closure attempt to clobber x0; the end state must still
    // see x0 == 0.
    RiscVOracleCase::words(&[
        addi(0, 0, 0x7FF),      // tries to set x0 = 0x7FF; must be ignored
        add(10, 0, 0),          // x10 = x0 + x0 = 0
    ])
    .setup(|st| {
        st.write_reg("x0", 0xDEAD_BEEF); // also ignored by run_sim
    })
    .expect(|st| {
        st.assert_reg("x0", 0);
        st.assert_reg("x10", 0);
    })
}
