// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;

/// Mock TMP102 I2C Temperature Sensor.
///
/// In a real system, this would be on a separate bus. For simulation,
/// we provide a memory-mapped version to demonstrate register modeling.
#[derive(Debug, serde::Serialize)]
pub struct Tmp102 {
    pub temp: i16,   // 0x00 - Temperature (12-bit)
    pub config: u16, // 0x01 - Configuration
    pub t_low: i16,  // 0x02 - T_LOW
    pub t_high: i16, // 0x03 - T_HIGH

    #[serde(skip)]
    ticks: u32,
}

impl Tmp102 {
    pub fn new() -> Self {
        Self {
            temp: 0x190, // 25.0°C
            config: 0x60A0,
            t_low: 0x4B0,  // 75°C
            t_high: 0x500, // 80°C
            ticks: 0,
        }
    }

    fn read_reg(&self, offset: u64) -> u16 {
        match offset {
            0x00 => self.temp as u16,
            0x04 => self.config,
            0x08 => self.t_low as u16,
            0x0C => self.t_high as u16,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u16) {
        match offset {
            0x04 => self.config = value,
            0x08 => self.t_low = value as i16,
            0x0C => self.t_high = value as i16,
            _ => {}
        }
    }
}

impl Default for Tmp102 {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Tmp102 {
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
        reg_val |= (value as u16) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.ticks += 1;
        // Simulate slight temperature drift every 1000 ticks
        if self.ticks >= 1000 {
            self.ticks = 0;
            self.temp = self.temp.wrapping_add(1);
        }

        PeripheralTickResult {
            irq: false,
            cycles: 1,
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
