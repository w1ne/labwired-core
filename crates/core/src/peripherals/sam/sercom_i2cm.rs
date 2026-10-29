// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Opt-in SAMD51 SERCOM I2C master slice (DS60001507F 36): 7-bit addressing,
//! writes, smart-mode reads, repeated start, STOP and the MB/SB/RXNACK
//! handshake against attached `I2cDevice` slaves. Each bus operation completes
//! on the next bus tick and only while the SERCOM's bus clock and core generic
//! clock run and a PAD0 (SDA) and PAD1 (SCL) pad are muxed to the SERCOM; an
//! address nobody claims is NACKed. When the core clock resolves on the live
//! clock tree, an operation completes after whole SCL periods of
//! `10 + BAUD.BAUD + BAUD.BAUDLOW` core-clock cycles (BAUDLOW 0 uses BAUD for
//! both halves; DS60001507F 36.6.2.4.1 with t_rise = 0, the rise time being a
//! board electrical property): a START plus nine bits for an address, nine
//! more for the first byte of a read, nine per written or read byte and one
//! for a STOP. Clock stretching, arbitration, timeouts, 10-bit/high-speed
//! modes, DMA and the slave mode are not modelled; their settings are refused.
use crate::peripherals::device::{I2cDevice, I2cSlaveHost};
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;
use std::cell::{Cell, RefCell};

const MB: u8 = 1;
const SB: u8 = 1 << 1;
const ERROR: u8 = 1 << 7;
const BUSERR: u16 = 1;
const RXNACK: u16 = 1 << 2;
const LOWTOUT: u16 = 1 << 6;
/// CTRLA.LOWTOUTEN: SCL held low beyond the time-out ends the transfer.
const LOWTOUTEN: u32 = 1 << 30;
/// SCL low time-out: 25-35 ms (DS60001507F 36.6.2.4.6); the model takes the
/// earliest, 25 ms.
const LOW_TIMEOUT_NS: u64 = 25_000_000;
const BUSSTATE: u16 = 3 << 4;
const IDLE: u16 = 1 << 4;
const OWNER: u16 = 2 << 4;
const MODE_I2C_MASTER: u32 = 5 << 2;

#[derive(Clone, Copy, Debug)]
pub struct SercomInstance {
    pub base: u64,
    pub first_irq: u32,
    pub core_gclk: u64,
    /// (MCLK mask register offset, bit)
    pub bus_clock: (u64, u32),
}

/// SERCOM0..SERCOM5 on the SAMD51J19A (ATDF instance parameters).
pub const SERCOM_INSTANCES: [SercomInstance; 6] = [
    SercomInstance {
        base: 0x40003000,
        first_irq: 46,
        core_gclk: 7,
        bus_clock: (0x14, 12),
    },
    SercomInstance {
        base: 0x40003400,
        first_irq: 50,
        core_gclk: 8,
        bus_clock: (0x14, 13),
    },
    SercomInstance {
        base: 0x41012000,
        first_irq: 54,
        core_gclk: 23,
        bus_clock: (0x18, 9),
    },
    SercomInstance {
        base: 0x41014000,
        first_irq: 58,
        core_gclk: 24,
        bus_clock: (0x18, 10),
    },
    SercomInstance {
        base: 0x43000000,
        first_irq: 62,
        core_gclk: 34,
        bus_clock: (0x20, 0),
    },
    SercomInstance {
        base: 0x43000400,
        first_irq: 66,
        core_gclk: 35,
        bus_clock: (0x20, 1),
    },
];

/// (SERCOM, PAD, port, pin, PMUX function) for SERCOM PAD0/PAD1 on the
/// ATSAMD51J19A (ATDF, Microchip SAMD51 DFP 3.9.276, Apache-2.0); C = 2, D = 3.
const PADS: [(u8, u8, u8, u8, u8); 30] = [
    (0, 0, 0, 4, 3),
    (0, 0, 0, 8, 2),
    (0, 1, 0, 5, 3),
    (0, 1, 0, 9, 2),
    (1, 0, 0, 0, 3),
    (1, 0, 0, 16, 2),
    (1, 1, 0, 1, 3),
    (1, 1, 0, 17, 2),
    (2, 0, 0, 9, 3),
    (2, 0, 0, 12, 2),
    (2, 1, 0, 8, 3),
    (2, 1, 0, 13, 2),
    (3, 0, 0, 17, 3),
    (3, 0, 0, 22, 2),
    (3, 1, 0, 16, 3),
    (3, 1, 0, 23, 2),
    (4, 0, 0, 13, 3),
    (4, 0, 1, 8, 3),
    (4, 0, 1, 12, 2),
    (4, 1, 0, 12, 3),
    (4, 1, 1, 9, 3),
    (4, 1, 1, 13, 2),
    (5, 0, 0, 23, 3),
    (5, 0, 1, 2, 3),
    (5, 0, 1, 16, 2),
    (5, 0, 1, 31, 3),
    (5, 1, 0, 22, 3),
    (5, 1, 1, 3, 3),
    (5, 1, 1, 17, 2),
    (5, 1, 1, 30, 3),
];

/// True when some pad carrying `pad` of SERCOM `sercom` is muxed to it.
fn pad_routed(bus: &dyn Bus, sercom: u8, pad: u8) -> bool {
    PADS.iter()
        .filter(|r| r.0 == sercom && r.1 == pad)
        .any(|&(_, _, port, pin, function)| {
            let group = 0x41008000 + 0x80 * u64::from(port);
            let pincfg = bus.read_u8(group + 0x40 + u64::from(pin)).unwrap_or(0);
            let pmux = bus.read_u8(group + 0x30 + u64::from(pin / 2)).unwrap_or(0);
            let selected = if pin & 1 != 0 { pmux >> 4 } else { pmux & 0xf };
            pincfg & 1 != 0 && selected == function
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Address(u8),
    Write(u8),
    ReadNext,
    Stop,
}

/// Core-clock cycles per SCL period: `10 + BAUD + BAUDLOW`, BAUDLOW 0 meaning
/// BAUD for both halves (DS60001507F 36.6.2.4.1, t_rise = 0).
fn scl_clocks(baud: u32) -> u64 {
    let high = u64::from(baud & 0xff);
    let low = match u64::from((baud >> 8) & 0xff) {
        0 => high,
        low => low,
    };
    10 + high + low
}

/// CPU cycles of `periods` SCL periods at BAUD, the rise time and the core
/// clock, plus `stretch_ns` of SCL held low by the device.
fn op_cycles(
    periods: u64,
    baud: u32,
    rise_ns: u64,
    stretch_ns: u64,
    gclk: u64,
    cpu: u64,
) -> Option<u64> {
    if gclk == 0 {
        return None;
    }
    // Core clocks x 1e9 per period, so the rise term stays exact.
    let period =
        u128::from(scl_clocks(baud)) * 1_000_000_000 + u128::from(gclk) * u128::from(rise_ns);
    let bus =
        (u128::from(periods) * period * u128::from(cpu)).div_ceil(u128::from(gclk) * 1_000_000_000);
    let stretch = (u128::from(stretch_ns) * u128::from(cpu)).div_ceil(1_000_000_000);
    Some((bus + stretch) as u64)
}

pub struct Samd51SercomI2cm {
    instance: SercomInstance,
    ctrla: u32,
    ctrlb: u32,
    baud: u32,
    inten: u8,
    intflag: Cell<u8>,
    status: u16,
    addr: u32,
    data: Cell<u8>,
    reading: bool,
    selected: Option<usize>,
    pending: Cell<Option<Op>>,
    devices: Vec<RefCell<Box<dyn I2cDevice>>>,
    transactions: u64,
    /// The first address bytes put on the bus (evidence, bounded).
    addresses: Vec<u8>,
    clock: Option<crate::CycleClock>,
    /// Cycle at which the pending operation completes on the bus.
    deadline: Option<u64>,
    /// An address written while a STOP was pending; issued after it.
    queued: Option<Op>,
    completed: u64,
    /// SCL rise time the board declares (`scl_rise_ns`): each SCL period is
    /// `10 + BAUD + BAUDLOW + f_core * t_rise` core clocks (DS60001507F 36.6.2.4.1).
    rise_ns: u64,
}

impl Samd51SercomI2cm {
    pub fn new(instance: SercomInstance) -> Self {
        Self {
            instance,
            ctrla: 0,
            ctrlb: 0,
            baud: 0,
            inten: 0,
            intflag: Cell::new(0),
            status: 0,
            addr: 0,
            data: Cell::new(0),
            reading: false,
            selected: None,
            pending: Cell::new(None),
            devices: Vec::new(),
            transactions: 0,
            addresses: Vec::new(),
            clock: None,
            deadline: None,
            queued: None,
            completed: 0,
            rise_ns: 0,
        }
    }
    /// The board's SCL rise time in nanoseconds.
    pub fn with_scl_rise_ns(mut self, rise_ns: u64) -> Self {
        self.rise_ns = rise_ns;
        self
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd51 SERCOM I2C master slice: {what}"
        )))
    }
    fn enabled(&self) -> bool {
        self.ctrla & 2 != 0
    }
    pub fn transactions(&self) -> u64 {
        self.transactions
    }
    /// An operation was issued and has not completed on the bus yet.
    pub fn busy(&self) -> bool {
        self.pending.get().is_some()
    }
    /// Operations completed on the bus so far (evidence for host timing).
    pub fn completed(&self) -> u64 {
        self.completed
    }
    fn reset(&mut self) {
        // SWRST resets registers; attached devices and the run's transaction
        // count (evidence, not register state) survive it.
        let devices = std::mem::take(&mut self.devices);
        let (transactions, addresses) = (self.transactions, std::mem::take(&mut self.addresses));
        let clock = self.clock.take();
        *self = Self::new(self.instance).with_scl_rise_ns(self.rise_ns);
        self.clock = clock;
        self.devices = devices;
        self.transactions = transactions;
        self.addresses = addresses;
    }
    fn stop_selected(&mut self) {
        if let Some(i) = self.selected.take() {
            self.devices[i].borrow_mut().stop();
        }
        self.reading = false;
        self.status = (self.status & !BUSSTATE) | IDLE;
    }
    fn perform(&mut self, op: Op) {
        match op {
            Op::Address(byte) => {
                if let Some(i) = self.selected.take() {
                    self.devices[i].borrow_mut().stop(); // repeated start ends the previous transfer
                }
                let address = byte >> 1;
                self.reading = byte & 1 != 0;
                self.status = (self.status & !(BUSSTATE | RXNACK)) | OWNER;
                self.transactions += 1;
                if self.addresses.len() < 32 {
                    self.addresses.push(byte);
                }
                match self
                    .devices
                    .iter()
                    .position(|d| d.borrow().claims_address(address))
                {
                    Some(i) => {
                        let mut d = self.devices[i].borrow_mut();
                        d.select_address(address);
                        d.start();
                        if self.reading {
                            self.data.set(d.read());
                            self.intflag.set(self.intflag.get() | SB);
                        } else {
                            self.intflag.set(self.intflag.get() | MB);
                        }
                        drop(d);
                        self.selected = Some(i);
                    }
                    None => {
                        // Address NACK: MB with RXNACK, for both directions.
                        self.status |= RXNACK;
                        self.intflag.set(self.intflag.get() | MB);
                    }
                }
            }
            Op::Write(byte) => match self.selected {
                Some(i) if !self.reading => {
                    self.devices[i].borrow_mut().write(byte);
                    self.status &= !RXNACK;
                    self.intflag.set(self.intflag.get() | MB);
                }
                _ => self.intflag.set(self.intflag.get() | ERROR),
            },
            Op::ReadNext => {
                if let (Some(i), true) = (self.selected, self.reading) {
                    self.data.set(self.devices[i].borrow_mut().read());
                    self.intflag.set(self.intflag.get() | SB);
                }
            }
            Op::Stop => self.stop_selected(),
        }
    }
    /// The pending operation `op` reaches the end of its bus time.
    fn complete(&mut self, op: Op) {
        self.pending.set(self.queued.take());
        self.completed += 1;
        let timed_out = self.times_out(op);
        self.perform(op);
        if timed_out {
            // The master releases SCL, completes the operation with MB or SB
            // as normal, sends STOP, and reports LOWTOUT and BUSERR (ERROR).
            self.status |= LOWTOUT | BUSERR;
            self.intflag.set(self.intflag.get() | ERROR);
            self.stop_selected();
        }
    }
    fn command(&mut self, cmd: u32) {
        match cmd {
            2 if self.reading => self.pending.set(Some(Op::ReadNext)),
            3 => self.pending.set(Some(Op::Stop)),
            _ => {}
        }
    }
    /// CPU cycles `op` holds the bus, or `None` when the core clock or the CPU
    /// clock does not resolve (the operation then completes on the next tick).
    fn duration(&self, bus: &dyn Bus, op: Op) -> Option<u64> {
        let gclk = super::clock_tree::peripheral_hz(bus, self.instance.core_gclk)?;
        let cpu = super::clock_tree::cpu_hz(bus)?;
        // The device that stretches: the one claiming the address, or the
        // selected one for its bytes. A NACKed address and STOP stretch nothing.
        let (periods, device) = match op {
            Op::Address(byte) => {
                let claimed = self
                    .devices
                    .iter()
                    .position(|d| d.borrow().claims_address(byte >> 1));
                match claimed {
                    // The read's first byte follows its address.
                    Some(i) if byte & 1 != 0 => (19, Some((i, 2))),
                    Some(i) => (10, Some((i, 1))),
                    None => (10, None),
                }
            }
            Op::Write(_) | Op::ReadNext => (9, self.selected.map(|i| (i, 1))),
            Op::Stop => (1, None),
        };
        let stretch = device.map_or(0, |(i, holds)| {
            let ns = self.devices[i].borrow().clock_stretch_ns();
            // A hold beyond the time-out is cut short there, once.
            if self.low_timeout_enabled() && ns > LOW_TIMEOUT_NS {
                LOW_TIMEOUT_NS
            } else {
                ns * holds
            }
        });
        op_cycles(periods, self.baud, self.rise_ns, stretch, gclk, cpu)
    }
    fn low_timeout_enabled(&self) -> bool {
        self.ctrla & LOWTOUTEN != 0
    }
    /// Whether `op` ends in an SCL low time-out: the addressed (or selected)
    /// device holds SCL longer than the time-out while it is enabled.
    fn times_out(&self, op: Op) -> bool {
        let device = match op {
            Op::Address(byte) => self
                .devices
                .iter()
                .position(|d| d.borrow().claims_address(byte >> 1)),
            Op::Write(_) | Op::ReadNext => self.selected,
            Op::Stop => None,
        };
        self.low_timeout_enabled()
            && device.is_some_and(|i| self.devices[i].borrow().clock_stretch_ns() > LOW_TIMEOUT_NS)
    }
    fn lines(&self) -> u8 {
        if self.enabled() {
            self.intflag.get() & self.inten
        } else {
            0
        }
    }
    fn read_reg(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            0x00 => self.ctrla,
            0x04 => self.ctrlb,
            0x08 => 0,
            0x0c => self.baud,
            0x14 | 0x16 => u32::from(self.inten),
            0x18 => u32::from(self.intflag.get()),
            0x1a => u32::from(self.status),
            0x1c => 0, // SYNCBUSY: SWRST/ENABLE/SYSOP complete immediately
            0x24 => self.addr,
            0x28 => {
                // Smart mode: reading DATA acknowledges and fetches the next
                // byte unless ACKACT selects NACK for the last one.
                let value = self.data.get();
                if self.ctrlb & (1 << 8) != 0 && self.reading && self.ctrlb & (1 << 18) == 0 {
                    self.intflag.set(self.intflag.get() & !SB);
                    self.pending.set(Some(Op::ReadNext));
                }
                u32::from(value)
            }
            0x30 => 0,
            _ => return Self::refuse("register"),
        })
    }
    fn write_reg(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            0x00 if value & 1 != 0 => self.reset(),
            0x00 => {
                // I2C master mode; SDAHOLD, LOWTOUTEN and RUNSTDBY admitted;
                // PINOUT, the other timeouts, SCLSM and speed modes refused.
                if value & 0x1c != MODE_I2C_MASTER
                    || value & !(0x0030_0000 | LOWTOUTEN | 0x80 | 0x1e) != 0
                {
                    return Self::refuse("CTRLA outside standard-mode I2C master");
                }
                if self.enabled() && (value ^ self.ctrla) & !2 != 0 {
                    return Self::refuse("CTRLA is enable-protected");
                }
                if value & 2 == 0 {
                    self.stop_selected();
                    self.status &= !BUSSTATE; // disabled: bus state unknown
                }
                self.ctrla = value;
            }
            0x04 => {
                if value & !((1 << 8) | (3 << 16) | (1 << 18)) != 0 {
                    return Self::refuse("CTRLB quick command or reserved bits");
                }
                self.ctrlb = value & !(3 << 16);
                self.command((value >> 16) & 3);
            }
            0x0c => {
                if self.enabled() {
                    return Self::refuse("BAUD is enable-protected");
                }
                if value & !0xffff != 0 {
                    return Self::refuse("high-speed baud fields");
                }
                self.baud = value;
            }
            0x14 => self.inten &= !(value as u8),
            0x16 => self.inten |= value as u8 & (MB | SB | ERROR),
            0x18 => self.intflag.set(self.intflag.get() & !(value as u8)),
            0x1a => {
                // BUSSTATE is writable only to force IDLE; error bits are W1C.
                let state = value as u16 & BUSSTATE;
                if state != 0 && state != IDLE {
                    return Self::refuse("only IDLE may be forced into BUSSTATE");
                }
                if state == IDLE && self.enabled() {
                    self.status = (self.status & !BUSSTATE) | IDLE;
                }
                self.status &= !(value as u16 & 0x0747);
            }
            0x24 => {
                if !self.enabled() || value & !0x7ff != 0 || value & 0x700 != 0 {
                    return Self::refuse("ADDR needs an enabled master and a 7-bit address");
                }
                if self.status & BUSSTATE != IDLE && self.status & BUSSTATE != OWNER {
                    return Self::refuse("ADDR with an unknown bus state");
                }
                self.addr = value;
                self.intflag.set(self.intflag.get() & !(MB | SB));
                // A START issued while a STOP is still on the bus follows it.
                if self.pending.get() == Some(Op::Stop) {
                    self.queued = Some(Op::Address(value as u8));
                } else {
                    self.pending.set(Some(Op::Address(value as u8)));
                }
            }
            0x28 => {
                self.intflag.set(self.intflag.get() & !MB);
                self.pending.set(Some(Op::Write(value as u8)));
            }
            0x30 => {}
            _ => return Self::refuse("store outside the I2C master slice"),
        }
        Ok(())
    }
}

impl std::fmt::Debug for Samd51SercomI2cm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Samd51SercomI2cm")
            .field("base", &self.instance.base)
            .field("ctrla", &self.ctrla)
            .field("status", &self.status)
            .field("devices", &self.devices.len())
            .finish()
    }
}

impl I2cSlaveHost for Samd51SercomI2cm {
    fn attach_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.devices.push(RefCell::new(device));
    }
}

impl Peripheral for Samd51SercomI2cm {
    fn on_system_reset(&mut self) {
        self.stop_selected();
        self.reset();
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let base = match offset {
            0x00..=0x0f | 0x1c..=0x1f | 0x24..=0x27 => offset & !3,
            0x1a..=0x1b => 0x1a,
            _ => offset,
        };
        if base == 0x28 && offset != 0x28 {
            return Ok(0);
        }
        Ok((self.read_reg(base)? >> (8 * (offset - base))) as u8)
    }
    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(self.read_reg(offset)? as u16)
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.read_reg(offset)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            0x14 | 0x16 | 0x18 | 0x28 | 0x30 => self.write_reg(offset, u32::from(value)),
            _ => Self::refuse("byte store to a wide register"),
        }
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write_reg(offset, u32::from(value))
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset, value)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        // Inspection must not trigger the DATA read side effect.
        if offset == 0x28 {
            return Some(self.data.get());
        }
        self.read(offset).ok()
    }
    fn i2c_slave_host(&mut self) -> Option<&mut dyn I2cSlaveHost> {
        Some(self)
    }
    fn needs_bus_tick(&self) -> bool {
        // An open read stays ticked: a smart-mode DATA read issues the next
        // byte, and a read cannot put the master back on the bus-tick list.
        self.pending.get().is_some() || self.reading && self.selected.is_some()
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        let (mask, bit) = self.instance.bus_clock;
        let clocked = bus
            .read_u32(0x40000800 + mask)
            .is_ok_and(|m| m & (1 << bit) != 0)
            && bus
                .read_u32(0x40001c80 + 4 * self.instance.core_gclk)
                .is_ok_and(|c| c & (1 << 6) != 0);
        let sercom = SERCOM_INSTANCES
            .iter()
            .position(|i| i.base == self.instance.base)
            .unwrap_or(0) as u8;
        let routed = pad_routed(bus, sercom, 0) && pad_routed(bus, sercom, 1);
        if !clocked || !self.enabled() || !routed {
            // An unclocked, disabled or unrouted master drives no bus: the
            // operation stays pending and the guest sees no MB/SB.
            self.deadline = None;
            return;
        }
        let Some(op) = self.pending.get() else {
            return;
        };
        if let Some(now) = self.clock.as_ref().map(|c| c.now()) {
            if let Some(duration) = self.duration(bus, op) {
                let deadline = *self.deadline.get_or_insert(now + duration);
                if now < deadline {
                    return;
                }
            }
        }
        self.deadline = None;
        self.complete(op);
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.clock = Some(clock);
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn legacy_tick_active(&self) -> bool {
        self.lines() != 0
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        let lines = self.lines();
        report(self.instance.first_irq, lines & MB != 0);
        report(self.instance.first_irq + 1, lines & SB != 0);
        report(self.instance.first_irq + 3, lines & ERROR != 0);
    }
    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        let lines = self.lines();
        let mut irqs = vec![];
        for (flag, line) in [(MB, 0), (SB, 1), (ERROR, 3)] {
            if lines & flag != 0 {
                irqs.push(self.instance.first_irq + line);
            }
        }
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "samd51_sercom_i2cm", "base": self.instance.base, "ctrla": self.ctrla,
            "status": self.status, "intflag": self.intflag.get(), "transactions": self.transactions,
            "addresses": self.addresses, "devices": self.devices.len()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scl_period_is_ten_plus_both_halves() {
        // codal's 100 kHz on 48 MHz: BAUD = BAUDLOW = 230, 470 clocks.
        assert_eq!(scl_clocks(230 | 230 << 8), 470);
        assert_eq!(scl_clocks(230), 470, "BAUDLOW 0 repeats BAUD");
        assert_eq!(scl_clocks(10 | 30 << 8), 50);
    }

    #[test]
    fn rise_time_and_stretching_lengthen_an_operation() {
        // ZI2C's 100 kHz on 48 MHz allowing 215 ns of rise: BAUD = BAUDLOW =
        // 230, so a period is 470 + 48e6 * 215e-9 = 480.32 core clocks.
        let baud = 230 | 230 << 8;
        let (gclk, cpu) = (48_000_000, 120_000_000);
        assert_eq!(op_cycles(1, baud, 0, 0, gclk, cpu), Some(1175));
        assert_eq!(op_cycles(10, baud, 215, 0, gclk, cpu), Some(12_008));
        assert_eq!(
            op_cycles(1, baud, 215, 0, gclk, cpu),
            Some(1201),
            "1200.8, rounded up"
        );
        // 25 us of stretching is 3000 CPU cycles on top.
        assert_eq!(
            op_cycles(9, baud, 0, 25_000, gclk, cpu),
            Some(9 * 1175 + 3000)
        );
        assert_eq!(op_cycles(1, baud, 0, 0, 0, cpu), None, "no core clock");
    }

    struct Holder(u64);
    impl I2cDevice for Holder {
        fn address(&self) -> u8 {
            0x40
        }
        fn read(&mut self) -> u8 {
            0
        }
        fn write(&mut self, _: u8) {}
        fn clock_stretch_ns(&self) -> u64 {
            self.0
        }
    }

    #[test]
    fn scl_held_past_the_low_timeout_ends_the_transfer() {
        for (lowtout, hold, timed_out) in [
            (true, 30_000_000, true),
            (true, 20_000_000, false),
            (false, 30_000_000, false),
        ] {
            let mut m = Samd51SercomI2cm::new(SERCOM_INSTANCES[2]);
            m.attach_slave(Box::new(Holder(hold)));
            let ctrla = MODE_I2C_MASTER | (2 << 20) | if lowtout { LOWTOUTEN } else { 0 };
            m.write_u32(0, ctrla).unwrap();
            m.write_u32(0, ctrla | 2).unwrap();
            m.write_u16(0x1a, IDLE).unwrap();
            m.write_u32(0x24, 0x40 << 1).unwrap();
            let op = m.pending.get().unwrap();
            m.complete(op);
            // MB as normal; LOWTOUT and BUSERR with ERROR, and the bus released.
            assert_eq!(m.read(0x18).unwrap() & MB, MB);
            assert_eq!(m.read(0x18).unwrap() & ERROR != 0, timed_out, "hold {hold}");
            let status = m.read_u16(0x1a).unwrap();
            assert_eq!(status & (LOWTOUT | BUSERR) == LOWTOUT | BUSERR, timed_out);
            assert_eq!(status & BUSSTATE == IDLE, timed_out);
        }
        // The cut-short hold counts the time-out once.
        assert_eq!(
            op_cycles(
                10,
                230 | 230 << 8,
                0,
                LOW_TIMEOUT_NS,
                48_000_000,
                120_000_000
            ),
            Some(11_750 + 3_000_000)
        );
    }

    struct Regs {
        address: u8,
        pointer: u8,
        memory: [u8; 256],
    }
    impl I2cDevice for Regs {
        fn address(&self) -> u8 {
            self.address
        }
        fn write(&mut self, data: u8) {
            self.pointer = data;
        }
        fn read(&mut self) -> u8 {
            let v = self.memory[usize::from(self.pointer)];
            self.pointer = self.pointer.wrapping_add(1);
            v
        }
    }

    fn master() -> Samd51SercomI2cm {
        let mut m = Samd51SercomI2cm::new(SERCOM_INSTANCES[2]);
        let mut memory = [0u8; 256];
        memory[0xd0] = 0x58;
        memory[0xd1] = 0x77;
        m.attach_slave(Box::new(Regs {
            address: 0x77,
            pointer: 0,
            memory,
        }));
        m.write_u32(0, MODE_I2C_MASTER | (2 << 20)).unwrap();
        m.write_u32(4, 1 << 8).unwrap(); // smart mode
        m.write_u32(0, MODE_I2C_MASTER | (2 << 20) | 2).unwrap();
        m.write_u16(0x1a, IDLE).unwrap();
        m
    }
    fn run(m: &mut Samd51SercomI2cm) {
        let op = m.pending.take().expect("pending operation");
        m.perform(op);
    }

    #[test]
    fn register_pointer_write_then_smart_mode_read() {
        let mut m = master();
        m.write_u32(0x24, 0x77 << 1).unwrap();
        run(&mut m);
        assert_eq!(m.read(0x18).unwrap() & MB, MB);
        assert_eq!(m.read_u16(0x1a).unwrap() & RXNACK, 0);
        m.write(0x28, 0xd0).unwrap();
        run(&mut m);
        m.write_u32(0x24, (0x77 << 1) | 1).unwrap(); // repeated start, read
        run(&mut m);
        assert_eq!(m.read(0x18).unwrap() & SB, SB);
        assert_eq!(m.read(0x28).unwrap(), 0x58);
        run(&mut m); // the smart-mode acknowledge fetched the next byte
        m.write_u32(4, (1 << 8) | (1 << 18) | (3 << 16)).unwrap(); // NACK + STOP
        assert_eq!(m.read(0x28).unwrap(), 0x77);
        run(&mut m);
        assert_eq!(m.read_u16(0x1a).unwrap() & BUSSTATE, IDLE);
        assert_eq!(m.transactions(), 2);
    }

    #[test]
    fn an_open_read_stays_on_the_bus_tick_between_bytes() {
        // A smart-mode DATA read issues the next byte from a bus read, which
        // cannot re-register the master: it must still be ticked then.
        let mut m = master();
        m.write_u32(0x24, 0x77 << 1).unwrap();
        run(&mut m);
        m.write(0x28, 0xd0).unwrap(); // register pointer
        run(&mut m);
        m.write_u32(0x24, (0x77 << 1) | 1).unwrap(); // repeated start, read
        run(&mut m);
        assert!(m.needs_bus_tick(), "read open, first byte waiting");
        assert_eq!(m.read(0x28).unwrap(), 0x58);
        assert!(m.busy() && m.needs_bus_tick());
        run(&mut m);
        m.write_u32(4, (1 << 8) | (1 << 18) | (3 << 16)).unwrap(); // NACK + STOP
        run(&mut m);
        assert!(!m.needs_bus_tick(), "closed after STOP");
    }

    #[test]
    fn a_start_written_during_a_stop_follows_it() {
        let mut m = master();
        m.write_u32(0x24, 0x77 << 1).unwrap();
        run(&mut m);
        m.write_u32(4, (1 << 8) | (3 << 16)).unwrap(); // STOP
        m.write_u32(0x24, 0x10 << 1).unwrap(); // next START while it is pending
        assert_eq!(m.pending.get(), Some(Op::Stop));
        assert_eq!(m.queued, Some(Op::Address(0x10 << 1)));
    }

    #[test]
    fn unclaimed_address_is_nacked_and_settings_refused() {
        let mut m = master();
        m.write_u32(0x24, 0x10 << 1).unwrap();
        run(&mut m);
        assert_eq!(m.read(0x18).unwrap() & MB, MB);
        assert_eq!(m.read_u16(0x1a).unwrap() & RXNACK, RXNACK);
        assert!(m.write_u32(0x0c, 10).is_err(), "BAUD while enabled");
        assert!(m.write_u32(0x24, 0x400).is_err(), "10-bit address");
        assert!(m.write_u32(4, 1 << 9).is_err(), "quick command");
        let mut d = Samd51SercomI2cm::new(SERCOM_INSTANCES[2]);
        assert!(
            d.write_u32(0, 1 << 2).is_err(),
            "USART mode on the I2C slice"
        );
        assert!(d.write_u32(0x24, 0x77 << 1).is_err(), "ADDR while disabled");
    }
}
