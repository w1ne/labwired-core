// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Split out of `step_inner` in `cpu/avr.rs` (pure move, no behaviour
// change). See that file's `step_inner` and `exec/mod.rs` for the shared
// fetch/next-pc plumbing and the fixed original check order this respects.

use crate::cpu::avr::Avr;
use crate::{Bus, SimResult, SimulationError};

impl Avr {
    /// NOP, SEI, CLI.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_system_a(
        &mut self,
        _bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        if op == 0x0000 {
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        if op == 0x9478 {
            self.set_flag_i(true);
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        if op == 0x94F8 {
            self.set_flag_i(false);
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// SLEEP, BREAK.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_system_b(
        &mut self,
        _bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        if op == 0x9588 {
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        if op == 0x9598 {
            return Err(SimulationError::Halt);
        }
        Ok(None)
    }

    /// OUT, IN.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_system_c(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        // OUT
        if (op & 0xF800) == 0xB800 {
            let a = (((op >> 5) & 0x30) | (op & 0x0F)) as u8;
            let rr = ((op >> 4) & 0x1F) as usize;
            let data_addr = 0x20u16 + a as u16;
            self.data_write(data_addr, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // IN
        if (op & 0xF800) == 0xB000 {
            let a = (((op >> 5) & 0x30) | (op & 0x0F)) as u8;
            let rd = ((op >> 4) & 0x1F) as usize;
            let data_addr = 0x20u16 + a as u16;
            self.r[rd] = self.data_read(data_addr, bus)?;
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        Ok(None)
    }
}
