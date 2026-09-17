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
    // RV32M Extension
    // Single call site in `step`. Left out of line, a per-class split costs a call and
    // a second variant match per instruction; the same split measured +13.7% Ir/step
    // on every Cortex-M board in the core-perf batch gate. Inlining keeps the codegen
    // of the pre-split function while the source stays split.
    #[inline(always)]
    pub(in crate::cpu::riscv) fn exec_muldiv(&mut self, instruction: Instruction) -> SimResult<()> {
        match instruction {
            Instruction::Mul { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1).wrapping_mul(self.read_reg(rs2));
                self.write_reg(rd, res);
            }
            Instruction::Mulh { rd, rs1, rs2 } => {
                let res = (self.read_reg(rs1) as i32 as i64)
                    .wrapping_mul(self.read_reg(rs2) as i32 as i64);
                self.write_reg(rd, (res >> 32) as u32);
            }
            Instruction::Mulhsu { rd, rs1, rs2 } => {
                let res = (self.read_reg(rs1) as i32 as i64)
                    .wrapping_mul(self.read_reg(rs2) as u64 as i64);
                self.write_reg(rd, (res >> 32) as u32);
            }
            Instruction::Mulhu { rd, rs1, rs2 } => {
                let res = (self.read_reg(rs1) as u64).wrapping_mul(self.read_reg(rs2) as u64);
                self.write_reg(rd, (res >> 32) as u32);
            }
            Instruction::Div { rd, rs1, rs2 } => {
                let dividend = self.read_reg(rs1) as i32;
                let divisor = self.read_reg(rs2) as i32;
                let res = if divisor == 0 {
                    -1
                } else if dividend == i32::MIN && divisor == -1 {
                    dividend
                } else {
                    dividend / divisor
                };
                self.write_reg(rd, res as u32);
            }
            Instruction::Divu { rd, rs1, rs2 } => {
                let dividend = self.read_reg(rs1);
                let divisor = self.read_reg(rs2);
                let res = dividend.checked_div(divisor).unwrap_or(u32::MAX);
                self.write_reg(rd, res);
            }
            Instruction::Rem { rd, rs1, rs2 } => {
                let dividend = self.read_reg(rs1) as i32;
                let divisor = self.read_reg(rs2) as i32;
                let res = if divisor == 0 {
                    dividend
                } else if dividend == i32::MIN && divisor == -1 {
                    0
                } else {
                    dividend % divisor
                };
                self.write_reg(rd, res as u32);
            }
            Instruction::Remu { rd, rs1, rs2 } => {
                let dividend = self.read_reg(rs1);
                let divisor = self.read_reg(rs2);
                let res = if divisor == 0 {
                    dividend
                } else {
                    dividend % divisor
                };
                self.write_reg(rd, res);
            }
            _ => unreachable!("exec_muldiv called with instruction from a different class"),
        }
        Ok(())
    }
}
