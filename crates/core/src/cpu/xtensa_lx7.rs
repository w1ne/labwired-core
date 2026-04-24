// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Xtensa LX7 CPU backend. Glues AR file, PS, SR file with the fetch loop
//! and `Cpu` trait. Exec is stubbed in Plan 1 (returns NotImplemented for
//! all decoded instructions); Phase D fills it in.

use crate::cpu::xtensa_regs::{ArFile, Ps};
use crate::cpu::xtensa_sr::{XtensaSrFile, VECBASE};
use crate::decoder::{xtensa, xtensa_length, xtensa_narrow};
use crate::snapshot::{CpuSnapshot, XtensaLx7CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationError, SimulationObserver};
use std::sync::Arc;

pub struct XtensaLx7 {
    pub regs: ArFile,
    pub ps: Ps,
    pub sr: XtensaSrFile,
    pub pc: u32,
}

impl XtensaLx7 {
    pub fn new() -> Self {
        Self {
            regs: ArFile::new(),
            // HW-verified: PS reset = 0x1F (EXCM=1, INTLEVEL=0xF).
            // Confirmed via OpenOCD `reset halt` on real S3-Zero: ps = 0x0000001f.
            ps: Ps::from_raw(0x1F),
            sr: XtensaSrFile::new(),
            pc: 0x4000_0400,
        }
    }

    fn execute(
        &mut self,
        ins: xtensa::Instruction,
        _bus: &mut dyn Bus,
        _len: u32,
    ) -> SimResult<()> {
        Err(SimulationError::NotImplemented(format!(
            "xtensa exec stub: {:?}",
            ins
        )))
    }
}

impl Default for XtensaLx7 {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu for XtensaLx7 {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.regs = ArFile::new();
        // HW-verified: PS reset = 0x1F (EXCM=1, INTLEVEL=0xF — all ints masked).
        // Confirmed via OpenOCD `reset halt` on real S3-Zero: ps = 0x0000001f.
        self.ps = Ps::from_raw(0x1F);
        self.sr = XtensaSrFile::new(); // sets VECBASE=0x40000000, PRID=0xCDCD
        self.pc = 0x4000_0400;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        _observers: &[Arc<dyn SimulationObserver>],
    ) -> SimResult<()> {
        let pc = self.pc;
        let b0 = bus.read_u8(pc as u64)?;
        let len = xtensa_length::instruction_length(b0);
        let ins = if len == 2 {
            let hw = bus.read_u16(pc as u64)?;
            xtensa_narrow::decode_narrow(hw)
        } else {
            let w = bus.read_u32(pc as u64)?;
            xtensa::decode(w)
        };
        self.execute(ins, bus, len)
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, val: u32) {
        // a1 is the stack pointer in the Xtensa windowed ABI.
        self.regs.write_logical(1, val);
    }

    fn set_exception_pending(&mut self, _exception_num: u32) {
        // Phase G implements interrupt dispatch; for Plan 1 this is a no-op.
    }

    fn get_register(&self, id: u8) -> u32 {
        if id < 16 {
            self.regs.read_logical(id)
        } else {
            0
        }
    }

    fn set_register(&mut self, id: u8, val: u32) {
        if id < 16 {
            self.regs.write_logical(id, val);
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::XtensaLx7(XtensaLx7CpuSnapshot {
            registers: (0u8..16).map(|i| self.regs.read_logical(i)).collect(),
            pc: self.pc,
            ps: self.ps.as_raw(),
            window_base: self.regs.windowbase(),
            window_start: self.regs.windowstart(),
            vecbase: self.sr.read(VECBASE),
        })
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::XtensaLx7(s) = snapshot {
            self.pc = s.pc;
            self.ps = Ps::from_raw(s.ps);
            self.regs.set_windowbase(s.window_base);
            self.regs.set_windowstart(s.window_start);
            for (i, &v) in s.registers.iter().enumerate().take(16) {
                self.regs.write_logical(i as u8, v);
            }
            self.sr.set_raw(VECBASE, s.vecbase);
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        (0..16).map(|i| format!("a{}", i)).collect()
    }
}
