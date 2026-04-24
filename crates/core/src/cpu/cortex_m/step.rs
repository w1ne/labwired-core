// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Thumb / Thumb-2 dispatch and execute for the CortexM interpreter.
//!
//! Lives in its own submodule so that `cortex_m/mod.rs` stays focused on the
//! struct definition, trait impl, and debug-facing accessors. The giant
//! per-instruction match belongs here and nowhere else.

use super::helpers::{add_with_flags, sub_with_flags};
use super::CortexM;
use crate::decoder::arm::{decode_thumb_16, Instruction};
use crate::{Bus, SimResult, SimulationObserver};
use std::sync::atomic::Ordering;
use std::sync::Arc;

impl CortexM {
    pub(super) fn step_execute(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
    ) -> SimResult<()> {
        // Check for pending exceptions before executing instruction
        if self.pending_exceptions != 0 {
            // Find highest priority exception (Simplified: highest bit)
            let exception_num = 31 - self.pending_exceptions.leading_zeros();
            self.pending_exceptions &= !(1 << exception_num);

            // Perform Stacking (Simplified)
            let sp = self.regs[13];
            let frame_ptr = sp.wrapping_sub(32);

            // Stack: R0, R1, R2, R3, R12, LR, PC, xPSR
            let _ = bus.write_u32(frame_ptr, self.regs[0]);
            let _ = bus.write_u32(frame_ptr + 4, self.regs[1]);
            let _ = bus.write_u32(frame_ptr + 8, self.regs[2]);
            let _ = bus.write_u32(frame_ptr + 12, self.regs[3]);
            let _ = bus.write_u32(frame_ptr + 16, self.regs[12]);
            let _ = bus.write_u32(frame_ptr + 20, self.regs[14]);
            let _ = bus.write_u32(frame_ptr + 24, self.regs[15]);
            let _ = bus.write_u32(frame_ptr + 28, self.regs[16]);

            self.regs[13] = frame_ptr;

            // EXC_RETURN: Thread Mode, MSP
            self.regs[14] = 0xFFFF_FFF9;

            // Jump to ISR handler
            let vtor = self.vtor.load(Ordering::SeqCst);
            let vector_addr = vtor + (exception_num * 4);
            if let Ok(handler) = bus.read_u32(vector_addr) {
                self.regs[15] = handler & !1;
                tracing::info!(
                    "Exception {} trigger, jump to {:#x} (VTOR={:#x})",
                    exception_num,
                    self.regs[15],
                    vtor
                );
            }

            return Ok(());
        }

        // Fetch 16-bit thumb instruction; consult the decode cache first
        // so we can skip both the bus read and the decoder on a hit. The
        // cache is flushed on reset / apply_snapshot; see the note on
        // `CortexM::decode_cache` for the self-modifying-code caveat.
        let fetch_pc = self.regs[15] & !1;
        let (opcode, instruction) = if let Some(hit) = self.decode_lookup(fetch_pc) {
            hit
        } else {
            let opcode = bus.read_u16(fetch_pc)?;
            let instruction = decode_thumb_16(opcode);
            self.decode_store(fetch_pc, opcode, instruction);
            (opcode, instruction)
        };

        for observer in observers {
            observer.on_step_start(self.regs[15], opcode as u32);
        }

        // Execute
        let mut pc_increment = 2; // Default for 16-bit instruction
        let mut cycles = 1;

        match instruction {
            Instruction::Bfi { .. }
            | Instruction::Bfc { .. }
            | Instruction::Sbfx { .. }
            | Instruction::Ubfx { .. }
            | Instruction::Clz { .. }
            | Instruction::Rbit { .. }
            | Instruction::Rev { .. }
            | Instruction::Rev16 { .. }
            | Instruction::RevSh { .. }
            | Instruction::DataProc32 { .. }
            | Instruction::Movw { .. }
            | Instruction::Movt { .. }
            | Instruction::Barrier
            | Instruction::Mrs { .. }
            | Instruction::Msr { .. }
            | Instruction::Smull { .. }
            | Instruction::Umull { .. }
            | Instruction::Smlal { .. }
            | Instruction::Umlal { .. }
            | Instruction::Mla { .. }
            | Instruction::Mls { .. } => {
                unreachable!(
                    "32-bit instruction {:?} should be handled via Prefix32",
                    instruction
                );
            }

            Instruction::Nop => { /* Do nothing */ }
            Instruction::MovImm { rd, imm } => {
                self.write_reg(rd, imm as u32);
                self.update_nz(imm as u32);
            }
            // Control Flow
            Instruction::Cbz { rn, imm } => {
                if self.read_reg(rn) == 0 {
                    self.regs[15] = self.regs[15].wrapping_add(4).wrapping_add(imm as u32);
                    pc_increment = 0;
                }
            }
            Instruction::Cbnz { rn, imm } => {
                if self.read_reg(rn) != 0 {
                    self.regs[15] = self.regs[15].wrapping_add(4).wrapping_add(imm as u32);
                    pc_increment = 0;
                }
            }
            Instruction::Branch { offset } => {
                let target = (self.regs[15] as i32 + 4 + offset) as u32;
                self.regs[15] = target;
                pc_increment = 0;
            }
            // Arithmetic
            Instruction::AddReg { rd, rn, rm } => {
                let op1 = self.read_reg(rn);
                let op2 = self.read_reg(rm);
                let (res, c, v) = add_with_flags(op1, op2);
                self.write_reg(rd, res);
                self.update_nzcv(res, c, v);
            }
            Instruction::AddImm3 { rd, rn, imm } => {
                let op1 = self.read_reg(rn);
                let (res, c, v) = add_with_flags(op1, imm as u32);
                self.write_reg(rd, res);
                self.update_nzcv(res, c, v);
            }
            Instruction::AddImm8 { rd, imm } => {
                let op1 = self.read_reg(rd);
                let (res, c, v) = add_with_flags(op1, imm as u32);
                self.write_reg(rd, res);
                self.update_nzcv(res, c, v);
            }
            Instruction::SubReg { rd, rn, rm } => {
                let op1 = self.read_reg(rn);
                let op2 = self.read_reg(rm);
                let (res, c, v) = sub_with_flags(op1, op2);
                self.write_reg(rd, res);
                self.update_nzcv(res, c, v);
            }
            Instruction::SubImm3 { rd, rn, imm } => {
                let op1 = self.read_reg(rn);
                let (res, c, v) = sub_with_flags(op1, imm as u32);
                self.write_reg(rd, res);
                self.update_nzcv(res, c, v);
            }
            Instruction::SubImm8 { rd, imm } => {
                let op1 = self.read_reg(rd);
                let (res, c, v) = sub_with_flags(op1, imm as u32);
                self.write_reg(rd, res);
                self.update_nzcv(res, c, v);
            }
            Instruction::AddSp { imm } => {
                let sp = self.read_reg(13).wrapping_add(imm as u32);
                self.write_reg(13, sp);
            }
            Instruction::SubSp { imm } => {
                let sp = self.read_reg(13).wrapping_sub(imm as u32);
                self.write_reg(13, sp);
            }
            Instruction::AddRegHigh { rd, rm } => {
                let val1 = self.read_reg(rd);
                let val2 = self.read_reg(rm);
                self.write_reg(rd, val1.wrapping_add(val2));
            }
            Instruction::CmpImm { rn, imm } => {
                let op1 = self.read_reg(rn);
                let (res, c, v) = sub_with_flags(op1, imm as u32);
                self.update_nzcv(res, c, v);
            }
            Instruction::CmpReg { rn, rm } => {
                let op1 = self.read_reg(rn);
                let op2 = self.read_reg(rm);
                let (res, c, v) = sub_with_flags(op1, op2);
                self.update_nzcv(res, c, v);
            }
            Instruction::MovReg { rd, rm } => {
                let val = self.read_reg(rm);
                self.write_reg(rd, val);
            }
            // Logic
            Instruction::And { rd, rm } => {
                let res = self.read_reg(rd) & self.read_reg(rm);
                self.write_reg(rd, res);
                self.update_nz(res);
            }
            Instruction::Orr { rd, rm } => {
                let res = self.read_reg(rd) | self.read_reg(rm);
                self.write_reg(rd, res);
                self.update_nz(res);
            }
            Instruction::Eor { rd, rm } => {
                let res = self.read_reg(rd) ^ self.read_reg(rm);
                self.write_reg(rd, res);
                self.update_nz(res);
            }
            Instruction::Mvn { rd, rm } => {
                let res = !self.read_reg(rm);
                self.write_reg(rd, res);
                self.update_nz(res);
            }
            Instruction::Mul { rd, rn } => {
                let op1 = self.read_reg(rd);
                let op2 = self.read_reg(rn);
                let res = op1.wrapping_mul(op2);
                self.write_reg(rd, res);
                self.update_nz(res);
            }

            Instruction::Cpsie => {
                self.primask = false;
            }
            Instruction::Cpsid => {
                self.primask = true;
            }

            // Shifts
            Instruction::Lsl { rd, rm, imm } => {
                let val = self.read_reg(rm);
                let res = val.wrapping_shl(imm as u32);
                self.write_reg(rd, res);
                self.update_nz(res);
                // Note: Carry out not fully implemented for shifts yet
            }
            Instruction::Lsr { rd, rm, imm } => {
                let val = self.read_reg(rm);
                let res = if imm == 0 {
                    0
                } else {
                    val.wrapping_shr(imm as u32)
                };
                // Actually LSR imm=0 is 32 in some contexts, but Thumb T1 usually:
                // imm5=0 for LSL is imm=0. imm5=0 for LSR is imm=32.
                // For MVP, letting wrapping_shr handle basics.
                self.write_reg(rd, res);
                self.update_nz(res);
            }
            Instruction::Asr { rd, rm, imm } => {
                let val = self.read_reg(rm) as i32;
                let res = (if imm == 0 {
                    val >> 31
                } else {
                    val >> (imm as u32)
                }) as u32;
                self.write_reg(rd, res);
                self.update_nz(res);
                self.write_reg(rd, res);
                self.update_nz(res);
            }
            Instruction::AsrReg { rd, rm } => {
                let val = self.read_reg(rd) as i32;
                let shift = self.read_reg(rm) & 0xFF;
                let res = if shift == 0 {
                    val as u32
                } else if shift >= 32 {
                    (val >> 31) as u32
                } else {
                    (val >> shift) as u32
                };
                self.write_reg(rd, res);
                self.update_nz(res);
            }
            Instruction::Rsbs { rd, rn } => {
                let op1 = self.read_reg(rn);
                let (res, c, v) = sub_with_flags(0, op1);
                self.write_reg(rd, res);
                self.update_nzcv(res, c, v);
            }

            // Memory Operations (Word)
            Instruction::LdrImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                if let Ok(val) = bus.read_u32(addr) {
                    self.write_reg(rt, val);
                } else {
                    tracing::error!("Bus Read Fault at {:#x}", addr);
                }
            }
            Instruction::StrImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                let val = self.read_reg(rt);
                if bus.write_u32(addr, val).is_err() {
                    tracing::error!("Bus Write Fault at {:#x}", addr);
                }
            }
            Instruction::LdrReg { rt, rn, rm } => {
                let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                if let Ok(val) = bus.read_u32(addr) {
                    self.write_reg(rt, val);
                } else {
                    tracing::error!("Bus Read Fault (LDR reg) at {:#x}", addr);
                }
            }

            Instruction::LdrLit { rt, imm } => {
                // ... (existing)
                let pc_val = (self.regs[15] & !3) + 4;
                let addr = pc_val.wrapping_add(imm as u32);
                if let Ok(val) = bus.read_u32(addr) {
                    self.write_reg(rt, val);
                } else {
                    tracing::error!("Bus Read Fault (LdrLit) at {:#x}", addr);
                }
            }

            Instruction::LdrSp { rt, imm } => {
                let addr = self.regs[13].wrapping_add(imm as u32);
                if let Ok(val) = bus.read_u32(addr) {
                    self.write_reg(rt, val);
                } else {
                    tracing::error!("Bus Read Fault (LdrSp) at {:#x}", addr);
                }
            }
            Instruction::StrSp { rt, imm } => {
                let addr = self.regs[13].wrapping_add(imm as u32);
                let val = self.read_reg(rt);
                if bus.write_u32(addr, val).is_err() {
                    tracing::error!("Bus Write Fault (StrSp) at {:#x}", addr);
                }
            }
            Instruction::AddSpReg { rd, imm } => {
                let res = self.regs[13].wrapping_add(imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Adr { rd, imm } => {
                let pc_val = (self.regs[15] & !3) + 4;
                let res = pc_val.wrapping_add(imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Uxtb { rd, rm } => {
                let val = self.read_reg(rm) & 0xFF;
                self.write_reg(rd, val);
            }

            // Memory Operations (Byte)
            Instruction::LdrbImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                if let Ok(val) = bus.read_u8(addr) {
                    self.write_reg(rt, val as u32);
                } else {
                    tracing::error!("Bus Read Fault (LDRB) at {:#x}", addr);
                }
            }
            Instruction::StrbImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                let val = (self.read_reg(rt) & 0xFF) as u8;
                if bus.write_u8(addr, val).is_err() {
                    tracing::error!("Bus Write Fault (STRB) at {:#x}", addr);
                }
            }
            Instruction::LdrhImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                if let Ok(val) = bus.read_u16(addr) {
                    self.write_reg(rt, val as u32);
                } else {
                    tracing::error!("Bus Read Fault (LDRH) at {:#x}", addr);
                }
            }
            Instruction::StrhImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                let val = (self.read_reg(rt) & 0xFFFF) as u16;
                if bus.write_u16(addr, val).is_err() {
                    tracing::error!("Bus Write Fault (STRH) at {:#x}", addr);
                }
            }

            // Stack Operations
            Instruction::Push { registers, m } => {
                let mut sp = self.read_reg(13);
                // Cycle through R14(LR), R7..R0 high to low

                // If M (LR) is set, push LR first (highest address)
                if m {
                    sp = sp.wrapping_sub(4);
                    let val = self.read_reg(14);
                    if bus.write_u32(sp, val).is_err() {
                        tracing::error!("Stack Overflow (PUSH LR)");
                    }
                }

                // Registers R7 down to R0
                for i in (0..=7).rev() {
                    if (registers & (1 << i)) != 0 {
                        sp = sp.wrapping_sub(4);
                        let val = self.read_reg(i);
                        if bus.write_u32(sp, val).is_err() {
                            tracing::error!("Stack Overflow (PUSH R{})", i);
                        }
                    }
                }

                self.write_reg(13, sp);
            }
            Instruction::Pop { registers, p } => {
                let mut sp = self.read_reg(13);

                // Registers R0 up to R7
                for i in 0..=7 {
                    if (registers & (1 << i)) != 0 {
                        if let Ok(val) = bus.read_u32(sp) {
                            self.write_reg(i, val);
                        }
                        sp = sp.wrapping_add(4);
                    }
                }

                // If P (PC) is set, pop PC (lowest address?? No, highest)
                // POP is inverse of PUSH. PUSH pushed LR last (lowest addr) ??
                // Wait. PUSH stores STMDB (Decrement Before). Highest reg = Highest address.
                // R0 is lowest register. LR is highest.
                // PUSH order: LR, R7, ... R0.
                // Stack grows down.
                // Low Addr [ R0 | R1 | ... | LR ] High Addr.
                // So POP (LDMIA) should read: R0, ... R7, PC.
                // My PUSH loop:
                // 1. If LR, sub 4, write LR. (Top of stack, highest addr - 4)
                // 2. Loop 7 down to 0: sub 4, write Rx.
                // Result: R0 is at current SP. LR is at SP + n*4.

                // My POP loop:
                // 1. Loop 0 to 7: read, add 4. (Read R0, R1...)
                // 2. If PC, read, add 4.

                if p {
                    if let Ok(val) = bus.read_u32(sp) {
                        self.branch_to(val, bus)?;
                        pc_increment = 0; // Branch taken
                    }
                    sp = sp.wrapping_add(4);
                }

                self.write_reg(13, sp);
            }
            Instruction::Ldm { rn, registers } => {
                let mut base = self.read_reg(rn);
                for i in 0..=7 {
                    if (registers & (1 << i)) != 0 {
                        if let Ok(val) = bus.read_u32(base) {
                            self.write_reg(i, val);
                        }
                        base = base.wrapping_add(4);
                    }
                }
                self.write_reg(rn, base);
            }
            Instruction::Stm { rn, registers } => {
                let mut base = self.read_reg(rn);
                for i in 0..=7 {
                    if (registers & (1 << i)) != 0 {
                        let val = self.read_reg(i);
                        if bus.write_u32(base, val).is_err() {
                            tracing::error!("Bus Write Fault (STM) at {:#x}", base);
                        }
                        base = base.wrapping_add(4);
                    }
                }
                self.write_reg(rn, base);
            }

            // Control Flow
            Instruction::Bl { offset } => {
                // BL: Branch with Link.
                // LR = Next Instruction Address | 1 (Thumb bit)
                let _next_pc = self.regs[15] + 4; // 32-bit instruction size for BL?
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

                self.regs[14] = (self.regs[15] + 4) | 1;
                let target = (self.regs[15] as i32 + 4 + offset) as u32;
                self.regs[15] = target;
                pc_increment = 0;
            }
            Instruction::BranchCond { cond, offset } => {
                if self.check_condition(cond) {
                    let target = (self.regs[15] as i32 + 4 + offset) as u32;
                    self.regs[15] = target;
                    pc_increment = 0;
                }
            }
            Instruction::Bx { rm } => {
                let target = self.read_reg(rm);
                self.branch_to(target, bus)?;
                pc_increment = 0;
            }

            Instruction::Prefix32(h1) => {
                let (inc, c) = self.dispatch_thumb2(bus, h1)?;
                pc_increment = inc;
                cycles = c;
            }

            Instruction::Unknown(op) => {
                tracing::warn!("Unknown instruction at {:#x}: Opcode {:#06x}", self.regs[15], op);
                pc_increment = 2; // Skip 16-bit
            }
        }

        self.regs[15] = self.regs[15].wrapping_add(pc_increment);

        for observer in observers {
            observer.on_step_end(cycles);
        }

        Ok(())
    }
}
