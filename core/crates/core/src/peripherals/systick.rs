// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;

/// Mocked SysTick Timer peripheral
/// Standard address: 0xE000_E010
#[derive(Debug, Default, serde::Serialize)]
pub struct Systick {
    csr: u32,
    rvr: u32,
    cvr: u32,
    calib: u32,
}

impl Systick {
    pub fn new() -> Self {
        Self {
            csr: 0,
            rvr: 0,
            cvr: 0,
            calib: 0x4000_0000, // No reference clock, no skew
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.csr,
            0x04 => self.rvr,
            0x08 => self.cvr,
            0x0C => self.calib,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                self.csr = value & 0x7;
            }
            0x04 => {
                self.rvr = value & 0x00FF_FFFF;
            }
            0x08 => {
                self.cvr = 0;
                self.csr &= !0x10000;
            }
            _ => {}
        }
    }
}

impl crate::Peripheral for Systick {
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

        // Modify byte
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        if (self.csr & 0x1) == 0 {
            return crate::PeripheralTickResult {
                irq: false,
                cycles: 0,
                ..Default::default()
            };
        }

        if self.cvr == 0 {
            self.cvr = self.rvr;
            self.csr |= 0x10000;
            crate::PeripheralTickResult {
                irq: (self.csr & 0x2) != 0,
                cycles: 1,
                ..Default::default()
            }
        } else {
            self.cvr -= 1;
            crate::PeripheralTickResult {
                irq: false,
                cycles: 1,
                ..Default::default()
            }
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
