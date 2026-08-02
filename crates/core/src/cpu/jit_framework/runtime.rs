// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Execution-runtime abstraction: native `wasmtime` vs browser
//! `js_sys::WebAssembly`, behind one interface.
//!
//! A [`JitRuntime`] turns a [`BlockPlan`] (runtime-neutral wasm bytes +
//! metadata) into an executable [`JitRuntime::Artifact`] and runs it,
//! returning a [`SideExit`]. Two production runtimes are planned:
//!
//!   * **Native** — `wasmtime`. Emitted modules import the engine's own
//!     backing memory and get compiled to host machine code.
//!   * **Browser** — `js_sys::WebAssembly::{Module, Instance}`. Emitted
//!     modules import the engine's own `WebAssembly.Memory`.
//!
//! Only the [`InterpreterRuntime`] ships here: it produces a no-op
//! artifact and always side-exits to the interpreter, which is exactly
//! what the passthrough frontend needs to prove the loop end-to-end
//! without a code generator.
//!
//! ## The memory binding — why it kills the host-import ceiling
//!
//! The Xtensa pilot dispatched every load/store through a **host import**
//! (`host.read_u8` / `host.store_u8`). Measured against `invaders` in the
//! browser that capped speedup at ~1.5–2×: each memory access crossed the
//! JS⇄wasm boundary, and that boundary crossing — not the arithmetic —
//! dominated.
//!
//! The fix is [`MemoryBinding`]: emitted blocks import the **engine's own
//! backing memory** and issue plain `i32.load` / `i32.store` against it.
//!
//!   * **Browser** — the engine's `LinearMemory` (RAM) / flash `Vec<u8>`s
//!     are exposed as a `WebAssembly.Memory`, imported into every emitted
//!     module. A guest load is one wasm memory op at
//!     `guest_addr - guest_base`, no host call. This is sound because the
//!     `LinearMemory` / flash `Vec`s never reallocate after config, so the
//!     base offset the emitter bakes in stays valid for the whole run
//!     (and the block cache is dropped on the rare flash write anyway).
//!   * **Native** — `wasmtime` maps the same region as linear memory (or
//!     the runtime resolves a raw base pointer into the `LinearMemory`
//!     `Vec` at instantiate time). Same emitted op, no host trampoline.
//!
//! Peripheral / MMIO addresses are *not* in the imported region: the
//! emitter range-checks against the RAM/flash window and emits a
//! [`SideExit::EnterInterpreter`] with [`BailReason::MemoryFault`] for
//! anything outside it, so MMIO still goes through the real `Bus`.

use super::frontend::BlockPlan;
use super::side_exit::{BailReason, SideExit};
use super::Pc;

/// How a compiled block reaches guest RAM/flash. Constructed by the engine
/// once per run (memory doesn't move) and handed to
/// [`JitRuntime::instantiate`] so the emitted module can import the right
/// backing store. Deliberately pointer-free at this layer — the concrete
/// runtime resolves the actual pointer / `WebAssembly.Memory` object
/// internally so this type stays `Send + Sync` and dependency-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryBinding {
    /// Native (`wasmtime`): the emitted module imports linear memory backed
    /// by the engine's `LinearMemory` `Vec`. `guest_base` is the guest
    /// address that maps to wasm-memory offset 0 for this region; `len` is
    /// the region size.
    NativeLinear { guest_base: Pc, len: usize },
    /// Browser (`js_sys`): the emitted module imports the engine's own
    /// `WebAssembly.Memory`. `guest_base` / `len` define the mapped
    /// window; `region` is an opaque index the runtime uses to fetch the
    /// actual `Memory` object (kept opaque so no `js_sys` dep leaks here).
    BrowserSharedMemory {
        guest_base: Pc,
        len: usize,
        region: u32,
    },
}

impl MemoryBinding {
    /// Guest base address mapped to offset 0 of the imported memory.
    pub fn guest_base(&self) -> Pc {
        match self {
            MemoryBinding::NativeLinear { guest_base, .. } => *guest_base,
            MemoryBinding::BrowserSharedMemory { guest_base, .. } => *guest_base,
        }
    }

    /// Whether `addr` falls in the directly-addressable window. Addresses
    /// outside it (MMIO / peripherals) must side-exit to the `Bus`.
    pub fn contains(&self, addr: Pc) -> bool {
        let (base, len) = match self {
            MemoryBinding::NativeLinear { guest_base, len } => (*guest_base, *len),
            MemoryBinding::BrowserSharedMemory {
                guest_base, len, ..
            } => (*guest_base, *len),
        };
        addr >= base && (addr - base) < len as u64
    }
}

/// Failure to instantiate a plan into an executable artifact. Never a
/// correctness problem — the dispatcher falls back to the interpreter for
/// that PC (and may retry later).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// The emitted wasm bytes failed to validate / compile.
    CompileFailed(String),
    /// The runtime backend is not available in this build target.
    Unavailable,
}

/// One executable, runnable compiled block. The concrete type depends on
/// the runtime (a `wasmtime` `TypedFunc`+`Store`, or a cached
/// `js_sys::Function`). The dispatcher only ever [`JitRuntime::run`]s it.
pub trait JitArtifact {
    /// The guest PC this artifact is entered at.
    fn entry_pc(&self) -> Pc;
}

/// Turns [`BlockPlan`]s into runnable artifacts and executes them. One
/// impl per execution engine.
pub trait JitRuntime {
    /// The runtime-specific compiled artifact.
    type Artifact: JitArtifact;

    /// Short backend identifier (`"wasmtime"`, `"js-webassembly"`,
    /// `"interpreter"`).
    fn backend_name(&self) -> &'static str;

    /// Instantiate a plan against the engine memory binding. `mem` tells
    /// the emitted module which backing store to import (see
    /// [`MemoryBinding`]).
    fn instantiate(
        &mut self,
        plan: &BlockPlan,
        mem: &MemoryBinding,
    ) -> Result<Self::Artifact, RuntimeError>;

    /// Run a compiled block to its next side-exit. Register marshalling
    /// (loading the guest register file into the block's params and
    /// writing results back) is the runtime's responsibility and is
    /// elided from this scaffold signature — the real runtimes take a
    /// register-file handle here.
    fn run(&mut self, artifact: &mut Self::Artifact) -> SideExit;
}

// ── Interpreter runtime — the zero-codegen fallback ───────────────────

/// A no-op artifact: it holds only the entry PC and always side-exits.
#[derive(Debug, Clone, Copy)]
pub struct InterpreterArtifact {
    entry_pc: Pc,
}

impl JitArtifact for InterpreterArtifact {
    fn entry_pc(&self) -> Pc {
        self.entry_pc
    }
}

/// The always-side-exit runtime. Every artifact it produces immediately
/// bails to the interpreter. Paired with
/// [`super::frontend::PassthroughFrontend`] it exercises the entire
/// framework — cache, dispatch, instantiate, run, side-exit, fallback —
/// with no instruction translation at all. It is also the honest runtime
/// for build targets where neither `wasmtime` nor `js_sys` is available.
#[derive(Debug, Clone, Copy, Default)]
pub struct InterpreterRuntime;

impl JitRuntime for InterpreterRuntime {
    type Artifact = InterpreterArtifact;

    fn backend_name(&self) -> &'static str {
        "interpreter"
    }

    fn instantiate(
        &mut self,
        plan: &BlockPlan,
        _mem: &MemoryBinding,
    ) -> Result<Self::Artifact, RuntimeError> {
        Ok(InterpreterArtifact {
            entry_pc: plan.entry_pc,
        })
    }

    fn run(&mut self, artifact: &mut Self::Artifact) -> SideExit {
        SideExit::EnterInterpreter {
            resume_pc: artifact.entry_pc,
            reason: BailReason::Passthrough,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_binding_window_check() {
        let b = MemoryBinding::NativeLinear {
            guest_base: 0x2000_0000,
            len: 0x1000,
        };
        assert!(b.contains(0x2000_0000));
        assert!(b.contains(0x2000_0FFF));
        assert!(!b.contains(0x2000_1000)); // one past end -> MMIO territory
        assert!(!b.contains(0x1FFF_FFFF));
        assert_eq!(b.guest_base(), 0x2000_0000);
    }

    #[test]
    fn interpreter_runtime_always_side_exits() {
        let mut rt = InterpreterRuntime;
        let plan = BlockPlan::side_exit_stub(0x400d_1000);
        let mem = MemoryBinding::NativeLinear {
            guest_base: 0x3ffa_e000,
            len: 0x8_0000,
        };
        let mut art = rt.instantiate(&plan, &mem).unwrap();
        assert_eq!(art.entry_pc(), 0x400d_1000);
        assert_eq!(
            rt.run(&mut art),
            SideExit::EnterInterpreter {
                resume_pc: 0x400d_1000,
                reason: BailReason::Passthrough,
            }
        );
    }
}
