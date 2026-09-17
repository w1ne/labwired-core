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
use crate::{Bus, SimResult};

impl RiscV {
    // Single call site in `step`. Left out of line, a per-class split costs a call and
    // a second variant match per instruction; the same split measured +13.7% Ir/step
    // on every Cortex-M board in the core-perf batch gate. Inlining keeps the codegen
    // of the pre-split function while the source stays split.
    #[inline(always)]
    pub(in crate::cpu::riscv) fn exec_load_store(
        &mut self,
        bus: &mut dyn Bus,
        instruction: Instruction,
    ) -> SimResult<()> {
        match instruction {
            Instruction::Lb { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u8(addr as u64)? as i8;
                self.write_reg(rd, val as i32 as u32);
            }
            Instruction::Lh { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u16(addr as u64)? as i16;
                self.write_reg(rd, val as i32 as u32);
            }
            Instruction::Lw { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u32(addr as u64)?;
                self.write_reg(rd, val);
            }
            Instruction::Lbu { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u8(addr as u64)?;
                self.write_reg(rd, val as u32);
            }
            Instruction::Lhu { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u16(addr as u64)?;
                self.write_reg(rd, val as u32);
            }
            Instruction::Sb { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2) as u8;
                bus.write_u8(addr as u64, val)?;
                self.invalidate_fetch_if_store_overlaps(addr, 1);
                self.reservation = None;
            }
            Instruction::Sh { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2) as u16;
                bus.write_u16(addr as u64, val)?;
                self.invalidate_fetch_if_store_overlaps(addr, 2);
                self.reservation = None;
            }
            Instruction::Sw { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2);
                bus.write_u32(addr as u64, val)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.reservation = None;
            }
            // RV32C Extension
            Instruction::CLw { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm);
                let val = bus.read_u32(addr as u64)?;
                self.write_reg(rd, val);
            }
            Instruction::CSw { rs2, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm);
                let val = self.read_reg(rs2);
                bus.write_u32(addr as u64, val)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.reservation = None;
            }
            Instruction::CLwsp { rd, imm } => {
                let sp = self.read_reg(2);
                let addr = sp.wrapping_add(imm);
                let val = bus.read_u32(addr as u64)?;
                self.write_reg(rd, val);
            }
            Instruction::CSwsp { rs2, imm } => {
                let sp = self.read_reg(2);
                let addr = sp.wrapping_add(imm);
                let val = self.read_reg(rs2);
                bus.write_u32(addr as u64, val)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.reservation = None;
            }
            _ => unreachable!("exec_load_store called with instruction from a different class"),
        }
        Ok(())
    }
}
