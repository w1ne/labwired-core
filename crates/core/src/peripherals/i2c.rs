// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::str::FromStr;

pub trait I2cDevice: Send {
    fn address(&self) -> u8;
    fn read(&mut self) -> u8;
    fn write(&mut self, data: u8);
    fn start(&mut self) {}
    fn stop(&mut self) {}
}

/// I2C register layout selector. STM32F1/F2/F4 share the legacy I2C
/// peripheral (CR1/CR2/OAR1/OAR2/DR/SR1/SR2/CCR/TRISE, all 16-bit).
/// STM32L4/F7/H5/G0/etc share the modern peripheral (CR1/CR2/OAR1/OAR2/
/// TIMINGR/TIMEOUTR/ISR/ICR/PECR/RXDR/TXDR, all 32-bit). Bit semantics
/// in CR1 / CR2 differ substantially between the two — START/STOP bits
/// live in CR1 on F1, CR2 on L4, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum I2cRegisterLayout {
    #[default]
    Stm32F1,
    /// STM32L4 family (also F7/H5/G0). Verified against real
    /// NUCLEO-L476RG silicon via SWD register dump.
    Stm32L4,
}

impl FromStr for I2cRegisterLayout {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32l4" | "l4" | "stm32f7" | "f7" | "stm32h5" | "h5" | "stm32g0" | "g0" => {
                Ok(Self::Stm32L4)
            }
            _ => Err(format!(
                "unsupported I2C register layout '{}'; supported: stm32f1, stm32l4",
                value
            )),
        }
    }
}

/// I2C peripheral with selectable register layout. Storage is u32 for
/// both layouts so the L4-only TIMINGR / 32-bit CR2 fit; F1 mode just
/// uses the lower 16 bits of each register.
#[derive(serde::Serialize)]
pub struct I2c {
    layout: I2cRegisterLayout,
    cr1: u32,
    cr2: u32,
    oar1: u32,
    oar2: u32,
    // Legacy F1-only:
    dr: u32,
    sr1: u32,
    sr2: u32,
    ccr: u32,
    trise: u32,
    // Modern L4-only:
    timingr: u32,
    timeoutr: u32,
    isr: u32,
    icr: u32,
    pecr: u32,
    rxdr: u32,
    txdr: u32,

    // Internal state (legacy state machine; L4 has its own simpler
    // semantics driven by ISR/CR2 directly).
    state: I2cState,
    cycles_remaining: u32,

    #[serde(skip)]
    pub attached_devices: Vec<Box<dyn I2cDevice>>,
    #[serde(skip)]
    current_target: Option<usize>,
    #[serde(skip)]
    is_reading: bool,
}

impl core::fmt::Debug for I2c {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("I2c").field("state", &self.state).finish()
    }
}

impl Default for I2c {
    fn default() -> Self {
        Self {
            layout: I2cRegisterLayout::Stm32F1,
            cr1: 0,
            cr2: 0,
            oar1: 0,
            oar2: 0,
            dr: 0,
            sr1: 0,
            sr2: 0,
            ccr: 0,
            trise: 0,
            timingr: 0,
            timeoutr: 0,
            // L4 reset value: TXE=1 (bit 0). When this struct is built
            // with the L4 layout the reset state surfaces this; for F1
            // mode the field is unused.
            isr: 0x0000_0001,
            icr: 0,
            pecr: 0,
            rxdr: 0,
            txdr: 0,
            state: I2cState::Idle,
            cycles_remaining: 0,
            attached_devices: Vec::new(),
            current_target: None,
            is_reading: false,
        }
    }
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

    pub fn new_with_layout(layout: I2cRegisterLayout) -> Self {
        Self { layout, ..Self::default() }
    }

    pub fn attach(&mut self, device: Box<dyn I2cDevice>) {
        self.attached_devices.push(device);
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match self.layout {
            I2cRegisterLayout::Stm32F1 => match offset {
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
            },
            I2cRegisterLayout::Stm32L4 => match offset {
                0x00 => self.cr1,
                0x04 => self.cr2,
                0x08 => self.oar1,
                0x0C => self.oar2,
                0x10 => self.timingr,
                0x14 => self.timeoutr,
                0x18 => self.isr,
                0x1C => self.icr,
                0x20 => self.pecr,
                0x24 => self.rxdr,
                0x28 => self.txdr,
                _ => 0,
            },
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match self.layout {
            I2cRegisterLayout::Stm32F1 => self.write_reg_f1(offset, value as u16),
            I2cRegisterLayout::Stm32L4 => self.write_reg_l4(offset, value),
        }
    }

    fn write_reg_f1(&mut self, offset: u64, value: u16) {
        match offset {
            0x00 => {
                self.cr1 = value as u32;
                if (value & 0x0100) != 0 {
                    self.state = I2cState::StartPending;
                    self.cycles_remaining = 10;
                }
                if (value & 0x0200) != 0 {
                    self.state = I2cState::StopPending;
                    self.cycles_remaining = 10;
                }
            }
            0x04 => self.cr2 = value as u32,
            0x08 => self.oar1 = value as u32,
            0x0C => self.oar2 = value as u32,
            0x10 => {
                self.dr = (value & 0xFF) as u32;
                if self.state == I2cState::Idle {
                    if (self.sr1 & 0x01) != 0 {
                        self.state = I2cState::AddressPending;
                        self.cycles_remaining = 20;
                        let addr = (self.dr >> 1) as u8;
                        self.is_reading = (self.dr & 1) != 0;
                        self.current_target = self
                            .attached_devices
                            .iter()
                            .position(|d| d.address() == addr);
                    } else {
                        self.state = I2cState::DataPending;
                        self.cycles_remaining = 20;
                        self.sr1 &= !0x80;
                        self.sr1 &= !0x04;
                        if !self.is_reading {
                            if let Some(idx) = self.current_target {
                                self.attached_devices[idx].write(self.dr as u8);
                            }
                        }
                    }
                }
            }
            0x14 => self.sr1 = value as u32,
            0x18 => self.sr2 = value as u32,
            0x1C => self.ccr = value as u32,
            0x20 => self.trise = value as u32,
            _ => {}
        }
    }

    fn write_reg_l4(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr1 = value & 0x00FF_E1FF, // PE, ANFOFF, DNF, etc.
            0x04 => {
                self.cr2 = value;
                // CR2.START (bit 13): hardware sets ISR.BUSY (bit 15)
                // when a master start is requested. Real silicon also
                // begins clocking SCL after this; we just reflect the
                // BUSY flag for register-fidelity purposes — driving
                // an actual transfer requires a slave device model.
                if (value & (1 << 13)) != 0 {
                    self.isr |= 1 << 15;
                }
                if (value & (1 << 14)) != 0 {
                    // STOP — clear BUSY.
                    self.isr &= !(1 << 15);
                }
            }
            0x08 => self.oar1 = value,
            0x0C => self.oar2 = value,
            0x10 => self.timingr = value,
            0x14 => self.timeoutr = value,
            0x18 => {
                // ISR is mostly read-only; some bits are W1C handled via ICR.
                // Allow direct writes only to RW bits — TXE (bit 0) is RW
                // (write 1 to flush TXDR). Conservative: allow setting/
                // clearing the writable bits, leave the rest as-is.
                let rw_mask: u32 = 0x0000_0001;
                self.isr = (self.isr & !rw_mask) | (value & rw_mask);
            }
            0x1C => {
                // ICR: writing 1 clears the corresponding ISR flag.
                // Bits cleared: ADDRCF=3, NACKCF=4, STOPCF=5, BERRCF=8,
                // ARLOCF=9, OVRCF=10, PECCF=11, TIMOUTCF=12, ALERTCF=13.
                let clearable: u32 = 0x0000_3F38;
                self.isr &= !(value & clearable);
                self.icr = 0; // ICR self-clears after write.
            }
            0x20 => self.pecr = value,
            0x24 => self.rxdr = value & 0xFF,
            0x28 => {
                self.txdr = value & 0xFF;
                // Writing TXDR clears TXE (bit 0) and TXIS (bit 1).
                self.isr &= !0x0000_0003;
            }
            _ => {}
        }
    }
}

impl crate::Peripheral for I2c {
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
        let mask: u32 = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);
        self.write_reg(reg_offset, reg_val);
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
                        if let Some(idx) = self.current_target {
                            self.attached_devices[idx].start();
                        }
                    }
                    I2cState::AddressPending => {
                        self.sr1 &= !0x0001; // Clear SB
                        self.sr1 |= 0x0002; // Set ADDR
                        self.sr2 |= 0x0001; // MSL (Master mode) set
                        self.sr2 |= 0x0002; // BUSY set

                        // If it's a read, automatically transition to grabbing the first byte
                        if self.is_reading {
                            self.state = I2cState::DataPending;
                            self.cycles_remaining = 20;
                        } else {
                            self.state = I2cState::Idle;
                        }
                    }
                    I2cState::DataPending => {
                        if self.is_reading {
                            self.sr1 |= 0x0040; // Set RXNE
                            if let Some(idx) = self.current_target {
                                self.dr = self.attached_devices[idx].read() as u32;
                            }
                            self.state = I2cState::Idle;
                        } else {
                            self.sr1 |= 0x0080; // Set TXE
                            self.sr1 |= 0x0004; // Set BTF
                            self.state = I2cState::Idle;
                        }
                    }
                    I2cState::StopPending => {
                        self.sr1 = 0; // Clear SR1
                        self.sr2 = 0; // Clear SR2
                        self.state = I2cState::Idle;
                        if let Some(idx) = self.current_target {
                            self.attached_devices[idx].stop();
                        }
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
            cycles: 0,
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
