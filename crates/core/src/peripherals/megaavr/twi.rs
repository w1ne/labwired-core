// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! megaAVR 0-series TWI, host (master) side (ATmega4808/4809 data sheet
//! DS40002173 §25; register offsets and bit positions from the Microchip
//! ATmega4809 ATDF).
//!
//! Registers: CTRLA 0x00, DUALCTRL 0x01, DBGCTRL 0x02, MCTRLA 0x03, MCTRLB
//! 0x04, MSTATUS 0x05, MBAUD 0x06, MADDR 0x07, MDATA 0x08, SCTRLA 0x09,
//! SCTRLB 0x0A, SSTATUS 0x0B, SADDR 0x0C, SDATA 0x0D, SADDRMASK 0x0E.
//!
//! Modelled: a single host on the bus. The bus state is UNKNOWN after
//! MCTRLA.ENABLE and becomes IDLE when firmware writes BUSSTATE = IDLE; a
//! MADDR write before that waits for it. A MADDR write sends a START (a
//! repeated START while the host owns the bus) and the address byte. An
//! acknowledged write address sets WIF; an address no device answers sets
//! WIF with RXACK. An acknowledged read address receives the first byte and
//! sets RIF. A MDATA write sends one byte and sets WIF. MCTRLB.MCMD
//! RECVTRANS receives the next byte, REPSTART keeps the bus for the next
//! MADDR write, and STOP releases the bus to IDLE. CLKHOLD reads 1 while the
//! host holds SCL for firmware. RIF and WIF clear on a MADDR write, a MDATA
//! read or write, a bus command, or a 1 written to the flag. MCTRLB.FLUSH
//! and a cleared MCTRLA.ENABLE drop the transfer and return the bus state to
//! UNKNOWN.
//!
//! Timing: one SCL period is `10 + 2 * MBAUD` CPU cycles (the data sheet
//! equation with the rise time taken as 0, CLK_PER taken as the CPU clock).
//! An address takes 10 periods (START + 9 bits), a data byte 9.
//!
//! Interrupt (`irq`: the part's TWIM vector number): raised once on each
//! event that sets RIF with RIEN set or WIF with WIEN set, and when RIEN /
//! WIEN goes 1 with its flag already set. A handler that returns with the
//! flag still set is not re-entered here, though silicon would. Each path of
//! ArduinoCore-megaavr's `twi.c` clears the flag.
//!
//! Refused rather than ignored: smart mode (SMEN), quick command (QCEN), the
//! inactive-bus time-out, client (slave) mode and dual mode. Not modelled:
//! pads (attached devices answer on any PORTMUX.TWISPIROUTEA route),
//! arbitration loss, bus errors (ARBLOST / BUSERR always read 0), a device
//! NACK of a data byte, clock stretching by a device, and SDA hold / setup
//! timing (CTRLA is stored).

use crate::peripherals::device::{drain_i2c_pin_drives, I2cDevice, I2cSlaveHost};
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::cell::Cell;

const CTRLA: u64 = 0x00;
const DUALCTRL: u64 = 0x01;
const DBGCTRL: u64 = 0x02;
const MCTRLA: u64 = 0x03;
const MCTRLB: u64 = 0x04;
const MSTATUS: u64 = 0x05;
const MBAUD: u64 = 0x06;
const MADDR: u64 = 0x07;
const MDATA: u64 = 0x08;
const SCTRLA: u64 = 0x09;
const SCTRLB: u64 = 0x0A;
const SADDR: u64 = 0x0C;
const SADDRMASK: u64 = 0x0E;
const WINDOW: u64 = 0x0F;

const CTRLA_MASK: u8 = 0x1E;
const DUALCTRL_ENABLE: u8 = 1 << 0;
const MCTRLA_RIEN: u8 = 1 << 7;
const MCTRLA_WIEN: u8 = 1 << 6;
const MCTRLA_QCEN: u8 = 1 << 4;
const MCTRLA_TIMEOUT: u8 = 0x0C;
const MCTRLA_SMEN: u8 = 1 << 1;
const MCTRLA_ENABLE: u8 = 1 << 0;
const MCTRLB_FLUSH: u8 = 1 << 3;
const MCTRLB_ACKACT: u8 = 1 << 2;
const MCMD_REPSTART: u8 = 1;
const MCMD_RECVTRANS: u8 = 2;
const MCMD_STOP: u8 = 3;
const MSTATUS_RIF: u8 = 1 << 7;
const MSTATUS_WIF: u8 = 1 << 6;
const MSTATUS_CLKHOLD: u8 = 1 << 5;
const MSTATUS_RXACK: u8 = 1 << 4;
const MSTATUS_ARBLOST: u8 = 1 << 3;
const MSTATUS_BUSERR: u8 = 1 << 2;
const BUSSTATE_UNKNOWN: u8 = 0;
const BUSSTATE_IDLE: u8 = 1;
const BUSSTATE_OWNER: u8 = 2;
const SCTRLA_ENABLE: u8 = 1 << 0;

/// Scheduler token of the interrupt wake; transfer wakes carry `generation`,
/// which is never 0.
const IRQ_TOKEN: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// START or repeated START, then the address byte.
    Address(u8),
    /// One data byte to the addressed device.
    Write(u8),
    /// One data byte from the addressed device.
    Receive,
}

pub struct MegaAvrTwi {
    ctrla: u8,
    dualctrl: u8,
    dbgctrl: u8,
    mctrla: u8,
    ackact: bool,
    /// MSTATUS bits 7..2. A MDATA read clears RIF / WIF.
    flags: Cell<u8>,
    bus_state: u8,
    mbaud: u8,
    maddr: u8,
    mdata: u8,
    sctrla: u8,
    sctrlb: u8,
    saddr: u8,
    saddrmask: u8,
    slaves: Vec<Box<dyn I2cDevice>>,
    selected: Option<usize>,
    /// The open transfer is a read: MCMD RECVTRANS is valid.
    reading: bool,
    /// MADDR was written while the bus state was UNKNOWN.
    address_waits: bool,
    op: Option<Op>,
    /// CPU cycle at which `op` completes.
    due: u64,
    generation: u32,
    /// A transfer wake waits to be handed to the scheduler.
    new_wake: bool,
    /// An interrupt assertion waits to be raised.
    irq_event: bool,
    irq_live: bool,
    /// TWIM vector number.
    irq: u32,
    clock: Option<CycleClock>,
}

impl std::fmt::Debug for MegaAvrTwi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MegaAvrTwi")
            .field("irq", &self.irq)
            .field("mctrla", &self.mctrla)
            .field("mstatus", &self.mstatus())
            .field("devices", &self.slaves.len())
            .finish_non_exhaustive()
    }
}

impl MegaAvrTwi {
    pub fn new(irq: u32) -> Self {
        Self {
            ctrla: 0,
            dualctrl: 0,
            dbgctrl: 0,
            mctrla: 0,
            ackact: false,
            flags: Cell::new(0),
            bus_state: BUSSTATE_UNKNOWN,
            mbaud: 0,
            maddr: 0,
            mdata: 0,
            sctrla: 0,
            sctrlb: 0,
            saddr: 0,
            saddrmask: 0,
            slaves: Vec::new(),
            selected: None,
            reading: false,
            address_waits: false,
            op: None,
            due: 0,
            generation: 0,
            new_wake: false,
            irq_event: false,
            irq_live: false,
            irq,
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn refuse(what: String) -> SimResult<()> {
        Err(SimulationError::NotImplemented(format!(
            "megaAVR TWI: {what} is not modelled"
        )))
    }

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn enabled(&self) -> bool {
        self.mctrla & MCTRLA_ENABLE != 0
    }

    fn mstatus(&self) -> u8 {
        self.flags.get() | self.bus_state
    }

    fn clear(&self, bits: u8) {
        self.flags.set(self.flags.get() & !bits);
    }

    /// CPU cycles per SCL period.
    fn scl_period(&self) -> u64 {
        10 + 2 * u64::from(self.mbaud)
    }

    fn begin(&mut self, op: Op) {
        let periods = match op {
            Op::Address(_) => 10,
            Op::Write(_) | Op::Receive => 9,
        };
        self.op = Some(op);
        self.due = self.now() + periods * self.scl_period();
        self.generation = self.generation.wrapping_add(1).max(1);
        self.new_wake = true;
    }

    /// The transfer ends and the host holds SCL until firmware acts.
    fn hold(&mut self, flag: u8) {
        self.flags.set(self.flags.get() | flag | MSTATUS_CLKHOLD);
        let enable = if flag == MSTATUS_RIF {
            MCTRLA_RIEN
        } else {
            MCTRLA_WIEN
        };
        if self.mctrla & enable != 0 {
            self.irq_event = true;
        }
    }

    fn set_rxack(&mut self, nack: bool) {
        self.clear(MSTATUS_RXACK);
        if nack {
            self.flags.set(self.flags.get() | MSTATUS_RXACK);
        }
    }

    fn complete(&mut self) {
        let Some(op) = self.op.take() else {
            return;
        };
        match op {
            Op::Address(address) => {
                let found = self.slaves.iter().position(|s| s.address() == address >> 1);
                if let Some(old) = self.selected.filter(|&old| Some(old) != found) {
                    self.slaves[old].stop();
                }
                self.selected = found;
                self.reading = false;
                self.set_rxack(found.is_none());
                match found {
                    Some(i) => {
                        self.slaves[i].start();
                        if address & 1 != 0 {
                            self.reading = true;
                            self.begin(Op::Receive);
                        } else {
                            self.hold(MSTATUS_WIF);
                        }
                    }
                    None => self.hold(MSTATUS_WIF),
                }
            }
            Op::Write(value) => {
                if let Some(i) = self.selected {
                    self.slaves[i].write(value);
                }
                self.set_rxack(self.selected.is_none());
                self.hold(MSTATUS_WIF);
            }
            Op::Receive => {
                self.mdata = match self.selected {
                    Some(i) => self.slaves[i].read(),
                    None => 0xFF,
                };
                self.hold(MSTATUS_RIF);
            }
        }
    }

    /// Drop the transfer and release the addressed device.
    fn release_bus(&mut self) {
        if let Some(i) = self.selected.take() {
            self.slaves[i].stop();
        }
        self.op = None;
        self.reading = false;
        self.clear(MSTATUS_RIF | MSTATUS_WIF | MSTATUS_CLKHOLD);
    }

    /// The host logic restarts: the bus state is UNKNOWN again.
    fn restart_host(&mut self) {
        self.release_bus();
        self.flags.set(0);
        self.address_waits = false;
        self.irq_event = false;
        self.bus_state = BUSSTATE_UNKNOWN;
    }

    fn start_address(&mut self) {
        self.bus_state = BUSSTATE_OWNER;
        self.begin(Op::Address(self.maddr));
    }

    fn reg(&self, offset: u64) -> u8 {
        match offset {
            CTRLA => self.ctrla,
            DUALCTRL => self.dualctrl,
            DBGCTRL => self.dbgctrl,
            MCTRLA => self.mctrla,
            // FLUSH and MCMD are strobes and read 0.
            MCTRLB if self.ackact => MCTRLB_ACKACT,
            MSTATUS => self.mstatus(),
            MBAUD => self.mbaud,
            MADDR => self.maddr,
            MDATA => self.mdata,
            SCTRLA => self.sctrla,
            SCTRLB => self.sctrlb,
            SADDR => self.saddr,
            SADDRMASK => self.saddrmask,
            // SSTATUS and SDATA: the client side never runs.
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            CTRLA => self.ctrla = value & CTRLA_MASK,
            DUALCTRL => {
                if value & DUALCTRL_ENABLE != 0 {
                    return Self::refuse(format!("dual mode (DUALCTRL {value:#04x})"));
                }
                self.dualctrl = value & 0x0E;
            }
            DBGCTRL => self.dbgctrl = value & 0x01,
            MCTRLA => {
                if value & MCTRLA_SMEN != 0 {
                    return Self::refuse(format!("smart mode (MCTRLA {value:#04x})"));
                }
                if value & MCTRLA_QCEN != 0 {
                    return Self::refuse(format!("quick command (MCTRLA {value:#04x})"));
                }
                if value & MCTRLA_TIMEOUT != 0 {
                    return Self::refuse(format!("inactive-bus time-out (MCTRLA {value:#04x})"));
                }
                let rising = value & !self.mctrla;
                let was_enabled = self.enabled();
                self.mctrla = value & 0xDF;
                if was_enabled && !self.enabled() {
                    self.restart_host();
                }
                let flags = self.flags.get();
                if (rising & MCTRLA_RIEN != 0 && flags & MSTATUS_RIF != 0)
                    || (rising & MCTRLA_WIEN != 0 && flags & MSTATUS_WIF != 0)
                {
                    self.irq_event = true;
                }
            }
            MCTRLB => {
                self.ackact = value & MCTRLB_ACKACT != 0;
                if value & MCTRLB_FLUSH != 0 {
                    self.restart_host();
                    return Ok(());
                }
                // A command needs the host to own the bus and hold SCL.
                if !self.enabled() || self.bus_state != BUSSTATE_OWNER || self.op.is_some() {
                    return Ok(());
                }
                match value & 0x03 {
                    MCMD_REPSTART => {
                        self.clear(MSTATUS_RIF | MSTATUS_WIF | MSTATUS_CLKHOLD);
                        self.reading = false;
                    }
                    MCMD_RECVTRANS if self.reading => {
                        self.clear(MSTATUS_RIF | MSTATUS_WIF | MSTATUS_CLKHOLD);
                        self.begin(Op::Receive);
                    }
                    MCMD_STOP => {
                        self.release_bus();
                        self.bus_state = BUSSTATE_IDLE;
                    }
                    _ => {}
                }
            }
            MSTATUS => {
                self.clear(value & (MSTATUS_RIF | MSTATUS_WIF | MSTATUS_ARBLOST | MSTATUS_BUSERR));
                if value & 0x03 == BUSSTATE_IDLE && self.enabled() {
                    self.release_bus();
                    self.bus_state = BUSSTATE_IDLE;
                    if std::mem::take(&mut self.address_waits) {
                        self.start_address();
                    }
                }
            }
            MBAUD => self.mbaud = value,
            MADDR => {
                self.maddr = value;
                if !self.enabled() {
                    return Ok(());
                }
                self.clear(MSTATUS_RIF | MSTATUS_WIF | MSTATUS_CLKHOLD);
                if self.bus_state == BUSSTATE_UNKNOWN {
                    self.address_waits = true;
                } else {
                    self.start_address();
                }
            }
            // MDATA takes a byte only while the host holds SCL in a write.
            MDATA
                if self.enabled()
                    && self.flags.get() & MSTATUS_CLKHOLD != 0
                    && !self.reading
                    && self.op.is_none() =>
            {
                self.mdata = value;
                self.clear(MSTATUS_RIF | MSTATUS_WIF | MSTATUS_CLKHOLD);
                self.begin(Op::Write(value));
            }
            SCTRLA => {
                if value & SCTRLA_ENABLE != 0 {
                    return Self::refuse(format!("client mode (SCTRLA {value:#04x})"));
                }
                self.sctrla = value & 0xE6;
            }
            SCTRLB => self.sctrlb = value & MCTRLB_ACKACT,
            SADDR => self.saddr = value,
            SADDRMASK => self.saddrmask = value,
            _ => {}
        }
        Ok(())
    }

    fn take_irq(&mut self) -> Vec<u32> {
        if std::mem::take(&mut self.irq_event) {
            vec![self.irq]
        } else {
            Vec::new()
        }
    }
}

impl I2cSlaveHost for MegaAvrTwi {
    fn attach_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.slaves.push(device);
    }
}

impl Peripheral for MegaAvrTwi {
    fn i2c_slave_host(&mut self) -> Option<&mut dyn I2cSlaveHost> {
        Some(self)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        (offset < WINDOW).then(|| self.reg(offset))
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let value = self.reg(offset);
        if offset == MDATA {
            self.clear(MSTATUS_RIF | MSTATUS_WIF);
        }
        Ok(value)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_reg(offset, value)
    }
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && (self.op.is_some() || self.irq_event)
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }
    fn tick(&mut self) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        // Without a clock a transfer completes on the next tick.
        if self.op.is_some() && self.clock.as_ref().is_none_or(|c| c.now() >= self.due) {
            self.complete();
        }
        let irqs = self.take_irq();
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        let mut events = Vec::new();
        if std::mem::take(&mut self.new_wake) && self.op.is_some() {
            let delay = self.due.saturating_sub(self.now()).max(1);
            events.push((delay, self.generation));
        }
        if self.irq_event && !self.irq_live {
            self.irq_live = true;
            events.push((1, IRQ_TOKEN));
        }
        events
    }
    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token == IRQ_TOKEN {
            self.irq_live = false;
            return crate::sched::EventResult {
                explicit_irqs: self.take_irq(),
                ..Default::default()
            };
        }
        if event_token != self.generation || self.op.is_none() {
            return crate::sched::EventResult::default();
        }
        self.complete();
        // A read address chains the first byte. The drain does not harvest
        // `take_scheduled_events` after an event: the next deadline goes
        // back as a reschedule of this token.
        let again = self.op.is_some().then(|| {
            self.generation = event_token;
            self.new_wake = false;
            self.due.saturating_sub(self.now()).max(1)
        });
        crate::sched::EventResult {
            explicit_irqs: self.take_irq(),
            reschedule_delay: again,
            ..Default::default()
        }
    }
    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }
    fn on_node_reset(&mut self) {
        let slaves = std::mem::take(&mut self.slaves);
        let clock = self.clock.take();
        // A stale wake must not complete a transfer started after the reset.
        let generation = self.generation;
        *self = Self::new(self.irq);
        self.slaves = slaves;
        self.clock = clock;
        self.generation = generation;
    }
    fn drives_central_device_time(&self) -> bool {
        true
    }
    fn has_attached_time_devices(&self) -> bool {
        !self.slaves.is_empty()
    }
    fn advance_attached_device_time_us(&mut self, us: u64) {
        for slave in &mut self.slaves {
            slave.advance_time_us(us);
        }
    }
    fn drain_attached_pin_drives(&mut self, out: &mut Vec<(String, String, bool)>) {
        for slave in &mut self.slaves {
            drain_i2c_pin_drives(&mut **slave, out);
        }
    }
    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for slave in &mut self.slaves {
            if slave.for_each_sim_input(f) {
                return true;
            }
        }
        false
    }
    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for slave in &self.slaves {
            crate::inspect::visit_i2c_device(&**slave, f);
        }
    }
    fn inspect(
        &self,
        base: u64,
        name: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::PeripheralInspect {
        let mut pi = crate::inspect::default_inspect(self, base, name, opts);
        pi.kind = "i2c".into();
        for slave in &self.slaves {
            pi.artifacts
                .extend(crate::inspect::i2c_artifacts(&**slave, opts));
        }
        pi
    }
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        self.slaves.iter().flat_map(|s| s.logs()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const TWIM: u32 = 15;
    const RUN: u8 = MCTRLA_RIEN | MCTRLA_WIEN | MCTRLA_ENABLE;

    /// A register-pointer device: the first byte written after a START
    /// selects the register, later writes store and reads return with
    /// auto-increment.
    #[derive(Debug, Default)]
    struct Regs {
        mem: Vec<u8>,
        ptr: u8,
        fresh: bool,
        log: Vec<&'static str>,
    }

    #[derive(Debug, Clone)]
    struct Dev(Arc<Mutex<Regs>>, u8);

    impl I2cDevice for Dev {
        fn address(&self) -> u8 {
            self.1
        }
        fn start(&mut self) {
            let mut r = self.0.lock().unwrap();
            r.fresh = true;
            r.log.push("start");
        }
        fn stop(&mut self) {
            self.0.lock().unwrap().log.push("stop");
        }
        fn write(&mut self, data: u8) {
            let mut r = self.0.lock().unwrap();
            if std::mem::take(&mut r.fresh) {
                r.ptr = data;
            } else {
                let at = usize::from(r.ptr);
                r.mem[at] = data;
                r.ptr = r.ptr.wrapping_add(1);
            }
        }
        fn read(&mut self) -> u8 {
            let mut r = self.0.lock().unwrap();
            let v = r.mem[usize::from(r.ptr)];
            r.ptr = r.ptr.wrapping_add(1);
            v
        }
    }

    fn twi() -> (MegaAvrTwi, Arc<Mutex<Regs>>) {
        let regs = Arc::new(Mutex::new(Regs {
            mem: (0..=255).map(|i| i ^ 0xA5).collect(),
            ..Regs::default()
        }));
        let mut twi = MegaAvrTwi::new(TWIM);
        twi.attach_slave(Box::new(Dev(regs.clone(), 0x40)));
        (twi, regs)
    }

    /// Tick until the transfer in flight completes; the vectors it raised.
    fn settle(twi: &mut MegaAvrTwi) -> Vec<u32> {
        let mut irqs = Vec::new();
        while twi.legacy_tick_active() {
            irqs.extend(twi.tick().explicit_irqs.unwrap_or_default());
        }
        irqs
    }

    fn start(twi: &mut MegaAvrTwi) {
        twi.write(MCTRLA, RUN).unwrap();
        twi.write(MSTATUS, BUSSTATE_IDLE).unwrap();
    }

    #[test]
    fn bus_state_is_unknown_until_firmware_forces_idle() {
        let (mut twi, _) = twi();
        twi.write(MCTRLA, RUN).unwrap();
        assert_eq!(twi.read(MSTATUS).unwrap(), BUSSTATE_UNKNOWN);
        twi.write(MADDR, 0x80).unwrap();
        assert!(settle(&mut twi).is_empty(), "the START waits for IDLE");
        twi.write(MSTATUS, BUSSTATE_IDLE).unwrap();
        assert_eq!(twi.read(MSTATUS).unwrap() & 0x03, BUSSTATE_OWNER);
        assert_eq!(settle(&mut twi), [TWIM]);
        assert_eq!(
            twi.read(MSTATUS).unwrap(),
            MSTATUS_WIF | MSTATUS_CLKHOLD | BUSSTATE_OWNER
        );
    }

    #[test]
    fn write_then_repeated_start_read_moves_exact_bytes() {
        let (mut twi, regs) = twi();
        start(&mut twi);
        twi.write(MADDR, 0x80).unwrap();
        assert_eq!(settle(&mut twi), [TWIM]);
        twi.write(MDATA, 0x10).unwrap();
        assert_eq!(twi.read(MSTATUS).unwrap() & MSTATUS_WIF, 0);
        assert_eq!(settle(&mut twi), [TWIM]);
        assert_eq!(twi.read(MSTATUS).unwrap() & MSTATUS_RXACK, 0);
        twi.write(MCTRLB, MCMD_REPSTART).unwrap();
        assert_eq!(twi.read(MSTATUS).unwrap(), BUSSTATE_OWNER);

        twi.write(MADDR, 0x81).unwrap();
        assert_eq!(settle(&mut twi), [TWIM]);
        assert_eq!(
            twi.read(MSTATUS).unwrap(),
            MSTATUS_RIF | MSTATUS_CLKHOLD | BUSSTATE_OWNER
        );
        assert_eq!(twi.read(MDATA).unwrap(), 0x10 ^ 0xA5);
        assert_eq!(twi.read(MSTATUS).unwrap() & MSTATUS_RIF, 0, "MDATA read");
        twi.write(MCTRLB, MCMD_RECVTRANS).unwrap();
        assert_eq!(settle(&mut twi), [TWIM]);
        assert_eq!(twi.read(MDATA).unwrap(), 0x11 ^ 0xA5);
        twi.write(MCTRLB, MCTRLB_ACKACT | MCMD_STOP).unwrap();
        assert_eq!(twi.read(MSTATUS).unwrap(), BUSSTATE_IDLE);
        assert_eq!(twi.read(MCTRLB).unwrap(), MCTRLB_ACKACT);
        assert_eq!(regs.lock().unwrap().log, ["start", "start", "stop"]);
    }

    #[test]
    fn data_write_reaches_the_device_register() {
        let (mut twi, regs) = twi();
        start(&mut twi);
        twi.write(MADDR, 0x80).unwrap();
        settle(&mut twi);
        for byte in [0x20, 0x5A] {
            twi.write(MDATA, byte).unwrap();
            settle(&mut twi);
        }
        twi.write(MCTRLB, MCMD_STOP).unwrap();
        assert_eq!(regs.lock().unwrap().mem[0x20], 0x5A);
    }

    #[test]
    fn absent_address_sets_wif_with_rxack() {
        let (mut twi, regs) = twi();
        start(&mut twi);
        for address in [0x50 << 1, 0x50 << 1 | 1] {
            twi.write(MADDR, address).unwrap();
            assert_eq!(settle(&mut twi), [TWIM]);
            assert_eq!(
                twi.read(MSTATUS).unwrap(),
                MSTATUS_WIF | MSTATUS_CLKHOLD | MSTATUS_RXACK | BUSSTATE_OWNER
            );
            twi.write(MCTRLB, MCMD_STOP).unwrap();
            assert_eq!(twi.read(MSTATUS).unwrap() & 0x03, BUSSTATE_IDLE);
        }
        assert!(regs.lock().unwrap().log.is_empty());
    }

    #[test]
    fn flags_raise_no_vector_without_their_enable() {
        let (mut twi, _) = twi();
        twi.write(MCTRLA, MCTRLA_ENABLE).unwrap();
        twi.write(MSTATUS, BUSSTATE_IDLE).unwrap();
        twi.write(MADDR, 0x80).unwrap();
        assert!(settle(&mut twi).is_empty());
        assert_ne!(twi.read(MSTATUS).unwrap() & MSTATUS_WIF, 0);
        twi.write(MCTRLA, MCTRLA_WIEN | MCTRLA_ENABLE).unwrap();
        assert_eq!(settle(&mut twi), [TWIM], "WIEN goes 1 with WIF set");
        twi.write(MSTATUS, MSTATUS_WIF).unwrap();
        assert_eq!(twi.read(MSTATUS).unwrap() & MSTATUS_WIF, 0);
    }

    #[test]
    fn transfer_takes_the_mbaud_bit_time() {
        let (mut twi, _) = twi();
        let clock = CycleClock::default();
        twi.attach_cycle_clock(clock.clone());
        start(&mut twi);
        twi.write(MBAUD, 67).unwrap();
        twi.write(MADDR, 0x80).unwrap();
        // START + 9 bits at 10 + 2 * 67 cycles a bit.
        clock.publish(1439);
        assert!(twi.tick().explicit_irqs.is_none());
        assert_eq!(twi.read(MSTATUS).unwrap() & MSTATUS_WIF, 0);
        clock.publish(1440);
        assert_eq!(twi.tick().explicit_irqs, Some(vec![TWIM]));
    }

    #[test]
    fn disable_and_flush_release_the_device_and_the_bus() {
        let (mut twi, regs) = twi();
        start(&mut twi);
        twi.write(MADDR, 0x80).unwrap();
        settle(&mut twi);
        twi.write(MCTRLB, MCTRLB_FLUSH).unwrap();
        assert_eq!(twi.read(MSTATUS).unwrap(), BUSSTATE_UNKNOWN);
        twi.write(MSTATUS, BUSSTATE_IDLE).unwrap();
        twi.write(MADDR, 0x80).unwrap();
        twi.write(MCTRLA, 0).unwrap();
        assert!(settle(&mut twi).is_empty());
        assert_eq!(twi.read(MSTATUS).unwrap(), BUSSTATE_UNKNOWN);
        assert_eq!(regs.lock().unwrap().log, ["start", "stop"]);
    }

    #[test]
    fn unmodelled_modes_are_refused() {
        let (mut twi, _) = twi();
        for (offset, value) in [
            (MCTRLA, MCTRLA_SMEN | MCTRLA_ENABLE),
            (MCTRLA, MCTRLA_QCEN | MCTRLA_ENABLE),
            (MCTRLA, 0x04 | MCTRLA_ENABLE),
            (SCTRLA, SCTRLA_ENABLE),
            (DUALCTRL, DUALCTRL_ENABLE),
        ] {
            assert!(twi.write(offset, value).is_err(), "{offset:#x} {value:#x}");
        }
        // ArduinoCore-megaavr TWI_Disable(): every store is accepted.
        for (offset, value) in [
            (MCTRLA, 0),
            (MBAUD, 0),
            (MSTATUS, 1),
            (SADDR, 0),
            (SCTRLA, 0),
        ] {
            twi.write(offset, value).unwrap();
        }
    }
}
