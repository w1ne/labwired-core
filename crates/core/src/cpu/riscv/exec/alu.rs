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
    #[allow(clippy::too_many_lines)]
    pub(in crate::cpu::riscv) fn exec_alu(&mut self, instruction: Instruction) -> SimResult<()> {
        match instruction {
            Instruction::Lui { rd, imm } => {
                self.write_reg(rd, imm);
            }
            Instruction::Auipc { rd, imm } => {
                let val = self.pc.wrapping_add(imm);
                self.write_reg(rd, val);
            }
            Instruction::Addi { rd, rs1, imm } => {
                let res = self.read_reg(rs1).wrapping_add(imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Slti { rd, rs1, imm } => {
                let val = if (self.read_reg(rs1) as i32) < imm {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Sltiu { rd, rs1, imm } => {
                let val = if self.read_reg(rs1) < (imm as u32) {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Xori { rd, rs1, imm } => {
                let res = self.read_reg(rs1) ^ (imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Ori { rd, rs1, imm } => {
                let res = self.read_reg(rs1) | (imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Andi { rd, rs1, imm } => {
                let res = self.read_reg(rs1) & (imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Slli { rd, rs1, shamt } => {
                let res = self.read_reg(rs1).wrapping_shl(shamt as u32);
                self.write_reg(rd, res);
            }
            Instruction::Srli { rd, rs1, shamt } => {
                let res = self.read_reg(rs1).wrapping_shr(shamt as u32);
                self.write_reg(rd, res);
            }
            Instruction::Srai { rd, rs1, shamt } => {
                let res = (self.read_reg(rs1) as i32).wrapping_shr(shamt as u32);
                self.write_reg(rd, res as u32);
            }
            Instruction::Add { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1).wrapping_add(self.read_reg(rs2));
                self.write_reg(rd, res);
            }
            Instruction::Sub { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1).wrapping_sub(self.read_reg(rs2));
                self.write_reg(rd, res);
            }
            Instruction::Sll { rd, rs1, rs2 } => {
                let shamt = self.read_reg(rs2) & 0x1F;
                let res = self.read_reg(rs1) << shamt;
                self.write_reg(rd, res);
            }
            Instruction::Slt { rd, rs1, rs2 } => {
                let val = if (self.read_reg(rs1) as i32) < (self.read_reg(rs2) as i32) {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Sltu { rd, rs1, rs2 } => {
                let val = if self.read_reg(rs1) < self.read_reg(rs2) {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Xor { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1) ^ self.read_reg(rs2);
                self.write_reg(rd, res);
            }
            Instruction::Srl { rd, rs1, rs2 } => {
                let shamt = self.read_reg(rs2) & 0x1F;
                let res = self.read_reg(rs1) >> shamt;
                self.write_reg(rd, res);
            }
            Instruction::Sra { rd, rs1, rs2 } => {
                let shamt = self.read_reg(rs2) & 0x1F;
                let res = (self.read_reg(rs1) as i32) >> shamt;
                self.write_reg(rd, res as u32);
            }
            Instruction::Or { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1) | self.read_reg(rs2);
                self.write_reg(rd, res);
            }
            Instruction::And { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1) & self.read_reg(rs2);
                self.write_reg(rd, res);
            }
            Instruction::Fence => {
                // No-op in single threaded core model
            }
            // RV32C Extension
            Instruction::CAddi { rd, imm } => {
                if rd != 0 {
                    let res = self.read_reg(rd).wrapping_add(imm as u32);
                    self.write_reg(rd, res);
                }
            }
            Instruction::CLi { rd, imm } => {
                if rd != 0 {
                    self.write_reg(rd, imm as u32);
                }
            }
            Instruction::CMv { rd, rs2 } => {
                if rd != 0 {
                    let val = self.read_reg(rs2);
                    self.write_reg(rd, val);
                }
            }
            Instruction::CAddi16sp { imm } => {
                let sp = self.read_reg(2);
                self.write_reg(2, sp.wrapping_add(imm as u32));
            }
            Instruction::CAddi4spn { rd, imm } => {
                let sp = self.read_reg(2);
                self.write_reg(rd, sp.wrapping_add(imm));
            }
            Instruction::CSli { rd, shamt } => {
                if rd != 0 {
                    let res = self.read_reg(rd).wrapping_shl(shamt as u32);
                    self.write_reg(rd, res);
                }
            }
            _ => unreachable!("exec_alu called with instruction from a different class"),
        }
        Ok(())
    }
}
