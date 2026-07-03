//! Fidelity monitor — surfaces simulator-coverage gaps early.
//!
//! When the model hits something it does not implement, the historical failure
//! mode was *silent*: an undecoded instruction was skipped (registers left
//! stale) and an unmapped MMIO access returned a swallowed error. Both look
//! like the firmware "just running", but the behaviour is wrong. The
//! UADD8/SEL incident is the canonical example — newlib's optimised `strlen`
//! used two SIMD instructions the decoder ignored, so every C-string length
//! came out garbage, and it took a deep debugging session to notice.
//!
//! This module records those gaps into a per-thread log so they can be:
//!   * printed as a ranked summary after a run (`report()` / `Display`),
//!   * asserted empty by a regression test (a known-good firmware must hit
//!     zero gaps), and
//!   * turned into a hard failure at the first gap via the
//!     `LABWIRED_STRICT_FIDELITY` env var (panics with opcode/address + PC).
//!
//! The log is thread-local: each test thread / world run accumulates its own,
//! which is the correct isolation for `cargo test`'s multi-threaded harness.
//! Recording only happens on the (rare) gap paths, so there is no cost on the
//! hot mapped-access / decoded-instruction paths.

use std::cell::RefCell;
use std::collections::BTreeMap;

/// One distinct coverage gap and how often it was hit.
#[derive(Clone, Debug)]
pub struct Gap {
    /// Number of times this exact gap was hit.
    pub count: u64,
    /// PC at the first occurrence (0 when the site has no PC, e.g. a bus access).
    pub first_pc: u32,
    /// Human-readable classification (mnemonic guess / access kind).
    pub detail: String,
}

/// Everything the model failed to model on the current thread.
#[derive(Clone, Debug, Default)]
pub struct FidelityReport {
    /// Undecoded / unhandled instructions, keyed by packed opcode
    /// (16-bit: the halfword; 32-bit: `(h1 << 16) | h2`).
    pub undecoded_instructions: BTreeMap<u64, Gap>,
    /// Accesses to addresses no peripheral or memory region claims,
    /// keyed by address.
    pub unmapped_mmio: BTreeMap<u64, Gap>,
}

impl FidelityReport {
    /// True when the model hit no coverage gaps at all.
    pub fn is_empty(&self) -> bool {
        self.undecoded_instructions.is_empty() && self.unmapped_mmio.is_empty()
    }

    /// Total gap hits across both categories.
    pub fn total_hits(&self) -> u64 {
        self.undecoded_instructions
            .values()
            .map(|g| g.count)
            .sum::<u64>()
            + self.unmapped_mmio.values().map(|g| g.count).sum::<u64>()
    }
}

impl std::fmt::Display for FidelityReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty() {
            return write!(
                f,
                "fidelity: clean (no undecoded instructions or unmapped MMIO)"
            );
        }
        writeln!(
            f,
            "fidelity: {} gap kind(s), {} total hit(s):",
            self.undecoded_instructions.len() + self.unmapped_mmio.len(),
            self.total_hits()
        )?;
        if !self.undecoded_instructions.is_empty() {
            writeln!(f, "  undecoded instructions:")?;
            for (opcode, g) in &self.undecoded_instructions {
                writeln!(
                    f,
                    "    {:#010x} ({})  x{}  first@pc {:#010x}",
                    opcode, g.detail, g.count, g.first_pc
                )?;
            }
        }
        if !self.unmapped_mmio.is_empty() {
            writeln!(f, "  unmapped MMIO:")?;
            for (addr, g) in &self.unmapped_mmio {
                writeln!(f, "    {:#010x} ({})  x{}", addr, g.detail, g.count)?;
            }
        }
        Ok(())
    }
}

thread_local! {
    static LOG: RefCell<FidelityReport> = RefCell::new(FidelityReport::default());
}

/// True when strict mode is on: the first gap panics instead of accumulating.
fn strict() -> bool {
    std::env::var_os("LABWIRED_STRICT_FIDELITY").is_some()
}

/// Record an undecoded/unhandled instruction. In strict mode this panics with
/// the opcode + PC so a CI lane fails loudly at the first gap.
pub fn record_undecoded(pc: u32, opcode: u64, detail: &str) {
    if strict() {
        panic!(
            "LABWIRED_STRICT_FIDELITY: undecoded instruction {opcode:#010x} ({detail}) at pc {pc:#010x}"
        );
    }
    LOG.with(|l| {
        l.borrow_mut()
            .undecoded_instructions
            .entry(opcode)
            .or_insert_with(|| Gap {
                count: 0,
                first_pc: pc,
                detail: detail.to_string(),
            })
            .count += 1;
    });
}

/// Record an access to an address no peripheral or memory region claims.
/// In strict mode this panics with the address.
pub fn record_unmapped(addr: u64, detail: &str) {
    if strict() {
        panic!("LABWIRED_STRICT_FIDELITY: unmapped MMIO {detail} at {addr:#010x}");
    }
    LOG.with(|l| {
        l.borrow_mut()
            .unmapped_mmio
            .entry(addr)
            .or_insert_with(|| Gap {
                count: 0,
                first_pc: 0,
                detail: detail.to_string(),
            })
            .count += 1;
    });
}

/// Snapshot the current thread's report without clearing it.
pub fn report() -> FidelityReport {
    LOG.with(|l| l.borrow().clone())
}

/// Take and clear the current thread's report (use at the start/end of a run
/// to scope the accounting).
pub fn take() -> FidelityReport {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

/// Clear the current thread's report.
pub fn reset() {
    LOG.with(|l| *l.borrow_mut() = FidelityReport::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_and_dedupes() {
        // This test exercises the non-strict accumulation path; under
        // LABWIRED_STRICT_FIDELITY the record_* calls panic by design.
        if strict() {
            return;
        }
        reset();
        record_undecoded(0x1000, 0xFA80_F040, "uadd8?");
        record_undecoded(0x1004, 0xFA80_F040, "uadd8?");
        record_unmapped(0x4002_0000, "read_u32");
        let r = report();
        assert_eq!(r.undecoded_instructions.len(), 1);
        assert_eq!(r.undecoded_instructions[&0xFA80_F040].count, 2);
        assert_eq!(r.undecoded_instructions[&0xFA80_F040].first_pc, 0x1000);
        assert_eq!(r.unmapped_mmio[&0x4002_0000].count, 1);
        assert!(!r.is_empty());
        assert_eq!(r.total_hits(), 3);
        let taken = take();
        assert_eq!(taken.total_hits(), 3);
        assert!(report().is_empty());
    }
}
