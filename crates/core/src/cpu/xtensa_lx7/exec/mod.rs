// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `execute`'s arm bodies, moved by instruction class. `execute` keeps the
//! one and only `match ins`; every non-trivial arm there is a one-line call
//! into one of these modules.

pub(in crate::cpu::xtensa_lx7) mod alu;
pub(in crate::cpu::xtensa_lx7) mod branch_jump;
pub(in crate::cpu::xtensa_lx7) mod call_window;
pub(in crate::cpu::xtensa_lx7) mod fp;
pub(in crate::cpu::xtensa_lx7) mod load_store;
pub(in crate::cpu::xtensa_lx7) mod misc;
pub(in crate::cpu::xtensa_lx7) mod shift_mul;
pub(in crate::cpu::xtensa_lx7) mod system;
