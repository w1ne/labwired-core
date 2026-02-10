// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;

/// STM32F1 compatible I2C peripheral (Master mode only)
#[derive(Debug, Default, serde::Serialize)]
pub struct I2c {
    cr1: u16,
    cr2: u16,
    oar1: u16,
    oar2: u16,
    dr: u16,
    sr1: u16,
    sr2: u16,
    ccr: u16,
    trise: u16,
}

impl I2c {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_reg(&self, offset: u64) -> u16 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.oar1,
            0x0C => self.oar2,
            0x10 => self.dr,
            0x14 => self.sr1,
            0x18 => self.sr2,
            0x1C => self.ccr,
            0x20 => self.trise,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u16) {
        match offset {
            0x00 => {
                self.cr1 = value;
                if (value & 0x0100) != 0 {
                    // START Generation
                    self.sr1 |= 0x01; // Set SB (Start Bit)
                }
                if (value & 0x0200) != 0 {
                    // STOP Generation
                    self.sr1 |= 0x10; // Set STOPF
                }
            }
            0x04 => self.cr2 = value,
            0x08 => self.oar1 = value,
            0x0C => self.oar2 = value,
            0x10 => {
                self.dr = value & 0xFF;
                // Simplified master TX: clear ADDR if it was set, set TXE
                if (self.sr1 & 0x02) != 0 {
                    self.sr1 &= !0x02; // Clear ADDR
                }
                self.sr1 |= 0x80; // Set TXE
                self.sr1 |= 0x04; // Set BTF
            }
            0x14 => self.sr1 = value, // Some bits are clear by write?
            0x18 => self.sr2 = value,
            0x1C => self.ccr = value,
            0x20 => self.trise = value,
            _ => {}
        }
    }
}

impl crate::Peripheral for I2c {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        // Registers are 16-bit but aligned to 32-bit boundaries

        let mut reg_val = self.read_reg(reg_offset);
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u16) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
