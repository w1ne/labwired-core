// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::decoder::xtensa::{decode_xtensa, Instruction};
use crate::{Bus, Cpu, SimResult, SimulationError, SimulationObserver};
use std::sync::Arc;

/// Xtensa LX7 CPU model (ESP32-S3 dual-core).
///
/// Implements the call0 ABI (no windowed registers) which is the default
/// for ESP-IDF and most modern Xtensa toolchains. 16 general-purpose registers
/// a0..a15, plus special registers for shifts, loops, and exceptions.
#[derive(Debug)]
pub struct Xtensa {
    /// General-purpose registers a0..a15
    pub a: [u32; 16],
    pub pc: u32,

    // Special registers
    /// Shift Amount Register
    pub sar: u32,
    /// Processor State register (interrupt level, etc.)
    pub ps: u32,
    /// Loop begin address
    pub lbeg: u32,
    /// Loop end address
    pub lend: u32,
    /// Loop count
    pub lcount: u32,
    /// Exception PC (level 1)
    pub epc1: u32,
    /// Exception cause
    pub exccause: u32,
    /// Exception save register (level 1)
    pub excsave1: u32,
    /// Exception virtual address
    pub excvaddr: u32,
    /// Vector base address
    pub vecbase: u32,

    /// Pending exception/interrupt number (0 = none)
    pending_exception: u32,
}

impl Default for Xtensa {
    fn default() -> Self {
        Self {
            a: [0; 16],
            pc: 0,
            sar: 0,
            ps: 0x0000_0020, // Default: UM=1 (user mode), INTLEVEL=0
            lbeg: 0,
            lend: 0,
            lcount: 0,
            epc1: 0,
            exccause: 0,
            excsave1: 0,
            excvaddr: 0,
            vecbase: 0x4037_8000, // ESP32-S3 default vector base
            pending_exception: 0,
        }
    }
}

// Xtensa special register numbers
const SR_LBEG: u8 = 0;
const SR_LEND: u8 = 1;
const SR_LCOUNT: u8 = 2;
const SR_SAR: u8 = 3;
const SR_PS: u8 = 230;
const SR_VECBASE: u8 = 231;
const SR_EPC1: u8 = 177;
const SR_EXCCAUSE: u8 = 232;
const SR_EXCSAVE1: u8 = 209;
const SR_EXCVADDR: u8 = 238;

impl Xtensa {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_reg(&self, n: u8) -> u32 {
        self.a[(n & 0xF) as usize]
    }

    fn write_reg(&mut self, n: u8, val: u32) {
        self.a[(n & 0xF) as usize] = val;
    }

    fn read_sr(&self, sr: u8) -> u32 {
        match sr {
            SR_LBEG => self.lbeg,
            SR_LEND => self.lend,
            SR_LCOUNT => self.lcount,
            SR_SAR => self.sar,
            SR_PS => self.ps,
            SR_VECBASE => self.vecbase,
            SR_EPC1 => self.epc1,
            SR_EXCCAUSE => self.exccause,
            SR_EXCSAVE1 => self.excsave1,
            SR_EXCVADDR => self.excvaddr,
            _ => {
                tracing::debug!("Read from unhandled SR {}", sr);
                0
            }
        }
    }

    fn write_sr(&mut self, sr: u8, val: u32) {
        match sr {
            SR_LBEG => self.lbeg = val,
            SR_LEND => self.lend = val,
            SR_LCOUNT => self.lcount = val,
            SR_SAR => self.sar = val & 0x3F,
            SR_PS => self.ps = val,
            SR_VECBASE => self.vecbase = val,
            SR_EPC1 => self.epc1 = val,
            SR_EXCCAUSE => self.exccause = val,
            SR_EXCSAVE1 => self.excsave1 = val,
            SR_EXCVADDR => self.excvaddr = val,
            _ => {
                tracing::debug!("Write to unhandled SR {} = {:#x}", sr, val);
            }
        }
    }

    fn handle_exception(&mut self, cause: u32) {
        self.epc1 = self.pc;
        self.exccause = cause;
        // Jump to exception vector
        self.pc = self.vecbase;
    }
}

impl Cpu for Xtensa {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.pc = 0;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &crate::SimulationConfig,
    ) -> SimResult<()> {
        // Check pending interrupts
        if self.pending_exception != 0 {
            let exc = self.pending_exception;
            self.pending_exception = 0;
            self.handle_exception(exc);
            return Ok(());
        }

        // Fetch 3 bytes (max instruction size)
        let b0 = bus.read_u8(self.pc as u64)?;
        let b1 = bus.read_u8(self.pc as u64 + 1)?;
        let b2 = bus.read_u8(self.pc as u64 + 2)?;
        let bytes = [b0, b1, b2];

        let opcode_u32 = (b0 as u32) | ((b1 as u32) << 8) | ((b2 as u32) << 16);
        for observer in observers {
            observer.on_step_start(self.pc, opcode_u32);
        }

        let (instruction, inst_len) = decode_xtensa(&bytes);
        tracing::debug!(
            "PC={:#x}, Op={:#08x}, Instr={:?}, Len={}",
            self.pc,
            opcode_u32,
            instruction,
            inst_len
        );

        let mut next_pc = self.pc.wrapping_add(inst_len);

        match instruction {
            // ALU
            Instruction::Add { rd, rs, rt } => {
                let res = self.read_reg(rs).wrapping_add(self.read_reg(rt));
                self.write_reg(rd, res);
            }
            Instruction::Addx2 { rd, rs, rt } => {
                let res = (self.read_reg(rs) << 1).wrapping_add(self.read_reg(rt));
                self.write_reg(rd, res);
            }
            Instruction::Addx4 { rd, rs, rt } => {
                let res = (self.read_reg(rs) << 2).wrapping_add(self.read_reg(rt));
                self.write_reg(rd, res);
            }
            Instruction::Addx8 { rd, rs, rt } => {
                let res = (self.read_reg(rs) << 3).wrapping_add(self.read_reg(rt));
                self.write_reg(rd, res);
            }
            Instruction::Sub { rd, rs, rt } => {
                let res = self.read_reg(rs).wrapping_sub(self.read_reg(rt));
                self.write_reg(rd, res);
            }
            Instruction::Subx2 { rd, rs, rt } => {
                let res = (self.read_reg(rs) << 1).wrapping_sub(self.read_reg(rt));
                self.write_reg(rd, res);
            }
            Instruction::Subx4 { rd, rs, rt } => {
                let res = (self.read_reg(rs) << 2).wrapping_sub(self.read_reg(rt));
                self.write_reg(rd, res);
            }
            Instruction::Subx8 { rd, rs, rt } => {
                let res = (self.read_reg(rs) << 3).wrapping_sub(self.read_reg(rt));
                self.write_reg(rd, res);
            }
            Instruction::And { rd, rs, rt } => {
                self.write_reg(rd, self.read_reg(rs) & self.read_reg(rt));
            }
            Instruction::Or { rd, rs, rt } => {
                self.write_reg(rd, self.read_reg(rs) | self.read_reg(rt));
            }
            Instruction::Xor { rd, rs, rt } => {
                self.write_reg(rd, self.read_reg(rs) ^ self.read_reg(rt));
            }
            Instruction::Neg { rd, rt } => {
                self.write_reg(rd, 0u32.wrapping_sub(self.read_reg(rt)));
            }
            Instruction::Abs { rd, rt } => {
                let val = self.read_reg(rt) as i32;
                self.write_reg(rd, val.wrapping_abs() as u32);
            }

            // Shifts
            Instruction::Sll { rd, rs } => {
                let sa = 32 - (self.sar & 0x1F);
                self.write_reg(rd, self.read_reg(rs) << sa);
            }
            Instruction::Srl { rd, rt } => {
                let sa = self.sar & 0x1F;
                self.write_reg(rd, self.read_reg(rt) >> sa);
            }
            Instruction::Sra { rd, rt } => {
                let sa = self.sar & 0x1F;
                self.write_reg(rd, ((self.read_reg(rt) as i32) >> sa) as u32);
            }
            Instruction::Slli { rd, rs, sa } => {
                self.write_reg(rd, self.read_reg(rs) << (sa & 0x1F));
            }
            Instruction::Srli { rd, rt, sa } => {
                self.write_reg(rd, self.read_reg(rt) >> (sa & 0x1F));
            }
            Instruction::Srai { rd, rt, sa } => {
                self.write_reg(rd, ((self.read_reg(rt) as i32) >> (sa & 0x1F)) as u32);
            }
            Instruction::Ssl { rs } => {
                self.sar = 32 - (self.read_reg(rs) & 0x1F);
            }
            Instruction::Ssr { rs } => {
                self.sar = self.read_reg(rs) & 0x1F;
            }
            Instruction::Ssa8l { rs } => {
                self.sar = (self.read_reg(rs) & 0x3) << 3;
            }
            Instruction::Ssai { sa } => {
                self.sar = (sa & 0x1F) as u32;
            }
            Instruction::Src { rd, rs, rt } => {
                let sa = self.sar & 0x1F;
                let combined = ((self.read_reg(rs) as u64) << 32) | (self.read_reg(rt) as u64);
                self.write_reg(rd, (combined >> sa) as u32);
            }
            Instruction::Extui { rd, rt, shift, mask_bits } => {
                let val = self.read_reg(rt) >> (shift & 0x1F);
                let mask = (1u32 << mask_bits) - 1;
                self.write_reg(rd, val & mask);
            }

            // Multiply
            Instruction::Mull { rd, rs, rt } => {
                self.write_reg(rd, self.read_reg(rs).wrapping_mul(self.read_reg(rt)));
            }
            Instruction::Muluh { rd, rs, rt } => {
                let result = (self.read_reg(rs) as u64).wrapping_mul(self.read_reg(rt) as u64);
                self.write_reg(rd, (result >> 32) as u32);
            }
            Instruction::Mulsh { rd, rs, rt } => {
                let result = (self.read_reg(rs) as i32 as i64)
                    .wrapping_mul(self.read_reg(rt) as i32 as i64);
                self.write_reg(rd, (result >> 32) as u32);
            }

            // Loads
            Instruction::L8ui { rt, rs, imm } => {
                let addr = self.read_reg(rs).wrapping_add(imm);
                let val = bus.read_u8(addr as u64)?;
                self.write_reg(rt, val as u32);
            }
            Instruction::L16ui { rt, rs, imm } => {
                let addr = self.read_reg(rs).wrapping_add(imm);
                let val = bus.read_u16(addr as u64)?;
                self.write_reg(rt, val as u32);
            }
            Instruction::L16si { rt, rs, imm } => {
                let addr = self.read_reg(rs).wrapping_add(imm);
                let val = bus.read_u16(addr as u64)? as i16;
                self.write_reg(rt, val as i32 as u32);
            }
            Instruction::L32i { rt, rs, imm } => {
                let addr = self.read_reg(rs).wrapping_add(imm);
                let val = bus.read_u32(addr as u64)?;
                self.write_reg(rt, val);
            }

            // Stores
            Instruction::S8i { rt, rs, imm } => {
                let addr = self.read_reg(rs).wrapping_add(imm);
                bus.write_u8(addr as u64, self.read_reg(rt) as u8)?;
            }
            Instruction::S16i { rt, rs, imm } => {
                let addr = self.read_reg(rs).wrapping_add(imm);
                bus.write_u16(addr as u64, self.read_reg(rt) as u16)?;
            }
            Instruction::S32i { rt, rs, imm } => {
                let addr = self.read_reg(rs).wrapping_add(imm);
                bus.write_u32(addr as u64, self.read_reg(rt))?;
            }

            // Immediates
            Instruction::Movi { rt, imm } => {
                self.write_reg(rt, imm as u32);
            }
            Instruction::Addi { rt, rs, imm } => {
                let res = self.read_reg(rs).wrapping_add(imm as u32);
                self.write_reg(rt, res);
            }
            Instruction::Addmi { rt, rs, imm } => {
                let res = self.read_reg(rs).wrapping_add(imm as u32);
                self.write_reg(rt, res);
            }

            // Branches
            Instruction::J { offset } => {
                next_pc = self.pc.wrapping_add(offset as u32);
            }
            Instruction::Jx { rs } => {
                next_pc = self.read_reg(rs);
            }
            Instruction::Call0 { offset } => {
                self.write_reg(0, next_pc); // a0 = return address
                next_pc = (self.pc & !3).wrapping_add(offset as u32);
            }
            Instruction::Callx0 { rs } => {
                let target = self.read_reg(rs);
                self.write_reg(0, next_pc); // a0 = return address
                next_pc = target;
            }
            Instruction::Ret | Instruction::NarrowRet => {
                next_pc = self.read_reg(0); // a0
            }
            Instruction::RetW | Instruction::NarrowRetW => {
                // In call0 ABI, RETW behaves like RET
                next_pc = self.read_reg(0);
            }

            // Conditional branches
            Instruction::Beqz { rs, offset } => {
                if self.read_reg(rs) == 0 {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bnez { rs, offset } => {
                if self.read_reg(rs) != 0 {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bltz { rs, offset } => {
                if (self.read_reg(rs) as i32) < 0 {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bgez { rs, offset } => {
                if (self.read_reg(rs) as i32) >= 0 {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Beq { rs, rt, offset } => {
                if self.read_reg(rs) == self.read_reg(rt) {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bne { rs, rt, offset } => {
                if self.read_reg(rs) != self.read_reg(rt) {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Blt { rs, rt, offset } => {
                if (self.read_reg(rs) as i32) < (self.read_reg(rt) as i32) {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bge { rs, rt, offset } => {
                if (self.read_reg(rs) as i32) >= (self.read_reg(rt) as i32) {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bltu { rs, rt, offset } => {
                if self.read_reg(rs) < self.read_reg(rt) {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bgeu { rs, rt, offset } => {
                if self.read_reg(rs) >= self.read_reg(rt) {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Beqi { rs, imm, offset } => {
                if self.read_reg(rs) as i32 == imm {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bnei { rs, imm, offset } => {
                if self.read_reg(rs) as i32 != imm {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Blti { rs, imm, offset } => {
                if (self.read_reg(rs) as i32) < imm {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bgei { rs, imm, offset } => {
                if (self.read_reg(rs) as i32) >= imm {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bltui { rs, imm, offset } => {
                if self.read_reg(rs) < imm {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bgeui { rs, imm, offset } => {
                if self.read_reg(rs) >= imm {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bbci { rs, bit, offset } => {
                if (self.read_reg(rs) >> bit) & 1 == 0 {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Bbsi { rs, bit, offset } => {
                if (self.read_reg(rs) >> bit) & 1 != 0 {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }

            // Conditional moves
            Instruction::Moveqz { rd, rs, rt } => {
                if self.read_reg(rt) == 0 {
                    self.write_reg(rd, self.read_reg(rs));
                }
            }
            Instruction::Movnez { rd, rs, rt } => {
                if self.read_reg(rt) != 0 {
                    self.write_reg(rd, self.read_reg(rs));
                }
            }
            Instruction::Movltz { rd, rs, rt } => {
                if (self.read_reg(rt) as i32) < 0 {
                    self.write_reg(rd, self.read_reg(rs));
                }
            }
            Instruction::Movgez { rd, rs, rt } => {
                if (self.read_reg(rt) as i32) >= 0 {
                    self.write_reg(rd, self.read_reg(rs));
                }
            }

            // Special register access
            Instruction::Rsr { rt, sr } => {
                let val = self.read_sr(sr);
                self.write_reg(rt, val);
            }
            Instruction::Wsr { rt, sr } => {
                let val = self.read_reg(rt);
                self.write_sr(sr, val);
            }
            Instruction::Xsr { rt, sr } => {
                let old_sr = self.read_sr(sr);
                let old_reg = self.read_reg(rt);
                self.write_sr(sr, old_reg);
                self.write_reg(rt, old_sr);
            }

            // Misc
            Instruction::Nop | Instruction::NarrowNop => {}
            Instruction::Memw | Instruction::Isync | Instruction::Dsync
            | Instruction::Esync | Instruction::Rsync | Instruction::Extw => {
                // Memory barriers / sync - no-op in single-core sim
            }
            Instruction::Ill => {
                return Err(SimulationError::DecodeError(self.pc as u64));
            }
            Instruction::Break { .. } => {
                tracing::warn!("BREAK at {:#x}", self.pc);
                return Err(SimulationError::Halt);
            }
            Instruction::Syscall => {
                tracing::warn!("SYSCALL at {:#x}", self.pc);
                self.handle_exception(1); // SYSCALL cause
                return Ok(());
            }

            // Loop instructions
            Instruction::Loop { rs, offset } => {
                self.lcount = self.read_reg(rs);
                self.lbeg = next_pc;
                self.lend = self.pc.wrapping_add(offset as u32);
            }
            Instruction::Loopnez { rs, offset } => {
                let count = self.read_reg(rs);
                if count != 0 {
                    self.lcount = count;
                    self.lbeg = next_pc;
                    self.lend = self.pc.wrapping_add(offset as u32);
                } else {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::Loopgtz { rs, offset } => {
                let count = self.read_reg(rs) as i32;
                if count > 0 {
                    self.lcount = count as u32;
                    self.lbeg = next_pc;
                    self.lend = self.pc.wrapping_add(offset as u32);
                } else {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }

            // Narrow instructions
            Instruction::NarrowL32iN { rt, rs, imm } => {
                let addr = self.read_reg(rs).wrapping_add(imm);
                let val = bus.read_u32(addr as u64)?;
                self.write_reg(rt, val);
            }
            Instruction::NarrowS32iN { rt, rs, imm } => {
                let addr = self.read_reg(rs).wrapping_add(imm);
                bus.write_u32(addr as u64, self.read_reg(rt))?;
            }
            Instruction::NarrowAdd { rd, rs, rt } => {
                let res = self.read_reg(rs).wrapping_add(self.read_reg(rt));
                self.write_reg(rd, res);
            }
            Instruction::NarrowAddi { rd, rs, imm } => {
                let res = self.read_reg(rs).wrapping_add(imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::NarrowMovi { rd, imm } => {
                self.write_reg(rd, imm as u32);
            }
            Instruction::NarrowBeqz { rs, offset } => {
                if self.read_reg(rs) == 0 {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::NarrowBnez { rs, offset } => {
                if self.read_reg(rs) != 0 {
                    next_pc = self.pc.wrapping_add(offset as u32);
                }
            }
            Instruction::NarrowMov { rd, rs } => {
                self.write_reg(rd, self.read_reg(rs));
            }

            Instruction::Unknown(op) => {
                tracing::warn!("Unknown Xtensa instruction at {:#x}: {:#08x}", self.pc, op);
                return Err(SimulationError::DecodeError(self.pc as u64));
            }
        }

        // Hardware loop support: if PC reaches lend, decrement lcount and jump to lbeg
        if self.lcount > 0 && next_pc == self.lend {
            self.lcount -= 1;
            if self.lcount > 0 {
                next_pc = self.lbeg;
            }
        }

        self.pc = next_pc;
        Ok(())
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, val: u32) {
        self.a[1] = val; // a1 is the stack pointer in Xtensa
    }

    fn set_exception_pending(&mut self, exception_num: u32) {
        self.pending_exception = exception_num;
    }

    fn get_register(&self, id: u8) -> u32 {
        if id < 16 {
            self.a[id as usize]
        } else {
            0
        }
    }

    fn set_register(&mut self, id: u8, val: u32) {
        if id < 16 {
            self.a[id as usize] = val;
        }
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::Xtensa(crate::snapshot::XtensaCpuSnapshot {
            registers: self.a.to_vec(),
            pc: self.pc,
            ps: self.ps,
            sar: self.sar,
            lbeg: self.lbeg,
            lend: self.lend,
            lcount: self.lcount,
            vecbase: self.vecbase,
            epc1: self.epc1,
            exccause: self.exccause,
            excsave1: self.excsave1,
            excvaddr: self.excvaddr,
        })
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::Xtensa(snap) = snapshot {
            for (i, &val) in snap.registers.iter().enumerate().take(16) {
                self.a[i] = val;
            }
            self.pc = snap.pc;
            self.ps = snap.ps;
            self.sar = snap.sar;
            self.lbeg = snap.lbeg;
            self.lend = snap.lend;
            self.lcount = snap.lcount;
            self.vecbase = snap.vecbase;
            self.epc1 = snap.epc1;
            self.exccause = snap.exccause;
            self.excsave1 = snap.excsave1;
            self.excvaddr = snap.excvaddr;
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        (0..16).map(|i| format!("a{}", i)).collect()
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        if let Some(rest) = name.strip_prefix('a') {
            rest.parse::<u8>().ok().filter(|&n| n < 16)
        } else {
            None
        }
    }
}
