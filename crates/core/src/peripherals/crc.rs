// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! CRC unit — STM32 layout (F1, F4, L4, H5 share the same register map).
//!
//! Five registers:
//!   0x00 DR     R/W  — data input and CRC result.
//!   0x04 IDR    R/W  — independent data register (8-bit on F1, 32-bit on L4).
//!   0x08 CR     R/W  — control register: RESET (bit 0), POLYSIZE (bits 4:3).
//!   0x10 INIT   R/W  — programmable initial CRC value (L4+ only).
//!   0x14 POL    R/W  — programmable polynomial (L4+ only).
//!
//! Reset state on real NUCLEO-L476RG silicon: DR = 0xFFFFFFFF,
//! IDR = 0, CR = 0, INIT = 0xFFFFFFFF, POL = 0x04C11DB7 (Ethernet
//! CRC-32 polynomial, the default).
//!
//! Engine: standard CRC-32, MSB-first, no reflection (input/output
//! reversed bits stay off unless firmware sets CR bits 5/6/7).

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Crc {
    dr: u32,
    idr: u32,
    cr: u32,
    init: u32,
    pol: u32,
}

impl Crc {
    pub fn new() -> Self {
        Self {
            dr: 0xFFFF_FFFF,
            idr: 0,
            cr: 0,
            init: 0xFFFF_FFFF,
            pol: 0x04C1_1DB7,
        }
    }

    fn step32(&mut self, value: u32) {
        // CR.POLYSIZE selects 7/8/16/32-bit polynomial. Default 32-bit.
        let poly_size = match (self.cr >> 3) & 0x3 {
            0 => 32,
            1 => 16,
            2 => 8,
            _ => 7,
        };
        let mut crc = self.dr;
        let bits = poly_size;
        let high_bit: u32 = 1u32 << (bits - 1);
        let poly_mask: u32 = if bits == 32 { 0xFFFF_FFFF } else { (1u32 << bits) - 1 };

        // Feed value MSB-first, 32 bits at a time.
        crc ^= value;
        for _ in 0..bits {
            if (crc & high_bit) != 0 {
                crc = ((crc << 1) ^ self.pol) & poly_mask;
            } else {
                crc = (crc << 1) & poly_mask;
            }
        }
        self.dr = crc;
    }
}

impl Default for Crc { fn default() -> Self { Self::new() } }

impl crate::Peripheral for Crc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let val = match reg {
            0x00 => self.dr,
            0x04 => self.idr,
            0x08 => self.cr,
            0x10 => self.init,
            0x14 => self.pol,
            _ => 0,
        };
        Ok(((val >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        // Read-modify-write the full register, then handle semantics.
        let cur = match reg {
            0x00 => self.dr,
            0x04 => self.idr,
            0x08 => self.cr,
            0x10 => self.init,
            0x14 => self.pol,
            _ => 0,
        };
        let mask: u32 = 0xFF << (byte * 8);
        let new = (cur & !mask) | ((value as u32) << (byte * 8));
        // For DR, the engine reacts only on full-word writes (real
        // hardware behaves the same — sub-word writes feed less data
        // through the polynomial). The trick is that a 32-bit STR
        // through the byte-write fallback completes the word on the
        // last byte; we feed the value once at byte 3.
        match reg {
            0x00 => {
                if byte == 3 {
                    self.step32(new);
                } else {
                    self.dr = new;
                }
            }
            0x04 => self.idr = new,
            0x08 => {
                self.cr = new & 0x000000FF;
                if (self.cr & 1) != 0 {
                    // RESET: reload DR from INIT, clear bit so reading
                    // back the CR shows it cleared (HAL polls this).
                    self.dr = self.init;
                    self.cr &= !1;
                }
            }
            0x10 => self.init = new,
            0x14 => self.pol = new,
            _ => {}
        }
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            0x00 => {
                // DR write: feed the polynomial engine with the new word.
                self.step32(value);
            }
            0x04 => self.idr = value,
            0x08 => {
                self.cr = value & 0x000000FF;
                if (self.cr & 1) != 0 {
                    self.dr = self.init;
                    self.cr &= !1;
                }
            }
            0x10 => self.init = value,
            0x14 => self.pol = value,
            _ => {}
        }
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
