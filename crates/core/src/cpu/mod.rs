// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod cortex_m;
pub mod riscv;
pub mod xtensa_lockstep;
pub mod xtensa_lx7;
pub mod xtensa_regs;
pub mod xtensa_sr;

// Phase 3.2 JIT pilot (issue #124). The wasmtime-backed adapters inside
// this module are themselves `#[cfg(feature = "jit")]`-gated (see
// `xtensa_jit::bb_multi` / `xtensa_jit::windowed_call`) so the browser
// build path (`labwired-wasm`) — which deliberately does NOT enable
// `jit` — still gets access to the runtime-agnostic `emit_core`
// submodule. Phase 4.1 lifted the gate from the module declaration so
// the walker + emit core are visible without a wasmtime dep.
pub mod xtensa_jit;

// Phase 4 (#124): pre-compiled wasm bytes + architectural constants for
// the hot BB, shared between native and browser JIT backends. Always
// compiled (no feature gate) so the browser crate can use them without
// pulling in wasmtime.
pub mod xtensa_jit_bytes;

// Speed plan Phase 2 (#124 follow-on): ISA-agnostic universal-dispatch JIT
// framework. This is the shared, architecture-neutral scaffold — block
// cache, side-exit protocol, per-ISA frontend trait, native/browser
// runtime abstraction, interpreter-fallback hooks, and the differential
// harness. It carries NO per-ISA codegen: the only frontend is a
// passthrough that side-exits every block to the interpreter.
//
// Compiled under EITHER `jit-framework` (the pure-Rust scaffold + lockstep
// tests) OR `jit` (the production/wasm feature set): chunk H wires the
// RISC-V frontend's native `wasmtime` executor (`riscv::exec`, itself gated
// on `jit`) into `Machine<RiscV>`'s dispatch, so the production `jit` build
// needs this module present too. Both features are off by default, so the
// default build is unaffected. See
// `docs/engineering/universal-jit-framework.md`.
#[cfg(any(feature = "jit", feature = "jit-framework"))]
pub mod jit_framework;

pub use cortex_m::CortexM;
pub use riscv::RiscV;
pub use xtensa_lx7::XtensaLx7;
