// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// I2C is one struct PER FAMILY behind the `I2c` enum:
//   * `F1I2c` — the legacy peripheral (CR1/CR2/OAR/DR/SR1/SR2/CCR/TRISE) AND
//     the full transaction state machine. START/STOP live in CR1.
//   * `L4I2c` — the modern peripheral (CR1/CR2/OAR/TIMINGR/ISR/ICR/RXDR/TXDR),
//     register-fidelity latching (START/STOP in CR2; no transaction engine).
// Each variant owns ALL of its own registers and state — an F1 I2C cannot
// carry TIMINGR/ISR, an L4 I2C cannot carry SR1/DR. CR1/CR2/OAR and the
// attached-device list exist on both because both families genuinely have
// them. The chip-yaml `profile` selects the variant.

use crate::SimResult;
use std::cell::{Cell, RefCell};
use std::str::FromStr;

pub trait I2cDevice: Send {
    fn address(&self) -> u8;
    fn read(&mut self) -> u8;
    fn write(&mut self, data: u8);
    fn start(&mut self) {}
    fn stop(&mut self) {}
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }
}

/// I2C register layout selector. STM32F1/F2/F4 share the legacy I2C
/// peripheral; STM32L4/F7/H5/G0 share the modern peripheral. The config-facing
/// value maps 1:1 to a dedicated family struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum I2cRegisterLayout {
    #[default]
    Stm32F1,
    /// STM32L4 family (also F7/H5/G0). Verified against real NUCLEO-L476RG
    /// silicon via SWD register dump.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Default)]
enum I2cState {
    #[default]
    Idle,
    StartPending,
    AddressPending,
    DataPending,
}

// ── STM32F1 legacy I2C (registers + transaction state machine) ───────────────
#[derive(serde::Serialize)]
pub struct F1I2c {
    cr1: u32,
    cr2: u32,
    oar1: u32,
    oar2: u32,
    dr: u32,
    sr1: u32,
    sr2: u32,
    ccr: u32,
    trise: u32,

    state: I2cState,
    cycles_remaining: u32,

    #[serde(skip)]
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
    #[serde(skip)]
    current_target: Option<usize>,
    #[serde(skip)]
    is_reading: bool,
    #[serde(skip)]
    stop_requested: bool,
    #[serde(skip)]
    rxne_consumed: Cell<bool>,
    #[serde(skip)]
    read_dr_consumed: Cell<bool>,
}

impl Default for F1I2c {
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
            // TRISE reset value is 0x0002 (RM0008 §26.6.9) — silicon-confirmed
            // on STM32F103 over SWD (reads 0x00000002 after RCC clock enable,
            // before any write).
            trise: 0x0002,
            state: I2cState::Idle,
            cycles_remaining: 0,
            attached_devices: Vec::new(),
            current_target: None,
            is_reading: false,
            stop_requested: false,
            rxne_consumed: Cell::new(false),
            read_dr_consumed: Cell::new(true),
        }
    }
}

impl F1I2c {
    fn read_reg(&self, offset: u64) -> u32 {
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
                self.cr1 = value as u32;
                if (value & 0x0100) != 0 && self.state == I2cState::Idle {
                    self.state = I2cState::StartPending;
                    self.cycles_remaining = 1;
                }
                if (value & 0x0200) != 0 {
                    // STOP requested. Defer if a data phase is in flight so
                    // RXNE/BTF latch first (HAL "NACK+STOP → poll RXNE → read
                    // DR" ordering); otherwise complete synchronously.
                    if matches!(self.state, I2cState::DataPending | I2cState::AddressPending) {
                        self.stop_requested = true;
                    } else {
                        self.cr1 &= !0x0200;
                        self.sr2 &= !0x0003;
                        if let Some(idx) = self.current_target {
                            self.attached_devices[idx].borrow_mut().stop();
                        }
                        self.current_target = None;
                        self.state = I2cState::Idle;
                    }
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
                            .position(|d| d.borrow().address() == addr);
                        if let Some(idx) = self.current_target {
                            self.attached_devices[idx].borrow_mut().start();
                        }
                    } else {
                        self.state = I2cState::DataPending;
                        self.cycles_remaining = 20;
                        self.sr1 &= !0x80;
                        self.sr1 &= !0x04;
                        if !self.is_reading {
                            if let Some(idx) = self.current_target {
                                self.attached_devices[idx].borrow_mut().write(self.dr as u8);
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

    fn read(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        if reg_offset == 0x10 && byte_offset == 0 && self.is_reading && (self.sr1 & 0x0040) != 0 {
            if !self.read_dr_consumed.replace(true) {
                return (self.dr & 0xFF) as u8;
            }
            if let Some(idx) = self.current_target {
                return self.attached_devices[idx].borrow_mut().read();
            }
        }

        let reg_val = self.read_reg(reg_offset);
        // Silicon clears RXNE when firmware reads DR; mark for next tick.
        if reg_offset == 0x10 && byte_offset == 0 && (self.sr1 & 0x40) != 0 {
            self.rxne_consumed.set(true);
        }
        ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
    }

    fn write(&mut self, offset: u64, value: u8) {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);
        let mask: u32 = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);
        self.write_reg(reg_offset, reg_val as u16);
    }

    fn peek(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        if byte_offset < 2 {
            ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
        } else {
            0
        }
    }

    /// One tick of the transaction state machine. Returns whether an IRQ
    /// should be raised. Logic relocated verbatim from the pre-split model.
    fn tick(&mut self) -> bool {
        let mut irq = false;

        // "RXNE clears on DR read" mirror, fires even when Idle.
        if self.rxne_consumed.replace(false) {
            self.sr1 &= !0x0040;
            self.sr1 &= !0x0004; // BTF tied to the same shift register
            if self.is_reading && self.current_target.is_some() {
                self.state = I2cState::DataPending;
                self.cycles_remaining = 1;
            }
        }

        if self.state != I2cState::Idle {
            self.cycles_remaining = self.cycles_remaining.saturating_sub(1);
            if self.cycles_remaining == 0 {
                match self.state {
                    I2cState::StartPending => {
                        self.sr1 = 0x0001; // Only SB set
                        self.cr1 &= !0x0100; // auto-clear START request
                        self.state = I2cState::Idle;
                    }
                    I2cState::AddressPending => {
                        self.sr1 &= !0x0001; // Clear SB

                        // No slave at this address → NACK (SR1.AF), bus stays
                        // master+BUSY until firmware STOPs (matches F407 silicon).
                        if self.current_target.is_none() {
                            self.sr1 |= 0x0400; // AF
                            self.sr2 |= 0x0001; // MSL
                            self.sr2 |= 0x0002; // BUSY
                            self.state = I2cState::Idle;
                            if (self.cr2 & (1 << 8)) != 0 {
                                irq = true; // ITERR
                            }
                            return irq;
                        }

                        self.sr1 |= 0x0002; // ADDR
                        self.sr2 |= 0x0001; // MSL
                        self.sr2 |= 0x0002; // BUSY

                        if self.is_reading {
                            self.state = I2cState::DataPending;
                            self.cycles_remaining = 20;
                        } else {
                            self.sr1 |= 0x0080; // TXE
                            self.state = I2cState::Idle;
                        }
                    }
                    I2cState::DataPending => {
                        if self.is_reading {
                            self.sr1 |= 0x0040; // RXNE
                            if let Some(idx) = self.current_target {
                                self.dr = self.attached_devices[idx].borrow_mut().read() as u32;
                                self.read_dr_consumed.set(false);
                            }
                            self.state = I2cState::Idle;
                        } else {
                            self.sr1 |= 0x0080; // TXE
                            self.sr1 |= 0x0004; // BTF
                            self.state = I2cState::Idle;
                        }
                        if self.stop_requested {
                            self.stop_requested = false;
                            self.cr1 &= !0x0200;
                            self.sr2 &= !0x0003;
                            if let Some(idx) = self.current_target {
                                self.attached_devices[idx].borrow_mut().stop();
                            }
                            self.current_target = None;
                        }
                    }
                    I2cState::Idle => {}
                }

                if (self.cr2 & (1 << 9)) != 0 || (self.cr2 & (1 << 10)) != 0 {
                    irq = true; // ITEVTEN or ITBUFEN
                }
            }
        }

        irq
    }
}

// ── STM32L4 modern I2C (register-fidelity latching; no engine) ───────────────
#[derive(serde::Serialize)]
pub struct L4I2c {
    cr1: u32,
    cr2: u32,
    oar1: u32,
    oar2: u32,
    timingr: u32,
    timeoutr: u32,
    isr: u32,
    icr: u32,
    pecr: u32,
    rxdr: u32,
    txdr: u32,
    #[serde(skip)]
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
}

impl Default for L4I2c {
    fn default() -> Self {
        Self {
            cr1: 0,
            cr2: 0,
            oar1: 0,
            oar2: 0,
            timingr: 0,
            timeoutr: 0,
            isr: 0x0000_0001, // TXE=1 at reset
            icr: 0,
            pecr: 0,
            rxdr: 0,
            txdr: 0,
            attached_devices: Vec::new(),
        }
    }
}

impl L4I2c {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
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
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr1 = value & 0x00FF_E1FF,
            0x04 => {
                self.cr2 = value;
                if (value & (1 << 13)) != 0 {
                    self.isr |= 1 << 15; // START → BUSY
                }
                if (value & (1 << 14)) != 0 {
                    self.isr &= !(1 << 15); // STOP → clear BUSY
                }
            }
            0x08 => self.oar1 = value,
            0x0C => self.oar2 = value,
            0x10 => self.timingr = value,
            0x14 => self.timeoutr = value,
            0x18 => {
                let rw_mask: u32 = 0x0000_0001; // TXE is RW
                self.isr = (self.isr & !rw_mask) | (value & rw_mask);
            }
            0x1C => {
                let clearable: u32 = 0x0000_3F38;
                self.isr &= !(value & clearable);
                self.icr = 0;
            }
            0x20 => self.pecr = value,
            0x24 => self.rxdr = value & 0xFF,
            0x28 => {
                self.txdr = value & 0xFF;
                self.isr &= !0x0000_0003; // writing TXDR clears TXE+TXIS
            }
            _ => {}
        }
    }

    fn read(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
    }

    fn write(&mut self, offset: u64, value: u8) {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);
        let mask: u32 = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);
        self.write_reg(reg_offset, reg_val);
    }

    fn peek(&self, offset: u64) -> u8 {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        if byte_offset < 2 {
            ((reg_val >> (byte_offset * 8)) & 0xFF) as u8
        } else {
            0
        }
    }
}

/// I2C peripheral — one variant per chip family. Register sets fully isolated.
#[derive(serde::Serialize)]
pub enum I2c {
    Stm32F1(F1I2c),
    Stm32L4(L4I2c),
}

impl core::fmt::Debug for I2c {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            I2c::Stm32F1(i) => f.debug_struct("I2c::F1").field("state", &i.state).finish(),
            I2c::Stm32L4(_) => f.debug_struct("I2c::L4").finish(),
        }
    }
}

impl Default for I2c {
    fn default() -> Self {
        Self::Stm32F1(F1I2c::default())
    }
}

impl I2c {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_with_layout(layout: I2cRegisterLayout) -> Self {
        match layout {
            I2cRegisterLayout::Stm32F1 => Self::Stm32F1(F1I2c::default()),
            I2cRegisterLayout::Stm32L4 => Self::Stm32L4(L4I2c::default()),
        }
    }

    pub fn attach(&mut self, device: Box<dyn I2cDevice>) {
        match self {
            Self::Stm32F1(i) => i.attached_devices.push(RefCell::new(device)),
            Self::Stm32L4(i) => i.attached_devices.push(RefCell::new(device)),
        }
    }

    /// Attached I2C devices (used by config/bus validation + tests).
    pub fn attached_devices(&self) -> &[RefCell<Box<dyn I2cDevice>>] {
        match self {
            Self::Stm32F1(i) => &i.attached_devices,
            Self::Stm32L4(i) => &i.attached_devices,
        }
    }
}

impl crate::Peripheral for I2c {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(match self {
            Self::Stm32F1(i) => i.read(offset),
            Self::Stm32L4(i) => i.read(offset),
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match self {
            Self::Stm32F1(i) => i.write(offset, value),
            Self::Stm32L4(i) => i.write(offset, value),
        }
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        let irq = match self {
            Self::Stm32F1(i) => i.tick(),
            Self::Stm32L4(_) => false, // L4 has no transaction engine
        };
        crate::PeripheralTickResult {
            irq,
            cycles: 0,
            ..Default::default()
        }
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some(match self {
            Self::Stm32F1(i) => i.peek(offset),
            Self::Stm32L4(i) => i.peek(offset),
        })
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        match self {
            Self::Stm32F1(i) => serde_json::to_value(i),
            Self::Stm32L4(i) => serde_json::to_value(i),
        }
        .unwrap_or(serde_json::Value::Null)
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
        assert_eq!(i2c.read(0x00).unwrap(), 0); // CR1
        assert_eq!(i2c.read(0x04).unwrap(), 0); // CR2
    }

    #[test]
    fn test_i2c_start_bit() {
        let mut i2c = I2c::new();
        i2c.write(0x01, 0x01).unwrap(); // CR1 SB (bit 8)
        assert_eq!(i2c.peek(0x14).unwrap() & 0x01, 0); // not immediate
        for _ in 0..10 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0); // SB set after ticks
    }

    #[test]
    fn test_i2c_full_transfer_flow() {
        use crate::peripherals::components::Mpu6050;
        let mut i2c = I2c::new();
        i2c.attach(Box::new(Mpu6050::new(0x50)));

        i2c.write(0x01, 0x01).unwrap(); // START
        for _ in 0..10 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0); // SB

        i2c.write(0x10, 0xA0).unwrap(); // addr 0x50<<1 | W
        for _ in 0..20 {
            i2c.tick();
        }
        assert_eq!(i2c.peek(0x14).unwrap() & 0x01, 0); // SB cleared
        assert_ne!(i2c.peek(0x14).unwrap() & 0x02, 0); // ADDR
        assert_ne!(i2c.peek(0x18).unwrap() & 0x01, 0); // MSL

        i2c.write(0x10, 0x42).unwrap();
        for _ in 0..20 {
            i2c.tick();
        }
        assert_ne!(i2c.peek(0x14).unwrap() & 0x80, 0); // TXE
        assert_ne!(i2c.peek(0x14).unwrap() & 0x04, 0); // BTF

        i2c.write(0x01, 0x02).unwrap(); // STOP (bit 9)
        for _ in 0..10 {
            i2c.tick();
        }
        assert_eq!(
            i2c.peek(0x18).unwrap() & 0x03,
            0,
            "STOP must clear MSL+BUSY"
        );
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
