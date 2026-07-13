// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ISA-agnostic universal-dispatch JIT framework (speed plan Phase 2).
//!
//! This module is the **shared, architecture-neutral scaffold** for the
//! universal dispatch JIT. It owns everything that does *not* depend on a
//! particular instruction set:
//!
//!   * [`block_cache`] — the flash-PC-keyed block cache, hot-counter
//!     promotion, and invalidate-all-on-flash-write policy.
//!   * [`side_exit`] — the side-exit protocol the compiled blocks use to
//!     hand control back to the interpreter.
//!   * [`frontend`] — the [`IsaFrontend`] trait a per-ISA translator
//!     implements, plus a trivial [`frontend::PassthroughFrontend`] that
//!     side-exits every block to the interpreter (zero codegen).
//!   * [`runtime`] — the [`JitRuntime`] abstraction over the native
//!     `wasmtime` engine and the browser `js_sys::WebAssembly` engine,
//!     plus the imported-`WebAssembly.Memory` binding scheme. Ships with
//!     an [`runtime::InterpreterRuntime`] that always side-exits, so the
//!     whole loop compiles and runs with no code generator present.
//!   * [`fallback`] — the interpreter-fallback hook ([`fallback::JitHost`])
//!     and the [`fallback::SafetyGate`] correctness rail (probes /
//!     observers / breakpoints / cycle-accurate mode force the
//!     interpreter).
//!   * [`dispatch`] — the chaining dispatch loop that ties the above
//!     together.
//!   * [`differential`] — the lockstep JIT-vs-interpreter equivalence
//!     harness scaffold (the merge gate's correctness proof).
//!
//! ## What this scaffold deliberately does NOT do
//!
//! No per-ISA instruction translation lives here. There is exactly one
//! frontend impl — [`frontend::PassthroughFrontend`] — and it emits no
//! code; it refuses every block so the dispatcher falls back to the
//! interpreter. The real Thumb-2 / RISC-V / Xtensa frontends (and the
//! `wasmtime` / `js_sys` runtime backends) land in later phases, gated on
//! the post-campaign CPU-share measurement. See
//! `docs/engineering/universal-jit-framework.md` for the full design,
//! ordering, and merge bar.
//!
//! Everything here is behind the `jit-framework` cargo feature; the
//! default build and the existing `jit` (Xtensa pilot) path are
//! untouched.

pub mod block_cache;
pub mod differential;
pub mod dispatch;
pub mod fallback;
pub mod frontend;
pub mod riscv;
pub mod runtime;
pub mod side_exit;

/// Architecture-neutral guest program counter. Wide enough for every
/// target LabWired models (Cortex-M / RISC-V 32-bit, Xtensa 32-bit) with
/// headroom for 64-bit address spaces.
pub type Pc = u64;

/// A read-only view of flash-resident guest code covering a candidate
/// block entry. The JIT only ever compiles code that lives in
/// (immutable-after-config) flash, so this slice is stable for the life of
/// a compiled block — see the invalidate-all-on-flash-write policy in
/// [`block_cache`].
#[derive(Debug, Clone, Copy)]
pub struct CodeView<'a> {
    /// Guest address of `bytes[0]`.
    pub base: Pc,
    /// Raw code bytes starting at `base`.
    pub bytes: &'a [u8],
}

impl<'a> CodeView<'a> {
    /// Construct a view.
    pub fn new(base: Pc, bytes: &'a [u8]) -> Self {
        Self { base, bytes }
    }

    /// Whether `pc` falls inside this view.
    pub fn covers(&self, pc: Pc) -> bool {
        pc >= self.base && (pc - self.base) < self.bytes.len() as u64
    }

    /// Bytes starting at `pc`, or `None` if `pc` is outside the view.
    pub fn from(&self, pc: Pc) -> Option<&'a [u8]> {
        if self.covers(pc) {
            Some(&self.bytes[(pc - self.base) as usize..])
        } else {
            None
        }
    }
}

/// A flattened snapshot of architectural state used by the differential
/// harness to compare an interpreter run against a JIT run. The per-ISA
/// frontend decides the word layout (typically: PC, then the GPR file,
/// then any status word the ISA needs for equivalence). The harness treats
/// it opaquely — it only checks equality position-by-position.
pub type StateVec = Vec<u32>;
