// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Oracle test suite: first oracle test covering the ADD instruction.
//!
//! Each `#[hw_oracle_test]` function expands into three tests:
//!
//! * `add_oracle_sim`  — always compiled; runs against the software simulator.
//! * `add_oracle_hw`   — gated on `feature = "hw-oracle"`, `#[ignore]`; requires ESP32-S3.
//! * `add_oracle_diff` — gated on `feature = "hw-oracle"`, `#[ignore]`; requires ESP32-S3.
//!
//! Run sim only:
//! ```
//! cargo test -p labwired-hw-oracle add_oracle_sim
//! ```
//!
//! Run HW / diff (board must be connected):
//! ```
//! cargo test -p labwired-hw-oracle --features hw-oracle -- --ignored
//! ```

use labwired_hw_oracle::{hw_oracle_test, OracleCase};

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
