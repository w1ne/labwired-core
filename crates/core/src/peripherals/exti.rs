// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;

/// STM32F1 External Interrupt/Event Controller (EXTI)
#[derive(Debug, Default, serde::Serialize)]
pub struct Exti {
    pub imr: u32,   // 0x00 - Interrupt mask register
    pub emr: u32,   // 0x04 - Event mask register
    pub rtsr: u32,  // 0x08 - Rising trigger selection register
    pub ftsr: u32,  // 0x0C - Falling trigger selection register
    pub swier: u32, // 0x10 - Software interrupt event register
    pub pr: u32,    // 0x14 - Pending register
}

impl Exti {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trigger_line(&mut self, line: u8) {
        if line < 20 {
            self.pr |= 1 << line;
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.imr,
            0x04 => self.emr,
            0x08 => self.rtsr,
            0x0C => self.ftsr,
            0x10 => self.swier,
            0x14 => self.pr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.imr = value & 0x7FFFF,
            0x04 => self.emr = value & 0x7FFFF,
            0x08 => self.rtsr = value & 0x7FFFF,
            0x0C => self.ftsr = value & 0x7FFFF,
            0x10 => {
                let diff = (self.swier ^ value) & value;
                self.swier = value & 0x7FFFF;
                // Writing 1 to SWIER triggers the interrupt if imr bit is set?
                // Actually SWIER just sets the PR bit if IMR is set.
                self.pr |= diff;
            }
            0x14 => {
                // PR is rc_w1: writing 1 clears the bit
                self.pr &= !value;
            }
            _ => {}
        }
    }
}

impl Peripheral for Exti {
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

    fn tick(&mut self) -> PeripheralTickResult {
        // EXTI interrupts are triggered when PR bits are set AND corresponding IMR bits are set.
        let mut explicit_irqs = Vec::new();
        let active = self.pr & self.imr;

        if active != 0 {
            // Map lines 0-4
            for i in 0..5 {
                if (active & (1 << i)) != 0 {
                    explicit_irqs.push(6 + i); // EXTI0..4 -> IRQ 6..10
                }
            }
            // Map lines 5-9
            if (active & 0x03E0) != 0 {
                explicit_irqs.push(23); // EXTI9_5 -> IRQ 23
            }
            // Map lines 10-15
            if (active & 0xFC00) != 0 {
                explicit_irqs.push(40); // EXTI15_10 -> IRQ 40
            }
        }

        PeripheralTickResult {
            explicit_irqs,
            ..Default::default()
        }
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
