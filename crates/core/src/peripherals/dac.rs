// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! DAC peripheral — STM32 layout (F1, F4, L4, H5 share register map for the
//! basic 12-bit dual-channel DAC).
//!
//! Registers:
//!   0x00 CR        Control register
//!   0x04 SWTRIGR   Software trigger register
//!   0x08..0x18     Channel-1 holding registers (DHR12R1, DHR12L1, DHR8R1)
//!   0x14..0x20     Channel-2 holding registers
//!   0x20 DHR12RD   Dual 12-bit right-aligned
//!   0x28 DOR1      Channel-1 data output (read-only)
//!   0x2C DOR2      Channel-2 data output
//!   0x34 SR        Status register
//!
//! Reset values: all zero.

use crate::SimResult;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Dac {
    cr: u32,
    swtrigr: u32,
    dhr12r1: u32, dhr12l1: u32, dhr8r1: u32,
    dhr12r2: u32, dhr12l2: u32, dhr8r2: u32,
    dhr12rd: u32, dhr12ld: u32, dhr8rd: u32,
    dor1: u32, dor2: u32,
    sr: u32,
}

impl Dac {
    pub fn new() -> Self { Self::default() }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.swtrigr,
            0x08 => self.dhr12r1,
            0x0C => self.dhr12l1,
            0x10 => self.dhr8r1,
            0x14 => self.dhr12r2,
            0x18 => self.dhr12l2,
            0x1C => self.dhr8r2,
            0x20 => self.dhr12rd,
            0x24 => self.dhr12ld,
            0x28 => self.dor1,
            0x2C => self.dor2,
            0x34 => self.sr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = value,
            0x04 => self.swtrigr = value & 0x3,
            0x08 => {
                self.dhr12r1 = value & 0xFFF;
                // Channel-1 enabled? mirror to DOR1.
                if (self.cr & 1) != 0 {
                    self.dor1 = self.dhr12r1;
                }
            }
            0x0C => self.dhr12l1 = value,
            0x10 => self.dhr8r1 = value & 0xFF,
            0x14 => {
                self.dhr12r2 = value & 0xFFF;
                if (self.cr & (1 << 16)) != 0 {
                    self.dor2 = self.dhr12r2;
                }
            }
            0x18 => self.dhr12l2 = value,
            0x1C => self.dhr8r2 = value & 0xFF,
            0x20 => self.dhr12rd = value,
            0x24 => self.dhr12ld = value,
            0x28 | 0x2C => {} // DOR is read-only
            0x34 => self.sr = value & 0x3000_3000, // BWST/CAL flags W1C
            _ => {}
        }
    }
}

impl crate::Peripheral for Dac {
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
