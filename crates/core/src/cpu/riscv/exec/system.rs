// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Split out of `step` in `cpu/riscv.rs` (pure move, no behaviour change).
// See that file's `step` for the shared fetch/decode/interrupt/pc-commit
// plumbing that wraps this per-class dispatch.
//
// Several of these arms originally did `return Ok(());` directly out of
// `step`, short-circuiting the timer update / interrupt check / pc-commit
// tail that follows the big match. A function boundary can't reach through
// to the caller's `return`, so this returns `Ok(true)` in exactly those
// cases instead; `step` propagates that into its own early `return Ok(())`.
// This is the one place the move needed more than a mechanical deref — every
// arm body is untouched, only the "already fully handled" signal changed
// shape.
use super::super::RiscV;
use crate::decoder::riscv::Instruction;
use crate::SimResult;

impl RiscV {
    /// Returns `Ok(true)` when the instruction is already fully handled and
    /// `step` must return immediately, matching the original inline
    /// `return Ok(());` arms.
    // Single call site in `step`. Left out of line, a per-class split costs a call and
    // a second variant match per instruction; the same split measured +13.7% Ir/step
    // on every Cortex-M board in the core-perf batch gate. Inlining keeps the codegen
    // of the pre-split function while the source stays split.
    #[inline(always)]
    pub(in crate::cpu::riscv) fn exec_system(
        &mut self,
        instruction: Instruction,
        opcode: u32,
    ) -> SimResult<bool> {
        match instruction {
            Instruction::Wfi => {
                // Wait-for-interrupt: implemented as a no-op busy-wait. The step
                // loop already polls pending interrupts every instruction, so
                // the idle task's WFI spin wakes as soon as a line asserts.
                self.waiting_for_interrupt = true;
            }
            Instruction::Ecall | Instruction::Ebreak => {
                // Should trap. For now, we can just log or halt.
                tracing::warn!("ECALL/EBREAK encountered at {:#x}", self.pc);
                self.handle_trap(
                    if instruction == Instruction::Ecall {
                        11
                    } else {
                        3
                    },
                    self.pc,
                );
                return Ok(true);
            }
            Instruction::Mret => {
                // Return from trap. Per the privileged spec:
                //   MIE <- MPIE, MPIE <- 1 (privilege <- MPP, but we stay M-mode).
                self.pc = self.mepc;
                let mpie = (self.mstatus >> 7) & 1;
                self.mstatus &= !(1 << 3); // clear MIE
                self.mstatus |= mpie << 3; // MIE <- MPIE
                self.mstatus |= 1 << 7; // MPIE <- 1
                return Ok(true);
            }
            Instruction::Csrrw { rd, rs1, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(true);
                };
                let val = self.read_reg(rs1);
                if !self.csr_write_or_trap(csr, val, opcode) {
                    return Ok(true);
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrs { rd, rs1, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(true);
                };
                if rs1 != 0 {
                    let val = self.read_reg(rs1);
                    if !self.csr_write_or_trap(csr, old | val, opcode) {
                        return Ok(true);
                    }
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrc { rd, rs1, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(true);
                };
                if rs1 != 0 {
                    let val = self.read_reg(rs1);
                    if !self.csr_write_or_trap(csr, old & !val, opcode) {
                        return Ok(true);
                    }
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrwi { rd, imm, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(true);
                };
                if !self.csr_write_or_trap(csr, imm as u32, opcode) {
                    return Ok(true);
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrsi { rd, imm, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(true);
                };
                if imm != 0 && !self.csr_write_or_trap(csr, old | (imm as u32), opcode) {
                    return Ok(true);
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrci { rd, imm, csr } => {
                let Some(old) = self.csr_read_or_trap(csr, opcode) else {
                    return Ok(true);
                };
                if imm != 0 && !self.csr_write_or_trap(csr, old & !(imm as u32), opcode) {
                    return Ok(true);
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            _ => unreachable!("exec_system called with instruction from a different class"),
        }
        Ok(false)
    }
}
