// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Oracle test suite: ALU + shift oracle bank (15 tests) + mem/branch bank (16 tests).
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

// ═══════════════════════════════════════════════════════════════════════════════
// H6 — Memory (loads/stores/L32R) + branch oracle bank (16 tests)
//
// Data area: a2 = IRAM_BASE + 0x1000  (well inside the 64 KiB oracle IRAM
// window, safely above the program bytes which are a few dozen bytes at most).
//
// All load/store instruction encodings verified with:
//   xtensa-esp32s3-elf-as --no-transform h6.S -o h6.o
//   xtensa-esp32s3-elf-objdump -D h6.o
//   xtensa-esp32s3-elf-objcopy -O binary --only-section=.text h6.o h6.bin
//   od -A x -t x1z h6.bin
//
// IRAM_BASE = 0x4037_0000 (defined in labwired_hw_oracle).
// ═══════════════════════════════════════════════════════════════════════════════

use labwired_hw_oracle::IRAM_BASE;

// Data area base: IRAM_BASE + 0x1000 = 0x4037_1000
const DATA: u32 = IRAM_BASE + 0x1000;

// ── 16. L8UI ──────────────────────────────────────────────────────────────────

/// L8UI a3, a2, 0 — zero-extends byte at mem[a2+0] into a3.
///
/// Encoding (LSAI format, r=0, op0=0x2):
///   L8UI a3, a2, 0: at=3, as_=2, imm8=0 → w = 0x000232
///   Assembler (h6.bin offset 0x00): 32 02 00 → w = 0x000232 ✓
///
/// Test: pre-write 0x000000AB → read byte 0xAB → a3 = 0x000000AB.
#[hw_oracle_test]
fn l8ui_oracle() -> OracleCase {
    // L8UI a3, a2, 0   (w=0x000232 → bytes [0x32, 0x02, 0x00])
    OracleCase::asm(".word 0x000232")
        .setup(|st| {
            st.write_reg("a2", DATA);
            // Low byte 0xAB at DATA; upper bytes 0x00.
            st.write_mem(DATA, 0x0000_00AB);
        })
        .expect(|st| {
            // Zero-extend: only the byte at offset 0 is read.
            st.assert_reg("a3", 0xAB);
        })
}

// ── 17. L16UI ─────────────────────────────────────────────────────────────────

/// L16UI a3, a2, 0 — zero-extends 16-bit halfword at mem[a2+0] into a3.
///
/// Encoding (LSAI format, r=1, op0=0x2):
///   L16UI a3, a2, 0: at=3, as_=2, imm8=0 → w = 0x001232
///   Assembler (h6.bin offset 0x03): 32 12 00 → w = 0x001232 ✓
///
/// Test: pre-write 0x0000CAFE → read u16 = 0xCAFE → a3 = 0x0000CAFE.
#[hw_oracle_test]
fn l16ui_oracle() -> OracleCase {
    // L16UI a3, a2, 0  (w=0x001232 → bytes [0x32, 0x12, 0x00])
    OracleCase::asm(".word 0x001232")
        .setup(|st| {
            st.write_reg("a2", DATA);
            // Low 16 bits 0xCAFE at DATA.
            st.write_mem(DATA, 0x0000_CAFE);
        })
        .expect(|st| {
            // Zero-extend: 16-bit read = 0xCAFE.
            st.assert_reg("a3", 0x0000_CAFE);
        })
}

// ── 18. L16SI ─────────────────────────────────────────────────────────────────

/// L16SI a3, a2, 0 — sign-extends 16-bit halfword at mem[a2+0] into a3.
///
/// Encoding (LSAI format, r=9, op0=0x2):
///   L16SI a3, a2, 0: at=3, as_=2, imm8=0 → w = 0x009232
///   Assembler (h6.bin offset 0x06): 32 92 00 → w = 0x009232 ✓
///
/// Test: pre-write 0x0000FF80 → u16 = 0xFF80 → sign-extend → 0xFFFF_FF80.
#[hw_oracle_test]
fn l16si_oracle() -> OracleCase {
    // L16SI a3, a2, 0  (w=0x009232 → bytes [0x32, 0x92, 0x00])
    OracleCase::asm(".word 0x009232")
        .setup(|st| {
            st.write_reg("a2", DATA);
            // 0xFF80 in low 16 bits (negative in i16, bit 15 set).
            st.write_mem(DATA, 0x0000_FF80);
        })
        .expect(|st| {
            // Sign-extend 0xFF80 → 0xFFFF_FF80.
            st.assert_reg("a3", 0xFFFF_FF80);
        })
}

// ── 19. L32I ──────────────────────────────────────────────────────────────────

/// L32I a3, a2, 0 — loads 32-bit word at mem[a2+0] into a3.
///
/// Encoding (LSAI format, r=2, op0=0x2):
///   L32I a3, a2, 0: at=3, as_=2, imm8=0 → w = 0x002232
///   Assembler (h6.bin offset 0x09): 32 22 00 → w = 0x002232 ✓
///
/// Test: pre-write 0xDEADBEEF → a3 = 0xDEADBEEF.
#[hw_oracle_test]
fn l32i_oracle() -> OracleCase {
    // L32I a3, a2, 0  (w=0x002232 → bytes [0x32, 0x22, 0x00])
    OracleCase::asm(".word 0x002232")
        .setup(|st| {
            st.write_reg("a2", DATA);
            st.write_mem(DATA, 0xDEAD_BEEF);
        })
        .expect(|st| {
            st.assert_reg("a3", 0xDEAD_BEEF);
        })
}

// ── 20. S8I ───────────────────────────────────────────────────────────────────

/// S8I a3, a2, 0 — stores low byte of a3 into mem[a2+0].
///
/// Encoding (LSAI format, r=4, op0=0x2):
///   S8I a3, a2, 0: at=3, as_=2, imm8=0 → w = 0x004232
///   Assembler (h6.bin offset 0x0c): 32 42 00 → w = 0x004232 ✓
///
/// Test: a3 = 0xDEADBEAB, store → mem[DATA] low byte = 0xAB.
/// Read back the full word: upper bytes are zero (DATA pre-cleared by oracle).
#[hw_oracle_test]
fn s8i_oracle() -> OracleCase {
    // S8I a3, a2, 0  (w=0x004232 → bytes [0x32, 0x42, 0x00])
    OracleCase::asm(".word 0x004232")
        .setup(|st| {
            st.write_reg("a2", DATA);
            st.write_reg("a3", 0xDEAD_BEAB); // low byte 0xAB
        })
        .capture_mem(&[DATA])
        .expect(|st| {
            // Only byte 0 is written; RamPeripheral initialised to 0x00.
            st.assert_mem(DATA, 0x0000_00AB);
        })
}

// ── 21. S16I ──────────────────────────────────────────────────────────────────

/// S16I a3, a2, 0 — stores low 16 bits of a3 into mem[a2+0].
///
/// Encoding (LSAI format, r=5, op0=0x2):
///   S16I a3, a2, 0: at=3, as_=2, imm8=0 → w = 0x005232
///   Assembler (h6.bin offset 0x0f): 32 52 00 → w = 0x005232 ✓
///
/// Test: a3 = 0xDEADCAFE, store → mem[DATA] low 16 bits = 0xCAFE.
#[hw_oracle_test]
fn s16i_oracle() -> OracleCase {
    // S16I a3, a2, 0  (w=0x005232 → bytes [0x32, 0x52, 0x00])
    OracleCase::asm(".word 0x005232")
        .setup(|st| {
            st.write_reg("a2", DATA);
            st.write_reg("a3", 0xDEAD_CAFE); // low 16 bits = 0xCAFE
        })
        .capture_mem(&[DATA])
        .expect(|st| {
            // Only 16 bits are written; upper 16 bits of the word remain 0x0000.
            st.assert_mem(DATA, 0x0000_CAFE);
        })
}

// ── 22. S32I ──────────────────────────────────────────────────────────────────

/// S32I a3, a2, 0 — stores full 32-bit a3 into mem[a2+0].
///
/// Encoding (LSAI format, r=6, op0=0x2):
///   S32I a3, a2, 0: at=3, as_=2, imm8=0 → w = 0x006232
///   Assembler (h6.bin offset 0x12): 32 62 00 → w = 0x006232 ✓
///
/// Test: a3 = 0xDEADBEEF → mem[DATA] = 0xDEADBEEF.
#[hw_oracle_test]
fn s32i_oracle() -> OracleCase {
    // S32I a3, a2, 0  (w=0x006232 → bytes [0x32, 0x62, 0x00])
    OracleCase::asm(".word 0x006232")
        .setup(|st| {
            st.write_reg("a2", DATA);
            st.write_reg("a3", 0xDEAD_BEEF);
        })
        .capture_mem(&[DATA])
        .expect(|st| {
            st.assert_mem(DATA, 0xDEAD_BEEF);
        })
}

// ── 23. L32R ──────────────────────────────────────────────────────────────────

/// L32R a3, literal — loads the 32-bit literal from a PC-relative address.
///
/// Program layout (raw bytes; `from_bytes` appends BREAK at the end):
///
///   IRAM_BASE+0x00:  0x06 0x01 0x00   → J +8  (jump to IRAM_BASE+8)
///   IRAM_BASE+0x03:  0x00             → padding (unreachable, literal alignment)
///   IRAM_BASE+0x04:  0xBE 0xBA 0xFE 0xCA  → literal 0xCAFEBABE
///   IRAM_BASE+0x08:  0x31 0xFF 0xFF   → L32R a3, [((pc+3)&~3) - 4]
///   IRAM_BASE+0x0B:  BREAK (auto-appended)
///
/// L32R EA calculation (pc = IRAM_BASE+8):
///   base = (IRAM_BASE+8+3) & ~3 = IRAM_BASE+8  (since IRAM_BASE is 4-byte aligned)
///   imm16 = 0xFFFF (−1 word), pc_rel_byte_offset = −1 × 4 = −4
///   EA = IRAM_BASE+8 − 4 = IRAM_BASE+4 ✓
///
/// J encoding (SI-J, op0=6, n=0, imm18=2 → offset=2*1+4=? No: J offset = sext18(imm18)+4,
/// imm18 in word offset → off = (imm18^0x2_0000 - 0x2_0000)*1? No, for J:
///   imm18 raw at bits[23:6]; off = sext18(imm18) + 4
///   imm18=0x000004 → sext18=4 → offset=4+4=8 ✓.
///   w = (4 << 6) | 0x6 = 0x106 → bytes [0x06, 0x01, 0x00] ✓
///
/// Assembler (h6_l32r.bin):
///   offset 0x00: 06 01 00 → J (w=0x000106)
///   offset 0x03: 00       → padding
///   offset 0x04: be ba fe ca → 0xCAFEBABE
///   offset 0x08: 31 ff ff → L32R a3, 0x4 (w=0xFFFF31)
#[hw_oracle_test]
fn l32r_oracle() -> OracleCase {
    OracleCase::from_bytes(vec![
        // J +8: jump over literal to L32R (at offset 8)
        // J w=0x000106: imm18=4, sext18(4)=4, offset=4+4=8 ✓
        0x06, 0x01, 0x00,
        // padding byte (literal must be 4-byte aligned at offset 4)
        0x00,
        // literal 0xCAFEBABE at offset 4 (4-byte LE)
        0xBE, 0xBA, 0xFE, 0xCA,
        // L32R a3, literal_at_offset_4
        // w=0xFFFF31: at=3, imm16=0xFFFF(−1), pc_rel=-4; EA=IRAM_BASE+8-4=IRAM_BASE+4 ✓
        0x31, 0xFF, 0xFF,
        // BREAK 1,15 (auto-appended by from_bytes after these bytes)
    ])
    .expect(|st| {
        st.assert_reg("a3", 0xCAFE_BABE);
    })
}

// ── 24. BEQ taken ─────────────────────────────────────────────────────────────

/// BEQ a3, a4, taken — taken when a3 == a4; verifies final PC is at taken BREAK.
///
/// Program:
///   offset 0: BEQ a3, a4, +6   → taken branch to offset 6
///   offset 3: BREAK 1,15        → not-taken path (unreachable when taken)
///   offset 6: BREAK 1,15        → taken path (auto-appended by asm())
///
/// BEQ encoding (RI8, op0=7, r=1):
///   w = (imm8<<16) | (r<<12) | (s<<8) | (t<<4) | op0
///   imm8 = offset − 4 = 6 − 4 = 2; r=1, s=3 (a3), t=4 (a4), op0=7
///   w = (0x02<<16)|(0x1<<12)|(0x3<<8)|(0x4<<4)|0x7 = 0x021347
///   Assembler (h6.bin offset 0x15): 47 13 FF → w=0xFF1347 (beq a3,a4,+3).
///   Note: assembler used imm8=0xFF (offset=3) to jump to the next instruction.
///   This test uses imm8=2 (offset=6) to skip the embedded BREAK.
#[hw_oracle_test]
fn beq_taken_oracle() -> OracleCase {
    // BEQ a3, a4, +6  (w=0x021347)
    // BREAK (not-taken path, w=0x0041F0)
    // BREAK (taken path, auto-appended)
    OracleCase::asm(
        ".word 0x021347
         .word 0x0041F0",
    )
    .setup(|st| {
        st.write_reg("a3", 0x42);
        st.write_reg("a4", 0x42); // a3 == a4 → taken
    })
    .expect(|st| {
        // Taken path: PC ends at offset 6 = IRAM_BASE + 6.
        st.assert_pc(IRAM_BASE + 6);
    })
}

// ── 25. BEQ not-taken ─────────────────────────────────────────────────────────

/// BEQ a3, a4, not-taken — not taken when a3 != a4; verifies PC at not-taken BREAK.
///
/// Same program as beq_taken_oracle but a3 != a4.
#[hw_oracle_test]
fn beq_not_taken_oracle() -> OracleCase {
    OracleCase::asm(
        ".word 0x021347
         .word 0x0041F0",
    )
    .setup(|st| {
        st.write_reg("a3", 0x42);
        st.write_reg("a4", 0x99); // a3 != a4 → not taken
    })
    .expect(|st| {
        // Not-taken path: falls through to BREAK at offset 3.
        st.assert_pc(IRAM_BASE + 3);
    })
}

// ── 26. BNE taken ─────────────────────────────────────────────────────────────

/// BNE a3, a4, taken — taken when a3 != a4; verifies PC at taken BREAK.
///
/// BNE encoding (RI8, op0=7, r=9):
///   w = (0x02<<16)|(0x9<<12)|(0x3<<8)|(0x4<<4)|0x7 = 0x029347
///   decode_b: imm8=2, r=9 (BNE), s=3, t=4, offset=6 ✓
#[hw_oracle_test]
fn bne_taken_oracle() -> OracleCase {
    // BNE a3, a4, +6  (w=0x029347)
    // BREAK (not-taken)
    // BREAK (taken, auto-appended)
    OracleCase::asm(
        ".word 0x029347
         .word 0x0041F0",
    )
    .setup(|st| {
        st.write_reg("a3", 0x11);
        st.write_reg("a4", 0x22); // a3 != a4 → taken
    })
    .expect(|st| {
        st.assert_pc(IRAM_BASE + 6);
    })
}

// ── 27. BNE not-taken ─────────────────────────────────────────────────────────

/// BNE a3, a4, not-taken — not taken when a3 == a4; verifies PC at not-taken BREAK.
#[hw_oracle_test]
fn bne_not_taken_oracle() -> OracleCase {
    OracleCase::asm(
        ".word 0x029347
         .word 0x0041F0",
    )
    .setup(|st| {
        st.write_reg("a3", 0x55);
        st.write_reg("a4", 0x55); // a3 == a4 → not taken
    })
    .expect(|st| {
        st.assert_pc(IRAM_BASE + 3);
    })
}

// ── 28. BEQZ ──────────────────────────────────────────────────────────────────

/// BEQZ a3, taken — taken when a3 == 0; verifies PC at taken BREAK.
///
/// BEQZ encoding (BRI12, op0=6, n=1, m=0):
///   imm12 = offset − 4 = 6 − 4 = 2; s=3 (a3)
///   w = (imm12<<12) | (s<<8) | (m<<6) | (n<<4) | op0
///     = (2<<12) | (3<<8) | (0<<6) | (1<<4) | 6
///     = 0x2000 | 0x0300 | 0 | 0x10 | 6
///     = 0x002316
///   decode_si: n=1, m=0 (BEQZ), s=3, imm12=2, off12=6 ✓
#[hw_oracle_test]
fn beqz_oracle() -> OracleCase {
    // BEQZ a3, +6  (w=0x002316)
    // BREAK (not-taken)
    // BREAK (taken, auto-appended)
    OracleCase::asm(
        ".word 0x002316
         .word 0x0041F0",
    )
    .setup(|st| {
        st.write_reg("a3", 0); // a3 == 0 → taken
    })
    .expect(|st| {
        st.assert_pc(IRAM_BASE + 6);
    })
}

// ── 29. BLTUI ─────────────────────────────────────────────────────────────────

/// BLTUI a3, 8, taken — taken when a3 < 8 (unsigned); verifies PC at taken BREAK.
///
/// BLTUI encoding (BIU, op0=6, n=3, m=2):
///   imm8 = offset − 4 = 6 − 4 = 2; s=3 (a3); r=8 (B4CONSTU[8]=8)
///   w = (imm8<<16) | (r<<12) | (s<<8) | (m<<6) | (n<<4) | op0
///     = (2<<16) | (8<<12) | (3<<8) | (2<<6) | (3<<4) | 6
///     = 0x20000 | 0x8000 | 0x0300 | 0x0080 | 0x0030 | 6
///     = 0x0283B6
///   decode_si: n=3, m=2 (BLTUI), s=3, r=8, imm8=2, off=6 ✓
#[hw_oracle_test]
fn bltui_oracle() -> OracleCase {
    // BLTUI a3, 8, +6  (w=0x0283B6)
    // BREAK (not-taken)
    // BREAK (taken, auto-appended)
    OracleCase::asm(
        ".word 0x0283B6
         .word 0x0041F0",
    )
    .setup(|st| {
        st.write_reg("a3", 5); // 5 < 8 (unsigned) → taken
    })
    .expect(|st| {
        st.assert_pc(IRAM_BASE + 6);
    })
}

// ── 30. J ─────────────────────────────────────────────────────────────────────

/// J +6 — unconditional jump; verifies PC lands at the target BREAK.
///
/// J encoding (SI-J, op0=6, n=0):
///   offset = sext18(imm18) + 4 → imm18 = offset − 4 = 6 − 4 = 2
///   w = (imm18<<6) | (n<<4) | op0 = (2<<6) | (0<<4) | 6 = 0x80 | 6 = 0x000086
///   decode_si: n=0 (J), imm18=2, off=sext18(2)+4=6 ✓
#[hw_oracle_test]
fn j_oracle() -> OracleCase {
    // J +6  (w=0x000086)
    // BREAK (unreachable, at offset 3)
    // BREAK (target of J, auto-appended at offset 6)
    OracleCase::asm(
        ".word 0x000086
         .word 0x0041F0",
    )
    .expect(|st| {
        st.assert_pc(IRAM_BASE + 6);
    })
}

// ── 31. CALL0 ─────────────────────────────────────────────────────────────────

/// CALL0 — saves return address in a0 and jumps to target; verifies both.
///
/// CALL0 target formula (ISA RM §4.4):
///   target = ((pc+3) & ~3) + offset
///   a0     = pc + 3  (return address: byte past the 3-byte instruction)
///
/// Program layout (raw bytes; BREAK at offset 4 is the call target):
///   IRAM_BASE+0:  0x45 0x00 0x00  → CALL0 w=0x000045 (offset=4)
///   IRAM_BASE+3:  0x00            → unreachable padding
///   IRAM_BASE+4:  0xF0 0x41 0x00  → BREAK (call target, also auto-appended)
///   (from_bytes appends an additional BREAK but it is never reached)
///
/// CALL0 encoding (CALLN, op0=5, n=0):
///   offset = imm18 * 4; target = (IRAM_BASE+3)&~3 + offset = IRAM_BASE + offset
///   Want target = IRAM_BASE + 4 → offset = 4 → imm18 = 1
///   w = (imm18<<6) | (n<<4) | op0 = (1<<6) | 0 | 5 = 0x40 | 5 = 0x000045
///
/// Assertions:
///   a0 = IRAM_BASE + 3  (return address)
///   pc = IRAM_BASE + 4  (BREAK at target)
#[hw_oracle_test]
fn call0_oracle() -> OracleCase {
    OracleCase::from_bytes(vec![
        // CALL0 target=IRAM_BASE+4, a0=IRAM_BASE+3
        // w=0x000045: op0=5, n=0 (CALL0), imm18=1, offset=1*4=4
        // target = (IRAM_BASE+3)&~3 + 4 = IRAM_BASE + 4 ✓
        0x45, 0x00, 0x00,
        // padding byte at offset 3 (unreachable)
        0x00,
        // BREAK 1,15 at offset 4 (CALL0 jumps here; from_bytes also appends BREAK)
        0xF0, 0x41, 0x00,
    ])
    .expect(|st| {
        st.assert_reg("a0", IRAM_BASE + 3);
        st.assert_pc(IRAM_BASE + 4);
    })
}
