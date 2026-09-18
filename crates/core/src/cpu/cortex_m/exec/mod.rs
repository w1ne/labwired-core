// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// `step_execute`'s instruction arms, moved by class. `step_execute` keeps the
// one and only `match instruction`; every non-trivial arm there is a one-line
// call into one of these modules. PC advance travels as the `PcAdvance`
// return value (no `&mut` out-parameters), and `it_block_instruction` is
// passed by value only to the arms that read it.

pub(in crate::cpu::cortex_m) mod alu;
pub(in crate::cpu::cortex_m) mod branch;
pub(in crate::cpu::cortex_m) mod load_store;
pub(in crate::cpu::cortex_m) mod misc;
pub(in crate::cpu::cortex_m) mod shift_mul;
pub(in crate::cpu::cortex_m) mod system;
pub(in crate::cpu::cortex_m) mod vfp;
