// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;

/// STM32F1 compatible SPI peripheral
#[derive(Debug, Default, serde::Serialize)]
pub struct Spi {
    cr1: u16,
    cr2: u16,
    sr: u16,
    dr: u16,
    crcpr: u16,
    rxcrcr: u16,
    txcrcr: u16,
    i2scfgr: u16,
    i2spr: u16,

    // Internal state
    transfer_in_progress: bool,
    transfer_cycles_remaining: u32,
    transfer_buffer: u8,
}

impl Spi {
    pub fn new() -> Self {
        Self {
            sr: 0x0002, // Reset value: TXE (Transmit buffer empty) set
            ..Default::default()
        }
    }

    fn read_reg(&self, offset: u64) -> u16 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.sr,
            0x0C => self.dr,
            0x10 => self.crcpr,
            0x14 => self.rxcrcr,
            0x18 => self.txcrcr,
            0x1C => self.i2scfgr,
            0x20 => self.i2spr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u16) {
        match offset {
            0x00 => {
                self.cr1 = value;
                // If SPE (SPI enable) is set, we might need to update state
            }
            0x04 => self.cr2 = value,
            0x08 => {
                // SR is mostly read-only, but some bits might be clearable?
                // For now, only allow clearing OVR (if we implemented it)
                self.sr = value & 0xFFBF;
            }
            0x0C => {
                self.dr = value;
                // Start transfer if enabled
                if (self.cr1 & (1 << 6)) != 0 {
                    // SPE set
                    self.sr &= !0x0002; // Clear TXE (Transmit buffer empty)
                    self.sr |= 0x0080; // Set BSY (Busy)
                    self.transfer_in_progress = true;

                    // Calculate cycles based on BR[2:0] in CR1 (bits 5:3)
                    // F_pclk / 2^(BR + 1)
                    let br = (self.cr1 >> 3) & 0x7;
                    let divider = 1 << (br + 1);
                    // For now, assume 1 bit per divider cycles, 8 bits total
                    // This is a simplification but better than instant.
                    self.transfer_cycles_remaining = 8 * divider;
                    self.transfer_buffer = (value & 0xFF) as u8;
                }
            }
            _ => {}
        }
    }
}

impl crate::Peripheral for Spi {
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

    fn tick(&mut self) -> crate::PeripheralTickResult {
        let mut irq = false;
        if self.transfer_in_progress {
            self.transfer_cycles_remaining = self.transfer_cycles_remaining.saturating_sub(1);
            if self.transfer_cycles_remaining == 0 {
                self.transfer_in_progress = false;
                self.sr &= !0x0080; // Clear BSY
                self.sr |= 0x0002; // Set TXE
                self.sr |= 0x0001; // Set RXNE
                                   // Put transmitted byte (or dummy) into DR for reading
                                   // In a real master, it would be the byte from the slave.
                self.dr = self.transfer_buffer as u16;

                if (self.cr2 & (1 << 7)) != 0 || (self.cr2 & (1 << 6)) != 0 {
                    // TXEIE or RXNEIE
                    irq = true;
                }
            }
        }

        crate::PeripheralTickResult {
            irq,
            cycles: 1,
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::Spi;
    use crate::Peripheral;

    #[test]
    fn test_spi_transfer_timing() {
        let mut spi = Spi::new();
        // Enable SPI (SPE=bit 6) and set Baud rate to f_pclk/4 (BR=1 -> bits 5:3 = 001)
        // CR1 offset 0x00 is 16-bit, aligned to 32.
        spi.write(0x00, 0x48).unwrap(); // (1 << 6) | (1 << 3) = 0x40 | 0x08 = 0x48

        // Check reset state: TXE should be set initially (bit 1 of SR at 0x08)
        assert_ne!(spi.read(0x08).unwrap() & 0x02, 0);

        // Write data to DR to start transfer (DR at 0x0C)
        spi.write(0x0C, 0xAA).unwrap();

        // Check that BSY is set (bit 7) and TXE is cleared (bit 1)
        let sr = spi.read(0x08).unwrap();
        assert_ne!(sr & 0x80, 0); // BSY set
        assert_eq!(sr & 0x02, 0); // TXE cleared

        // Transfer with BR=1 (divider=4) takes 8 * 4 = 32 cycles.
        for _ in 0..31 {
            spi.tick();
            assert_ne!(spi.read(0x08).unwrap() & 0x80, 0); // Still busy
        }

        // 32nd tick should complete it
        spi.tick();
        let sr = spi.read(0x08).unwrap();
        assert_eq!(sr & 0x80, 0); // BSY cleared
        assert_ne!(sr & 0x02, 0); // TXE set
        assert_ne!(sr & 0x01, 0); // RXNE set

        // DR should contain the transmitted byte (simplified loopback)
        assert_eq!(spi.read(0x0C).unwrap(), 0xAA);
    }
}
