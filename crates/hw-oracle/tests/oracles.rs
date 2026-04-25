// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Oracle test suite: ALU + shift oracle bank (15 tests).
//!
//! Each `#[hw_oracle_test]` function expands into three tests:
//!
//! * `*_sim`  — always compiled; runs against the software simulator.
//! * `*_hw`   — gated on `feature = "hw-oracle"`, `#[ignore]`; requires ESP32-S3.
//! * `*_diff` — gated on `feature = "hw-oracle"`, `#[ignore]`; requires ESP32-S3.
//!
//! Run sim only:
//! ```
//! cargo test -p labwired-hw-oracle
//! ```
//!
//! Run HW / diff (board must be connected):
//! ```
//! cargo test -p labwired-hw-oracle --features hw-oracle -- --ignored
//! ```
//!
//! All encodings verified with:
//!   xtensa-esp32s3-elf-as --no-transform h5.S -o h5.o
//!   xtensa-esp32s3-elf-objdump -D h5.o

use labwired_hw_oracle::{hw_oracle_test, OracleCase};

// ── 1. ADD ────────────────────────────────────────────────────────────────────

/// ADD a3, a4, a5 — verifies that the ADD instruction computes a3 = a4 + a5.
///
/// Encoding derivation (RRR format):
///   rrr(op2, op1, r, s, t) = (op2<<20)|(op1<<16)|(r<<12)|(s<<8)|(t<<4)
///   ADD ar, as_, at: op2=0x8, op1=0.
///   ADD a3, a4, a5: r=3, s=4, t=5
///     → (0x8<<20)|(0<<16)|(3<<12)|(4<<8)|(5<<4)
///     = 0x800000 | 0x3000 | 0x0400 | 0x0050
///     = 0x803450
///
/// Verified against the decoder (xtensa.rs line 225: `0x8 => Add { ar: r, as_: s, at: t }`)
/// and executor (xtensa_lx7.rs: `Add` arm performs as_ + at → ar).
/// Cross-check: `rrr(0x8, 0, 4, 2, 3)` = ADD a4,a2,a3 used in test_exec_add_movi_break_sequence.
///
/// Note: the plan document cited 0x00008530 which decodes as op2=0,r=8,s=5,t=3
/// (an unrelated opcode), not ADD.  This test uses the verified encoding 0x803450.
#[hw_oracle_test]
fn add_oracle() -> OracleCase {
    // ADD a3, a4, a5
    // Encoding: rrr(op2=0x8, op1=0, r=3, s=4, t=5) = 0x803450
    OracleCase::asm(".word 0x803450")
        .setup(|st| {
            st.write_reg("a4", 0x11);
            st.write_reg("a5", 0x22);
        })
        .expect(|st| {
            st.assert_reg("a3", 0x33);
        })
}

// ── 2. SUB ────────────────────────────────────────────────────────────────────

/// SUB a3, a4, a5 — verifies a3 = a4 - a5.
///
/// Encoding (RRR format, op2=0xC, op1=0):
///   SUB a3, a4, a5: r=3, s=4, t=5 → 0xC03450
///   Assembler confirms: `sub a3, a4, a5` → c03450
#[hw_oracle_test]
fn sub_oracle() -> OracleCase {
    // 0x55 - 0x22 = 0x33
    OracleCase::asm(".word 0xC03450")
        .setup(|st| {
            st.write_reg("a4", 0x55);
            st.write_reg("a5", 0x22);
        })
        .expect(|st| {
            st.assert_reg("a3", 0x33);
        })
}

// ── 3. AND ────────────────────────────────────────────────────────────────────

/// AND a3, a4, a5 — verifies a3 = a4 & a5.
///
/// Encoding (RRR format, op2=0x1, op1=0):
///   AND a3, a4, a5: r=3, s=4, t=5 → 0x103450
///   Assembler confirms: `and a3, a4, a5` → 103450
#[hw_oracle_test]
fn and_oracle() -> OracleCase {
    // 0xF0F0F0F0 & 0x0F0F0F0F = 0x00000000
    OracleCase::asm(".word 0x103450")
        .setup(|st| {
            st.write_reg("a4", 0xF0F0_F0F0);
            st.write_reg("a5", 0x0F0F_0F0F);
        })
        .expect(|st| {
            st.assert_reg("a3", 0x0000_0000);
        })
}

// ── 4. OR ─────────────────────────────────────────────────────────────────────

/// OR a3, a4, a5 — verifies a3 = a4 | a5.
///
/// Encoding (RRR format, op2=0x2, op1=0):
///   OR a3, a4, a5: r=3, s=4, t=5 → 0x203450
///   Assembler confirms: `or a3, a4, a5` → 203450
#[hw_oracle_test]
fn or_oracle() -> OracleCase {
    // 0xF0F0F0F0 | 0x0F0F0F0F = 0xFFFFFFFF
    OracleCase::asm(".word 0x203450")
        .setup(|st| {
            st.write_reg("a4", 0xF0F0_F0F0);
            st.write_reg("a5", 0x0F0F_0F0F);
        })
        .expect(|st| {
            st.assert_reg("a3", 0xFFFF_FFFF);
        })
}

// ── 5. XOR ────────────────────────────────────────────────────────────────────

/// XOR a3, a4, a5 — verifies a3 = a4 ^ a5.
///
/// Encoding (RRR format, op2=0x3, op1=0):
///   XOR a3, a4, a5: r=3, s=4, t=5 → 0x303450
///   Assembler confirms: `xor a3, a4, a5` → 303450
#[hw_oracle_test]
fn xor_oracle() -> OracleCase {
    // 0xAAAA5555 ^ 0xFFFFFFFF = 0x5555AAAA
    OracleCase::asm(".word 0x303450")
        .setup(|st| {
            st.write_reg("a4", 0xAAAA_5555);
            st.write_reg("a5", 0xFFFF_FFFF);
        })
        .expect(|st| {
            st.assert_reg("a3", 0x5555_AAAA);
        })
}

// ── 6. NEG ────────────────────────────────────────────────────────────────────

/// NEG a3, a4 — verifies a3 = 0 - a4 (two's complement negation).
///
/// Encoding (RRR format, op2=0x6, op1=0, r=3, s=0, t=4):
///   neg a3, a4: `neg ar, at` format; assembler → 603040
///   Bytes from assembler: 603040
#[hw_oracle_test]
fn neg_oracle() -> OracleCase {
    // NEG(7) = -7 = 0xFFFFFFF9
    OracleCase::asm(".word 0x603040")
        .setup(|st| {
            st.write_reg("a4", 7);
        })
        .expect(|st| {
            st.assert_reg("a3", 0xFFFF_FFF9);
        })
}

// ── 7. ABS ────────────────────────────────────────────────────────────────────

/// ABS a3, a4 — verifies a3 = |a4| (absolute value).
///
/// Encoding (RRR format, op2=0x6, op1=1, r=3, s=0, t=4):
///   abs a3, a4: assembler → 603140
///   Bytes from assembler: 603140
#[hw_oracle_test]
fn abs_oracle() -> OracleCase {
    // ABS(-5) = ABS(0xFFFFFFFB) = 5
    OracleCase::asm(".word 0x603140")
        .setup(|st| {
            st.write_reg("a4", 0xFFFF_FFFB); // -5 in two's complement
        })
        .expect(|st| {
            st.assert_reg("a3", 5);
        })
}

// ── 8. ADDX2 ──────────────────────────────────────────────────────────────────

/// ADDX2 a3, a4, a5 — verifies a3 = (a4 << 1) + a5.
///
/// Encoding (RRR format, op2=0x9, op1=0):
///   addx2 a3, a4, a5: r=3, s=4, t=5 → 0x903450
///   Assembler confirms: `addx2 a3, a4, a5` → 903450
#[hw_oracle_test]
fn addx2_oracle() -> OracleCase {
    // (0x10 << 1) + 0x05 = 0x20 + 0x05 = 0x25
    OracleCase::asm(".word 0x903450")
        .setup(|st| {
            st.write_reg("a4", 0x10);
            st.write_reg("a5", 0x05);
        })
        .expect(|st| {
            st.assert_reg("a3", 0x25);
        })
}

// ── 9. ADDX4 ──────────────────────────────────────────────────────────────────

/// ADDX4 a3, a4, a5 — verifies a3 = (a4 << 2) + a5.
///
/// Encoding (RRR format, op2=0xA, op1=0):
///   addx4 a3, a4, a5: r=3, s=4, t=5 → 0xA03450
///   Assembler confirms: `addx4 a3, a4, a5` → a03450
#[hw_oracle_test]
fn addx4_oracle() -> OracleCase {
    // (0x10 << 2) + 0x05 = 0x40 + 0x05 = 0x45
    OracleCase::asm(".word 0xA03450")
        .setup(|st| {
            st.write_reg("a4", 0x10);
            st.write_reg("a5", 0x05);
        })
        .expect(|st| {
            st.assert_reg("a3", 0x45);
        })
}

// ── 10. ADDX8 ─────────────────────────────────────────────────────────────────

/// ADDX8 a3, a4, a5 — verifies a3 = (a4 << 3) + a5.
///
/// Encoding (RRR format, op2=0xB, op1=0):
///   addx8 a3, a4, a5: r=3, s=4, t=5 → 0xB03450
///   Assembler confirms: `addx8 a3, a4, a5` → b03450
#[hw_oracle_test]
fn addx8_oracle() -> OracleCase {
    // (0x10 << 3) + 0x05 = 0x80 + 0x05 = 0x85
    OracleCase::asm(".word 0xB03450")
        .setup(|st| {
            st.write_reg("a4", 0x10);
            st.write_reg("a5", 0x05);
        })
        .expect(|st| {
            st.assert_reg("a3", 0x85);
        })
}

// ── 11. SUBX2 ─────────────────────────────────────────────────────────────────

/// SUBX2 a3, a4, a5 — verifies a3 = (a4 << 1) - a5.
///
/// Encoding (RRR format, op2=0xD, op1=0):
///   subx2 a3, a4, a5: r=3, s=4, t=5 → 0xD03450
///   Assembler confirms: `subx2 a3, a4, a5` → d03450
#[hw_oracle_test]
fn subx2_oracle() -> OracleCase {
    // (0x10 << 1) - 0x05 = 0x20 - 0x05 = 0x1B
    OracleCase::asm(".word 0xD03450")
        .setup(|st| {
            st.write_reg("a4", 0x10);
            st.write_reg("a5", 0x05);
        })
        .expect(|st| {
            st.assert_reg("a3", 0x1B);
        })
}

// ── 12. SUBX4 ─────────────────────────────────────────────────────────────────

/// SUBX4 a3, a4, a5 — verifies a3 = (a4 << 2) - a5.
///
/// Encoding (RRR format, op2=0xE, op1=0):
///   subx4 a3, a4, a5: r=3, s=4, t=5 → 0xE03450
///   Assembler confirms: `subx4 a3, a4, a5` → e03450
#[hw_oracle_test]
fn subx4_oracle() -> OracleCase {
    // (0x10 << 2) - 0x05 = 0x40 - 0x05 = 0x3B
    OracleCase::asm(".word 0xE03450")
        .setup(|st| {
            st.write_reg("a4", 0x10);
            st.write_reg("a5", 0x05);
        })
        .expect(|st| {
            st.assert_reg("a3", 0x3B);
        })
}

// ── 13. SUBX8 ─────────────────────────────────────────────────────────────────

/// SUBX8 a3, a4, a5 — verifies a3 = (a4 << 3) - a5.
///
/// Encoding (RRR format, op2=0xF, op1=0):
///   subx8 a3, a4, a5: r=3, s=4, t=5 → 0xF03450
///   Assembler confirms: `subx8 a3, a4, a5` → f03450
#[hw_oracle_test]
fn subx8_oracle() -> OracleCase {
    // (0x10 << 3) - 0x05 = 0x80 - 0x05 = 0x7B
    OracleCase::asm(".word 0xF03450")
        .setup(|st| {
            st.write_reg("a4", 0x10);
            st.write_reg("a5", 0x05);
        })
        .expect(|st| {
            st.assert_reg("a3", 0x7B);
        })
}

// ── 14. SLL ───────────────────────────────────────────────────────────────────

/// SLL a3, a4 — verifies a3 = a4 << (32 - SAR), where SAR is set by SSAI.
///
/// Program (2 instructions):
///   ssai 4      → SAR = 4     (0x404400)
///   sll a3, a4  → a3 = a4 << (32 - 4) = a4 << 28   (0xA13400)
///
/// Encoding verified with assembler (--no-transform):
///   ssai 4     → 404400
///   sll a3, a4 → a13400
///
/// Test: a4 = 0x00000001 → a3 = 0x10000000
#[hw_oracle_test]
fn sll_oracle() -> OracleCase {
    // SSAI 4 sets SAR=4; SLL a3, a4 shifts a4 left by (32-SAR)=28.
    // 0x1 << 28 = 0x10000000
    OracleCase::asm(
        ".word 0x404400
         .word 0xA13400",
    )
    .setup(|st| {
        st.write_reg("a4", 0x0000_0001);
    })
    .expect(|st| {
        st.assert_reg("a3", 0x1000_0000);
    })
}

// ── 15. SRA ───────────────────────────────────────────────────────────────────

/// SRA a3, a5 — verifies a3 = a5 >> SAR (arithmetic, sign-extending), where
/// SAR is set by SSR.
///
/// Program (2 instructions):
///   ssr a4      → SAR = a4 & 0x1F           (0x400400)
///   sra a3, a5  → a3 = (int32_t)a5 >> SAR   (0xB13050)
///
/// Encoding verified with assembler (--no-transform):
///   ssr a4     → 400400
///   sra a3, a5 → b13050
///
/// Test: a4 = 4 (SAR=4), a5 = 0x80000010
///   0x80000010 >> 4 (arithmetic) = 0xF8000001
#[hw_oracle_test]
fn sra_oracle() -> OracleCase {
    // SSR a4 sets SAR = a4 & 0x1F = 4; SRA a3, a5 shifts a5 right by 4 (signed).
    // 0x80000010 >> 4 = 0xF8000001 (sign bit replicated)
    OracleCase::asm(
        ".word 0x400400
         .word 0xB13050",
    )
    .setup(|st| {
        st.write_reg("a4", 4);
        st.write_reg("a5", 0x8000_0010);
    })
    .expect(|st| {
        st.assert_reg("a3", 0xF800_0001);
    })
}
