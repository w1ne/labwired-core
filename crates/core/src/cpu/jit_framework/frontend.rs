// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Per-ISA frontend interface.
//!
//! A frontend is the *only* ISA-aware part of the JIT. Given a candidate
//! block entry PC and a read-only view of the flash-resident code around
//! it, it walks a basic block and emits a [`BlockPlan`] — the runtime-
//! neutral description of a compiled block (emitted wasm bytes, metadata,
//! and the side-exit map). The [`super::runtime::JitRuntime`] then turns the
//! plan into an executable artifact for either the native or browser
//! engine.
//!
//! ## Order of implementation (see design doc)
//!
//! 1. **Thumb-2** (Cortex-M) — the widest board coverage in LabWired and
//!    a clean fixed/16-32-bit encoding; the emit is the simplest and the
//!    most reusable across the STM32 / nRF / RP2040 families.
//! 2. **RISC-V** (RV32IMC, ESP32-C3) — regular encoding, no register
//!    windows; validates the framework on a second, structurally
//!    different ISA.
//! 3. **Xtensa** (LX7, ESP32-S3/classic) — hardest last: register
//!    windows (CALL8/RETW), the `L32R` literal pool, and the existing
//!    pilot in [`crate::cpu::xtensa_jit`] to absorb as the reference
//!    frontend.
//!
//! Only the [`PassthroughFrontend`] ships in this scaffold.

use super::side_exit::BailReason;
use super::{CodeView, Pc};

/// The ISA-specific translator. One impl per instruction set.
pub trait IsaFrontend {
    /// Short ISA identifier for diagnostics and the differential-harness
    /// report (e.g. `"thumb2"`, `"rv32imc"`, `"xtensa-lx7"`).
    fn isa_name(&self) -> &'static str;

    /// Walk a basic block starting at `pc` over `code` and produce a
    /// [`BlockPlan`], or refuse with a [`FrontendRefusal`].
    ///
    /// Contract:
    ///   * The frontend MUST only translate flash-resident, position-
    ///     stable code (the block cache invalidates on any flash write).
    ///   * The frontend MUST terminate the walk at the first control-flow
    ///     instruction it fully models, or cut the block (partial) at the
    ///     first instruction it does not model — emitting an
    ///     [`super::side_exit::SideExit::EnterInterpreter`] there.
    ///   * A refusal is never an error: the dispatcher simply keeps the PC
    ///     on the interpreter.
    fn translate_block(&self, pc: Pc, code: &CodeView<'_>) -> Result<BlockPlan, FrontendRefusal>;
}

/// Runtime-neutral description of one compiled block. Both the native
/// (`wasmtime`) and browser (`js_sys::WebAssembly`) runtimes consume the
/// identical `code` byte stream — that byte-for-byte sharing is the whole
/// point of splitting translation (here) from execution
/// ([`super::runtime`]).
#[derive(Debug, Clone)]
pub struct BlockPlan {
    /// Guest PC this block is entered at (the cache key).
    pub entry_pc: Pc,
    /// Guest PC one past the last translated instruction — the natural
    /// fall-through continuation.
    pub end_pc: Pc,
    /// Number of guest instructions the block retires when it runs to the
    /// terminator (used to advance the cycle counter in one shot).
    pub instr_count: u32,
    /// Emitted WebAssembly module bytes. Empty for the passthrough stub,
    /// which carries no body and side-exits at entry.
    pub code: Vec<u8>,
    /// Static description of every side-exit edge the block can take,
    /// indexed by the `i32` wire code the emitted body returns.
    pub exits: Vec<ExitEdge>,
}

impl BlockPlan {
    /// A zero-body stub that, when "run", immediately side-exits back to
    /// the interpreter at `entry_pc`. This is what the passthrough
    /// frontend produces: it exercises the full cache + dispatch + runtime
    /// + fallback loop end-to-end with no code generation whatsoever.
    pub fn side_exit_stub(entry_pc: Pc) -> Self {
        Self {
            entry_pc,
            end_pc: entry_pc,
            instr_count: 0,
            code: Vec::new(),
            exits: vec![ExitEdge {
                wire_code: 0,
                reason: BailReason::Passthrough,
            }],
        }
    }

    /// Whether this plan carries emitted code. `false` for the
    /// passthrough stub.
    pub fn is_stub(&self) -> bool {
        self.code.is_empty()
    }
}

/// One static side-exit edge out of a compiled block. Maps the `i32` wire
/// code the emitted body returns to the [`BailReason`] the dispatcher
/// reports. Concrete branch targets are resolved at runtime from the
/// register file, so only the *classification* is static.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitEdge {
    /// The `i32` value the emitted wasm body returns for this edge.
    pub wire_code: i32,
    /// What this edge means to the dispatcher.
    pub reason: BailReason,
}

/// Reason a frontend declined to translate a block. Never an error — the
/// PC just stays on the interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendRefusal {
    /// The block shape / opcode set is not translated by this frontend.
    Unsupported,
    /// The supplied [`CodeView`] does not cover `pc`.
    PcOutOfRange,
    /// The block degenerates to a single terminator — not worth compiling.
    BlockTooShort,
}

/// The scaffold's only frontend: it translates nothing. Every block is a
/// [`BlockPlan::side_exit_stub`], so the dispatcher always falls back to
/// the interpreter. Its job is to prove the framework compiles and the
/// full loop runs with zero codegen — the real per-ISA frontends replace
/// it phase by phase.
#[derive(Debug, Clone, Copy, Default)]
pub struct PassthroughFrontend;

impl IsaFrontend for PassthroughFrontend {
    fn isa_name(&self) -> &'static str {
        "passthrough"
    }

    fn translate_block(&self, pc: Pc, code: &CodeView<'_>) -> Result<BlockPlan, FrontendRefusal> {
        if !code.covers(pc) {
            return Err(FrontendRefusal::PcOutOfRange);
        }
        // Emit a body-less stub that side-exits at entry. This is the
        // "compiles + runs, zero codegen" fallback path.
        Ok(BlockPlan::side_exit_stub(pc))
    }
}
