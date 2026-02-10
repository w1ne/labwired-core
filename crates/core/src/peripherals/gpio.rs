// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;

/// STM32F1-compatible GPIO peripheral
#[derive(Debug, Default, serde::Serialize)]
pub struct GpioPort {
    crl: u32,  // 0x00: configuration register low
    crh: u32,  // 0x04: configuration register high
    idr: u32,  // 0x08: input data register
    odr: u32,  // 0x0C: output data register
    lckr: u32, // 0x18: configuration lock register
    bsrr_buf: u32,
    bsrr_mask: u8,
    brr_buf: u32,
    brr_mask: u8,
}

impl GpioPort {
    pub fn new() -> Self {
        Self {
            crl: 0x4444_4444, // Reset value: floating input
            crh: 0x4444_4444, // Reset value: floating input
            ..Default::default()
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.crl,
            0x04 => self.crh,
            0x08 => self.idr,
            0x0C => self.odr,
            0x18 => self.lckr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
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
        }
    }

    fn handle_write_only_buffer(&mut self, reg_offset: u64, byte_offset: u32, value: u8) -> bool {
        let (buf, mask) = if reg_offset == 0x10 {
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

        if (reg_offset == 0x10 || reg_offset == 0x14)
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
    use super::GpioPort;
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
}
