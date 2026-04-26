// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtiRegisterLayout {
    /// STM32F1 / F4-class: single bank, 20 lines, registers at 0x00..0x14.
    #[default]
    Stm32F1,
    /// STM32L4: two banks (40 lines total). Bank1 at 0x00..0x14, bank2 at 0x20..0x34.
    /// IMR1 line layout matches F1 for lines 0..19; lines 20..31 are L4-specific
    /// (RTC alarm, USB wakeup, etc); bank2 covers lines 32..39.
    Stm32L4,
}

impl FromStr for ExtiRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32l4" | "l4" => Ok(Self::Stm32L4),
            _ => Err(format!(
                "unsupported EXTI register layout '{}'; supported: stm32f1, stm32l4",
                value
            )),
        }
    }
}

/// External Interrupt/Event Controller (EXTI).
///
/// Models both F1-style (single 20-line bank) and L4-style (two banks, 40 lines).
/// L4 bank 2 lives at offsets 0x20..0x34 (mirroring bank 1 layout) and only the
/// low 8 bits are valid — lines 32..39 are dedicated to LPTIM/COMP/I2C wakeup.
#[derive(Debug, serde::Serialize)]
pub struct Exti {
    layout: ExtiRegisterLayout,
    pub imr1: u32,
    pub emr1: u32,
    pub rtsr1: u32,
    pub ftsr1: u32,
    pub swier1: u32,
    pub pr1: u32,
    pub imr2: u32,
    pub emr2: u32,
    pub rtsr2: u32,
    pub ftsr2: u32,
    pub swier2: u32,
    pub pr2: u32,
}

impl Default for Exti {
    fn default() -> Self {
        Self::new()
    }
}

impl Exti {
    pub fn new() -> Self {
        Self::new_with_layout(ExtiRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: ExtiRegisterLayout) -> Self {
        Self {
            layout,
            imr1: 0,
            emr1: 0,
            rtsr1: 0,
            ftsr1: 0,
            swier1: 0,
            pr1: 0,
            imr2: 0,
            emr2: 0,
            rtsr2: 0,
            ftsr2: 0,
            swier2: 0,
            pr2: 0,
        }
    }

    pub fn trigger_line(&mut self, line: u8) {
        match line {
            0..=31 => self.pr1 |= 1u32 << line,
            32..=39 if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => {
                self.pr2 |= 1u32 << (line - 32);
            }
            _ => {}
        }
    }

    fn bank1_mask(&self) -> u32 {
        match self.layout {
            ExtiRegisterLayout::Stm32F1 => 0x000F_FFFF, // 20 lines
            ExtiRegisterLayout::Stm32L4 => 0xFFFF_FFFF, // full word for bank 1
        }
    }

    fn bank2_mask(&self) -> u32 {
        match self.layout {
            ExtiRegisterLayout::Stm32F1 => 0,
            ExtiRegisterLayout::Stm32L4 => 0x0000_00FF, // lines 32..39
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.imr1,
            0x04 => self.emr1,
            0x08 => self.rtsr1,
            0x0C => self.ftsr1,
            0x10 => self.swier1,
            0x14 => self.pr1,
            0x20 if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => self.imr2,
            0x24 if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => self.emr2,
            0x28 if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => self.rtsr2,
            0x2C if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => self.ftsr2,
            0x30 if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => self.swier2,
            0x34 if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => self.pr2,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let m1 = self.bank1_mask();
        let m2 = self.bank2_mask();
        match offset {
            0x00 => self.imr1 = value & m1,
            0x04 => self.emr1 = value & m1,
            0x08 => self.rtsr1 = value & m1,
            0x0C => self.ftsr1 = value & m1,
            0x10 => {
                let diff = (self.swier1 ^ value) & value & m1;
                self.swier1 = value & m1;
                self.pr1 |= diff;
            }
            0x14 => self.pr1 &= !(value & m1), // rc_w1
            0x20 if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => self.imr2 = value & m2,
            0x24 if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => self.emr2 = value & m2,
            0x28 if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => self.rtsr2 = value & m2,
            0x2C if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => self.ftsr2 = value & m2,
            0x30 if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => {
                let diff = (self.swier2 ^ value) & value & m2;
                self.swier2 = value & m2;
                self.pr2 |= diff;
            }
            0x34 if matches!(self.layout, ExtiRegisterLayout::Stm32L4) => {
                self.pr2 &= !(value & m2)
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
        let mut explicit_irqs: Option<Vec<u32>> = None;
        let active1 = self.pr1 & self.imr1;
        let active2 = self.pr2 & self.imr2;

        if active1 != 0 || active2 != 0 {
            let mut irqs = Vec::new();
            // Lines 0-4 (EXTI0..EXTI4) -> NVIC IRQ 6..10
            for i in 0..5 {
                if (active1 & (1 << i)) != 0 {
                    irqs.push(6 + i);
                }
            }
            if (active1 & 0x0000_03E0) != 0 {
                irqs.push(23); // EXTI9_5
            }
            if (active1 & 0x0000_FC00) != 0 {
                irqs.push(40); // EXTI15_10
            }
            // L4 bank-2 line -> NVIC IRQ map per RM0351 §13.3.
            // Most bank-2 lines are wakeup events that re-trigger their
            // peripheral's existing NVIC IRQ when the chip exits Stop mode.
            // Mapping covers the common firmware paths; lines without an
            // entry here are tracked at the register level but don't
            // synthesize an IRQ until a specific firmware needs the route.
            if matches!(self.layout, ExtiRegisterLayout::Stm32L4) {
                let line35 = (active2 >> (35 - 32)) & 1 != 0; // LPUART1 wakeup
                let line36 = (active2 >> (36 - 32)) & 1 != 0; // I2C1 wakeup
                let line37 = (active2 >> (37 - 32)) & 1 != 0; // I2C2 wakeup
                let line38 = (active2 >> (38 - 32)) & 1 != 0; // I2C3 wakeup
                let line39 = (active2 >> (39 - 32)) & 1 != 0; // USART1 wakeup
                if line35 { irqs.push(70); }
                if line36 { irqs.push(31); }
                if line37 { irqs.push(33); }
                if line38 { irqs.push(72); }
                if line39 { irqs.push(37); }
            }
            explicit_irqs = Some(irqs);
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

#[cfg(test)]
mod tests {
    use super::{Exti, ExtiRegisterLayout};
    use crate::Peripheral;

    fn poke(exti: &mut Exti, off: u64, val: u32) {
        for i in 0..4 {
            exti.write(off + i, ((val >> (i * 8)) & 0xFF) as u8).unwrap();
        }
    }

    #[test]
    fn l4_bank2_line35_routes_to_lpuart1_irq() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32L4);
        // Arm IMR2 line 3 (= EXTI line 35), then trigger via SWIER2.
        poke(&mut e, 0x20, 1 << 3);
        poke(&mut e, 0x30, 1 << 3);
        let r = e.tick();
        let irqs = r.explicit_irqs.expect("expected IRQ list");
        assert!(irqs.contains(&70), "LPUART1 IRQ 70 should fire, got {irqs:?}");
    }

    #[test]
    fn l4_bank2_line38_routes_to_i2c3_irq() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32L4);
        poke(&mut e, 0x20, 1 << 6);
        poke(&mut e, 0x30, 1 << 6);
        let r = e.tick();
        let irqs = r.explicit_irqs.expect("expected IRQ list");
        assert!(irqs.contains(&72), "I2C3 IRQ 72 should fire, got {irqs:?}");
    }

    #[test]
    fn f1_layout_does_not_synth_bank2_irqs() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32F1);
        poke(&mut e, 0x20, 1 << 3); // bank 2 doesn't even exist on F1
        poke(&mut e, 0x30, 1 << 3);
        let r = e.tick();
        // No bank-2 -> no IRQ
        assert!(r.explicit_irqs.is_none() || r.explicit_irqs.unwrap().is_empty());
    }
}

