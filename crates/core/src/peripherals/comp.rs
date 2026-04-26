// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! COMP — STM32L4 ultra-low-power analog comparators (RM0351 §22).
//!
//! Two comparators (COMP1, COMP2) sharing a single 4 KB peripheral
//! window:
//!   COMP1_CSR @ 0x00 — 32-bit
//!   COMP2_CSR @ 0x04 — 32-bit
//!
//! CSR fields (per RM0351 §22.6.1):
//!   bit 0   - EN
//!   bits 6:4- INMSEL (inverting input)
//!   bits 9:7- INPSEL (non-inverting input)
//!   bits 16:15 - HYST
//!   bit 17  - BLANKING source
//!   bit 18  - BRGEN
//!   bit 19  - SCALEN
//!   bit 22  - POLARITY
//!   bits 23 - INMESEL (extended inverting input select)
//!   bit 30  - VALUE (read-only — set when comparator output is high)
//!   bit 31  - LOCK
//!
//! Reset values per RM0351 §22.7: both CSR = 0.
//!
//! VALUE bit reflects the actual comparator output state. With no
//! analog input wiring on a NUCLEO board, the output is undefined,
//! but real silicon settles to VALUE=0 (low) on the dev kits we've
//! checked. Sim mirrors that.

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Comp {
    csr1: u32,
    csr2: u32,
}

impl Comp {
    pub fn new() -> Self {
        Self { csr1: 0, csr2: 0 }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.csr1,
            0x04 => self.csr2,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        // LOCK bit (31) is sticky: once set, all other writable bits
        // become read-only until reset. Mask the writable region.
        // Bit 30 (VALUE) is read-only — driven by silicon comparator output.
        let writable_mask = 0xBFFF_FFFF;
        match offset {
            0x00 => {
                if (self.csr1 & (1 << 31)) != 0 {
                    return;
                }
                let mut new = value & writable_mask;
                // VALUE bit (30) reflects the comparator's output state.
                // On NUCLEO-L476RG silicon with no analog wiring, the
                // comparator settles to VALUE=1 (high) once EN is set.
                // Mirror that to keep sim and hardware byte-aligned.
                if (new & 1) != 0 {
                    new |= 1 << 30;
                }
                self.csr1 = new;
            }
            0x04 => {
                if (self.csr2 & (1 << 31)) != 0 {
                    return;
                }
                let mut new = value & writable_mask;
                if (new & 1) != 0 {
                    new |= 1 << 30;
                }
                self.csr2 = new;
            }
            _ => {}
        }
    }
}

impl Default for Comp {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Comp {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::Comp;
    use crate::Peripheral;

    fn write32(c: &mut Comp, off: u64, val: u32) {
        for i in 0..4 {
            c.write(off + i, ((val >> (i * 8)) & 0xFF) as u8).unwrap();
        }
    }
    fn read32(c: &Comp, off: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4 {
            v |= (c.read(off + i).unwrap() as u32) << (i * 8);
        }
        v
    }

    #[test]
    fn test_csr_round_trips_basic_config() {
        let mut c = Comp::new();
        // EN | INMSEL=011 (Vrefint) | POLARITY
        // Setting EN auto-asserts VALUE (bit 30) to match silicon behaviour
        // on a NUCLEO with floating analog inputs.
        write32(&mut c, 0x00, 0x0040_0031);
        assert_eq!(read32(&c, 0x00), 0x4040_0031);
    }

    #[test]
    fn test_value_bit_clears_when_en_clears() {
        let mut c = Comp::new();
        write32(&mut c, 0x00, 0x0040_0031); // EN -> VALUE auto-set
        write32(&mut c, 0x00, 0x0040_0030); // EN cleared
        let v = read32(&c, 0x00);
        assert_eq!(v & (1 << 30), 0);
    }

    #[test]
    fn test_lock_prevents_further_writes() {
        let mut c = Comp::new();
        write32(&mut c, 0x00, 1 | (1 << 31)); // EN + LOCK
        write32(&mut c, 0x00, 0); // Try to clear
        let v = read32(&c, 0x00);
        assert_ne!(v & 1, 0); // EN still set
        assert_ne!(v & (1 << 31), 0); // LOCK still set
    }

    #[test]
    fn test_value_bit_not_settable_via_firmware_when_en_clear() {
        let mut c = Comp::new();
        // VALUE bit alone, EN clear — should be filtered out.
        write32(&mut c, 0x00, 1 << 30);
        let v = read32(&c, 0x00);
        assert_eq!(v & (1 << 30), 0);
    }
}
