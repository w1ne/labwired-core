// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! megaAVR 0-series TWI host (master) mode (ATmega4808/4809 data sheet
//! DS40002173 section 25; register map and reset values from the ATmega4809
//! ATDF `TWI` module).
//!
//! Registers: CTRLA 0x00, DUALCTRL 0x01, DBGCTRL 0x02, MCTRLA 0x03, MCTRLB
//! 0x04, MSTATUS 0x05, MBAUD 0x06, MADDR 0x07, MDATA 0x08, SCTRLA 0x09,
//! SCTRLB 0x0A, SSTATUS 0x0B, SADDR 0x0C, SDATA 0x0D, SADDRMASK 0x0E.
//!
//! Modelled, against attached `I2cDevice` slaves with 7-bit addresses:
//! - A write to MADDR while MCTRLA.ENABLE is set and the bus is IDLE or
//!   OWNER issues a START (a repeated START when OWNER) and the address. An
//!   address no slave claims sets WIF and RXACK (NACK) in both directions.
//!   An ACKed write address sets WIF; an ACKed read address receives the
//!   first byte into MDATA and sets RIF. BUSSTATE becomes OWNER.
//! - A write to MDATA in a write transfer sends the byte: WIF, RXACK clear.
//! - MCTRLB.MCMD: RECVTRANS receives the next byte (RIF); STOP sends a STOP
//!   and the bus goes IDLE; REPSTART issues a repeated START and sends MADDR
//!   again. ACKACT is the acknowledge the master gives before the command.
//! - Writing MADDR, MDATA or a command clears RIF, WIF and CLKHOLD. MSTATUS
//!   RIF/WIF/ARBLOST/BUSERR are write-one-to-clear; writing BUSSTATE = IDLE
//!   forces the bus idle, as ArduinoCore-megaavr's TWI_MasterInit does.
//! - The TWIM vector (`irq`) is raised whenever RIF is set with RIEN, or WIF
//!   with WIEN, including when the enable is set over a pending flag.
//!
//! Every operation completes at once: SCL timing (MBAUD, CTRLA SDA setup /
//! hold, FMPEN) is stored but not modelled, so there is no clock stretching,
//! arbitration loss or bus error. The bus timeout (MCTRLA.TIMEOUT) is stored;
//! the only way out of the reset UNKNOWN bus state is writing IDLE, and a
//! MADDR write in UNKNOWN is refused rather than left hanging. Refused
//! rather than ignored: smart mode (SMEN), quick command (QCEN), dual-control
//! mode (DUALCTRL.ENABLE), the client (slave) side (SCTRLA.ENABLE) and a
//! master command or data write that has no transfer to act on.

use crate::peripherals::device::{I2cDevice, I2cSlaveHost};
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;

const EVENT_TOKEN: u32 = 1;

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
const SSTATUS: u64 = 0x0B;
const SADDR: u64 = 0x0C;
const SDATA: u64 = 0x0D;
const SADDRMASK: u64 = 0x0E;

const MCTRLA_RIEN: u8 = 1 << 7;
const MCTRLA_WIEN: u8 = 1 << 6;
const MCTRLA_QCEN: u8 = 1 << 4;
const MCTRLA_SMEN: u8 = 1 << 1;
const MCTRLA_ENABLE: u8 = 1 << 0;
const MCTRLB_FLUSH: u8 = 1 << 3;
const MCTRLB_ACKACT: u8 = 1 << 2;
const MCMD_REPSTART: u8 = 1;
const MCMD_RECVTRANS: u8 = 2;
const MCMD_STOP: u8 = 3;
const RIF: u8 = 1 << 7;
const WIF: u8 = 1 << 6;
const CLKHOLD: u8 = 1 << 5;
const RXACK: u8 = 1 << 4;
const ARBLOST: u8 = 1 << 3;
const BUSERR: u8 = 1 << 2;
const BUSSTATE: u8 = 0x03;
const BUS_UNKNOWN: u8 = 0;
const BUS_IDLE: u8 = 1;
const BUS_OWNER: u8 = 2;

pub struct MegaAvrTwi {
    ctrla: u8,
    dualctrl: u8,
    dbgctrl: u8,
    mctrla: u8,
    /// ACKACT only; MCMD and FLUSH are strobes and read 0.
    ackact: u8,
    mstatus: u8,
    mbaud: u8,
    maddr: u8,
    mdata: u8,
    sctrlb: u8,
    saddr: u8,
    sdata: u8,
    saddrmask: u8,
    /// The addressed slave, while the bus is owned.
    selected: Option<usize>,
    reading: bool,
    devices: Vec<Box<dyn I2cDevice>>,
    /// TWIM vector number.
    irq: u32,
    /// A TWIM assertion waits to be raised.
    irq_event: bool,
    event_live: bool,
    pending_events: Vec<(u64, u32)>,
    clock: Option<CycleClock>,
    /// Evidence: address bytes sent (capped) and completed byte transfers.
    addresses: Vec<u8>,
    bytes: u64,
}

impl std::fmt::Debug for MegaAvrTwi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MegaAvrTwi")
            .field("irq", &self.irq)
            .field("devices", &self.devices.len())
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
            ackact: 0,
            mstatus: BUS_UNKNOWN,
            mbaud: 0,
            maddr: 0,
            mdata: 0,
            sctrlb: 0,
            saddr: 0,
            sdata: 0,
            saddrmask: 0,
            selected: None,
            reading: false,
            devices: Vec::new(),
            irq,
            irq_event: false,
            event_live: false,
            pending_events: Vec::new(),
            clock: None,
            addresses: Vec::new(),
            bytes: 0,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    pub fn push_slave(&mut self, dev: Box<dyn I2cDevice>) {
        self.devices.push(dev);
    }

    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "megaAVR TWI: {what} is not modelled"
        )))
    }

    fn bus_state(&self) -> u8 {
        self.mstatus & BUSSTATE
    }

    fn set_bus(&mut self, state: u8) {
        self.mstatus = (self.mstatus & !BUSSTATE) | state;
    }

    fn irq_due(&self) -> bool {
        (self.mstatus & RIF != 0 && self.mctrla & MCTRLA_RIEN != 0)
            || (self.mstatus & WIF != 0 && self.mctrla & MCTRLA_WIEN != 0)
    }

    /// RIF or WIF was just set: the host holds SCL until software acts.
    fn flag(&mut self, f: u8) {
        self.mstatus |= f | CLKHOLD;
        if self.irq_due() {
            self.irq_event = true;
        }
    }

    fn release(&mut self) {
        self.mstatus &= !(RIF | WIF | CLKHOLD);
    }

    fn end_selected(&mut self) {
        if let Some(i) = self.selected.take() {
            self.devices[i].stop();
        }
        self.reading = false;
    }

    fn receive(&mut self) {
        if let Some(i) = self.selected {
            self.mdata = self.devices[i].read();
            self.bytes += 1;
        }
        self.flag(RIF);
    }

    /// START (or repeated START) and the address in MADDR.
    fn address(&mut self) {
        self.release();
        self.end_selected();
        let byte = self.maddr;
        let address = byte >> 1;
        self.reading = byte & 1 != 0;
        self.set_bus(BUS_OWNER);
        if self.addresses.len() < 64 {
            self.addresses.push(byte);
        }
        match self.devices.iter().position(|d| d.claims_address(address)) {
            Some(i) => {
                let d = &mut self.devices[i];
                d.select_address(address);
                d.start();
                self.selected = Some(i);
                self.mstatus &= !RXACK;
                if self.reading {
                    self.receive();
                } else {
                    self.flag(WIF);
                }
            }
            None => {
                self.reading = false;
                self.mstatus |= RXACK;
                self.flag(WIF);
            }
        }
    }

    fn command(&mut self, mcmd: u8) -> SimResult<()> {
        if self.bus_state() != BUS_OWNER {
            return Self::refuse("a master command (MCTRLB.MCMD) while the bus is not owned");
        }
        self.release();
        match mcmd {
            MCMD_REPSTART => self.address(),
            MCMD_RECVTRANS if self.reading && self.selected.is_some() => self.receive(),
            MCMD_RECVTRANS => {
                return Self::refuse("MCMD RECVTRANS outside an acknowledged read transfer")
            }
            MCMD_STOP => {
                self.end_selected();
                self.set_bus(BUS_IDLE);
            }
            _ => unreachable!("MCMD NOACT carries no command"),
        }
        Ok(())
    }

    fn write_data(&mut self, v: u8) -> SimResult<()> {
        let Some(i) = self.selected.filter(|_| !self.reading) else {
            return Self::refuse("an MDATA write outside an acknowledged write transfer");
        };
        self.release();
        self.mdata = v;
        self.devices[i].write(v);
        self.bytes += 1;
        self.mstatus &= !RXACK;
        self.flag(WIF);
        Ok(())
    }

    fn reg(&self, o: u64) -> Option<u8> {
        Some(match o {
            CTRLA => self.ctrla,
            DUALCTRL => self.dualctrl,
            DBGCTRL => self.dbgctrl,
            MCTRLA => self.mctrla,
            MCTRLB => self.ackact,
            MSTATUS => self.mstatus,
            MBAUD => self.mbaud,
            MADDR => self.maddr,
            MDATA => self.mdata,
            SCTRLA => 0,
            SCTRLB => self.sctrlb,
            SSTATUS => 0,
            SADDR => self.saddr,
            SDATA => self.sdata,
            SADDRMASK => self.saddrmask,
            _ => return None,
        })
    }

    fn write_reg(&mut self, o: u64, v: u8) -> SimResult<()> {
        match o {
            CTRLA => self.ctrla = v & 0x1E,
            DUALCTRL if v & 1 != 0 => return Self::refuse("dual-control mode (DUALCTRL.ENABLE)"),
            DUALCTRL => self.dualctrl = v & 0x1E,
            DBGCTRL => self.dbgctrl = v & 1,
            MCTRLA => {
                if v & MCTRLA_SMEN != 0 {
                    return Self::refuse("smart mode (MCTRLA.SMEN)");
                }
                if v & MCTRLA_QCEN != 0 {
                    return Self::refuse("quick command (MCTRLA.QCEN)");
                }
                let was_on = self.mctrla & MCTRLA_ENABLE != 0;
                self.mctrla = v & 0xDD;
                if v & MCTRLA_ENABLE == 0 {
                    // Disabling the host drops the transfer and the bus state.
                    self.end_selected();
                    self.mstatus = BUS_UNKNOWN;
                } else if !was_on {
                    self.mstatus = BUS_UNKNOWN;
                }
                if self.irq_due() {
                    self.irq_event = true;
                }
            }
            MCTRLB => {
                self.ackact = v & MCTRLB_ACKACT;
                if self.mctrla & MCTRLA_ENABLE == 0 {
                    return Ok(());
                }
                if v & MCTRLB_FLUSH != 0 {
                    self.end_selected();
                    self.release();
                    self.set_bus(BUS_IDLE);
                } else if v & 3 != 0 {
                    self.command(v & 3)?;
                }
            }
            MSTATUS => {
                self.mstatus &= !(v & (RIF | WIF | ARBLOST | BUSERR));
                if self.mstatus & (RIF | WIF) == 0 {
                    self.mstatus &= !CLKHOLD;
                }
                if v & BUSSTATE == BUS_IDLE && self.mctrla & MCTRLA_ENABLE != 0 {
                    self.end_selected();
                    self.set_bus(BUS_IDLE);
                }
            }
            MBAUD => self.mbaud = v,
            MADDR => {
                self.maddr = v;
                if self.mctrla & MCTRLA_ENABLE != 0 {
                    if self.bus_state() == BUS_UNKNOWN {
                        return Self::refuse(
                            "a START with the bus state UNKNOWN (write MSTATUS.BUSSTATE = IDLE first)",
                        );
                    }
                    self.address();
                }
            }
            MDATA if self.mctrla & MCTRLA_ENABLE == 0 => self.mdata = v,
            MDATA => self.write_data(v)?,
            SCTRLA if v & 1 != 0 => return Self::refuse("the TWI client (SCTRLA.ENABLE)"),
            SCTRLA => {}
            // Client ACKACT is stored; SCMD is a client strobe. With the
            // client off, SSTATUS has no flags to clear.
            SCTRLB => self.sctrlb = v & 0x04,
            SSTATUS => {}
            SADDR => self.saddr = v,
            SDATA => self.sdata = v,
            SADDRMASK => self.saddrmask = v,
            _ => return Self::refuse("a store outside the register map"),
        }
        self.want_wake();
        Ok(())
    }

    fn want_wake(&mut self) {
        if self.scheduler_mode() && self.irq_event && !self.event_live {
            self.event_live = true;
            self.pending_events.push((1, EVENT_TOKEN));
        }
    }

    fn take_irqs(&mut self) -> Vec<u32> {
        if std::mem::take(&mut self.irq_event) && self.irq_due() {
            vec![self.irq]
        } else {
            Vec::new()
        }
    }
}

impl I2cSlaveHost for MegaAvrTwi {
    fn attach_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.push_slave(device);
    }
}

impl Peripheral for MegaAvrTwi {
    fn i2c_slave_host(&mut self) -> Option<&mut dyn I2cSlaveHost> {
        Some(self)
    }
    fn read(&self, o: u64) -> SimResult<u8> {
        match self.reg(o) {
            Some(v) => Ok(v),
            None => Self::refuse("a load outside the register map"),
        }
    }
    fn peek(&self, o: u64) -> Option<u8> {
        self.reg(o)
    }
    fn write(&mut self, o: u64, v: u8) -> SimResult<()> {
        self.write_reg(o, v)
    }
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && self.irq_event
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
        let irqs = self.take_irqs();
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        self.want_wake();
        std::mem::take(&mut self.pending_events)
    }
    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        self.event_live = false;
        crate::sched::EventResult {
            explicit_irqs: self.take_irqs(),
            ..Default::default()
        }
    }
    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }
    fn on_node_reset(&mut self) {
        let devices = std::mem::take(&mut self.devices);
        let (clock, addresses, bytes) = (
            self.clock.take(),
            std::mem::take(&mut self.addresses),
            self.bytes,
        );
        *self = Self::new(self.irq);
        self.devices = devices;
        self.clock = clock;
        self.addresses = addresses;
        self.bytes = bytes;
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "megaavr_twi", "mctrla": self.mctrla,
            "mstatus": self.mstatus, "mbaud": self.mbaud, "addresses": self.addresses,
            "bytes": self.bytes, "devices": self.devices.len()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Register file of a 16-bit-register sensor: the first written byte is
    /// the pointer, reads stream the register big-endian.
    struct Regs {
        addr: u8,
        ptr: u8,
        first: bool,
        idx: usize,
        log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }
    impl I2cDevice for Regs {
        fn address(&self) -> u8 {
            self.addr
        }
        fn start(&mut self) {
            self.first = true;
            self.idx = 0;
            self.log.lock().unwrap().push("start".into());
        }
        fn stop(&mut self) {
            self.log.lock().unwrap().push("stop".into());
        }
        fn write(&mut self, b: u8) {
            if std::mem::take(&mut self.first) {
                self.ptr = b;
            }
            self.log.lock().unwrap().push(format!("w{b:02x}"));
        }
        fn read(&mut self) -> u8 {
            let v = [0x39u16, 0x9F, 0x19, 0xCA][(usize::from(self.ptr) & 2) + (self.idx & 1)];
            self.idx += 1;
            v as u8
        }
    }

    fn twi() -> (MegaAvrTwi, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut t = MegaAvrTwi::new(15);
        t.push_slave(Box::new(Regs {
            addr: 0x40,
            ptr: 0,
            first: false,
            idx: 0,
            log: log.clone(),
        }));
        // ArduinoCore-megaavr TWI_MasterInit.
        t.write(MCTRLA, MCTRLA_RIEN | MCTRLA_WIEN | MCTRLA_ENABLE)
            .unwrap();
        t.write(MBAUD, 67).unwrap();
        t.write(MSTATUS, BUS_IDLE).unwrap();
        (t, log)
    }

    fn irq(t: &mut MegaAvrTwi) -> bool {
        t.tick().explicit_irqs == Some(vec![15])
    }

    #[test]
    fn reset_bus_state_is_unknown_and_refuses_a_start() {
        let mut t = MegaAvrTwi::new(15);
        t.write(MCTRLA, MCTRLA_ENABLE).unwrap();
        assert_eq!(t.read(MSTATUS).unwrap() & BUSSTATE, BUS_UNKNOWN);
        assert!(t.write(MADDR, 0x80).is_err());
    }

    #[test]
    fn pointer_write_then_two_byte_read_like_wire() {
        let (mut t, log) = twi();
        // Wire.beginTransmission(0x40); write(0x02); endTransmission().
        t.write(MADDR, 0x80).unwrap();
        assert_eq!(t.read(MSTATUS).unwrap(), WIF | CLKHOLD | BUS_OWNER);
        assert!(irq(&mut t));
        assert!(!irq(&mut t), "one assertion per flag");
        t.write(MDATA, 0x02).unwrap();
        assert_eq!(t.read(MSTATUS).unwrap() & (WIF | RXACK), WIF);
        assert!(irq(&mut t));
        t.write(MCTRLB, MCMD_STOP).unwrap();
        assert_eq!(t.read(MSTATUS).unwrap(), BUS_IDLE);
        assert!(!irq(&mut t));
        // Wire.requestFrom(0x40, 2).
        t.write(MADDR, 0x81).unwrap();
        assert_eq!(t.read(MSTATUS).unwrap(), RIF | CLKHOLD | BUS_OWNER);
        assert!(irq(&mut t));
        assert_eq!(t.read(MDATA).unwrap(), 0x19);
        t.write(MCTRLB, MCMD_RECVTRANS).unwrap();
        assert!(irq(&mut t));
        assert_eq!(t.read(MDATA).unwrap(), 0xCA);
        t.write(MCTRLB, MCTRLB_ACKACT | MCMD_STOP).unwrap();
        assert_eq!(t.read(MSTATUS).unwrap(), BUS_IDLE);
        assert_eq!(
            *log.lock().unwrap(),
            ["start", "w02", "stop", "start", "stop"]
        );
        assert_eq!(t.addresses, [0x80, 0x81]);
    }

    #[test]
    fn unclaimed_address_nacks_in_both_directions() {
        let (mut t, _) = twi();
        for a in [0x90, 0x91] {
            t.write(MADDR, a).unwrap();
            assert_eq!(t.read(MSTATUS).unwrap(), WIF | CLKHOLD | RXACK | BUS_OWNER);
            assert!(irq(&mut t));
            assert!(t.write(MDATA, 0).is_err(), "no transfer to write into");
            t.write(MCTRLB, MCMD_STOP).unwrap();
            assert_eq!(t.read(MSTATUS).unwrap(), RXACK | BUS_IDLE);
        }
    }

    #[test]
    fn repstart_resends_maddr_and_enable_over_flag_raises() {
        let (mut t, log) = twi();
        t.write(MCTRLA, MCTRLA_ENABLE).unwrap();
        t.write(MSTATUS, BUS_IDLE).unwrap();
        t.write(MADDR, 0x80).unwrap();
        assert!(!irq(&mut t), "WIEN clear");
        t.write(MCTRLA, MCTRLA_WIEN | MCTRLA_ENABLE).unwrap();
        assert!(irq(&mut t), "enabling over a pending WIF asserts");
        t.write(MCTRLB, MCMD_REPSTART).unwrap();
        assert_eq!(t.read(MSTATUS).unwrap(), WIF | CLKHOLD | BUS_OWNER);
        assert_eq!(*log.lock().unwrap(), ["start", "stop", "start"]);
        assert_eq!(t.addresses, [0x80, 0x80]);
    }

    #[test]
    fn modes_outside_the_model_are_refused() {
        let (mut t, _) = twi();
        assert!(t.write(MCTRLA, MCTRLA_SMEN | MCTRLA_ENABLE).is_err());
        assert!(t.write(MCTRLA, MCTRLA_QCEN | MCTRLA_ENABLE).is_err());
        assert!(t.write(DUALCTRL, 1).is_err());
        assert!(t.write(SCTRLA, 1).is_err());
        assert!(t.write(MCTRLB, MCMD_STOP).is_err(), "bus not owned");
        // ArduinoCore-megaavr TWI_Disable.
        for (o, v) in [
            (MCTRLA, 0),
            (MBAUD, 0),
            (MSTATUS, BUS_IDLE),
            (SADDR, 0),
            (SCTRLA, 0),
        ] {
            t.write(o, v).unwrap();
        }
        assert_eq!(t.read(MSTATUS).unwrap() & BUSSTATE, BUS_UNKNOWN);
    }
}
