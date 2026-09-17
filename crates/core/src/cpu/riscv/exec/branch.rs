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
    // Single call site in `step`. Left out of line, a per-class split costs a call and
    // a second variant match per instruction; the same split measured +13.7% Ir/step
    // on every Cortex-M board in the core-perf batch gate. Inlining keeps the codegen
    // of the pre-split function while the source stays split.
    #[inline(always)]
    pub(in crate::cpu::riscv) fn exec_branch(
        &mut self,
        instruction: Instruction,
        inst_len: u32,
        next_pc: &mut u32,
    ) -> SimResult<()> {
        match instruction {
            Instruction::Jal { rd, imm } => {
                let target = self.pc.wrapping_add(imm as u32);
                // Link address is the NEXT instruction: pc + inst_len. The
                // decoder maps the 2-byte C.JAL to Jal, so a hardcoded +4 would
                // set ra 2 bytes too far and corrupt every compressed call's
                // return — use inst_len so c.jal links pc+2 and jal links pc+4.
                self.write_reg(rd, self.pc.wrapping_add(inst_len));
                *next_pc = target;
            }
            Instruction::Jalr { rd, rs1, imm } => {
                let base = self.read_reg(rs1);
                let target = base.wrapping_add(imm as u32) & !1;
                self.write_reg(rd, self.pc.wrapping_add(inst_len));
                *next_pc = target;
            }
            Instruction::Beq { rs1, rs2, imm } => {
                if self.read_reg(rs1) == self.read_reg(rs2) {
                    *next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bne { rs1, rs2, imm } => {
                if self.read_reg(rs1) != self.read_reg(rs2) {
                    *next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Blt { rs1, rs2, imm } => {
                if (self.read_reg(rs1) as i32) < (self.read_reg(rs2) as i32) {
                    *next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bge { rs1, rs2, imm } => {
                if (self.read_reg(rs1) as i32) >= (self.read_reg(rs2) as i32) {
                    *next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bltu { rs1, rs2, imm } => {
                if self.read_reg(rs1) < self.read_reg(rs2) {
                    *next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bgeu { rs1, rs2, imm } => {
                if self.read_reg(rs1) >= self.read_reg(rs2) {
                    *next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::CJr { rs1 } => {
                *next_pc = self.read_reg(rs1) & !1;
            }
            Instruction::CJalr { rs1 } => {
                let target = self.read_reg(rs1) & !1;
                self.write_reg(1, self.pc.wrapping_add(2));
                *next_pc = target;
            }
            Instruction::CJ { imm } => {
                *next_pc = self.pc.wrapping_add(imm as u32);
            }
            Instruction::CBeqz { rs1, imm } => {
                if self.read_reg(rs1) == 0 {
                    *next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::CBnez { rs1, imm } => {
                if self.read_reg(rs1) != 0 {
                    *next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            _ => unreachable!("exec_branch called with instruction from a different class"),
        }
        Ok(())
    }
}
