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
    // ---- RV32A: atomic memory operations (word) ----
    //
    // Single-hart semantics: aq/rl are ignored. LR.W records a
    // reservation on the effective address; SC.W succeeds iff the
    // current reservation matches its effective address. Any store
    // (including any AMO*) invalidates the reservation per §8.2.
    #[allow(clippy::too_many_lines)]
    pub(in crate::cpu::riscv) fn exec_atomic(
        &mut self,
        bus: &mut dyn Bus,
        instruction: Instruction,
    ) -> SimResult<()> {
        match instruction {
            Instruction::LrW { rd, rs1 } => {
                let addr = self.read_reg(rs1);
                let val = bus.read_u32(addr as u64)?;
                self.write_reg(rd, val);
                self.reservation = Some(addr);
            }
            Instruction::ScW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let store_ok = self.reservation == Some(addr);
                if store_ok {
                    bus.write_u32(addr as u64, self.read_reg(rs2))?;
                    self.invalidate_fetch_if_store_overlaps(addr, 4);
                    self.write_reg(rd, 0); // success
                } else {
                    self.write_reg(rd, 1); // failure
                }
                self.reservation = None;
            }
            Instruction::AmoSwapW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, self.read_reg(rs2))?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoAddW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old.wrapping_add(self.read_reg(rs2)))?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoXorW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old ^ self.read_reg(rs2))?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoOrW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old | self.read_reg(rs2))?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoAndW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                bus.write_u32(addr as u64, old & self.read_reg(rs2))?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMinW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let rhs = self.read_reg(rs2);
                let new = (old as i32).min(rhs as i32) as u32;
                bus.write_u32(addr as u64, new)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMaxW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let rhs = self.read_reg(rs2);
                let new = (old as i32).max(rhs as i32) as u32;
                bus.write_u32(addr as u64, new)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMinuW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let new = old.min(self.read_reg(rs2));
                bus.write_u32(addr as u64, new)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMaxuW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr as u64)?;
                let new = old.max(self.read_reg(rs2));
                bus.write_u32(addr as u64, new)?;
                self.invalidate_fetch_if_store_overlaps(addr, 4);
                self.write_reg(rd, old);
                self.reservation = None;
            }
            _ => unreachable!("exec_atomic called with instruction from a different class"),
        }
        Ok(())
    }
}
