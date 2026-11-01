// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

// A discarded bus access is a fabricated fact: `bus/accessors.rs` returns
// `Err(SimulationError::MemoryViolation(addr))` for unmapped space, and a core
// that throws that away runs firmware that would fault on silicon and reports a
// green verdict. `let _ = bus.write_*` is how that happened 25 times in
// `cortex_m.rs`. `Result` is `#[must_use]`, so denying
// `clippy::let_underscore_must_use` here makes reintroducing the shape a
// compile error under CI's `cargo clippy --all-targets -- -D warnings`.
//
// Scoped to the modules that hold the line rather than to `cpu/**`: `xtensa_lx7`
// still carries four discarded stores (see the shrink-only allowlist in
// `crate::tests::cortex_m_memory_contract`, which covers the whole `cpu/` tree
// and catches the `is_err()` / `.ok()` discard shapes clippy cannot see).
mod trait_def;

pub(crate) use trait_def::{advance_batch_cycle, default_step_batch, default_step_batch_with};
pub use trait_def::{Cpu, CpuJitStats, SecondaryExecutionState};

#[deny(clippy::let_underscore_must_use)]
pub mod aarch64;
#[deny(clippy::let_underscore_must_use)]
pub mod aarch64_gic;
#[deny(clippy::let_underscore_must_use)]
pub mod avr;
#[cfg(all(feature = "qemu-arm", not(target_arch = "wasm32")))]
#[deny(clippy::let_underscore_must_use)]
pub mod cortex_a;
#[deny(clippy::let_underscore_must_use)]
pub mod cortex_m;
#[deny(clippy::let_underscore_must_use)]
pub mod pic16;
#[deny(clippy::let_underscore_must_use)]
pub mod pic18;
#[deny(clippy::let_underscore_must_use)]
pub mod pic24;
#[cfg(all(feature = "qemu-arm", not(target_arch = "wasm32")))]
pub use cortex_a::CortexA7;
#[deny(clippy::let_underscore_must_use)]
pub mod riscv;
#[deny(clippy::let_underscore_must_use)]
pub mod riscv64;
#[deny(clippy::let_underscore_must_use)]
pub mod rx;
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
// RISC-V and Cortex-M frontends' native `wasmtime` executors into
// `Machine<RiscV>` / `Machine<CortexM>` dispatch, so the production `jit`
// build needs this module present too. Both features are off by default, so the
// default build is unaffected. See
#[cfg(any(feature = "jit", feature = "jit-framework"))]
pub mod jit_framework;

pub use aarch64::Aarch64;
pub use avr::{Avr, AvrModel};
pub use cortex_m::CortexM;
pub use pic16::Pic16;
pub use pic18::Pic18;
pub use pic24::Pic24;
pub use riscv::{RiscV, RiscVCoreProfile};
pub use riscv64::RiscV64;
pub use rx::Rx;
pub use xtensa_lx7::XtensaLx7;

#[cfg(feature = "qemu-arm-system")]
pub mod cortex_a_system;
#[cfg(feature = "qemu-arm-system")]
pub use cortex_a_system::{ArmModel, CortexA7System};
