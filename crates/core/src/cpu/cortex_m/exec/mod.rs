// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// `step_execute`'s instruction match, split by instruction class. Each
// submodule holds a verbatim move of the arms for its class out of
// `cpu/cortex_m.rs`; the dispatch and all exception/IT-state plumbing stays
// in `step_execute` itself.

pub(in crate::cpu::cortex_m) mod alu;
pub(in crate::cpu::cortex_m) mod branch;
pub(in crate::cpu::cortex_m) mod load_store;
pub(in crate::cpu::cortex_m) mod misc;
pub(in crate::cpu::cortex_m) mod shift_mul;
pub(in crate::cpu::cortex_m) mod system;
pub(in crate::cpu::cortex_m) mod vfp;
