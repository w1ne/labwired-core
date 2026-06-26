// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Runtime program-counter coverage.
//!
//! This records which instruction addresses a firmware run actually executed,
//! plus the control-flow edges between them, by observing the per-instruction
//! step hook. It is the raw execution-coverage source that higher layers map
//! back to source statements and branches via DWARF debug info.
//!
//! This is distinct from the [`crate::coverage`] module: that one measures, by
//! observable behaviour, whether a chip model implements the registers a part's
//! SVD declares (model faithfulness). This one measures which addresses of the
//! firmware under test ran (firmware coverage).

use crate::SimulationObserver;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Taken / not-taken counts observed for the instruction at one source address.
///
/// `taken` counts the times execution left via a non-fall-through path (a taken
/// conditional branch, an unconditional jump, a call, a return or a trap);
/// `not_taken` counts the times it fell through to the next instruction. A
/// source address with `taken > 0` is a branch site.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BranchCounts {
    pub taken: u64,
    pub not_taken: u64,
    pub target: Option<u32>,
}

/// Observer that accumulates the set of executed instruction addresses and the
/// per-instruction taken/not-taken control-flow outcomes between them.
///
/// Addresses are stored half-word aligned (the Thumb bit is masked off) so a PC
/// and its interworking alias collapse to one entry. For each source address we
/// record whether execution fell through or diverged, which yields both branch
/// edges and branch taken/not-taken coverage.
#[derive(Debug, Default)]
pub struct PcCoverageObserver {
    executed: Mutex<BTreeSet<u32>>,
    branches: Mutex<BTreeMap<u32, BranchCounts>>,
    last: Mutex<Option<(u32, u32)>>, // (aligned_pc, opcode_len_in_bytes)
    total: AtomicU64,
}

impl PcCoverageObserver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Total instructions observed (including repeats of the same address).
    pub fn total_instructions(&self) -> u64 {
        self.total.load(Ordering::SeqCst)
    }

    /// Distinct instruction addresses executed, ascending.
    pub fn covered_addresses(&self) -> Vec<u32> {
        self.executed
            .lock()
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Number of distinct instruction addresses executed.
    pub fn covered_count(&self) -> usize {
        self.executed.lock().map(|s| s.len()).unwrap_or(0)
    }

    /// Control-flow edges taken, as `(from, to)` aligned address pairs, ascending.
    pub fn edges(&self) -> Vec<(u32, u32)> {
        self.branches
            .lock()
            .map(|b| {
                b.iter()
                    .filter_map(|(src, c)| c.target.filter(|_| c.taken > 0).map(|t| (*src, t)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Branch sites — source addresses that diverged at least once — with their
    /// taken/not-taken counts, ascending by address.
    pub fn branch_sites(&self) -> Vec<(u32, BranchCounts)> {
        self.branches
            .lock()
            .map(|b| {
                b.iter()
                    .filter(|(_, c)| c.taken > 0)
                    .map(|(src, c)| (*src, *c))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// True if the given (aligned) address was executed at least once.
    pub fn was_executed(&self, addr: u32) -> bool {
        let addr = addr & !1;
        self.executed
            .lock()
            .map(|s| s.contains(&addr))
            .unwrap_or(false)
    }
}

impl SimulationObserver for PcCoverageObserver {
    fn on_step_start(&self, pc: u32, opcode: u32) {
        let aligned = pc & !1;
        self.total.fetch_add(1, Ordering::SeqCst);

        if let Ok(mut e) = self.executed.lock() {
            e.insert(aligned);
        }

        // Thumb instructions are 16 or 32 bits. A 32-bit instruction has its
        // top 5 opcode bits in 0b11101/0b11110/0b11111, encoded so the high
        // half-word leads; treat anything with those high bits set as 4 bytes.
        let len = if (opcode >> 11) & 0b11111 >= 0b11101 && (opcode >> 16) != 0 {
            4
        } else {
            2
        };

        if let Ok(mut last) = self.last.lock() {
            if let Some((prev_pc, prev_len)) = *last {
                let fallthrough = prev_pc.wrapping_add(prev_len);
                if let Ok(mut branches) = self.branches.lock() {
                    let counts = branches.entry(prev_pc).or_default();
                    if aligned != fallthrough {
                        counts.taken += 1;
                        counts.target = Some(aligned);
                    } else {
                        counts.not_taken += 1;
                    }
                }
            }
            *last = Some((aligned, len));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_distinct_addresses_and_count() {
        let cov = PcCoverageObserver::new();
        // Three 16-bit instructions, the middle one executed twice.
        cov.on_step_start(0x0800_0000, 0x4600);
        cov.on_step_start(0x0800_0002, 0x4600);
        cov.on_step_start(0x0800_0002, 0x4600);
        cov.on_step_start(0x0800_0004, 0x4600);

        assert_eq!(cov.total_instructions(), 4);
        assert_eq!(cov.covered_count(), 3);
        assert_eq!(
            cov.covered_addresses(),
            vec![0x0800_0000, 0x0800_0002, 0x0800_0004]
        );
        assert!(cov.was_executed(0x0800_0002));
        assert!(!cov.was_executed(0x0800_0006));
    }

    #[test]
    fn masks_thumb_bit() {
        let cov = PcCoverageObserver::new();
        cov.on_step_start(0x0800_0001, 0x4600); // interworking alias
        assert!(cov.was_executed(0x0800_0000));
        assert_eq!(cov.covered_count(), 1);
    }

    #[test]
    fn records_branch_edges_not_fallthrough() {
        let cov = PcCoverageObserver::new();
        // Sequential fall-through: no edges.
        cov.on_step_start(0x0800_0000, 0x4600);
        cov.on_step_start(0x0800_0002, 0x4600);
        assert!(cov.edges().is_empty());

        // A jump backwards to 0x0800_0000: one edge from 0x0800_0002.
        cov.on_step_start(0x0800_0000, 0x4600);
        assert_eq!(cov.edges(), vec![(0x0800_0002, 0x0800_0000)]);
    }

    #[test]
    fn records_branch_taken_and_not_taken_counts() {
        let cov = PcCoverageObserver::new();
        // Branch at 0x0800_0100: once falls through to 0x0800_0102, once taken
        // to 0x0800_0200. (Re-entering the branch is itself a divergence from
        // 0x0800_0102, so assert on 0x0800_0100's own counts, not the site
        // total.)
        cov.on_step_start(0x0800_0100, 0x4600);
        cov.on_step_start(0x0800_0102, 0x4600); // 0x...0100 fell through
        cov.on_step_start(0x0800_0100, 0x4600); // re-enter the branch
        cov.on_step_start(0x0800_0200, 0x4600); // 0x...0100 taken to 0x...0200

        let sites = cov.branch_sites();
        let (_, counts) = sites
            .iter()
            .find(|(src, _)| *src == 0x0800_0100)
            .expect("0x0800_0100 is a branch site");
        assert_eq!(counts.not_taken, 1);
        assert_eq!(counts.taken, 1);
        assert_eq!(counts.target, Some(0x0800_0200));
    }
}
