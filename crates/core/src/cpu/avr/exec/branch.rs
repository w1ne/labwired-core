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
use crate::{Bus, SimResult};

impl Avr {
    /// RET, RETI.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_branch_a(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        _next: u32,
    ) -> SimResult<Option<()>> {
        if op == 0x9508 {
            self.pop_pc(bus)?;
            self.cycles += 4;
            return Ok(Some(()));
        }
        if op == 0x9518 {
            self.pop_pc(bus)?;
            self.set_flag_i(true);
            self.cycles += 4;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// RJMP, RCALL.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_branch_b(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        // RJMP
        if (op & 0xF000) == 0xC000 {
            let k = op & 0x0FFF;
            let offset = if k & 0x0800 != 0 {
                (k | 0xF000) as i16
            } else {
                k as i16
            };
            let pc_word = (pc / 2) as i32 + 1 + offset as i32;
            self.pc = (pc_word as u32) * 2;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // RCALL
        if (op & 0xF000) == 0xD000 {
            let k = op & 0x0FFF;
            let offset = if k & 0x0800 != 0 {
                (k | 0xF000) as i16
            } else {
                k as i16
            };
            self.pc = next;
            self.push_pc(bus)?;
            let pc_word = (pc / 2) as i32 + 1 + offset as i32;
            self.pc = (pc_word as u32) * 2;
            self.cycles += 3;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// ICALL: 1001 0101 0000 1001 — call to Z (word address).
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_branch_c(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        if op == 0x9509 {
            self.pc = next;
            self.push_pc(bus)?;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.pc = (z as u32) * 2;
            self.cycles += 3;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// BRcc.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_branch_d(
        &mut self,
        _bus: &mut dyn Bus,
        op: u16,
        pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        if (op & 0xF800) == 0xF000 {
            let k = ((op >> 3) & 0x7F) as i8;
            let offset = if k & 0x40 != 0 { k | !0x7F } else { k };
            let bit = (op & 0x07) as u8;
            let complement = (op & 0x0400) != 0;
            let flag = (self.sreg >> bit) & 1 != 0;
            let take = if complement { !flag } else { flag };
            if take {
                let pc_word = (pc / 2) as i32 + 1 + offset as i32;
                self.pc = (pc_word as u32) * 2;
                self.cycles += 2;
            } else {
                self.pc = next;
                self.cycles += 1;
            }
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// IJMP.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_branch_e(
        &mut self,
        _bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        _next: u32,
    ) -> SimResult<Option<()>> {
        if op == 0x9409 {
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.pc = (z as u32) * 2;
            self.cycles += 2;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// JMP, CALL.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_branch_f(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        // JMP
        if (op & 0xFE0E) == 0x940C {
            let k_hi = ((op >> 3) & 0x3E) | (op & 0x01);
            let k_lo = self.fetch_word(next)?;
            let k = ((k_hi as u32) << 16) | k_lo as u32;
            self.pc = k * 2;
            self.cycles += 3;
            return Ok(Some(()));
        }

        // CALL
        if (op & 0xFE0E) == 0x940E {
            let k_hi = ((op >> 3) & 0x3E) | (op & 0x01);
            let k_lo = self.fetch_word(next)?;
            let k = ((k_hi as u32) << 16) | k_lo as u32;
            self.pc = next.wrapping_add(2);
            self.push_pc(bus)?;
            self.pc = k * 2;
            self.cycles += 4;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// SBIS, SBIC, SBRS, SBRC, CPSE.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_branch_g(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        // SBIS
        if (op & 0xFF00) == 0x9B00 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let v = self.data_read(0x20 + a as u16, bus)?;
            let mut next = next;
            if v & (1 << b) != 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // SBIC
        if (op & 0xFF00) == 0x9900 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let v = self.data_read(0x20 + a as u16, bus)?;
            let mut next = next;
            if v & (1 << b) == 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // SBRS
        if (op & 0xFE08) == 0xFE00 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let b = (op & 0x07) as u8;
            let mut next = next;
            if self.r[rr] & (1 << b) != 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // SBRC
        if (op & 0xFE08) == 0xFC00 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let b = (op & 0x07) as u8;
            let mut next = next;
            if self.r[rr] & (1 << b) == 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // CPSE
        if (op & 0xFC00) == 0x1000 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let mut next = next;
            if self.r[rd] == self.r[rr] {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        Ok(None)
    }
}
