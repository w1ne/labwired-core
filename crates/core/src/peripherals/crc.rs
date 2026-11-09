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
//! Engine: MSB-first CRC of CR.POLYSIZE bits (32/16/8/7) with the
//! programmable POL, fed by the size of each DR store (byte, half-word or
//! word). CR.REV_IN / REV_OUT (bits 6:5 / 7) are stored but NOT modelled:
//! the data and the result are never bit-reversed.

use crate::SimResult;

fn full_idr_mask() -> u32 {
    0xFFFF_FFFF
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Crc {
    dr: u32,
    idr: u32,
    cr: u32,
    init: u32,
    pol: u32,
    /// Implemented width of the IDR scratch register: 8-bit on F0/F1/L0
    /// (mask 0xFF), 32-bit on F2+/L4+ (mask 0xFFFF_FFFF). Set via
    /// [`Crc::with_idr_width`]. Transient config — serde-skipped so the
    /// snapshot format is unchanged; defaults to 32-bit on resume.
    #[serde(skip, default = "full_idr_mask")]
    idr_mask: u32,
}

impl Crc {
    pub fn new() -> Self {
        Self {
            dr: 0xFFFF_FFFF,
            idr: 0,
            cr: 0,
            init: 0xFFFF_FFFF,
            pol: 0x04C1_1DB7,
            idr_mask: 0xFFFF_FFFF,
        }
    }

    /// Set the IDR implemented width (8 → mask 0xFF, ≥32 → 0xFFFF_FFFF).
    /// STM32F1/F0/L0 have an 8-bit IDR (RM0008 §6.4.2); F2+/L4+ have 32-bit.
    pub fn with_idr_width(mut self, bits: u8) -> Self {
        self.idr_mask = if bits >= 32 {
            0xFFFF_FFFF
        } else {
            (1u32 << bits) - 1
        };
        self
    }

    /// Width of the CRC, from CR.POLYSIZE: 32, 16, 8 or 7 bits.
    fn poly_size(&self) -> u32 {
        match (self.cr >> 3) & 0x3 {
            0 => 32,
            1 => 16,
            2 => 8,
            _ => 7,
        }
    }

    /// Feed the low `width` bits of `value` (the size of the DR write: 8, 16
    /// or 32), MSB first, through the CR.POLYSIZE-bit CRC (RM0385 §11.3.2:
    /// "the data size can be dynamically adjusted ... a byte, half-word or
    /// word"). Bit-at-a-time, so any data size works with any polynomial size.
    fn feed(&mut self, value: u32, width: u32) {
        let size = self.poly_size();
        let mask = if size == 32 {
            u32::MAX
        } else {
            (1u32 << size) - 1
        };
        let pol = self.pol & mask;
        let mut crc = self.dr & mask;
        for i in (0..width).rev() {
            let bit = (value >> i) & 1;
            let top = (crc >> (size - 1)) & 1;
            crc = (crc << 1) & mask;
            if top ^ bit != 0 {
                crc ^= pol;
            }
        }
        self.dr = crc;
    }
}

impl Default for Crc {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Crc {
    // Inert walk: register bank (CRC computed on write); tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let val = match reg {
            0x00 => self.dr,
            0x04 => self.idr,
            0x08 => self.cr,
            0x10 => self.init,
            0x14 => self.pol,
            _ => {
                crate::census_reg!("crc:Crc", reg, "read");
                0
            }
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
            _ => {
                crate::census_reg!("crc:Crc", reg, "write");
                0
            }
        };
        let mask: u32 = 0xFF << (byte * 8);
        let new = (cur & !mask) | ((value as u32) << (byte * 8));
        match reg {
            // A byte store to DR feeds 8 bits of data (HAL's
            // CRC_INPUTDATA_FORMAT_BYTES path); it never overwrites the CRC.
            0x00 => self.feed(u32::from(value), 8),
            0x04 => self.idr = new & self.idr_mask,
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
            _ => {
                crate::census_reg!("crc:Crc", reg, "write");
            }
        }
        Ok(())
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        match offset {
            // A half-word store to DR feeds 16 bits.
            0x00 => {
                self.feed(u32::from(value), 16);
                Ok(())
            }
            _ => {
                self.write(offset, value as u8)?;
                self.write(offset + 1, (value >> 8) as u8)
            }
        }
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            0x00 => self.feed(value, 32),
            0x04 => self.idr = value & self.idr_mask,
            0x08 => {
                self.cr = value & 0x000000FF;
                if (self.cr & 1) != 0 {
                    self.dr = self.init;
                    self.cr &= !1;
                }
            }
            0x10 => self.init = value,
            0x14 => self.pol = value,
            _ => {
                crate::census_reg!("crc:Crc", offset, "write");
            }
        }
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::Crc;
    use crate::Peripheral;

    #[test]
    fn idr_width_masks_on_write() {
        // 8-bit IDR (F0/F1/L0): only the low byte survives. Silicon-verified on
        // the bench F103 (stm32f1_exec_oracle::crc_idr_is_8bit_on_f1).
        let mut f1 = Crc::new().with_idr_width(8);
        f1.write_u32(0x04, 0x1234_5678).unwrap();
        assert_eq!(f1.read_u32(0x04).unwrap(), 0x0000_0078);

        // 32-bit IDR (F2+/L4+, the default): the full word is retained.
        let mut l4 = Crc::new();
        l4.write_u32(0x04, 0x1234_5678).unwrap();
        assert_eq!(l4.read_u32(0x04).unwrap(), 0x1234_5678);
    }

    #[test]
    fn word_write_is_crc32_mpeg2() {
        let mut c = Crc::new();
        c.write_u32(0x00, 0x1234_5678).unwrap();
        assert_eq!(c.read_u32(0x00).unwrap(), 0xDF8A_8A2B);
    }

    #[test]
    fn byte_and_halfword_writes_feed_their_own_width() {
        // Four byte stores of 0x12 0x34 0x56 0x78 = one word store of
        // 0x12345678 (MSB first).
        let mut c = Crc::new();
        for b in [0x12u8, 0x34, 0x56, 0x78] {
            c.write(0x00, b).unwrap();
        }
        assert_eq!(c.read_u32(0x00).unwrap(), 0xDF8A_8A2B);
        let mut h = Crc::new();
        h.write_u16(0x00, 0x1234).unwrap();
        h.write_u16(0x00, 0x5678).unwrap();
        assert_eq!(h.read_u32(0x00).unwrap(), 0xDF8A_8A2B);
    }

    #[test]
    fn sixteen_bit_polynomial_over_bytes_is_crc16_ccitt_false() {
        // CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF): "123456789" -> 0x29B1.
        let mut c = Crc::new();
        c.write_u32(0x14, 0x1021).unwrap();
        c.write_u32(0x10, 0xFFFF).unwrap();
        c.write_u32(0x08, (0b01 << 3) | 1).unwrap(); // POLYSIZE 16 | RESET
        for b in b"123456789" {
            c.write(0x00, *b).unwrap();
        }
        assert_eq!(c.read_u32(0x00).unwrap(), 0x29B1);
    }
}
