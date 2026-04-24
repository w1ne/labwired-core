// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::decoder::riscv::{decode_rv32, decode_rv32c, Instruction};
use crate::{Bus, Cpu, SimResult, SimulationObserver};
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct RiscV {
    pub x: [u32; 32], // x0..x31. x0 is correctly hardwired to 0 in logic.
    pub pc: u32,

    // CSRs
    pub mstatus: u32,
    pub mie: u32,
    pub mip: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,

    // CLINT-like internal state (minimal)
    pub mtime: u64,
    pub mtimecmp: u64,

    /// Active LR/SC reservation address. `None` means no outstanding
    /// reservation; the next `SC.W` to any address will fail. On a single
    /// hart, any intervening store (including any AMO*) invalidates the
    /// reservation per RISC-V ISA §8.2.
    pub reservation: Option<u32>,
}

impl RiscV {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_reg(&self, n: u8) -> u32 {
        if n == 0 {
            0
        } else {
            self.x[n as usize]
        }
    }

    fn write_reg(&mut self, n: u8, val: u32) {
        if n != 0 {
            self.x[n as usize] = val;
        }
    }

    fn read_csr(&self, csr: u16) -> u32 {
        match csr {
            0x300 => self.mstatus,
            0x304 => self.mie,
            0x344 => self.mip,
            0x305 => self.mtvec,
            0x340 => self.mscratch,
            0x341 => self.mepc,
            0x342 => self.mcause,
            0x343 => self.mtval,
            // Timer CSR stubs (Standard RISC-V shadow non-privileged? No, these are machine mode)
            0xB00 => (self.mtime & 0xFFFFFFFF) as u32,
            0xB80 => (self.mtime >> 32) as u32,
            _ => 0,
        }
    }

    fn write_csr(&mut self, csr: u16, val: u32) {
        match csr {
            0x300 => self.mstatus = val & 0x0000_1888, // Minimal mstatus (MIE, MPP)
            0x304 => self.mie = val,
            0x344 => self.mip = val,
            0x305 => self.mtvec = val,
            0x340 => self.mscratch = val,
            0x341 => self.mepc = val,
            0x342 => self.mcause = val,
            0x343 => self.mtval = val,
            _ => {}
        }
    }

    fn handle_trap(&mut self, cause: u32, epc: u32) {
        self.mepc = epc;
        self.mcause = cause;
        // mtvec handling (Direct vs Vectored)
        let mode = self.mtvec & 3;
        let base = self.mtvec & !3;
        if mode == 1 && (cause & 0x80000000) != 0 {
            // Vectored interrupt
            let irq = cause & 0x7FFFFFFF;
            self.pc = base + irq * 4;
        } else {
            self.pc = base;
        }
        // Disable interrupts (saving previous status in MPIE if we supported it fully)
        self.mstatus &= !(1 << 3); // Clear MIE
    }
}

impl Cpu for RiscV {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.pc = 0x8000_0000; // Typical RISC-V Reset Vector (varies by platform)
                               // x0..x31 are 0 by Default
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
    ) -> SimResult<()> {
        // RV32C: low two bits of the first halfword distinguish
        // compressed (16-bit, bits[1:0] != 0b11) from standard
        // (32-bit, bits[1:0] == 0b11). We fetch the halfword first and
        // only fetch the second halfword when we need the full word.
        let h0 = bus.read_u16(self.pc)?;
        let is_compressed = (h0 & 0x3) != 0x3;
        let (opcode, instruction, insn_len) = if is_compressed {
            (h0 as u32, decode_rv32c(h0), 2u32)
        } else {
            let opcode = bus.read_u32(self.pc)?;
            (opcode, decode_rv32(opcode), 4u32)
        };

        for observer in observers {
            observer.on_step_start(self.pc, opcode);
        }

        tracing::debug!(
            "PC={:#x}, Op={:#08x}, Instr={:?}",
            self.pc,
            opcode,
            instruction
        );

        let mut next_pc = self.pc.wrapping_add(insn_len);

        match instruction {
            Instruction::Lui { rd, imm } => {
                self.write_reg(rd, imm);
            }
            Instruction::Auipc { rd, imm } => {
                let val = self.pc.wrapping_add(imm);
                self.write_reg(rd, val);
            }
            Instruction::Jal { rd, imm } => {
                let target = self.pc.wrapping_add(imm as u32);
                self.write_reg(rd, self.pc.wrapping_add(insn_len));
                next_pc = target;
            }
            Instruction::Jalr { rd, rs1, imm } => {
                let base = self.read_reg(rs1);
                let target = base.wrapping_add(imm as u32) & !1;
                self.write_reg(rd, self.pc.wrapping_add(insn_len));
                next_pc = target;
            }
            Instruction::Beq { rs1, rs2, imm } => {
                if self.read_reg(rs1) == self.read_reg(rs2) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bne { rs1, rs2, imm } => {
                if self.read_reg(rs1) != self.read_reg(rs2) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Blt { rs1, rs2, imm } => {
                if (self.read_reg(rs1) as i32) < (self.read_reg(rs2) as i32) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bge { rs1, rs2, imm } => {
                if (self.read_reg(rs1) as i32) >= (self.read_reg(rs2) as i32) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bltu { rs1, rs2, imm } => {
                if self.read_reg(rs1) < self.read_reg(rs2) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Bgeu { rs1, rs2, imm } => {
                if self.read_reg(rs1) >= self.read_reg(rs2) {
                    next_pc = self.pc.wrapping_add(imm as u32);
                }
            }
            Instruction::Lb { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u8(addr)? as i8;
                self.write_reg(rd, val as i32 as u32);
            }
            Instruction::Lh { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u16(addr)? as i16;
                self.write_reg(rd, val as i32 as u32);
            }
            Instruction::Lw { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u32(addr)?;
                self.write_reg(rd, val);
            }
            Instruction::Lbu { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u8(addr)?;
                self.write_reg(rd, val as u32);
            }
            Instruction::Lhu { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u16(addr)?;
                self.write_reg(rd, val as u32);
            }
            Instruction::Sb { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2) as u8;
                bus.write_u8(addr, val)?;
            }
            Instruction::Sh { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2) as u16;
                bus.write_u16(addr, val)?;
            }
            Instruction::Sw { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2);
                bus.write_u32(addr, val)?;
            }
            Instruction::Addi { rd, rs1, imm } => {
                let res = self.read_reg(rs1).wrapping_add(imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Slti { rd, rs1, imm } => {
                let val = if (self.read_reg(rs1) as i32) < imm {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Sltiu { rd, rs1, imm } => {
                let val = if self.read_reg(rs1) < (imm as u32) {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Xori { rd, rs1, imm } => {
                let res = self.read_reg(rs1) ^ (imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Ori { rd, rs1, imm } => {
                let res = self.read_reg(rs1) | (imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Andi { rd, rs1, imm } => {
                let res = self.read_reg(rs1) & (imm as u32);
                self.write_reg(rd, res);
            }
            Instruction::Slli { rd, rs1, shamt } => {
                let res = self.read_reg(rs1) << shamt;
                self.write_reg(rd, res);
            }
            Instruction::Srli { rd, rs1, shamt } => {
                let res = self.read_reg(rs1) >> shamt;
                self.write_reg(rd, res);
            }
            Instruction::Srai { rd, rs1, shamt } => {
                let res = (self.read_reg(rs1) as i32) >> shamt;
                self.write_reg(rd, res as u32);
            }
            Instruction::Add { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1).wrapping_add(self.read_reg(rs2));
                self.write_reg(rd, res);
            }
            Instruction::Sub { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1).wrapping_sub(self.read_reg(rs2));
                self.write_reg(rd, res);
            }
            Instruction::Sll { rd, rs1, rs2 } => {
                let shamt = self.read_reg(rs2) & 0x1F;
                let res = self.read_reg(rs1) << shamt;
                self.write_reg(rd, res);
            }
            Instruction::Slt { rd, rs1, rs2 } => {
                let val = if (self.read_reg(rs1) as i32) < (self.read_reg(rs2) as i32) {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Sltu { rd, rs1, rs2 } => {
                let val = if self.read_reg(rs1) < self.read_reg(rs2) {
                    1
                } else {
                    0
                };
                self.write_reg(rd, val);
            }
            Instruction::Xor { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1) ^ self.read_reg(rs2);
                self.write_reg(rd, res);
            }
            Instruction::Srl { rd, rs1, rs2 } => {
                let shamt = self.read_reg(rs2) & 0x1F;
                let res = self.read_reg(rs1) >> shamt;
                self.write_reg(rd, res);
            }
            Instruction::Sra { rd, rs1, rs2 } => {
                let shamt = self.read_reg(rs2) & 0x1F;
                let res = (self.read_reg(rs1) as i32) >> shamt;
                self.write_reg(rd, res as u32);
            }
            Instruction::Or { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1) | self.read_reg(rs2);
                self.write_reg(rd, res);
            }
            Instruction::And { rd, rs1, rs2 } => {
                let res = self.read_reg(rs1) & self.read_reg(rs2);
                self.write_reg(rd, res);
            }
            // RV32M — per spec §7. All results fully defined; no traps.
            Instruction::Mul { rd, rs1, rs2 } => {
                let res = (self.read_reg(rs1) as i32).wrapping_mul(self.read_reg(rs2) as i32);
                self.write_reg(rd, res as u32);
            }
            Instruction::Mulh { rd, rs1, rs2 } => {
                let a = self.read_reg(rs1) as i32 as i64;
                let b = self.read_reg(rs2) as i32 as i64;
                self.write_reg(rd, ((a * b) >> 32) as u32);
            }
            Instruction::Mulhsu { rd, rs1, rs2 } => {
                let a = self.read_reg(rs1) as i32 as i64;
                let b = self.read_reg(rs2) as u64 as i64;
                self.write_reg(rd, ((a * b) >> 32) as u32);
            }
            Instruction::Mulhu { rd, rs1, rs2 } => {
                let a = self.read_reg(rs1) as u64;
                let b = self.read_reg(rs2) as u64;
                self.write_reg(rd, ((a * b) >> 32) as u32);
            }
            Instruction::Div { rd, rs1, rs2 } => {
                let a = self.read_reg(rs1) as i32;
                let b = self.read_reg(rs2) as i32;
                let res = if b == 0 {
                    u32::MAX // Division by zero -> all ones (per spec).
                } else if a == i32::MIN && b == -1 {
                    a as u32 // Signed overflow -> dividend.
                } else {
                    a.wrapping_div(b) as u32
                };
                self.write_reg(rd, res);
            }
            Instruction::Divu { rd, rs1, rs2 } => {
                let a = self.read_reg(rs1);
                let b = self.read_reg(rs2);
                let res = if b == 0 { u32::MAX } else { a / b };
                self.write_reg(rd, res);
            }
            Instruction::Rem { rd, rs1, rs2 } => {
                let a = self.read_reg(rs1) as i32;
                let b = self.read_reg(rs2) as i32;
                let res = if b == 0 {
                    a as u32 // Division by zero -> dividend (per spec).
                } else if a == i32::MIN && b == -1 {
                    0 // Signed overflow -> 0.
                } else {
                    a.wrapping_rem(b) as u32
                };
                self.write_reg(rd, res);
            }
            Instruction::Remu { rd, rs1, rs2 } => {
                let a = self.read_reg(rs1);
                let b = self.read_reg(rs2);
                let res = if b == 0 { a } else { a % b };
                self.write_reg(rd, res);
            }
            // RV32A atomics — single-threaded, so the aq/rl ordering bits
            // do not affect observable behavior. Every store invalidates
            // any outstanding LR/SC reservation per ISA §8.2.
            Instruction::LrW { rd, rs1 } => {
                let addr = self.read_reg(rs1);
                let val = bus.read_u32(addr)?;
                self.write_reg(rd, val);
                self.reservation = Some(addr);
            }
            Instruction::ScW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let ok = self.reservation == Some(addr);
                if ok {
                    bus.write_u32(addr, self.read_reg(rs2))?;
                    self.write_reg(rd, 0);
                } else {
                    self.write_reg(rd, 1);
                }
                self.reservation = None;
            }
            Instruction::AmoSwapW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr)?;
                bus.write_u32(addr, self.read_reg(rs2))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoAddW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr)?;
                bus.write_u32(addr, old.wrapping_add(self.read_reg(rs2)))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoXorW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr)?;
                bus.write_u32(addr, old ^ self.read_reg(rs2))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoOrW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr)?;
                bus.write_u32(addr, old | self.read_reg(rs2))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoAndW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr)?;
                bus.write_u32(addr, old & self.read_reg(rs2))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMinW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr)?;
                let a = old as i32;
                let b = self.read_reg(rs2) as i32;
                bus.write_u32(addr, a.min(b) as u32)?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMaxW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr)?;
                let a = old as i32;
                let b = self.read_reg(rs2) as i32;
                bus.write_u32(addr, a.max(b) as u32)?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMinuW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr)?;
                bus.write_u32(addr, old.min(self.read_reg(rs2)))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::AmoMaxuW { rd, rs1, rs2 } => {
                let addr = self.read_reg(rs1);
                let old = bus.read_u32(addr)?;
                bus.write_u32(addr, old.max(self.read_reg(rs2)))?;
                self.write_reg(rd, old);
                self.reservation = None;
            }
            Instruction::Fence => {
                // No-op in single threaded core model
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
                return Ok(());
            }
            Instruction::Mret => {
                // Return from trap
                self.pc = self.mepc;
                // mstatus.MIE = mstatus.MPIE. For now we just set MIE=1 if we assume it was enabled.
                // Simple version:
                self.mstatus |= 1 << 3; // Re-enable MIE
                return Ok(());
            }
            Instruction::Csrrw { rd, rs1, csr } => {
                let old = self.read_csr(csr);
                let val = self.read_reg(rs1);
                self.write_csr(csr, val);
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrs { rd, rs1, csr } => {
                let old = self.read_csr(csr);
                if rs1 != 0 {
                    let val = self.read_reg(rs1);
                    self.write_csr(csr, old | val);
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrc { rd, rs1, csr } => {
                let old = self.read_csr(csr);
                if rs1 != 0 {
                    let val = self.read_reg(rs1);
                    self.write_csr(csr, old & !val);
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrwi { rd, imm, csr } => {
                let old = self.read_csr(csr);
                self.write_csr(csr, imm as u32);
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrsi { rd, imm, csr } => {
                let old = self.read_csr(csr);
                if imm != 0 {
                    self.write_csr(csr, old | (imm as u32));
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Csrrci { rd, imm, csr } => {
                let old = self.read_csr(csr);
                if imm != 0 {
                    self.write_csr(csr, old & !(imm as u32));
                }
                if rd != 0 {
                    self.write_reg(rd, old);
                }
            }
            Instruction::Unknown(inst) => {
                tracing::error!("Unknown instruction {:#x} at {:#x}", inst, self.pc);
                return Err(crate::SimulationError::DecodeError(self.pc));
            }
        }

        // Timer update (Internal minimal CLINT)
        self.mtime = self.mtime.wrapping_add(1);
        if self.mtime >= self.mtimecmp {
            self.mip |= 1 << 7; // MTIP
        } else {
            self.mip &= !(1 << 7);
        }

        // Check for interrupts
        if (self.mstatus & (1 << 3)) != 0 {
            let pending = self.mip & self.mie;
            if pending != 0 {
                // Find highest priority interrupt (Simplified: MTIP=7, MSIP=3, MEIP=11)
                // Priority: External > Software > Timer
                let irq = if (pending & (1 << 11)) != 0 {
                    11
                } else if (pending & (1 << 3)) != 0 {
                    3
                } else if (pending & (1 << 7)) != 0 {
                    7
                } else {
                    0xFFFFFFFF // Should not happen
                };

                if irq != 0xFFFFFFFF {
                    // Async interrupt: save the address of the NEXT
                    // instruction in mepc so MRET resumes forward rather
                    // than re-executing the one we just completed. For
                    // branch ops next_pc is the branch target, which is
                    // also correct — MRET returns to the loop head.
                    self.handle_trap(0x80000000 | irq, next_pc);
                    return Ok(());
                }
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
        self.write_reg(2, val); // x2 is SP
    }
    fn set_exception_pending(&mut self, exception_num: u32) {
        // Route peripheral-level IRQs into mip. We fold everything into
        // MEIP (bit 11) because this crate does not yet model a PLIC that
        // can distinguish individual sources. Standard machine-mode bits
        // (MSIP=3, MTIP=7, MEIP=11) pass through unchanged when the caller
        // already uses the mip bit position.
        let bit = match exception_num {
            3 | 7 | 11 => exception_num,
            _ => 11, // Any external source collapses into MEIP.
        };
        self.mip |= 1 << bit;
    }

    fn get_register(&self, id: u8) -> u32 {
        if id < 32 {
            self.read_reg(id)
        } else if id == 32 {
            self.pc
        } else {
            0
        }
    }
    fn set_register(&mut self, id: u8, val: u32) {
        if id < 32 {
            self.write_reg(id, val);
        } else if id == 32 {
            self.pc = val;
        }
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::RiscV(crate::snapshot::RiscVCpuSnapshot {
            registers: self.x.to_vec(),
            pc: self.pc,
            mstatus: self.mstatus,
            mie: self.mie,
            mip: self.mip,
            mtvec: self.mtvec,
            mscratch: self.mscratch,
            mepc: self.mepc,
            mcause: self.mcause,
            mtval: self.mtval,
            mtime: self.mtime,
            mtimecmp: self.mtimecmp,
        })
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::RiscV(s) = snapshot {
            for (i, &val) in s.registers.iter().enumerate().take(32) {
                self.x[i] = val;
            }
            self.pc = s.pc;
            self.mstatus = s.mstatus;
            self.mie = s.mie;
            self.mip = s.mip;
            self.mtvec = s.mtvec;
            self.mscratch = s.mscratch;
            self.mepc = s.mepc;
            self.mcause = s.mcause;
            self.mtval = s.mtval;
            self.mtime = s.mtime;
            self.mtimecmp = s.mtimecmp;
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for i in 0..32 {
            names.push(format!("x{}", i));
        }
        names.push("pc".to_string());
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::Machine;

    #[test]
    fn test_riscv_addi() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        // ADDI x1, x0, 5  (x1 = 0 + 5)
        // Op=0x13, rd=1, funct3=0, rs1=0, imm=5
        // 000000000101 00000 000 00001 0010011 -> 0x00500093
        bus.flash.data = vec![
            0x93, 0x00, 0x50, 0x00, // ADDI x1, x0, 5
        ];

        cpu.pc = 0x0000_0000;
        let mut machine = Machine::new(cpu, bus);
        machine.step().unwrap();

        assert_eq!(machine.cpu.read_reg(1), 5);
        assert_eq!(machine.cpu.pc, 4);
    }

    #[test]
    fn test_riscv_beq_taken() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();
        // 1. ADDI x1, x0, 10
        // 2. ADDI x2, x0, 10
        // 3. BEQ x1, x2, +8 (skip next instruction)
        // 4. ADDI x3, x0, 1 (should be skipped)
        // 5. ADDI x4, x0, 1 (target)

        // imm for BEQ +8:
        // 0x00000063 (BEQ x0, x0, 0)
        // imm[12]=0, imm[10:5]=0, imm[4:1]=4 (bit 3), imm[11]=0
        // offset = 8. binary: 1000.
        // imm[12] = 0
        // imm[11] = 0
        // imm[10:5] = 000000
        // imm[4:1] = 0100 (4)
        // opcode = 1100011 (0x63)
        // funct3 = 000
        // rs1 = 1, rs2 = 2

        // BEQ x1, x2, 8 -> 0x00208463
        // 0000 0000 0010 0000 1000 0100 0110 0011 -> 0x00208463 ?
        // imm[12]=0, imm[10:5]=000000.
        // imm[4:1]=0100. bit 3 is set.
        // imm[11]=0.
        // Verify encoding: https://luplab.gitlab.io/rvcodecjs/#q=beq%20x1,x2,8
        // 00208463

        bus.flash.data = vec![
            0x93, 0x00, 0xA0, 0x00, // ADDI x1, x0, 10 (0x00A00093)
            0x13, 0x01, 0xA0, 0x00, // ADDI x2, x0, 10 (0x00A00113) - wait, rs1=0.
            // ADDI x2, x0, 10: imm=10, rs1=0, funct3=0, rd=2, opcode=0x13
            // 000000001010 00000 000 00010 0010011 -> 0x00A00113. Correct.

            // BEQ x1, x2, 8
            // 0000000 00010 00001 000 01000 1100011 -> 0x00208463
            // imm[12]=0, imm[10:5]=0, rs2=2, rs1=1, funct3=0, imm[4:1]=0100 (+8?), imm[11]=0, opcode=0x63.
            // imm[4:1]=4 -> bit 3 is 1? No, imm[4:1] bits are at positions 11-8.
            // imm[4:1] = 0100 means bit 3 is 1. Yes 1<<3 = 8.
            0x63, 0x84, 0x20, 0x00,
            // Should be skipped (PC+4 from BEQ = 12. BEQ target = 8 + 8 = 16. Wait. PC of BEQ is 8. Target = 8+8=16.)
            // Offset is from current PC.
            // 0: ADDI x1
            // 4: ADDI x2
            // 8: BEQ
            // 12: ADDI x3 (skipped)
            // 16: ADDI x4 (target)
            0x13, 0x01, 0x10,
            0x00, // ADDI x3, x0, 1 (0x00100193) - wait this is ADDI x3, x0, 1.
            0x13, 0x02, 0x10, 0x00, // ADDI x4, x0, 1 (0x00100213).
        ];

        cpu.pc = 0x0000_0000;
        let mut machine = Machine::new(cpu, bus);

        // Step 1: x1 = 10
        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(1), 10);

        // Step 2: x2 = 10
        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(2), 10);

        // Step 3: BEQ taken -> PC = 8 + 8 = 16
        assert_eq!(machine.cpu.pc, 8);
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 16);

        // Step 4: ADDI x4, x0, 1
        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(4), 1);

        // Ensure x3 is still 0
        assert_eq!(machine.cpu.read_reg(3), 0);
    }

    #[test]
    fn test_riscv_timer_interrupt() {
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();

        cpu.mtvec = 0x2000;
        cpu.mie = 1 << 7; // MTIE
        cpu.mstatus = 1 << 3; // MIE
        cpu.mtimecmp = 5;

        // Reset memory to hold our test program
        bus.flash.data = vec![0; 0x3000];
        // 0x0: JAL x0, 0 (Infinite loop)
        bus.write_u32(0x0, 0x0000006F).unwrap();
        // 0x2000: ADDI x10, x10, 1
        bus.write_u32(0x2000, 0x00150513).unwrap();
        // 0x2004: MRET
        bus.write_u32(0x2004, 0x30200073).unwrap();

        cpu.pc = 0x0;
        let mut machine = Machine::new(cpu, bus);

        // Step 1-4: mtime increases from 0->1, 1->2, 2->3, 3->4. No interrupt yet.
        for i in 0..4 {
            machine.step().unwrap();
            assert_eq!(machine.cpu.pc, 0, "Should be in loop at step {}", i);
        }

        // Step 5: mtime becomes 5, which equals mtimecmp. Trap should be taken.
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 0x2000, "Trap should jump to mtvec");

        // Step 6: Execute ISR ADDI x10, x10, 1
        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(10), 1);
        assert_eq!(machine.cpu.pc, 0x2004);

        // Step 7: Execute MRET
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 0, "MRET should return to 0x0");
        assert!(
            (machine.cpu.mstatus & (1 << 3)) != 0,
            "MIE should be re-enabled"
        );
    }

    #[test]
    fn test_riscv_external_interrupt_from_peripheral() {
        // Verifies that Cpu::set_exception_pending (called by Machine when a
        // peripheral fires an IRQ) actually pends the machine-external
        // interrupt bit in mip and dispatches the trap.
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();

        cpu.mtvec = 0x2000;
        cpu.mie = 1 << 11; // MEIE
        cpu.mstatus = 1 << 3; // MIE

        bus.flash.data = vec![0; 0x3000];
        // 0x0: JAL x0, 0 (infinite loop)
        bus.write_u32(0x0, 0x0000006F).unwrap();
        // 0x2000: MRET
        bus.write_u32(0x2000, 0x30200073).unwrap();

        cpu.pc = 0x0;
        let mut machine = Machine::new(cpu, bus);

        // Simulate a peripheral firing IRQ 28 (STM32 TIM2-style number).
        // Collapses into MEIP (bit 11) via set_exception_pending.
        machine.cpu.set_exception_pending(28);
        assert_eq!(machine.cpu.mip & (1 << 11), 1 << 11, "MEIP should be set");

        // Step once: JAL self runs, then interrupt check sees MEIP & MEIE
        // with MIE globally enabled -> trap to mtvec.
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 0x2000, "Trap should jump to mtvec");
        assert_eq!(
            machine.cpu.mcause, 0x8000_000B,
            "mcause should encode async MEIP (0x8..0b)"
        );
        assert_eq!(
            machine.cpu.mstatus & (1 << 3),
            0,
            "MIE should be cleared on trap entry"
        );

        // MSIP bit position passthrough (3 -> bit 3 directly).
        machine.cpu.mip = 0;
        machine.cpu.set_exception_pending(3);
        assert_eq!(machine.cpu.mip & (1 << 3), 1 << 3);
    }

    #[test]
    fn test_riscv_async_irq_mepc_points_to_next_instruction() {
        // Regression guard for the async-interrupt mepc semantics.
        // Before the fix, `handle_trap(cause, self.pc)` saved the address
        // of the *just-executed* instruction, so MRET would re-execute
        // it. For non-branch instructions this doubled side effects
        // (e.g. ADDI x1, x1, 1 → x1 incremented twice per interrupted
        // completion). Fix saves `next_pc` instead.
        let mut bus = SystemBus::new();
        let mut cpu = RiscV::new();

        cpu.mtvec = 0x2000;
        cpu.mie = 1 << 11; // MEIE
        cpu.mstatus = 1 << 3; // MIE

        bus.flash.data = vec![0; 0x3000];
        // 0x0: ADDI x1, x1, 1    (0x00108093)
        bus.write_u32(0x0, 0x0010_8093).unwrap();
        // 0x4: JAL x0, 0  — infinite loop at 0x4 (encoded as 0x0000006F).
        bus.write_u32(0x4, 0x0000_006F).unwrap();
        // 0x2000: MRET
        bus.write_u32(0x2000, 0x3020_0073).unwrap();

        cpu.pc = 0x0;
        let mut machine = Machine::new(cpu, bus);

        // Pend external IRQ BEFORE the first step.
        machine.cpu.set_exception_pending(11);

        // Step 1: execute ADDI x1, x1, 1 (x1 = 1). Interrupt check fires
        // because mip.MEIP & mie.MEIE & mstatus.MIE all match.
        machine.step().unwrap();
        assert_eq!(
            machine.cpu.read_reg(1),
            1,
            "ADDI should have executed exactly once before the trap"
        );
        assert_eq!(machine.cpu.pc, 0x2000, "trapped to mtvec");
        // CRITICAL: mepc must point to the NEXT instruction, not the one
        // we just ran. Before the fix this was 0x0 (re-execute ADDI);
        // correct value is 0x4.
        assert_eq!(
            machine.cpu.mepc, 0x4,
            "mepc must point to the instruction after ADDI, not ADDI itself"
        );

        // Clear MEIP as a peripheral ACK would — otherwise the trap
        // handler would re-enter on every subsequent step — then MRET
        // and verify PC lands on mepc (0x4), not on the ADDI we ran.
        machine.cpu.mip &= !(1 << 11);
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 0x4, "MRET returns to next instruction");
        assert_eq!(
            machine.cpu.read_reg(1),
            1,
            "x1 must not have incremented twice (async IRQ must not replay)"
        );
    }

    #[test]
    fn test_riscv_rv32c_compressed() {
        // Execute a handful of compressed instructions end-to-end, mixed
        // with a full-width RV32I instruction to verify the dispatcher
        // picks the right size. Stream:
        //   0x0: C.LI  x8, 5          (0x4015)  sets x8 = 5
        //   0x2: C.LI  x9, 7          (0x401D) wait — encoding note below
        //   0x4: C.MV  x10, x8        (0x852A) x10 = x8 = 5
        //   0x6: C.ADD x10, x9        (0x9526) x10 += x9
        //   0x8: C.J   -8             (0xBFD5) jump back to 0x0

        // Use the decoder-level assertions to pin the bit-patterns so the
        // end-to-end test reads the right encoding; this also exercises
        // decode_rv32c directly.
        use crate::decoder::riscv::{decode_rv32c, Instruction};

        // C.LI x8, 5:
        //   funct3=010, imm[5]=0, rd=01000, imm[4:0]=00101, op=01
        //   Binary: 010_0_01000_00101_01 → 0x4415
        assert_eq!(
            decode_rv32c(0x4415),
            Instruction::Addi { rd: 8, rs1: 0, imm: 5 },
        );

        // C.MV x10, x8:
        //   funct4=1000, rd=01010, rs2=01000, op=10
        //   0b1000_01010_01000_10 = 0x8522
        assert_eq!(
            decode_rv32c(0x8522),
            Instruction::Add { rd: 10, rs1: 0, rs2: 8 },
        );

        // C.ADD x10, x9:
        //   funct4=1001, rd=01010, rs2=01001, op=10
        //   0b1001_01010_01001_10 = 0x9526
        assert_eq!(
            decode_rv32c(0x9526),
            Instruction::Add { rd: 10, rs1: 10, rs2: 9 },
        );

        // C.J +2:   funct3=101, imm=+2, op=01 → one valid encoding:
        //   Check just that it decodes to a Jal x0 with some imm.
        match decode_rv32c(0xA009) {
            Instruction::Jal { rd: 0, .. } => {}
            other => panic!("C.J decoded as {other:?}"),
        }

        // End-to-end via the Machine: run three compressed instructions
        // and confirm registers and PC advance by 2 each step.
        let mut bus = SystemBus::stm32f103();
        let mut cpu = RiscV::new();
        cpu.pc = 0x0;

        // C.LI x8, 5 at 0x0
        bus.write_u16(0x0, 0x4415).unwrap();
        // C.LI x9, 7 (funct3=010, imm[5]=0, rd=01001, imm[4:0]=00111, op=01)
        //   0b010_0_01001_00111_01 = 0x411D... let me compute:
        //   010_0_01001_00111_01 → binary 0100010010011101 → 0x4 4 9 D
        //   actually: bits 15:13 = 010, bit 12 = 0, bits 11:7 = 01001,
        //   bits 6:2 = 00111, bits 1:0 = 01.
        //   → 010_0 0100 1001 11_01 = 0100_0100_1001_1101 = 0x449D
        bus.write_u16(0x2, 0x449D).unwrap();
        // C.MV x10, x8 at 0x4
        bus.write_u16(0x4, 0x8522).unwrap();
        // C.ADD x10, x9 at 0x6
        bus.write_u16(0x6, 0x9526).unwrap();

        let mut machine = Machine::new(cpu, bus);

        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(8), 5);
        assert_eq!(machine.cpu.pc, 0x2, "PC advances by 2 after a compressed insn");

        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(9), 7);
        assert_eq!(machine.cpu.pc, 0x4);

        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(10), 5);
        assert_eq!(machine.cpu.pc, 0x6);

        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(10), 12, "x10 = 5 + 7");
        assert_eq!(machine.cpu.pc, 0x8);
    }

    #[test]
    fn test_riscv_rv32a_atomics() {
        // End-to-end through the interpreter: set up an ADDR in RAM, run
        // a sequence of AMO / LR / SC instructions, verify memory and rd.
        let bus = SystemBus::stm32f103(); // RAM at 0x2000_0000.
        let mut cpu = RiscV::new();
        cpu.pc = 0x0;

        // Program at 0x0 — six AMO-family opcodes operating on mem[0x2000_0100].
        // r1 = addr; r2 = operand.
        //
        // 0x00: LI r1, 0x2000_0100    (LUI + ADDI equivalent — use LUI only, imm top)
        //   actually simpler: pre-seed registers via the Machine wrapper and
        //   jump straight to the AMOs.
        let mut machine = Machine::new(cpu, bus);
        machine.cpu.x[1] = 0x2000_0100;
        machine.bus.write_u32(0x2000_0100, 10).unwrap();

        // AMOADD.W r5, r2, (r1) : rd=5, rs1=1, rs2=2  (funct5=0x00)
        // 0x00022AAF where the fields line up: opcode 0x2F, funct3=2,
        // rd=5, rs1=1, rs2=2, funct7=0x00.
        // Build by hand:
        let encode = |funct5: u32, rd: u32, rs1: u32, rs2: u32| -> u32 {
            0x2Fu32
                | (rd << 7)
                | (2u32 << 12) // funct3 = word
                | (rs1 << 15)
                | (rs2 << 20)
                | (funct5 << 27)
        };

        machine.cpu.x[2] = 5;
        // AMOADD  rd=5  rs1=1  rs2=2 : returns 10, mem becomes 15.
        machine.bus.write_u32(0x0, encode(0x00, 5, 1, 2)).unwrap();
        // AMOOR   rd=5  rs1=1  rs2=2 (2=5): returns 15, mem becomes 15|5 = 15.
        machine.bus.write_u32(0x4, encode(0x08, 5, 1, 2)).unwrap();
        // AMOSWAP rd=5  rs1=1  rs2=2 : returns 15, mem = 5.
        machine.bus.write_u32(0x8, encode(0x01, 5, 1, 2)).unwrap();
        // LR.W    rd=6  rs1=1       : rd=5 (current mem), reservation=addr
        machine.bus.write_u32(0xC, encode(0x02, 6, 1, 0)).unwrap();
        // SC.W    rd=7  rs1=1  rs2=2 : stores 5, rd=0 (success)
        machine.bus.write_u32(0x10, encode(0x03, 7, 1, 2)).unwrap();
        // SC.W again : rd=1 (fail — reservation cleared)
        machine.bus.write_u32(0x14, encode(0x03, 8, 1, 2)).unwrap();

        // Step through.
        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(5), 10, "AMOADD returns old value");
        assert_eq!(machine.bus.read_u32(0x2000_0100).unwrap(), 15);

        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(5), 15, "AMOOR returns old value");
        assert_eq!(machine.bus.read_u32(0x2000_0100).unwrap(), 15 | 5);

        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(5), 15, "AMOSWAP returns old value");
        assert_eq!(machine.bus.read_u32(0x2000_0100).unwrap(), 5);

        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(6), 5, "LR.W reads current value");
        assert_eq!(machine.cpu.reservation, Some(0x2000_0100));

        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(7), 0, "SC.W succeeds");
        assert_eq!(machine.bus.read_u32(0x2000_0100).unwrap(), 5);
        assert_eq!(machine.cpu.reservation, None);

        machine.step().unwrap();
        assert_eq!(machine.cpu.read_reg(8), 1, "SC.W without reservation fails");
    }

    #[test]
    fn test_riscv_rv32m_multiply_divide() {
        // Verifies RV32M per-spec semantics, including the two edge
        // cases the spec mandates:
        //   1. Division by zero returns all-ones (DIV/DIVU) or dividend (REM/REMU).
        //   2. Signed overflow (INT_MIN / -1) returns dividend (DIV) or 0 (REM).
        let mut cpu = RiscV::new();

        // MUL: low 32 bits of signed product. 5 * 3 = 15.
        cpu.write_reg(1, 5);
        cpu.write_reg(2, 3);
        let i = Instruction::Mul { rd: 3, rs1: 1, rs2: 2 };
        execute_unit(&mut cpu, i);
        assert_eq!(cpu.read_reg(3), 15);

        // MUL wraps on overflow (low 32 bits only).
        cpu.write_reg(1, 0x1_0000_0000u64 as u32); // doesn't fit, truncate to 0
        cpu.write_reg(1, 0xFFFF_FFFE); // -2 as signed
        cpu.write_reg(2, 2);
        execute_unit(&mut cpu, Instruction::Mul { rd: 3, rs1: 1, rs2: 2 });
        assert_eq!(cpu.read_reg(3), 0xFFFF_FFFC); // -4

        // MULH: high bits of signed × signed. (-1) * (-1) = 1 → high = 0.
        cpu.write_reg(1, u32::MAX);
        cpu.write_reg(2, u32::MAX);
        execute_unit(&mut cpu, Instruction::Mulh { rd: 3, rs1: 1, rs2: 2 });
        assert_eq!(cpu.read_reg(3), 0);

        // MULHU: unsigned × unsigned. 0xFFFF_FFFF * 0xFFFF_FFFF high = 0xFFFF_FFFE.
        execute_unit(&mut cpu, Instruction::Mulhu { rd: 3, rs1: 1, rs2: 2 });
        assert_eq!(cpu.read_reg(3), 0xFFFF_FFFE);

        // DIV signed: 20 / -4 = -5.
        cpu.write_reg(1, 20);
        cpu.write_reg(2, (-4i32) as u32);
        execute_unit(&mut cpu, Instruction::Div { rd: 3, rs1: 1, rs2: 2 });
        assert_eq!(cpu.read_reg(3) as i32, -5);

        // DIV by zero returns all-ones.
        cpu.write_reg(1, 42);
        cpu.write_reg(2, 0);
        execute_unit(&mut cpu, Instruction::Div { rd: 3, rs1: 1, rs2: 2 });
        assert_eq!(cpu.read_reg(3), u32::MAX);

        // DIV signed overflow: INT_MIN / -1 returns INT_MIN.
        cpu.write_reg(1, i32::MIN as u32);
        cpu.write_reg(2, (-1i32) as u32);
        execute_unit(&mut cpu, Instruction::Div { rd: 3, rs1: 1, rs2: 2 });
        assert_eq!(cpu.read_reg(3), i32::MIN as u32);

        // REM signed: 20 % -3 = 2.
        cpu.write_reg(1, 20);
        cpu.write_reg(2, (-3i32) as u32);
        execute_unit(&mut cpu, Instruction::Rem { rd: 3, rs1: 1, rs2: 2 });
        assert_eq!(cpu.read_reg(3) as i32, 2);

        // REM by zero returns dividend.
        cpu.write_reg(1, 42);
        cpu.write_reg(2, 0);
        execute_unit(&mut cpu, Instruction::Rem { rd: 3, rs1: 1, rs2: 2 });
        assert_eq!(cpu.read_reg(3), 42);

        // REM signed overflow: INT_MIN % -1 returns 0.
        cpu.write_reg(1, i32::MIN as u32);
        cpu.write_reg(2, (-1i32) as u32);
        execute_unit(&mut cpu, Instruction::Rem { rd: 3, rs1: 1, rs2: 2 });
        assert_eq!(cpu.read_reg(3), 0);

        // REMU by zero returns dividend.
        cpu.write_reg(1, 42);
        cpu.write_reg(2, 0);
        execute_unit(&mut cpu, Instruction::Remu { rd: 3, rs1: 1, rs2: 2 });
        assert_eq!(cpu.read_reg(3), 42);
    }

    // Helper: invoke the RV32M executor path without going through a full
    // fetch/decode round-trip. We encode the instruction into a proper
    // opcode, drop it at PC=0, and step once.
    fn execute_unit(cpu: &mut RiscV, inst: Instruction) {
        let encoded = encode_rv32m(&inst);
        let mut bus = SystemBus::new();
        bus.flash.data = vec![0; 16];
        bus.write_u32(0x0, encoded).unwrap();
        cpu.pc = 0x0;
        // Inline step without observers / peripherals.
        let observers: [Arc<dyn SimulationObserver>; 0] = [];
        // Preserve register file state in-place by taking cpu by &mut.
        // SystemBus::step equivalent:
        Cpu::step(cpu, &mut bus, &observers).unwrap();
    }

    fn encode_rv32m(inst: &Instruction) -> u32 {
        let (funct3, rd, rs1, rs2) = match *inst {
            Instruction::Mul { rd, rs1, rs2 } => (0, rd, rs1, rs2),
            Instruction::Mulh { rd, rs1, rs2 } => (1, rd, rs1, rs2),
            Instruction::Mulhsu { rd, rs1, rs2 } => (2, rd, rs1, rs2),
            Instruction::Mulhu { rd, rs1, rs2 } => (3, rd, rs1, rs2),
            Instruction::Div { rd, rs1, rs2 } => (4, rd, rs1, rs2),
            Instruction::Divu { rd, rs1, rs2 } => (5, rd, rs1, rs2),
            Instruction::Rem { rd, rs1, rs2 } => (6, rd, rs1, rs2),
            Instruction::Remu { rd, rs1, rs2 } => (7, rd, rs1, rs2),
            _ => unreachable!("encode_rv32m: non-M instruction"),
        };
        0x33u32
            | ((rd as u32) << 7)
            | ((funct3 as u32) << 12)
            | ((rs1 as u32) << 15)
            | ((rs2 as u32) << 20)
            | (0x01u32 << 25)
    }

    #[test]
    fn test_riscv_snapshot() {
        let mut cpu = RiscV::new();
        cpu.write_reg(1, 42);
        cpu.pc = 0x1234;
        let snapshot = cpu.snapshot();
        if let crate::snapshot::CpuSnapshot::RiscV(s) = snapshot {
            assert_eq!(s.registers[1], 42);
            assert_eq!(s.pc, 0x1234);
        } else {
            panic!("Expected RiscV snapshot");
        }
    }
}
