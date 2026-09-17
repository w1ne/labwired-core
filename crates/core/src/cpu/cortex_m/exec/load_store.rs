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
use crate::cpu::cortex_m::AccessWidth;
use crate::decoder::arm::Instruction;
use crate::{Bus, SimResult};

impl CortexM {
    #[allow(clippy::too_many_lines)]
    pub(in crate::cpu::cortex_m) fn exec_load_store<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        instruction: Instruction,
        pc_increment: &mut u32,
        _it_block_instruction: &mut bool,
    ) -> SimResult<()> {
        match instruction {
            Instruction::LdrImm32 { rt, rn, imm12 } => {
                // When rn==PC, ARM spec requires Align(PC+4, 4) as base (literal load)
                let base = if rn == 15 {
                    (self.pc.wrapping_add(4)) & !3
                } else {
                    self.read_reg(rn)
                };
                let addr = base.wrapping_add(imm12 as u32);
                let val = self.load(bus, addr, AccessWidth::Word)?;
                if rt == 15 {
                    // LDR PC, [...] is an interworking branch — must go through branch_to
                    self.branch_to(val, bus)?;
                    *pc_increment = 0;
                } else {
                    self.write_reg(rt, val);
                }
                // pc_increment stays at 4 (set by decode) unless we took a branch above
            }
            Instruction::StrImm32 { rt, rn, imm12 } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm12 as u32);
                let val = self.read_reg(rt);
                self.store(bus, addr, AccessWidth::Word, val)?;
                *pc_increment = 4;
            }
            Instruction::LdrImm32Idx {
                rt,
                rn,
                imm8,
                pre_index,
                add,
                writeback,
            } => {
                // LDR T4 indexed. Offset address = base ± imm8. The access
                // uses the offset address when pre_index, else the base
                // (post-index). Writeback stores the offset address in Rn.
                let base = self.read_reg(rn);
                let offset = imm8 as u32;
                let offset_addr = if add {
                    base.wrapping_add(offset)
                } else {
                    base.wrapping_sub(offset)
                };
                let access_addr = if pre_index { offset_addr } else { base };
                let val = self.load(bus, access_addr, AccessWidth::Word)?;
                // Commit writeback before branching so a load-to-PC
                // (function return) leaves Rn=SP correct.
                if writeback {
                    self.write_reg(rn, offset_addr);
                }
                if rt == 15 {
                    // LDR PC, [...] — interworking branch (function return).
                    self.branch_to(val, bus)?;
                    *pc_increment = 0;
                } else {
                    self.write_reg(rt, val);
                    *pc_increment = 4;
                }
            }
            Instruction::StrImm32Idx {
                rt,
                rn,
                imm8,
                pre_index,
                add,
                writeback,
            } => {
                let base = self.read_reg(rn);
                let offset = imm8 as u32;
                let offset_addr = if add {
                    base.wrapping_add(offset)
                } else {
                    base.wrapping_sub(offset)
                };
                let access_addr = if pre_index { offset_addr } else { base };
                let val = self.read_reg(rt);
                self.store(bus, access_addr, AccessWidth::Word, val)?;
                if writeback {
                    self.write_reg(rn, offset_addr);
                }
                *pc_increment = 4;
            }
            Instruction::Ldrd {
                rt,
                rt2,
                rn,
                imm8,
                add_imm,
                index,
                writeback,
            } => {
                // ARMv8-M LDRD (immediate): offset_addr = Rn ± imm32;
                // access_addr = index ? offset_addr : Rn; if writeback,
                // Rn = offset_addr.
                let base = self.read_reg(rn);
                let offset_addr = if add_imm {
                    base.wrapping_add(imm8 << 2)
                } else {
                    base.wrapping_sub(imm8 << 2)
                };
                let addr = if index { offset_addr } else { base };
                let v1 = self.load(bus, addr, AccessWidth::Word)?;
                self.write_reg(rt, v1);
                let v2 = self.load(bus, addr.wrapping_add(4), AccessWidth::Word)?;
                self.write_reg(rt2, v2);
                if writeback {
                    self.write_reg(rn, offset_addr);
                }
                *pc_increment = 4;
            }
            Instruction::Strd {
                rt,
                rt2,
                rn,
                imm8,
                add_imm,
                index,
                writeback,
            } => {
                let base = self.read_reg(rn);
                let offset_addr = if add_imm {
                    base.wrapping_add(imm8 << 2)
                } else {
                    base.wrapping_sub(imm8 << 2)
                };
                let addr = if index { offset_addr } else { base };
                let v1 = self.read_reg(rt);
                let v2 = self.read_reg(rt2);
                self.store(bus, addr, AccessWidth::Word, v1)?;
                self.store(bus, addr.wrapping_add(4), AccessWidth::Word, v2)?;
                if writeback {
                    self.write_reg(rn, offset_addr);
                }
                *pc_increment = 4;
            }
            Instruction::Tbb { rn, rm } => {
                let mut base = self.read_reg(rn);
                if rn == 15 {
                    // ARMv7-M: the table base is the in-execution PC
                    // (insn address + 4) — NOT word-aligned. The old
                    // Align(PC,4) read the table 2 bytes early whenever
                    // the TBB sat at a 2-mod-4 address; ST's
                    // HAL_DMA_RegisterCallback dispatched every
                    // callback ID into the same slot because of it.
                    base = self.pc.wrapping_add(4);
                }
                let index = self.read_reg(rm);
                let addr = base.wrapping_add(index);
                let byte = self.load(bus, addr, AccessWidth::Byte)?;
                let offset = byte << 1;
                self.pc = self.pc.wrapping_add(4).wrapping_add(offset);
                *pc_increment = 0;
            }
            Instruction::Tbh { rn, rm } => {
                let mut base = self.read_reg(rn);
                if rn == 15 {
                    // Same unaligned PC+4 base rule as TBB above.
                    base = self.pc.wrapping_add(4);
                }
                let index = self.read_reg(rm);
                let addr = base.wrapping_add(index << 1);
                let halfword = self.load(bus, addr, AccessWidth::Half)?;
                let offset = halfword << 1;
                self.pc = self.pc.wrapping_add(4).wrapping_add(offset);
                *pc_increment = 0;
            }
            Instruction::LdrImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                let val = self.load(bus, addr, AccessWidth::Word)?;
                self.write_reg(rt, val);
                if val == 0x021d0000 {
                    tracing::info!(
                        "LDR Literal/Imm SUSPICIOUS: R{} loaded with {:#x} from {:#x} (PC={:#x})",
                        rt,
                        val,
                        addr,
                        self.pc
                    );
                }
            }
            Instruction::StrImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                let val = self.read_reg(rt);
                self.store(bus, addr, AccessWidth::Word, val)?;
            }
            Instruction::LdrReg { rt, rn, rm } => {
                let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                let val = self.load(bus, addr, AccessWidth::Word)?;
                self.write_reg(rt, val);
            }
            Instruction::StrReg { rt, rn, rm } => {
                let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                let val = self.read_reg(rt);
                self.store(bus, addr, AccessWidth::Word, val)?;
            }

            Instruction::LdrLit { rt, imm } => {
                let pc_val = (self.pc & !3).wrapping_add(4);
                let addr = pc_val.wrapping_add(imm as u32);
                let val = self.load(bus, addr, AccessWidth::Word)?;
                self.write_reg(rt, val);
            }

            Instruction::LdrSp { rt, imm } => {
                let addr = self.sp.wrapping_add(imm as u32);
                let val = self.load(bus, addr, AccessWidth::Word)?;
                self.write_reg(rt, val);
            }
            Instruction::StrSp { rt, imm } => {
                let addr = self.sp.wrapping_add(imm as u32);
                let val = self.read_reg(rt);
                self.store(bus, addr, AccessWidth::Word, val)?;
            }
            Instruction::LdrbImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                let val = self.load(bus, addr, AccessWidth::Byte)?;
                self.write_reg(rt, val);
            }
            Instruction::LdrbReg { rt, rn, rm } => {
                let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                let val = self.load(bus, addr, AccessWidth::Byte)?;
                self.write_reg(rt, val);
            }
            Instruction::StrbReg { rt, rn, rm } => {
                let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                let val = self.read_reg(rt) & 0xFF;
                self.store(bus, addr, AccessWidth::Byte, val)?;
            }
            Instruction::LdrsbReg { rt, rn, rm } => {
                let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                let val = self.load(bus, addr, AccessWidth::Byte)?;
                let res = (val as u8 as i8) as i32 as u32;
                self.write_reg(rt, res);
            }
            Instruction::LdrhReg { rt, rn, rm } => {
                let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                let val = self.load(bus, addr, AccessWidth::Half)?;
                self.write_reg(rt, val);
            }
            Instruction::StrhReg { rt, rn, rm } => {
                let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                let val = self.read_reg(rt) & 0xFFFF;
                self.store(bus, addr, AccessWidth::Half, val)?;
            }
            Instruction::LdrshReg { rt, rn, rm } => {
                let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                let val = self.load(bus, addr, AccessWidth::Half)?;
                let res = (val as u16 as i16) as i32 as u32;
                self.write_reg(rt, res);
            }
            Instruction::StrbImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                let val = self.read_reg(rt) & 0xFF;
                self.store(bus, addr, AccessWidth::Byte, val)?;
            }
            Instruction::LdrhImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                let val = self.load(bus, addr, AccessWidth::Half)?;
                self.write_reg(rt, val);
            }
            Instruction::StrhImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                let val = self.read_reg(rt) & 0xFFFF;
                self.store(bus, addr, AccessWidth::Half, val)?;
            }
            Instruction::Push { registers, m } => {
                let mut sp = self.read_reg(13);
                // Cycle through R14(LR), R7..R0 high to low

                // If M (LR) is set, push LR first (highest address)
                if m {
                    sp = sp.wrapping_sub(4);
                    let val = self.read_reg(14);
                    self.store(bus, sp, AccessWidth::Word, val)?;
                }

                // Registers R7 down to R0
                for i in (0..=7).rev() {
                    if (registers & (1 << i)) != 0 {
                        sp = sp.wrapping_sub(4);
                        let val = self.read_reg(i);
                        self.store(bus, sp, AccessWidth::Word, val)?;
                    }
                }

                self.write_reg(13, sp);
            }
            Instruction::Pop { registers, p } => {
                let mut sp = self.read_reg(13);

                // Registers R0 up to R7
                for i in 0..=7 {
                    if (registers & (1 << i)) != 0 {
                        let val = self.load(bus, sp, AccessWidth::Word)?;
                        self.write_reg(i, val);
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
                    let val = self.load(bus, sp, AccessWidth::Word)?;
                    // Commit SP before branching so EXC_RETURN unstacking reads the
                    // hardware exception frame, not this function's software save area.
                    sp = sp.wrapping_add(4);
                    self.write_reg(13, sp);
                    self.branch_to(val, bus)?;
                    *pc_increment = 0; // Branch taken
                } else {
                    self.write_reg(13, sp);
                }
            }
            Instruction::Ldm { rn, registers } => {
                let mut base = self.read_reg(rn);
                for i in 0..=7 {
                    if (registers & (1 << i)) != 0 {
                        let val = self.load(bus, base, AccessWidth::Word)?;
                        self.write_reg(i, val);
                        base = base.wrapping_add(4);
                    }
                }
                // LDM (T1) writeback: the base register is written back with
                // the incremented address ONLY when it is NOT in the register
                // list. When the base IS in the list (the assembler emits no
                // `!`, e.g. `ldmia r2, {r0,r1,r2}`), the ARMv6-M architecture
                // specifies the loaded value wins and no writeback occurs.
                // Writing back unconditionally clobbered the just-loaded value
                // — which corrupted the compiler's struct-copy / stacked-arg
                // reload idiom and silently dropped a loaded argument.
                if (registers & (1 << rn)) == 0 {
                    self.write_reg(rn, base);
                }
            }
            Instruction::Stm { rn, registers } => {
                let mut base = self.read_reg(rn);
                for i in 0..=7 {
                    if (registers & (1 << i)) != 0 {
                        let val = self.read_reg(i);
                        self.store(bus, base, AccessWidth::Word, val)?;
                        base = base.wrapping_add(4);
                    }
                }
                self.write_reg(rn, base);
            }
            // STMDB Rn(!), {reg_list} — 32-bit store multiple, decrement before.
            // Lowest-numbered register stored at lowest address.
            Instruction::StmdbW {
                rn,
                reg_list,
                writeback,
            } => {
                let count = reg_list.count_ones();
                let mut addr = self.read_reg(rn).wrapping_sub(count * 4);
                let start = addr;
                for i in 0u8..=15 {
                    if (reg_list & (1 << i)) != 0 {
                        let val = self.read_reg(i);
                        self.store(bus, addr, AccessWidth::Word, val)?;
                        addr = addr.wrapping_add(4);
                    }
                }
                if writeback {
                    self.write_reg(rn, start);
                }
                *pc_increment = 4;
            }
            // STMIA.W Rn(!), {reg_list} — 32-bit store multiple, increment
            // after. Lowest-numbered register stored at the base address.
            Instruction::StmiaW {
                rn,
                reg_list,
                writeback,
            } => {
                let mut addr = self.read_reg(rn);
                for i in 0u8..=14 {
                    if (reg_list & (1 << i)) != 0 {
                        let val = self.read_reg(i);
                        self.store(bus, addr, AccessWidth::Word, val)?;
                        addr = addr.wrapping_add(4);
                    }
                }
                if writeback {
                    self.write_reg(rn, addr);
                }
                *pc_increment = 4;
            }
            // LDMDB.W Rn(!), {reg_list} — 32-bit load multiple, decrement
            // before. Lowest-numbered register loaded from the lowest address.
            Instruction::LdmdbW {
                rn,
                reg_list,
                writeback,
            } => {
                let count = reg_list.count_ones();
                let start = self.read_reg(rn).wrapping_sub(count * 4);
                let mut addr = start;
                for i in 0u8..=14 {
                    if (reg_list & (1 << i)) != 0 {
                        let val = self.load(bus, addr, AccessWidth::Word)?;
                        self.write_reg(i, val);
                        addr = addr.wrapping_add(4);
                    }
                }
                if writeback {
                    self.write_reg(rn, start);
                }
                if (reg_list & (1 << 15)) != 0 {
                    let pc_val = self.load(bus, addr, AccessWidth::Word)?;
                    self.branch_to(pc_val, bus)?;
                    *pc_increment = 0;
                } else {
                    *pc_increment = 4;
                }
            }
            // LDMIA.W Rn(!), {reg_list} — 32-bit load multiple, increment after.
            // Lowest-numbered register loaded from lowest address.
            Instruction::LdmiaW {
                rn,
                reg_list,
                writeback,
            } => {
                let mut addr = self.read_reg(rn);
                // Load R0-R14 (skip PC; handle separately to commit SP first)
                for i in 0u8..=14 {
                    if (reg_list & (1 << i)) != 0 {
                        let val = self.load(bus, addr, AccessWidth::Word)?;
                        self.write_reg(i, val);
                        addr = addr.wrapping_add(4);
                    }
                }
                // Handle PC (bit 15) — commit writeback before branching
                if (reg_list & (1 << 15)) != 0 {
                    let pc_val = self.load(bus, addr, AccessWidth::Word)?;
                    addr = addr.wrapping_add(4);
                    if writeback {
                        self.write_reg(rn, addr);
                    }
                    self.branch_to(pc_val, bus)?;
                    *pc_increment = 0;
                } else {
                    if writeback {
                        self.write_reg(rn, addr);
                    }
                    *pc_increment = 4;
                }
            }

            // Control Flow
            _ => unreachable!("exec_load_store called with instruction from a different class"),
        }
        Ok(())
    }
}
