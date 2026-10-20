// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nRF52 SPIM (EasyDMA SPI master) on a world `gpio_net`: a bit-level
//! master between chips, on the shared wire engine
//! ([`crate::peripherals::spi_bitbus`]).
//!
//! Off a net `do_nrf52_easydma` exchanges the whole buffer with attached
//! device models inside one call. When a pad the SPIM's `PSEL.SCK`,
//! `PSEL.MOSI` or `PSEL.MISO` names is on a net (and `ENABLE = 7`), the wire
//! belongs to several chips and `TASKS_START` runs this engine instead:
//!
//! * EasyDMA reads `TXD.MAXCNT` bytes at `TXD.PTR` when the transfer starts
//!   (`EVENTS_STARTED`). The transfer is `max(TXD.MAXCNT, RXD.MAXCNT)` bytes;
//!   past the TX buffer `ORC` goes out.
//! * Each byte is clocked at the `FREQUENCY` rate (64 MHz core clock taken
//!   as the engine clock) in the `CONFIG` shape (CPOL, CPHA, bit order),
//!   back to back. MISO is sampled at the level the net delivers to its pad.
//! * At the end the received bytes land at `RXD.PTR` (up to `RXD.MAXCNT`),
//!   `TXD.AMOUNT` / `RXD.AMOUNT` are set and `EVENTS_ENDTX`, `EVENTS_ENDRX`
//!   and `EVENTS_END` fire, with the interrupt where `INTEN` asks for it.
//! * `TASKS_STOP` ends the transfer after the byte on the wire:
//!   `EVENTS_STOPPED`, with the amounts of what crossed.
//!
//! SCK and MOSI are driven push-pull while the SPIM is enabled; MISO is an
//! input. On SPIM0..2 chip select is the firmware's GPIO.
//!
//! SPIM3 (nRF52840 PS v1.x §6.25, registers in the SVD at 0x514/0x564-0x570)
//! also drives its own select and data/command lines when the wiring pass
//! gave it their claims ([`Spi::install_nrf_hw_select_claims`]):
//!
//! * `PSEL.CSN` connected: CSN is driven at its inactive level while the
//!   SPIM is enabled (`CSNPOL` = 0: active low, idle high; 1: active high).
//!   A transfer asserts it once the TX buffer is read, waits
//!   `IFTIMING.CSNDUR` cycles of the 64 MHz clock, then clocks the first
//!   byte (so the first SCK edge is `CSNDUR` + half an SCK period after the
//!   CSN edge). After the last SCK edge it waits `CSNDUR` again, releases
//!   CSN, and only then is the transfer over (`END`). The next assertion
//!   waits until CSN has been inactive for `CSNDUR` ("minimum duration CSN
//!   must stay high between transactions").
//! * `PSELDCX` connected: D/CX idles high while enabled; it is low for the
//!   first `DCXCNT` bytes of a transfer (all of them at 0xF) and high for the
//!   rest. It falls with CSN (with the first byte without a CSN), rises at
//!   the last SCK edge of the last command byte, and returns high when CSN
//!   is released (at the end of the transfer without one).
//!
//! `IFTIMING.RXDELAY` is stored, not modelled: MISO is sampled at the SCK
//! edge itself.

use super::{
    Spi, SpiRegs, SpiSignal, INTEN_END, INTEN_ENDRX, INTEN_ENDTX, INTEN_STOPPED, NRF_SPIM_LINE_DCX,
};
use crate::peripherals::pad_lines::{LineDrive, PadLines};
use crate::peripherals::spi_bitbus::{SpiBitBus, SpiEvent, SpiLines, SpiShape};

/// Scheduler token for the nRF net engine.
pub(super) const NRF_NET_TOKEN: u32 = 3;

/// nRF52 SPIM `INTEN.STARTED` (bit 19, PS v1.11 §6.25.6.9).
pub(super) const INTEN_STARTED: u32 = 1 << 19;

const LINES: SpiLines = SpiLines {
    sck: SpiSignal::Sck as usize,
    mosi: SpiSignal::Mosi as usize,
    miso: SpiSignal::Miso as usize,
};
/// SPIM3's hardware chip select line.
const LINE_CSN: usize = SpiSignal::Nss as usize;
/// SPIM3's data/command line.
const LINE_DCX: usize = NRF_SPIM_LINE_DCX;
/// `DCXCNT` value meaning "every byte is a command byte".
const DCXCNT_ALL: u32 = 0xF;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    /// `TASKS_START` was written: EasyDMA reads the TX buffer next.
    Starting,
    /// SPIM3: waiting for CSN to have been inactive for `CSNDUR` before
    /// asserting it again.
    Selecting,
    /// SPIM3: CSN asserted, `CSNDUR` to wait before the first byte.
    Setup,
    /// Bytes are on the wire.
    Running,
    /// SPIM3: the last byte is out, `CSNDUR` to wait before CSN is released.
    Holding,
    /// The last byte is in: EasyDMA writes the RX buffer next.
    Finishing,
}

/// The net engine's state, kept beside the register file.
#[derive(Debug, Default)]
pub(super) struct SpimNet {
    bus: SpiBitBus,
    phase: Phase,
    tx: Vec<u8>,
    rx: Vec<u8>,
    /// `TASKS_STOP` came while a byte was on the wire.
    stop: bool,
    armed: Vec<u64>,
    irq_armed: bool,
    /// SPIM3: the end of the current CSN timing wait (`Selecting`, `Setup`,
    /// `Holding`).
    wait: Option<u64>,
    /// SPIM3: CSN may be asserted again from this cycle (`CSNDUR` after its
    /// release).
    csn_free_at: u64,
    /// SPIM3: `DCXCNT` latched at the start of the transfer.
    dcx_cmd: u32,
}

impl Spi {
    fn nrf_net_pads(&self) -> Option<std::sync::Arc<PadLines>> {
        self.lines.as_ref().map(|l| l.pad_lines().clone())
    }

    /// `true` when this nRF52 SPIM is enabled on pads that sit on a world
    /// `gpio_net`.
    pub(super) fn nrf_net_mode(&self) -> bool {
        matches!(&self.regs, SpiRegs::Nrf52(r) if r.map == super::NrfSpimMap::Nrf52)
            && (self.nrf52_claim_sck.on_net()
                || self.nrf52_claim_mosi.on_net()
                || self.nrf52_claim_miso.on_net()
                || self.nrf52_claim_csn.on_net()
                || self.nrf52_claim_dcx.on_net())
    }

    /// SPIM3 with `PSEL.CSN` connected and the SPIM enabled: `Some(active
    /// level)` (`CSNPOL`), else `None`.
    fn nrf_net_csn(&self) -> Option<bool> {
        match &self.regs {
            SpiRegs::Nrf52(r)
                if self.nrf52_hw_select
                    && r.enable & 0xF == super::NRF52_ENABLE_SPIM
                    && r.psel_csn & 0x8000_0000 == 0 =>
            {
                Some(r.csnpol & 1 != 0)
            }
            _ => None,
        }
    }

    /// SPIM3 with `PSELDCX` connected and the SPIM enabled.
    fn nrf_net_dcx(&self) -> bool {
        matches!(&self.regs, SpiRegs::Nrf52(r)
            if self.nrf52_hw_select
                && r.enable & 0xF == super::NRF52_ENABLE_SPIM
                && r.psel_dcx & 0x8000_0000 == 0)
    }

    /// `IFTIMING.CSNDUR` in engine cycles (the 64 MHz clock).
    fn nrf_net_csndur(&self) -> u64 {
        match &self.regs {
            SpiRegs::Nrf52(r) => u64::from(r.csndur & 0xFF),
            _ => 0,
        }
    }

    /// The next timed step: a bit-engine edge or the end of a CSN wait.
    fn nrf_net_deadline(&self) -> Option<u64> {
        match (self.nrf_net.bus.deadline(), self.nrf_net.wait) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// D/CX for byte `index` of the transfer: low while it is a command byte.
    fn nrf_net_drive_dcx(&self, index: usize, pads: &PadLines) {
        if !self.nrf_net_dcx() {
            return;
        }
        let cmd = self.nrf_net.dcx_cmd;
        let command = cmd == DCXCNT_ALL || (index as u32) < cmd;
        pads.drive_line(LINE_DCX, LineDrive::PushPull, !command);
    }

    /// Assert CSN (SPIM3) and set D/CX for the first byte at `now`, then
    /// clock the first byte after `CSNDUR`, or now without a CSN.
    fn nrf_net_select(&mut self, now: u64, pads: &PadLines) {
        pads.set_stamp(Some(now));
        let csn = self.nrf_net_csn();
        if let Some(active) = csn {
            pads.drive_line(LINE_CSN, LineDrive::PushPull, active);
        }
        self.nrf_net_drive_dcx(0, pads);
        let dur = if csn.is_some() {
            self.nrf_net_csndur()
        } else {
            0
        };
        if dur == 0 {
            self.nrf_net_first_byte(now, pads);
        } else {
            self.nrf_net.phase = Phase::Setup;
            self.nrf_net.wait = Some(now + dur);
        }
    }

    fn nrf_net_first_byte(&mut self, now: u64, pads: &PadLines) {
        if self.nrf_net.stop {
            self.nrf_net_bytes_done(now, pads);
            return;
        }
        self.nrf_net.phase = Phase::Running;
        pads.set_stamp(Some(now));
        let first = u16::from(self.nrf_net.tx[0]);
        self.nrf_net.bus.m_begin(first, now, LINES, pads);
    }

    /// The last byte is out (or the transfer was stopped): hold CSN for
    /// `CSNDUR`, or end now.
    fn nrf_net_bytes_done(&mut self, now: u64, pads: &PadLines) {
        let dur = if self.nrf_net_csn().is_some() {
            self.nrf_net_csndur()
        } else {
            0
        };
        if dur == 0 {
            self.nrf_net_release(now, pads);
        } else {
            self.nrf_net.phase = Phase::Holding;
            self.nrf_net.wait = Some(now + dur);
        }
    }

    /// Release CSN and return D/CX to its idle high; EasyDMA writes the RX
    /// buffer next.
    fn nrf_net_release(&mut self, now: u64, pads: &PadLines) {
        pads.set_stamp(Some(now));
        if let Some(active) = self.nrf_net_csn() {
            pads.drive_line(LINE_CSN, LineDrive::PushPull, !active);
            self.nrf_net.csn_free_at = now + self.nrf_net_csndur();
        }
        if self.nrf_net_dcx() {
            pads.drive_line(LINE_DCX, LineDrive::PushPull, true);
        }
        self.nrf_net.phase = Phase::Finishing;
    }

    /// A CSN wait ended at `now`.
    fn nrf_net_wait_done(&mut self, now: u64, pads: &PadLines) {
        match self.nrf_net.phase {
            Phase::Selecting => self.nrf_net_select(now, pads),
            Phase::Setup => self.nrf_net_first_byte(now, pads),
            Phase::Holding => self.nrf_net_release(now, pads),
            _ => {}
        }
    }

    fn nrf_net_now(&self) -> u64 {
        self.h5_clock.as_ref().map_or(0, crate::CycleClock::now)
    }

    fn nrf_net_shape(&self) -> (SpiShape, u64) {
        let config = match &self.regs {
            SpiRegs::Nrf52(r) => r.config,
            _ => 0,
        };
        let shape = SpiShape {
            cpol: config & super::NRF52_CONFIG_CPOL != 0,
            cpha: config & super::NRF52_CONFIG_CPHA != 0,
            lsb_first: config & super::NRF52_CONFIG_ORDER_LSB != 0,
            bits: 8,
        };
        (shape, (self.nrf52_sck_bit_cycles() / 2).max(1))
    }

    /// Re-state the line drives after a register write in net mode: SCK at
    /// CPOL and MOSI driven while enabled, MISO an input.
    pub(super) fn nrf_net_apply_drive(&mut self) {
        let Some(pads) = self.nrf_net_pads() else {
            return;
        };
        if !self.nrf_net_mode() {
            return;
        }
        let (shape, half) = self.nrf_net_shape();
        if !self.nrf_net.bus.master_busy() {
            self.nrf_net.bus.shape = shape;
            self.nrf_net.bus.half = half;
        }
        pads.set_stamp(Some(self.nrf_net_now()));
        self.nrf_net.bus.m_idle_drive(LINES, &pads);
        // SPIM3's select and D/CX: between transfers at their idle levels
        // (CSN inactive, D/CX high), driven only while their PSEL is live.
        if matches!(self.nrf_net.phase, Phase::Idle | Phase::Starting) {
            match self.nrf_net_csn() {
                Some(active) => pads.drive_line(LINE_CSN, LineDrive::PushPull, !active),
                None => pads.set_mode(LINE_CSN, LineDrive::Input),
            }
            if self.nrf_net_dcx() {
                pads.drive_line(LINE_DCX, LineDrive::PushPull, true);
            } else {
                pads.set_mode(LINE_DCX, LineDrive::Input);
            }
        }
        pads.set_stamp(None);
    }

    /// `TASKS_START` in net mode.
    pub(super) fn nrf_net_start(&mut self) {
        if self.nrf_net.phase == Phase::Idle {
            self.nrf_net.phase = Phase::Starting;
            self.nrf_net.stop = false;
        }
    }

    /// `TASKS_STOP` in net mode: after the byte on the wire, or now.
    pub(super) fn nrf_net_stop(&mut self) {
        match self.nrf_net.phase {
            Phase::Idle => {
                if let SpiRegs::Nrf52(r) = &mut self.regs {
                    r.events_stopped = 1;
                }
            }
            Phase::Starting | Phase::Selecting => {
                self.nrf_net.phase = Phase::Idle;
                self.nrf_net.wait = None;
                if let SpiRegs::Nrf52(r) = &mut self.regs {
                    r.events_stopped = 1;
                }
            }
            Phase::Setup | Phase::Running | Phase::Holding | Phase::Finishing => {
                self.nrf_net.stop = true
            }
        }
    }

    /// `true` while EasyDMA has RAM to move.
    pub(super) fn nrf_net_needs_bus(&self) -> bool {
        matches!(self.nrf_net.phase, Phase::Starting | Phase::Finishing)
    }

    /// `true` while a transfer is anywhere between `TASKS_START` and its end.
    pub(super) fn nrf_net_busy(&self) -> bool {
        self.nrf_net.phase != Phase::Idle || self.nrf_net.bus.master_busy()
    }

    /// The interrupt level: an event whose `INTEN` bit is set.
    pub(super) fn nrf_net_irq(&self) -> bool {
        match &self.regs {
            SpiRegs::Nrf52(r) => {
                (r.events_stopped != 0 && r.inten & INTEN_STOPPED != 0)
                    || (r.events_endrx != 0 && r.inten & INTEN_ENDRX != 0)
                    || (r.events_end != 0 && r.inten & INTEN_END != 0)
                    || (r.events_endtx != 0 && r.inten & INTEN_ENDTX != 0)
                    || (r.events_started != 0 && r.inten & INTEN_STARTED != 0)
            }
            _ => false,
        }
    }

    /// EasyDMA: read the TX buffer at the start, write the RX buffer at the
    /// end. Called from `tick_with_bus` and from the scheduler event.
    pub(super) fn nrf_net_dma(&mut self, bus: &mut dyn crate::Bus) {
        let Some(pads) = self.nrf_net_pads() else {
            return;
        };
        let SpiRegs::Nrf52(r) = &self.regs else {
            return;
        };
        let (txd_ptr, txd_max, rxd_ptr, rxd_max, orc) = (
            u64::from(r.txd_ptr),
            r.txd_maxcnt as usize & 0xFFFF,
            u64::from(r.rxd_ptr),
            r.rxd_maxcnt as usize & 0xFFFF,
            (r.orc & 0xFF) as u8,
        );
        let now = self.nrf_net_now();
        match self.nrf_net.phase {
            Phase::Starting => {
                let n = txd_max.max(rxd_max);
                self.nrf_net.tx = crate::with_device_write_bypass(bus, |bus| {
                    (0..n)
                        .map(|i| {
                            if i < txd_max {
                                bus.read_u8(txd_ptr + i as u64).unwrap_or(0)
                            } else {
                                orc
                            }
                        })
                        .collect()
                });
                self.nrf_net.rx.clear();
                if let SpiRegs::Nrf52(r) = &mut self.regs {
                    r.events_started = 1;
                    self.nrf_net.dcx_cmd = r.dcxcnt & 0xF;
                }
                let (shape, half) = self.nrf_net_shape();
                self.nrf_net.bus.shape = shape;
                self.nrf_net.bus.half = half;
                if n == 0 {
                    self.nrf_net.phase = Phase::Finishing;
                    self.nrf_net_dma(bus);
                    return;
                }
                // SPIM3: CSN may have to stay inactive a while longer.
                let at = if self.nrf_net_csn().is_some() {
                    now.max(self.nrf_net.csn_free_at)
                } else {
                    now
                };
                if at > now {
                    self.nrf_net.phase = Phase::Selecting;
                    self.nrf_net.wait = Some(at);
                } else {
                    self.nrf_net_select(now, &pads);
                }
                pads.set_stamp(None);
            }
            Phase::Finishing => {
                let done = self.nrf_net.rx.len();
                crate::with_device_write_bypass(bus, |bus| {
                    for (i, &b) in self.nrf_net.rx.iter().take(rxd_max).enumerate() {
                        let _ = bus.write_u8(rxd_ptr + i as u64, b);
                    }
                });
                let stopped = std::mem::take(&mut self.nrf_net.stop);
                let complete = done >= self.nrf_net.tx.len();
                if let SpiRegs::Nrf52(r) = &mut self.regs {
                    r.txd_amount = done.min(txd_max) as u32;
                    r.rxd_amount = done.min(rxd_max) as u32;
                    if done >= txd_max {
                        r.events_endtx = 1;
                    }
                    if done >= rxd_max {
                        r.events_endrx = 1;
                    }
                    if complete {
                        r.events_end = 1;
                    }
                    if stopped {
                        r.events_stopped = 1;
                    }
                }
                self.nrf_net.phase = Phase::Idle;
            }
            _ => {}
        }
    }

    fn nrf_net_step(&mut self, now: u64) {
        let Some(pads) = self.nrf_net_pads() else {
            return;
        };
        while let Some(due) = self.nrf_net_deadline() {
            if due > now {
                break;
            }
            if self.nrf_net.wait == Some(due) {
                self.nrf_net.wait = None;
                self.nrf_net_wait_done(due, &pads);
                continue;
            }
            self.nrf_net.bus.step(due, LINES, &pads);
            for event in self.nrf_net.bus.take_events() {
                let SpiEvent::MDone(word) = event else {
                    continue;
                };
                self.nrf_net.rx.push(word as u8);
                let next = self.nrf_net.rx.len();
                if next < self.nrf_net.tx.len() && !self.nrf_net.stop {
                    self.nrf_net_drive_dcx(next, &pads);
                    let byte = u16::from(self.nrf_net.tx[next]);
                    self.nrf_net.bus.m_begin(byte, due, LINES, &pads);
                } else {
                    self.nrf_net_bytes_done(due, &pads);
                }
            }
        }
        pads.set_stamp(None);
    }

    /// Walk path: one tick.
    pub(super) fn nrf_net_tick(&mut self) {
        let now = self.nrf_net_now();
        if self.nrf_net_deadline().is_some_and(|d| d <= now) {
            self.nrf_net_step(now);
        }
    }

    /// Scheduler path: events to arm (delays from now).
    pub(super) fn nrf_net_take_events(&mut self) -> Vec<(u64, u32)> {
        let now = self.nrf_net_now();
        let due = if self.nrf_net_needs_bus() {
            Some(now + 1)
        } else {
            self.nrf_net_deadline()
        };
        let mut events = Vec::new();
        if let Some(due) = due {
            let due = due.max(now + 1);
            if !self.nrf_net.armed.contains(&due) {
                self.nrf_net.armed.push(due);
                events.push((due - now - 1, NRF_NET_TOKEN));
            }
        } else if self.nrf_net_irq() && !self.nrf_net.irq_armed {
            self.nrf_net.irq_armed = true;
            events.push((0, NRF_NET_TOKEN));
        }
        events
    }

    /// Scheduler path: an armed event fired at `now`.
    pub(super) fn nrf_net_on_event(
        &mut self,
        now: u64,
        bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        self.nrf_net.armed.retain(|&t| t > now);
        self.nrf_net.irq_armed = false;
        self.nrf_net_step(now);
        if self.nrf_net_needs_bus() {
            self.nrf_net_dma(bus);
        }
        let irq = self.nrf_net_irq();
        let mut res = crate::sched::EventResult {
            raise_own_irq: irq,
            ..Default::default()
        };
        let next = if self.nrf_net_needs_bus() {
            Some(now + 1)
        } else {
            self.nrf_net_deadline()
        };
        match next {
            Some(due) => {
                let due = due.max(now + 1);
                if !self.nrf_net.armed.contains(&due) {
                    self.nrf_net.armed.push(due);
                    res.reschedule_delay = Some(due - now);
                }
            }
            None if irq => {
                // A level: re-raised while the event and its enable hold.
                self.nrf_net.irq_armed = true;
                res.reschedule_delay = Some(1);
            }
            None => {}
        }
        res
    }

    /// The engine forgets a transfer when the SPIM is disabled.
    pub(super) fn nrf_net_disable(&mut self) {
        self.nrf_net.bus.reset();
        self.nrf_net.phase = Phase::Idle;
        self.nrf_net.stop = false;
        self.nrf_net.wait = None;
        if let Some(pads) = self.nrf_net_pads() {
            let lines = pads.names().len();
            for line in [LINES.sck, LINES.mosi, LINES.miso, LINE_CSN, LINE_DCX] {
                if line >= lines {
                    continue;
                }
                pads.set_mode(line, LineDrive::Input);
            }
        }
    }
}
