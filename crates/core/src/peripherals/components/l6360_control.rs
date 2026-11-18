// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! L6360 control plane, ST DS8900 Rev 9 §§6.8–7.2 and §8.1.
//!
//! https://www.st.com/resource/en/datasheet/l6360.pdf
//! This is a digital, datasheet-derived model, not a silicon-qualified analog
//! model. The caller supplies physical pin levels, threshold conditions and
//! monotonic simulated nanoseconds. UART serialization remains in the shared
//! timed UART engine. Unassigned register readback bits are deterministic zero;
//! they are not asserted to be the silicon's unspecified reset values.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::peripherals::device::I2cDevice;

pub const STATUS_PO: u8 = 0x80;
pub const STATUS_OVT: u8 = 0x20;
pub const STATUS_CQOL: u8 = 0x10;
pub const STATUS_LOL: u8 = 0x08;
pub const STATUS_REGLN: u8 = 0x02;
pub const STATUS_PE: u8 = 0x01;
const STATUS_MASK: u8 = 0xbb;
const RESET_REGISTERS: [u8; 9] = [0, 0x80, 0x21, 0x21, 0, 0, 0, 0, 0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L6360Mode {
    Off,
    LowSide,
    HighSide,
    PushPull,
    TriState,
    LowSideOn,
    HighSideOn,
}
impl L6360Mode {
    pub fn from_configuration(value: u8) -> Self {
        match value >> 5 {
            0 => Self::Off,
            1 => Self::LowSide,
            2 => Self::HighSide,
            3 | 7 => Self::PushPull,
            4 => Self::TriState,
            5 => Self::LowSideOn,
            6 => Self::HighSideOn,
            _ => unreachable!(),
        }
    }
    fn active(self) -> bool {
        !matches!(self, Self::Off | Self::TriState)
    }
    fn requests_driver(self, encq: bool) -> bool {
        match self {
            Self::Off | Self::TriState => false,
            Self::LowSideOn | Self::HighSideOn => true,
            _ => encq,
        }
    }
}

/// Boolean threshold scenarios, not inferred analog voltages or currents.
/// `cq_overload` and `lplus_overload` mean the programmed cut-off threshold is
/// exceeded; the control state applies the actual delay and restart settings.
/// The caller must report changes at their simulated time, including ENL+ edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct L6360Inputs {
    pub reset_n: bool,
    pub vcc_valid: bool,
    pub encq: bool,
    pub enl: bool,
    pub lplus_voltage_valid: bool,
    pub regulator_undervoltage: bool,
    pub overtemperature: bool,
    pub cq_overload: bool,
    pub lplus_overload: bool,
}
impl Default for L6360Inputs {
    fn default() -> Self {
        Self {
            reset_n: true,
            vcc_valid: true,
            encq: false,
            enl: false,
            lplus_voltage_valid: true,
            regulator_undervoltage: false,
            overtemperature: false,
            cq_overload: false,
            lplus_overload: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L6360Error {
    InvalidAddress(u8),
    TimeWentBackwards { previous_ns: u64, requested_ns: u64 },
}
impl fmt::Display for L6360Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddress(address) => {
                write!(
                    f,
                    "L6360 seven-bit address {address:#04x} is outside 0x60..=0x67"
                )
            }
            Self::TimeWentBackwards {
                previous_ns,
                requested_ns,
            } => {
                write!(
                    f,
                    "L6360 time moved backwards: {previous_ns} ns to {requested_ns} ns"
                )
            }
        }
    }
}
impl std::error::Error for L6360Error {}

/// An undefined/unsupported bus exchange fails closed until reset. These are
/// model protocol errors, not invented silicon STATUS bits. The existing I2C
/// slave trait cannot return a byte-level NACK; subsequent address selection is
/// refused and an in-progress invalid read returns the released-bus value 0xff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L6360ProtocolError {
    MissingStop,
    InvalidWriteLength(usize),
    InvalidRegister(u8),
    ReadOnlyRegister(u8),
    MixedDirection,
    ReadPastEnd,
    MissingStart,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct L6360Counters {
    pub current_writes: u64,
    pub sequential_writes: u64,
    pub rejected_writes: u64,
    pub current_reads: u64,
    pub random_reads: u64,
    pub protocol_errors: u64,
    pub resets: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct L6360Snapshot {
    pub address: u8,
    pub registers: [u8; 9],
    pub mode: L6360Mode,
    pub control1: u8,
    pub control2: u8,
    pub status: u8,
    pub parity: u8,
    pub inputs: L6360Inputs,
    pub cq_driver_requested: bool,
    pub cq_driver_enabled: bool,
    pub cq_receiver_enabled: bool,
    pub cq_forced_shutdown: bool,
    pub lplus_driver_enabled: bool,
    pub lplus_forced_shutdown: bool,
    pub irq_asserted: bool,
    pub tx_inverted: bool,
    pub rx_inverted: bool,
    pub cq_cutoff_current_ma: u16,
    pub cq_cutoff_delay_ns: u64,
    pub cq_restart_delay_ns: Option<u64>,
    pub cq_debounce_ns: u64,
    pub iq_debounce_ns: u64,
    pub cq_pulldown_enabled: bool,
    pub cq_pulldown_current_ua: u16,
    pub iq_pulldown_enabled: bool,
    pub lplus_cutoff_enabled: bool,
    pub lplus_cutoff_delay_ns: u64,
    pub lplus_restart_delay_ns: Option<u64>,
    pub cq_cutoff_active: bool,
    pub lplus_cutoff_active: bool,
    pub now_ns: u64,
    pub transaction_generation: u64,
    pub protocol_error: Option<L6360ProtocolError>,
    pub counters: L6360Counters,
}

#[derive(Debug, Clone, Copy, Default)]
struct Cutoff {
    overload_since_ns: Option<u64>,
    restart_at_ns: Option<u64>,
    latched: bool,
}
impl Cutoff {
    fn blocked(self) -> bool {
        self.latched || self.restart_at_ns.is_some()
    }
    fn unlatch(&mut self) {
        if self.latched {
            *self = Self::default();
        }
    }
    /// Return the number of newly crossed cut-off deadlines. Constant-time
    /// arithmetic also preserves a continuously asserted scenario over a long
    /// caller interval; it never loops once per simulated restart.
    fn advance(&mut self, now: u64, overload: bool, delay: u64, restart: Option<u64>) -> u64 {
        if self.latched {
            return 0;
        }
        if let Some(at) = self.restart_at_ns {
            if now < at {
                return 0;
            }
            self.restart_at_ns = None;
            self.overload_since_ns = overload.then_some(at);
        }
        if !overload {
            self.overload_since_ns = None;
            return 0;
        }
        let since = *self.overload_since_ns.get_or_insert(now);
        let trip = since.saturating_add(delay);
        if now < trip {
            return 0;
        }
        self.overload_since_ns = None;
        let Some(restart) = restart else {
            self.latched = true;
            return 1;
        };
        let period = delay.saturating_add(restart);
        // Both documented automatic restart intervals are strictly positive.
        let elapsed = now - trip;
        let trips = elapsed / period + 1;
        let phase = elapsed % period;
        if phase < restart {
            self.restart_at_ns = Some(now.saturating_add(restart - phase));
        } else {
            self.overload_since_ns = Some(now - (phase - restart));
        }
        trips
    }
}

#[derive(Debug)]
struct L6360State {
    address: u8,
    registers: [u8; 9],
    inputs: L6360Inputs,
    now_ns: u64,
    generation: u64,
    fault_epochs: [u64; 8],
    cq: Cutoff,
    lplus: Cutoff,
    protocol_error: Option<L6360ProtocolError>,
    counters: L6360Counters,
}
impl L6360State {
    fn powered(&self) -> bool {
        self.inputs.reset_n && self.inputs.vcc_valid
    }
    fn available(&self) -> bool {
        self.powered() && self.protocol_error.is_none()
    }
    fn mode(&self) -> L6360Mode {
        L6360Mode::from_configuration(self.registers[1])
    }
    fn refresh_status_parity(&mut self) {
        self.registers[8] = (self.registers[8] & 0x7f) | (parity(self.registers[0]) << 7);
    }
    fn latch(&mut self, mask: u8, events: u64) {
        self.registers[0] |= mask & STATUS_MASK;
        for bit in 0..8 {
            if mask & (1 << bit) != 0 {
                self.fault_epochs[bit] = self.fault_epochs[bit].saturating_add(events);
            }
        }
        self.refresh_status_parity();
    }
    fn reset(&mut self) {
        self.registers = RESET_REGISTERS;
        self.cq = Cutoff::default();
        self.lplus = Cutoff::default();
        self.protocol_error = None;
        self.fault_epochs = [0; 8];
        self.generation = self.generation.wrapping_add(1);
        self.counters.resets += 1;
    }
    fn reject_protocol(&mut self, error: L6360ProtocolError) {
        if self.protocol_error.is_none() {
            self.protocol_error = Some(error);
            self.counters.protocol_errors += 1;
        }
    }
    fn cq_delay(&self) -> u64 {
        [100_000, 150_000, 200_000, 250_000][((self.registers[2] >> 3) & 3) as usize]
    }
    fn cq_restart(&self) -> Option<u64> {
        (self.registers[2] & 4 == 0).then(|| 255 * self.cq_delay())
    }
    fn lplus_delay(&self) -> u64 {
        if self.registers[3] & 8 != 0 {
            0
        } else {
            500_000
        }
    }
    fn lplus_restart(&self) -> Option<u64> {
        (self.registers[3] & 4 == 0).then_some(64_000_000)
    }
    fn advance_protection(&mut self, now: u64) {
        let base = self.powered() && self.registers[0] & STATUS_OVT == 0;
        let cq_overload =
            base && self.mode().requests_driver(self.inputs.encq) && self.inputs.cq_overload;
        let lplus_overload =
            base && self.inputs.enl && self.inputs.lplus_overload && self.registers[3] & 0x10 == 0;
        let (cq_delay, cq_restart) = (self.cq_delay(), self.cq_restart());
        let (lplus_delay, lplus_restart) = (self.lplus_delay(), self.lplus_restart());
        let cq_trips = self.cq.advance(now, cq_overload, cq_delay, cq_restart);
        let lplus_trips = self
            .lplus
            .advance(now, lplus_overload, lplus_delay, lplus_restart);
        if cq_trips != 0 {
            self.latch(STATUS_CQOL, cq_trips);
        }
        if lplus_trips != 0 {
            self.latch(STATUS_LOL, lplus_trips);
        }
        self.now_ns = now;
    }
    fn update_inputs(&mut self, now: u64, inputs: L6360Inputs) -> Result<(), L6360Error> {
        if now < self.now_ns {
            return Err(L6360Error::TimeWentBackwards {
                previous_ns: self.now_ns,
                requested_ns: now,
            });
        }
        // The old levels held until the caller's timestamp.
        self.advance_protection(now);
        let previous = self.inputs;
        self.inputs = inputs;
        if previous.reset_n && !inputs.reset_n {
            self.reset();
        } else if previous.vcc_valid != inputs.vcc_valid {
            // Selection epochs cannot survive a loss/restoration of supply.
            // Register retention across a VCC brownout is not a silicon claim;
            // only explicit RST establishes the documented register defaults.
            self.generation = self.generation.wrapping_add(1);
        }
        if !self.powered() {
            return Ok(());
        }
        let became_powered = !previous.reset_n || !previous.vcc_valid;
        if inputs.enl
            && (!previous.enl
                || became_powered
                || (previous.lplus_voltage_valid && !inputs.lplus_voltage_valid))
        {
            self.latch(STATUS_PO, 1);
        }
        if inputs.overtemperature && (!previous.overtemperature || became_powered) {
            self.latch(STATUS_OVT, 1);
        }
        if inputs.regulator_undervoltage && (!previous.regulator_undervoltage || became_powered) {
            self.latch(STATUS_REGLN, 1);
        }
        self.advance_protection(now);
        Ok(())
    }
    fn snapshot(&self) -> L6360Snapshot {
        let mode = self.mode();
        let power = self.powered();
        let thermal = self.registers[0] & STATUS_OVT != 0;
        let cq_forced_shutdown = !power || thermal || self.cq.blocked();
        let lplus_forced_shutdown = !power || thermal || self.lplus.blocked();
        let cq_driver_requested = mode.requests_driver(self.inputs.encq);
        L6360Snapshot {
            address: self.address,
            registers: self.registers,
            mode,
            control1: self.registers[2],
            control2: self.registers[3],
            status: self.registers[0],
            parity: self.registers[8],
            inputs: self.inputs,
            cq_driver_requested,
            cq_driver_enabled: cq_driver_requested && !cq_forced_shutdown,
            cq_receiver_enabled: power && mode != L6360Mode::Off && !self.inputs.encq,
            cq_forced_shutdown,
            lplus_driver_enabled: self.inputs.enl && !lplus_forced_shutdown,
            lplus_forced_shutdown,
            irq_asserted: power && self.registers[0] & STATUS_MASK != 0,
            tx_inverted: true,
            rx_inverted: true,
            cq_cutoff_current_ma: [115, 220, 350, 580][((self.registers[2] >> 5) & 3) as usize],
            cq_cutoff_delay_ns: self.cq_delay(),
            cq_restart_delay_ns: self.cq_restart(),
            cq_debounce_ns: [0, 5_000, 20_000, 100_000][(self.registers[2] & 3) as usize],
            iq_debounce_ns: [0, 5_000, 20_000, 100_000][(self.registers[3] & 3) as usize],
            cq_pulldown_enabled: power && self.registers[2] & 0x80 != 0 && !self.inputs.encq,
            cq_pulldown_current_ua: if self.registers[3] & 0x20 != 0 {
                5_500
            } else {
                2_500
            },
            iq_pulldown_enabled: power && self.registers[3] & 0x80 != 0,
            lplus_cutoff_enabled: self.registers[3] & 0x10 == 0,
            lplus_cutoff_delay_ns: self.lplus_delay(),
            lplus_restart_delay_ns: self.lplus_restart(),
            cq_cutoff_active: self.cq.blocked(),
            lplus_cutoff_active: self.lplus.blocked(),
            now_ns: self.now_ns,
            transaction_generation: self.generation,
            protocol_error: self.protocol_error,
            counters: self.counters,
        }
    }
    fn record_received_parity(&mut self, address: usize, value: u8) {
        let mask = 1 << (7 - address);
        self.registers[8] = (self.registers[8] & !mask) | (parity(value) << (7 - address));
    }
    fn allowed_configuration(&self, next: u8) -> bool {
        let current = self.mode();
        let next = L6360Mode::from_configuration(next);
        current == next || !current.active() || !next.active()
    }
    fn unlatch_cutoffs(&mut self) {
        // DS8900 tables 15/21: any internal-register read/write releases the
        // latched protection. Historical CQOL/LOL still require current read.
        self.cq.unlatch();
        self.lplus.unlatch();
    }
    fn apply_write(&mut self, bytes: &[u8]) {
        match bytes {
            [value, check] => {
                let address = (check & 0x0f) as usize;
                if address == 0 || address == 8 {
                    self.reject_protocol(L6360ProtocolError::ReadOnlyRegister(address as u8));
                    return;
                }
                if address > 8 {
                    self.reject_protocol(L6360ProtocolError::InvalidRegister(address as u8));
                    return;
                }
                self.record_received_parity(address, *value);
                if check & 0xe0 != write_check_byte(address as u8, *value) & 0xe0
                    || (address == 1 && !self.allowed_configuration(*value))
                {
                    self.counters.rejected_writes += 1;
                    self.latch(STATUS_PE, 1);
                    return;
                }
                self.unlatch_cutoffs();
                self.registers[address] = *value & register_mask(address);
                self.counters.current_writes += 1;
            }
            [configuration, control1, control2, led1h, led1l, led2h, led2l, check] => {
                let data = [
                    *configuration,
                    *control1,
                    *control2,
                    *led1h,
                    *led1l,
                    *led2h,
                    *led2l,
                ];
                let mut expected = 0;
                for (index, value) in data.iter().enumerate() {
                    self.record_received_parity(index + 1, *value);
                    expected |= parity(*value) << (6 - index);
                }
                if check & 0x7f != expected || !self.allowed_configuration(*configuration) {
                    self.counters.rejected_writes += 1;
                    self.latch(STATUS_PE, 1);
                    return;
                }
                self.unlatch_cutoffs();
                for (index, value) in data.iter().enumerate() {
                    self.registers[index + 1] = *value & register_mask(index + 1);
                }
                self.counters.sequential_writes += 1;
            }
            _ => self.reject_protocol(L6360ProtocolError::InvalidWriteLength(bytes.len())),
        }
    }
    fn finish_current_read(&mut self, returned_status: u8, epochs: [u64; 8]) {
        let mut persistent = 0;
        if self.inputs.enl && !self.inputs.lplus_voltage_valid {
            persistent |= STATUS_PO;
        }
        if self.inputs.overtemperature {
            persistent |= STATUS_OVT;
        }
        if self.inputs.regulator_undervoltage {
            persistent |= STATUS_REGLN;
        }
        if self.cq.blocked() && self.registers[2] & 4 == 0 {
            persistent |= STATUS_CQOL;
        }
        if self.lplus.blocked() && self.registers[3] & 4 == 0 {
            persistent |= STATUS_LOL;
        }
        for (bit, epoch) in epochs.iter().enumerate() {
            let mask = 1 << bit;
            if returned_status & mask != 0
                && persistent & mask == 0
                && *epoch == self.fault_epochs[bit]
            {
                self.registers[0] &= !mask;
            }
        }
        self.refresh_status_parity();
        self.counters.current_reads += 1;
    }
}

fn parity(value: u8) -> u8 {
    (value.count_ones() & 1) as u8
}
fn register_mask(address: usize) -> u8 {
    match address {
        1 => 0xe0,
        3 => 0xbf,
        _ => 0xff,
    }
}
/// DS8900 equation 2, DATA then P2/P1/P0/unused/register-address.
pub fn write_check_byte(address: u8, data: u8) -> u8 {
    (parity(data) << 5) | (parity(data & 0xaa) << 6) | (parity(data & 0x55) << 7) | (address & 0x0f)
}

#[derive(Debug, Clone)]
pub struct L6360Control {
    state: Arc<Mutex<L6360State>>,
}
impl L6360Control {
    pub fn new(address: u8) -> Result<Self, L6360Error> {
        if !(0x60..=0x67).contains(&address) {
            return Err(L6360Error::InvalidAddress(address));
        }
        Ok(Self {
            state: Arc::new(Mutex::new(L6360State {
                address,
                registers: RESET_REGISTERS,
                inputs: L6360Inputs::default(),
                now_ns: 0,
                generation: 0,
                fault_epochs: [0; 8],
                cq: Cutoff::default(),
                lplus: Cutoff::default(),
                protocol_error: None,
                counters: L6360Counters::default(),
            })),
        })
    }
    fn lock(&self) -> MutexGuard<'_, L6360State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
    pub fn snapshot(&self) -> L6360Snapshot {
        self.lock().snapshot()
    }
    pub fn update_inputs(&self, now_ns: u64, inputs: L6360Inputs) -> Result<(), L6360Error> {
        self.lock().update_inputs(now_ns, inputs)
    }
    /// Construct the single bus endpoint for this IC; clones of the control
    /// handle let GPIO/UART wiring inspect the same state without another bus.
    pub fn i2c_device(&self) -> L6360I2c {
        L6360I2c {
            control: self.clone(),
            transaction: Transaction::Idle,
            pending_random: None,
            generation: self.lock().generation,
            selected: true,
        }
    }
}

#[derive(Debug, Clone)]
enum Transaction {
    Idle,
    Open {
        epochs: [u64; 8],
    },
    Write {
        bytes: [u8; 8],
        len: usize,
    },
    Read {
        bytes: [u8; 9],
        len: usize,
        position: usize,
        current: bool,
        epochs: [u64; 8],
    },
    Invalid,
}

#[derive(Debug)]
pub struct L6360I2c {
    control: L6360Control,
    transaction: Transaction,
    pending_random: Option<u8>,
    generation: u64,
    selected: bool,
}
impl L6360I2c {
    fn synchronize_generation(&mut self) -> bool {
        let generation = self.control.lock().generation;
        if generation != self.generation {
            self.transaction = Transaction::Idle;
            self.pending_random = None;
            self.generation = generation;
            return false;
        }
        true
    }
    fn reject(&mut self, error: L6360ProtocolError) {
        self.control.lock().reject_protocol(error);
        self.transaction = Transaction::Invalid;
        self.pending_random = None;
    }
    fn usable(&self) -> bool {
        self.selected && self.control.lock().available()
    }
}
impl I2cDevice for L6360I2c {
    fn address(&self) -> u8 {
        self.control.lock().address
    }
    fn claims_address(&self, address: u8) -> bool {
        let state = self.control.lock();
        address == state.address && state.available()
    }
    fn select_address(&mut self, address: u8) {
        self.selected = self.claims_address(address);
    }
    fn transaction_generation(&self) -> Option<u64> {
        Some(self.control.lock().generation)
    }
    fn start(&mut self) {
        self.synchronize_generation();
        if !self.usable() {
            self.transaction = Transaction::Invalid;
            return;
        }
        if !matches!(self.transaction, Transaction::Idle) {
            self.reject(L6360ProtocolError::MissingStop);
            return;
        }
        self.transaction = Transaction::Open {
            epochs: self.control.lock().fault_epochs,
        };
    }
    fn write(&mut self, data: u8) {
        if !self.synchronize_generation() || !self.usable() {
            return;
        }
        match &mut self.transaction {
            Transaction::Open { .. } => {
                self.pending_random = None;
                let mut bytes = [0; 8];
                bytes[0] = data;
                self.transaction = Transaction::Write { bytes, len: 1 };
            }
            Transaction::Write { bytes, len } if *len < 8 => {
                bytes[*len] = data;
                *len += 1;
            }
            Transaction::Write { len, .. } => {
                let invalid_length = *len + 1;
                self.reject(L6360ProtocolError::InvalidWriteLength(invalid_length));
            }
            Transaction::Read { .. } => self.reject(L6360ProtocolError::MixedDirection),
            Transaction::Idle => self.reject(L6360ProtocolError::MissingStart),
            Transaction::Invalid => {}
        }
    }
    fn read(&mut self) -> u8 {
        if !self.synchronize_generation() || !self.usable() {
            return 0xff;
        }
        if let Transaction::Open { epochs } = self.transaction {
            let mut state = self.control.lock();
            let mut bytes = [0; 9];
            let current = self.pending_random.is_none();
            let len = if let Some(start) = self.pending_random.take() {
                let count = 9 - start as usize;
                bytes[..count].copy_from_slice(&state.registers[start as usize..]);
                state.counters.random_reads += 1;
                count
            } else {
                bytes[0] = state.registers[0];
                bytes[1] = state.registers[8];
                2
            };
            state.unlatch_cutoffs();
            self.transaction = Transaction::Read {
                bytes,
                len,
                position: 0,
                current,
                epochs,
            };
        }
        match &mut self.transaction {
            Transaction::Read {
                bytes,
                len,
                position,
                ..
            } if *position < *len => {
                let value = bytes[*position];
                *position += 1;
                value
            }
            Transaction::Read { .. } => {
                self.reject(L6360ProtocolError::ReadPastEnd);
                0xff
            }
            Transaction::Write { .. } => {
                self.reject(L6360ProtocolError::MixedDirection);
                0xff
            }
            Transaction::Idle => {
                self.reject(L6360ProtocolError::MissingStart);
                0xff
            }
            _ => 0xff,
        }
    }
    fn stop(&mut self) {
        if !self.synchronize_generation() {
            return;
        }
        let transaction = std::mem::replace(&mut self.transaction, Transaction::Idle);
        if !self.usable() {
            self.pending_random = None;
            return;
        }
        match transaction {
            Transaction::Write { bytes, len: 1 } => {
                if bytes[0] <= 8 {
                    self.pending_random = Some(bytes[0]);
                } else {
                    self.reject(L6360ProtocolError::InvalidRegister(bytes[0]));
                }
            }
            Transaction::Write { bytes, len } => self.control.lock().apply_write(&bytes[..len]),
            Transaction::Read {
                bytes,
                position: 2,
                current: true,
                epochs,
                ..
            } => {
                self.control.lock().finish_current_read(bytes[0], epochs);
            }
            _ => {}
        }
    }
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        let s = self.control.snapshot();
        vec![crate::peripheral_log::PeripheralLog::new("l6360-control", vec![format!(
            "address={:#04x} mode={:?} status={:#04x} parity={:#04x} cq={} lplus={} irq={} counters={:?} protocol_error={:?}; DS8900 Rev9 digital threshold model; no silicon capture",
            s.address, s.mode, s.status, s.parity, s.cq_driver_enabled, s.lplus_driver_enabled,
            s.irq_asserted, s.counters, s.protocol_error
        )])]
    }
}

#[cfg(test)]
mod tests;
