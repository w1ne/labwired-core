// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, SimResult};
use std::any::Any;

/// STM32F1 Alternate Function I/O (AFIO)
#[derive(Debug, Default, serde::Serialize)]
pub struct Afio {
    pub evcr: u32,
    pub mapr: u32,
    pub exticr: [u32; 4], // EXTICR1, EXTICR2, EXTICR3, EXTICR4
    pub mapr2: u32,
}

impl Afio {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the GPIO port index (0=A, 1=B, etc.) for a given EXTI line (0-15)
    pub fn get_exti_mapping(&self, line: u8) -> u8 {
        if line >= 16 {
            return 0;
        }
        let reg_idx = (line / 4) as usize;
        let shift = (line % 4) * 4;
        ((self.exticr[reg_idx] >> shift) & 0xF) as u8
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.evcr,
            0x04 => self.mapr,
            0x08 => self.exticr[0],
            0x0C => self.exticr[1],
            0x10 => self.exticr[2],
            0x14 => self.exticr[3],
            0x1C => self.mapr2,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.evcr = value,
            0x04 => self.mapr = value,
            0x08 => self.exticr[0] = value,
            0x0C => self.exticr[1] = value,
            0x10 => self.exticr[2] = value,
            0x14 => self.exticr[3] = value,
            0x1C => self.mapr2 = value,
            _ => {}
        }
    }
}

impl Peripheral for Afio {
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

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
