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
    Stm32F4,
    Stm32V2,
}

impl FromStr for RccRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32f4" | "f4" => Ok(Self::Stm32F4),
            "stm32v2" | "v2" | "modern" | "stm32-modern" | "h5" | "stm32h5" => Ok(Self::Stm32V2),
            _ => Err(format!(
                "unsupported RCC register layout '{}'; supported: stm32f1, stm32f4, stm32v2",
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
    cr: u32,
    cfgr: u32,
    ahbenr: u32,
    apb1enr: u32,
    apb2enr: u32,
    ahbrstr: u32,
    apb1rstr: u32,
    apb2rstr: u32,
}

impl Rcc {
    const CR_HSION: u32 = 1 << 0;
    const CR_HSIRDY: u32 = 1 << 1;
    const CR_HSEON: u32 = 1 << 16;
    const CR_HSERDY: u32 = 1 << 17;
    const CR_PLLON: u32 = 1 << 24;
    const CR_PLLRDY: u32 = 1 << 25;

    pub fn new() -> Self {
        Self::new_with_layout(RccRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: RccRegisterLayout) -> Self {
        let mut rcc = Self {
            layout,
            cr: Self::CR_HSION,
            cfgr: 0,
            ahbenr: 0,
            apb1enr: 0,
            apb2enr: 0,
            ahbrstr: 0,
            apb1rstr: 0,
            apb2rstr: 0,
        };
        rcc.update_ready_flags();
        rcc
    }

    fn update_ready_flags(&mut self) {
        self.cr &= !(Self::CR_HSIRDY | Self::CR_HSERDY | Self::CR_PLLRDY);
        if (self.cr & Self::CR_HSION) != 0 {
            self.cr |= Self::CR_HSIRDY;
        }
        if (self.cr & Self::CR_HSEON) != 0 {
            self.cr |= Self::CR_HSERDY;
        }
        if (self.cr & Self::CR_PLLON) != 0 {
            self.cr |= Self::CR_PLLRDY;
        }
    }

    fn apb2enr_offset(&self) -> u64 {
        match self.layout {
            RccRegisterLayout::Stm32F1 => 0x18,
            RccRegisterLayout::Stm32F4 => 0x44, // APB2ENR on STM32F4
            RccRegisterLayout::Stm32V2 => 0xA4, // APB2ENR on STM32H5-style RCC
        }
    }

    fn apb1enr_offset(&self) -> u64 {
        match self.layout {
            RccRegisterLayout::Stm32F1 => 0x1C,
            RccRegisterLayout::Stm32F4 => 0x40, // APB1ENR on STM32F4
            RccRegisterLayout::Stm32V2 => 0x9C, // APB1LENR on STM32H5-style RCC
        }
    }

    fn ahbenr_offset(&self) -> u64 {
        match self.layout {
            RccRegisterLayout::Stm32F1 => 0x14, // AHBENR
            RccRegisterLayout::Stm32F4 => 0x30, // AHB1ENR
            RccRegisterLayout::Stm32V2 => 0x8C, // AHB2ENR
        }
    }

    fn apb2rstr_offset(&self) -> u64 {
        match self.layout {
            RccRegisterLayout::Stm32F1 => 0x0C,
            RccRegisterLayout::Stm32F4 => 0x24,
            RccRegisterLayout::Stm32V2 => 0x84,
        }
    }

    fn apb1rstr_offset(&self) -> u64 {
        match self.layout {
            RccRegisterLayout::Stm32F1 => 0x10,
            RccRegisterLayout::Stm32F4 => 0x20,
            RccRegisterLayout::Stm32V2 => 0x7C,
        }
    }

    fn ahbrstr_offset(&self) -> u64 {
        match self.layout {
            RccRegisterLayout::Stm32F1 => 0x28,
            RccRegisterLayout::Stm32F4 => 0x10,
            RccRegisterLayout::Stm32V2 => 0x6C,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        if offset == 0x00 {
            return self.cr;
        }
        if offset == 0x04 {
            return self.cfgr;
        }
        if offset == self.ahbenr_offset() {
            return self.ahbenr;
        }
        if offset == self.apb2enr_offset() {
            return self.apb2enr;
        }
        if offset == self.apb1enr_offset() {
            return self.apb1enr;
        }
        if offset == self.ahbrstr_offset() {
            return self.ahbrstr;
        }
        if offset == self.apb2rstr_offset() {
            return self.apb2rstr;
        }
        if offset == self.apb1rstr_offset() {
            return self.apb1rstr;
        }
        0
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if offset == 0x00 {
            self.cr = value;
            self.update_ready_flags();
            return;
        }
        if offset == 0x04 {
            self.cfgr = value;
            // Mirror clock switch status (SWS) from software selection (SW).
            let sw = self.cfgr & 0x3;
            self.cfgr = (self.cfgr & !(0x3 << 2)) | (sw << 2);
            return;
        }
        if offset == self.ahbenr_offset() {
            self.ahbenr = value;
            return;
        }
        if offset == self.apb2enr_offset() {
            self.apb2enr = value;
            return;
        }
        if offset == self.apb1enr_offset() {
            self.apb1enr = value;
            return;
        }
        if offset == self.ahbrstr_offset() {
            self.ahbrstr = value;
            return;
        }
        if offset == self.apb2rstr_offset() {
            self.apb2rstr = value;
            return;
        }
        if offset == self.apb1rstr_offset() {
            self.apb1rstr = value;
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
        rcc.write(0x14, 0x11).unwrap();
        rcc.write(0x18, 0xAA).unwrap();
        rcc.write(0x1C, 0x55).unwrap();
        assert_eq!(rcc.read(0x14).unwrap(), 0x11);
        assert_eq!(rcc.read(0x18).unwrap(), 0xAA);
        assert_eq!(rcc.read(0x1C).unwrap(), 0x55);
    }

    #[test]
    fn test_rcc_f4_offsets() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32F4);
        rcc.write(0x30, 0x12).unwrap(); // AHB1ENR
        rcc.write(0x44, 0x34).unwrap(); // APB2ENR
        rcc.write(0x40, 0x56).unwrap(); // APB1ENR
        assert_eq!(rcc.read(0x30).unwrap(), 0x12);
        assert_eq!(rcc.read(0x44).unwrap(), 0x34);
        assert_eq!(rcc.read(0x40).unwrap(), 0x56);
    }

    #[test]
    fn test_rcc_v2_offsets() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);
        rcc.write(0x8C, 0xF0).unwrap(); // AHB2ENR
        rcc.write(0xA4, 0xCC).unwrap();
        rcc.write(0x9C, 0x33).unwrap();
        assert_eq!(rcc.read(0x8C).unwrap(), 0xF0);
        assert_eq!(rcc.read(0xA4).unwrap(), 0xCC);
        assert_eq!(rcc.read(0x9C).unwrap(), 0x33);
        assert_eq!(rcc.read(0x18).unwrap(), 0x00);
    }

    #[test]
    fn test_rcc_cr_ready_flags_follow_enable_bits() {
        let mut rcc = Rcc::new();
        assert_eq!(rcc.read(0x00).unwrap() & 0x02, 0x02); // HSIRDY set at reset

        rcc.write(0x00, 0x00).unwrap();
        assert_eq!(rcc.read(0x00).unwrap() & 0x02, 0x00); // HSIRDY clears with HSION=0

        // Enable HSE (bit 16) and PLL (bit 24). RDY bits should follow.
        rcc.write(0x02, 0x01).unwrap(); // byte containing bit16
        rcc.write(0x03, 0x01).unwrap(); // byte containing bit24

        let cr_b2 = rcc.read(0x02).unwrap(); // bits 16..23
        let cr_b3 = rcc.read(0x03).unwrap(); // bits 24..31
        assert_eq!(cr_b2 & 0x02, 0x02); // HSERDY (bit17)
        assert_eq!(cr_b3 & 0x02, 0x02); // PLLRDY (bit25)
    }

    #[test]
    fn test_rcc_cfgr_sws_mirrors_sw() {
        let mut rcc = Rcc::new();
        rcc.write(0x04, 0b10).unwrap();
        let cfgr = rcc.read(0x04).unwrap();
        assert_eq!(cfgr & 0b11, 0b10); // SW
        assert_eq!((cfgr >> 2) & 0b11, 0b10); // SWS mirrors SW
    }
}
