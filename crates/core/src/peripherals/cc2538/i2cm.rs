// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CC2538 I2C master (`cc2538_i2cm`) at 0x4002_0000 (SWRU319C Table 4-1),
//! interrupt 8 in the regular map (Table 5-2). Renode's `cc2538.repl` has no
//! I2C model, so this goes beyond Renode parity.
//!
//! Register map (SWRU319C §21.6, the Stellaris-family I2C master):
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
//! | 0x020 | I2CM_CR | R/W | SFE 5, MFE 4 (master enable), LPBK 0 (stored only) |
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
//! Not modelled: the I2C slave (I2CS_* at 0x4002_0800 stays a stub), loopback
//! (LPBK), multi-master arbitration (ARBLST is never set), clock stretching,
//! the SCL rate from TPR and transfer time (BUSY is never seen set, because
//! each command completes at its CTRL write), glitch filtering, and the
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

const CTRL_RUN: u32 = 1 << 0;
const CTRL_START: u32 = 1 << 1;
const CTRL_STOP: u32 = 1 << 2;

const STAT_ERROR: u32 = 1 << 1;
const STAT_ADRACK: u32 = 1 << 2;
const STAT_DATACK: u32 = 1 << 3;
const STAT_IDLE: u32 = 1 << 5;
const STAT_BUSBSY: u32 = 1 << 6;

const CR_MFE: u32 = 1 << 4;
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
        self.ris & self.imr & 1 != 0
    }

    fn status(&self) -> u32 {
        let mut stat = self.error;
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
    }

    /// START (or repeated START) and the address phase. Returns whether the
    /// address was acknowledged.
    /// A repeated START is not a STOP: the device it re-addresses sees only
    /// a new `start()`; one it leaves sees `stop()`.
    fn address(&mut self) -> bool {
        self.holding = true;
        let address = ((self.sa >> 1) & 0x7F) as u8;
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

    /// One data byte in the SA.RS direction.
    fn data(&mut self) {
        let Some(i) = self.selected else {
            self.error = STAT_ERROR | STAT_DATACK;
            return;
        };
        if self.sa & 1 != 0 {
            self.dr = self.slaves[i].read() as u32;
        } else {
            self.slaves[i].write(self.dr as u8);
        }
    }

    /// ACK is not read: it only shapes the acknowledge the master returns on
    /// a received byte, which attached devices do not observe.
    fn command(&mut self, ctrl: u32) {
        let run = ctrl & CTRL_RUN != 0;
        let start = ctrl & CTRL_START != 0;
        let stop = ctrl & CTRL_STOP != 0;
        // Without MFE, without RUN or STOP, or from idle without START, the
        // write is not a command of the master state machine.
        if self.cr & CR_MFE == 0 || !(run || stop) || (!self.holding && !start) {
            return;
        }
        self.error = 0;
        if run && (!start || self.address()) {
            self.data();
        }
        if stop {
            self.release();
        }
        self.ris = 1;
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
