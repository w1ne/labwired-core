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

use std::sync::atomic::Ordering;

use crate::bus::SystemBus;
use crate::memory::LinearMemory;

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

/// FNV-1a over a byte slice, chained from a running hash so multiple regions
/// (RAM, each `extra_mem` window) can be folded into one fingerprint.
fn fnv1a(bytes: &[u8], mut hash: u64) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// Cheap 64-bit fingerprint of everything a wrong JIT store could corrupt
/// that isn't already in the [`StateVec`]: main RAM, every `extra_mem`
/// window (e.g. ESP32 IRAM), and NVIC pending/enable state. Two buses with
/// the same fingerprint are extremely unlikely to differ; a mismatch is a
/// cue to fall back to [`diff_memory`] for the exact address.
///
/// Deliberately does NOT hash flash: firmware is read-only in every current
/// test and flash-dirty tracking does not exist yet (see
/// `CortexMJitHost::take_flash_dirty`), so comparing it would just cost
/// cycles without catching anything the block-invalidation logic doesn't
/// already assume.
pub fn memory_fingerprint(bus: &SystemBus) -> u64 {
    let mut hash = fnv1a(&bus.ram.data, FNV_OFFSET_BASIS);
    for region in &bus.extra_mem {
        hash = fnv1a(&region.data, hash);
    }
    if let Some(nvic) = &bus.nvic {
        for word in nvic.iser.iter().chain(nvic.ispr.iter()) {
            hash = fnv1a(&word.load(Ordering::Relaxed).to_le_bytes(), hash);
        }
    }
    hash
}

/// A detected byte-level divergence in bus-visible memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryDivergence {
    /// Instruction/unit index (or comparison point) at which they diverged.
    pub at_step: u64,
    /// Guest PC (interpreter side) at the comparison point, for context.
    pub pc: u32,
    /// Absolute bus address of the first differing byte.
    pub address: u64,
    /// Byte on the interpreter (reference) side.
    pub interp: u8,
    /// Byte on the JIT side.
    pub jit: u8,
}

fn diff_linear_memory(a: &LinearMemory, b: &LinearMemory) -> Option<(u64, u8, u8)> {
    let n = a.data.len().max(b.data.len());
    for i in 0..n {
        let av = a.data.get(i).copied().unwrap_or(0);
        let bv = b.data.get(i).copied().unwrap_or(0);
        if av != bv {
            return Some((a.base_addr + i as u64, av, bv));
        }
    }
    None
}

/// Byte-level diff of bus-visible memory, reporting the first differing
/// address. Only worth calling when [`memory_fingerprint`] has already
/// shown the two buses disagree — this is O(bytes), the fingerprint is the
/// fast path taken on every comparison.
pub fn diff_memory(
    at_step: u64,
    pc: u32,
    interp: &SystemBus,
    jit: &SystemBus,
) -> Option<MemoryDivergence> {
    if let Some((address, i, j)) = diff_linear_memory(&interp.ram, &jit.ram) {
        return Some(MemoryDivergence {
            at_step,
            pc,
            address,
            interp: i,
            jit: j,
        });
    }
    for (a, b) in interp.extra_mem.iter().zip(jit.extra_mem.iter()) {
        if let Some((address, i, j)) = diff_linear_memory(a, b) {
            return Some(MemoryDivergence {
                at_step,
                pc,
                address,
                interp: i,
                jit: j,
            });
        }
    }
    None
}

/// Fast RAM/peripheral-state check for a comparison point: hashes both
/// buses and, only on a mismatch, does the byte-level diff to name the
/// exact address. Returns `None` when the two buses agree.
pub fn compare_memory(
    at_step: u64,
    pc: u32,
    interp: &SystemBus,
    jit: &SystemBus,
) -> Option<MemoryDivergence> {
    if memory_fingerprint(interp) == memory_fingerprint(jit) {
        return None;
    }
    diff_memory(at_step, pc, interp, jit).or(Some(MemoryDivergence {
        at_step,
        pc,
        address: u64::MAX,
        interp: 0,
        jit: 0,
    }))
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
