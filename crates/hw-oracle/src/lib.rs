//! HW oracle harness. Filled in during Phase J.

pub use labwired_hw_oracle_macros::hw_oracle_test;

pub mod flash;
pub mod openocd;

// ── OracleCase stub ──────────────────────────────────────────────────────────
//
// TODO(H4): Replace this minimal stub with the full OracleCase builder API:
//   - `OracleCase::asm(encoding: &str)`
//   - `.setup(|st| { st.write_reg(...); })`
//   - `.expect(|st| { st.assert_reg(...); })`
//
// For H3 these stubs exist solely to make the macro trybuild tests compile.

/// An oracle test case description.
///
/// Carries the assembly encoding, register setup, and expected outcomes that
/// the sim/hw/diff runners will execute.  The full builder API is implemented
/// in Task H4.
pub struct OracleCase;

impl OracleCase {
    /// Minimal no-op constructor used by trybuild tests and smoke tests.
    ///
    /// TODO(H4): replace with `asm(encoding: &str) -> Self`.
    pub fn stub() -> Self {
        OracleCase
    }
}

/// Run `case` against the software simulator.
///
/// TODO(H4): implement simulator execution and register assertion.
pub fn run_sim(_case: OracleCase) {}

/// Run `case` against a physical ESP32-S3 board via JTAG.
///
/// TODO(H4): implement OpenOCD-based execution and register readback.
#[cfg(feature = "hw-oracle")]
pub fn run_hw(_case: OracleCase) {}

/// Run `case` against both simulator and hardware, diff the register state.
///
/// TODO(H4): implement diff comparison and panic-on-mismatch.
#[cfg(feature = "hw-oracle")]
pub fn run_diff(_case: OracleCase) {}
