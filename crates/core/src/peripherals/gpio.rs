// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GpioRegisterLayout {
    Stm32F1,
    Stm32V2,
}

impl Default for GpioRegisterLayout {
    fn default() -> Self {
        Self::Stm32F1
    }
}

impl FromStr for GpioRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32v2" | "v2" | "modern" | "stm32-modern" | "h5" | "stm32h5" => Ok(Self::Stm32V2),
            _ => Err(format!(
                "unsupported GPIO register layout '{}'; supported: stm32f1, stm32v2",
                value
            )),
        }
    }
}

/// STM32 GPIO peripheral with selectable register layout (STM32F1 or STM32v2/H5-style).
#[derive(Debug, Default, serde::Serialize)]
pub struct GpioPort {
    layout: GpioRegisterLayout,
    crl: u32,     // 0x00: configuration register low
    crh: u32,     // 0x04: configuration register high
    moder: u32,   // 0x00: mode register (STM32v2)
    otyper: u32,  // 0x04: output type register (STM32v2)
    ospeedr: u32, // 0x08: output speed register (STM32v2)
    pupdr: u32,   // 0x0C: pull-up/pull-down register (STM32v2)
    idr: u32,     // 0x08: input data register
    odr: u32,     // 0x0C: output data register
    lckr: u32,    // 0x18: configuration lock register
    afrl: u32,    // 0x20: alternate function low register (STM32v2)
    afrh: u32,    // 0x24: alternate function high register (STM32v2)
    bsrr_buf: u32,
    bsrr_mask: u8,
    brr_buf: u32,
    brr_mask: u8,
}

impl GpioPort {
    pub fn new() -> Self {
        Self::new_with_layout(GpioRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: GpioRegisterLayout) -> Self {
        let mut port = Self {
            layout,
            ..Default::default()
        };

        if matches!(layout, GpioRegisterLayout::Stm32F1) {
            // Reset value: floating input
            port.crl = 0x4444_4444;
            port.crh = 0x4444_4444;
        } else {
            // Generic STM32v2-like default: all pins input, push-pull, low speed, no pull-up/down.
            port.moder = 0x0000_0000;
            port.otyper = 0x0000_0000;
            port.ospeedr = 0x0000_0000;
            port.pupdr = 0x0000_0000;
        }

        port
    }

    fn bsrr_offset(&self) -> u64 {
        match self.layout {
            GpioRegisterLayout::Stm32F1 => 0x10,
            GpioRegisterLayout::Stm32V2 => 0x18,
        }
    }

    fn brr_offset(&self) -> u64 {
        match self.layout {
            GpioRegisterLayout::Stm32F1 => 0x14,
            GpioRegisterLayout::Stm32V2 => 0x28,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match self.layout {
            GpioRegisterLayout::Stm32F1 => match offset {
                0x00 => self.crl,
                0x04 => self.crh,
                0x08 => self.idr,
                0x0C => self.odr,
                0x18 => self.lckr,
                _ => 0,
            },
            GpioRegisterLayout::Stm32V2 => match offset {
                0x00 => self.moder,
                0x04 => self.otyper,
                0x08 => self.ospeedr,
                0x0C => self.pupdr,
                0x10 => self.idr,
                0x14 => self.odr,
                0x1C => self.lckr,
                0x20 => self.afrl,
                0x24 => self.afrh,
                _ => 0,
            },
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match self.layout {
            GpioRegisterLayout::Stm32F1 => match offset {
                0x00 => self.crl = value,
                0x04 => self.crh = value,
                0x0C => self.odr = value & 0xFFFF,
                0x10 => {
                    // BSRR: Bit Set/Reset Register
                    let set = value & 0xFFFF;
                    let reset = (value >> 16) & 0xFFFF;
                    self.odr |= set;
                    self.odr &= !reset;
                }
                0x14 => {
                    // BRR: Bit Reset Register
                    let reset = value & 0xFFFF;
                    self.odr &= !reset;
                }
                0x18 => self.lckr = value,
                _ => {}
            },
            GpioRegisterLayout::Stm32V2 => match offset {
                0x00 => self.moder = value,
                0x04 => self.otyper = value & 0xFFFF,
                0x08 => self.ospeedr = value,
                0x0C => self.pupdr = value,
                0x10 => self.idr = value & 0xFFFF,
                0x14 => self.odr = value & 0xFFFF,
                0x18 => {
                    // BSRR: lower 16 bits set, upper 16 bits reset.
                    let set = value & 0xFFFF;
                    let reset = (value >> 16) & 0xFFFF;
                    self.odr |= set;
                    self.odr &= !reset;
                }
                0x1C => self.lckr = value,
                0x20 => self.afrl = value,
                0x24 => self.afrh = value,
                0x28 => {
                    // BRR: reset selected ODR bits.
                    let reset = value & 0xFFFF;
                    self.odr &= !reset;
                }
                _ => {}
            },
        }
    }

    fn handle_write_only_buffer(&mut self, reg_offset: u64, byte_offset: u32, value: u8) -> bool {
        let (buf, mask) = if reg_offset == self.bsrr_offset() {
            (&mut self.bsrr_buf, &mut self.bsrr_mask)
        } else {
            (&mut self.brr_buf, &mut self.brr_mask)
        };

        let shift = byte_offset * 8;
        let byte_mask = 1u8 << byte_offset;
        *buf &= !(0xFF << shift);
        *buf |= (value as u32) << shift;
        *mask |= byte_mask;

        if *mask == 0x0F {
            let val = *buf;
            *buf = 0;
            *mask = 0;
            self.write_reg(reg_offset, val);
            return true;
        }

        if *mask == 0x03 {
            let val = *buf & 0x0000_FFFF;
            *buf = 0;
            *mask = 0;
            self.write_reg(reg_offset, val);
            return true;
        }

        if *mask == 0x0C {
            let val = *buf & 0xFFFF_0000;
            *buf = 0;
            *mask = 0;
            self.write_reg(reg_offset, val);
            return true;
        }

        false
    }
}

impl crate::Peripheral for GpioPort {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let bsrr_offset = self.bsrr_offset();
        let brr_offset = self.brr_offset();

        if (reg_offset == bsrr_offset || reg_offset == brr_offset)
            && self.handle_write_only_buffer(reg_offset, byte_offset, value)
        {
            return Ok(());
        }

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
    use super::{GpioPort, GpioRegisterLayout};
    use crate::Peripheral;

    #[test]
    fn test_gpio_reset_values() {
        let gpio = GpioPort::new();
        assert_eq!(gpio.crl, 0x4444_4444);
        assert_eq!(gpio.crh, 0x4444_4444);
        assert_eq!(gpio.odr, 0);
    }

    #[test]
    fn test_gpio_odr_write() {
        let mut gpio = GpioPort::new();
        // Write to ODR (offset 0x0C)
        gpio.write(0x0C, 0x55).unwrap(); // Write byte 0
        gpio.write(0x0D, 0xAA).unwrap(); // Write byte 1
        assert_eq!(gpio.odr, 0xAA55);
    }

    #[test]
    fn test_gpio_bsrr_set() {
        let mut gpio = GpioPort::new();
        // BSRR is 32-bit, offset 0x10. Writing to lower 16 bits sets ODR bits.
        // We write 4 bytes to trigger handle_write_only_buffer
        gpio.write(0x10, 0x01).unwrap();
        gpio.write(0x11, 0x00).unwrap();
        gpio.write(0x12, 0x00).unwrap();
        gpio.write(0x13, 0x00).unwrap();
        assert_eq!(gpio.odr, 0x0001);
    }

    #[test]
    fn test_gpio_bsrr_reset() {
        let mut gpio = GpioPort::new();
        gpio.odr = 0xFFFF;
        // BSRR upper 16 bits reset ODR bits.
        gpio.write(0x10, 0x00).unwrap();
        gpio.write(0x11, 0x00).unwrap();
        gpio.write(0x12, 0x01).unwrap();
        gpio.write(0x13, 0x00).unwrap();
        assert_eq!(gpio.odr, 0xFFFE);
    }

    #[test]
    fn test_gpio_brr() {
        let mut gpio = GpioPort::new();
        gpio.odr = 0xFFFF;
        // BRR is offset 0x14. Lower 16 bits reset ODR bits.
        // Needs 4 bytes to flush the buffer (handled as 32-bit in the mock for complexity)
        // Actually GpioPort handle_write_only_buffer for BRR also checks for byte mask.
        gpio.write(0x14, 0x01).unwrap();
        gpio.write(0x15, 0x00).unwrap();
        gpio.write(0x16, 0x00).unwrap();
        gpio.write(0x17, 0x00).unwrap();
        assert_eq!(gpio.odr, 0xFFFE);
    }

    #[test]
    fn test_gpio_v2_moder_and_odr() {
        let mut gpio = GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2);

        // MODER @ 0x00
        gpio.write(0x00, 0xAA).unwrap();
        gpio.write(0x01, 0x55).unwrap();
        assert_eq!(gpio.moder & 0xFFFF, 0x55AA);

        // ODR @ 0x14
        gpio.write(0x14, 0x34).unwrap();
        gpio.write(0x15, 0x12).unwrap();
        assert_eq!(gpio.odr, 0x1234);
    }

    #[test]
    fn test_gpio_v2_bsrr_and_brr() {
        let mut gpio = GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2);

        // BSRR @ 0x18 (set pin 0, reset pin 1)
        gpio.write(0x18, 0x01).unwrap();
        gpio.write(0x19, 0x00).unwrap();
        gpio.write(0x1A, 0x02).unwrap();
        gpio.write(0x1B, 0x00).unwrap();
        assert_eq!(gpio.odr & 0x0003, 0x0001);

        // BRR @ 0x28 (reset pin 0)
        gpio.write(0x28, 0x01).unwrap();
        gpio.write(0x29, 0x00).unwrap();
        gpio.write(0x2A, 0x00).unwrap();
        gpio.write(0x2B, 0x00).unwrap();
        assert_eq!(gpio.odr & 0x0001, 0x0000);
    }
}
