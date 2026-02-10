// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;

/// Basic STM32 General Purpose Timer (TIM2-TIM5 compatible)
#[derive(Debug, Default, serde::Serialize)]
pub struct Timer {
    cr1: u32,
    dier: u32,
    sr: u32,
    cnt: u32,
    psc: u32,
    arr: u32,

    // Internal state
    psc_cnt: u32,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            arr: 0xFFFF, // Default reset value
            ..Default::default()
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x0C => self.dier,
            0x10 => self.sr,
            0x24 => self.cnt,
            0x28 => self.psc,
            0x2C => self.arr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr1 = value & 0x3FF,  // Support only basic bits
            0x0C => self.dier = value & 0x5F,  // Update Interrupt Enable (bit 0)
            0x10 => self.sr = value & 0x1FFFF, // Update interrupt flag (bit 0)
            0x24 => self.cnt = value & 0xFFFF,
            0x28 => self.psc = value & 0xFFFF,
            0x2C => self.arr = value & 0xFFFF,
            _ => {}
        }
    }
}

impl crate::Peripheral for Timer {
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

    fn tick(&mut self) -> crate::PeripheralTickResult {
        // Counter Enable (bit 0)
        if (self.cr1 & 0x1) == 0 {
            return crate::PeripheralTickResult {
                irq: false,
                cycles: 0,
                ..Default::default()
            };
        }

        self.psc_cnt = self.psc_cnt.wrapping_add(1);
        if self.psc_cnt > self.psc {
            self.psc_cnt = 0;
            self.cnt = self.cnt.wrapping_add(1);

            if self.cnt > self.arr {
                self.cnt = 0;
                self.sr |= 1; // Set UIF (Update Interrupt Flag)

                // Return true if Update Interrupt Enable (UIE) is set
                return crate::PeripheralTickResult {
                    irq: (self.dier & 1) != 0,
                    cycles: 1,
                    ..Default::default()
                };
            }
        }

        crate::PeripheralTickResult {
            irq: false,
            cycles: 1,
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
