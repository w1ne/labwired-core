// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CC2538 I2C master and slave (`cc2538_i2cm`): I2CM at 0x4002_0000 and
//! I2CS at 0x4002_0800 (SWRU319C Table 4-1), sharing interrupt 8 in the
//! regular map (Table 5-2): "only a single interrupt signal is sent to the
//! interrupt controller" (§20.3.3). One model owns both windows so that the
//! LPBK loopback (§20.3.4) can tie master and slave together. The chip maps
//! it as one 0x900-byte window; the reserved 0x100..0x7FF gap between the two
//! register files reads 0 and drops writes. Renode's `cc2538.repl` has no
//! I2C model, so this goes beyond Renode parity.
//!
//! Master register map (SWRU319C §20.5.1, the Stellaris-family I2C master):
//!
//! | Offset | Name | Access | Meaning |
//! | --- | --- | --- | --- |
//! | 0x000 | I2CM_SA | R/W | SA \[7:1\] slave address, RS 0 (1 = receive) |
//! | 0x004 | I2CM_CTRL | W | ACK 3, STOP 2, START 1, RUN 0 |
//! | 0x004 | I2CM_STAT | R | BUSBSY 6, IDLE 5, ARBLST 4, DATACK 3, ADRACK 2, ERROR 1, BUSY 0; 0x20 at reset |
//! | 0x008 | I2CM_DR | R/W | byte to send / last byte received |
//! | 0x00C | I2CM_TPR | R/W | TPR \[6:0\] (stored only); 0x01 at reset |
//! | 0x010 | I2CM_IMR | R/W | IM 0 |
//! | 0x014 | I2CM_RIS | R | RIS 0, set when a command completes |
//! | 0x018 | I2CM_MIS | R | RIS & IMR |
//! | 0x01C | I2CM_ICR | W1C | IC 0 |
//! | 0x020 | I2CM_CR | R/W | SFE 5 (slave enable), MFE 4 (master enable), LPBK 0 (loopback) |
//!
//! Slave register map (SWRU319C §20.5.2, Table 20-5):
//!
//! | Offset | Name | Access | Meaning |
//! | --- | --- | --- | --- |
//! | 0x800 | I2CS_OAR | R/W | OAR \[6:0\] own address |
//! | 0x804 | I2CS_CTRL | W | DA 0 (device active) |
//! | 0x804 | I2CS_STAT | R | FBR 2, TREQ 1, RREQ 0 |
//! | 0x808 | I2CS_DR | R/W | byte received / byte to transmit |
//! | 0x80C | I2CS_IMR | R/W | STOPIM 2, STARTIM 1, DATAIM 0 |
//! | 0x810 | I2CS_RIS | R | STOPRIS 2, STARTRIS 1, DATARIS 0 |
//! | 0x814 | I2CS_MIS | R | RIS & IMR |
//! | 0x818 | I2CS_ICR | W1C | STOPIC 2, STARTIC 1, DATAIC 0 |
//!
//! The register table types STOPIM/STARTIM as RO, but §20.3.3.2 says they
//! are enabled "by setting the STARTIM and STOPIM bits", so they are RW here.
//!
//! A CTRL write runs one command of the SWRU319C §21.3.1 master state
//! machine (TI driverlib `I2C_MASTER_CMD_*`) when MFE is set: from idle
//! START+RUN addresses SA and transfers one byte, STOP added ends the transfer
//! with a STOP; while the bus is held RUN moves the next byte (receive: ACK
//! set acknowledges it), START+RUN sends a repeated START, and STOP alone
//! releases the bus. The addressed device must be attached by
//! `external_devices:`; any other address is not acknowledged (ERROR +
//! ADRACK), and a data byte with no selected device is not acknowledged
//! (ERROR + DATACK). A NACK does not release the bus by itself: the bus stays
//! held until a command with STOP, which TI's driverlib sends as
//! `I2C_MASTER_CMD_BURST_SEND_ERROR_STOP` (and the single-byte commands
//! already carry). Attached devices always acknowledge data bytes. Every
//! command, including a bare STOP, sets RIS on completion.
//!
//! **Loopback.** With LPBK set the master's bus is the internal one tied to
//! this chip's own slave, so `external_devices:` are not reachable. The slave
//! takes part when it is active (CR.SFE and CTRL.DA): every START (including a
//! repeated one) sets STARTRIS and a STOP sets STOPRIS. An address equal to
//! OAR is acknowledged; any other is not (ERROR + ADRACK). A master write
//! latches its byte into I2CS_DR and sets RREQ (and FBR for the first byte
//! after the address) plus DATARIS; reading I2CS_DR clears RREQ and FBR. A
//! master read sets TREQ and DATARIS and stalls the master (STAT.BUSY) until
//! firmware writes I2CS_DR, which hands the byte to I2CM_DR, clears TREQ,
//! completes the command (RIS, and STOP if it carried one) — the clock
//! stretching of Figure 20-13. CTRL writes while the master is stalled are
//! ignored.
//!
//! Not modelled: the slave on the external bus (without LPBK it is never
//! addressed; IOC routing of I2CMSSDA/SCL is not followed), clock stretching
//! on a master write (the write completes at the CTRL write; a second byte
//! before I2CS_DR is read overwrites it), multi-master arbitration (ARBLST is
//! never set), the SCL rate from TPR and transfer time (BUSY is only seen
//! while a loopback read waits for the slave), glitch filtering, and the
//! SYS_CTRL RCGCI2C clock gate.

use crate::peripherals::device::{drain_i2c_pin_drives, I2cDevice};
use crate::{Peripheral, SimResult};

const SA: u64 = 0x000;
const CTRL_STAT: u64 = 0x004;
const DR: u64 = 0x008;
const TPR: u64 = 0x00C;
const IMR: u64 = 0x010;
const RIS: u64 = 0x014;
const MIS: u64 = 0x018;
const ICR: u64 = 0x01C;
const CR: u64 = 0x020;

const S_OAR: u64 = 0x800;
const S_CTRL_STAT: u64 = 0x804;
const S_DR: u64 = 0x808;
const S_IMR: u64 = 0x80C;
const S_RIS: u64 = 0x810;
const S_MIS: u64 = 0x814;
const S_ICR: u64 = 0x818;

const S_STAT_RREQ: u32 = 1 << 0;
const S_STAT_TREQ: u32 = 1 << 1;
const S_STAT_FBR: u32 = 1 << 2;

const S_INT_DATA: u32 = 1 << 0;
const S_INT_START: u32 = 1 << 1;
const S_INT_STOP: u32 = 1 << 2;
const S_INT_MASK: u32 = 0x7;

const CTRL_RUN: u32 = 1 << 0;
const CTRL_START: u32 = 1 << 1;
const CTRL_STOP: u32 = 1 << 2;

const STAT_BUSY: u32 = 1 << 0;
const STAT_ERROR: u32 = 1 << 1;
const STAT_ADRACK: u32 = 1 << 2;
const STAT_DATACK: u32 = 1 << 3;
const STAT_IDLE: u32 = 1 << 5;
const STAT_BUSBSY: u32 = 1 << 6;

const CR_LPBK: u32 = 1 << 0;
const CR_MFE: u32 = 1 << 4;
const CR_SFE: u32 = 1 << 5;
const CR_MASK: u32 = 0x31;
const TPR_MASK: u32 = 0x7F;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Cc2538I2cm {
    sa: u32,
    dr: u32,
    tpr: u32,
    imr: u32,
    ris: u32,
    cr: u32,
    /// ERROR | ADRACK | DATACK of the last command.
    error: u32,
    /// The master holds the bus (START sent, no STOP yet).
    holding: bool,
    /// Index into `slaves` of the device addressed by the current transfer.
    selected: Option<usize>,
    /// I2CS_OAR.
    s_oar: u32,
    /// I2CS_CTRL.DA.
    s_da: bool,
    s_dr: u32,
    s_imr: u32,
    s_ris: u32,
    /// I2CS_STAT. A cell because reading I2CS_DR clears RREQ and FBR.
    s_stat: std::cell::Cell<u32>,
    /// The own slave is addressed by the current loopback transfer.
    s_addressed: bool,
    /// The next byte the own slave receives is the first after its address.
    s_first: bool,
    /// A loopback master read waits for the slave to write I2CS_DR; the
    /// value is whether the stalled command carried STOP.
    stalled: Option<bool>,
    #[serde(skip)]
    slaves: Vec<Box<dyn I2cDevice>>,
}

impl std::fmt::Debug for Cc2538I2cm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cc2538I2cm")
            .field("sa", &self.sa)
            .field("cr", &self.cr)
            .field("holding", &self.holding)
            .field("selected", &self.selected)
            .field("slaves", &self.slaves.len())
            .finish_non_exhaustive()
    }
}

impl Default for Cc2538I2cm {
    fn default() -> Self {
        Self::new()
    }
}

impl Cc2538I2cm {
    pub fn new() -> Self {
        Self {
            sa: 0,
            dr: 0,
            tpr: 0x01,
            imr: 0,
            ris: 0,
            cr: 0,
            error: 0,
            holding: false,
            selected: None,
            s_oar: 0,
            s_da: false,
            s_dr: 0,
            s_imr: 0,
            s_ris: 0,
            s_stat: std::cell::Cell::new(0),
            s_addressed: false,
            s_first: false,
            stalled: None,
            slaves: Vec::new(),
        }
    }

    fn reset_registers(&mut self) {
        self.deselect();
        let slaves = std::mem::take(&mut self.slaves);
        *self = Self::new();
        self.slaves = slaves;
    }

    fn irq_active(&self) -> bool {
        self.ris & self.imr & 1 != 0 || self.s_ris & self.s_imr != 0
    }

    fn loopback(&self) -> bool {
        self.cr & CR_LPBK != 0
    }

    fn slave_active(&self) -> bool {
        self.cr & CR_SFE != 0 && self.s_da
    }

    /// A START or STOP on the loopback bus, as the own slave sees it.
    fn slave_condition(&mut self, int: u32) {
        if self.loopback() && self.slave_active() {
            self.s_ris |= int;
        }
    }

    fn status(&self) -> u32 {
        let mut stat = self.error;
        if self.stalled.is_some() {
            stat |= STAT_BUSY;
        }
        if self.holding {
            stat |= STAT_BUSBSY;
        } else {
            stat |= STAT_IDLE;
        }
        stat
    }

    fn deselect(&mut self) {
        if let Some(i) = self.selected.take() {
            self.slaves[i].stop();
        }
    }

    fn release(&mut self) {
        self.deselect();
        self.holding = false;
        self.s_addressed = false;
        self.stalled = None;
    }

    /// The loopback address phase: only the own slave is on the bus.
    fn address_own_slave(&mut self, address: u8) -> bool {
        self.deselect();
        self.slave_condition(S_INT_START);
        self.s_addressed = self.loopback() && self.slave_active() && address as u32 == self.s_oar;
        if self.s_addressed {
            self.s_first = true;
        } else {
            self.error = STAT_ERROR | STAT_ADRACK;
        }
        self.s_addressed
    }

    /// One loopback data byte. Returns whether the master now waits for the
    /// slave (a read stretches the clock until I2CS_DR is written).
    fn data_own_slave(&mut self) -> bool {
        if !self.s_addressed {
            self.error = STAT_ERROR | STAT_DATACK;
            return false;
        }
        let mut stat = self.s_stat.get();
        self.s_ris |= S_INT_DATA;
        if self.sa & 1 != 0 {
            self.s_stat.set(stat | S_STAT_TREQ);
            return true;
        }
        self.s_dr = self.dr;
        stat |= S_STAT_RREQ;
        if self.s_first {
            stat |= S_STAT_FBR;
        }
        self.s_first = false;
        self.s_stat.set(stat);
        false
    }

    /// Bus-level end of a command: STOP if it carried one, then RIS.
    fn complete(&mut self, stop: bool) {
        if stop {
            self.slave_condition(S_INT_STOP);
            self.release();
        }
        self.ris = 1;
    }

    /// I2CS_DR write: the byte a stalled loopback read was waiting for.
    fn slave_transmit(&mut self, value: u32) {
        self.s_dr = value & 0xFF;
        if let Some(stop) = self.stalled.take() {
            self.dr = self.s_dr;
            self.s_stat.set(self.s_stat.get() & !S_STAT_TREQ);
            self.complete(stop);
        }
    }

    /// START (or repeated START) and the address phase. Returns whether the
    /// address was acknowledged.
    /// A repeated START is not a STOP: the device it re-addresses sees only
    /// a new `start()`; one it leaves sees `stop()`.
    fn address(&mut self) -> bool {
        self.holding = true;
        let address = ((self.sa >> 1) & 0x7F) as u8;
        if self.loopback() {
            return self.address_own_slave(address);
        }
        let found = self.slaves.iter().position(|d| d.claims_address(address));
        if self.selected != found {
            self.deselect();
        }
        match found {
            Some(i) => {
                self.slaves[i].select_address(address);
                self.slaves[i].start();
                self.selected = Some(i);
                true
            }
            None => {
                self.error = STAT_ERROR | STAT_ADRACK;
                false
            }
        }
    }

    /// One data byte in the SA.RS direction. Returns whether the master now
    /// waits for the loopback slave.
    fn data(&mut self) -> bool {
        if self.loopback() {
            return self.data_own_slave();
        }
        let Some(i) = self.selected else {
            self.error = STAT_ERROR | STAT_DATACK;
            return false;
        };
        if self.sa & 1 != 0 {
            self.dr = self.slaves[i].read() as u32;
        } else {
            self.slaves[i].write(self.dr as u8);
        }
        false
    }

    /// ACK is not read: it only shapes the acknowledge the master returns on
    /// a received byte, which attached devices do not observe.
    fn command(&mut self, ctrl: u32) {
        let run = ctrl & CTRL_RUN != 0;
        let start = ctrl & CTRL_START != 0;
        let stop = ctrl & CTRL_STOP != 0;
        // Without MFE, without RUN or STOP, or from idle without START, the
        // write is not a command of the master state machine.
        // A stalled loopback read is still in progress: the master is busy.
        if self.cr & CR_MFE == 0
            || self.stalled.is_some()
            || !(run || stop)
            || (!self.holding && !start)
        {
            return;
        }
        self.error = 0;
        if run && (!start || self.address()) && self.data() {
            self.stalled = Some(stop);
            return;
        }
        self.complete(stop);
    }

    fn read_reg(&self, reg: u64) -> u32 {
        match reg {
            SA => self.sa,
            CTRL_STAT => self.status(),
            DR => self.dr,
            TPR => self.tpr,
            IMR => self.imr,
            RIS => self.ris,
            MIS => self.ris & self.imr,
            CR => self.cr,
            S_OAR => self.s_oar,
            S_CTRL_STAT => self.s_stat.get(),
            S_DR => {
                self.s_stat
                    .set(self.s_stat.get() & !(S_STAT_RREQ | S_STAT_FBR));
                self.s_dr
            }
            S_IMR => self.s_imr,
            S_RIS => self.s_ris,
            S_MIS => self.s_ris & self.s_imr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, reg: u64, value: u32) {
        match reg {
            SA => self.sa = value & 0xFF,
            CTRL_STAT => self.command(value & 0xF),
            DR => self.dr = value & 0xFF,
            TPR => self.tpr = value & TPR_MASK,
            IMR => self.imr = value & 1,
            ICR => self.ris &= !(value & 1),
            CR => {
                self.cr = value & CR_MASK;
                if self.cr & CR_MFE == 0 {
                    self.release();
                    self.error = 0;
                }
            }
            S_OAR => self.s_oar = value & 0x7F,
            S_CTRL_STAT => self.s_da = value & 1 != 0,
            S_DR => self.slave_transmit(value),
            S_IMR => self.s_imr = value & S_INT_MASK,
            S_ICR => self.s_ris &= !(value & S_INT_MASK),
            _ => {}
        }
    }
}

impl crate::peripherals::device::I2cSlaveHost for Cc2538I2cm {
    fn attach_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.slaves.push(device);
    }
}

impl Peripheral for Cc2538I2cm {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let lane = (offset & 3) * 8;
        Ok((self.read_reg(offset & !3) >> lane) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    /// Every field sits in byte 0 of its register, so a byte store to lane 0
    /// is the whole write; the upper lanes carry nothing.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_reg(offset, value as u32);
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn i2c_slave_host(&mut self) -> Option<&mut dyn crate::peripherals::device::I2cSlaveHost> {
        Some(self)
    }

    fn on_system_reset(&mut self) {
        self.reset_registers();
    }

    fn on_node_reset(&mut self) {
        self.reset_registers();
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_active())
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> crate::PeripheralTickResult {
        crate::PeripheralTickResult {
            irq: self.irq_active(),
            ..Default::default()
        }
    }

    /// The line changes only through this block's own MMIO, after which the
    /// bus re-checks this predicate, so an idle controller leaves the walk.
    fn legacy_tick_active(&self) -> bool {
        self.irq_active()
    }

    fn drives_central_device_time(&self) -> bool {
        true
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
            crate::SimulationError::NotImplemented(format!("Cc2538I2cm snapshot decode: {error}"))
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
    use crate::peripherals::device::I2cSlaveHost;
    use std::sync::{Arc, Mutex};

    /// A register-pointer device: the first written byte selects a register,
    /// later writes store into it, reads return consecutive registers.
    struct RegDevice {
        addr: u8,
        regs: [u8; 4],
        ptr: Option<usize>,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl I2cDevice for RegDevice {
        fn address(&self) -> u8 {
            self.addr
        }
        fn read(&mut self) -> u8 {
            let p = self.ptr.unwrap_or(0);
            self.ptr = Some((p + 1) % 4);
            self.regs[p]
        }
        fn write(&mut self, data: u8) {
            self.events.lock().unwrap().push(format!("w{data:02x}"));
            match self.ptr {
                None => self.ptr = Some(data as usize % 4),
                Some(p) => {
                    self.regs[p] = data;
                    self.ptr = Some((p + 1) % 4);
                }
            }
        }
        fn start(&mut self) {
            self.events.lock().unwrap().push("start".into());
        }
        fn stop(&mut self) {
            self.ptr = None;
            self.events.lock().unwrap().push("stop".into());
        }
    }

    fn with_device() -> (Cc2538I2cm, Arc<Mutex<Vec<String>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut i = Cc2538I2cm::new();
        i.attach_slave(Box::new(RegDevice {
            addr: 0x48,
            regs: [0x19, 0x80, 0x60, 0xA0],
            ptr: None,
            events: events.clone(),
        }));
        i.write_reg(CR, CR_MFE);
        (i, events)
    }

    #[test]
    fn reset_values() {
        let i = Cc2538I2cm::new();
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE);
        assert_eq!(i.read_reg(TPR), 0x01);
        assert_eq!(i.read_reg(CR), 0);
        assert_eq!(i.read_reg(RIS), 0);
    }

    #[test]
    fn commands_are_ignored_until_mfe() {
        let (mut i, events) = with_device();
        i.write_reg(CR, 0);
        i.write_reg(SA, 0x48 << 1);
        i.write_reg(CTRL_STAT, 0x07);
        assert_eq!(i.read_reg(RIS), 0);
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn single_send_to_absent_address_sets_adrack_and_releases() {
        let (mut i, _) = with_device();
        i.write_reg(SA, 0x50 << 1);
        i.write_reg(CTRL_STAT, 0x07);
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE | STAT_ADRACK | STAT_ERROR);
        assert_eq!(i.read_reg(RIS), 1);
    }

    #[test]
    fn burst_nack_holds_the_bus_until_error_stop() {
        let (mut i, _) = with_device();
        i.write_reg(SA, 0x50 << 1);
        i.write_reg(CTRL_STAT, 0x03);
        assert_eq!(
            i.read_reg(CTRL_STAT),
            STAT_BUSBSY | STAT_ADRACK | STAT_ERROR
        );
        i.write_reg(CTRL_STAT, 0x04);
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE);
    }

    #[test]
    fn burst_send_then_restart_read() {
        let (mut i, events) = with_device();
        i.write_reg(SA, 0x48 << 1);
        i.write_reg(DR, 0x02);
        i.write_reg(CTRL_STAT, 0x03); // BURST_SEND_START
        assert_eq!(i.read_reg(CTRL_STAT), STAT_BUSBSY);
        i.write_reg(DR, 0x55);
        i.write_reg(CTRL_STAT, 0x05); // BURST_SEND_FINISH
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE);
        assert_eq!(*events.lock().unwrap(), vec!["start", "w02", "w55", "stop"]);

        i.write_reg(SA, 0x48 << 1);
        i.write_reg(DR, 0x01);
        i.write_reg(CTRL_STAT, 0x03); // pointer = 1
        i.write_reg(SA, (0x48 << 1) | 1);
        i.write_reg(CTRL_STAT, 0x0B); // repeated START, receive with ACK
        assert_eq!(i.read_reg(DR), 0x80);
        i.write_reg(CTRL_STAT, 0x05); // receive last, STOP
        assert_eq!(i.read_reg(DR), 0x55);
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE);
    }

    #[test]
    fn interrupt_follows_imr_and_icr() {
        let (mut i, _) = with_device();
        i.write_reg(SA, 0x48 << 1);
        i.write_reg(CTRL_STAT, 0x07);
        assert_eq!(i.read_reg(RIS), 1);
        assert_eq!(i.read_reg(MIS), 0);
        assert!(!i.irq_active());
        i.write_reg(IMR, 1);
        assert_eq!(i.read_reg(MIS), 1);
        assert!(i.irq_active());
        i.write_reg(ICR, 1);
        assert_eq!(i.read_reg(RIS), 0);
        assert!(!i.irq_active());
    }

    #[test]
    fn run_from_idle_without_start_does_nothing() {
        let (mut i, events) = with_device();
        i.write_reg(SA, 0x48 << 1);
        i.write_reg(CTRL_STAT, 0x01);
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE);
        assert_eq!(i.read_reg(RIS), 0);
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn clearing_mfe_releases_the_bus() {
        let (mut i, events) = with_device();
        i.write_reg(SA, 0x48 << 1);
        i.write_reg(CTRL_STAT, 0x03);
        i.write_reg(CR, 0);
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE);
        assert_eq!(events.lock().unwrap().last().unwrap(), "stop");
    }

    /// LPBK | MFE | SFE, own slave at 0x3C with DA set, the external TMP102-
    /// style device still attached at 0x48.
    fn looped() -> (Cc2538I2cm, Arc<Mutex<Vec<String>>>) {
        let (mut i, events) = with_device();
        i.write_reg(CR, CR_LPBK | CR_MFE | CR_SFE);
        i.write_reg(S_OAR, 0x3C);
        i.write_reg(S_CTRL_STAT, 1);
        (i, events)
    }

    #[test]
    fn slave_registers_reset_to_zero_and_keep_field_bits() {
        let mut i = Cc2538I2cm::new();
        for off in [S_OAR, S_CTRL_STAT, S_DR, S_IMR, S_RIS, S_MIS] {
            assert_eq!(i.read_reg(off), 0, "reset @ {off:#x}");
        }
        i.write_reg(S_OAR, 0xFF);
        i.write_reg(S_DR, 0x1FF);
        i.write_reg(S_IMR, 0xFF);
        assert_eq!(i.read_reg(S_OAR), 0x7F);
        assert_eq!(i.read_reg(S_DR), 0xFF);
        assert_eq!(i.read_reg(S_IMR), 0x7);
        // CTRL.DA is write-only; STAT stays clear without a transfer.
        i.write_reg(S_CTRL_STAT, 1);
        assert_eq!(i.read_reg(S_CTRL_STAT), 0);
        // The reserved gap between the two register files.
        i.write_reg(0x400, 0xFFFF_FFFF);
        assert_eq!(i.read_reg(0x400), 0);
    }

    #[test]
    fn loopback_master_write_reaches_own_slave() {
        let (mut i, events) = looped();
        i.write_reg(SA, 0x3C << 1);
        i.write_reg(DR, 0x5A);
        i.write_reg(CTRL_STAT, 0x03); // BURST_SEND_START
        assert_eq!(i.read_reg(CTRL_STAT), STAT_BUSBSY);
        assert_eq!(i.read_reg(S_CTRL_STAT), S_STAT_RREQ | S_STAT_FBR);
        assert_eq!(i.read_reg(S_RIS), S_INT_START | S_INT_DATA);
        assert_eq!(i.read_reg(S_DR), 0x5A);
        assert_eq!(i.read_reg(S_CTRL_STAT), 0, "DR read clears RREQ/FBR");

        i.write_reg(DR, 0xC3);
        i.write_reg(CTRL_STAT, 0x05); // BURST_SEND_FINISH
        assert_eq!(i.read_reg(S_CTRL_STAT), S_STAT_RREQ, "no FBR on byte 2");
        assert_eq!(i.read_reg(S_DR), 0xC3);
        assert_eq!(i.read_reg(S_RIS), S_INT_STOP | S_INT_START | S_INT_DATA);
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE);
        assert_eq!(i.read_reg(RIS), 1);
        // The external device is not on the loopback bus.
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn loopback_master_read_stalls_until_slave_writes_dr() {
        let (mut i, _) = looped();
        i.write_reg(SA, (0x3C << 1) | 1);
        i.write_reg(CTRL_STAT, 0x07); // SINGLE_RECEIVE
        assert_eq!(i.read_reg(CTRL_STAT), STAT_BUSBSY | STAT_BUSY);
        assert_eq!(i.read_reg(S_CTRL_STAT), S_STAT_TREQ);
        assert_eq!(i.read_reg(RIS), 0);
        // A command while stalled is ignored.
        i.write_reg(CTRL_STAT, 0x04);
        assert_eq!(i.read_reg(CTRL_STAT), STAT_BUSBSY | STAT_BUSY);

        i.write_reg(S_DR, 0xA7);
        assert_eq!(i.read_reg(S_CTRL_STAT), 0);
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE);
        assert_eq!(i.read_reg(DR), 0xA7);
        assert_eq!(i.read_reg(RIS), 1);
        assert_eq!(i.read_reg(S_RIS), S_INT_STOP | S_INT_START | S_INT_DATA);
    }

    #[test]
    fn loopback_wrong_address_or_inactive_slave_is_not_acknowledged() {
        let (mut i, _) = looped();
        i.write_reg(SA, 0x48 << 1); // the external device's address
        i.write_reg(CTRL_STAT, 0x07);
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE | STAT_ADRACK | STAT_ERROR);
        assert_eq!(i.read_reg(S_RIS), S_INT_START | S_INT_STOP);

        i.write_reg(S_ICR, 0x7);
        i.write_reg(S_CTRL_STAT, 0); // DA off
        i.write_reg(SA, 0x3C << 1);
        i.write_reg(CTRL_STAT, 0x07);
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE | STAT_ADRACK | STAT_ERROR);
        assert_eq!(i.read_reg(S_RIS), 0, "an inactive slave sees no conditions");
    }

    #[test]
    fn without_lpbk_the_own_slave_is_not_on_the_bus() {
        let (mut i, events) = with_device();
        i.write_reg(CR, CR_MFE | CR_SFE);
        i.write_reg(S_OAR, 0x3C);
        i.write_reg(S_CTRL_STAT, 1);
        i.write_reg(SA, 0x3C << 1);
        i.write_reg(CTRL_STAT, 0x07);
        assert_eq!(i.read_reg(CTRL_STAT), STAT_IDLE | STAT_ADRACK | STAT_ERROR);
        assert_eq!(i.read_reg(S_RIS), 0);
        i.write_reg(SA, 0x48 << 1);
        i.write_reg(CTRL_STAT, 0x07);
        assert_eq!(*events.lock().unwrap(), vec!["start", "w00", "stop"]);
    }

    #[test]
    fn slave_interrupts_share_the_line_and_clear_by_icr() {
        let (mut i, _) = looped();
        i.write_reg(SA, 0x3C << 1);
        i.write_reg(CTRL_STAT, 0x07);
        assert!(!i.irq_active());
        i.write_reg(S_IMR, S_INT_DATA);
        assert_eq!(i.read_reg(S_MIS), S_INT_DATA);
        assert!(i.irq_active());
        i.write_reg(S_ICR, S_INT_DATA);
        assert_eq!(i.read_reg(S_RIS), S_INT_START | S_INT_STOP);
        assert!(!i.irq_active());
        i.write_reg(S_IMR, S_INT_STOP);
        assert!(i.irq_active());
        i.write_reg(S_ICR, 0x7);
        assert!(!i.irq_active());
    }

    #[test]
    fn byte_access_and_snapshot_round_trip() {
        let (mut i, _) = with_device();
        Peripheral::write(&mut i, TPR, 0xFF).unwrap();
        Peripheral::write(&mut i, TPR + 1, 0xFF).unwrap();
        assert_eq!(i.read_reg(TPR), 0x7F);
        assert_eq!(Peripheral::read(&i, CTRL_STAT).unwrap(), STAT_IDLE as u8);
        let snap = i.snapshot();
        let mut j = Cc2538I2cm::new();
        j.restore(snap).unwrap();
        assert_eq!(j.read_reg(TPR), 0x7F);
        assert_eq!(j.read_reg(CR), CR_MFE);
    }
}
