// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Split out of `step_inner` in `cpu/avr.rs` (pure move, no behaviour
// change). See that file's `step_inner` and `exec/mod.rs` for the shared
// fetch/next-pc plumbing and the fixed original check order this respects.

use crate::cpu::avr::{Avr, AvrModel};
use crate::{Bus, SimResult};

/// Registers in the SBI/CBI range where a written 1 acts and a 0 does nothing:
/// PINB/PINC/PIND (toggle PORTx), TIFR0..2, PCIFR and EIFR (clear a flag).
/// AVRrc (ATtiny10): PINB 0x00, PCIFR 0x11 and EIFR 0x14 (register summary).
#[inline]
fn write_one_register(data_addr: u16, model: AvrModel) -> bool {
    if model.uses_avrxt_data_map() {
        matches!(data_addr, 0x03 | 0x07)
    } else if model.is_reduced_core() {
        matches!(data_addr, 0x00 | 0x11 | 0x14)
    } else if model.has_classic_pin_toggle() {
        // Classic tinyAVR PINx toggle PORTx on a written 1.
        match model {
            AvrModel::Attiny2313 | AvrModel::Attiny4313 => matches!(data_addr, 0x30 | 0x36 | 0x39),
            AvrModel::Attiny24 | AvrModel::Attiny44 | AvrModel::Attiny84 => {
                matches!(data_addr, 0x36 | 0x39)
            }
            AvrModel::Attiny13 | AvrModel::Attiny25 | AvrModel::Attiny45 | AvrModel::Attiny85 => {
                data_addr == 0x36
            }
            // ATtiny1634: PINC 0x27, PINB 0x2B, PINA 0x2F.
            AvrModel::Attiny1634 => matches!(data_addr, 0x27 | 0x2B | 0x2F),
            _ => false,
        }
    } else if model.has_classic_io_file() || model == AvrModel::Atmega128 {
        // Classic layout: 0x35..0x3B are PORTC..PORTA/DDRx/PINx (read-only
        // PINx), so a single-bit write there would clobber the latch.
        false
    } else if matches!(
        model,
        AvrModel::Atmega1281
            | AvrModel::Atmega2560
            | AvrModel::Atmega169
            | AvrModel::Atmega128rfa1
            | AvrModel::Atmega128rfr2
    ) {
        // Ports A-G: PINA..PING at 0x20, 0x23, .., 0x32.
        matches!(
            data_addr,
            0x20 | 0x23 | 0x26 | 0x29 | 0x2C | 0x2F | 0x32 | 0x35..=0x37 | 0x3B | 0x3C
        )
    } else if matches!(model, AvrModel::Atmega16m1 | AvrModel::Atmega64m1) {
        // The M1 parts add PINE at 0x2C.
        matches!(
            data_addr,
            0x23 | 0x26 | 0x29 | 0x2C | 0x35..=0x37 | 0x3B | 0x3C
        )
    } else {
        matches!(data_addr, 0x23 | 0x26 | 0x29 | 0x35..=0x37 | 0x3B | 0x3C)
    }
}

impl Avr {
    /// [`write_one_register`] plus every PINx of this part in SBI reach (the
    /// mega's PINA/PINE/PINF/PING, the 32U4's PINE/PINF) and the extra timer
    /// flag registers: TIFR3..5 on the ATmega1281/2560 (DS2549 register
    /// summary 0x18..0x1A), TIFR3/TIFR4 on the ATmega32U4.
    #[inline]
    fn sbi_write_one(&self, data_addr: u16) -> bool {
        write_one_register(data_addr, self.model)
            || (!self.model.uses_avrxt_data_map()
                && !self.model.is_reduced_core()
                && (self.is_pin_reg(data_addr)
                    || match self.model {
                        AvrModel::Atmega1281 | AvrModel::Atmega2560 => {
                            (0x38..=0x3A).contains(&data_addr)
                        }
                        AvrModel::Atmega32u4 => matches!(data_addr, 0x38 | 0x39),
                        _ => false,
                    }))
    }

    /// SBI, CBI.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_bitops_a(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        next: u32,
    ) -> SimResult<Option<()>> {
        // SBI
        if (op & 0xFF00) == 0x9A00 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let data_addr = self.io_data_address(a);
            // On the ATmega328P SBI/CBI "only operate on the specified bit":
            // on a write-one register (PINx toggles, interrupt flags clear)
            // a read-modify-write would hit every other bit that reads 1.
            let v = if self.sbi_write_one(data_addr) {
                1 << b
            } else {
                self.data_read(data_addr, bus)? | (1 << b)
            };
            self.data_write(data_addr, v, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // CBI
        if (op & 0xFF00) == 0x9800 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let data_addr = self.io_data_address(a);
            // Writing 0 to a write-one register bit does nothing.
            if !self.sbi_write_one(data_addr) {
                let v = self.data_read(data_addr, bus)? & !(1 << b);
                self.data_write(data_addr, v, bus)?;
            }
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// SWAP, ASR, LSR, ROR, BST, BLD.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_bitops_b(
        &mut self,
        op: u16,
        next: u32,
    ) -> SimResult<Option<()>> {
        // SWAP Rd: 1001 010d dddd 0010
        if (op & 0xFE0F) == 0x9402 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let v = self.r[rd];
            self.r[rd] = v.rotate_left(4);
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // ASR Rd: 1001 010d dddd 0101
        if (op & 0xFE0F) == 0x9405 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let a = self.r[rd];
            let res = ((a as i8) >> 1) as u8;
            self.r[rd] = res;
            self.set_c(a & 1 != 0);
            self.set_z(res);
            self.set_n(res);
            self.set_v(((res >> 7) ^ (a & 1)) != 0);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // LSR Rd: 1001 010d dddd 0110
        if (op & 0xFE0F) == 0x9406 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let a = self.r[rd];
            let c = a & 1 != 0;
            let res = a >> 1;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.sreg &= !0x04; // N = 0
            self.set_v(c); // V = N⊕C = C
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // ROR Rd: 1001 010d dddd 0111
        if (op & 0xFE0F) == 0x9407 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let a = self.r[rd];
            let c_in = self.sreg & 1;
            let c_out = a & 1 != 0;
            let res = (a >> 1) | (c_in << 7);
            self.r[rd] = res;
            self.set_c(c_out);
            self.set_z(res);
            self.set_n(res);
            let n = (res >> 7) & 1;
            self.set_v((n ^ (c_out as u8)) != 0);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        // BST Rr,b: 1111 101r rrrr 0bbb — copy bit b of Rr into SREG.T
        // Used by avr-libc __divmodsi4 (map()/division) before BRTC/BRTS.
        if (op & 0xFE08) == 0xFA00 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let b = (op & 0x07) as u8;
            self.set_flag_t((self.r[rr] >> b) & 1 != 0);
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // BLD Rd,b: 1111 100d dddd 0bbb — copy SREG.T into bit b of Rd
        if (op & 0xFE08) == 0xF800 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let b = (op & 0x07) as u8;
            if self.flag_t() {
                self.r[rd] |= 1 << b;
            } else {
                self.r[rd] &= !(1 << b);
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        Ok(None)
    }
}
