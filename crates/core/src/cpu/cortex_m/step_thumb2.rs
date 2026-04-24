// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Dispatch + execute for Thumb-2 32-bit instructions. The Prefix32 arm of
//! the main step() match fell here so step.rs could keep its focus on
//! Thumb-16 dispatch. Returns `(pc_increment, cycles)` on success; a bus
//! fault fetching the second halfword falls back to pc_increment = 2
//! (same as a 16-bit instruction) to match the historical behavior.

use super::helpers::{thumb_expand_imm, PSR_C};
use super::CortexM;
use crate::decoder::arm::Instruction;
use crate::{Bus, SimResult};

impl CortexM {
    pub(super) fn dispatch_thumb2(
        &mut self,
        bus: &mut dyn Bus,
        h1: u16,
    ) -> SimResult<(u32, u32)> {
        let mut pc_increment: u32 = 2;
        let cycles: u32 = 2;
        let next_pc = (self.regs[15] & !1) + 2;
    if let Ok(h2) = bus.read_u16(next_pc as u64) {
        // Use the new modular decoder
        let instruction32 = crate::decoder::arm::decode_thumb_32(h1, h2);

        tracing::debug!(" decoded 32-bit: {:?}", instruction32);

        match instruction32 {
            Instruction::Bfi { rd, rn, lsb, width } => {
                let src = self.read_reg(rn);
                let dst = self.read_reg(rd);
                let mask = if width == 32 {
                    !0
                } else {
                    ((1u32.wrapping_shl(width as u32)).wrapping_sub(1))
                        .wrapping_shl(lsb as u32)
                };
                let result = (dst & !mask) | ((src.wrapping_shl(lsb as u32)) & mask);
                self.write_reg(rd, result);
                pc_increment = 4;
            }
            Instruction::Bfc { rd, lsb, width } => {
                let dst = self.read_reg(rd);
                let mask = if width == 32 {
                    !0
                } else {
                    ((1u32.wrapping_shl(width as u32)).wrapping_sub(1))
                        .wrapping_shl(lsb as u32)
                };
                let result = dst & !mask;
                self.write_reg(rd, result);
                pc_increment = 4;
            }
            Instruction::Sbfx { rd, rn, lsb, width } => {
                let src = self.read_reg(rn);
                let width_mask = if width == 32 {
                    !0
                } else {
                    (1u32.wrapping_shl(width as u32)).wrapping_sub(1)
                };
                let val = (src.wrapping_shr(lsb as u32)) & width_mask;

                let result = if width == 32 {
                    val
                } else {
                    let shift = 32 - width;
                    ((val.wrapping_shl(shift as u32)) as i32).wrapping_shr(shift as u32)
                        as u32
                };

                self.write_reg(rd, result);
                pc_increment = 4;
            }
            Instruction::Ubfx { rd, rn, lsb, width } => {
                let src = self.read_reg(rn);
                let width_mask = if width == 32 {
                    !0
                } else {
                    (1u32.wrapping_shl(width as u32)).wrapping_sub(1)
                };
                let result = (src.wrapping_shr(lsb as u32)) & width_mask;
                self.write_reg(rd, result);
                pc_increment = 4;
            }
            Instruction::Clz { rd, rm } => {
                let val = self.read_reg(rm);
                let result = val.leading_zeros();
                self.write_reg(rd, result);
                pc_increment = 4;
            }
            Instruction::Rbit { rd, rm } => {
                let val = self.read_reg(rm);
                let result = val.reverse_bits();
                self.write_reg(rd, result);
                pc_increment = 4;
            }
            Instruction::Rev { rd, rm } => {
                let val = self.read_reg(rm);
                let result = val.swap_bytes();
                self.write_reg(rd, result);
                pc_increment = 4;
            }
            Instruction::Rev16 { rd, rm } => {
                let val = self.read_reg(rm);
                let low = ((val & 0xFF) << 8) | ((val >> 8) & 0xFF);
                let high = ((val & 0x00FF0000) << 8) | ((val & 0xFF000000) >> 8);
                self.write_reg(rd, high | low);
                pc_increment = 4;
            }
            Instruction::RevSh { rd, rm } => {
                let val = self.read_reg(rm);
                // REVSH: Reverse byte order in lower halfword, sign extend
                let low = ((val & 0xFF) << 8) | ((val >> 8) & 0xFF);
                let result = (low as i16) as u32; // Sign extend
                self.write_reg(rd, result);
                pc_increment = 4;
            }
            Instruction::DataProc32 {
                op,
                rn,
                rd,
                rm,
                imm5,
                shift_type,
                set_flags,
            } => {
                let op1 = self.read_reg(rn);
                let mut op2 = self.read_reg(rm);

                // Apply shift to op2
                match shift_type {
                    0 => op2 <<= imm5,                                  // LSL
                    1 => op2 = if imm5 == 0 { 0 } else { op2 >> imm5 }, // LSR
                    2 => {
                        // ASR
                        op2 = if imm5 == 0 {
                            if (op2 & 0x80000000) != 0 {
                                0xFFFFFFFF
                            } else {
                                0
                            }
                        } else {
                            ((op2 as i32) >> imm5) as u32
                        };
                    }
                    3 => {
                        if imm5 != 0 {
                            op2 = op2.rotate_right(imm5 as u32)
                        }
                    } // ROR
                    _ => {}
                }

                let mut result = 0u32;
                match op {
                    0x0 => {
                        result = op1 & op2;
                        self.write_reg(rd, result);
                    } // AND
                    0x1 => {
                        result = op1 & !op2;
                        self.write_reg(rd, result);
                    } // BIC
                    0x2 => {
                        // ORR / MOV
                        result = if rn == 0xF { op2 } else { op1 | op2 };
                        self.write_reg(rd, result);
                    }
                    0x3 => {
                        // ORN / MVN
                        result = if rn == 0xF { !op2 } else { op1 | !op2 };
                        self.write_reg(rd, result);
                    }
                    0x4 => {
                        result = op1 ^ op2;
                        self.write_reg(rd, result);
                    } // EOR
                    0x8 => {
                        result = op1.wrapping_add(op2);
                        self.write_reg(rd, result);
                    } // ADD
                    0xD => {
                        result = op1.wrapping_sub(op2);
                        self.write_reg(rd, result);
                    } // SUB
                    _ => {
                        tracing::warn!("Unknown DataProc32 op {:#x}", op);
                    }
                }

                if set_flags {
                    self.update_nz(result);
                }
                pc_increment = 4;
            }
            Instruction::Barrier => {
                // DMB / DSB / ISB are all architectural no-ops in this
                // single-threaded simulator; we just consume 4 bytes.
                pc_increment = 4;
            }
            _ => {
                // Fallback to legacy decoding
                if (h1 & 0xFE00) == 0xE800 {
                    // Load/store dual, load/store exclusive, table branch
                    let op = ((h1 >> 7) & 3) as u8;
                    let rn = (h1 & 0xF) as u8;
                    let rt = ((h2 >> 12) & 0xF) as u8;
                    let rt2 = ((h2 >> 8) & 0xF) as u8;
                    let imm8 = (h2 & 0xFF) as u32;

                    if (h1 & 0x01F0) == 0x00D0 && (h2 & 0xFFF0) == 0xF000 {
                        // TBB / TBH
                        let rm = (h2 & 0xF) as u8;
                        let is_tbh = (h2 & 0x0010) != 0;

                        let mut base = self.read_reg(rn);
                        if rn == 15 {
                            base = (self.regs[15] & !3).wrapping_add(4);
                        }
                        let index = self.read_reg(rm);

                        if is_tbh {
                            let addr = base.wrapping_add(index << 1);
                            if let Ok(halfword) = bus.read_u16(addr as u64) {
                                let offset = (halfword as u32) << 1;
                                self.regs[15] = self.regs[15].wrapping_add(4).wrapping_add(offset);
                                pc_increment = 0;
                            }
                        } else {
                            let addr = base.wrapping_add(index);
                            if let Ok(byte) = bus.read_u8(addr as u64) {
                                let offset = (byte as u32) << 1;
                                self.regs[15] = self.regs[15].wrapping_add(4).wrapping_add(offset);
                                pc_increment = 0;
                            }
                        }
                    } else if op == 2 || op == 3 {
                        // STRD / LDRD (immediate) - simplified
                        let is_load = op == 3;
                        let base = self.read_reg(rn);
                        let addr = base.wrapping_add(imm8 << 2);

                        if is_load {
                            if let Ok(v1) = bus.read_u32(addr as u64) {
                                self.write_reg(rt, v1);
                            }
                            if let Ok(v2) = bus.read_u32((addr + 4) as u64) {
                                self.write_reg(rt2, v2);
                            }
                        } else {
                            let v1 = self.read_reg(rt);
                            let v2 = self.read_reg(rt2);
                            let _ = bus.write_u32(addr as u64, v1);
                            let _ = bus.write_u32((addr + 4) as u64, v2);
                        }
                        pc_increment = 4;
                    } else {
                        // ...
                        pc_increment = 4;
                    }
                } else if (h1 & 0xF800) == 0xF000 && (h2 & 0x8000) == 0x8000 {
                    // B.W / BL
                    let s = ((h1 >> 10) & 0x1) as i32;
                    let j1 = ((h2 >> 13) & 0x1) as i32;
                    let j2 = ((h2 >> 11) & 0x1) as i32;
                    let i1 = (!(j1 ^ s)) & 0x1;
                    let i2 = (!(j2 ^ s)) & 0x1;
                    let imm11 = (h2 & 0x7FF) as i32;

                    let is_bl = (h2 & 0x1000) != 0;
                    let imm_h1 = if is_bl {
                        (h1 & 0x3FF) as i32
                    } else {
                        (h1 & 0x7FF) as i32
                    };

                    let mut offset = if is_bl {
                        (s << 24)
                            | (i1 << 23)
                            | (i2 << 22)
                            | (imm_h1 << 12)
                            | (imm11 << 1)
                    } else {
                        // T4 (B): S:I1:I2:imm11:imm11:0. Total 25 bits.
                        (s << 24)
                            | (i1 << 23)
                            | (i2 << 22)
                            | (imm_h1 << 12)
                            | (imm11 << 1)
                    };

                    if (offset & (1 << 24)) != 0 {
                        offset |= !0x01FF_FFFF;
                    }

                    if is_bl {
                        self.regs[14] = (self.regs[15] + 4) | 1;
                    }
                    self.regs[15] = (self.regs[15] as i32 + 4 + offset) as u32;
                    pc_increment = 0;
                } else if (h1 & 0xFBF0) == 0xF240 {
                    // MOVW (T1)
                    let i = (h1 >> 10) & 0x1;
                    let imm4 = h1 & 0xF;
                    let imm3 = (h2 >> 12) & 0x7;
                    let rd = ((h2 >> 8) & 0xF) as u8;
                    let imm8 = h2 & 0xFF;
                    let imm16 = (imm4 << 12) | (i << 11) | (imm3 << 8) | imm8;
                    self.write_reg(rd, imm16 as u32);
                    pc_increment = 4;
                } else if (h1 & 0xFBF0) == 0xF2C0 {
                    // MOVT (T1)
                    let i = (h1 >> 10) & 0x1;
                    let imm4 = h1 & 0xF;
                    let imm3 = (h2 >> 12) & 0x7;
                    let rd = ((h2 >> 8) & 0xF) as u8;
                    let imm8 = h2 & 0xFF;
                    let imm16 = (imm4 << 12) | (i << 11) | (imm3 << 8) | imm8;
                    let old_val = self.read_reg(rd);
                    let new_val = (old_val & 0x0000FFFF) | ((imm16 as u32) << 16);
                    self.write_reg(rd, new_val);
                    pc_increment = 4;
                } else if (h1 & 0xFB00) == 0xF000 && (h2 & 0x8000) == 0 {
                    // Data-processing (modified immediate)
                    let i = (h1 >> 10) & 0x1;
                    let op = ((h1 >> 5) & 0xF) as u8;
                    let s = ((h1 >> 4) & 0x1) != 0;
                    let rn = (h1 & 0xF) as u8;
                    let imm3 = (h2 >> 12) & 0x7;
                    let rd = ((h2 >> 8) & 0xF) as u8;
                    let imm8 = h2 & 0xFF;
                    let imm12 = (i << 11) | (imm3 << 8) | imm8;
                    let imm32 = thumb_expand_imm(imm12 as u32); // restored usage
                    let op1 = self.read_reg(rn);
                    let mut result = 0u32;
                    let mut update_pc = true;

                    match op {
                        0x0 => {
                            result = op1 & imm32;
                            self.write_reg(rd, result);
                        } // AND
                        0x1 => {
                            result = op1 & !imm32;
                            self.write_reg(rd, result);
                        } // BIC
                        0x2 => {
                            // ORR / MOV
                            result = if rn == 0xF { imm32 } else { op1 | imm32 };
                            self.write_reg(rd, result);
                        }
                        0x3 => {
                            // ORN / MVN
                            result = if rn == 0xF { !imm32 } else { op1 | !imm32 };
                            self.write_reg(rd, result);
                        }
                        0x4 => {
                            result = op1 ^ imm32;
                            self.write_reg(rd, result);
                        } // EOR
                        0x8 => {
                            result = op1.wrapping_add(imm32);
                            self.write_reg(rd, result);
                        } // ADD
                        0xA => {
                            // ADC
                            let carry = if self.regs[16] & PSR_C != 0 { 1 } else { 0 };
                            result = op1.wrapping_add(imm32).wrapping_add(carry);
                            self.write_reg(rd, result);
                        }
                        0xB => {
                            // SBC
                            let carry = if self.regs[16] & PSR_C != 0 { 1 } else { 0 };
                            result = op1.wrapping_sub(imm32).wrapping_sub(1 - carry);
                            self.write_reg(rd, result);
                        }
                        0xD => {
                            result = op1.wrapping_sub(imm32);
                            self.write_reg(rd, result);
                        } // SUB
                        0xE => {
                            result = imm32.wrapping_sub(op1);
                            self.write_reg(rd, result);
                        } // RSB
                        _ => {
                            update_pc = false;
                        }
                    }
                    if s && update_pc {
                        self.update_nz(result);
                    }
                    if update_pc {
                        pc_increment = 4;
                    }
                } else if (h1 & 0xFB00) == 0xF100 && (h2 & 0x8000) == 0 {
                    // Data-processing (plain binary immediate)
                    let i = (h1 >> 10) & 0x1;
                    let op = ((h1 >> 5) & 0xF) as u8;
                    let rn = (h1 & 0xF) as u8;
                    let imm3 = (h2 >> 12) & 0x7;
                    let rd = ((h2 >> 8) & 0xF) as u8;
                    let imm8 = h2 & 0xFF;
                    let imm12 = (i << 11) | (imm3 << 8) | imm8;
                    let op1 = self.read_reg(rn);
                    match op {
                        0x0 => {
                            self.write_reg(rd, op1.wrapping_add(imm12 as u32));
                            pc_increment = 4;
                        } // ADD
                        0xA => {
                            self.write_reg(rd, op1.wrapping_sub(imm12 as u32));
                            pc_increment = 4;
                        } // SUB
                        _ => {}
                    }
                } else if (h1 & 0xFF00) == 0xF800 {
                    // LDR/STR (immediate) T3/T4
                    let op1 = (h1 >> 4) & 0xF;
                    let rn = (h1 & 0xF) as u8;
                    let rt = ((h2 >> 12) & 0xF) as u8;
                    let is_t4 = (op1 & 0x8) == 0;
                    let is_reg_offset = is_t4 && (h2 & 0x0800) == 0;

                    if !is_reg_offset {
                        let mut supported = true;
                        let addr: u32;
                        let mut wb = false;
                        let mut wb_val = 0u32;

                        if !is_t4 {
                            // T3
                            let offset = (h2 & 0xFFF) as i32;
                            addr = self.read_reg(rn).wrapping_add(offset as u32);
                        } else {
                            // T4
                            let p = (h2 >> 10) & 1;
                            let u = (h2 >> 9) & 1;
                            let w = (h2 >> 8) & 1;
                            let imm8 = (h2 & 0xFF) as i32;
                            let offset = if u != 0 { imm8 } else { -imm8 };
                            let base = self.read_reg(rn);
                            if p != 0 {
                                addr = base.wrapping_add(offset as u32);
                                if w != 0 {
                                    wb = true;
                                    wb_val = addr;
                                }
                            } else {
                                addr = base;
                                wb = true;
                                wb_val = base.wrapping_add(offset as u32);
                            }
                        }

                        match op1 & 0x7 {
                            0 => {
                                let val = (self.read_reg(rt) & 0xFF) as u8;
                                let _ = bus.write_u8(addr as u64, val);
                            }
                            1 => {
                                if let Ok(v) = bus.read_u8(addr as u64) {
                                    self.write_reg(rt, v as u32);
                                }
                            }
                            2 => {
                                let val = (self.read_reg(rt) & 0xFFFF) as u16;
                                let _ = bus.write_u16(addr as u64, val);
                            }
                            3 => {
                                if let Ok(v) = bus.read_u16(addr as u64) {
                                    self.write_reg(rt, v as u32);
                                }
                            }
                            4 => {
                                let val = self.read_reg(rt);
                                let _ = bus.write_u32(addr as u64, val);
                            }
                            5 => {
                                if let Ok(v) = bus.read_u32(addr as u64) {
                                    self.write_reg(rt, v);
                                }
                            }
                            _ => {
                                supported = false;
                            }
                        }
                        if supported {
                            if wb {
                                self.write_reg(rn, wb_val);
                            }
                            pc_increment = 4;
                        }
                    } else {
                        // Reg offset
                        let rm = (h2 & 0xF) as u8;
                        let imm2 = ((h2 >> 4) & 0x3) as u32;
                        let addr =
                            self.read_reg(rn).wrapping_add(self.read_reg(rm) << imm2);
                        match op1 & 0x7 {
                            0 => {
                                let val = (self.read_reg(rt) & 0xFF) as u8;
                                let _ = bus.write_u8(addr as u64, val);
                            }
                            1 => {
                                if let Ok(v) = bus.read_u8(addr as u64) {
                                    self.write_reg(rt, v as u32);
                                }
                            }
                            2 => {
                                let val = (self.read_reg(rt) & 0xFFFF) as u16;
                                let _ = bus.write_u16(addr as u64, val);
                            }
                            3 => {
                                if let Ok(v) = bus.read_u16(addr as u64) {
                                    self.write_reg(rt, v as u32);
                                }
                            }
                            4 => {
                                let val = self.read_reg(rt);
                                let _ = bus.write_u32(addr as u64, val);
                            }
                            5 => {
                                if let Ok(v) = bus.read_u32(addr as u64) {
                                    self.write_reg(rt, v);
                                }
                            }
                            _ => {}
                        }
                        pc_increment = 4;
                    }
                } else if (h1 & 0xFFF0) == 0xFB90 {
                    // SDIV
                    let rn = (h1 & 0xF) as u8;
                    let rd = ((h2 >> 8) & 0xF) as u8;
                    let rm = (h2 & 0xF) as u8;
                    let dividend = self.read_reg(rn) as i32;
                    let divisor = self.read_reg(rm) as i32;
                    let result = if divisor == 0 {
                        0
                    } else {
                        dividend.wrapping_div(divisor) as u32
                    };
                    self.write_reg(rd, result);
                    pc_increment = 4;
                } else if (h1 & 0xFFF0) == 0xFBB0 {
                    // UDIV
                    let rn = (h1 & 0xF) as u8;
                    let rd = ((h2 >> 8) & 0xF) as u8;
                    let rm = (h2 & 0xF) as u8;
                    let dividend = self.read_reg(rn);
                    let divisor = self.read_reg(rm);
                    let result = if divisor == 0 { 0 } else { dividend / divisor };
                    self.write_reg(rd, result);
                    pc_increment = 4;
                } else {
                    tracing::warn!("Internal: Unhandled 32-bit: {:04x} {:04x}", h1, h2);
                    pc_increment = 4;
                }
            }
        }
    } else {
        tracing::error!("Bus Read Fault (32-bit suffix) at {:#x}", next_pc);
    }
        Ok((pc_increment, cycles))
    }
}
