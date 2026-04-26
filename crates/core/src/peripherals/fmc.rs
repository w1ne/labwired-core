// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! FMC — STM32L4 Flexible Memory Controller (RM0351 §13).
//!
//! On STM32L476RG the FMC drives external NOR flash / SRAM (banks 1-4)
//! at 0x6000_0000-0x6FFF_FFFF and PCCARD/NAND (bank 3) at 0x8000_0000.
//! The control window itself is at 0xA000_0000.
//!
//! Register layout (interleaved BCR/BTR per bank):
//!   BCR1   0x00 — bank 1 control
//!   BTR1   0x04 — bank 1 timing
//!   BCR2   0x08 — bank 2 control
//!   BTR2   0x0C — bank 2 timing
//!   BCR3   0x10
//!   BTR3   0x14
//!   BCR4   0x18
//!   BTR4   0x1C
//!   ...
//!   BWTR1  0x104  — bank 1 write timing
//!   BWTR2  0x10C
//!   BWTR3  0x114
//!   BWTR4  0x11C
//!
//! NAND/PCCARD slot (bank 3) lives at +0x80..+0x90:
//!   PCR    0x80 — control
//!   SR     0x84 — status (FEMPT etc)
//!   PMEM   0x88 — common-memory timing
//!   PATT   0x8C — attribute-memory timing
//!   ECCR   0x94 — ECC result (read-only)
//!
//! Reset values per RM0351 §13.5: BCR1 = 0x000030DB (defaults to NOR
//! enabled with default timings on a few L4 variants), all other
//! BCR/BTR/BWTR = 0x0FFF_FFFF, PCR = 0x18, SR = 0x40, ECCR = 0.
//!
//! BCR1 reset value confirmed against silicon for survival flow.

use crate::SimResult;
use std::collections::HashMap;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Fmc {
    bcr: [u32; 4],
    btr: [u32; 4],
    bwtr: [u32; 4],
    pcr: u32,
    sr: u32,
    pmem: u32,
    patt: u32,
    eccr: u32,
    /// Catch-all for register offsets we don't model individually.
    /// Real silicon has additional bank-specific NORSRAM / NAND / SDRAM
    /// registers that are rarely touched by survival firmware; pass-through
    /// keeps reads consistent with prior writes.
    extra: HashMap<u64, u32>,
}

impl Fmc {
    pub fn new() -> Self {
        Self {
            bcr: [
                // BCR1 reset = 0x000030DB on L4 (NOR-flash defaults)
                0x0000_30DB,
                0x0FFF_FFFF,
                0x0FFF_FFFF,
                0x0FFF_FFFF,
            ],
            btr: [0x0FFF_FFFF; 4],
            bwtr: [0x0FFF_FFFF; 4],
            pcr: 0x0000_0018,
            sr: 0x0000_0040,
            pmem: 0xFCFC_FCFC,
            patt: 0xFCFC_FCFC,
            eccr: 0,
            extra: HashMap::new(),
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.bcr[0],
            0x04 => self.btr[0],
            0x08 => self.bcr[1],
            0x0C => self.btr[1],
            0x10 => self.bcr[2],
            0x14 => self.btr[2],
            0x18 => self.bcr[3],
            0x1C => self.btr[3],
            0x80 => self.pcr,
            0x84 => self.sr,
            0x88 => self.pmem,
            0x8C => self.patt,
            0x94 => self.eccr,
            0x104 => self.bwtr[0],
            0x10C => self.bwtr[1],
            0x114 => self.bwtr[2],
            0x11C => self.bwtr[3],
            other => self.extra.get(&other).copied().unwrap_or(0),
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.bcr[0] = value,
            0x04 => self.btr[0] = value,
            0x08 => self.bcr[1] = value,
            0x0C => self.btr[1] = value,
            0x10 => self.bcr[2] = value,
            0x14 => self.btr[2] = value,
            0x18 => self.bcr[3] = value,
            0x1C => self.btr[3] = value,
            0x80 => self.pcr = value & 0x0007_FFFF,
            0x84 => {
                // SR is rc_w0 for ILS/IRS/IFS (interrupt status) and ro for FEMPT.
                // Match silicon: writing 0 clears ILS/IRS/IFS; FEMPT (bit 6)
                // stays asserted on a fresh peripheral with no NAND command
                // outstanding.
                self.sr = (value & 0x0F) | 0x40;
            }
            0x88 => self.pmem = value,
            0x8C => self.patt = value,
            0x94 => {} // ECCR is read-only
            0x104 => self.bwtr[0] = value,
            0x10C => self.bwtr[1] = value,
            0x114 => self.bwtr[2] = value,
            0x11C => self.bwtr[3] = value,
            other => {
                self.extra.insert(other, value);
            }
        }
    }
}

impl Default for Fmc {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Fmc {
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
    use super::Fmc;
    use crate::Peripheral;

    fn read32(f: &Fmc, off: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4 {
            v |= (f.read(off + i).unwrap() as u32) << (i * 8);
        }
        v
    }
    fn write32(f: &mut Fmc, off: u64, val: u32) {
        for i in 0..4 {
            f.write(off + i, ((val >> (i * 8)) & 0xFF) as u8).unwrap();
        }
    }

    #[test]
    fn test_reset_values_match_silicon() {
        let f = Fmc::new();
        assert_eq!(read32(&f, 0x00), 0x0000_30DB); // BCR1
        assert_eq!(read32(&f, 0x04), 0x0FFF_FFFF); // BTR1
        assert_eq!(read32(&f, 0x80), 0x0000_0018); // PCR
        assert_eq!(read32(&f, 0x84), 0x0000_0040); // SR
    }

    #[test]
    fn test_bcr_btr_round_trip() {
        let mut f = Fmc::new();
        write32(&mut f, 0x00, 0xABCD_1234);
        assert_eq!(read32(&f, 0x00), 0xABCD_1234);
        write32(&mut f, 0x04, 0x1234_5678);
        assert_eq!(read32(&f, 0x04), 0x1234_5678);
    }

    #[test]
    fn test_sr_fempt_bit_sticky() {
        let mut f = Fmc::new();
        write32(&mut f, 0x84, 0); // try to clear all
        assert_ne!(read32(&f, 0x84) & 0x40, 0); // FEMPT still set
    }
}
