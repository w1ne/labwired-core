// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nRF52 TWI master (`ENABLE = 5`): the byte-register I²C controller that
//! shares a window with TWIM (nRF52840 PS v1.11 §6.30). nrfx `nrfx_twi.c`
//! drives it, and Mbed OS `i2c_api.c` uses that driver for every blocking
//! transfer, so `Wire` on the Arduino Nano 33 BLE runs here and not on TWIM.
//!
//! It is a personality of [`Nrf52Twim`]: the attached devices, `PSEL`,
//! `FREQUENCY`, `ADDRESS`, `ERRORSRC` and the `STOPPED` / `ERROR` /
//! `SUSPENDED` events are the same words on silicon. This file adds what TWI
//! has and TWIM does not:
//!
//! * `TXD` (0x51C) and `RXD` (0x518), one byte each, with `EVENTS_TXDSENT`
//!   (0x11C), `EVENTS_RXDREADY` (0x108) and `EVENTS_BB` (0x138).
//! * `SHORTS`: `BB_SUSPEND` (bit 0) and `BB_STOP` (bit 1).
//! * `INTENSET` / `INTENCLR` bits 1, 2, 7, 9, 14 and 18.
//!
//! Sequence:
//!
//! * `TASKS_STARTTX` / `TASKS_STARTRX` send START (a repeated START when the
//!   bus is held) and the `ADDRESS` byte. No device at that address sets
//!   `ERRORSRC.ANACK` with `EVENTS_ERROR`; the bus is then held until
//!   `TASKS_STOP`.
//! * Transmit: each write to `TXD` sends one byte, then `TXDSENT` and `BB`.
//! * Receive: each byte sets `RXDREADY` and `BB`. The controller stretches
//!   the clock until firmware reads `RXD`, so the next byte starts only after
//!   that read (and after `TASKS_RESUME` when `BB_SUSPEND` held the bus).
//! * `TASKS_STOP` sends STOP after the byte in flight; `EVENTS_STOPPED`
//!   follows. `TASKS_SUSPEND` holds the bus with `EVENTS_SUSPENDED`.
//!
//! Each address and data byte takes nine SCL periods of `FREQUENCY`, STOP
//! one. A driver that polls the events sees them after that wire time.
//!
//! Limits: attached device models acknowledge every data byte, so
//! `ERRORSRC.DNACK` is never set. `OVERRUN` cannot occur, because the next
//! byte waits for the `RXD` read. A STOP that `BB_STOP` asked for does not
//! wait for the `RXD` read of the last byte. The pads carry the narrated
//! waveform of a finished transaction; this personality does not run the
//! bit-level engine on a world `gpio_net`.

use super::{
    Nrf52Twim, WireEvent, ADDRESS_MASK, ERRORSRC_ANACK, ERRORSRC_DNACK, INTEN_ERROR, INTEN_STOPPED,
    INTEN_SUSPENDED, OFF_ERRORSRC, OFF_EVENTS_LASTRX, OFF_EVENTS_LASTTX, OFF_EVENTS_RXSTARTED,
    OFF_EVENTS_TXSTARTED, OFF_INTEN, OFF_INTENCLR, OFF_INTENSET, OFF_RXD_AMOUNT, OFF_RXD_MAXCNT,
    OFF_RXD_PTR, OFF_SHORTS, OFF_TASKS_RESUME, OFF_TASKS_STARTRX, OFF_TASKS_STARTTX,
    OFF_TASKS_STOP, OFF_TASKS_SUSPEND, OFF_TXD_AMOUNT, OFF_TXD_MAXCNT, OFF_TXD_PTR, PENDING_NONE,
};
use std::cell::Cell;

/// `ENABLE` value that selects the TWI master personality (PS §6.30.8.11).
pub(crate) const ENABLE_TWI: u32 = 5;
/// `Nrf52Twim::pending` token: a TWI wire operation is in flight.
pub(super) const PENDING_TWI: u8 = 4;

const OFF_EVENTS_RXDREADY: u64 = 0x108;
const OFF_EVENTS_TXDSENT: u64 = 0x11C;
const OFF_EVENTS_BB: u64 = 0x138;
const OFF_RXD: u64 = 0x518;
const OFF_TXD: u64 = 0x51C;

const SHORT_BB_SUSPEND: u32 = 1 << 0;
const SHORT_BB_STOP: u32 = 1 << 1;
const SHORTS_MASK: u32 = SHORT_BB_SUSPEND | SHORT_BB_STOP;

const INTEN_RXDREADY: u32 = 1 << 2;
const INTEN_TXDSENT: u32 = 1 << 7;
const INTEN_BB: u32 = 1 << 14;
const INTEN_MASK: u32 =
    INTEN_STOPPED | INTEN_RXDREADY | INTEN_TXDSENT | INTEN_ERROR | INTEN_BB | INTEN_SUSPENDED;

const ERRORSRC_OVERRUN: u32 = 1 << 0;
const ERRORSRC_MASK: u32 = ERRORSRC_OVERRUN | ERRORSRC_ANACK | ERRORSRC_DNACK;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Dir {
    #[default]
    Idle,
    Tx,
    Rx,
}

/// What the wire is doing while `pending == PENDING_TWI`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Op {
    #[default]
    None,
    Address,
    TxByte,
    RxByte,
    Stop,
}

/// TWI-only state of an [`Nrf52Twim`].
#[derive(Debug, Default)]
pub(super) struct TwiLegacy {
    shorts: u32,
    events_rxdready: u32,
    events_txdsent: u32,
    events_bb: u32,
    txd: u8,
    rxd: u8,
    /// `RXD` holds a byte firmware has not read. A `Cell`, because the read
    /// that clears it comes through `read_u32(&self)`.
    rxd_unread: Cell<bool>,
    dir: Dir,
    op: Op,
    /// START and the address are out and a device acknowledged.
    addressed: bool,
    /// The address was not acknowledged; only STOP or a new START moves on.
    nacked: bool,
    /// Index of the addressed device in `attached_devices`.
    device: Option<usize>,
    /// `TXD` was written and the byte is not sent yet.
    tx_loaded: bool,
    suspended: bool,
    /// `TASKS_SUSPEND` arrived during a byte: `EVENTS_SUSPENDED` follows it.
    suspend_event_due: bool,
    stop_requested: bool,
    /// Cycle at which the operation in flight ends (0 without a clock).
    due: u64,
}

impl Nrf52Twim {
    pub(super) fn twi_mode(&self) -> bool {
        self.enable & super::ENABLE_MASK == ENABLE_TWI
    }

    /// `ENABLE` changed: the transaction in flight is gone. Events and
    /// `ERRORSRC` stay, as registers do.
    pub(super) fn twi_enable_changed(&mut self) {
        if self.pending == PENDING_TWI {
            self.pending = PENDING_NONE;
            self.busy_cycles = 0;
        }
        let twi = &mut self.twi;
        twi.dir = Dir::Idle;
        twi.op = Op::None;
        twi.addressed = false;
        twi.nacked = false;
        twi.device = None;
        twi.tx_loaded = false;
        twi.suspended = false;
        twi.suspend_event_due = false;
        twi.stop_requested = false;
        self.wire_events.clear();
    }

    /// TWI register read. `None`: a word TWI shares with TWIM.
    pub(super) fn twi_read(&self, offset: u64) -> Option<u32> {
        Some(match offset {
            OFF_EVENTS_RXDREADY => self.twi.events_rxdready,
            OFF_EVENTS_TXDSENT => self.twi.events_txdsent,
            OFF_EVENTS_BB => self.twi.events_bb,
            OFF_SHORTS => self.twi.shorts,
            OFF_INTENSET | OFF_INTENCLR => self.inten & INTEN_MASK,
            OFF_ERRORSRC => self.errorsrc & ERRORSRC_MASK,
            OFF_RXD => {
                self.twi.rxd_unread.set(false);
                u32::from(self.twi.rxd)
            }
            OFF_TXD => u32::from(self.twi.txd),
            // TWIM words that TWI does not have.
            OFF_INTEN | OFF_EVENTS_RXSTARTED | OFF_EVENTS_TXSTARTED | OFF_EVENTS_LASTRX
            | OFF_EVENTS_LASTTX | OFF_RXD_PTR | OFF_RXD_MAXCNT | OFF_RXD_AMOUNT | OFF_TXD_PTR
            | OFF_TXD_MAXCNT | OFF_TXD_AMOUNT => 0,
            _ => return None,
        })
    }

    /// TWI register write. `false`: a word TWI shares with TWIM.
    pub(super) fn twi_write(&mut self, offset: u64, value: u32) -> bool {
        match offset {
            OFF_TASKS_STARTTX | OFF_TASKS_STARTRX if value != 0 => {
                let twi = &mut self.twi;
                twi.dir = if offset == OFF_TASKS_STARTTX {
                    Dir::Tx
                } else {
                    Dir::Rx
                };
                twi.addressed = false;
                twi.nacked = false;
            }
            OFF_TASKS_STOP if value != 0 => self.twi.stop_requested = true,
            OFF_TASKS_SUSPEND if value != 0 => {
                self.twi.suspended = true;
                if self.pending == PENDING_NONE {
                    self.events_suspended = 1;
                } else {
                    self.twi.suspend_event_due = true;
                }
            }
            OFF_TASKS_RESUME if value != 0 => {
                self.twi.suspended = false;
                self.twi.suspend_event_due = false;
            }
            OFF_TASKS_STARTTX | OFF_TASKS_STARTRX | OFF_TASKS_STOP | OFF_TASKS_SUSPEND
            | OFF_TASKS_RESUME => {}
            OFF_EVENTS_RXDREADY if value == 0 => self.twi.events_rxdready = 0,
            OFF_EVENTS_TXDSENT if value == 0 => self.twi.events_txdsent = 0,
            OFF_EVENTS_BB if value == 0 => self.twi.events_bb = 0,
            OFF_EVENTS_RXDREADY | OFF_EVENTS_TXDSENT | OFF_EVENTS_BB => {}
            OFF_SHORTS => self.twi.shorts = value & SHORTS_MASK,
            OFF_INTENSET => self.inten |= value & INTEN_MASK,
            OFF_INTENCLR => self.inten &= !(value & INTEN_MASK),
            OFF_ERRORSRC => self.errorsrc &= !(value & ERRORSRC_MASK),
            OFF_TXD => {
                self.twi.txd = value as u8;
                self.twi.tx_loaded = true;
            }
            // Read-only, or TWIM words that TWI does not have.
            OFF_RXD | OFF_INTEN | OFF_EVENTS_RXSTARTED | OFF_EVENTS_TXSTARTED
            | OFF_EVENTS_LASTRX | OFF_EVENTS_LASTTX | OFF_RXD_PTR | OFF_RXD_MAXCNT
            | OFF_RXD_AMOUNT | OFF_TXD_PTR | OFF_TXD_MAXCNT | OFF_TXD_AMOUNT => {}
            _ => return false,
        }
        self.twi_kick();
        true
    }

    /// Start the next wire operation when the wire is free and one is due.
    pub(super) fn twi_kick(&mut self) {
        if !self.twi_mode() || self.pending != PENDING_NONE {
            return;
        }
        let twi = &self.twi;
        let started = twi.dir != Dir::Idle && !twi.nacked;
        let (op, bits) = if twi.stop_requested {
            (Op::Stop, 1)
        } else if started && !twi.addressed {
            (Op::Address, 9)
        } else if started && twi.dir == Dir::Tx && twi.tx_loaded && !twi.suspended {
            (Op::TxByte, 9)
        } else if started && twi.dir == Dir::Rx && !twi.rxd_unread.get() && !twi.suspended {
            (Op::RxByte, 9)
        } else {
            return;
        };
        let cycles = bits * self.scl_bit_cycles();
        self.twi.op = op;
        self.twi.due = self.clock.as_ref().map_or(0, |c| c.now() + cycles);
        self.pending = PENDING_TWI;
        self.busy_cycles = cycles as u32;
    }

    /// Cycles still to run of the operation in flight (0 without a clock).
    pub(super) fn twi_remaining(&self, now: u64) -> u64 {
        self.twi.due.saturating_sub(now)
    }

    /// The operation in flight is over: move the byte and raise its events.
    pub(super) fn twi_complete(&mut self) {
        let addr7 = (self.address & ADDRESS_MASK) as u8;
        let reading = self.twi.dir == Dir::Rx;
        match std::mem::take(&mut self.twi.op) {
            Op::None => {}
            Op::Address => match self.device_for(addr7) {
                Some(idx) => {
                    self.attached_devices[idx].borrow_mut().start();
                    self.wire_address(addr7, reading, true);
                    self.twi.device = Some(idx);
                    self.twi.addressed = true;
                }
                None => {
                    self.wire_address(addr7, reading, false);
                    self.twi.device = None;
                    self.twi.nacked = true;
                    self.errorsrc |= ERRORSRC_ANACK;
                    self.events_error = 1;
                }
            },
            Op::TxByte => {
                let byte = self.twi.txd;
                if let Some(idx) = self.twi.device {
                    self.attached_devices[idx].borrow_mut().write(byte);
                }
                self.wire_record(WireEvent::Frame(byte, true));
                self.twi.tx_loaded = false;
                self.twi.events_txdsent = 1;
                self.twi_byte_boundary();
            }
            Op::RxByte => {
                let byte = match self.twi.device {
                    Some(idx) => self.attached_devices[idx].borrow_mut().read(),
                    None => 0xFF,
                };
                // The controller NACKs the byte a STOP follows.
                let last = self.twi.shorts & SHORT_BB_STOP != 0 || self.twi.stop_requested;
                self.wire_record(WireEvent::Frame(byte, !last));
                self.twi.rxd = byte;
                self.twi.rxd_unread.set(true);
                self.twi.events_rxdready = 1;
                self.twi_byte_boundary();
            }
            Op::Stop => {
                if let Some(idx) = self.twi.device.take() {
                    self.attached_devices[idx].borrow_mut().stop();
                }
                let twi = &mut self.twi;
                twi.dir = Dir::Idle;
                twi.addressed = false;
                twi.nacked = false;
                twi.tx_loaded = false;
                twi.suspended = false;
                twi.suspend_event_due = false;
                twi.stop_requested = false;
                self.events_stopped = 1;
                self.wire_flush();
            }
        }
        self.twi_kick();
    }

    /// `EVENTS_BB` and what hangs on it.
    fn twi_byte_boundary(&mut self) {
        self.twi.events_bb = 1;
        if self.twi.shorts & SHORT_BB_STOP != 0 {
            self.twi.stop_requested = true;
        }
        if self.twi.shorts & SHORT_BB_SUSPEND != 0 {
            self.twi.suspended = true;
            self.twi.suspend_event_due = true;
        }
        if std::mem::take(&mut self.twi.suspend_event_due) {
            self.events_suspended = 1;
        }
    }

    /// Add the offsets of the TWI-only events that raise the interrupt.
    pub(super) fn twi_irq_events(&self, fired: &mut Vec<u32>) {
        for (event, mask, offset) in [
            (
                self.twi.events_rxdready,
                INTEN_RXDREADY,
                OFF_EVENTS_RXDREADY,
            ),
            (self.twi.events_txdsent, INTEN_TXDSENT, OFF_EVENTS_TXDSENT),
            (self.twi.events_bb, INTEN_BB, OFF_EVENTS_BB),
        ] {
            if event != 0 && self.inten & mask != 0 {
                fired.push(offset as u32);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        OFF_ADDRESS, OFF_ENABLE, OFF_EVENTS_ERROR, OFF_EVENTS_STOPPED, OFF_EVENTS_SUSPENDED,
    };
    use super::*;
    use crate::peripherals::i2c::I2cDevice;
    use crate::peripherals::nrf52::serial_instance::Nrf52SerialInstance;
    use crate::{Bus, DmaRequest, Peripheral, SimulationConfig};
    use std::sync::{Arc, Mutex};

    struct NoRam(SimulationConfig);

    impl Bus for NoRam {
        fn read_u8(&self, _addr: u64) -> crate::SimResult<u8> {
            Ok(0)
        }
        fn write_u8(&mut self, _addr: u64, _value: u8) -> crate::SimResult<()> {
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> crate::SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.0
        }
    }

    /// What the device saw, in order: `S`, `W<byte>`, `R<byte>`, `P`.
    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<String>>>);

    impl Log {
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
        fn push(&self, s: String) {
            self.0.lock().unwrap().push(s);
        }
    }

    struct Dev {
        log: Log,
        next: u8,
    }

    impl I2cDevice for Dev {
        fn address(&self) -> u8 {
            0x40
        }
        fn read(&mut self) -> u8 {
            self.next += 1;
            self.log.push(format!("R{:02x}", self.next));
            self.next
        }
        fn write(&mut self, data: u8) {
            self.log.push(format!("W{data:02x}"));
        }
        fn start(&mut self) {
            self.log.push("S".into());
        }
        fn stop(&mut self) {
            self.log.push("P".into());
        }
    }

    /// A TWI master with one device at 0x40 whose reads count up from 0xA1.
    fn twi() -> (Nrf52Twim, Log) {
        let log = Log::default();
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(Dev {
            log: log.clone(),
            next: 0xA0,
        }));
        t.write_u32(OFF_ENABLE, ENABLE_TWI).unwrap();
        (t, log)
    }

    /// Walk ticks until the wire is free. Returns the cycles that took.
    fn settle(t: &mut Nrf52Twim) -> u64 {
        let mut bus = NoRam(SimulationConfig::default());
        let step = u64::from(bus.0.peripheral_tick_interval.max(1));
        let mut cycles = 0;
        while t.needs_bus_tick() {
            t.tick_with_bus(&mut bus);
            cycles += step;
            assert!(cycles < 10_000_000, "TWI never went idle");
        }
        cycles
    }

    fn rd(t: &Nrf52Twim, offset: u64) -> u32 {
        t.read_u32(offset).unwrap()
    }

    fn wr(t: &mut Nrf52Twim, offset: u64, value: u32) {
        t.write_u32(offset, value).unwrap();
    }

    /// nrfx `twi_tx_start_transfer`: STARTTX, one byte per TXD write, STOP.
    /// TXDSENT comes only after the address and the byte had their wire time.
    #[test]
    fn transmit_sends_one_byte_per_txd_write_then_stops() {
        let (mut t, log) = twi();
        wr(&mut t, OFF_ADDRESS, 0x40);
        wr(&mut t, OFF_TASKS_STARTTX, 1);
        wr(&mut t, OFF_TXD, 0x05);
        assert_eq!(rd(&t, OFF_EVENTS_TXDSENT), 0, "no byte time has passed");
        let cycles = settle(&mut t);
        // Address + data = 18 bits, 256 cycles each at the reset FREQUENCY
        // (250 kbps of the 64 MHz core clock).
        assert!(cycles >= 18 * 256, "took {cycles} cycles");
        assert_eq!(rd(&t, OFF_EVENTS_TXDSENT), 1);
        assert_eq!(rd(&t, OFF_EVENTS_BB), 1);
        assert_eq!(log.take(), ["S", "W05"]);

        wr(&mut t, OFF_EVENTS_TXDSENT, 0);
        wr(&mut t, OFF_TXD, 0x9F);
        settle(&mut t);
        assert_eq!(rd(&t, OFF_EVENTS_TXDSENT), 1);
        assert_eq!(rd(&t, OFF_EVENTS_STOPPED), 0);
        wr(&mut t, OFF_TASKS_STOP, 1);
        settle(&mut t);
        assert_eq!(rd(&t, OFF_EVENTS_STOPPED), 1);
        assert_eq!(log.take(), ["W9f", "P"]);
        assert_eq!(rd(&t, OFF_ERRORSRC), 0);
    }

    /// nrfx `twi_rx_start_transfer` for two bytes: BB_SUSPEND holds the bus
    /// after the first, firmware reads RXD, arms BB_STOP and resumes.
    #[test]
    fn receive_waits_for_resume_and_stops_after_the_last_byte() {
        let (mut t, log) = twi();
        wr(&mut t, OFF_ADDRESS, 0x40);
        wr(&mut t, OFF_SHORTS, SHORT_BB_SUSPEND);
        wr(&mut t, OFF_TASKS_RESUME, 1);
        wr(&mut t, OFF_TASKS_STARTRX, 1);
        settle(&mut t);
        assert_eq!(rd(&t, OFF_EVENTS_RXDREADY), 1);
        assert_eq!(rd(&t, OFF_EVENTS_SUSPENDED), 1);
        assert_eq!(log.take(), ["S", "Ra1"], "one byte, then the bus is held");

        wr(&mut t, OFF_EVENTS_RXDREADY, 0);
        assert_eq!(rd(&t, OFF_RXD), 0xA1);
        wr(&mut t, OFF_SHORTS, SHORT_BB_STOP);
        wr(&mut t, OFF_TASKS_RESUME, 1);
        settle(&mut t);
        assert_eq!(rd(&t, OFF_EVENTS_RXDREADY), 1);
        assert_eq!(rd(&t, OFF_RXD), 0xA2);
        assert_eq!(rd(&t, OFF_EVENTS_STOPPED), 1);
        assert_eq!(log.take(), ["Ra2", "P"]);
    }

    /// Without BB_SUSPEND the clock is stretched until RXD is read.
    #[test]
    fn receive_holds_the_next_byte_until_rxd_is_read() {
        let (mut t, log) = twi();
        wr(&mut t, OFF_ADDRESS, 0x40);
        wr(&mut t, OFF_TASKS_STARTRX, 1);
        settle(&mut t);
        assert_eq!(log.take(), ["S", "Ra1"]);
        assert_eq!(rd(&t, OFF_RXD), 0xA1);
        t.tick();
        settle(&mut t);
        assert_eq!(log.take(), ["Ra2"]);
    }

    /// No device at ADDRESS: ANACK and ERROR, no TXDSENT, and STOP still
    /// answers with STOPPED (nrfx waits for it). ERRORSRC is write-1-to-clear.
    #[test]
    fn absent_address_reports_anack() {
        let (mut t, log) = twi();
        wr(&mut t, OFF_ADDRESS, 0x41);
        wr(&mut t, OFF_TASKS_STARTTX, 1);
        wr(&mut t, OFF_TXD, 0x00);
        settle(&mut t);
        assert_eq!(rd(&t, OFF_EVENTS_ERROR), 1);
        assert_eq!(rd(&t, OFF_EVENTS_TXDSENT), 0);
        assert_eq!(rd(&t, OFF_ERRORSRC), ERRORSRC_ANACK);
        wr(&mut t, OFF_TASKS_STOP, 1);
        settle(&mut t);
        assert_eq!(rd(&t, OFF_EVENTS_STOPPED), 1);
        wr(&mut t, OFF_ERRORSRC, ERRORSRC_ANACK);
        assert_eq!(rd(&t, OFF_ERRORSRC), 0);
        assert!(log.take().is_empty(), "the device never saw the bus");
    }

    /// A write with no STOP (`TASKS_SUSPEND`) then STARTRX is a repeated
    /// START: the device sees no STOP between the two.
    #[test]
    fn suspend_then_startrx_is_a_repeated_start() {
        let (mut t, log) = twi();
        wr(&mut t, OFF_ADDRESS, 0x40);
        wr(&mut t, OFF_TASKS_STARTTX, 1);
        wr(&mut t, OFF_TXD, 0x02);
        settle(&mut t);
        wr(&mut t, OFF_TASKS_SUSPEND, 1);
        assert_eq!(rd(&t, OFF_EVENTS_SUSPENDED), 1);
        wr(&mut t, OFF_SHORTS, SHORT_BB_STOP);
        wr(&mut t, OFF_TASKS_RESUME, 1);
        wr(&mut t, OFF_TASKS_STARTRX, 1);
        settle(&mut t);
        assert_eq!(log.take(), ["S", "W02", "S", "Ra1", "P"]);
    }

    /// The shared window at 0x40003000 hands `ENABLE = 5` to this model.
    #[test]
    fn serial_instance_routes_enable_5_here() {
        let log = Log::default();
        let mut s = Nrf52SerialInstance::new();
        s.attach_i2c(Box::new(Dev {
            log: log.clone(),
            next: 0xA0,
        }));
        s.write_u32(OFF_ENABLE, ENABLE_TWI).unwrap();
        s.write_u32(OFF_ADDRESS, 0x40).unwrap();
        s.write_u32(OFF_TASKS_STARTTX, 1).unwrap();
        s.write_u32(OFF_TXD, 0x77).unwrap();
        let mut bus = NoRam(SimulationConfig::default());
        for _ in 0..100_000 {
            if !s.needs_bus_tick() {
                break;
            }
            s.tick_with_bus(&mut bus);
        }
        assert_eq!(s.read_u32(OFF_EVENTS_TXDSENT).unwrap(), 1);
        assert_eq!(log.take(), ["S", "W77"]);
    }
}
