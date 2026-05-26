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

pub use cortex_m::CortexM;
pub use riscv::RiscV;
pub use xtensa_lx7::XtensaLx7;
