// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Nuvoton NPCX SMBus master controller, matching Renode's `NPCX_SMBus`.
//!
//! The eight NPCX9 instances are byte registers on two-byte offsets in a
//! 0x20-byte window. This model covers the common register bank, bank select,
//! master START/address/data/STOP flow, attached I²C device reads and writes,
//! address NACK, and the level interrupt predicates. It does not model slave
//! mode, clock timing/arbitration, SMBus protocol extensions, or FIFO burst
//! semantics. The FIFO register bank is exposed for configuration/status, but
//! transfers remain byte-at-a-time.
//!
//! Register offsets and transaction ordering follow Renode's
//! `NPCX_SMBus.cs`; the NPCX9 bases and IRQs follow `nuvoton_npcx9.repl`.

use crate::peripherals::device::{drain_i2c_pin_drives, I2cDevice, I2cSlaveHost};
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::RefCell;

const DATA: u64 = 0x00;
const STATUS: u64 = 0x02;
const CONTROL_STATUS: u64 = 0x04;
const CONTROL1: u64 = 0x06;
const OWN_ADDRESS1: u64 = 0x08;
const CONTROL2: u64 = 0x0A;
const OWN_ADDRESS2: u64 = 0x0C;
const CONTROL3: u64 = 0x0E;
const BUS_TIMEOUT: u64 = 0x0F;

const START: u8 = 1 << 0;
const STOP: u8 = 1 << 1;
const INTERRUPTS_ENABLED: u8 = 1 << 2;
const NEGATIVE_ACK: u8 = 1 << 4;
const READY_FOR_TRANSACTION: u8 = 1 << 6;
const MODULE_ENABLED: u8 = 1;
const BANK_SELECT: u8 = 1 << 5;
const FIFO_ENABLE: u8 = 1 << 4;
const FIFO_CAPACITY: usize = 32;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum State {
    #[default]
    Idle,
    Start,
    Reading,
    Writing,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Registers {
    state: State,
    data: u8,
    control1: u8,
    control2: u8,
    control3: u8,
    negative_ack: bool,
    ready: bool,
    module_enabled: bool,
    fifo_enabled: bool,
    rx_fifo_threshold: u8,
    rx_fifo_threshold_interrupt: bool,
    rx_fifo_threshold_status: bool,
    rx_full_tx_empty_interrupt: bool,
    rx_full_tx_empty_status: bool,
    last_packet: bool,
    bank0: [u8; 0x20],
    bank1: [u8; 0x20],
    tx_queue: Vec<u8>,
    rx_queue: Vec<u8>,
    selected: Option<usize>,
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            state: State::Idle,
            data: 0,
            control1: 0,
            control2: 0,
            control3: 0,
            negative_ack: false,
            ready: false,
            module_enabled: false,
            fifo_enabled: false,
            rx_fifo_threshold: 0,
            rx_fifo_threshold_interrupt: false,
            rx_fifo_threshold_status: false,
            rx_full_tx_empty_interrupt: false,
            rx_full_tx_empty_status: false,
            last_packet: false,
            bank0: [0; 0x20],
            bank1: [0; 0x20],
            tx_queue: Vec::new(),
            rx_queue: Vec::new(),
            selected: None,
        }
    }
}

/// NPCX SMBus controller. Instances attach ordinary LabWired `I2cDevice`
/// models through the system manifest's `external_devices` connection.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct NpcxSmbus {
    regs: RefCell<Registers>,
    #[serde(skip)]
    slaves: Vec<Box<dyn I2cDevice>>,
}

impl std::fmt::Debug for NpcxSmbus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NpcxSmbus")
            .field("regs", &self.regs.borrow())
            .field("slaves", &self.slaves.len())
            .finish()
    }
}

impl Default for NpcxSmbus {
    fn default() -> Self {
        Self::new()
    }
}

impl NpcxSmbus {
    pub fn new() -> Self {
        Self {
            regs: RefCell::new(Registers::default()),
            slaves: Vec::new(),
        }
    }

    fn first_bank_selected(regs: &Registers) -> bool {
        !regs.fifo_enabled || regs.control3 & BANK_SELECT == 0
    }

    fn irq_active(regs: &Registers) -> bool {
        regs.control1 & INTERRUPTS_ENABLED != 0
            && (regs.negative_ack
                || regs.ready
                || (regs.rx_fifo_threshold_interrupt && regs.rx_fifo_threshold_status)
                || (regs.rx_full_tx_empty_interrupt && regs.rx_full_tx_empty_status))
    }

    fn status(regs: &Registers) -> u8 {
        let mut value = 0;
        if regs.state == State::Writing {
            value |= 1; // XMIT
        }
        if regs.state != State::Idle {
            value |= 1 << 1; // MASTER
        }
        if regs.negative_ack {
            value |= NEGATIVE_ACK;
        }
        if regs.ready {
            value |= READY_FOR_TRANSACTION;
        }
        value
    }

    fn control_status(regs: &Registers) -> u8 {
        let mut value = u8::from(regs.state != State::Idle); // BUSY
                                                             // The Renode model reports both idle bus lines high.
        if regs.state == State::Idle {
            value |= (1 << 6) | (1 << 7);
        }
        value
    }

    fn read_byte(regs: &mut Registers, offset: u64) -> u8 {
        match offset {
            DATA => {
                if let Some(value) = regs.rx_queue.first().copied() {
                    regs.rx_queue.remove(0);
                    regs.data = value;
                    if regs.rx_queue.is_empty() {
                        regs.ready = false;
                    }
                    Self::update_fifo_status(regs);
                    value
                } else {
                    regs.ready = false;
                    0
                }
            }
            STATUS => Self::status(regs),
            CONTROL_STATUS => Self::control_status(regs),
            CONTROL1 => regs.control1,
            OWN_ADDRESS1 => regs.bank0[OWN_ADDRESS1 as usize],
            CONTROL2 => regs.control2,
            OWN_ADDRESS2 => regs.bank0[OWN_ADDRESS2 as usize],
            CONTROL3 => regs.control3,
            BUS_TIMEOUT => regs.bank0[BUS_TIMEOUT as usize],
            0x10..=0x1F => {
                if Self::first_bank_selected(regs) {
                    regs.bank0[offset as usize]
                } else {
                    Self::read_fifo_bank(regs, offset)
                }
            }
            _ => 0,
        }
    }

    fn peek_byte(regs: &Registers, offset: u64) -> u8 {
        match offset {
            DATA => regs.rx_queue.first().copied().unwrap_or(regs.data),
            STATUS => Self::status(regs),
            CONTROL_STATUS => Self::control_status(regs),
            CONTROL1 => regs.control1,
            OWN_ADDRESS1 => regs.bank0[OWN_ADDRESS1 as usize],
            CONTROL2 => regs.control2,
            OWN_ADDRESS2 => regs.bank0[OWN_ADDRESS2 as usize],
            CONTROL3 => regs.control3,
            BUS_TIMEOUT => regs.bank0[BUS_TIMEOUT as usize],
            0x10..=0x1F if Self::first_bank_selected(regs) => regs.bank0[offset as usize],
            0x10..=0x1F => Self::peek_fifo_bank(regs, offset),
            _ => 0,
        }
    }

    fn read_fifo_bank(regs: &Registers, offset: u64) -> u8 {
        match offset {
            0x10 => {
                u8::from(regs.rx_full_tx_empty_status) << 1
                    | u8::from(regs.rx_full_tx_empty_interrupt) << 3
            }
            0x12 => regs.bank1[0x12],
            0x13 => regs.bank1[0x13],
            0x14 => regs.bank1[0x14],
            0x16 => regs.bank1[0x16],
            0x1A => (regs.tx_queue.len().min(0x3F) as u8) | (regs.bank1[0x1A] & (1 << 6)),
            0x1C => {
                (regs.rx_queue.len().min(0x3F) as u8)
                    | (u8::from(regs.rx_fifo_threshold_status) << 6)
            }
            0x1E => {
                (regs.rx_fifo_threshold & 0x3F)
                    | (u8::from(regs.rx_fifo_threshold_interrupt) << 6)
                    | (u8::from(regs.last_packet) << 7)
            }
            _ => regs.bank1[offset as usize],
        }
    }

    fn peek_fifo_bank(regs: &Registers, offset: u64) -> u8 {
        match offset {
            0x10 => {
                u8::from(regs.rx_full_tx_empty_status) << 1
                    | u8::from(regs.rx_full_tx_empty_interrupt) << 3
            }
            0x1A => (regs.tx_queue.len().min(0x3F) as u8) | (regs.bank1[0x1A] & (1 << 6)),
            0x1C => {
                (regs.rx_queue.len().min(0x3F) as u8)
                    | (u8::from(regs.rx_fifo_threshold_status) << 6)
            }
            0x1E => {
                (regs.rx_fifo_threshold & 0x3F)
                    | (u8::from(regs.rx_fifo_threshold_interrupt) << 6)
                    | (u8::from(regs.last_packet) << 7)
            }
            _ => regs.bank1[offset as usize],
        }
    }

    fn update_fifo_status(regs: &mut Registers) {
        regs.rx_full_tx_empty_status |= regs.rx_queue.len() == FIFO_CAPACITY;
        regs.rx_fifo_threshold_status |= regs.fifo_enabled
            && !regs.rx_queue.is_empty()
            && regs.rx_queue.len() == usize::from(regs.rx_fifo_threshold);
    }

    fn write_byte(regs: &mut Registers, slaves: &mut [Box<dyn I2cDevice>], offset: u64, value: u8) {
        match offset {
            DATA => Self::write_data(regs, slaves, value),
            STATUS if value & NEGATIVE_ACK != 0 => regs.negative_ack = false,
            STATUS => {}
            CONTROL1 => Self::write_control1(regs, slaves, value),
            OWN_ADDRESS1 => regs.bank0[OWN_ADDRESS1 as usize] = value,
            CONTROL2 => {
                regs.control2 = value;
                regs.module_enabled = value & MODULE_ENABLED != 0;
            }
            OWN_ADDRESS2 => regs.bank0[OWN_ADDRESS2 as usize] = value,
            CONTROL3 => regs.control3 = value,
            BUS_TIMEOUT => regs.bank0[BUS_TIMEOUT as usize] = value,
            0x10..=0x1F if Self::first_bank_selected(regs) => {
                regs.bank0[offset as usize] = value;
                if offset == 0x1D {
                    regs.fifo_enabled = value & FIFO_ENABLE != 0;
                }
            }
            0x10..=0x1F => Self::write_fifo_bank(regs, offset, value),
            _ => {}
        }
    }

    fn write_fifo_bank(regs: &mut Registers, offset: u64, value: u8) {
        match offset {
            0x10 => {
                if value & (1 << 1) != 0 {
                    regs.rx_full_tx_empty_status = false;
                }
                regs.rx_full_tx_empty_interrupt = value & (1 << 3) != 0;
                if value & (1 << 6) != 0 {
                    regs.rx_queue.clear();
                    regs.tx_queue.clear();
                }
            }
            0x12 | 0x13 | 0x14 | 0x16 | 0x1A => regs.bank1[offset as usize] = value,
            0x1C if value & (1 << 6) != 0 => regs.rx_fifo_threshold_status = false,
            0x1C => {}
            0x1E => {
                regs.rx_fifo_threshold = value & 0x3F;
                regs.rx_fifo_threshold_interrupt = value & (1 << 6) != 0;
                regs.last_packet = value & (1 << 7) != 0;
            }
            _ => {}
        }
    }

    fn write_control1(regs: &mut Registers, slaves: &mut [Box<dyn I2cDevice>], value: u8) {
        regs.control1 = value & !((1 << 0) | (1 << 1));
        if value & START != 0 {
            Self::begin(regs, slaves);
        } else if value & STOP != 0 {
            Self::finish(regs, slaves);
        }
    }

    fn begin(regs: &mut Registers, slaves: &mut [Box<dyn I2cDevice>]) {
        if regs.state != State::Idle {
            Self::finish(regs, slaves);
        }
        if regs.module_enabled {
            regs.state = State::Start;
        }
        regs.ready = true;
    }

    fn write_data(regs: &mut Registers, slaves: &mut [Box<dyn I2cDevice>], value: u8) {
        match regs.state {
            State::Idle => {}
            State::Start => Self::address(regs, slaves, value),
            State::Writing => {
                if regs.tx_queue.len() < FIFO_CAPACITY {
                    regs.tx_queue.push(value);
                }
                regs.ready = true;
                regs.rx_full_tx_empty_status = true;
            }
            State::Reading => {}
        }
    }

    fn address(regs: &mut Registers, slaves: &mut [Box<dyn I2cDevice>], value: u8) {
        let address = value >> 1;
        let read = value & 1 != 0;
        let found = slaves
            .iter()
            .position(|slave| slave.claims_address(address));
        let Some(index) = found else {
            regs.negative_ack = true;
            regs.state = State::Idle;
            return;
        };

        slaves[index].select_address(address);
        slaves[index].start();
        regs.selected = Some(index);
        if read {
            regs.state = State::Reading;
            regs.rx_queue.push(slaves[index].read());
            regs.ready = true;
            Self::update_fifo_status(regs);
        } else {
            regs.state = State::Writing;
            regs.ready = true;
        }
    }

    fn finish(regs: &mut Registers, slaves: &mut [Box<dyn I2cDevice>]) {
        if let Some(index) = regs.selected.take() {
            for value in regs.tx_queue.drain(..) {
                slaves[index].write(value);
            }
            slaves[index].stop();
        } else {
            regs.tx_queue.clear();
        }
        regs.rx_queue.clear();
        regs.state = State::Idle;
        regs.ready = false;
    }

    fn reset_registers(&mut self) {
        let regs = self.regs.get_mut();
        if let Some(index) = regs.selected.take() {
            if let Some(slave) = self.slaves.get_mut(index) {
                slave.stop();
            }
        }
        *regs = Registers::default();
    }
}

impl I2cSlaveHost for NpcxSmbus {
    fn attach_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.slaves.push(device);
    }
}

impl Peripheral for NpcxSmbus {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(Self::read_byte(&mut self.regs.borrow_mut(), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        Self::write_byte(self.regs.get_mut(), &mut self.slaves, offset, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some(Self::peek_byte(&self.regs.borrow(), offset))
    }

    fn i2c_slave_host(&mut self) -> Option<&mut dyn I2cSlaveHost> {
        Some(self)
    }

    fn on_system_reset(&mut self) {
        self.reset_registers();
    }

    fn on_node_reset(&mut self) {
        self.reset_registers();
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(Self::irq_active(&self.regs.borrow()))
    }

    fn reads_can_deassert_irq(&self) -> bool {
        true
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        PeripheralTickResult {
            irq: Self::irq_active(&self.regs.borrow()),
            ..Default::default()
        }
    }

    fn legacy_tick_active(&self) -> bool {
        Self::irq_active(&self.regs.borrow())
    }

    fn drives_central_i2c_time(&self) -> bool {
        true
    }

    fn advance_attached_i2c_us(&mut self, us: u64) {
        for slave in &mut self.slaves {
            slave.advance_time_us(us);
        }
    }

    fn drain_attached_pin_drives(&mut self, out: &mut Vec<(String, String, bool)>) {
        for slave in &mut self.slaves {
            drain_i2c_pin_drives(&mut **slave, out);
        }
    }

    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for slave in &self.slaves {
            crate::inspect::visit_i2c_device(&**slave, f);
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("NpcxSmbus snapshot decode: {error}"))
        })?;
        let slaves = std::mem::take(&mut self.slaves);
        *self = restored;
        self.slaves = slaves;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Probe {
        address: u8,
        reads: Vec<u8>,
        writes: Arc<Mutex<Vec<u8>>>,
        stops: Arc<Mutex<u32>>,
    }

    impl I2cDevice for Probe {
        fn address(&self) -> u8 {
            self.address
        }
        fn read(&mut self) -> u8 {
            if self.reads.is_empty() {
                0xFF
            } else {
                self.reads.remove(0)
            }
        }
        fn write(&mut self, value: u8) {
            self.writes.lock().unwrap().push(value);
        }
        fn stop(&mut self) {
            *self.stops.lock().unwrap() += 1;
        }
    }

    fn attach_probe(smbus: &mut NpcxSmbus) -> (Arc<Mutex<Vec<u8>>>, Arc<Mutex<u32>>) {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let stops = Arc::new(Mutex::new(0));
        smbus.attach_slave(Box::new(Probe {
            address: 0x50,
            reads: vec![0xA5],
            writes: writes.clone(),
            stops: stops.clone(),
        }));
        (writes, stops)
    }

    fn start(smbus: &mut NpcxSmbus) {
        smbus.write(CONTROL2, MODULE_ENABLED).unwrap();
        smbus.write(CONTROL1, INTERRUPTS_ENABLED | START).unwrap();
    }

    #[test]
    fn master_write_and_read_use_attached_device() {
        let mut smbus = NpcxSmbus::new();
        let (writes, stops) = attach_probe(&mut smbus);

        start(&mut smbus);
        smbus.write(DATA, 0xA0).unwrap(); // address 0x50, write
        smbus.write(DATA, 0x12).unwrap();
        smbus.write(DATA, 0x34).unwrap();
        smbus.write(CONTROL1, INTERRUPTS_ENABLED | STOP).unwrap();
        assert_eq!(*writes.lock().unwrap(), vec![0x12, 0x34]);
        assert_eq!(*stops.lock().unwrap(), 1);

        start(&mut smbus);
        smbus.write(DATA, 0xA1).unwrap(); // address 0x50, read
        assert_eq!(smbus.read(DATA).unwrap(), 0xA5);
        smbus.write(CONTROL1, INTERRUPTS_ENABLED | STOP).unwrap();
        assert_eq!(*stops.lock().unwrap(), 2);
    }

    #[test]
    fn absent_device_nacks_and_interrupt_is_level_cleared() {
        let mut smbus = NpcxSmbus::new();
        start(&mut smbus);
        smbus.write(DATA, 0xA0).unwrap();
        assert_eq!(smbus.read(STATUS).unwrap() & NEGATIVE_ACK, NEGATIVE_ACK);
        assert_eq!(smbus.irq_line_level(), Some(true));

        smbus.write(STATUS, NEGATIVE_ACK).unwrap();
        assert_eq!(smbus.read(STATUS).unwrap() & NEGATIVE_ACK, 0);
        assert_eq!(smbus.irq_line_level(), Some(true)); // START's SDAST is still set.
        smbus.write(CONTROL1, INTERRUPTS_ENABLED | STOP).unwrap();
        assert_eq!(smbus.irq_line_level(), Some(false));
    }

    #[test]
    fn fifo_bank_selection_changes_the_aliased_register_window() {
        let mut smbus = NpcxSmbus::new();
        smbus.write(0x10, 0x33).unwrap(); // bank-0 own address 3
        smbus.write(0x1D, FIFO_ENABLE).unwrap();
        smbus.write(CONTROL3, BANK_SELECT).unwrap();
        smbus.write(0x10, 1 << 3).unwrap(); // bank-1 interrupt enable
        assert_eq!(smbus.read(0x10).unwrap(), 1 << 3);
        smbus.write(CONTROL3, 0).unwrap();
        assert_eq!(smbus.read(0x10).unwrap(), 0x33);
    }
}
