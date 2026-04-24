// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Bus, Cpu, SimResult, SimulationObserver};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

mod helpers;
mod step;

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
        self.step_execute(bus, observers)
    }
}

