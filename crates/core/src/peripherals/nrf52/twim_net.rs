// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nRF52 TWIM (EasyDMA I²C controller) on a world `gpio_net`: a bit-level
//! controller between chips, on the shared wire engine
//! ([`crate::peripherals::i2c_bitbus`]).
//!
//! Off a net `twim.rs` moves a whole EasyDMA buffer to an attached device
//! model per task. When a pad `PSEL.SCL` or `PSEL.SDA` names is on a net (and
//! `ENABLE = 6`) the wire belongs to several chips and the tasks run this
//! engine instead (nRF52840 PS v1.11 §6.31):
//!
//! * `TASKS_STARTTX` / `TASKS_STARTRX`: EasyDMA reads the TX buffer
//!   (`TXD.PTR`/`MAXCNT`), START (a repeated START from a held bus) and the
//!   `ADDRESS` byte with R/W go out, `EVENTS_TXSTARTED` / `RXSTARTED`. Bytes
//!   are sent from the buffer, or received and ACKed into it. Starting the
//!   last byte fires `EVENTS_LASTTX` / `LASTRX` and its `SHORTS`
//!   (`LASTTX_STARTRX`, `LASTTX_SUSPEND`, `LASTTX_STOP`, `LASTRX_STARTTX`,
//!   `LASTRX_SUSPEND`, `LASTRX_STOP`). A received byte is NACKed when STOP or
//!   a change of direction follows it. With nothing to do after a buffer the
//!   controller holds SCL low until the next task.
//! * A NACK sets `ERRORSRC.ANACK` (address) or `DNACK` (data) with
//!   `EVENTS_ERROR`, and the controller holds the bus until `TASKS_STOP`
//!   (nrfx's `RESUME` + `STOP`), or stops at once when a `LASTTX_STOP`
//!   already asked for it. `TXD.AMOUNT` counts the NACKed byte.
//! * `TASKS_STOP` sends STOP after the byte in flight (at once from a held
//!   bus) and `EVENTS_STOPPED` follows; `TASKS_SUSPEND` holds the bus after
//!   the byte with `EVENTS_SUSPENDED` until `TASKS_RESUME`.
//! * Received bytes reach RAM at `RXD.PTR` before `STOPPED`/`SUSPENDED`
//!   (and before a following phase starts); `RXD.AMOUNT` counts them.
//!
//! Timing is `FREQUENCY` (K100/K250/K400 of the 64 MHz core clock): SCL low
//! and high half a bit period each, SDA changed a quarter of the low period
//! after SCL falls. The SCL high period counts from when SCL is seen high,
//! so a target stretching the clock holds the controller.

use super::{
    Nrf52Twim, ADDRESS_MASK, ERRORSRC_ANACK, ERRORSRC_DNACK, MAXCNT_MASK, SHORT_LASTRX_STARTTX,
    SHORT_LASTRX_STOP, SHORT_LASTRX_SUSPEND, SHORT_LASTTX_STARTRX, SHORT_LASTTX_STOP,
    SHORT_LASTTX_SUSPEND,
};
use crate::peripherals::i2c_bitbus::{BitBus, BusEvent, Timing};
use crate::peripherals::pad_lines::PadLines;

/// Scheduler token for the net engine (the byte-level path uses 1 and 2).
pub(super) const NET_TOKEN: u32 = 0x5457_0000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum MState {
    #[default]
    Idle,
    /// START (or repeated START) on its way; the address follows.
    Starting,
    /// The address byte is out.
    Address,
    /// A data byte is on the wire.
    Busy,
    /// Holding SCL low for the next task.
    Hold,
    /// STOP on its way.
    Stopping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Task {
    Tx,
    Rx,
}

/// The net state, kept beside the register file.
#[derive(Debug, Default)]
pub(super) struct TwimNet {
    bus: BitBus,
    m: MState,
    /// Direction of the phase on the wire (`true`: receiving).
    reading: bool,
    /// A task waiting for EasyDMA (and for the byte in flight to end).
    task: Option<Task>,
    tx: Vec<u8>,
    tx_sent: usize,
    rx: Vec<u8>,
    rx_max: usize,
    /// RX bytes not yet in RAM.
    rx_dirty: bool,
    stop: bool,
    suspend: bool,
    /// The bus is held after a NACK until `TASKS_STOP`.
    errored: bool,
    /// `STOPPED` / `SUSPENDED` wait for the RX bytes to reach RAM.
    deferred_stopped: bool,
    deferred_suspended: bool,
    armed: Vec<u64>,
    irq_armed: bool,
}

impl Nrf52Twim {
    fn net_pads(&self) -> Option<std::sync::Arc<PadLines>> {
        self.lines.clone()
    }

    /// `true` when this TWIM is enabled on pads that sit on a world
    /// `gpio_net`.
    pub(super) fn net_mode(&self) -> bool {
        self.lines.is_some() && (self.claim_scl.on_net() || self.claim_sda.on_net())
    }

    fn net_now(&self) -> u64 {
        self.clock.as_ref().map_or(0, crate::CycleClock::now)
    }

    fn net_timing(&self) -> Timing {
        let bit = self.scl_bit_cycles();
        let low = (bit / 2).max(4);
        Timing::new(low, (bit - bit / 2).max(2), (low / 4).max(1))
    }

    /// `true` while a transfer is anywhere between a task and its STOP.
    pub(super) fn net_busy(&self) -> bool {
        self.net.m != MState::Idle || self.net.task.is_some()
    }

    /// `true` while EasyDMA has RAM to move.
    pub(super) fn net_needs_bus(&self) -> bool {
        self.net.task.is_some() || self.net.rx_dirty
    }

    /// A task register write in net mode. Returns `true` when handled here.
    pub(super) fn net_task(&mut self, offset: u64) -> bool {
        let Some(pads) = self.net_pads() else {
            return false;
        };
        let now = self.net_now();
        pads.set_stamp(Some(now));
        match offset {
            super::OFF_TASKS_STARTTX => self.net.task = Some(Task::Tx),
            super::OFF_TASKS_STARTRX => self.net.task = Some(Task::Rx),
            super::OFF_TASKS_STOP => match self.net.m {
                MState::Idle if self.net.task.is_none() => {
                    if self.net.rx_dirty {
                        self.net.deferred_stopped = true;
                    } else {
                        self.events_stopped = 1;
                    }
                }
                MState::Hold => {
                    self.net.task = None;
                    self.net.errored = false;
                    self.net.m = MState::Stopping;
                    self.net.bus.m_stop(now, &pads);
                }
                _ => {
                    self.net.task = None;
                    self.net.stop = true;
                }
            },
            super::OFF_TASKS_SUSPEND => {
                if self.net.m != MState::Idle {
                    self.net.suspend = true;
                }
            }
            super::OFF_TASKS_RESUME => {
                if std::mem::take(&mut self.net.suspend) && self.net.m == MState::Hold {
                    self.net_after_byte(now, &pads);
                }
            }
            _ => {
                pads.set_stamp(None);
                return false;
            }
        }
        self.net_process(now, &pads);
        pads.set_stamp(None);
        true
    }

    /// The interrupt level, for the scheduler path.
    pub(super) fn net_irq(&self) -> bool {
        self.irq_from_events().irq
    }

    /// EasyDMA: write received bytes to RAM, then read the TX buffer for a
    /// waiting task and start it.
    pub(super) fn net_dma(&mut self, bus: &mut dyn crate::Bus) {
        let Some(pads) = self.net_pads() else {
            return;
        };
        let now = self.net_now();
        pads.set_stamp(Some(now));
        if std::mem::take(&mut self.net.rx_dirty) {
            let ptr = u64::from(self.rxd_ptr);
            crate::with_device_write_bypass(bus, |bus| {
                for (i, &b) in self.net.rx.iter().enumerate() {
                    let _ = bus.write_u8(ptr + i as u64, b);
                }
            });
            self.rxd_amount = self.net.rx.len() as u32;
        }
        if std::mem::take(&mut self.net.deferred_stopped) {
            self.events_stopped = 1;
        }
        if std::mem::take(&mut self.net.deferred_suspended) {
            self.events_suspended = 1;
        }
        // A task starts from an idle or held bus; a byte in flight finishes
        // first.
        if matches!(self.net.m, MState::Idle | MState::Hold) {
            if let Some(task) = self.net.task.take() {
                self.net_begin(task, bus, now, &pads);
            }
        }
        self.net_process(now, &pads);
        pads.set_stamp(None);
    }

    fn net_begin(&mut self, task: Task, bus: &mut dyn crate::Bus, now: u64, pads: &PadLines) {
        self.net.bus.timing = self.net_timing();
        self.net.errored = false;
        self.net.suspend = false;
        match task {
            Task::Tx => {
                let ptr = u64::from(self.txd_ptr);
                let n = (self.txd_maxcnt & MAXCNT_MASK) as usize;
                self.net.tx = crate::with_device_write_bypass(bus, |bus| {
                    (0..n)
                        .map(|i| bus.read_u8(ptr + i as u64).unwrap_or(0))
                        .collect()
                });
                self.net.tx_sent = 0;
                self.txd_amount = 0;
                self.net.reading = false;
                self.events_txstarted = 1;
            }
            Task::Rx => {
                self.net.rx.clear();
                self.net.rx_max = (self.rxd_maxcnt & MAXCNT_MASK) as usize;
                self.rxd_amount = 0;
                self.net.reading = true;
                self.events_rxstarted = 1;
            }
        }
        self.net.m = MState::Starting;
        self.net.bus.m_start(now, pads);
    }

    /// The byte (or address) in flight is done: STOP, a waiting task, a
    /// suspend, the next byte, or hold.
    fn net_after_byte(&mut self, now: u64, pads: &PadLines) {
        if std::mem::take(&mut self.net.stop) {
            self.net.m = MState::Stopping;
            self.net.bus.m_stop(now, pads);
            return;
        }
        if self.net.task.is_some() {
            // EasyDMA runs first (`net_dma`); the bus is held meanwhile.
            self.net.m = MState::Hold;
            if self.net.reading && !self.net.rx.is_empty() {
                self.net.rx_dirty = true;
            }
            return;
        }
        if self.net.suspend {
            self.net.m = MState::Hold;
            if self.net.reading {
                self.net.rx_dirty = true;
            }
            self.net.deferred_suspended = true;
            return;
        }
        if self.net.reading {
            if self.net.rx.len() < self.net.rx_max {
                self.net_recv_next(now, pads);
            } else {
                self.net.rx_dirty = true;
                self.net.m = MState::Hold;
            }
        } else if self.net.tx_sent < self.net.tx.len() {
            self.net_send_next(now, pads);
        } else {
            self.net.m = MState::Hold;
        }
    }

    fn net_send_next(&mut self, now: u64, pads: &PadLines) {
        let byte = self.net.tx[self.net.tx_sent];
        if self.net.tx_sent + 1 == self.net.tx.len() {
            self.net_last(false);
        }
        self.net.m = MState::Busy;
        self.net.bus.m_send(byte, now, pads);
    }

    fn net_recv_next(&mut self, now: u64, pads: &PadLines) {
        if self.net.rx.len() + 1 == self.net.rx_max {
            self.net_last(true);
        }
        self.net.m = MState::Busy;
        self.net.bus.m_recv(now, pads);
    }

    /// `EVENTS_LASTTX` / `LASTRX` and the shortcuts it triggers.
    fn net_last(&mut self, rx: bool) {
        let (to_other, suspend, stop, other) = if rx {
            self.events_lastrx = 1;
            (
                SHORT_LASTRX_STARTTX,
                SHORT_LASTRX_SUSPEND,
                SHORT_LASTRX_STOP,
                Task::Tx,
            )
        } else {
            self.events_lasttx = 1;
            (
                SHORT_LASTTX_STARTRX,
                SHORT_LASTTX_SUSPEND,
                SHORT_LASTTX_STOP,
                Task::Rx,
            )
        };
        if self.shorts & to_other != 0 {
            self.net.task = Some(other);
        } else if self.shorts & suspend != 0 {
            self.net.suspend = true;
        } else if self.shorts & stop != 0 {
            self.net.stop = true;
        }
    }

    /// A NACK: `ERROR`, and the bus is held until `TASKS_STOP` (at once when
    /// a STOP was already asked for).
    fn net_nack(&mut self, source: u32, now: u64, pads: &PadLines) {
        self.errorsrc |= source;
        self.events_error = 1;
        self.net.task = None;
        self.net.suspend = false;
        if std::mem::take(&mut self.net.stop) {
            self.net.m = MState::Stopping;
            self.net.bus.m_stop(now, pads);
        } else {
            self.net.errored = true;
            self.net.m = MState::Hold;
        }
    }

    fn net_on_event(&mut self, event: BusEvent, now: u64, pads: &PadLines) {
        match event {
            BusEvent::MStarted => {
                self.net.m = MState::Address;
                let addr =
                    (((self.address & ADDRESS_MASK) as u8) << 1) | u8::from(self.net.reading);
                self.net.bus.m_send(addr, now, pads);
            }
            BusEvent::MSent { acked } if self.net.m == MState::Address => {
                if !acked {
                    self.net_nack(ERRORSRC_ANACK, now, pads);
                    return;
                }
                if self.net.reading {
                    if self.net.rx_max == 0 {
                        self.net_last(true);
                        self.net_after_byte(now, pads);
                    } else {
                        self.net_recv_next(now, pads);
                    }
                } else if self.net.tx.is_empty() {
                    self.net_last(false);
                    self.net_after_byte(now, pads);
                } else {
                    self.net_send_next(now, pads);
                }
            }
            BusEvent::MSent { acked } => {
                self.net.tx_sent += 1;
                self.txd_amount = self.net.tx_sent as u32;
                if acked {
                    self.net_after_byte(now, pads);
                } else {
                    self.net_nack(ERRORSRC_DNACK, now, pads);
                }
            }
            BusEvent::MByteIn(byte) => {
                self.net.rx.push(byte);
                let last = self.net.rx.len() >= self.net.rx_max;
                // The last byte before STOP or a change of direction is
                // NACKed; any other byte is ACKed.
                let nack = self.net.stop || (last && self.net.task.is_some());
                self.net.bus.m_ack(!nack, now, pads);
            }
            BusEvent::MAckDone => self.net_after_byte(now, pads),
            BusEvent::MStopped => {
                self.net.m = MState::Idle;
                self.net.errored = false;
                if self.net.reading {
                    self.net.rx_dirty = true;
                }
                self.net.deferred_stopped = true;
            }
            BusEvent::MArbLost => {
                self.net.m = MState::Idle;
                self.net.task = None;
                self.events_error = 1;
            }
            // A controller sees its own START/STOP on the wire; it never
            // answers as a target.
            BusEvent::TAddress(_) => self.net.bus.t_ack(false, now, pads),
            _ => {}
        }
    }

    fn net_process(&mut self, now: u64, pads: &PadLines) {
        loop {
            let events = self.net.bus.take_events();
            if events.is_empty() {
                break;
            }
            for event in events {
                self.net_on_event(event, now, pads);
            }
        }
        self.net.bus.settle(pads);
    }

    /// The net delivered a new level on SCL or SDA at this chip's pad.
    pub(super) fn net_edge(&mut self, line: usize, level: bool, now: u64) -> bool {
        if !self.net_mode() {
            return false;
        }
        let Some(pads) = self.net_pads() else {
            return false;
        };
        self.net.bus.timing = self.net_timing();
        pads.set_stamp(Some(now));
        self.net.bus.on_edge(line, level, now, &pads);
        self.net_process(now, &pads);
        pads.set_stamp(None);
        self.net.bus.deadline().is_some() || self.net_needs_bus() || self.net_irq()
    }

    fn net_step(&mut self, now: u64) {
        let Some(pads) = self.net_pads() else {
            return;
        };
        while let Some(due) = self.net.bus.deadline() {
            if due > now {
                break;
            }
            self.net.bus.step(due, &pads);
            pads.set_stamp(Some(due));
            self.net_process(due, &pads);
        }
        pads.set_stamp(None);
    }

    /// Walk path: one tick.
    pub(super) fn net_tick(&mut self) {
        let now = self.net_now();
        if self.net.bus.deadline().is_some_and(|d| d <= now) {
            self.net_step(now);
        }
    }

    fn net_next_due(&self, now: u64) -> Option<u64> {
        if self.net_needs_bus() || self.net.deferred_stopped || self.net.deferred_suspended {
            Some(now + 1)
        } else {
            self.net.bus.deadline()
        }
    }

    /// Scheduler path: events to arm (delays from now).
    pub(super) fn net_take_events(&mut self) -> Vec<(u64, u32)> {
        let now = self.net_now();
        match self.net_next_due(now) {
            Some(due) => {
                let due = due.max(now + 1);
                if self.net.armed.contains(&due) {
                    return Vec::new();
                }
                self.net.armed.push(due);
                vec![(due - now - 1, NET_TOKEN)]
            }
            None if self.net_irq() && !self.net.irq_armed => {
                self.net.irq_armed = true;
                vec![(0, NET_TOKEN)]
            }
            None => Vec::new(),
        }
    }

    /// Scheduler path: an armed event fired at `now`.
    pub(super) fn net_on_sched(
        &mut self,
        now: u64,
        bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        self.net.armed.retain(|&t| t > now);
        self.net.irq_armed = false;
        self.net_step(now);
        if self.net_needs_bus() || self.net.deferred_stopped || self.net.deferred_suspended {
            self.net_dma(bus);
        }
        let res = self.irq_from_events();
        let mut out = crate::sched::EventResult {
            raise_own_irq: res.irq,
            fired_events: res.fired_events,
            ..Default::default()
        };
        match self.net_next_due(now) {
            Some(due) => {
                let due = due.max(now + 1);
                if !self.net.armed.contains(&due) {
                    self.net.armed.push(due);
                    out.reschedule_delay = Some(due - now);
                }
            }
            None if out.raise_own_irq => {
                self.net.irq_armed = true;
                out.reschedule_delay = Some(1);
            }
            None => {}
        }
        out
    }

    /// `ENABLE` left TWIM: release the lines and forget the transfer.
    pub(super) fn net_disable(&mut self) {
        if let Some(pads) = self.net_pads() {
            self.net.bus.reset(&pads);
        }
        self.net.m = MState::Idle;
        self.net.task = None;
        self.net.stop = false;
        self.net.suspend = false;
        self.net.errored = false;
    }

    /// `true` while the walk/bus-tick path has engine or EasyDMA work.
    pub(super) fn net_has_work(&self) -> bool {
        self.net_needs_bus() || self.net.deferred_stopped || self.net.deferred_suspended
    }
}
