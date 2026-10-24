// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nRF52 TWIS (EasyDMA I²C target) on a world `gpio_net`: a bit-level
//! target between chips, on the shared wire engine
//! ([`crate::peripherals::i2c_bitbus`]).
//!
//! When a pad `PSEL.SCL` or `PSEL.SDA` names is on a net (and `ENABLE = 9`),
//! the target watches the wire (nRF52840 PS v1.11 §6.32):
//!
//! * It ACKs `ADDRESS[0]` / `ADDRESS[1]` where `CONFIG` enables them and
//!   records which in `MATCH`; any other address is left to the pull-up.
//! * A write fires `EVENTS_WRITE`; a read `EVENTS_READ`. The target needs a
//!   buffer for each: `TASKS_PREPARERX` (RX buffer) and `TASKS_PREPARETX`
//!   (EasyDMA reads `TXD.PTR`/`MAXCNT`). Until the matching task, and while
//!   `SHORTS.WRITE_SUSPEND` / `READ_SUSPEND` hold it until `TASKS_RESUME`, it
//!   stretches SCL. Then `EVENTS_RXSTARTED` / `TXSTARTED`.
//! * Received bytes are ACKed into the RX buffer up to `RXD.MAXCNT`; past it
//!   the byte is NACKed with `ERRORSRC.OVERFLOW | DNACK` and `EVENTS_ERROR`.
//!   A read sends the TX buffer, then `ORC` (`ERRORSRC.OVERREAD`,
//!   `EVENTS_ERROR`) until the controller NACKs.
//! * A repeated START or STOP ends the phase; each phase uses up its
//!   prepare. On STOP the RX bytes reach RAM at `RXD.PTR`, `RXD.AMOUNT` /
//!   `TXD.AMOUNT` count what crossed, and `EVENTS_STOPPED` fires.

use super::{
    Nrf52Twis, ADDRESS_MASK, ENABLE_MASK, ENABLE_TWIS, ERRORSRC_DNACK, ERRORSRC_OVERFLOW,
    ERRORSRC_OVERREAD, MAXCNT_MASK, OFF_TASKS_PREPARERX, OFF_TASKS_PREPARETX, OFF_TASKS_RESUME,
    OFF_TASKS_STOP, OFF_TASKS_SUSPEND, SHORT_READ_SUSPEND, SHORT_WRITE_SUSPEND,
};
use crate::peripherals::i2c_bitbus::{BitBus, BusEvent, Timing};
use crate::peripherals::pad_lines::PadLines;

/// What the stretched target waits for.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Wait {
    #[default]
    None,
    /// A write phase: `TASKS_PREPARERX` (or `RESUME` after a suspend).
    Rx,
    /// A read phase: `TASKS_PREPARETX` (or `RESUME`).
    Tx,
}

/// The net state, kept beside the register file.
#[derive(Debug, Default)]
pub(super) struct TwisNet {
    bus: BitBus,
    rx_prepared: bool,
    tx_prepared: bool,
    /// EasyDMA owes a read of the TX buffer (`TASKS_PREPARETX`).
    prefetch: bool,
    tx: Vec<u8>,
    tx_sent: usize,
    rx: Vec<u8>,
    /// RX bytes not yet in RAM.
    rx_dirty: bool,
    /// The target was addressed in this transfer.
    addressed: bool,
    /// The phase in progress (`Some(true)`: a read).
    phase: Option<bool>,
    wait: Wait,
    suspended: bool,
    /// `STOPPED` waits for the RX bytes to reach RAM.
    deferred_stopped: bool,
    armed: Vec<u64>,
    irq_armed: bool,
}

impl Nrf52Twis {
    fn net_pads(&self) -> Option<std::sync::Arc<PadLines>> {
        self.lines.clone()
    }

    /// `true` when the target is enabled on pads that sit on a world
    /// `gpio_net`.
    pub(super) fn net_mode(&self) -> bool {
        self.lines.is_some() && (self.claim_scl.on_net() || self.claim_sda.on_net())
    }

    fn net_now(&self) -> u64 {
        self.clock.as_ref().map_or(0, crate::CycleClock::now)
    }

    /// The target's data setup time after it ends a stretch with a new bit
    /// on SDA: 250 ns of the 64 MHz clock.
    fn net_timing(&self) -> Timing {
        Timing::new(64, 64, 16)
    }

    pub(super) fn net_needs_bus(&self) -> bool {
        self.net.prefetch || self.net.rx_dirty || self.net.deferred_stopped
    }

    /// A task in net mode.
    pub(super) fn net_task(&mut self, offset: u64) {
        let Some(pads) = self.net_pads() else {
            return;
        };
        let now = self.net_now();
        pads.set_stamp(Some(now));
        match offset {
            OFF_TASKS_PREPARERX => {
                self.net.rx_prepared = true;
                if self.net.wait == Wait::Rx && !self.net.suspended {
                    self.net_start_rx(now, &pads);
                }
            }
            OFF_TASKS_PREPARETX => {
                // EasyDMA reads the buffer first; a waiting read goes on then.
                self.net.prefetch = true;
            }
            OFF_TASKS_SUSPEND => self.net.suspended = true,
            OFF_TASKS_RESUME => {
                self.net.suspended = false;
                match self.net.wait {
                    Wait::Rx if self.net.rx_prepared => self.net_start_rx(now, &pads),
                    Wait::Tx if self.net.tx_prepared => self.net_start_tx(now, &pads),
                    _ => {}
                }
            }
            OFF_TASKS_STOP if !self.net.addressed => self.events_stopped = 1,
            _ => {}
        }
        self.net_process(now, &pads);
        pads.set_stamp(None);
    }

    fn net_start_rx(&mut self, now: u64, pads: &PadLines) {
        self.net.wait = Wait::None;
        self.net.rx.clear();
        self.events_rxstarted = 1;
        self.net.bus.t_continue(now, pads);
    }

    fn net_start_tx(&mut self, now: u64, pads: &PadLines) {
        self.net.wait = Wait::None;
        self.net.tx_sent = 0;
        self.txd_amount = 0;
        self.events_txstarted = 1;
        self.net_send_next(now, pads);
    }

    fn net_send_next(&mut self, now: u64, pads: &PadLines) {
        let byte = match self.net.tx.get(self.net.tx_sent) {
            Some(&b) => b,
            None => {
                self.errorsrc |= ERRORSRC_OVERREAD;
                self.events_error = 1;
                (self.orc & 0xFF) as u8
            }
        };
        self.net.tx_sent += 1;
        let max = (self.txd_maxcnt & MAXCNT_MASK) as usize;
        self.txd_amount = self.net.tx_sent.min(max) as u32;
        self.net.bus.t_send(byte, now, pads);
    }

    /// The phase in progress ended (repeated START or STOP): it used up its
    /// prepare.
    fn net_end_phase(&mut self) {
        match self.net.phase.take() {
            Some(false) => {
                self.net.rx_prepared = false;
                self.net.rx_dirty = true;
            }
            Some(true) => self.net.tx_prepared = false,
            None => {}
        }
        self.net.wait = Wait::None;
    }

    fn net_on_event(&mut self, event: BusEvent, now: u64, pads: &PadLines) {
        match event {
            BusEvent::TStart => self.net_end_phase(),
            BusEvent::TStop => {
                self.net_end_phase();
                if std::mem::take(&mut self.net.addressed) {
                    self.net.deferred_stopped = true;
                }
            }
            BusEvent::TAddress(byte) => {
                let addr = u32::from(byte >> 1);
                let matched = (0..2).find(|&i| {
                    self.config & (1 << i) != 0 && self.address[i] & ADDRESS_MASK == addr
                });
                if let Some(i) = matched {
                    self.match_reg = i as u32;
                }
                self.net.bus.t_ack(matched.is_some(), now, pads);
            }
            BusEvent::TAddressed { read } => {
                self.net.addressed = true;
                self.net.phase = Some(read);
                if read {
                    self.events_read = 1;
                    if self.shorts & SHORT_READ_SUSPEND != 0 {
                        self.net.suspended = true;
                    }
                    self.net.wait = Wait::Tx;
                    if self.net.tx_prepared && !self.net.suspended {
                        self.net_start_tx(now, pads);
                    }
                } else {
                    self.events_write = 1;
                    if self.shorts & SHORT_WRITE_SUSPEND != 0 {
                        self.net.suspended = true;
                    }
                    self.net.wait = Wait::Rx;
                    if self.net.rx_prepared && !self.net.suspended {
                        self.net_start_rx(now, pads);
                    }
                }
            }
            BusEvent::TByteIn(byte) => {
                let max = (self.rxd_maxcnt & MAXCNT_MASK) as usize;
                if self.net.rx.len() < max {
                    self.net.rx.push(byte);
                    self.net.bus.t_ack(true, now, pads);
                } else {
                    self.errorsrc |= ERRORSRC_OVERFLOW | ERRORSRC_DNACK;
                    self.events_error = 1;
                    self.net.bus.t_ack(false, now, pads);
                }
            }
            BusEvent::TByteOut { acked: true } => self.net_send_next(now, pads),
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

    /// The net delivered a new level on SCL or SDA at this target's pad.
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
        self.net.bus.deadline().is_some() || self.net_needs_bus() || self.irq_level()
    }

    /// EasyDMA: RX bytes to RAM (then `STOPPED`), and the TX buffer for a
    /// `TASKS_PREPARETX` (then a waiting read goes on).
    pub(super) fn net_dma(&mut self, bus: &mut dyn crate::Bus) {
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
        if std::mem::take(&mut self.net.prefetch) {
            let ptr = u64::from(self.txd_ptr);
            let n = (self.txd_maxcnt & MAXCNT_MASK) as usize;
            self.net.tx = crate::with_device_write_bypass(bus, |bus| {
                (0..n)
                    .map(|i| bus.read_u8(ptr + i as u64).unwrap_or(0))
                    .collect()
            });
            self.net.tx_prepared = true;
            if self.net.wait == Wait::Tx && !self.net.suspended {
                if let Some(pads) = self.net_pads() {
                    let now = self.net_now();
                    pads.set_stamp(Some(now));
                    self.net_start_tx(now, &pads);
                    self.net_process(now, &pads);
                    pads.set_stamp(None);
                }
            }
        }
    }

    /// After `ENABLE` or `PSEL`: a target that left its net lets go of the
    /// lines and forgets the transfer.
    pub(super) fn net_after_pins(&mut self) {
        if self.enable & ENABLE_MASK == ENABLE_TWIS && self.net_mode() {
            return;
        }
        if let Some(pads) = self.net_pads() {
            if self.net.addressed || self.net.wait != Wait::None {
                self.net.bus.reset(&pads);
                self.net.addressed = false;
                self.net.phase = None;
                self.net.wait = Wait::None;
            }
        }
    }

    /// Walk path: the stretch release a data setup time after an answer.
    pub(super) fn net_tick(&mut self) {
        let now = self.net_now();
        if self.net.bus.deadline().is_some_and(|d| d <= now) {
            if let Some(pads) = self.net_pads() {
                self.net.bus.step(now, &pads);
            }
        }
    }

    fn net_next_due(&self, now: u64) -> Option<u64> {
        if self.net_needs_bus() {
            Some(now + 1)
        } else {
            self.net.bus.deadline()
        }
    }

    /// Scheduler path: events to arm.
    pub(super) fn net_take_events(&mut self) -> Vec<(u64, u32)> {
        let now = self.net_now();
        match self.net_next_due(now) {
            Some(due) => {
                let due = due.max(now + 1);
                if self.net.armed.contains(&due) {
                    return Vec::new();
                }
                self.net.armed.push(due);
                vec![(due - now - 1, 0)]
            }
            None if self.irq_level() && !self.net.irq_armed => {
                self.net.irq_armed = true;
                vec![(0, 0)]
            }
            None => Vec::new(),
        }
    }

    pub(super) fn net_on_sched(
        &mut self,
        now: u64,
        bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        self.net.armed.retain(|&t| t > now);
        self.net.irq_armed = false;
        if let Some(pads) = self.net_pads() {
            if self.net.bus.deadline().is_some_and(|d| d <= now) {
                self.net.bus.step(now, &pads);
            }
        }
        if self.net_needs_bus() {
            self.net_dma(bus);
        }
        let irq = self.irq_level();
        let mut res = crate::sched::EventResult {
            raise_own_irq: irq,
            ..Default::default()
        };
        match self.net_next_due(now) {
            Some(due) => {
                let due = due.max(now + 1);
                if !self.net.armed.contains(&due) {
                    self.net.armed.push(due);
                    res.reschedule_delay = Some(due - now);
                }
            }
            None if irq => {
                self.net.irq_armed = true;
                res.reschedule_delay = Some(1);
            }
            None => {}
        }
        res
    }
}
