// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::bus::SystemBus;
use crate::decoder::arm::{decode_thumb_16, decode_thumb_32, Instruction};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationObserver};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

// PSR Bits (Internal usage) - Omitted if unused
const PSR_C: u32 = 1 << 29;

#[derive(Debug, Clone, Copy)]
pub struct DecodeCacheEntry {
    pub tag: u32,
    pub instruction: Instruction,
    pub opcode: u32,
    pub pc_increment: u8,
    pub cycles: u32,
}

#[derive(Debug)]
pub struct CortexM {
    pub r0: u32,
    pub r1: u32,
    pub r2: u32,
    pub r3: u32,
    pub r4: u32,
    pub r5: u32,
    pub r6: u32,
    pub r7: u32,
    pub r8: u32,
    pub r9: u32,
    pub r10: u32,
    pub r11: u32,
    pub r12: u32,
    pub sp: u32, // R13
    pub lr: u32, // R14
    pub pc: u32, // R15
    pub xpsr: u32,
    pub pending_exceptions: u32, // Bitmask
    pub primask: bool,           // Interrupt mask (true = disabled)
    pub vtor: Arc<AtomicU32>,    // Shared Vector Table Offset Register
    pub it_state: u8,            // Thumb IT block state
    pub decode_cache: Box<[Option<DecodeCacheEntry>; 4096]>,
}

impl Default for CortexM {
    fn default() -> Self {
        Self {
            r0: 0,
            r1: 0,
            r2: 0,
            r3: 0,
            r4: 0,
            r5: 0,
            r6: 0,
            r7: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            sp: 0,
            lr: 0,
            pc: 0,
            xpsr: 0x01000000, // Typical reset state (Thumb bit set)
            pending_exceptions: 0,
            primask: false,
            vtor: Arc::new(AtomicU32::new(0)),
            it_state: 0,
            decode_cache: Box::new([None; 4096]),
        }
    }
}

impl CortexM {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_vtor(&self) -> u32 {
        self.vtor.load(Ordering::SeqCst)
    }

    pub fn set_vtor(&mut self, val: u32) {
        self.vtor.store(val, Ordering::SeqCst);
    }

    pub fn set_shared_vtor(&mut self, vtor: Arc<AtomicU32>) {
        self.vtor = vtor;
    }

    fn read_reg(&self, n: u8) -> u32 {
        match n {
            0 => self.r0,
            1 => self.r1,
            2 => self.r2,
            3 => self.r3,
            4 => self.r4,
            5 => self.r5,
            6 => self.r6,
            7 => self.r7,
            8 => self.r8,
            9 => self.r9,
            10 => self.r10,
            11 => self.r11,
            12 => self.r12,
            13 => self.sp,
            14 => self.lr,
            15 => self.pc,
            16 => self.xpsr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, n: u8, val: u32) {
        match n {
            0 => self.r0 = val,
            1 => self.r1 = val,
            2 => self.r2 = val,
            3 => self.r3 = val,
            4 => self.r4 = val,
            5 => self.r5 = val,
            6 => self.r6 = val,
            7 => self.r7 = val,
            8 => self.r8 = val,
            9 => self.r9 = val,
            10 => self.r10 = val,
            11 => self.r11 = val,
            12 => self.r12 = val,
            13 => self.sp = val,
            14 => self.lr = val,
            15 => self.pc = val,
            16 => self.xpsr = val,
            _ => {}
        }
    }

    fn expand_imm_thumb(imm12: u32) -> u32 {
        let i_imm3 = (imm12 >> 8) & 0xF;
        let abcdefgh = imm12 & 0xFF;

        if (i_imm3 & 0xC) == 0 {
            match i_imm3 {
                0 => abcdefgh,
                1 => (abcdefgh << 16) | abcdefgh,
                2 => (abcdefgh << 24) | (abcdefgh << 8),
                3 => (abcdefgh << 24) | (abcdefgh << 16) | (abcdefgh << 8) | abcdefgh,
                _ => abcdefgh,
            }
        } else {
            let unrotated = 0x80 | abcdefgh;
            let rotation = (imm12 >> 7) & 0x1F;
            unrotated.rotate_right(rotation)
        }
    }

    fn update_nz(&mut self, result: u32) {
        let n = (result >> 31) & 1;
        let z = if result == 0 { 1 } else { 0 };
        // Clear N/Z (bits 31, 30)
        self.xpsr &= !(0xC000_0000);
        self.xpsr |= (n << 31) | (z << 30);
    }

    fn update_nzcv(&mut self, result: u32, carry: bool, overflow: bool) {
        let n = (result >> 31) & 1;
        let z = if result == 0 { 1 } else { 0 };
        let c = if carry { 1 } else { 0 };
        let v = if overflow { 1 } else { 0 };

        self.xpsr &= !(0xF000_0000);
        self.xpsr |= (n << 31) | (z << 30) | (c << 29) | (v << 28);
    }

    #[inline(always)]
    fn check_condition(&self, cond: u8) -> bool {
        let n = (self.xpsr >> 31) & 1 == 1;
        let z = (self.xpsr >> 30) & 1 == 1;
        let c = (self.xpsr >> 29) & 1 == 1;
        let v = (self.xpsr >> 28) & 1 == 1;

        match cond {
            0x0 => z,              // EQ (Equal)
            0x1 => !z,             // NE (Not Equal)
            0x2 => c,              // CS/HS (Carry Set)
            0x3 => !c,             // CC/LO (Carry Clear)
            0x4 => n,              // MI (Minus)
            0x5 => !n,             // PL (Plus)
            0x6 => v,              // VS (Overflow)
            0x7 => !v,             // VC (No Overflow)
            0x8 => c && !z,        // HI (Unsigned Higher)
            0x9 => !c || z,        // LS (Unsigned Lower or Same)
            0xA => n == v,         // GE (Signed Greater or Equal)
            0xB => n != v,         // LT (Signed Less Than)
            0xC => !z && (n == v), // GT (Signed Greater Than)
            0xD => z || (n != v),  // LE (Signed Less or Equal)
            0xE => true,           // AL (Always)
            _ => false,            // Undefined/Reserved
        }
    }

    fn branch_to<B: Bus + ?Sized>(&mut self, addr: u32, bus: &mut B) -> SimResult<()> {
        if (addr & 0xF000_0000) == 0xF000_0000 {
            // EXC_RETURN logic
            self.exception_return(bus)?;
        } else {
            self.pc = addr & !1;
        }
        Ok(())
    }

    fn exception_return<B: Bus + ?Sized>(&mut self, bus: &mut B) -> SimResult<()> {
        // Perform Unstacking
        let frame_ptr = self.sp;

        self.r0 = bus.read_u32(frame_ptr as u64)?;
        self.r1 = bus.read_u32((frame_ptr + 4) as u64)?;
        self.r2 = bus.read_u32((frame_ptr + 8) as u64)?;
        self.r3 = bus.read_u32((frame_ptr + 12) as u64)?;
        self.r12 = bus.read_u32((frame_ptr + 16) as u64)?;
        self.lr = bus.read_u32((frame_ptr + 20) as u64)?;
        self.pc = bus.read_u32((frame_ptr + 24) as u64)?;
        self.xpsr = bus.read_u32((frame_ptr + 28) as u64)?;

        self.sp = frame_ptr + 32;

        tracing::info!("Exception return to {:#x}", self.pc);
        Ok(())
    }
}

impl Cpu for CortexM {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        self.pc = 0x0000_0000;
        self.sp = 0x2000_0000;
        self.pending_exceptions = 0;
        self.decode_cache.fill(None);

        let vtor = self.vtor.load(Ordering::SeqCst) as u64;
        if let Ok(sp) = bus.read_u32(vtor) {
            self.sp = sp;
        }
        if let Ok(pc) = bus.read_u32(vtor + 4) {
            self.pc = pc;
        }

        Ok(())
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }
    fn set_sp(&mut self, val: u32) {
        self.sp = val;
    }
    fn set_exception_pending(&mut self, exception_num: u32) {
        if exception_num < 32 {
            self.pending_exceptions |= 1 << exception_num;
        }
    }

    fn get_register(&self, id: u8) -> u32 {
        self.read_reg(id)
    }

    fn set_register(&mut self, id: u8, val: u32) {
        self.write_reg(id, val);
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::Arm(crate::snapshot::ArmCpuSnapshot {
            registers: vec![
                self.r0, self.r1, self.r2, self.r3, self.r4, self.r5, self.r6, self.r7, self.r8,
                self.r9, self.r10, self.r11, self.r12, self.sp, self.lr, self.pc,
            ],
            xpsr: self.xpsr,
            primask: self.primask,
            pending_exceptions: self.pending_exceptions,
            vtor: self.vtor.load(Ordering::Relaxed),
        })
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::Arm(s) = snapshot {
            if s.registers.len() >= 16 {
                self.r0 = s.registers[0];
                self.r1 = s.registers[1];
                self.r2 = s.registers[2];
                self.r3 = s.registers[3];
                self.r4 = s.registers[4];
                self.r5 = s.registers[5];
                self.r6 = s.registers[6];
                self.r7 = s.registers[7];
                self.r8 = s.registers[8];
                self.r9 = s.registers[9];
                self.r10 = s.registers[10];
                self.r11 = s.registers[11];
                self.r12 = s.registers[12];
                self.sp = s.registers[13];
                self.lr = s.registers[14];
                self.pc = s.registers[15];
            }
            self.xpsr = s.xpsr;
            self.primask = s.primask;
            self.pending_exceptions = s.pending_exceptions;
            self.vtor.store(s.vtor, Ordering::Relaxed);
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for i in 0..13 {
            names.push(format!("R{}", i));
        }
        names.push("SP".to_string());
        names.push("LR".to_string());
        names.push("PC".to_string());
        names
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        self.step_internal(bus, observers, config)
    }

    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        if !config.batch_mode_enabled {
            for _ in 0..max_count {
                self.step(bus, observers, config)?;
            }
            return Ok(max_count);
        }

        let mut executed = 0;
        let empty_observers: &[Arc<dyn SimulationObserver>] = &[];

        if let Some(sysbus) = bus.as_any_mut().and_then(|a| a.downcast_mut::<SystemBus>()) {
            while executed < max_count {
                if self.pending_exceptions != 0 && !self.primask {
                    break;
                }
                let old_pc = self.pc;
                self.step_internal(sysbus, empty_observers, config)?;
                executed += 1;
                let pc_diff = self.pc.wrapping_sub(old_pc);
                if pc_diff != 2 && pc_diff != 4 {
                    break;
                }
            }
        } else {
            while executed < max_count {
                if self.pending_exceptions != 0 && !self.primask {
                    break;
                }
                let old_pc = self.pc;
                self.step_internal(bus, empty_observers, config)?;
                executed += 1;
                let pc_diff = self.pc.wrapping_sub(old_pc);
                if pc_diff != 2 && pc_diff != 4 {
                    break;
                }
            }
        }

        Ok(executed)
    }
}

impl CortexM {
    #[inline(always)]
    fn step_internal<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        _observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        // Check for pending exceptions before executing instruction
        if self.pending_exceptions != 0 {
            // Find highest priority exception (Simplified: highest bit)
            let exception_num = 31 - self.pending_exceptions.leading_zeros();
            self.pending_exceptions &= !(1 << exception_num);

            // Perform Stacking (Simplified)
            let sp = self.sp;
            let frame_ptr = sp.wrapping_sub(32);

            // Stack: R0, R1, R2, R3, R12, LR, PC, xPSR
            let _ = bus.write_u32(frame_ptr as u64, self.r0);
            let _ = bus.write_u32((frame_ptr + 4) as u64, self.r1);
            let _ = bus.write_u32((frame_ptr + 8) as u64, self.r2);
            let _ = bus.write_u32((frame_ptr + 12) as u64, self.r3);
            let _ = bus.write_u32((frame_ptr + 16) as u64, self.r12);
            let _ = bus.write_u32((frame_ptr + 20) as u64, self.lr);
            let _ = bus.write_u32((frame_ptr + 24) as u64, self.pc);
            let _ = bus.write_u32((frame_ptr + 28) as u64, self.xpsr);

            self.sp = frame_ptr;

            // EXC_RETURN: Thread Mode, MSP
            self.lr = 0xFFFF_FFF9;

            // Jump to ISR handler
            let vtor = self.vtor.load(Ordering::SeqCst);
            let vector_addr = vtor + (exception_num * 4);
            if let Ok(handler) = bus.read_u32(vector_addr as u64) {
                self.pc = handler & !1;
                tracing::info!(
                    "Exception {} trigger, jump to {:#x} (VTOR={:#x})",
                    exception_num,
                    self.pc,
                    vtor
                );
            }

            return Ok(());
        }
        // Fetch/Decode with optional Cache
        let cache_idx = ((self.pc >> 1) & 0xFFF) as usize;
        let entry = if config.decode_cache_enabled {
            if let Some(e) = self.decode_cache[cache_idx] {
                if e.tag == self.pc {
                    Some(e)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let (instruction, opcode, mut pc_increment, _cycles) = if let Some(e) = entry {
            (e.instruction, e.opcode, e.pc_increment as u32, e.cycles)
        } else {
            let fetch_pc = self.pc & !1;
            let h1 = bus.read_u16(fetch_pc as u64)?;
            let is_32bit = (h1 & 0xE000) == 0xE000 && (h1 & 0x1800) != 0;

            let (instr, op, pincr, cyc) = if is_32bit {
                let h2 = bus.read_u16((fetch_pc + 2) as u64)?;
                let instr = decode_thumb_32(h1, h2);
                let op = ((h1 as u32) << 16) | h2 as u32;
                (instr, op, 4, 2)
            } else {
                let instr = decode_thumb_16(h1);
                (instr, h1 as u32, 2, 1)
            };

            if config.decode_cache_enabled {
                self.decode_cache[cache_idx] = Some(DecodeCacheEntry {
                    tag: self.pc,
                    instruction: instr,
                    opcode: op,
                    pc_increment: pincr as u8,
                    cycles: cyc,
                });
            }

            (instr, op, pincr as u32, cyc)
        };

        if !_observers.is_empty() {
            for observer in _observers {
                observer.on_step_start(self.pc, opcode);
            }
        }

        let mut execute = true;
        let mut it_block_instruction = false;

        if self.it_state != 0 {
            it_block_instruction = true;
            let cond = self.it_state >> 4;
            execute = self.check_condition(cond);
        }

        if execute {
            #[cfg(debug_assertions)]
            tracing::debug!(
                "PC={:#x}, Opcode={:#04x}, Instr={:?}",
                self.pc,
                opcode,
                instruction
            );

            // Execute
            match instruction {
                Instruction::Bfi { rd, rn, lsb, width } => {
                    let src = self.read_reg(rn);
                    let dst = self.read_reg(rd);
                    let mask = if width == 32 {
                        !0
                    } else {
                        ((1u32.wrapping_shl(width as u32)).wrapping_sub(1)).wrapping_shl(lsb as u32)
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
                        ((1u32.wrapping_shl(width as u32)).wrapping_sub(1)).wrapping_shl(lsb as u32)
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
                        ((val.wrapping_shl(shift as u32)) as i32).wrapping_shr(shift as u32) as u32
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
                Instruction::Sdiv { rd, rn, rm } => {
                    let n = self.read_reg(rn) as i32;
                    let m = self.read_reg(rm) as i32;
                    let result = if m == 0 {
                        0
                    } else if n == i32::MIN && m == -1 {
                        i32::MIN as u32
                    } else {
                        (n / m) as u32
                    };
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Udiv { rd, rn, rm } => {
                    let n = self.read_reg(rn);
                    let m = self.read_reg(rm);
                    let result = if m == 0 { 0 } else { n / m };
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
                    let mut op2 = self.read_reg(rm);
                    match shift_type {
                        0 => op2 = op2.wrapping_shl(imm5 as u32), // LSL
                        1 => {
                            op2 = if imm5 == 0 {
                                0
                            } else {
                                op2.wrapping_shr(imm5 as u32)
                            }
                        } // LSR
                        2 => {
                            op2 = if imm5 == 0 {
                                if (op2 & 0x80000000) != 0 {
                                    0xFFFFFFFF
                                } else {
                                    0
                                }
                            } else {
                                ((op2 as i32) >> (imm5 as u32)) as u32
                            }
                        } // ASR
                        3 => {
                            if imm5 != 0 {
                                op2 = op2.rotate_right(imm5 as u32)
                            }
                        } // ROR
                        _ => {}
                    }
                    let op1 = self.read_reg(rn);
                    let result = match op {
                        0x0 => op1 & op2,  // AND
                        0x1 => op1 & !op2, // BIC
                        0x2 => {
                            if rn == 0xF {
                                op2
                            } else {
                                op1 | op2
                            }
                        } // ORR / MOV
                        0x3 => {
                            if rn == 0xF {
                                !op2
                            } else {
                                op1 | !op2
                            }
                        } // ORN / MVN
                        0x4 => op1 ^ op2,  // EOR
                        0x8 => op1.wrapping_add(op2), // ADD
                        0xD => op1.wrapping_sub(op2), // SUB
                        _ => {
                            #[cfg(debug_assertions)]
                            tracing::warn!("Unknown DataProc32 op {:#x}", op);
                            op2
                        }
                    };
                    if rd != 15 {
                        self.write_reg(rd, result);
                    }
                    if set_flags {
                        self.update_nz(result);
                    }
                    pc_increment = 4;
                }
                Instruction::DataProcImm32 {
                    op,
                    rn,
                    rd,
                    imm12,
                    set_flags,
                } => {
                    let imm = Self::expand_imm_thumb(imm12);
                    let op1 = self.read_reg(rn);
                    let result = match op {
                        0x0 => op1 & imm,  // AND
                        0x1 => op1 & !imm, // BIC
                        0x2 => {
                            if rn == 0xF {
                                imm
                            } else {
                                op1 | imm
                            }
                        } // ORR / MOV
                        0x3 => {
                            if rn == 0xF {
                                !imm
                            } else {
                                op1 | !imm
                            }
                        } // ORN / MVN
                        0x4 => op1 ^ imm,  // EOR
                        0x8 => op1.wrapping_add(imm), // ADD
                        0xA => {
                            let c = if self.xpsr & PSR_C != 0 { 1 } else { 0 };
                            op1.wrapping_add(imm).wrapping_add(c)
                        } // ADC
                        0xB => {
                            let c = if self.xpsr & PSR_C != 0 { 1 } else { 0 };
                            op1.wrapping_sub(imm).wrapping_sub(1 - c)
                        } // SBC
                        0xD => op1.wrapping_sub(imm), // SUB
                        0xE => imm.wrapping_sub(op1), // RSB
                        _ => {
                            #[cfg(debug_assertions)]
                            tracing::warn!("Unknown DataProcImm32 op {:#x}", op);
                            imm
                        }
                    };
                    if rd != 15 {
                        self.write_reg(rd, result);
                    }
                    if set_flags {
                        self.update_nz(result);
                    }
                    pc_increment = 4;
                }
                Instruction::ShiftReg32 {
                    rd,
                    rn,
                    rm,
                    shift_type,
                } => {
                    let value = self.read_reg(rn);
                    let shift = self.read_reg(rm) & 0xFF;
                    let result = match shift_type {
                        0 => {
                            if shift >= 32 {
                                0
                            } else {
                                value.wrapping_shl(shift)
                            }
                        }
                        1 => {
                            if shift == 0 {
                                value
                            } else if shift >= 32 {
                                0
                            } else {
                                value.wrapping_shr(shift)
                            }
                        }
                        2 => {
                            if shift == 0 {
                                value
                            } else if shift >= 32 {
                                if (value & 0x8000_0000) != 0 {
                                    0xFFFF_FFFF
                                } else {
                                    0
                                }
                            } else {
                                ((value as i32) >> shift) as u32
                            }
                        }
                        3 => {
                            if shift == 0 {
                                value
                            } else {
                                value.rotate_right(shift % 32)
                            }
                        }
                        _ => value,
                    };
                    self.write_reg(rd, result);
                    pc_increment = 4;
                }
                Instruction::Movw { rd, imm } => {
                    self.write_reg(rd, imm as u32);
                    pc_increment = 4;
                }
                Instruction::Movt { rd, imm } => {
                    let old_val = self.read_reg(rd);
                    let new_val = (old_val & 0x0000FFFF) | ((imm as u32) << 16);
                    self.write_reg(rd, new_val);
                    pc_increment = 4;
                }
                Instruction::LdrImm32 { rt, rn, imm12 } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm12 as u32);
                    if let Ok(val) = bus.read_u32(addr as u64) {
                        self.write_reg(rt, val);
                    }
                    pc_increment = 4;
                }
                Instruction::StrImm32 { rt, rn, imm12 } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm12 as u32);
                    let val = self.read_reg(rt);
                    let _ = bus.write_u32(addr as u64, val);
                    pc_increment = 4;
                }
                Instruction::Ldrd { rt, rt2, rn, imm8 } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm8 << 2);
                    if let Ok(v1) = bus.read_u32(addr as u64) {
                        self.write_reg(rt, v1);
                    }
                    if let Ok(v2) = bus.read_u32((addr + 4) as u64) {
                        self.write_reg(rt2, v2);
                    }
                    pc_increment = 4;
                }
                Instruction::Strd { rt, rt2, rn, imm8 } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm8 << 2);
                    let v1 = self.read_reg(rt);
                    let v2 = self.read_reg(rt2);
                    let _ = bus.write_u32(addr as u64, v1);
                    let _ = bus.write_u32((addr + 4) as u64, v2);
                    pc_increment = 4;
                }
                Instruction::Tbb { rn, rm } => {
                    let mut base = self.read_reg(rn);
                    if rn == 15 {
                        base = (self.pc & !3).wrapping_add(4);
                    }
                    let index = self.read_reg(rm);
                    let addr = base.wrapping_add(index);
                    if let Ok(byte) = bus.read_u8(addr as u64) {
                        let offset = (byte as u32) << 1;
                        self.pc = self.pc.wrapping_add(4).wrapping_add(offset);
                        pc_increment = 0;
                    }
                }
                Instruction::Tbh { rn, rm } => {
                    let mut base = self.read_reg(rn);
                    if rn == 15 {
                        base = (self.pc & !3).wrapping_add(4);
                    }
                    let index = self.read_reg(rm);
                    let addr = base.wrapping_add(index << 1);
                    if let Ok(halfword) = bus.read_u16(addr as u64) {
                        let offset = (halfword as u32) << 1;
                        self.pc = self.pc.wrapping_add(4).wrapping_add(offset);
                        pc_increment = 0;
                    }
                }
                Instruction::Unknown32(h1, h2) => {
                    // Manual fallback for complex bit patterns not yet in Instruction enum
                    if (h1 & 0xFE00) == 0xE800 {
                        // Table branch, load/store exclusive etc (handled by Tbb/Tbh above usually, but just in case)
                        pc_increment = 4;
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
                                let offset = (h2 & 0xFFF) as i32;
                                addr = self.read_reg(rn).wrapping_add(offset as u32);
                            } else {
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
                                    let _ = bus.write_u32(addr as u64, self.read_reg(rt));
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
                            // Register offset (T2)
                            let rn = (h1 & 0xF) as u8;
                            let rt = ((h2 >> 12) & 0xF) as u8;
                            let rm = (h2 & 0xF) as u8;
                            let imm2 = ((h2 >> 4) & 0x3) as u8;
                            let base = self.read_reg(rn);
                            let offset = self.read_reg(rm).wrapping_shl(imm2 as u32);
                            let addr = base.wrapping_add(offset);
                            match op1 & 0x7 {
                                1 => {
                                    if let Ok(v) = bus.read_u8(addr as u64) {
                                        self.write_reg(rt, v as u32);
                                    }
                                }
                                3 => {
                                    if let Ok(v) = bus.read_u16(addr as u64) {
                                        self.write_reg(rt, v as u32);
                                    }
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
                    } else if (h1 & 0xFB00) == 0xF000 && (h2 & 0x8000) == 0 {
                        // Data-processing (modified immediate) - repeated here for safety but usually handled by DataProcImm32
                        let i = (h1 >> 10) & 0x1;
                        let op = ((h1 >> 5) & 0xF) as u8;
                        let s = ((h1 >> 4) & 0x1) != 0;
                        let rn = (h1 & 0xF) as u8;
                        let imm3 = (h2 >> 12) & 0x7;
                        let rd = ((h2 >> 8) & 0xF) as u8;
                        let imm8 = h2 & 0xFF;
                        let imm12 = (i << 11) | (imm3 << 8) | imm8;
                        let imm32 = thumb_expand_imm(imm12 as u32);
                        let op1 = self.read_reg(rn);
                        let mut result = 0u32;
                        let mut update_rd = true;
                        match op {
                            0x0 => result = op1 & imm32,                                  // AND
                            0x1 => result = op1 & !imm32,                                 // BIC
                            0x2 => result = if rn == 15 { imm32 } else { op1 | imm32 },   // ORR/MOV
                            0x3 => result = if rn == 15 { !imm32 } else { op1 | !imm32 }, // ORN/MVN
                            0x4 => result = op1 ^ imm32,                                  // EOR
                            0x8 => result = op1.wrapping_add(imm32),                      // ADD
                            0xD => result = op1.wrapping_sub(imm32),                      // SUB
                            _ => update_rd = false,
                        }
                        if update_rd {
                            if rd != 15 {
                                self.write_reg(rd, result);
                            }
                            if s {
                                self.update_nz(result);
                            }
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
                    } else if (h1 & 0xF000) == 0xF000 && (h2 & 0x8000) == 0x8000 {
                        // B.W / BL (handled elsewhere but just in case)
                        pc_increment = 4;
                    } else {
                        tracing::warn!(
                            "Unknown 32-bit instruction at {:#x}: {:#x} {:#x}",
                            self.pc,
                            h1,
                            h2
                        );
                        pc_increment = 4;
                    }
                }

                Instruction::Nop => { /* Do nothing */ }
                Instruction::MovImm { rd, imm } => {
                    self.write_reg(rd, imm as u32);
                    self.update_nz(imm as u32);
                }
                // Control Flow
                Instruction::Cbz { rn, imm } => {
                    if self.read_reg(rn) == 0 {
                        self.pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
                        pc_increment = 0;
                    }
                }
                Instruction::Cbnz { rn, imm } => {
                    if self.read_reg(rn) != 0 {
                        self.pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
                        pc_increment = 0;
                    }
                }
                Instruction::Branch { offset } => {
                    let target = (self.pc as i32 + 4 + offset) as u32;
                    self.pc = target;
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

                Instruction::Uxtb { rd, rm } => {
                    let val = self.read_reg(rm);
                    self.write_reg(rd, val & 0xFF);
                }

                Instruction::It { cond, mask } => {
                    self.it_state = (cond << 4) | mask;
                    it_block_instruction = false; // The IT instruction itself doesn't count towards the block's instructions
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
                Instruction::Bic { rd, rm } => {
                    let res = self.read_reg(rd) & !self.read_reg(rm);
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
                Instruction::Adc { rd, rm } => {
                    let op1 = self.read_reg(rd);
                    let op2 = self.read_reg(rm);
                    let carry_in = (self.xpsr >> 29) & 1;
                    let (res, c, v) = adc_with_flags(op1, op2, carry_in);
                    self.write_reg(rd, res);
                    self.update_nzcv(res, c, v);
                }
                Instruction::Sbc { rd, rm } => {
                    let op1 = self.read_reg(rd);
                    let op2 = self.read_reg(rm);
                    let carry_in = (self.xpsr >> 29) & 1;
                    let (res, c, v) = sbc_with_flags(op1, op2, carry_in);
                    self.write_reg(rd, res);
                    self.update_nzcv(res, c, v);
                }
                Instruction::Ror { rd, rm } => {
                    let val = self.read_reg(rd);
                    let shift = self.read_reg(rm) & 0xFF;
                    let res = if shift == 0 {
                        val
                    } else {
                        val.rotate_right(shift % 32)
                    };
                    self.write_reg(rd, res);
                    self.update_nz(res);
                }
                Instruction::Rev { rd, rm } => {
                    let val = self.read_reg(rm);
                    self.write_reg(rd, val.swap_bytes());
                }
                Instruction::Rev16 { rd, rm } => {
                    let val = self.read_reg(rm);
                    let low = ((val & 0xFF) << 8) | ((val >> 8) & 0xFF);
                    let high = ((val & 0x00FF0000) << 8) | ((val & 0xFF000000) >> 8);
                    self.write_reg(rd, high | low);
                }
                Instruction::RevSh { rd, rm } => {
                    let val = self.read_reg(rm);
                    let low = ((val & 0xFF) << 8) | ((val >> 8) & 0xFF);
                    self.write_reg(rd, (low as i16) as u32);
                }
                Instruction::Tst { rn, rm } => {
                    let res = self.read_reg(rn) & self.read_reg(rm);
                    self.update_nz(res);
                }
                Instruction::Cmn { rn, rm } => {
                    let op1 = self.read_reg(rn);
                    let op2 = self.read_reg(rm);
                    let (res, c, v) = add_with_flags(op1, op2);
                    self.update_nzcv(res, c, v);
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
                    if let Ok(val) = bus.read_u32(addr as u64) {
                        self.write_reg(rt, val);
                        if val == 0x021d0000 {
                            tracing::info!("LDR Literal/Imm SUSPICIOUS: R{} loaded with {:#x} from {:#x} (PC={:#x})", rt, val, addr, self.pc);
                        }
                    } else {
                        tracing::error!(
                            "Bus Read Fault at {:#x} (PC={:#x}, Opcode={:#04x})",
                            addr,
                            self.pc,
                            opcode
                        );
                    }
                }
                Instruction::StrImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = self.read_reg(rt);
                    if bus.write_u32(addr as u64, val).is_err() {
                        tracing::error!(
                            "Bus Write Fault at {:#x} (PC={:#x}, Opcode={:#04x})",
                            addr,
                            self.pc,
                            opcode
                        );
                    }
                }
                Instruction::LdrReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    if let Ok(val) = bus.read_u32(addr as u64) {
                        self.write_reg(rt, val);
                    } else {
                        tracing::error!("Bus Read Fault (LDR reg) at {:#x}", addr);
                    }
                }

                Instruction::LdrLit { rt, imm } => {
                    // ... (existing)
                    let pc_val = (self.pc & !3) + 4;
                    let addr = pc_val.wrapping_add(imm as u32);
                    if let Ok(val) = bus.read_u32(addr as u64) {
                        self.write_reg(rt, val);
                        if val == 0x021d0000 {
                            tracing::info!(
                                "LDR Lit SUSPICIOUS: R{} loaded with {:#x} from {:#x} (PC={:#x})",
                                rt,
                                val,
                                addr,
                                self.pc
                            );
                        }
                    } else {
                        tracing::error!("Bus Read Fault (LdrLit) at {:#x}", addr);
                    }
                }

                Instruction::LdrSp { rt, imm } => {
                    let addr = self.sp.wrapping_add(imm as u32);
                    if let Ok(val) = bus.read_u32(addr as u64) {
                        self.write_reg(rt, val);
                        if val == 0x021d0000 {
                            tracing::info!(
                                "LDR Sp SUSPICIOUS: R{} loaded with {:#x} from {:#x} (PC={:#x})",
                                rt,
                                val,
                                addr,
                                self.pc
                            );
                        }
                    } else {
                        tracing::error!("Bus Read Fault (LdrSp) at {:#x}", addr);
                    }
                }
                Instruction::StrSp { rt, imm } => {
                    let addr = self.sp.wrapping_add(imm as u32);
                    let val = self.read_reg(rt);
                    if bus.write_u32(addr as u64, val).is_err() {
                        tracing::error!("Bus Write Fault (StrSp) at {:#x}", addr);
                    }
                }
                Instruction::AddSpReg { rd, imm } => {
                    let res = self.sp.wrapping_add(imm as u32);
                    self.write_reg(rd, res);
                }
                Instruction::Adr { rd, imm } => {
                    let pc_val = (self.pc & !3) + 4;
                    let res = pc_val.wrapping_add(imm as u32);
                    self.write_reg(rd, res);
                }

                // Memory Operations (Byte)
                Instruction::LdrbImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    if let Ok(val) = bus.read_u8(addr as u64) {
                        self.write_reg(rt, val as u32);
                    } else {
                        tracing::error!(
                            "Bus Read Fault (LDRB) at {:#x} (PC={:#x}, Opcode={:#04x})",
                            addr,
                            self.pc,
                            opcode
                        );
                    }
                }
                Instruction::LdrbReg { rt, rn, rm } => {
                    let addr = self.read_reg(rn).wrapping_add(self.read_reg(rm));
                    if let Ok(val) = bus.read_u8(addr as u64) {
                        self.write_reg(rt, val as u32);
                    } else {
                        tracing::error!("Bus Read Fault (LDRB reg) at {:#x}", addr);
                    }
                }
                Instruction::StrbImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = (self.read_reg(rt) & 0xFF) as u8;
                    if bus.write_u8(addr as u64, val).is_err() {
                        tracing::error!(
                            "Bus Write Fault (STRB) at {:#x} (PC={:#x}, Opcode={:#04x})",
                            addr,
                            self.pc,
                            opcode
                        );
                    }
                }
                Instruction::LdrhImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    if let Ok(val) = bus.read_u16(addr as u64) {
                        self.write_reg(rt, val as u32);
                    } else {
                        tracing::error!("Bus Read Fault (LDRH) at {:#x}", addr);
                    }
                }
                Instruction::StrhImm { rt, rn, imm } => {
                    let base = self.read_reg(rn);
                    let addr = base.wrapping_add(imm as u32);
                    let val = (self.read_reg(rt) & 0xFFFF) as u16;
                    if bus.write_u16(addr as u64, val).is_err() {
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
                        if bus.write_u32(sp as u64, val).is_err() {
                            tracing::error!("Stack Overflow (PUSH LR)");
                        }
                    }

                    // Registers R7 down to R0
                    for i in (0..=7).rev() {
                        if (registers & (1 << i)) != 0 {
                            sp = sp.wrapping_sub(4);
                            let val = self.read_reg(i);
                            if bus.write_u32(sp as u64, val).is_err() {
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
                            if let Ok(val) = bus.read_u32(sp as u64) {
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
                        if let Ok(val) = bus.read_u32(sp as u64) {
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
                            if let Ok(val) = bus.read_u32(base as u64) {
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
                            if bus.write_u32(base as u64, val).is_err() {
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
                    let _next_pc = self.pc + 4; // 32-bit instruction size for BL?
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

                    self.lr = (self.pc + 4) | 1;
                    let target = (self.pc as i32 + 4 + offset) as u32;
                    self.pc = target;
                    pc_increment = 0;
                }
                Instruction::BranchCond { cond, offset } => {
                    if self.check_condition(cond) {
                        let target = (self.pc as i32 + 4 + offset) as u32;
                        self.pc = target;
                        pc_increment = 0;
                    }
                }
                Instruction::Bx { rm } => {
                    let target = self.read_reg(rm);
                    self.branch_to(target, bus)?;
                    pc_increment = 0;
                }

                Instruction::Unknown(op) => {
                    tracing::warn!("Unknown instruction at {:#x}: Opcode {:#06x}", self.pc, op);
                    pc_increment = 2; // Skip 16-bit
                }
            }
        }

        if it_block_instruction && self.it_state != 0 {
            // ITSTATEUpdate()
            if (self.it_state & 0x7) == 0 {
                self.it_state = 0;
            } else {
                let cond = self.it_state & 0xF0;
                let mask = (self.it_state & 0x07) << 1;
                self.it_state = cond | mask;
            }
        }

        self.pc = self.pc.wrapping_add(pc_increment);

        if !_observers.is_empty() {
            for observer in _observers {
                observer.on_step_end(_cycles);
            }
        }

        Ok(())
    }
}

// Thumb expand immediate - implements ARM's modified immediate constant expansion
fn thumb_expand_imm(imm12: u32) -> u32 {
    let i = (imm12 >> 11) & 1;
    let imm3 = (imm12 >> 8) & 7;
    let imm8 = imm12 & 0xFF;

    if i == 0 && (imm3 >> 2) == 0 {
        // i:imm3 is 0000, 0001, 0010, 0011.
        // Match repetition patterns:
        match imm3 {
            0 => imm8,                       // 00000000 00000000 00000000 abcdefgh
            1 => (imm8 << 16) | imm8,        // 00000000 abcdefgh 00000000 abcdefgh
            2 => (imm8 << 24) | (imm8 << 8), // abcdefgh 00000000 abcdefgh 00000000
            3 => (imm8 << 24) | (imm8 << 16) | (imm8 << 8) | imm8, // abcdefgh abcdefgh abcdefgh abcdefgh
            _ => unreachable!(),
        }
    } else {
        // Rotated immediate
        // The value to rotate is '1' concatenated with bits 6:0 of imm8.
        let val = 0x80 | (imm8 & 0x7F);
        // The rotation amount 'n' is i:imm3:imm8[7]
        let n = (i << 4) | (imm3 << 1) | (imm8 >> 7);
        val.rotate_right(n)
    }
}

fn add_with_flags(op1: u32, op2: u32) -> (u32, bool, bool) {
    let (res, overflow1) = op1.overflowing_add(op2);
    let carry = overflow1;
    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    let overflow = (neg_op1 == neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

fn adc_with_flags(op1: u32, op2: u32, carry_in: u32) -> (u32, bool, bool) {
    let (res1, c1) = op1.overflowing_add(op2);
    let (res, c2) = res1.overflowing_add(carry_in);
    let carry = c1 || c2;

    // Overflow: operands have same sign AND result has different sign
    // Effectively (op1 + op2 + carry) overflowed signed range.
    // Approximate check:
    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    // Overflow if inputs same sign, output different
    // Note: Carry_in 0 or 1 usually doesn't change sign logic much, but rigorous check:
    // Sign of (op1 + op2 + carry). It's simpler to rely on basic sign logic or specific algo.
    // ARM ref: Overflow = (op1<31> == op2<31>) && (res<31> != op1<31>)
    // Wait, carry_in effectively adds small value.
    // If op1=MAX, op2=1, c=0 -> overflow pos to neg.
    // Standard V flag logic:
    let overflow = (neg_op1 == neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

fn sub_with_flags(op1: u32, op2: u32) -> (u32, bool, bool) {
    let (res, borrow) = op1.overflowing_sub(op2);
    let carry = !borrow;
    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    let overflow = (neg_op1 != neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}

fn sbc_with_flags(op1: u32, op2: u32, carry_in: u32) -> (u32, bool, bool) {
    // SBC: op1 - op2 - NOT(carry) = op1 - op2 - (1 - carry)
    let borrow_in = 1 - carry_in;
    let (res1, b1) = op1.overflowing_sub(op2);
    let (res, b2) = res1.overflowing_sub(borrow_in);
    let borrow = b1 || b2;
    let carry = !borrow;

    let neg_op1 = (op1 as i32) < 0;
    let neg_op2 = (op2 as i32) < 0;
    let neg_res = (res as i32) < 0;
    let overflow = (neg_op1 != neg_op2) && (neg_res != neg_op1);
    (res, carry, overflow)
}
