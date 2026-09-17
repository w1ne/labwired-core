// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Split out of `step_inner` in `cpu/avr.rs` (pure move, no behaviour
// change). See that file's `step_inner` and `exec/mod.rs` for the shared
// fetch/next-pc plumbing and the fixed original check order this respects.
//
// `exec_load_store_g` keeps several independent opcode checks in one
// function, exactly as they appeared in `step_inner`, because one of them
// (the Y+/-Y/Z+/-Z combined check) can fall through without matching, and
// the checks that follow it in the original file — including one that is
// dead code by construction (see `exec/mod.rs`) — must stay reachable in
// the same relative order.

use crate::cpu::avr::Avr;
use crate::{Bus, SimResult};

impl Avr {
    /// LDI.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_load_store_a(
        &mut self,
        _bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        if (op & 0xF000) == 0xE000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            self.r[rd] = k;
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// LDS, STS.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_load_store_b(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        // LDS
        if (op & 0xFE0F) == 0x9000 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let mut next = next;
            let k = self.fetch_word(next)?;
            next = next.wrapping_add(2);
            self.r[rd] = self.data_read(k, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // STS
        if (op & 0xFE0F) == 0x9200 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let mut next = next;
            let k = self.fetch_word(next)?;
            next = next.wrapping_add(2);
            self.data_write(k, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// MOV.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_load_store_c(
        &mut self,
        _bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        if (op & 0xFC00) == 0x2C00 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            self.r[rd] = self.r[rr];
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// PUSH, POP.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_load_store_d(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        // PUSH
        if (op & 0xFE0F) == 0x920F {
            let rr = ((op >> 4) & 0x1F) as usize;
            self.push_byte(self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // POP
        if (op & 0xFE0F) == 0x900F {
            let rd = ((op >> 4) & 0x1F) as usize;
            self.r[rd] = self.pop_byte(bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// MOVW.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_load_store_e(
        &mut self,
        _bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        if (op & 0xFF00) == 0x0100 {
            let rd = ((op >> 4) & 0x0F) as usize * 2;
            let rr = (op & 0x0F) as usize * 2;
            self.r[rd] = self.r[rr];
            self.r[rd + 1] = self.r[rr + 1];
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// LPM (bare, Rd,Z and Rd,Z+), LD/ST via X with pre/post inc/dec.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_load_store_f(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        // LPM
        if op == 0x95C8 {
            let z = u16::from_le_bytes([self.r[30], self.r[31]]) as usize;
            self.r[0] = self.flash.get(z).copied().unwrap_or(0xFF);
            self.pc = next;
            self.cycles += 3;
            return Ok(Some(()));
        }

        // LPM Rd,Z
        if (op & 0xFE0F) == 0x9004 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]) as usize;
            self.r[rd] = self.flash.get(z).copied().unwrap_or(0xFF);
            self.pc = next;
            self.cycles += 3;
            return Ok(Some(()));
        }

        // LPM Rd,Z+
        if (op & 0xFE0F) == 0x9005 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.r[rd] = self.flash.get(z as usize).copied().unwrap_or(0xFF);
            let z2 = z.wrapping_add(1);
            self.r[30] = (z2 & 0xFF) as u8;
            self.r[31] = (z2 >> 8) as u8;
            self.pc = next;
            self.cycles += 3;
            return Ok(Some(()));
        }

        // LD Rd,X
        if (op & 0xFE0F) == 0x900C {
            let rd = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]);
            self.r[rd] = self.data_read(x, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // ST X,Rr
        // ST X, Rr: 1001 001r rrrr 1100
        if (op & 0xFE0F) == 0x920C {
            let rr = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]);
            self.data_write(x, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // ST X+, Rr: 1001 001r rrrr 1101
        if (op & 0xFE0F) == 0x920D {
            let rr = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]);
            self.data_write(x, self.r[rr], bus)?;
            let x2 = x.wrapping_add(1);
            self.r[26] = (x2 & 0xFF) as u8;
            self.r[27] = (x2 >> 8) as u8;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // ST -X, Rr: 1001 001r rrrr 1110
        if (op & 0xFE0F) == 0x920E {
            let rr = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]).wrapping_sub(1);
            self.r[26] = (x & 0xFF) as u8;
            self.r[27] = (x >> 8) as u8;
            self.data_write(x, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // LD Rd, X+: 1001 000d dddd 1101
        if (op & 0xFE0F) == 0x900D {
            let rd = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]);
            self.r[rd] = self.data_read(x, bus)?;
            let x2 = x.wrapping_add(1);
            self.r[26] = (x2 & 0xFF) as u8;
            self.r[27] = (x2 >> 8) as u8;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // LD Rd, -X: 1001 000d dddd 1110
        if (op & 0xFE0F) == 0x900E {
            let rd = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]).wrapping_sub(1);
            self.r[26] = (x & 0xFF) as u8;
            self.r[27] = (x >> 8) as u8;
            self.r[rd] = self.data_read(x, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// LDD/STD (Y+q/Z+q), LD/ST Y+/-Y/Z+/-Z, and the bare LD/ST Y/Z checks —
    /// the last of which are dead code: the generic LDD/STD check above them
    /// (mask `0xD000 == 0x8000`) already intercepts every opcode they target
    /// when q=0, since that is a strictly broader mask. Reproduced exactly,
    /// not fixed.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_load_store_g(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        _pc: u32,
        next: u32,
    ) -> SimResult<Option<()>> {
        // LDD Rd, Z+q: 10q0 qq0d dddd 0qqq  (bit3=0 → Z)
        // LDD Rd, Y+q: 10q0 qq0d dddd 1qqq  (bit3=1 → Y)
        // STD Z+q / Y+q: same with bit9=1 (store).
        if (op & 0xD000) == 0x8000 {
            let q = ((op & 0x2000) >> 8) | ((op & 0x0C00) >> 7) | (op & 0x07);
            let reg = ((op >> 4) & 0x1F) as usize;
            let is_st = (op & 0x0200) != 0;
            // ISA: bit 3 clear = Z, set = Y (not the other way around).
            let use_y = (op & 0x0008) != 0;
            let base = if use_y {
                u16::from_le_bytes([self.r[28], self.r[29]])
            } else {
                u16::from_le_bytes([self.r[30], self.r[31]])
            };
            let addr = base.wrapping_add(q);
            if is_st {
                self.data_write(addr, self.r[reg], bus)?;
            } else {
                self.r[reg] = self.data_read(addr, bus)?;
            }
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // LD Rd, Y+ / -Y / Z+ / -Z and ST counterparts
        // LD Rd, Y+: 1001 000d dddd 1001
        // LD Rd, -Y: 1001 000d dddd 1010
        // LD Rd, Z+: 1001 000d dddd 0001
        // LD Rd, -Z: 1001 000d dddd 0010
        // ST Y+, Rr: 1001 001r rrrr 1001 etc.
        if (op & 0xFE0C) == 0x9008
            || (op & 0xFE0C) == 0x9000
            || (op & 0xFE0C) == 0x9208
            || (op & 0xFE0C) == 0x9200
        {
            let is_st = (op & 0x0200) != 0;
            let reg = ((op >> 4) & 0x1F) as usize;
            let mode = op & 0x0F;
            // Y modes: 1001, 1010, 1100(ld Y), Z: 0001, 0010, 0000(ld Z bare handled elsewhere)
            let (base_lo, base_hi, predec, postinc) = match mode {
                0x9 => (28usize, 29usize, false, true),  // Y+
                0xA => (28, 29, true, false),            // -Y
                0x1 => (30, 31, false, true),            // Z+
                0x2 => (30, 31, true, false),            // -Z
                0xC if !is_st => (28, 29, false, false), // LD Rd, Y
                0x8 if is_st => (28, 29, false, false),  // unlikely
                0x0 if !is_st && (op & 0xFE0F) == 0x9000 => {
                    // already LDS
                    (0, 0, false, false)
                }
                _ => (0, 0, false, false),
            };
            if base_lo != 0 {
                let mut base = u16::from_le_bytes([self.r[base_lo], self.r[base_hi]]);
                if predec {
                    base = base.wrapping_sub(1);
                }
                if is_st {
                    self.data_write(base, self.r[reg], bus)?;
                } else {
                    self.r[reg] = self.data_read(base, bus)?;
                }
                if postinc {
                    base = base.wrapping_add(1);
                }
                if predec || postinc {
                    self.r[base_lo] = (base & 0xFF) as u8;
                    self.r[base_hi] = (base >> 8) as u8;
                }
                self.pc = next;
                self.cycles += 2;
                return Ok(Some(()));
            }
        }

        // LD Rd, Y: 1000 000d dddd 1000
        // ST Y, Rr: 1000 001r rrrr 1000
        // LD Rd, Z: 1000 000d dddd 0000
        // ST Z, Rr: 1000 001r rrrr 0000
        if (op & 0xD208) == 0x8000 || (op & 0xD208) == 0x8008 {
            // might overlap LDD — already handled with q bits
        }
        if (op & 0xFE0F) == 0x8008 {
            // LD Rd, Y (q=0 form without q bits) — actually 1000 000d dddd 1000
            let rd = ((op >> 4) & 0x1F) as usize;
            let y = u16::from_le_bytes([self.r[28], self.r[29]]);
            self.r[rd] = self.data_read(y, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        if (op & 0xFE0F) == 0x8208 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let y = u16::from_le_bytes([self.r[28], self.r[29]]);
            self.data_write(y, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        if (op & 0xFE0F) == 0x8000 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.r[rd] = self.data_read(z, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        if (op & 0xFE0F) == 0x8200 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.data_write(z, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        Ok(None)
    }
}
