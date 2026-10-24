// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! A reusable oracle for peripheral scheduler migrations.
//!
//! # What this is for
//!
//! Moving a peripheral off the per-cycle legacy walk and onto the event
//! scheduler is the single highest-value optimisation available in this tree,
//! because `SystemBus::max_safe_tick_interval` returns 1 while ANY peripheral
//! answers `needs_legacy_walk() == true`, and at a one-instruction window the
//! Cortex-M hot-loop fast path never engages (it needs a budget of >= 8).
//! Measured on run 35559786580 — same perf fixture, same ISA, same memory map:
//!
//! ```text
//! nrf52840     54.7 Ir/step   interval 512
//! nrf54l15   2119.5 Ir/step   interval   1    38.7x
//! atsamd21   2567.5 Ir/step   interval   1    ~47x
//! ```
//!
//! It is also the easiest change in this tree to get silently wrong. The
//! migration swaps WHEN a peripheral does its work, and a model's own unit
//! tests almost never notice: they construct the model directly, never attach
//! a [`CycleClock`], and therefore keep exercising the LEGACY path after the
//! migration lands. `nrf54l/clock.rs` had thirteen such tests and not one of
//! them could have witnessed a regression in the path that replaced it.
//!
//! # Why a harness rather than another copy
//!
//! Eleven `*_walk_differential` tests already exist, totalling 6207 lines, and
//! each hand-rolls the same four pieces: a `Probe` struct, a `probe()` that
//! snapshots it, a `run_probed()` that single-steps and collects, and an
//! `assert_probes_identical()`. Copying that again for each of the nine
//! peripherals still pinning nrf54l15 / nrf54lm20a / atsamd21g18a is how a
//! migration ends up with no gate at all — the cost of writing one is what
//! makes skipping it tempting.
//!
//! # The contract
//!
//! Build the SAME machine twice. In [`WalkMode::LegacyWalk`] the peripheral
//! under test is pinned onto the walk with `force_legacy_walk()` — that run is
//! the REFERENCE, and it is the behaviour that shipped. In
//! [`WalkMode::Scheduler`] it is left to the scheduler. Run identical firmware
//! in both and compare every observable at every instruction boundary.
//!
//! What "identical" covers is deliberately wide: `total_cycles`, `pc` and all
//! sixteen core registers come for free, and the caller adds whatever the
//! peripheral's own product is — RAM counters an ISR increments, a DMA
//! destination, captured UART bytes. A migration that changes only timing
//! nobody observes is fine; one that changes a byte or a cycle is not.
//!
//! ## What it does NOT prove
//!
//! * That the scheduler path is REACHED. A model whose `uses_scheduler()`
//!   wrongly returns false passes trivially — both runs are the walk. Assert
//!   the mode separately; [`assert_modes_differ`] is provided for that.
//! * Anything at a tick interval you did not run. IRQ delivery quantises to
//!   the batch grid at interval > 1 (documented, bounded by one interval), so
//!   a byte-identical claim belongs at interval 1, and coarser intervals want
//!   a count-based assertion instead.

#![allow(dead_code)]

// `run`, `get_pc` and `read_core_reg` reach the machine through DebugControl.
use labwired_core::{Cpu, DebugControl, Machine};

/// Which side of the differential a machine is being built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkMode {
    /// The peripheral under test is pinned onto the per-cycle walk with
    /// `force_legacy_walk()`. This run is the reference: it is what shipped.
    LegacyWalk,
    /// The peripheral is left to the event scheduler — the candidate.
    Scheduler,
}

impl WalkMode {
    /// `true` for the reference side, so a builder can write
    /// `if mode.is_legacy_walk() { p.force_legacy_walk() }`.
    pub fn is_legacy_walk(self) -> bool {
        matches!(self, WalkMode::LegacyWalk)
    }
}

/// One instruction boundary's observable state.
///
/// `extra` is the caller's, and is what makes the gate bite: the architectural
/// state alone would not notice a peripheral that stopped moving bytes, only
/// one that changed the instruction stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub step: u64,
    pub total_cycles: u64,
    pub pc: u32,
    pub regs: [u32; 16],
    pub extra: Vec<(&'static str, u64)>,
}

impl Probe {
    /// Look up one caller-supplied observable by name.
    pub fn extra(&self, name: &str) -> u64 {
        self.extra
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("probe has no extra observable named {name:?}"))
            .1
    }
}

/// Snapshot one instruction boundary without stepping: for gates that run a
/// batched `Machine::run` (final-state comparisons at interval > 1, WFI
/// fast-forward end-states), where per-instruction probing is not possible.
pub fn probe_at<C: Cpu, E>(machine: &Machine<C>, step: u64, extra: &E) -> Probe
where
    E: Fn(&Machine<C>) -> Vec<(&'static str, u64)>,
{
    let mut regs = [0u32; 16];
    for (i, r) in regs.iter_mut().enumerate() {
        *r = machine.read_core_reg(i as u8);
    }
    Probe {
        step,
        total_cycles: machine.total_cycles,
        pc: machine.get_pc(),
        regs,
        extra: extra(machine),
    }
}

/// Single-step `steps` instructions from `entry`, snapshotting after each.
///
/// Stepping ONE instruction at a time is the point: a batched run would hide
/// exactly the mid-batch divergence this exists to catch.
pub fn run_probed<C: Cpu, E>(
    machine: &mut Machine<C>,
    entry: u32,
    steps: u64,
    extra: &E,
) -> Vec<Probe>
where
    E: Fn(&Machine<C>) -> Vec<(&'static str, u64)>,
{
    machine.cpu.set_pc(entry);
    let mut probes = Vec::with_capacity(steps as usize);
    for s in 0..steps {
        machine.run(Some(1)).expect("machine step");
        probes.push(probe_at(machine, s + 1, extra));
    }
    probes
}

/// Compare two probe streams, failing at the FIRST divergence.
///
/// The message names the step and the field, because "these 3000-element
/// vectors differ" is not a diagnosis.
pub fn assert_probes_identical(reference: &[Probe], candidate: &[Probe], what: &str) {
    assert_eq!(
        reference.len(),
        candidate.len(),
        "{what}: probe counts differ ({} walk vs {} scheduler) — one run \
         stopped early, which is itself the divergence",
        reference.len(),
        candidate.len(),
    );
    for (r, c) in reference.iter().zip(candidate.iter()) {
        if r == c {
            continue;
        }
        let field = if r.total_cycles != c.total_cycles {
            format!("total_cycles {} vs {}", r.total_cycles, c.total_cycles)
        } else if r.pc != c.pc {
            format!("pc {:#010x} vs {:#010x}", r.pc, c.pc)
        } else if r.regs != c.regs {
            let i = (0..16).find(|&i| r.regs[i] != c.regs[i]).unwrap_or(0);
            format!("r{i} {:#010x} vs {:#010x}", r.regs[i], c.regs[i])
        } else {
            let names: Vec<String> = r
                .extra
                .iter()
                .zip(c.extra.iter())
                .filter(|((_, a), (_, b))| a != b)
                .map(|((n, a), (_, b))| format!("{n} {a} vs {b}"))
                .collect();
            names.join(", ")
        };
        panic!(
            "{what}: first divergence at step {} (walk-reference vs scheduler): {field}",
            r.step
        );
    }
}

/// The whole gate: build both sides, run both, compare.
///
/// `build` is called twice and must produce an otherwise-identical machine,
/// differing only in whether the peripheral under test is pinned to the walk.
/// Load firmware inside it.
pub fn assert_walk_differential<C: Cpu, B, E>(
    what: &str,
    entry: u32,
    steps: u64,
    build: B,
    extra: E,
) -> Vec<Probe>
where
    B: Fn(WalkMode) -> Machine<C>,
    E: Fn(&Machine<C>) -> Vec<(&'static str, u64)>,
{
    let mut walk = build(WalkMode::LegacyWalk);
    let reference = run_probed(&mut walk, entry, steps, &extra);

    let mut sched = build(WalkMode::Scheduler);
    let candidate = run_probed(&mut sched, entry, steps, &extra);

    assert_probes_identical(&reference, &candidate, what);
    reference
}

/// Guard against the vacuous pass: prove the two sides really are different
/// modes before believing they agree.
///
/// Without this, a model whose `uses_scheduler()` returns false — because the
/// bus never attached a clock, or the feature is off, or the migration was
/// reverted — makes [`assert_walk_differential`] compare the walk against
/// itself and report success. That is the exact shape of a gate that ran and
/// told you nothing.
pub fn assert_modes_differ(walk_uses_scheduler: bool, sched_uses_scheduler: bool, what: &str) {
    assert!(
        !walk_uses_scheduler,
        "{what}: the REFERENCE side reports uses_scheduler() — force_legacy_walk() \
         did not take, so this differential compares the scheduler against itself"
    );
    assert!(
        sched_uses_scheduler,
        "{what}: the CANDIDATE side does not report uses_scheduler() — the model \
         is still walk-driven (no CycleClock attached, feature off, or the \
         migration is not in effect), so this differential is vacuous"
    );
}
