// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;

/// Minimal RCC (Reset and Clock Control) peripheral
/// Base address: 0x4002_1000
#[derive(Debug, Default, serde::Serialize)]
pub struct Rcc {
    apb1enr: u32,
    apb2enr: u32,
}

impl Rcc {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x18 => self.apb2enr,
            0x1C => self.apb1enr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x18 => self.apb2enr = value,
            0x1C => self.apb1enr = value,
            _ => {}
        }
    }
}

impl crate::Peripheral for Rcc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);

        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
