// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential (lockstep) JIT-vs-interpreter equivalence harness.
//!
//! This is the merge gate's correctness proof: run the *same* firmware
//! twice from the same reset state — once JIT-enabled, once pure
//! interpreter — and assert the architectural state matches at every
//! comparison point. Any divergence is a JIT bug, reported with the exact
//! step and the first differing state word.
//!
//! This scaffold owns the ISA-neutral comparison machinery
//! ([`compare`], [`DiffPolicy`], [`Divergence`]) and the driver shape
//! ([`DifferentialHarness`]). The Xtensa pilot already ships a concrete,
//! richer version of this in [`crate::cpu::xtensa_lockstep`]
//! (`LockstepRunner` / `compare_traces`); as the framework absorbs each
//! ISA, that logic generalises onto this [`StateVec`]-based interface.
//!
//! ## Comparison cadence
//!
//! Comparing after *every* instruction is the strongest check but is
//! O(state) per step. Because a compiled block retires many instructions
//! atomically, the natural cadence is **per compiled block boundary**
//! (compare when the JIT side-exits) with an optional per-N-instruction
//! cap for long straight-line runs. Both sides must be aligned to the same
//! guest instruction before a compare — the harness only compares at
//! points where the two runs are known to be at the same PC.

use super::StateVec;

/// How lenient a state comparison is. Some architectural words legitimately
/// differ between a batched JIT run and a per-instruction interpreter run
/// (a free-running cycle counter observed mid-block, say) and must be
/// masked or bounded rather than compared for exact equality.
#[derive(Debug, Clone)]
pub struct DiffPolicy {
    /// Indices into the [`StateVec`] to skip entirely (e.g. a volatile
    /// cycle counter). Everything else is compared for exact equality.
    pub ignore_indices: Vec<usize>,
    /// Compare only at compiled-block boundaries (`false` = also compare
    /// on interpreter-only steps when both sides are PC-aligned).
    pub block_boundary_only: bool,
}

impl Default for DiffPolicy {
    fn default() -> Self {
        Self {
            ignore_indices: Vec::new(),
            block_boundary_only: true,
        }
    }
}

/// A detected mismatch between the JIT and interpreter runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// Instruction index (or comparison point) at which they diverged.
    pub at_step: u64,
    /// Index into the [`StateVec`] of the first differing word.
    pub word_index: usize,
    /// Value on the interpreter (reference) side.
    pub interp: u32,
    /// Value on the JIT side.
    pub jit: u32,
}

/// Compare two state snapshots under `policy`. Returns the first
/// divergence, or `None` if they agree. A length mismatch is reported as a
/// divergence at the first missing word.
pub fn compare(
    at_step: u64,
    interp: &StateVec,
    jit: &StateVec,
    policy: &DiffPolicy,
) -> Option<Divergence> {
    let n = interp.len().max(jit.len());
    for i in 0..n {
        if policy.ignore_indices.contains(&i) {
            continue;
        }
        let a = interp.get(i).copied();
        let b = jit.get(i).copied();
        if a != b {
            return Some(Divergence {
                at_step,
                word_index: i,
                interp: a.unwrap_or(0),
                jit: b.unwrap_or(0),
            });
        }
    }
    None
}

/// Report from a full differential run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffReport {
    /// Comparison points checked.
    pub compares: u64,
    /// First divergence found, if any. `None` == equivalence proven over
    /// the run.
    pub divergence: Option<Divergence>,
}

impl DiffReport {
    /// Whether the two runs were equivalent everywhere compared.
    pub fn is_equivalent(&self) -> bool {
        self.divergence.is_none()
    }
}

/// Driver that steps a JIT run and an interpreter run in lockstep,
/// comparing at each aligned point. The concrete stepping is supplied by
/// the caller as two closures over their respective machines (matching the
/// existing [`crate::cpu::xtensa_lockstep::LockstepRunner`] factory shape);
/// this scaffold owns the compare loop and the report.
pub struct DifferentialHarness {
    policy: DiffPolicy,
    max_compares: u64,
}

impl DifferentialHarness {
    /// New harness with a comparison budget.
    pub fn new(max_compares: u64) -> Self {
        Self {
            policy: DiffPolicy::default(),
            max_compares,
        }
    }

    /// Override the comparison policy.
    pub fn with_policy(mut self, policy: DiffPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Drive both sides. `interp_step` and `jit_step` each advance their
    /// machine to the next aligned comparison point and return its
    /// [`StateVec`], or `None` when that machine halts. The run ends at the
    /// first divergence, the first halt on either side, or the compare
    /// budget.
    pub fn run<I, J>(&self, mut interp_step: I, mut jit_step: J) -> DiffReport
    where
        I: FnMut() -> Option<StateVec>,
        J: FnMut() -> Option<StateVec>,
    {
        let mut compares = 0;
        while compares < self.max_compares {
            let (Some(i), Some(j)) = (interp_step(), jit_step()) else {
                break; // one side halted — nothing left to compare
            };
            compares += 1;
            if let Some(d) = compare(compares, &i, &j, &self.policy) {
                return DiffReport {
                    compares,
                    divergence: Some(d),
                };
            }
        }
        DiffReport {
            compares,
            divergence: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_states_are_equivalent() {
        assert_eq!(
            compare(0, &vec![1, 2, 3], &vec![1, 2, 3], &DiffPolicy::default()),
            None
        );
    }

    #[test]
    fn first_differing_word_is_reported() {
        let d = compare(7, &vec![1, 2, 3], &vec![1, 9, 3], &DiffPolicy::default()).unwrap();
        assert_eq!(
            d,
            Divergence {
                at_step: 7,
                word_index: 1,
                interp: 2,
                jit: 9
            }
        );
    }

    #[test]
    fn ignored_indices_are_skipped() {
        let policy = DiffPolicy {
            ignore_indices: vec![1], // e.g. volatile cycle counter
            block_boundary_only: true,
        };
        // word 1 differs but is ignored; word 2 agrees -> equivalent
        assert!(compare(0, &vec![1, 2, 3], &vec![1, 999, 3], &policy).is_none());
    }

    #[test]
    fn harness_detects_divergence_at_step() {
        let interp = vec![vec![10], vec![20], vec![30]];
        let jit = vec![vec![10], vec![99], vec![30]]; // diverges at compare 2
        let mut ii = interp.into_iter();
        let mut ji = jit.into_iter();
        let report = DifferentialHarness::new(100).run(|| ii.next(), || ji.next());
        assert!(!report.is_equivalent());
        let d = report.divergence.unwrap();
        assert_eq!(d.at_step, 2);
        assert_eq!((d.interp, d.jit), (20, 99));
    }

    #[test]
    fn harness_proves_equivalence_until_halt() {
        let interp = vec![vec![1], vec![2]];
        let jit = vec![vec![1], vec![2]];
        let mut ii = interp.into_iter();
        let mut ji = jit.into_iter();
        let report = DifferentialHarness::new(100).run(|| ii.next(), || ji.next());
        assert!(report.is_equivalent());
        assert_eq!(report.compares, 2);
    }
}
