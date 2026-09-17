// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Split out of `step` in `cpu/riscv.rs` (pure move, no behaviour change).
// See that file's `step` for the shared fetch/decode/interrupt/pc-commit
// plumbing that wraps this per-class dispatch.

use super::super::RiscV;
use crate::decoder::riscv::Instruction;
use crate::SimResult;

impl RiscV {
    pub(in crate::cpu::riscv) fn exec_misc(&mut self, instruction: Instruction) -> SimResult<()> {
        match instruction {
            Instruction::Unknown(inst) => {
                tracing::error!("Unknown instruction {:#x} at {:#x}", inst, self.pc);
                Err(crate::SimulationError::DecodeError(self.pc as u64))
            }
            _ => unreachable!("exec_misc called with instruction from a different class"),
        }
    }
}
