// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod cortex_m;
pub mod riscv;
pub mod xtensa_regs;
pub mod xtensa_sr;

pub use cortex_m::CortexM;
pub use riscv::RiscV;
