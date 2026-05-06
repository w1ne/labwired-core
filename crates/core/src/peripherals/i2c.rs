// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::cell::{Cell, RefCell};

pub trait I2cDevice: Send {
    fn address(&self) -> u8;
    fn read(&mut self) -> u8;
    fn write(&mut self, data: u8);
    fn start(&mut self) {}
    fn stop(&mut self) {}
}

/// STM32F1 compatible I2C peripheral (Master mode only)
#[derive(serde::Serialize)]
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

    #[serde(skip)]
    pub attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
    #[serde(skip)]
    current_target: Option<usize>,
    #[serde(skip)]
    is_reading: bool,
    #[serde(skip)]
    read_dr_consumed: Cell<bool>,
}

impl core::fmt::Debug for I2c {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("I2c").field("state", &self.state).finish()
    }
}

impl Default for I2c {
    fn default() -> Self {
        Self {
            cr1: 0,
            cr2: 0,
            oar1: 0,
            oar2: 0,
            dr: 0,
            sr1: 0,
            sr2: 0,
            ccr: 0,
            trise: 0,
            state: I2cState::Idle,
            cycles_remaining: 0,
            attached_devices: Vec::new(),
            current_target: None,
            is_reading: false,
            read_dr_consumed: Cell::new(true),
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

    pub fn attach(&mut self, device: Box<dyn I2cDevice>) {
        self.attached_devices.push(RefCell::new(device));
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
                        let addr = (self.dr >> 1) as u8;
                        self.is_reading = (self.dr & 1) != 0;
                        self.current_target = self
                            .attached_devices
                            .iter()
                            .position(|d| d.borrow().address() == addr);
                    } else {
                        // This is data
                        self.state = I2cState::DataPending;
                        self.cycles_remaining = 20;
                        self.sr1 &= !0x80; // Clear TXE
                        self.sr1 &= !0x04; // Clear BTF

                        // Pass written data to device immediately
                        if !self.is_reading {
                            if let Some(idx) = self.current_target {
                                self.attached_devices[idx].borrow_mut().write(self.dr as u8);
                            }
                        }
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
        if reg_offset == 0x10 && byte_offset == 0 && self.is_reading && (self.sr1 & 0x0040) != 0 {
            if !self.read_dr_consumed.replace(true) {
                return Ok((self.dr & 0xFF) as u8);
            }

            if let Some(idx) = self.current_target {
                return Ok(self.attached_devices[idx].borrow_mut().read());
            }
        }

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
                        if let Some(idx) = self.current_target {
                            self.attached_devices[idx].borrow_mut().start();
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
                                self.dr = self.attached_devices[idx].borrow_mut().read() as u16;
                                self.read_dr_consumed.set(false);
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
                        self.read_dr_consumed.set(true);
                        self.state = I2cState::Idle;
                        if let Some(idx) = self.current_target {
                            self.attached_devices[idx].borrow_mut().stop();
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
    use super::{I2c, I2cDevice};
    use crate::Peripheral;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct CountingDevice {
        address: u8,
        reads: Arc<AtomicUsize>,
    }

    impl CountingDevice {
        fn new(address: u8, reads: Arc<AtomicUsize>) -> Self {
            Self { address, reads }
        }
    }

    impl I2cDevice for CountingDevice {
        fn address(&self) -> u8 {
            self.address
        }

        fn read(&mut self) -> u8 {
            self.reads.fetch_add(1, Ordering::SeqCst) as u8
        }

        fn write(&mut self, _data: u8) {}
    }

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

    #[test]
    fn test_adxl345_devid_and_axis_read() {
        use crate::peripherals::components::Adxl345;

        let mut i2c = I2c::new();
        let mut sensor = Adxl345::new(0x53);
        sensor.set_sample(256, -128, 64);
        i2c.attach(Box::new(sensor));

        i2c.write(0x00, 0x01).unwrap();
        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0);

        i2c.write(0x10, 0xA6).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x02, 0);

        i2c.write(0x10, 0x00).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }

        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        i2c.write(0x10, 0xA7).unwrap();
        for _ in 0..40 {
            i2c.tick();
        }
        assert_eq!(i2c.read(0x10).unwrap(), 0xE5);

        i2c.write(0x01, 0x02).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }

        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        i2c.write(0x10, 0xA6).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        i2c.write(0x10, 0x32).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }
        i2c.write(0x10, 0xA7).unwrap();
        for _ in 0..40 {
            i2c.tick();
        }

        assert_eq!(i2c.read(0x10).unwrap(), 0x00);
        assert_eq!(i2c.read(0x10).unwrap(), 0x01);
        assert_eq!(i2c.read(0x10).unwrap(), 0x80);
        assert_eq!(i2c.read(0x10).unwrap(), 0xFF);
        assert_eq!(i2c.read(0x10).unwrap(), 0x40);
        assert_eq!(i2c.read(0x10).unwrap(), 0x00);
    }

    #[test]
    fn test_i2c_single_byte_read_advances_device_once() {
        let reads = Arc::new(AtomicUsize::new(0));
        let mut i2c = I2c::new();
        i2c.attach(Box::new(CountingDevice::new(0x42, reads.clone())));

        i2c.write(0x01, 0x01).unwrap();
        for _ in 0..10 {
            i2c.tick();
        }

        i2c.write(0x10, 0x85).unwrap();
        for _ in 0..40 {
            i2c.tick();
        }

        assert_ne!(i2c.peek(0x14).unwrap() & 0x40, 0);
        assert_eq!(i2c.read(0x10).unwrap(), 0);
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }
}
