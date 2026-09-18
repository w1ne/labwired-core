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
    /// ADD, ADC, EOR, AND, OR, CP, SUB.
    #[allow(clippy::too_many_lines)]
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_arith_a(
        &mut self,
        op: u16,
        next: u32,
    ) -> SimResult<Option<()>> {
        // ADD
        if (op & 0xFC00) == 0x0C00 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let a = self.r[rd];
            let b = self.r[rr];
            let (res, c) = a.overflowing_add(b);
            let v = (!(a ^ b) & (a ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        // ADC Rd,Rr: 0001 11rd dddd rrrr
        if (op & 0xFC00) == 0x1C00 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let carry = self.sreg & 1;
            let sum = self.r[rd] as u16 + self.r[rr] as u16 + carry as u16;
            let res = sum as u8;
            let c = sum > 0xFF;
            let v = (!(self.r[rd] ^ self.r[rr]) & (self.r[rd] ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // EOR
        if (op & 0xFC00) == 0x2400 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let res = self.r[rd] ^ self.r[rr];
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // AND
        if (op & 0xFC00) == 0x2000 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let res = self.r[rd] & self.r[rr];
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // OR
        if (op & 0xFC00) == 0x2800 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let res = self.r[rd] | self.r[rr];
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // CP
        if (op & 0xFC00) == 0x1400 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let a = self.r[rd];
            let b = self.r[rr];
            let (res, c) = a.overflowing_sub(b);
            let v = ((a ^ b) & (a ^ res)) & 0x80 != 0;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // SUB Rd,Rr: 0001 10rd dddd rrrr
        if (op & 0xFC00) == 0x1800 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let a = self.r[rd];
            let b = self.r[rr];
            let (res, c) = a.overflowing_sub(b);
            let v = ((a ^ b) & (a ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// NEG Rd: 1001 010d dddd 0001.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_arith_b(
        &mut self,
        op: u16,
        next: u32,
    ) -> SimResult<Option<()>> {
        if (op & 0xFE0F) == 0x9401 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let a = self.r[rd];
            let res = (0u8).wrapping_sub(a);
            self.r[rd] = res;
            self.set_c(a != 0);
            self.set_z(res);
            self.set_n(res);
            self.set_v(res == 0x80);
            self.update_s_from_nv();
            // H flag: roughly from borrow into bit 3
            if (a & 0x0F) != 0 {
                self.sreg |= 0x20;
            } else {
                self.sreg &= !0x20;
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// CPI.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_arith_c(
        &mut self,
        op: u16,
        next: u32,
    ) -> SimResult<Option<()>> {
        if (op & 0xF000) == 0x3000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            let a = self.r[rd];
            let (res, c) = a.overflowing_sub(k);
            let v = ((a ^ k) & (a ^ res)) & 0x80 != 0;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// ADIW, SBIW.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_arith_d(
        &mut self,
        op: u16,
        next: u32,
    ) -> SimResult<Option<()>> {
        // ADIW
        if (op & 0xFF00) == 0x9600 {
            let d = ((op >> 4) & 0x03) as usize;
            let rd = 24 + d * 2;
            let k = (((op >> 6) & 0x03) << 4) | (op & 0x0F);
            let before = u16::from_le_bytes([self.r[rd], self.r[rd + 1]]);
            let val = before.wrapping_add(k);
            self.r[rd] = (val & 0xFF) as u8;
            self.r[rd + 1] = (val >> 8) as u8;
            // AVR instruction set: V = !Rdh7 & R15, C = !R15 & Rdh7. Leaving C
            // alone made a following `ADC Rn, r1` add a stale carry: Arduino's
            // Timer0 ISR does `ADIW r24,1; ADC r26,r1; ADC r27,r1` on
            // `timer0_millis`, so the top half of `millis()` counted interrupts.
            let rdh7 = before & 0x8000 != 0;
            let r15 = val & 0x8000 != 0;
            self.set_z(if val == 0 { 0 } else { 1 });
            self.set_n((val >> 8) as u8);
            self.set_v(!rdh7 && r15);
            self.set_c(rdh7 && !r15);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // SBIW
        if (op & 0xFF00) == 0x9700 {
            let d = ((op >> 4) & 0x03) as usize;
            let rd = 24 + d * 2;
            let k = (((op >> 6) & 0x03) << 4) | (op & 0x0F);
            let before = u16::from_le_bytes([self.r[rd], self.r[rd + 1]]);
            let val = before.wrapping_sub(k);
            self.r[rd] = (val & 0xFF) as u8;
            self.r[rd + 1] = (val >> 8) as u8;
            // AVR instruction set: V = Rdh7 & !R15, C = R15 & !Rdh7 (borrow).
            let rdh7 = before & 0x8000 != 0;
            let r15 = val & 0x8000 != 0;
            self.set_z(if val == 0 { 0 } else { 1 });
            self.set_n((val >> 8) as u8);
            self.set_v(rdh7 && !r15);
            self.set_c(r15 && !rdh7);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// INC, DEC, ANDI, ORI, SUBI, SBCI, CPC, SBC, COM.
    #[allow(clippy::too_many_lines)]
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_arith_e(
        &mut self,
        op: u16,
        next: u32,
    ) -> SimResult<Option<()>> {
        // INC
        if (op & 0xFE0F) == 0x9403 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let res = self.r[rd].wrapping_add(1);
            self.r[rd] = res;
            self.set_v(res == 0x80);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // DEC
        if (op & 0xFE0F) == 0x940A {
            let rd = ((op >> 4) & 0x1F) as usize;
            let res = self.r[rd].wrapping_sub(1);
            self.r[rd] = res;
            self.set_v(res == 0x7F);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // ANDI Rd,K: 0111 KKKK dddd KKKK  (Rd 16..31)
        if (op & 0xF000) == 0x7000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            let res = self.r[rd] & k;
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // ORI Rd,K: 0110 KKKK dddd KKKK
        if (op & 0xF000) == 0x6000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            let res = self.r[rd] | k;
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // SUBI Rd,K: 0101 KKKK dddd KKKK
        if (op & 0xF000) == 0x5000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            let a = self.r[rd];
            let (res, c) = a.overflowing_sub(k);
            let v = ((a ^ k) & (a ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // SBCI Rd,K: 0100 KKKK dddd KKKK
        if (op & 0xF000) == 0x4000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            let carry = self.sreg & 1;
            let a = self.r[rd] as u16;
            let sub = k as u16 + carry as u16;
            let (res16, c1) = a.overflowing_sub(sub);
            let res = res16 as u8;
            let v = ((self.r[rd] ^ k) & (self.r[rd] ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c1 || a < sub);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // CPC Rd,Rr: 0000 01rd dddd rrrr
        if (op & 0xFC00) == 0x0400 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let carry = self.sreg & 1;
            let a = self.r[rd] as u16;
            let b = self.r[rr] as u16 + carry as u16;
            let (res16, _) = a.overflowing_sub(b);
            let res = res16 as u8;
            let c = a < b;
            let v = ((self.r[rd] ^ self.r[rr]) & (self.r[rd] ^ res)) & 0x80 != 0;
            self.set_c(c);
            // Z is sticky for CPC: only clear if res != 0
            if res != 0 {
                self.sreg &= !0x02;
            }
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // SBC Rd,Rr: 0000 10rd dddd rrrr
        if (op & 0xFC00) == 0x0800 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let carry = self.sreg & 1;
            let a = self.r[rd] as u16;
            let b = self.r[rr] as u16 + carry as u16;
            let res = a.wrapping_sub(b) as u8;
            let c = a < b;
            let v = ((self.r[rd] ^ self.r[rr]) & (self.r[rd] ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c);
            if res != 0 {
                self.sreg &= !0x02;
            } else { /* Z sticky leave */
            }
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // COM Rd: 1001 010d dddd 0000
        if (op & 0xFE0F) == 0x9400 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let res = !self.r[rd];
            self.r[rd] = res;
            self.set_c(true);
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        Ok(None)
    }
}
