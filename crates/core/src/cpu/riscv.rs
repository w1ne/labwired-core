// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::decoder::riscv::{decode_rv32, Instruction};
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
        self.pc = 0; 
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
    ) -> SimResult<()> {
        let opcode = bus.read_u32(self.pc as u64)?;

        for observer in observers {
            observer.on_step_start(self.pc, opcode);
        }

        let instruction = decode_rv32(opcode);
        tracing::debug!(
            "PC={:#x}, Op={:#08x}, Instr={:?}",
            self.pc,
            opcode,
            instruction
        );

        let mut next_pc = self.pc.wrapping_add(4);

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
                self.write_reg(rd, self.pc.wrapping_add(4));
                next_pc = target;
            }
            Instruction::Jalr { rd, rs1, imm } => {
                let base = self.read_reg(rs1);
                let target = base.wrapping_add(imm as u32) & !1;
                self.write_reg(rd, self.pc.wrapping_add(4));
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
                let val = bus.read_u8(addr as u64)? as i8;
                self.write_reg(rd, val as i32 as u32);
            }
            Instruction::Lh { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u16(addr as u64)? as i16;
                self.write_reg(rd, val as i32 as u32);
            }
            Instruction::Lw { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u32(addr as u64)?;
                self.write_reg(rd, val);
            }
            Instruction::Lbu { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u8(addr as u64)?;
                self.write_reg(rd, val as u32);
            }
            Instruction::Lhu { rd, rs1, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = bus.read_u16(addr as u64)?;
                self.write_reg(rd, val as u32);
            }
            Instruction::Sb { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2) as u8;
                bus.write_u8(addr as u64, val)?;
            }
            Instruction::Sh { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2) as u16;
                bus.write_u16(addr as u64, val)?;
            }
            Instruction::Sw { rs1, rs2, imm } => {
                let addr = self.read_reg(rs1).wrapping_add(imm as u32);
                let val = self.read_reg(rs2);
                bus.write_u32(addr as u64, val)?;
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
                return Err(crate::SimulationError::DecodeError(self.pc as u64));
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
                    self.handle_trap(0x80000000 | irq, self.pc);
                    // Trap taken, next instruction will be handled in trap handler
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
    fn set_exception_pending(&mut self, _exception_num: u32) {
        // TODO: RISC-V Interrupts
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
