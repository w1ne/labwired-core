// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! PolarFire SoC MSS I2C (`mpfs_i2c`) — master mode of the CoreI2C-family
//! controller Renode maps as `I2C.MPFS_I2C` at 0x2010A000 (I2C0, PLIC 58)
//! and 0x2010B000 (I2C1, PLIC 61) in `polarfire-soc.repl`.
//!
//! Register map (8-bit registers on 4-byte strides; Renode `MPFS_I2C`,
//! Microchip `mss_i2c_regs.h`):
//!
//! | Offset | Name | Access | Meaning |
//! | --- | --- | --- | --- |
//! | 0x00 | CTRL | R/W | bit0 CR0, bit1 CR1, bit2 AA, bit3 SI, bit4 STO, bit5 STA, bit6 ENS1, bit7 CR2 |
//! | 0x04 | STATUS | R | serial state code (table below); 0xF8 at reset |
//! | 0x08 | DATA | R/W | byte to send / last byte received |
//! | 0x0C | SLAVE0_ADDR | R/W | stored only |
//! | 0x10 | SMBUS | R/W | stored only |
//! | 0x14 | FREQ | R/W | stored only |
//! | 0x18 | GLITCHREG | R/W | stored only |
//! | 0x1C | SLAVE1_ADDR | R/W | stored only |
//!
//! The controller advances when firmware clears SI (or sets STA while idle),
//! as the `mss_i2c.c` interrupt handler drives it, and then sets SI with the
//! new STATUS. The interrupt line is SI while ENS1 is set. Master status codes
//! (Renode's `State` enum; the CoreI2C serial-state table):
//!
//! | Code | Meaning |
//! | --- | --- |
//! | 0x08 / 0x10 | START / repeated START sent |
//! | 0x18 / 0x20 | SLA+W sent, ACK / NACK |
//! | 0x28 | data byte sent, ACK |
//! | 0x40 / 0x48 | SLA+R sent, ACK / NACK |
//! | 0x50 / 0x58 | data byte received, ACK / NACK returned (CTRL.AA) |
//! | 0xF8 | idle, no relevant state |
//!
//! After an address or data phase, STA sends a repeated START, STO a STOP
//! (back to 0xF8 with SI clear, STO self-clearing), and both a STOP then a
//! START. An address with no attached device is NACKed. Attached devices
//! always ACK data bytes, so 0x30 (data NACK) never occurs.
//!
//! Not modelled: slave mode (SLAVE0/1_ADDR matching and the 0x60..0xC8
//! codes), SMBus, bus arbitration, the bit-rate divider (CR0..CR2, FREQ) and
//! transfer timing: each phase completes at the SI-clearing write.

use crate::peripherals::device::{drain_i2c_pin_drives, I2cDevice};
use crate::{Peripheral, SimResult};

const CTRL: u64 = 0x00;
const STATUS: u64 = 0x04;
const DATA: u64 = 0x08;
const SLAVE0_ADDR: u64 = 0x0C;
const SMBUS: u64 = 0x10;
const FREQ: u64 = 0x14;
const GLITCHREG: u64 = 0x18;
const SLAVE1_ADDR: u64 = 0x1C;

const AA: u8 = 1 << 2;
const SI: u8 = 1 << 3;
const STO: u8 = 1 << 4;
const STA: u8 = 1 << 5;
const ENS1: u8 = 1 << 6;

const ST_START: u8 = 0x08;
const ST_RESTART: u8 = 0x10;
const ST_SLAW_ACK: u8 = 0x18;
const ST_SLAW_NACK: u8 = 0x20;
const ST_TX_ACK: u8 = 0x28;
const ST_SLAR_ACK: u8 = 0x40;
const ST_SLAR_NACK: u8 = 0x48;
const ST_RX_ACK: u8 = 0x50;
const ST_RX_NACK: u8 = 0x58;
const ST_IDLE: u8 = 0xF8;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct MpfsI2c {
    ctrl: u8,
    status: u8,
    data: u8,
    slave0_addr: u8,
    smbus: u8,
    freq: u8,
    glitchreg: u8,
    slave1_addr: u8,
    /// Index into `slaves` of the device addressed by the current transfer.
    selected: Option<usize>,
    #[serde(skip)]
    slaves: Vec<Box<dyn I2cDevice>>,
}

impl std::fmt::Debug for MpfsI2c {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MpfsI2c")
            .field("ctrl", &self.ctrl)
            .field("status", &self.status)
            .field("data", &self.data)
            .field("selected", &self.selected)
            .field("slaves", &self.slaves.len())
            .finish_non_exhaustive()
    }
}

impl Default for MpfsI2c {
    fn default() -> Self {
        Self::new()
    }
}

impl MpfsI2c {
    pub fn new() -> Self {
        Self {
            ctrl: 0,
            status: ST_IDLE,
            data: 0,
            slave0_addr: 0,
            smbus: 0,
            freq: 0,
            glitchreg: 0,
            slave1_addr: 0,
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
        self.ctrl & (ENS1 | SI) == ENS1 | SI
    }

    fn deselect(&mut self) {
        if let Some(i) = self.selected.take() {
            self.slaves[i].stop();
        }
    }

    fn complete(&mut self, status: u8) {
        self.status = status;
        self.ctrl |= SI;
    }

    fn write_ctrl(&mut self, value: u8) {
        // SI is set only by the controller; firmware can clear it, not set it.
        let si = self.ctrl & SI & value;
        let clearing_si = self.ctrl & SI != 0 && si == 0;
        self.ctrl = (value & !SI) | si;
        if self.ctrl & ENS1 == 0 {
            self.deselect();
            self.ctrl &= !(SI | STO);
            self.status = ST_IDLE;
            return;
        }
        if self.ctrl & SI != 0 {
            return;
        }
        if self.status == ST_IDLE {
            if self.ctrl & STO != 0 {
                self.ctrl &= !STO;
            }
            if self.ctrl & STA != 0 {
                self.complete(ST_START);
            }
            return;
        }
        if clearing_si {
            self.step();
        }
    }

    /// The action the controller takes for the current STATUS once SI clears.
    fn step(&mut self) {
        match self.status {
            ST_START | ST_RESTART => self.address(),
            ST_SLAR_ACK | ST_RX_ACK => self.receive(),
            _ => match self.ctrl & (STA | STO) {
                s if s == STA | STO => {
                    self.deselect();
                    self.ctrl &= !STO;
                    self.complete(ST_START);
                }
                STO => {
                    self.deselect();
                    self.ctrl &= !STO;
                    self.status = ST_IDLE;
                }
                STA => {
                    self.deselect();
                    self.complete(ST_RESTART);
                }
                _ if matches!(self.status, ST_SLAW_ACK | ST_TX_ACK) => {
                    if let Some(i) = self.selected {
                        self.slaves[i].write(self.data);
                    }
                    self.complete(ST_TX_ACK);
                }
                // A NACK state with neither STA nor STO: the bus waits.
                _ => {}
            },
        }
    }

    fn address(&mut self) {
        let address = self.data >> 1;
        let read = self.data & 1 != 0;
        let found = self.slaves.iter().position(|d| d.claims_address(address));
        let status = match (found, read) {
            (Some(i), _) => {
                self.slaves[i].select_address(address);
                self.slaves[i].start();
                self.selected = Some(i);
                if read {
                    ST_SLAR_ACK
                } else {
                    ST_SLAW_ACK
                }
            }
            (None, true) => ST_SLAR_NACK,
            (None, false) => ST_SLAW_NACK,
        };
        self.complete(status);
    }

    fn receive(&mut self) {
        self.data = match self.selected {
            Some(i) => self.slaves[i].read(),
            None => 0xFF,
        };
        let status = if self.ctrl & AA != 0 {
            ST_RX_ACK
        } else {
            ST_RX_NACK
        };
        self.complete(status);
    }

    fn read_reg(&self, offset: u64) -> u8 {
        match offset {
            CTRL => self.ctrl,
            STATUS => self.status,
            DATA => self.data,
            SLAVE0_ADDR => self.slave0_addr,
            SMBUS => self.smbus,
            FREQ => self.freq,
            GLITCHREG => self.glitchreg,
            SLAVE1_ADDR => self.slave1_addr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u8) {
        match offset {
            CTRL => self.write_ctrl(value),
            DATA => self.data = value,
            SLAVE0_ADDR => self.slave0_addr = value,
            SMBUS => self.smbus = value,
            FREQ => self.freq = value,
            GLITCHREG => self.glitchreg = value,
            SLAVE1_ADDR => self.slave1_addr = value,
            _ => {}
        }
    }
}

impl crate::peripherals::device::I2cSlaveHost for MpfsI2c {
    fn attach_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.slaves.push(device);
    }
}

impl Peripheral for MpfsI2c {
    /// Each register is byte 0 of its 32-bit slot; bytes 1..3 read 0.
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(if offset % 4 == 0 {
            self.read_reg(offset)
        } else {
            0
        })
    }

    /// A word store arrives as four byte writes; only byte 0 carries a field.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset % 4 == 0 {
            self.write_reg(offset, value);
        }
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
            crate::SimulationError::NotImplemented(format!("MpfsI2c snapshot decode: {error}"))
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

    /// A register-pointer device: the first byte written selects a register,
    /// later writes store into it, reads return consecutive registers.
    struct RegDevice {
        addr: u8,
        regs: [u8; 4],
        ptr: usize,
        fresh: bool,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl I2cDevice for RegDevice {
        fn address(&self) -> u8 {
            self.addr
        }
        fn read(&mut self) -> u8 {
            let v = self.regs[self.ptr % 4];
            self.ptr += 1;
            v
        }
        fn write(&mut self, data: u8) {
            if self.fresh {
                self.ptr = data as usize;
                self.fresh = false;
            } else {
                self.regs[self.ptr % 4] = data;
                self.ptr += 1;
            }
        }
        fn start(&mut self) {
            self.fresh = true;
            self.events.lock().unwrap().push("start".into());
        }
        fn stop(&mut self) {
            self.events.lock().unwrap().push("stop".into());
        }
    }

    fn with_device() -> (MpfsI2c, Arc<Mutex<Vec<String>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut i2c = MpfsI2c::new();
        i2c.attach_slave(Box::new(RegDevice {
            addr: 0x50,
            regs: [0xE5, 0x11, 0x22, 0x33],
            ptr: 0,
            fresh: false,
            events: events.clone(),
        }));
        (i2c, events)
    }

    fn ctrl(i: &mut MpfsI2c, v: u8) {
        i.write(CTRL, v).unwrap();
    }

    fn status(i: &MpfsI2c) -> u8 {
        i.read(STATUS).unwrap()
    }

    fn si(i: &MpfsI2c) -> bool {
        i.read(CTRL).unwrap() & SI != 0
    }

    #[test]
    fn reset_status_is_idle_and_line_low() {
        let i = MpfsI2c::new();
        assert_eq!(status(&i), ST_IDLE);
        assert_eq!(i.read(CTRL).unwrap(), 0);
        assert_eq!(i.irq_line_level(), Some(false));
        assert_eq!(i.read(STATUS + 1).unwrap(), 0);
    }

    #[test]
    fn start_needs_ens1() {
        let mut i = MpfsI2c::new();
        ctrl(&mut i, STA);
        assert_eq!(status(&i), ST_IDLE);
        assert!(!si(&i));
        ctrl(&mut i, ENS1 | STA);
        assert_eq!(status(&i), ST_START);
        assert!(si(&i));
        assert_eq!(i.irq_line_level(), Some(true));
    }

    #[test]
    fn firmware_cannot_set_si() {
        let mut i = MpfsI2c::new();
        ctrl(&mut i, ENS1 | SI);
        assert!(!si(&i));
        assert_eq!(status(&i), ST_IDLE);
    }

    #[test]
    fn absent_address_is_nacked_then_stop_returns_idle() {
        let mut i = MpfsI2c::new();
        ctrl(&mut i, ENS1 | STA);
        i.write(DATA, 0x50 << 1).unwrap();
        ctrl(&mut i, ENS1);
        assert_eq!(status(&i), ST_SLAW_NACK);
        assert!(si(&i));
        ctrl(&mut i, ENS1 | STO);
        assert_eq!(status(&i), ST_IDLE);
        assert_eq!(
            i.read(CTRL).unwrap(),
            ENS1,
            "STO self-clears, SI stays clear"
        );
        assert_eq!(i.irq_line_level(), Some(false));

        ctrl(&mut i, ENS1 | STA);
        i.write(DATA, (0x50 << 1) | 1).unwrap();
        ctrl(&mut i, ENS1);
        assert_eq!(status(&i), ST_SLAR_NACK);
    }

    #[test]
    fn write_then_repeated_start_read() {
        let (mut i, events) = with_device();
        ctrl(&mut i, ENS1 | STA);
        i.write(DATA, 0x50 << 1).unwrap();
        ctrl(&mut i, ENS1);
        assert_eq!(status(&i), ST_SLAW_ACK);
        i.write(DATA, 0x00).unwrap(); // register pointer
        ctrl(&mut i, ENS1);
        assert_eq!(status(&i), ST_TX_ACK);
        ctrl(&mut i, ENS1 | STA);
        assert_eq!(status(&i), ST_RESTART);
        i.write(DATA, (0x50 << 1) | 1).unwrap();
        ctrl(&mut i, ENS1 | AA);
        assert_eq!(status(&i), ST_SLAR_ACK);
        ctrl(&mut i, ENS1 | AA);
        assert_eq!(status(&i), ST_RX_ACK);
        assert_eq!(i.read(DATA).unwrap(), 0xE5);
        ctrl(&mut i, ENS1);
        assert_eq!(status(&i), ST_RX_NACK);
        assert_eq!(i.read(DATA).unwrap(), 0x11);
        ctrl(&mut i, ENS1 | STO);
        assert_eq!(status(&i), ST_IDLE);
        assert_eq!(
            *events.lock().unwrap(),
            ["start", "stop", "start", "stop"],
            "repeated start ends the write phase"
        );
    }

    #[test]
    fn written_data_reaches_device() {
        let (mut i, _) = with_device();
        ctrl(&mut i, ENS1 | STA);
        i.write(DATA, 0x50 << 1).unwrap();
        ctrl(&mut i, ENS1);
        for b in [2u8, 0xAB] {
            i.write(DATA, b).unwrap();
            ctrl(&mut i, ENS1);
        }
        ctrl(&mut i, ENS1 | STO | STA);
        assert_eq!(status(&i), ST_START, "STO+STA: stop then start");
        i.write(DATA, (0x50 << 1) | 1).unwrap();
        ctrl(&mut i, ENS1);
        ctrl(&mut i, ENS1);
        // The read pointer continues after the written register.
        assert_eq!(i.read(DATA).unwrap(), 0x33);
    }

    #[test]
    fn clearing_ens1_aborts_to_idle() {
        let (mut i, events) = with_device();
        ctrl(&mut i, ENS1 | STA);
        i.write(DATA, 0x50 << 1).unwrap();
        ctrl(&mut i, ENS1);
        ctrl(&mut i, 0);
        assert_eq!(status(&i), ST_IDLE);
        assert!(!si(&i));
        assert_eq!(*events.lock().unwrap(), ["start", "stop"]);
    }

    #[test]
    fn stored_registers_read_back_and_snapshot_round_trips() {
        let mut i = MpfsI2c::new();
        for (off, v) in [
            (SLAVE0_ADDR, 0x22),
            (SMBUS, 0x05),
            (FREQ, 0x3C),
            (GLITCHREG, 0x03),
            (SLAVE1_ADDR, 0x44),
        ] {
            i.write(off, v).unwrap();
            assert_eq!(i.read(off).unwrap(), v);
        }
        ctrl(&mut i, ENS1 | STA);
        let snap = i.snapshot();
        let mut j = MpfsI2c::new();
        j.restore(snap).unwrap();
        assert_eq!(status(&j), ST_START);
        assert_eq!(j.read(FREQ).unwrap(), 0x3C);
    }
}
