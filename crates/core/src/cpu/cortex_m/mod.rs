// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::decoder::arm::{decode_thumb_16, Instruction};
use crate::{Bus, Cpu, SimResult, SimulationObserver};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

mod helpers;
use helpers::{add_with_flags, sub_with_flags, thumb_expand_imm, PSR_C};

// Register-file indices. Reads/writes to the `regs` array use these.
pub const SP: usize = 13; // R13 — Stack Pointer
pub const LR: usize = 14; // R14 — Link Register
pub const PC: usize = 15; // R15 — Program Counter
pub const XPSR: usize = 16;

#[derive(Debug, Default)]
pub struct CortexM {
    /// General-purpose + special registers: R0..R12, SP (13), LR (14), PC (15), XPSR (16).
    pub regs: [u32; 17],
    pub pending_exceptions: u32, // Bitmask
    pub primask: bool,           // Interrupt mask (true = disabled)
    pub vtor: Arc<AtomicU32>,    // Shared Vector Table Offset Register
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
        self.regs.get(n as usize).copied().unwrap_or(0)
    }

    fn write_reg(&mut self, n: u8, val: u32) {
        if let Some(r) = self.regs.get_mut(n as usize) {
            *r = val;
        }
    }

    fn update_nz(&mut self, result: u32) {
        let n = (result >> 31) & 1;
        let z = if result == 0 { 1 } else { 0 };
        // Clear N/Z (bits 31, 30)
        self.regs[16] &= !(0xC000_0000);
        self.regs[16] |= (n << 31) | (z << 30);
    }

    fn update_nzcv(&mut self, result: u32, carry: bool, overflow: bool) {
        let n = (result >> 31) & 1;
        let z = if result == 0 { 1 } else { 0 };
        let c = if carry { 1 } else { 0 };
        let v = if overflow { 1 } else { 0 };

        self.regs[16] &= !(0xF000_0000);
        self.regs[16] |= (n << 31) | (z << 30) | (c << 29) | (v << 28);
    }

    fn check_condition(&self, cond: u8) -> bool {
        let n = (self.regs[16] >> 31) & 1 == 1;
        let z = (self.regs[16] >> 30) & 1 == 1;
        let c = (self.regs[16] >> 29) & 1 == 1;
        let v = (self.regs[16] >> 28) & 1 == 1;

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

    fn branch_to(&mut self, addr: u32, bus: &mut dyn Bus) -> SimResult<()> {
        if (addr & 0xF000_0000) == 0xF000_0000 {
            // EXC_RETURN logic
            self.exception_return(bus)?;
        } else {
            self.regs[15] = addr & !1;
        }
        Ok(())
    }

    fn exception_return(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        // Perform Unstacking
        let frame_ptr = self.regs[13];

        self.regs[0] = bus.read_u32(frame_ptr as u64)?;
        self.regs[1] = bus.read_u32((frame_ptr + 4) as u64)?;
        self.regs[2] = bus.read_u32((frame_ptr + 8) as u64)?;
        self.regs[3] = bus.read_u32((frame_ptr + 12) as u64)?;
        self.regs[12] = bus.read_u32((frame_ptr + 16) as u64)?;
        self.regs[14] = bus.read_u32((frame_ptr + 20) as u64)?;
        self.regs[15] = bus.read_u32((frame_ptr + 24) as u64)?;
        self.regs[16] = bus.read_u32((frame_ptr + 28) as u64)?;

        self.regs[13] = frame_ptr + 32;

        tracing::info!("Exception return to {:#x}", self.regs[15]);
        Ok(())
    }
}

impl Cpu for CortexM {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        self.regs[15] = 0x0000_0000;
        self.regs[13] = 0x2000_0000;
        self.pending_exceptions = 0;

        let vtor = self.vtor.load(Ordering::SeqCst) as u64;
        if let Ok(sp) = bus.read_u32(vtor) {
            self.regs[13] = sp;
        }
        if let Ok(pc) = bus.read_u32(vtor + 4) {
            self.regs[15] = pc;
        }

        Ok(())
    }

    fn get_pc(&self) -> u32 {
        self.regs[15]
    }
    fn set_pc(&mut self, val: u32) {
        self.regs[15] = val;
    }
    fn set_sp(&mut self, val: u32) {
        self.regs[13] = val;
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
            registers: self.regs[..16].to_vec(),
            xpsr: self.regs[16],
            primask: self.primask,
            pending_exceptions: self.pending_exceptions,
            vtor: self.vtor.load(Ordering::Relaxed),
        })
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::Arm(s) = snapshot {
            let n = s.registers.len().min(16);
            self.regs[..n].copy_from_slice(&s.registers[..n]);
            self.regs[16] = s.xpsr;
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
    ) -> SimResult<()> {
        static STEP_COUNT: AtomicU32 = AtomicU32::new(0);
        // Check for pending exceptions before executing instruction
        if self.pending_exceptions != 0 {
            // Find highest priority exception (Simplified: highest bit)
            let exception_num = 31 - self.pending_exceptions.leading_zeros();
            self.pending_exceptions &= !(1 << exception_num);

            // Perform Stacking (Simplified)
            let sp = self.regs[13];
            let frame_ptr = sp.wrapping_sub(32);

            // Stack: R0, R1, R2, R3, R12, LR, PC, xPSR
            let _ = bus.write_u32(frame_ptr as u64, self.regs[0]);
            let _ = bus.write_u32((frame_ptr + 4) as u64, self.regs[1]);
            let _ = bus.write_u32((frame_ptr + 8) as u64, self.regs[2]);
            let _ = bus.write_u32((frame_ptr + 12) as u64, self.regs[3]);
            let _ = bus.write_u32((frame_ptr + 16) as u64, self.regs[12]);
            let _ = bus.write_u32((frame_ptr + 20) as u64, self.regs[14]);
            let _ = bus.write_u32((frame_ptr + 24) as u64, self.regs[15]);
            let _ = bus.write_u32((frame_ptr + 28) as u64, self.regs[16]);

            self.regs[13] = frame_ptr;

            // EXC_RETURN: Thread Mode, MSP
            self.regs[14] = 0xFFFF_FFF9;

            // Jump to ISR handler
            let vtor = self.vtor.load(Ordering::SeqCst);
            let vector_addr = vtor + (exception_num * 4);
            if let Ok(handler) = bus.read_u32(vector_addr as u64) {
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

        // ... (existing logic)
        // Fetch 16-bit thumb instruction
        let fetch_pc = self.regs[15] & !1;
        let opcode = bus.read_u16(fetch_pc as u64)?;

        for observer in observers {
            observer.on_step_start(self.regs[15], opcode as u32);
        }

        // Decode
        let instruction = decode_thumb_16(opcode);

        let count = STEP_COUNT.fetch_add(1, Ordering::SeqCst);
        if count.is_multiple_of(100000) {
            tracing::info!("CPU STEP {}: PC={:#x}", count, self.regs[15]);
        }

        tracing::debug!(
            "PC={:#x}, Opcode={:#04x}, Instr={:?}",
            self.regs[15],
            opcode,
            instruction
        );

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
            | Instruction::Movt { .. } => {
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
                if let Ok(val) = bus.read_u32(addr as u64) {
                    self.write_reg(rt, val);
                } else {
                    tracing::error!("Bus Read Fault at {:#x}", addr);
                }
            }
            Instruction::StrImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                let val = self.read_reg(rt);
                if bus.write_u32(addr as u64, val).is_err() {
                    tracing::error!("Bus Write Fault at {:#x}", addr);
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
                let pc_val = (self.regs[15] & !3) + 4;
                let addr = pc_val.wrapping_add(imm as u32);
                if let Ok(val) = bus.read_u32(addr as u64) {
                    self.write_reg(rt, val);
                } else {
                    tracing::error!("Bus Read Fault (LdrLit) at {:#x}", addr);
                }
            }

            Instruction::LdrSp { rt, imm } => {
                let addr = self.regs[13].wrapping_add(imm as u32);
                if let Ok(val) = bus.read_u32(addr as u64) {
                    self.write_reg(rt, val);
                } else {
                    tracing::error!("Bus Read Fault (LdrSp) at {:#x}", addr);
                }
            }
            Instruction::StrSp { rt, imm } => {
                let addr = self.regs[13].wrapping_add(imm as u32);
                let val = self.read_reg(rt);
                if bus.write_u32(addr as u64, val).is_err() {
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
                if let Ok(val) = bus.read_u8(addr as u64) {
                    self.write_reg(rt, val as u32);
                } else {
                    tracing::error!("Bus Read Fault (LDRB) at {:#x}", addr);
                }
            }
            Instruction::StrbImm { rt, rn, imm } => {
                let base = self.read_reg(rn);
                let addr = base.wrapping_add(imm as u32);
                let val = (self.read_reg(rt) & 0xFF) as u8;
                if bus.write_u8(addr as u64, val).is_err() {
                    tracing::error!("Bus Write Fault (STRB) at {:#x}", addr);
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
                cycles = 2;
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

