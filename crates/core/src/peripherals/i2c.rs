// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;

/// STM32F1 compatible I2C peripheral (Master mode only)
#[derive(Debug, Default, serde::Serialize)]
pub struct I2c {
    cr1: u16,
    cr2: u16,
    oar1: u16,
    oar2: u16,
    dr: u16,
    sr1: u16,
    sr2: u16,
    ccr: u16,
    trise: u16,

    // Internal state
    state: I2cState,
    cycles_remaining: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Default)]
enum I2cState {
    #[default]
    Idle,
    StartPending,
    AddressPending,
    DataPending,
    StopPending,
}

impl I2c {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_reg(&self, offset: u64) -> u16 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.oar1,
            0x0C => self.oar2,
            0x10 => self.dr,
            0x14 => self.sr1,
            0x18 => self.sr2,
            0x1C => self.ccr,
            0x20 => self.trise,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u16) {
        match offset {
            0x00 => {
                self.cr1 = value;
                if (value & 0x0100) != 0 {
                    // START Generation
                    self.state = I2cState::StartPending;
                    // Simplified timing: 10 cycles for START
                    self.cycles_remaining = 10;
                }
                if (value & 0x0200) != 0 {
                    // STOP Generation
                    self.state = I2cState::StopPending;
                    self.cycles_remaining = 10;
                }
            }
            0x04 => self.cr2 = value,
            0x08 => self.oar1 = value,
            0x0C => self.oar2 = value,
            0x10 => {
                self.dr = value & 0xFF;
                if self.state == I2cState::Idle {
                    if (self.sr1 & 0x01) != 0 {
                        // SB set, this is address
                        self.state = I2cState::AddressPending;
                        self.cycles_remaining = 20; // Address phase longer
                    } else {
                        // This is data
                        self.state = I2cState::DataPending;
                        self.cycles_remaining = 20;
                        self.sr1 &= !0x80; // Clear TXE
                        self.sr1 &= !0x04; // Clear BTF
                    }
                }
            }
            0x14 => self.sr1 = value,
            0x18 => self.sr2 = value,
            0x1C => self.ccr = value,
            0x20 => self.trise = value,
            _ => {}
        }
    }
}

impl crate::Peripheral for I2c {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        if byte_offset < 2 {
            Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
        } else {
            Ok(0) // Higher bytes of 16-bit registers at 32-bit offsets are 0
        }
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        // Registers are 16-bit but aligned to 32-bit boundaries

        if byte_offset < 2 {
            let mut reg_val = self.read_reg(reg_offset);
            let mask = 0xFF << (byte_offset * 8);
            reg_val &= !mask;
            reg_val |= (value as u16) << (byte_offset * 8);
            self.write_reg(reg_offset, reg_val);
        }
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        let mut irq = false;

        if self.state != I2cState::Idle {
            self.cycles_remaining = self.cycles_remaining.saturating_sub(1);
            if self.cycles_remaining == 0 {
                match self.state {
                    I2cState::StartPending => {
                        self.sr1 |= 0x0001; // Set SB
                        self.state = I2cState::Idle;
                    }
                    I2cState::AddressPending => {
                        self.sr1 &= !0x0001; // Clear SB
                        self.sr1 |= 0x0002; // Set ADDR
                        self.sr2 |= 0x0001; // MSL (Master mode) set
                        self.sr2 |= 0x0002; // BUSY set
                        self.state = I2cState::Idle;
                    }
                    I2cState::DataPending => {
                        self.sr1 |= 0x0080; // Set TXE
                        self.sr1 |= 0x0004; // Set BTF
                        self.state = I2cState::Idle;
                    }
                    I2cState::StopPending => {
                        self.sr1 = 0; // Clear SR1
                        self.sr2 = 0; // Clear SR2
                        self.state = I2cState::Idle;
                    }
                    I2cState::Idle => {}
                }

                if (self.cr2 & (1 << 9)) != 0 || (self.cr2 & (1 << 10)) != 0 {
                    // ITEVTEN or ITBUFEN
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

    fn peek(&self, offset: u64) -> Option<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        if byte_offset < 2 {
            Some(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
        } else {
            Some(0)
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::I2c;
    use crate::Peripheral;

    #[test]
    fn test_i2c_reset_values() {
        let i2c = I2c::new();
        assert_eq!(i2c.cr1, 0);
        assert_eq!(i2c.cr2, 0);
    }

    #[test]
    fn test_i2c_start_bit() {
        let mut i2c = I2c::new();
        // Set SB (Start Bit) in CR1 (bit 8)
        i2c.write(0x01, 0x01).unwrap();

        // Should NOT be set immediately
        assert_eq!(i2c.sr1 & 0x01, 0);

        // Tick 10 times
        for _ in 0..10 {
            i2c.tick();
        }

        assert_ne!(i2c.sr1 & 0x01, 0); // SB bit in SR1 set after ticks
    }

    #[test]
    fn test_i2c_full_transfer_flow() {
        let mut i2c = I2c::new();

        // 1. START
        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        assert_ne!(i2c.sr1 & 0x01, 0); // SB set

        // 2. Address
        i2c.write(0x10, 0xA0).unwrap(); // Write address to DR
        for _ in 0..20 {
            i2c.tick();
        }
        assert_eq!(i2c.sr1 & 0x01, 0); // SB cleared
        assert_ne!(i2c.sr1 & 0x02, 0); // ADDR set
        assert_ne!(i2c.sr2 & 0x01, 0); // MSL set

        // 3. Data
        // Clear ADDR by reading SR1 followed by SR2 (simplified in our model by just writing DR)
        i2c.write(0x10, 0x42).unwrap();
        // ADDR cleared effectively by state transition
        for _ in 0..20 {
            i2c.tick();
        }
        assert_ne!(i2c.sr1 & 0x80, 0); // TXE set
        assert_ne!(i2c.sr1 & 0x04, 0); // BTF set

        // 4. STOP
        i2c.write(0x01, 0x02).unwrap(); // STOP bit 9
        for _ in 0..10 {
            i2c.tick();
        }
        assert_eq!(i2c.sr1, 0);
        assert_eq!(i2c.sr2, 0);
    }
}
