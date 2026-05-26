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

// Phase 3.2 JIT pilot (issue #124). Behind the `jit` feature so wasmtime
// only enters the dep graph when explicitly opted-in. The browser build
// path (`labwired-wasm`) deliberately does NOT enable this feature.
#[cfg(feature = "jit")]
pub mod xtensa_jit;

// Phase 4 (#124): pre-compiled wasm bytes + architectural constants for
// the hot BB, shared between native and browser JIT backends. Always
// compiled (no feature gate) so the browser crate can use them without
// pulling in wasmtime.
pub mod xtensa_jit_bytes;

pub use cortex_m::CortexM;
pub use riscv::RiscV;
pub use xtensa_lx7::XtensaLx7;
