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
use crate::SimResult;

impl Avr {
    /// MUL, MULS, MULSU.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_mul_a(&mut self, op: u16, next: u32) -> SimResult<Option<()>> {
        // MUL Rd,Rr: 1001 11rd dddd rrrr → R1:R0 = Rd * Rr (unsigned)
        if (op & 0xFC00) == 0x9C00 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let prod = (self.r[rd] as u16) * (self.r[rr] as u16);
            self.r[0] = (prod & 0xFF) as u8;
            self.r[1] = (prod >> 8) as u8;
            self.set_c((prod & 0x8000) != 0);
            self.set_z(if prod == 0 { 0 } else { 1 });
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // MULS Rd,Rr: 0000 0010 dddd rrrr  (Rd,Rr in 16..31)
        if (op & 0xFF00) == 0x0200 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let rr = 16 + (op & 0x0F) as usize;
            let prod = (self.r[rd] as i8 as i16) * (self.r[rr] as i8 as i16);
            let prod_u = prod as u16;
            self.r[0] = (prod_u & 0xFF) as u8;
            self.r[1] = (prod_u >> 8) as u8;
            self.set_c((prod_u & 0x8000) != 0);
            self.set_z(if prod_u == 0 { 0 } else { 1 });
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // MULSU Rd,Rr: 0000 0011 0ddd 0rrr (Rd,Rr in 16..23)
        if (op & 0xFF88) == 0x0300 {
            let rd = 16 + ((op >> 4) & 0x07) as usize;
            let rr = 16 + (op & 0x07) as usize;
            let prod = (self.r[rd] as i8 as i16) * (self.r[rr] as i16);
            let prod_u = prod as u16;
            self.r[0] = (prod_u & 0xFF) as u8;
            self.r[1] = (prod_u >> 8) as u8;
            self.set_c((prod_u & 0x8000) != 0);
            self.set_z(if prod_u == 0 { 0 } else { 1 });
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// FMUL Rd,Rr: 0000 0011 0ddd 1rrr (Rd,Rr in 16..23).
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_mul_b(&mut self, op: u16, next: u32) -> SimResult<Option<()>> {
        if (op & 0xFF88) == 0x0308 {
            let rd = 16 + ((op >> 4) & 0x07) as usize;
            let rr = 16 + (op & 0x07) as usize;
            let prod = (self.r[rd] as u16) * (self.r[rr] as u16);
            let shifted = prod << 1;
            self.r[0] = (shifted & 0xFF) as u8;
            self.r[1] = (shifted >> 8) as u8;
            self.set_c((prod & 0x8000) != 0);
            self.set_z(if shifted == 0 { 0 } else { 1 });
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        Ok(None)
    }
}
