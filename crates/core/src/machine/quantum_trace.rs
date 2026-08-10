// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Which clause bound the CPU quantum (opt-in, `quantum-trace` feature).
//!
//! `plan_cpu_window` narrows a batch width through a dozen independent clauses
//! and returns only the winner. When a board sits at 1.00 steps/batch that
//! answer is unusable: every clause is a candidate and the only way to find the
//! real one is to delete clauses one at a time and re-measure. #835 spent a
//! whole investigation that way, ruled out three clauses, and still ended on a
//! guess — the issue says so explicitly.
//!
//! So the planner records its own verdict instead. Each plan attributes the
//! final `count` to the clause that last lowered it, and the counts are
//! readable per board:
//!
//! ```ignore
//! quantum_trace::reset();
//! machine.run(20_000_000)?;
//! for (clause, hits) in quantum_trace::snapshot() {
//!     println!("{clause:24} {hits}");
//! }
//! ```
//!
//! Off by default and compiled out entirely — `plan_cpu_window` is per-batch
//! hot path, and #778 exists because per-step host cost is a real regression
//! class here.

#[cfg(feature = "quantum-trace")]
use std::cell::RefCell;
#[cfg(feature = "quantum-trace")]
use std::collections::BTreeMap;

/// Names of the clauses in `plan_cpu_window` that can bind the quantum.
/// String constants rather than an enum so a new clause is one line at the
/// call site and needs no match arm kept in sync.
pub mod clause {
    pub const UNBOUNDED: &str = "unbounded";
    pub const FUEL_LIMIT: &str = "fuel_limit";
    pub const CYCLE_LIMIT: &str = "cycle_limit";
    pub const BATCH_POLICY: &str = "batch_policy";
    pub const MOTOR_DEADLINE: &str = "motor_deadline";
    pub const RESET_FIDELITY: &str = "reset_fidelity";
    pub const SECONDARY_LOCKSTEP: &str = "secondary_lockstep";
    pub const CYCLE_ACCURATE_BUS: &str = "cycle_accurate_bus";
    pub const POLL_SAMPLING: &str = "poll_sampling";
    pub const HONORED_BREAKPOINTS: &str = "honored_breakpoints";
    pub const SECONDARY_PARKED: &str = "secondary_parked";
    pub const TICK_BOUNDARY: &str = "tick_boundary";
    pub const HCSR04_DEADLINE: &str = "hcsr04_deadline";
    pub const SCHEDULER_DEADLINE: &str = "scheduler_deadline";
}

#[cfg(feature = "quantum-trace")]
thread_local! {
    /// clause -> (times it bound the quantum, total instructions those batches ran)
    static HITS: RefCell<BTreeMap<&'static str, (u64, u64)>> = RefCell::new(BTreeMap::new());
}

/// Record that `clause` produced a batch of `count` instructions.
#[cfg(feature = "quantum-trace")]
#[inline]
pub(crate) fn record(clause: &'static str, count: u64) {
    HITS.with(|h| {
        let mut h = h.borrow_mut();
        let e = h.entry(clause).or_insert((0, 0));
        e.0 += 1;
        e.1 += count;
    });
}

/// Drop everything recorded so far — call before the measured window so
/// warm-up and ELF loading do not pollute the histogram.
#[cfg(feature = "quantum-trace")]
pub fn reset() {
    HITS.with(|h| h.borrow_mut().clear());
}

/// `(clause, batches, mean batch width)`, widest-impact first: the clause that
/// bound the most batches is the one holding a board's throughput down.
#[cfg(feature = "quantum-trace")]
pub fn snapshot() -> Vec<(&'static str, u64, f64)> {
    let mut rows: Vec<_> = HITS.with(|h| {
        h.borrow()
            .iter()
            .map(|(k, (n, total))| (*k, *n, *total as f64 / *n as f64))
            .collect()
    });
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    rows
}

/// The clause that bound the most batches, if anything was recorded.
#[cfg(feature = "quantum-trace")]
pub fn dominant() -> Option<&'static str> {
    snapshot().first().map(|(clause, _, _)| *clause)
}
