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

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// A FLAT, serialisable view of a single coverage gap, shaped for a consumer
/// list (the CLI's `result.json` → builder `/run` → MCP `unmodeled_access[]`).
///
/// This is the additive, wire-facing projection of [`FidelityReport`]. The
/// in-memory report keys gaps by opcode/address in `BTreeMap`s for dedup/ranking;
/// this struct unrolls each entry into one record with hex-formatted fields so a
/// downstream consumer can render "the model did not model X at PC Y, N times"
/// without knowing the internal keying.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FidelityGap {
    /// `"unmapped_mmio"` or `"undecoded_instruction"`.
    pub kind: String,
    /// Hex address of the unmapped MMIO access (e.g. `"0x40020000"`).
    /// `Some` for `unmapped_mmio`, `None` for undecoded instructions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Hex packed opcode of the undecoded instruction (e.g. `"0xfa80f040"`).
    /// `Some` for `undecoded_instruction`, `None` for MMIO.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opcode: Option<String>,
    /// Hex PC at the first occurrence (e.g. `"0x08000abc"`; `"0x0"` when the
    /// site has no PC, as with a bus access).
    pub first_pc: String,
    /// Number of times this exact gap was hit.
    pub count: u64,
    /// Human-readable classification (mnemonic guess / access kind).
    pub detail: String,
}

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

    /// Flatten the report into a serialisable list of [`FidelityGap`]s: every
    /// unmapped-MMIO entry (address set, opcode absent) followed by every
    /// undecoded-instruction entry (opcode set, address absent). This is the
    /// shape the CLI emits in `result.json` and the builder/MCP surface as
    /// structured unmodeled-access faults.
    pub fn to_gaps(&self) -> Vec<FidelityGap> {
        let mut gaps =
            Vec::with_capacity(self.unmapped_mmio.len() + self.undecoded_instructions.len());
        for (addr, g) in &self.unmapped_mmio {
            gaps.push(FidelityGap {
                kind: "unmapped_mmio".to_string(),
                address: Some(format!("{addr:#x}")),
                opcode: None,
                first_pc: format!("{:#x}", g.first_pc),
                count: g.count,
                detail: g.detail.clone(),
            });
        }
        for (opcode, g) in &self.undecoded_instructions {
            gaps.push(FidelityGap {
                kind: "undecoded_instruction".to_string(),
                address: None,
                opcode: Some(format!("{opcode:#x}")),
                first_pc: format!("{:#x}", g.first_pc),
                count: g.count,
                detail: g.detail.clone(),
            });
        }
        gaps
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

    #[test]
    fn to_gaps_flattens_with_correct_shape() {
        let mut report = FidelityReport::default();
        report.unmapped_mmio.insert(
            0x4002_0000,
            Gap {
                count: 3,
                first_pc: 0,
                detail: "read_u32".to_string(),
            },
        );
        report.undecoded_instructions.insert(
            0xFA80_F040,
            Gap {
                count: 2,
                first_pc: 0x0800_0ABC,
                detail: "uadd8?".to_string(),
            },
        );

        let gaps = report.to_gaps();
        assert_eq!(gaps.len(), 2);

        // unmapped MMIO first: address set, opcode absent.
        let mmio = &gaps[0];
        assert_eq!(mmio.kind, "unmapped_mmio");
        assert_eq!(mmio.address.as_deref(), Some("0x40020000"));
        assert_eq!(mmio.opcode, None);
        assert_eq!(mmio.first_pc, "0x0");
        assert_eq!(mmio.count, 3);
        assert_eq!(mmio.detail, "read_u32");

        // undecoded instruction next: opcode set, address absent.
        let insn = &gaps[1];
        assert_eq!(insn.kind, "undecoded_instruction");
        assert_eq!(insn.address, None);
        assert_eq!(insn.opcode.as_deref(), Some("0xfa80f040"));
        assert_eq!(insn.first_pc, "0x8000abc");
        assert_eq!(insn.count, 2);
        assert_eq!(insn.detail, "uadd8?");

        // Round-trips through serde (the wire contract) and skips None fields.
        let json = serde_json::to_string(insn).unwrap();
        assert!(
            !json.contains("address"),
            "None address must be skipped: {json}"
        );
        assert!(json.contains("\"opcode\":\"0xfa80f040\""), "{json}");
        let back: FidelityGap = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, insn);
    }
}
