// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Side-exit protocol.
//!
//! A compiled block never runs "forever". It executes a straight run of
//! translated instructions and then hands control back to the dispatcher
//! with a [`SideExit`]. The dispatcher decides whether to chain into the
//! next compiled block, resume the interpreter, or bubble an exception up
//! to the [`crate::Machine`].
//!
//! This is the ISA-neutral vocabulary. The per-ISA emit backends
//! (Xtensa's existing [`crate::cpu::xtensa_jit_bytes`], future Thumb-2 /
//! RISC-V) encode a matching `i32` wire code inside the emitted wasm body;
//! the runtime adapter maps that wire code back onto this enum. Keeping
//! the enum here (rather than raw `i32`s) means the dispatch loop and the
//! differential harness reason about *meaning*, not magic numbers.

use super::Pc;

/// Why a compiled block returned control to the dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideExit {
    /// The block ran to its terminator and control flows on to `next_pc`.
    /// If `next_pc` is itself the entry of a hot compiled block the
    /// dispatcher chains straight into it without touching the
    /// interpreter (the chaining fast path).
    Chain {
        /// Guest PC of the next instruction to execute.
        next_pc: Pc,
    },

    /// The block could not complete in JIT: it hit an instruction the
    /// frontend does not translate, reached the end of a partially
    /// compiled block, or a correctness rail tripped. The interpreter must
    /// resume at `resume_pc` for at least one instruction before the
    /// dispatcher reconsiders the JIT.
    EnterInterpreter {
        /// Guest PC the interpreter must resume from.
        resume_pc: Pc,
        /// Diagnostic reason (never affects correctness — only telemetry
        /// and the "why did this PC never compile" story).
        reason: BailReason,
    },

    /// A synchronous exception / taken interrupt was raised inside the
    /// block. The block has already unwound its own side effects up to the
    /// faulting instruction; the dispatcher hands `cause` to the machine's
    /// exception path and resumes interpretation at `resume_pc`.
    Exception {
        /// Guest PC of the faulting instruction (the exception PC).
        resume_pc: Pc,
        /// Architecture-specific exception / interrupt cause code.
        cause: u32,
    },
}

impl SideExit {
    /// Convenience: an "unsupported instruction at `pc`" bail.
    pub fn unsupported(pc: Pc) -> Self {
        SideExit::EnterInterpreter {
            resume_pc: pc,
            reason: BailReason::UnsupportedInstruction,
        }
    }

    /// The PC the machine should observe after this exit resolves. For a
    /// chain it is the chained target; otherwise it is the resume PC.
    pub fn continuation_pc(&self) -> Pc {
        match self {
            SideExit::Chain { next_pc } => *next_pc,
            SideExit::EnterInterpreter { resume_pc, .. } => *resume_pc,
            SideExit::Exception { resume_pc, .. } => *resume_pc,
        }
    }

    /// Whether the dispatcher must drop to the interpreter for this exit.
    pub fn needs_interpreter(&self) -> bool {
        !matches!(self, SideExit::Chain { .. })
    }
}

/// Diagnostic classification for an [`SideExit::EnterInterpreter`]. Purely
/// informational — the fallback path is always correct regardless of the
/// reason, so this only drives telemetry and coverage reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BailReason {
    /// Frontend does not (yet) translate an instruction in the block.
    UnsupportedInstruction,
    /// The block was compiled only up to some cut point (e.g. a call into
    /// a non-compiled region) and ran out of translated body.
    PartialBlock,
    /// A load/store resolved to a peripheral / MMIO address the block does
    /// not model, so it must be replayed on the interpreter's bus.
    MemoryFault,
    /// A correctness rail ([`super::fallback::SafetyGate`]) was active, so
    /// the block refused to run and deferred to the interpreter.
    SafetyGate,
    /// The passthrough frontend refuses everything (scaffold only).
    Passthrough,
}
