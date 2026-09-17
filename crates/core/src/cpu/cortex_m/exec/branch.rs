// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Split out of `step_execute` in `cpu/cortex_m.rs` (pure move, no behaviour
// change). See that file's `step_execute` for the shared exception/IT-state
// plumbing that wraps these per-class dispatches.

use super::super::CortexM;
use crate::decoder::arm::Instruction;
use crate::{Bus, SimResult};

impl CortexM {
    #[allow(clippy::too_many_lines)]
    pub(in crate::cpu::cortex_m) fn exec_branch<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        instruction: Instruction,
        pc_increment: &mut u32,
        it_block_instruction: &mut bool,
    ) -> SimResult<()> {
        match instruction {
            Instruction::Cbz { rn, imm } => {
                if self.read_reg(rn) == 0 {
                    self.pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
                    *pc_increment = 0;
                }
            }
            Instruction::Cbnz { rn, imm } => {
                if self.read_reg(rn) != 0 {
                    self.pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
                    *pc_increment = 0;
                }
            }
            Instruction::Branch { offset } => {
                let target = (self.pc as i32).wrapping_add(4).wrapping_add(offset) as u32;
                self.pc = target;
                *pc_increment = 0;
            }
            // Arithmetic
            Instruction::It { cond, mask } => {
                self.it_state = (cond << 4) | mask;
                *it_block_instruction = false; // The IT instruction itself doesn't count towards the block's instructions
            }
            Instruction::Bl { offset } => {
                // BL: Branch with Link.
                // LR = Next Instruction Address | 1 (Thumb bit)
                let _next_pc = self.pc.wrapping_add(4); // 32-bit instruction size for BL?
                                                        // Wait. BL is decoded as 32-bit.
                                                        // If we assume decode_thumb_16 handled a 32-bit stream, then PC increment should be adjusted?
                                                        // Or does `decode_thumb_16` return `BlPrefix` and then we handle it?
                                                        // The current `decoder` returns `Bl` with full offset if it sees the pair??
                                                        // NO. My decoder implementation for BL (in previous turn) was:
                                                        // `Instruction::Bl { offset: offset << 1 }`
                                                        // But `decode_thumb_16` ONLY sees 16 bits. It cannot see the second half!
                                                        // Real decoding of BL requires fetching 32 bits.

                // CRITICAL CORRECTION: `decode_thumb_16` is 16-bit.
                // BL is 32-bit (encoded as two 16-bit halves).
                // Fetch loop fetches 16 bits.
                // 1. Fetch High Half (0xF0xx). Returns BlPrefix?
                // 2. Fetch Low Half (0xF8xx). Combine?

                // My logic in decoder needs revisit. I put `Bl { offset }` thinking T1/T2 but BL is always 32-bit in Thumb-2.
                // T1 encoding of BL doesn't exist as single 16-bit.

                // For now, let's just implement the execution stub assuming the decoder *somehow* gave us the full BL.
                // But since the decoder only sees 16 bits, we need to handle the prefix state in the CPU loop!

                self.lr = self.pc.wrapping_add(4) | 1;
                let target = (self.pc as i32).wrapping_add(4).wrapping_add(offset) as u32;
                self.pc = target;
                *pc_increment = 0;
            }
            Instruction::BranchCond { cond, offset } => {
                if self.check_condition(cond) {
                    let target = (self.pc as i32).wrapping_add(4).wrapping_add(offset) as u32;
                    self.pc = target;
                    *pc_increment = 0;
                }
            }
            Instruction::Bx { rm } => {
                let target = self.read_reg(rm);
                self.branch_to(target, bus)?;
                *pc_increment = 0;
            }

            // BLX Rm (T1): branch-with-link to register address.
            // Sets LR = (PC_of_blx + 2) | 1 before branching.
            Instruction::BlxReg { rm } => {
                let target = self.read_reg(rm);
                self.lr = (self.pc.wrapping_add(2)) | 1;
                self.branch_to(target, bus)?;
                *pc_increment = 0;
            }

            // --- Thumb-2 ARMv7-M additions ---
            _ => unreachable!("exec_branch called with instruction from a different class"),
        }
        Ok(())
    }
}
