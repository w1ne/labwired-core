// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// `step`'s instruction match, split by instruction class. Each submodule
// holds a verbatim move of the arms for its class out of `cpu/riscv.rs`; the
// fetch/decode, interrupt/trap entry, cycle accounting and pc-commit
// plumbing stays in `step` itself.

pub(in crate::cpu::riscv) mod alu;
pub(in crate::cpu::riscv) mod atomic;
pub(in crate::cpu::riscv) mod branch;
pub(in crate::cpu::riscv) mod load_store;
pub(in crate::cpu::riscv) mod misc;
pub(in crate::cpu::riscv) mod muldiv;
pub(in crate::cpu::riscv) mod system;
