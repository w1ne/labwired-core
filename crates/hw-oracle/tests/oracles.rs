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

// Data area base for 32-bit (word) load/store tests: IRAM_BASE + 0x1000.
// The IRAM alias (0x4037_xxxx) supports only 32-bit data access from the CPU
// load/store unit; byte/halfword accesses to this window silently fail on the
// ESP32-S3 I-bus.  32-bit tests (L32I, S32I, S32E) use this address.
const DATA: u32 = IRAM_BASE + 0x1000;

// Data area base for sub-word (byte/halfword) load/store tests.
// The DRAM alias (0x3FC8_xxxx) is the same physical SRAM but accessed via the
// D-bus, which supports byte and halfword operations.  IRAM_BASE maps to
// DRAM_BASE: 0x4037_0000 ↔ 0x3FC8_8000.  So IRAM_BASE+0x1000 = 0x3FC8_9000.
const DATA_DRAM: u32 = 0x3FC8_9000;

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
    // Use DATA_DRAM (0x3FC89000) — the DRAM alias — because L8UI is a byte
    // load and the IRAM window (0x40370000) does not support sub-word access.
    OracleCase::asm(".word 0x000232")
        .setup(|st| {
            st.write_reg("a2", DATA_DRAM);
            // Low byte 0xAB at DATA_DRAM; upper bytes 0x00.
            st.write_mem(DATA_DRAM, 0x0000_00AB);
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
    // Use DATA_DRAM: halfword load requires DRAM alias.
    OracleCase::asm(".word 0x001232")
        .setup(|st| {
            st.write_reg("a2", DATA_DRAM);
            // Low 16 bits 0xCAFE at DATA_DRAM.
            st.write_mem(DATA_DRAM, 0x0000_CAFE);
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
    // Use DATA_DRAM: halfword load requires DRAM alias.
    OracleCase::asm(".word 0x009232")
        .setup(|st| {
            st.write_reg("a2", DATA_DRAM);
            // 0xFF80 in low 16 bits (negative in i16, bit 15 set).
            st.write_mem(DATA_DRAM, 0x0000_FF80);
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
    // Use DATA_DRAM: byte store requires DRAM alias.
    OracleCase::asm(".word 0x004232")
        .setup(|st| {
            st.write_reg("a2", DATA_DRAM);
            st.write_reg("a3", 0xDEAD_BEAB); // low byte 0xAB
        })
        .capture_mem(&[DATA_DRAM])
        .expect(|st| {
            // Only byte 0 is written; memory initialised to 0x00.
            st.assert_mem(DATA_DRAM, 0x0000_00AB);
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
    // Use DATA_DRAM: halfword store requires DRAM alias.
    OracleCase::asm(".word 0x005232")
        .setup(|st| {
            st.write_reg("a2", DATA_DRAM);
            st.write_reg("a3", 0xDEAD_CAFE); // low 16 bits = 0xCAFE
        })
        .capture_mem(&[DATA_DRAM])
        .expect(|st| {
            // Only 16 bits are written; upper 16 bits of the word remain 0x0000.
            st.assert_mem(DATA_DRAM, 0x0000_CAFE);
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

// ═══════════════════════════════════════════════════════════════════════════════
// H7 — Windowing oracle bank (7 tests + 1 deferred)
//
// Tests the windowed register ABI: CALL4 / ENTRY / RETW (overflow and underflow),
// S32E inside exception context, ROTW, and MOVSP safe path.
//
// All instruction encodings are 24-bit LE words written via parse_dot_word or
// from_bytes. The encoding convention is: word bits[3:0] = op0, and the word
// is stored LE (byte0 = low byte of word at the lowest address).
//
// Windowing instruction encodings (verified against xtensa_exec.rs unit tests):
//   CALL4  imm18=2 (target +8 from 4-aligned base): w=0x000095 bytes=[0x95,0x00,0x00]
//   CALL4  imm18=1 (target +4 from 4-aligned base): w=0x000055 bytes=[0x55,0x00,0x00]
//   ENTRY a1, 32:  w=0x004136 bytes=[0x36,0x41,0x00]
//   RETW:          w=0x000090 bytes=[0x90,0x00,0x00]
//   S32E a3,a4,-16: w=0x30C449 bytes=[0x49,0xC4,0x30]  (PS.EXCM-gated)
//   ROTW 1:        w=0x408010 bytes=[0x10,0x80,0x40]
//   MOVSP a3,a4:   w=0x001430 bytes=[0x30,0x14,0x00]
//   BREAK 1,15:    w=0x0041F0 bytes=[0xF0,0x41,0x00]
//
// VECBASE_SR = 231 (0xE7), EPC1_SR = 177 (0xB1).
// For overflow/underflow tests, VECBASE is set to IRAM_BASE+0x800 so that the
// window vectors fall within the 64 KiB oracle IRAM region.
//   OF4 vector:  VECBASE + 0x000 = IRAM_BASE+0x800
//   UF4 vector:  VECBASE + 0x040 = IRAM_BASE+0x840
//
// Deferred:
//   H7.6 (S32E outside vector) — The simulator decodes op0=9 bytes as the
//   narrow S32I.N instruction when PS.EXCM=0, not as S32E. Therefore no
//   IllegalInstruction exception is raised by the simulator (this matches the
//   documented design intent in xtensa_lx7.rs which explicitly avoids the wide
//   decode outside EXCM). HW-side IllegalInstruction behavior is architecturally
//   correct but not modelled in the Plan-1 sim.
//
// All encodings verified consistent with:
//   crates/core/tests/xtensa_exec.rs test_exec_s32e_hw_oracle_bytes
//   crates/core/tests/xtensa_decode.rs test_decode_s32e / test_decode_rotw / ...
// ═══════════════════════════════════════════════════════════════════════════════

// VECBASE SR ID (labwired_core::cpu::xtensa_sr::VECBASE = 231).
const VECBASE_SR: u16 = 231;

// ── H7.1: CALL4 + ENTRY + RETW (no overflow, no underflow) ───────────────────

/// CALL4 + ENTRY + RETW round-trip without window overflow or underflow.
///
/// Program layout:
///   IRAM_BASE+0:  CALL4 (imm18=2, target=IRAM_BASE+8)  [0x95,0x00,0x00]
///   IRAM_BASE+3:  BREAK                                  [0xF0,0x41,0x00] ← RETW returns here
///   IRAM_BASE+6:  padding                               [0x00,0x00]
///   IRAM_BASE+8:  ENTRY a1, 32                           [0x36,0x41,0x00]
///   IRAM_BASE+11: RETW                                   [0x90,0x00,0x00]
///   (BREAK auto-appended at IRAM_BASE+14 — unreachable)
///
/// CALL4 sets CALLINC=1, stores return address in logical a4 of the old frame.
/// ENTRY rotates WB to 1, sets WS[1].
/// RETW reads N=1 from a0[31:30], verifies WS[0]=1 (no UF), returns to IRAM_BASE+3.
/// BREAK fires at IRAM_BASE+3. After RETW: WB=0, WS[1] cleared.
#[hw_oracle_test]
fn call4_entry_retw_no_overflow() -> OracleCase {
    OracleCase::from_bytes(vec![
        // IRAM_BASE+0: CALL4 (imm18=2, target=IRAM_BASE+8)
        // w=0x000095: op0=5(CALLN), n=1(CALL4), imm18=2, off=8
        // target = ((IRAM_BASE+3)&~3) + 8 = IRAM_BASE + 8
        0x95, 0x00, 0x00,
        // IRAM_BASE+3: BREAK — RETW returns here (CALL4 return addr = IRAM_BASE+3)
        0xF0, 0x41, 0x00,
        // IRAM_BASE+6: padding (unreachable)
        0x00, 0x00,
        // IRAM_BASE+8: ENTRY a1, 32  (w=0x004136)
        0x36, 0x41, 0x00,
        // IRAM_BASE+11: RETW  (w=0x000090)
        0x90, 0x00, 0x00,
        // (BREAK auto-appended by from_bytes at IRAM_BASE+14 — unreachable)
    ])
    .expect(|st| {
        // RETW returned to IRAM_BASE+3 where BREAK is.
        st.assert_pc(IRAM_BASE + 3);
        // WindowBase restored to 0 after RETW.
        st.assert_windowbase(0);
        // WS[1] was cleared by RETW; bit 0 remains (initial reset value).
        st.assert_windowstart(0x0001);
    })
}

// ── H7.2: ENTRY triggers Window Overflow (OF4) ───────────────────────────────

/// CALL4 → ENTRY with WS[2]=1 triggers OF4; PC redirects to VECBASE+0x000.
///
/// Setup: WindowStart = 0x0005 (bits 0 and 2 set).
///   After CALL4 (CALLINC=1), ENTRY computes WB_new = 0+1 = 1.
///   Overflow check: WS[(1+1)%16] = WS[2] = 1 → Window Overflow 4!
///   EPC1 = ENTRY PC (IRAM_BASE+8). PS.EXCM = 1.
///   PC = VECBASE + 0x000 = (IRAM_BASE+0x800) + 0x000 = IRAM_BASE+0x800.
///
/// VECBASE is set to IRAM_BASE+0x800 so the OF4 vector lands inside the
/// 64 KiB oracle IRAM, where we place a BREAK to halt cleanly.
///
/// Program layout:
///   IRAM_BASE+0:     CALL4 (imm18=2, target=IRAM_BASE+8)  [0x95,0x00,0x00]
///   IRAM_BASE+3..7:  zeros (unreachable)
///   IRAM_BASE+8:     ENTRY a1, 32                          [0x36,0x41,0x00]
///   IRAM_BASE+0x800: BREAK  ← OF4 vector (VECBASE+0x000)  [0xF0,0x41,0x00]
#[hw_oracle_test]
fn entry_window_overflow_of4() -> OracleCase {
    let mut prog = vec![0u8; 0x803 + 3]; // space for BREAK at 0x800 plus 3 bytes
    // CALL4 (imm18=2, target=IRAM_BASE+8)
    prog[0..3].copy_from_slice(&[0x95, 0x00, 0x00]);
    // ENTRY a1, 32 at offset 8
    prog[8..11].copy_from_slice(&[0x36, 0x41, 0x00]);
    // BREAK at IRAM_BASE+0x800 (OF4 vector = VECBASE+0x000 when VECBASE=IRAM_BASE+0x800)
    prog[0x800..0x803].copy_from_slice(&[0xF0, 0x41, 0x00]);
    OracleCase::from_bytes(prog)
        .setup(|st| {
            // Set VECBASE to IRAM_BASE+0x800 so OF4 vector is inside oracle IRAM.
            st.write_sr(VECBASE_SR, IRAM_BASE + 0x800);
            // WS bits 0 and 2 set: WS[2]=1 causes overflow when WB_new=1.
            st.write_windowstart(0x0005);
        })
        .expect(|st| {
            // OF4 redirected PC to VECBASE+0x000 = IRAM_BASE+0x800 where BREAK is.
            st.assert_pc(IRAM_BASE + 0x800);
            // EPC1 holds the faulting ENTRY's address.
            st.assert_epc1(IRAM_BASE + 8);
            // PS.EXCM was set to 1 on overflow entry.
            st.assert_excm(true);
            // WindowBase was NOT rotated by the overflow (stays at 0).
            st.assert_windowbase(0);
        })
}

// ── H7.3: Nested 2-level CALL4 → ENTRY → CALL4 → ENTRY (no overflow) ────────

/// Two successive CALL4→ENTRY pairs; WindowBase ends at 2, WindowStart = 0x0007.
///
/// Program layout:
///   IRAM_BASE+0:  CALL4 (imm18=1, target=IRAM_BASE+4)   [0x55,0x00,0x00]
///   IRAM_BASE+3:  padding                                [0x00]
///   IRAM_BASE+4:  ENTRY a1, 32                           [0x36,0x41,0x00]
///   IRAM_BASE+7:  CALL4 (imm18=1, target=IRAM_BASE+12)  [0x55,0x00,0x00]
///   IRAM_BASE+10: padding                                [0x00,0x00]
///   IRAM_BASE+12: ENTRY a1, 32                           [0x36,0x41,0x00]
///   IRAM_BASE+15: BREAK  (auto-appended by from_bytes)
///
/// CALL4 at IRAM_BASE+7: ((IRAM_BASE+7+3)&~3) = IRAM_BASE+8; +4 → target=IRAM_BASE+12. ✓
/// After 2 ENTRY executions: WB=2, WS=0x0007 (bits 0,1,2 all set).
#[hw_oracle_test]
fn nested_2level_call_no_overflow() -> OracleCase {
    OracleCase::from_bytes(vec![
        // IRAM_BASE+0: CALL4 (imm18=1, target=IRAM_BASE+4)
        0x55, 0x00, 0x00,
        // IRAM_BASE+3: padding
        0x00,
        // IRAM_BASE+4: ENTRY a1, 32
        0x36, 0x41, 0x00,
        // IRAM_BASE+7: CALL4 (imm18=1, target=IRAM_BASE+12)
        // base = ((IRAM_BASE+10)&~3) = IRAM_BASE+8; +4 → IRAM_BASE+12 ✓
        0x55, 0x00, 0x00,
        // IRAM_BASE+10: padding
        0x00, 0x00,
        // IRAM_BASE+12: ENTRY a1, 32
        0x36, 0x41, 0x00,
        // IRAM_BASE+15: BREAK (auto-appended)
    ])
    .expect(|st| {
        // BREAK fires at IRAM_BASE+15.
        st.assert_pc(IRAM_BASE + 15);
        // Two ENTRY calls advanced WB by 1 each: WB = 0 + 1 + 1 = 2.
        st.assert_windowbase(2);
        // WS bits 0, 1, 2 are all set (initial + 2 ENTRY calls).
        st.assert_windowstart(0x0007);
    })
}

// ── H7.4: RETW triggers Window Underflow (UF4) ───────────────────────────────

/// RETW with WS[wb_dest]=0 triggers UF4; PC redirects to VECBASE+0x040.
///
/// Setup: WB=1, WindowStart=0x0002 (only bit 1 set; bit 0 clear → UF on N=1 return).
///   a0 in the current frame (WB=1) = logical a4 in WB=0 = phys[4].
///   Set via write_reg("a4", 0x40370003): N = a0[31:30] = 01 → N=1 (CALL4 style).
///   wb_dest = 1 - 1 = 0. WS[0] = 0 → Window Underflow 4!
///   EPC1 = RETW PC (IRAM_BASE+0). PS.EXCM = 1.
///   PC = VECBASE + 0x040 = (IRAM_BASE+0x800) + 0x040 = IRAM_BASE+0x840.
///
/// The return address value 0x40370003 has bits[31:30]=01, giving N=1.
/// It doesn't matter what the low 30 bits are since RETW faults before using them.
///
/// Program layout:
///   IRAM_BASE+0:     RETW  ← triggers UF4             [0x90,0x00,0x00]
///   IRAM_BASE+0x840: BREAK ← UF4 vector (VECBASE+0x040) [0xF0,0x41,0x00]
#[hw_oracle_test]
fn retw_window_underflow_uf4() -> OracleCase {
    let mut prog = vec![0u8; 0x843 + 3];
    // RETW at offset 0
    prog[0..3].copy_from_slice(&[0x90, 0x00, 0x00]);
    // BREAK at IRAM_BASE+0x840 (UF4 vector = VECBASE+0x040 when VECBASE=IRAM_BASE+0x800)
    prog[0x840..0x843].copy_from_slice(&[0xF0, 0x41, 0x00]);
    OracleCase::from_bytes(prog)
        .setup(|st| {
            // VECBASE → IRAM_BASE+0x800; UF4 vector = VECBASE+0x040 = IRAM_BASE+0x840.
            st.write_sr(VECBASE_SR, IRAM_BASE + 0x800);
            // WindowBase = 1 (callee frame).
            st.write_windowbase(1);
            // WindowStart = 0x0002: bit 1 set (callee), bit 0 CLEAR (caller → UF!).
            st.write_windowstart(0x0002);
            // a4 in WB=0 = phys[4] = logical a0 in WB=1 = callee's a0.
            // Bits[31:30] = 01 → N=1 (CALL4-style return).
            // 0x40370003 = IRAM_BASE + 3, bits[31:30] = 0x40000000 >> 30 = 1. ✓
            st.write_reg("a4", 0x4037_0003);
        })
        .expect(|st| {
            // UF4 redirected PC to VECBASE+0x040 = IRAM_BASE+0x840 where BREAK is.
            st.assert_pc(IRAM_BASE + 0x840);
            // EPC1 holds the faulting RETW's address.
            st.assert_epc1(IRAM_BASE);
            // PS.EXCM was set to 1 on underflow entry.
            st.assert_excm(true);
            // WindowBase was NOT rotated by the underflow (stays at 1).
            st.assert_windowbase(1);
        })
}

// ── H7.5: S32E inside exception vector (PS.EXCM=1) ───────────────────────────

/// S32E executes correctly when PS.EXCM=1; stores a3 to [a4 - 16].
///
/// S32E is only recognized as a 3-byte wide instruction when PS.EXCM=1 and
/// byte0 & 0xF = 9.  The capture_hw_state harness resets PS to a clean
/// baseline (WOE=1, EXCM=0, INTLEVEL=0) before every test, so this test
/// must explicitly set EXCM=1 in its setup closure.
///
/// Encoding: S32E a3, a4, -16  → w=0x30C449
///   byte0=0x49 (bits[3:0]=9=op0, bits[7:4]=4=subop→S32E), len=2(narrow), but
///   step() detects EXCM=1 + op0=9 → re-reads as 3-byte wide instruction ✓
///
/// Program:
///   IRAM_BASE+0: S32E a3, a4, -16  (w=0x30C449, bytes=[0x49,0xC4,0x30])
///   IRAM_BASE+3: BREAK (auto-appended)
///
/// Setup: a4 = IRAM_BASE+0x1000 (DATA), a3 = 0xDEAD_BEEF.
/// Expected: mem[IRAM_BASE+0xFF0] = 0xDEADBEEF (a4 - 16 = IRAM_BASE+0xFF0).
#[hw_oracle_test]
fn s32e_inside_vector() -> OracleCase {
    // S32E a3, a4, -16: w=0x30C449 → bytes [0x49, 0xC4, 0x30]
    let data_addr = IRAM_BASE + 0x1000;
    let ea = data_addr - 16; // = IRAM_BASE + 0xFF0
    OracleCase::asm(".word 0x30C449")
        .setup(move |st| {
            // Explicitly set PS.EXCM=1: required for S32E to decode as a
            // 3-byte wide instruction rather than the narrow S32I.N form.
            st.write_ps_excm(true);
            st.write_reg("a4", data_addr);
            st.write_reg("a3", 0xDEAD_BEEF);
        })
        .capture_mem(&[ea])
        .expect(move |st| {
            st.assert_mem(ea, 0xDEAD_BEEF);
        })
}

// ── H7.6: S32E outside exception vector — DEFERRED ───────────────────────────
//
// Plan: PS.EXCM=0, run S32E bytes, expect IllegalInstruction (cause=0).
// Status: DEFERRED — the simulator's step() function intentionally treats
// op0=9 bytes as the narrow S32I.N instruction when PS.EXCM=0. The EXCM
// gate exists to avoid false-decoding S32I.N as S32E. Therefore, the
// IllegalInstruction exception is NOT raised by the simulator when EXCM=0.
// HW raises IllegalInstruction; simulator silently executes S32I.N instead.
// A future plan should add a distinct "outside-EXCM S32E" detection path
// that raises IllegalInstruction, or document the divergence explicitly.

// ── H7.7: ROTW 1 — rotate WindowBase by +1 ───────────────────────────────────

/// ROTW 1: WindowBase = (WindowBase + 1) mod 16; WindowStart unchanged.
///
/// Encoding: ROTW 1  → w=0x408010
///   op0=0(QRST), op1=0, op2=4, r=8(ROTW), t=1(n=+1)
///
/// Program:
///   IRAM_BASE+0: ROTW 1  (w=0x408010)
///   IRAM_BASE+3: BREAK (auto-appended)
///
/// Setup: WB=0 (reset default). After ROTW 1: WB=1. WS unchanged (=0x0001).
#[hw_oracle_test]
fn rotw_1() -> OracleCase {
    // ROTW 1: w=0x408010
    OracleCase::asm(".word 0x408010")
        .expect(|st| {
            // WindowBase advanced by 1 from reset value of 0.
            st.assert_windowbase(1);
            // WindowStart is NOT modified by ROTW.
            st.assert_windowstart(0x0001);
        })
}

// ── H7.8: MOVSP safe path (adjacent frame NOT live) ──────────────────────────

/// MOVSP at, as_: copies a4 into a3 when WS[(WB+1)%16] == 0 (safe path).
///
/// Encoding: MOVSP a3, a4  → w=0x001430
///   op0=0(QRST), op1=0, op2=0 (ST0 group), r=1 (MOVSP), s=as_=4, t=at=3
///
/// Program:
///   IRAM_BASE+0: MOVSP a3, a4  (w=0x001430)
///   IRAM_BASE+3: BREAK (auto-appended)
///
/// Setup: WB=0, WS=0x0001 (only bit 0 set). WS[(0+1)%16] = WS[1] = 0 → safe path.
/// a4 = 0xCAFE_BABE. After MOVSP: a3 = 0xCAFEBABE.
///
/// Note: the reset WS=0x0001 already satisfies the safe-path condition (WS[1]=0),
/// so no explicit write_windowstart is needed.
#[hw_oracle_test]
fn movsp_safe_path() -> OracleCase {
    // MOVSP a3, a4: w=0x001430
    OracleCase::asm(".word 0x001430")
        .setup(|st| {
            st.write_reg("a4", 0xCAFE_BABE);
        })
        .expect(|st| {
            st.assert_reg("a3", 0xCAFE_BABE);
        })
}

// ═══════════════════════════════════════════════════════════════════════════════
// H8 — Exception / Interrupt oracle bank (6 tests)
//
// Verifies:
//   - IllegalInstruction (EXCCAUSE=0) on unknown opcode.
//   - EPC1/EXCCAUSE readback after exception entry.
//   - RFE: clears PS.EXCM, restores PC from EPC1.
//   - Level-1 interrupt dispatch (INTENABLE+INTERRUPT → VECBASE+0x300).
//   - RFI 2: restores full PS from EPS2, PC from EPC2.
//   - VECBASE relocation: exception vector redirect to new VECBASE+0x300.
//
// Instruction encodings used (all verified in xtensa_exec.rs G2 tests):
//   Unknown opcode 0x008530  (bytes [0x00, 0x85, 0x30]): r=8 in ST0 → Unknown → EXCCAUSE=0
//   Unknown opcode 0x008540  (bytes [0x00, 0x85, 0x40]): another Unknown in ST0
//   RFE   (0x003000, bytes [0x00, 0x30, 0x00]): r=3, s=0, t=0 → Rfe
//   RFI 2 (0x003210, bytes [0x10, 0x32, 0x00]): r=3, s=2, t=1 → Rfi{level:2}
//   BREAK (0x0041F0, bytes [0xF0, 0x41, 0x00]): halt sentinel
//
// ExceptionRaised halt semantics (hw-oracle lib.rs):
//   When raise_general_exception() fires, end.pc = cpu.get_pc() = VECBASE+0x300
//   (the kernel exception vector redirect). end.epc1 = faulting PC.
//
// Interrupt dispatch halt semantics:
//   dispatch_irq() returns Ok(()) and redirects cpu.pc = VECBASE+0x300.
//   The step loop then continues; a BREAK at VECBASE+0x300 (planted in the
//   oracle IRAM program buffer) halts with end.pc = VECBASE+0x300.
//
// SR IDs (from xtensa_sr.rs):
//   VECBASE = 231, EPC1 = 177, EPC2 = 178, EPS2 = 194, INTENABLE = 228.
// ═══════════════════════════════════════════════════════════════════════════════

// ── H8.1: Illegal instruction raises EXCCAUSE=0 ──────────────────────────────

/// Execute an unknown opcode and verify IllegalInstruction exception entry.
///
/// The opcode `0x008530` decodes as `Unknown` in the ST0 group (r=8 is not a
/// valid ST0 sub-opcode). Since H8, `Unknown` raises EXCCAUSE=0 (IllegalInstruction)
/// per Xtensa LX7 ISA RM §5.2, matching real ESP32-S3 hardware behaviour.
///
/// Program:
///   IRAM_BASE+0x000: [0x30, 0x85, 0x00]  — unknown opcode w=0x008530
///   IRAM_BASE+0x003: BREAK (appended; unreachable — exception fires first)
///   IRAM_BASE+0x300: BREAK  ← exception kernel vector landing pad
///
/// VECBASE is relocated to IRAM_BASE so the kernel exception vector
/// (VECBASE+0x300) falls inside the 64 KiB oracle IRAM region where we can
/// plant a BREAK.  Without this, the CPU jumps to the ROM vector at
/// 0x40000300 and execution is uncontrolled.
///
/// After exception entry:
///   EPC1     = IRAM_BASE (faulting PC)
///   EXCCAUSE = 0
///   PS.EXCM  = 1
///   CPU PC   = VECBASE + 0x300 = IRAM_BASE + 0x300
#[hw_oracle_test]
fn illegal_instruction_oracle() -> OracleCase {
    // w=0x008530 in 3-byte LE: [0x30, 0x85, 0x00]
    // decode: op0=0 (QRST), op1=0, op2=0 (ST0), r=8 → Unknown → raise_general_exception(0).
    let mut prog = vec![0u8; 0x303];
    prog[0..3].copy_from_slice(&[0x30, 0x85, 0x00]); // illegal opcode
    prog[0x300..0x303].copy_from_slice(&[0xF0, 0x41, 0x00]); // BREAK at exception vector
    OracleCase::from_bytes(prog)
        .setup(|st| {
            st.write_vecbase(IRAM_BASE);
        })
        .expect(|st| {
            // CPU redirected to kernel exception vector (VECBASE+0x300 = IRAM_BASE+0x300).
            st.assert_pc(IRAM_BASE + 0x300);
            // EPC1 holds the faulting instruction's address.
            st.assert_epc1(IRAM_BASE);
            // EXCCAUSE = 0: IllegalInstruction.
            st.assert_exccause(0);
            // PS.EXCM was set by exception entry.
            st.assert_excm(true);
        })
}

// ── H8.2: RFE returns from exception ─────────────────────────────────────────

/// RFE clears PS.EXCM and jumps to EPC1.
///
/// Setup a fake exception state: PS.EXCM=1, EPC1=IRAM_BASE+3 (address of
/// the BREAK that follows the RFE instruction).  Execute RFE; the CPU returns
/// to IRAM_BASE+3, hits BREAK, and halts.
///
/// Program:
///   IRAM_BASE+0: RFE   (w=0x003000, bytes [0x00, 0x30, 0x00])
///   IRAM_BASE+3: BREAK (halt sentinel, also the RFE return target)
///
/// Encoding derivation (ST0, r=3, s=0, t=0):
///   w = (3<<12)|(0<<8)|(0<<4) = 0x3000   bytes: [0x00, 0x30, 0x00]
///   Verified by xtensa-esp32s3-elf-as: rfe → 0x003000 (G2 test comment).
#[hw_oracle_test]
fn rfe_returns_oracle() -> OracleCase {
    // RFE (0x003000) followed by BREAK (the return target).
    OracleCase::from_bytes(vec![
        0x00, 0x30, 0x00,   // RFE at IRAM_BASE+0
        // BREAK at IRAM_BASE+3 (appended by from_bytes AND used as EPC1 target)
    ])
    .setup(|st| {
        // Pre-load EPC1 = IRAM_BASE+3 (the BREAK instruction address).
        st.write_epc1(IRAM_BASE + 3);
        // Set PS.EXCM=1 so that RFE has something to clear.
        st.write_ps_excm(true);
    })
    .expect(|st| {
        // After RFE: PC = EPC1 = IRAM_BASE+3 (BREAK fires there).
        st.assert_pc(IRAM_BASE + 3);
        // PS.EXCM was cleared by RFE.
        st.assert_excm(false);
    })
}

// ── H8.3: Level-1 interrupt dispatch ─────────────────────────────────────────

/// INTENABLE bit-0 + INTERRUPT bit-0 → level-1 interrupt vector dispatch.
///
/// Bit 0 maps to IRQ level 1 (IRQ_LEVELS[0] = 1). Level-1 interrupt dispatch:
///   EPC1     ← pre-dispatch PC (= IRAM_BASE, where the next instruction would have run)
///   EXCCAUSE ← 4 (Level1InterruptCause)
///   PS.EXCM  ← 1
///   CPU PC   ← VECBASE + 0x300 (kernel exception vector, shared with general exceptions)
///
/// VECBASE is relocated to IRAM_BASE so the L1 vector (IRAM_BASE + 0x300) is
/// within the 64 KiB oracle IRAM region.  A BREAK is planted there to halt.
///
/// Program layout (raw bytes):
///   IRAM_BASE+0x000: NOP.N  [0x3D, 0xF0]  — first instruction (never fetched; IRQ fires first)
///   IRAM_BASE+0x300: BREAK  [0xF0, 0x41, 0x00]  — L1 interrupt vector landing pad
///
/// Dispatch is pre-fetch, so the PC captured in EPC1 = IRAM_BASE+0 (not after NOP.N).
#[hw_oracle_test]
fn interrupt_dispatch_oracle() -> OracleCase {
    // Oracle IRAM program: NOP.N at 0, BREAK at 0x300.
    let mut prog = vec![0u8; 0x300 + 3];
    // NOP.N at IRAM_BASE+0 (2-byte narrow; will not be fetched when IRQ fires)
    prog[0] = 0x3D;
    prog[1] = 0xF0;
    // BREAK at IRAM_BASE+0x300 (L1 vector = VECBASE+0x300 when VECBASE=IRAM_BASE)
    prog[0x300..0x303].copy_from_slice(&[0xF0, 0x41, 0x00]);
    OracleCase::from_bytes(prog)
        .setup(|st| {
            // Relocate VECBASE to IRAM_BASE so the L1 vector is inside oracle IRAM.
            st.write_vecbase(IRAM_BASE);
            // Enable bit-0 IRQ (level 1).
            st.write_intenable(1 << 0);
            // Inject pending bit-0 IRQ via hardware raw path.
            st.write_interrupt(1 << 0);
            // Clear EXCM and INTLEVEL so the IRQ can fire (reset has EXCM=1, INTLEVEL=0xF).
            st.write_ps_excm(false);
            st.write_intlevel(0);
        })
        .expect(|st| {
            // Halted at BREAK in the L1 interrupt vector.
            st.assert_pc(IRAM_BASE + 0x300);
            // EPC1 = pre-dispatch PC (IRAM_BASE+0).
            st.assert_epc1(IRAM_BASE);
            // EXCCAUSE = 4: Level1InterruptCause.
            st.assert_exccause(4);
            // PS.EXCM = 1 (set by level-1 dispatch).
            st.assert_excm(true);
        })
}

// ── H8.4: RFI 2 returns from level-2 interrupt ───────────────────────────────

/// RFI 2 restores PS from EPS2 and PC from EPC2.
///
/// Setup: EPC2 = IRAM_BASE+0x200 (where a BREAK is planted), EPS2 = 0x0005
/// (INTLEVEL=5, EXCM=0).  Execute RFI 2; the CPU restores PS and jumps to
/// IRAM_BASE+0x200, hits BREAK, and halts.
///
/// Program layout (raw bytes):
///   IRAM_BASE+0x000: RFI 2  [0x10, 0x32, 0x00]  (w=0x003210)
///   IRAM_BASE+0x200: BREAK  [0xF0, 0x41, 0x00]   — RFI return target
///   (BREAK appended by from_bytes at IRAM_BASE+3 — never reached)
///
/// Encoding derivation (ST0, r=3, s=level=2, t=1):
///   w = (3<<12)|(2<<8)|(1<<4) = 0x3210   bytes: [0x10, 0x32, 0x00]
///   Verified by xtensa-esp32s3-elf-as: rfi 2 → 0x003210 (G2 test comment).
///
/// EPS2 = 0x0000_0005: bits[3:0]=INTLEVEL=5, bit[4]=EXCM=0.
/// After RFI 2: PS.INTLEVEL=5, PS.EXCM=0.
#[hw_oracle_test]
fn rfi_returns_oracle() -> OracleCase {
    let mut prog = vec![0u8; 0x200 + 3];
    // RFI 2 at IRAM_BASE+0
    prog[0..3].copy_from_slice(&[0x10, 0x32, 0x00]);
    // BREAK at IRAM_BASE+0x200 (EPC2 return target)
    prog[0x200..0x203].copy_from_slice(&[0xF0, 0x41, 0x00]);
    OracleCase::from_bytes(prog)
        .setup(|st| {
            // EPC2 = IRAM_BASE+0x200 (where BREAK is).
            st.write_epc(2, IRAM_BASE + 0x200);
            // EPS2 = 0x0005: INTLEVEL=5, EXCM=0.
            st.write_eps(2, 0x0000_0005);
        })
        .expect(|st| {
            // After RFI 2: PC = EPC2 = IRAM_BASE+0x200 (BREAK fires there).
            st.assert_pc(IRAM_BASE + 0x200);
            // PS.INTLEVEL restored from EPS2.
            st.assert_intlevel(5);
            // PS.EXCM restored from EPS2 (bit 4 of 0x0005 = 0).
            st.assert_excm(false);
        })
}

// ── H8.5: VECBASE relocation — exception redirects to new vector ──────────────

/// Write a new VECBASE, raise IllegalInstruction, verify redirect to new vector.
///
/// Default VECBASE = 0x4000_0000. This test writes VECBASE = IRAM_BASE
/// (= 0x4037_0000) and verifies that the exception entry vector is
/// IRAM_BASE + 0x300 (= 0x4037_0300), not the ROM default 0x4000_0300.
///
/// Program:
///   IRAM_BASE+0: [0x30, 0x85, 0x00]  — unknown opcode w=0x008530 → IllegalInstruction
///   (BREAK appended; never reached — exception fires first)
///
/// The oracle runtime halts on ExceptionRaised and captures cpu.get_pc()
/// (= new VECBASE + 0x300).
#[hw_oracle_test]
fn vecbase_relocation_oracle() -> OracleCase {
    // Program layout:
    //   IRAM_BASE+0x000: illegal opcode [0x30, 0x85, 0x00]  → IllegalInstruction
    //   IRAM_BASE+0x003: BREAK (appended by from_bytes; never reached)
    //   IRAM_BASE+0x300: BREAK  ← exception vector landing pad (VECBASE+0x300)
    //
    // Without a BREAK at IRAM_BASE+0x300, the CPU jumps to the exception vector
    // and executes zeroed (= NOP.N or garbage) memory, causing undefined behavior.
    // We plant a BREAK at offset 0x300 to terminate cleanly.
    let mut prog = vec![0u8; 0x303];
    // Illegal opcode at IRAM_BASE+0
    prog[0..3].copy_from_slice(&[0x30, 0x85, 0x00]);
    // BREAK at IRAM_BASE+0x300 (exception kernel vector = VECBASE+0x300)
    prog[0x300..0x303].copy_from_slice(&[0xF0, 0x41, 0x00]);
    OracleCase::from_bytes(prog)
        .setup(|st| {
            // Relocate VECBASE to IRAM_BASE so exception vector is inside oracle IRAM.
            st.write_vecbase(IRAM_BASE);
        })
        .expect(|st| {
            // PC must be at new VECBASE + 0x300 (not the default 0x4000_0300).
            st.assert_pc(IRAM_BASE + 0x300);
            // EPC1 = faulting PC (IRAM_BASE, where the unknown opcode was).
            st.assert_epc1(IRAM_BASE);
            // EXCCAUSE = 0 (IllegalInstruction).
            st.assert_exccause(0);
            // PS.EXCM was set by exception entry.
            st.assert_excm(true);
        })
}

// ── H8.6: EPC1 / EXCCAUSE readback after second unknown opcode ───────────────

/// Second IllegalInstruction test using a different unknown opcode byte pattern.
///
/// Verifies that any `Unknown` opcode raises EXCCAUSE=0 regardless of the
/// specific bytes — not just the one opcode pattern from H8.1.
/// Also verifies that EXCCAUSE and EPC1 are accurately readable from the
/// end state (the "readback" part of the H8 specification).
///
/// The opcode `0x008540` (w=0x008540: op0=0, op1=0, op2=0, r=8, s=5, t=4 →
/// r=8 is not a valid ST0 sub-opcode → Unknown) uses a different bit pattern
/// from H8.1 but triggers the same IllegalInstruction path.
///
/// A NOP.N is prepended so the faulting PC is IRAM_BASE+2 (not IRAM_BASE),
/// making the EPC1 readback a non-trivial check (EPC1 ≠ IRAM_BASE).
///
/// VECBASE is relocated to IRAM_BASE and a BREAK is planted at IRAM_BASE+0x300
/// so the kernel exception vector lands in the oracle IRAM window.
///
/// Byte layout: w=0x008540 → [0x40, 0x85, 0x00] (3-byte LE).
#[hw_oracle_test]
fn exccause_epc1_readback_oracle() -> OracleCase {
    // Place the unknown opcode at IRAM_BASE+2 by prepending a NOP.N (2 bytes).
    // NOP.N: bytes [0x3D, 0xF0] (narrow, op0=0xD → narrow path, decoded as Nop).
    // After NOP.N, the unknown opcode at IRAM_BASE+2 fires IllegalInstruction.
    // BREAK at IRAM_BASE+0x300 serves as the exception vector landing pad.
    let mut prog = vec![0u8; 0x303];
    prog[0] = 0x3D; // NOP.N byte 0
    prog[1] = 0xF0; // NOP.N byte 1
    prog[2] = 0x40; // unknown opcode byte 0
    prog[3] = 0x85; // unknown opcode byte 1
    prog[4] = 0x00; // unknown opcode byte 2
    prog[0x300..0x303].copy_from_slice(&[0xF0, 0x41, 0x00]); // BREAK at exception vector
    OracleCase::from_bytes(prog)
        .setup(|st| {
            // Relocate VECBASE so the exception kernel vector is inside oracle IRAM.
            st.write_vecbase(IRAM_BASE);
        })
        .expect(|st| {
            // EPC1 = IRAM_BASE+2 (the unknown opcode's address, not IRAM_BASE+0).
            st.assert_epc1(IRAM_BASE + 2);
            // EXCCAUSE = 0 (IllegalInstruction) — readable from end state.
            st.assert_exccause(0);
            // PS.EXCM = 1.
            st.assert_excm(true);
            // CPU PC = VECBASE + 0x300 = IRAM_BASE + 0x300 (where BREAK is).
            st.assert_pc(IRAM_BASE + 0x300);
        })
}

// ── 36. Fibonacci(10) — ELF fixture ──────────────────────────────────────────

/// Fibonacci(10) = 55, verified via a hand-assembled ELF fixture.
///
/// The fixture (`fixtures/xtensa-asm/fibonacci.s`) computes fib(10) using a
/// simple iterative loop (movi / add / addi / bnez) and terminates with
/// `break 1, 15`.  No ENTRY/RETW is used; see the assembly source for
/// rationale.
///
/// `run_sim` loads the ELF PT_LOAD segment at its virtual address (0x40370000)
/// and sets PC to the ELF entry point.  On HW, OpenOCD's `program` command
/// flashes the ELF and sets PC.
///
/// Expected: a2 = 55 (fib(10)).
#[hw_oracle_test]
fn fibonacci_10() -> OracleCase {
    // Path is absolute (via CARGO_MANIFEST_DIR) so the test works regardless
    // of which directory `cargo test` sets as CWD.
    OracleCase::elf(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/xtensa-asm/fibonacci.elf"
    ))
    .expect(|st| st.assert_reg("a2", 55))
    .tolerance(labwired_hw_oracle::Tolerance::exact())
}
