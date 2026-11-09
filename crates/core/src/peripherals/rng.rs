// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! RNG (random-number generator) peripheral — STM32L4 layout.
//!
//! Three registers: CR (control), SR (status), DR (data). Real silicon
//! samples thermal / shot noise; the simulator drives a linear-feedback
//! deterministic PRNG so survival tests are reproducible.
//!
//! Reset values verified against NUCLEO-L476RG silicon: CR = SR = 0,
//! DR untouched until firmware writes CR.RNGEN.

use crate::SimResult;
use std::cell::Cell;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Rng {
    cr: u32,
    sr: u32,
    /// xorshift32 state. A `DR` read draws the next word, and reads take
    /// `&self`, hence the `Cell`. Fixed seed, so runs are reproducible.
    lfsr: Cell<u32>,
}

impl Rng {
    pub fn new() -> Self {
        Self {
            cr: 0,
            sr: 0,
            lfsr: Cell::new(0xACE1_5EED),
        }
    }

    /// Next xorshift32 word (never zero: the state never reaches zero).
    fn draw(&self) -> u32 {
        let mut x = self.lfsr.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.lfsr.set(x);
        x
    }

    /// `DR` while `CR.RNGEN` is set draws a fresh word; `DRDY` stays set
    /// (the next word is ready at once). With RNGEN clear `DR` reads 0.
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.sr,
            0x08 if self.cr & 0x4 != 0 => self.draw(),
            0x08 => 0,
            _ => {
                crate::census_reg!("rng:Rng", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                // RNGEN, IE, CED, BOOST; DRDY follows RNGEN.
                self.cr = value & 0x0000_001D;
                if value & 0x4 != 0 {
                    self.sr |= 1;
                } else {
                    self.sr &= !1;
                }
            }
            0x04 => {
                // SR clear-on-write semantics for SEIS / CEIS error flags.
                let clearable: u32 = 0b0110_0000;
                self.sr &= !(value & clearable);
            }
            _ => {
                crate::census_reg!("rng:Rng", offset, "write");
            }
        }
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Rng {
    // Inert walk: register bank (output computed on read); tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    /// Byte reads of CR/SR return their bytes. `DR` is a 32-bit register
    /// (RM0376 §22.8.3: word access only); a byte read of it returns 0 and
    /// draws nothing.
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let v = match reg {
            0x00 => self.cr,
            0x04 => self.sr,
            _ => 0,
        };
        Ok(((v >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = match reg {
            0x00 => self.cr,
            0x04 => self.sr,
            _ => {
                crate::census_reg!("rng:Rng", reg, "write");
                0
            }
        };
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset))
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn each_dr_read_draws_a_new_word_only_while_enabled() {
        let mut rng = Rng::new();
        assert_eq!(rng.read_u32(0x08).unwrap(), 0);
        rng.write_u32(0x00, 0x4).unwrap();
        assert_eq!(rng.read_u32(0x04).unwrap() & 1, 1, "DRDY with RNGEN");
        let a = rng.read_u32(0x08).unwrap();
        let b = rng.read_u32(0x08).unwrap();
        assert!(a != 0 && b != 0 && a != b, "{a:#x} {b:#x}");
        assert_eq!(rng.read_u32(0x04).unwrap() & 1, 1, "DRDY stays set");
        rng.write_u32(0x00, 0).unwrap();
        assert_eq!(rng.read_u32(0x04).unwrap() & 1, 0);
        assert_eq!(rng.read_u32(0x08).unwrap(), 0);
    }
}
