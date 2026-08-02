// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Interpreter-fallback hook and the correctness safety gate.
//!
//! The JIT is a *speed* optimization layered on top of the interpreter,
//! which stays the single source of truth for behavior. Two mechanisms
//! keep it honest:
//!
//!   * [`SafetyGate`] — a per-dispatch check that forces the interpreter
//!     whenever anything is watching cycle-by-cycle or instruction-by-
//!     instruction (probes, observers, breakpoints, cycle-accurate mode).
//!   * [`JitHost`] — the interface back to the machine: run one
//!     interpreter instruction, fetch flash code, snapshot state, and read
//!     the current safety gate. The dispatch loop drives the machine
//!     entirely through this trait so the framework never reaches into a
//!     concrete `Cpu`/`Bus`.

use super::{Pc, StateVec};

/// Snapshot of the observation surface that must force interpretation.
///
/// The JIT compiles *away* per-instruction structure: a block retires many
/// guest instructions as one host call, advancing the cycle counter in a
/// lump and never invoking `on_step_start` / `on_memory_write` per
/// instruction. That is invisible to a bulk run, but wrong the moment
/// something needs per-instruction granularity. So whenever any of these
/// is active the dispatcher runs the interpreter instead — correctness
/// beats speed, unconditionally.
///
/// How each is detected on the real machine (recomputed each dispatch
/// entry so toggling mid-run bails immediately):
///   * `observers_active` — `!observers.is_empty()` (the `&[Arc<dyn
///     SimulationObserver>]` passed to `Cpu::step`). Any observer wants
///     `on_step_start`/`on_step_end`/`on_memory_write` per instruction.
///   * `breakpoints_active` — the machine's breakpoint set is non-empty;
///     a compiled block could step over a breakpoint address without
///     stopping.
///   * `probes_active` — any logic-analyzer / signal probe / DAP watch is
///     armed (`bus.logic_tap().is_some()` and friends); these need
///     per-cycle pad visibility the JIT elides.
///   * `cycle_accurate` — cycle-accurate mode is selected (as opposed to
///     the batched instruction-throughput mode); the JIT only models
///     retirement, not per-cycle timing.
///
/// A running block that observes the gate flip mid-flight side-exits via
/// [`super::side_exit::BailReason::SafetyGate`]; the *next* dispatch entry
/// sees `jit_allowed() == false` and stays on the interpreter until the
/// gate clears.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SafetyGate {
    pub observers_active: bool,
    pub breakpoints_active: bool,
    pub probes_active: bool,
    pub cycle_accurate: bool,
}

impl SafetyGate {
    /// The JIT may run only when nothing needs per-instruction /
    /// per-cycle visibility.
    pub fn jit_allowed(&self) -> bool {
        !(self.observers_active
            || self.breakpoints_active
            || self.probes_active
            || self.cycle_accurate)
    }
}

/// Outcome of one interpreter step driven through [`JitHost::interpret_one`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStep {
    /// The machine advanced one instruction; keep dispatching.
    Advanced,
    /// The machine reached a stopping condition (halt, breakpoint, error).
    /// The dispatch loop returns.
    Halted,
}

/// The framework's view of the machine. The dispatch loop drives the guest
/// exclusively through this trait, so the JIT stays fully decoupled from
/// the concrete `Cpu` / `Bus` / `Machine` types (the eventual adapter
/// implements `JitHost` for `Machine<C>`).
pub trait JitHost {
    /// Current guest program counter.
    fn pc(&self) -> Pc;

    /// Run exactly one instruction on the interpreter — the correctness
    /// reference path. This is the universal fallback: every unsupported
    /// instruction, partial block, and tripped safety gate routes here.
    fn interpret_one(&mut self) -> HostStep;

    /// Advance the guest to `pc` after a compiled block chained past a run
    /// of instructions (the JIT already applied their effects; this just
    /// moves the interpreter's PC to the continuation).
    fn resume_at(&mut self, pc: Pc);

    /// Materialise up to a block's worth of contiguous guest code bytes
    /// starting at `pc`, fetched through the SAME path instruction fetch uses
    /// (so XIP/MMU translation is applied), or `None` if `pc` is not in a
    /// compilable code region. The dispatch loop builds a
    /// [`CodeView`](super::CodeView) with `base = pc` from the result. Owned
    /// (rather than a borrowed slice) so the fetch can translate through an
    /// MMU-mapped window into a freshly materialised buffer.
    fn code_bytes(&self, pc: Pc) -> Option<Vec<u8>>;

    /// The current safety gate — recomputed every call so a mid-run toggle
    /// (a debugger attaching, a probe arming) takes effect on the next
    /// dispatch entry.
    fn safety(&self) -> SafetyGate;

    /// Flattened architectural state for the differential harness (PC +
    /// register file + any status word the ISA needs for equivalence).
    fn snapshot_state(&self) -> StateVec;

    /// Whether a flash write has occurred since the last poll. The
    /// dispatcher calls this and, on `true`, invalidates the whole block
    /// cache before the next lookup.
    fn take_flash_dirty(&mut self) -> bool;
}
