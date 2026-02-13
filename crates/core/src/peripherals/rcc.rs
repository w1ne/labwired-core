// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RccRegisterLayout {
    #[default]
    Stm32F1,
    Stm32V2,
}

impl FromStr for RccRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32v2" | "v2" | "modern" | "stm32-modern" | "h5" | "stm32h5" => Ok(Self::Stm32V2),
            _ => Err(format!(
                "unsupported RCC register layout '{}'; supported: stm32f1, stm32v2",
                value
            )),
        }
    }
}

/// Minimal RCC (Reset and Clock Control) peripheral
/// with selectable register layout for clock-enable registers.
#[derive(Debug, Default, serde::Serialize)]
pub struct Rcc {
    layout: RccRegisterLayout,
    apb1enr: u32,
    apb2enr: u32,
}

impl Rcc {
    pub fn new() -> Self {
        Self::new_with_layout(RccRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: RccRegisterLayout) -> Self {
        Self {
            layout,
            apb1enr: 0,
            apb2enr: 0,
        }
    }

    fn apb2enr_offset(&self) -> u64 {
        match self.layout {
            RccRegisterLayout::Stm32F1 => 0x18,
            RccRegisterLayout::Stm32V2 => 0xA4, // APB2ENR on STM32H5-style RCC
        }
    }

    fn apb1enr_offset(&self) -> u64 {
        match self.layout {
            RccRegisterLayout::Stm32F1 => 0x1C,
            RccRegisterLayout::Stm32V2 => 0x9C, // APB1LENR on STM32H5-style RCC
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        if offset == self.apb2enr_offset() {
            return self.apb2enr;
        }
        if offset == self.apb1enr_offset() {
            return self.apb1enr;
        }
        0
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if offset == self.apb2enr_offset() {
            self.apb2enr = value;
            return;
        }
        if offset == self.apb1enr_offset() {
            self.apb1enr = value;
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

#[cfg(test)]
mod tests {
    use super::{Rcc, RccRegisterLayout};
    use crate::Peripheral;

    #[test]
    fn test_rcc_f1_offsets() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32F1);
        rcc.write(0x18, 0xAA).unwrap();
        rcc.write(0x1C, 0x55).unwrap();
        assert_eq!(rcc.read(0x18).unwrap(), 0xAA);
        assert_eq!(rcc.read(0x1C).unwrap(), 0x55);
    }

    #[test]
    fn test_rcc_v2_offsets() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);
        rcc.write(0xA4, 0xCC).unwrap();
        rcc.write(0x9C, 0x33).unwrap();
        assert_eq!(rcc.read(0xA4).unwrap(), 0xCC);
        assert_eq!(rcc.read(0x9C).unwrap(), 0x33);
        assert_eq!(rcc.read(0x18).unwrap(), 0x00);
    }
}
