// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! megaAVR 0-series USART (ATmega4808/4809 data sheet DS40002173 §23;
//! register offsets and reset values from the Microchip ATmega4809 ATDF).
//!
//! Registers: RXDATAL 0x00, RXDATAH 0x01, TXDATAL 0x02, TXDATAH 0x03,
//! STATUS 0x04, CTRLA 0x05, CTRLB 0x06, CTRLC 0x07, BAUD 0x08/0x09, CTRLD
//! 0x0A, DBGCTRL 0x0B, EVCTRL 0x0C, TXPLCTRL 0x0D, RXPLCTRL 0x0E.
//!
//! Modelled: asynchronous 8-bit frames. A byte written to TXDATAL while
//! CTRLB.TXEN is set leaves at once (no baud pacing), so STATUS.DREIF always
//! reads 1 and TXCIF is set by every byte; TXCIF is write-one-to-clear. RX is
//! the shared host queue, gated by CTRLB.RXEN: RXCIF (STATUS bit 7 and
//! RXDATAH bit 7) reads 1 while a byte waits, and reading RXDATAL pops it.
//! CTRLA.LBME loops every transmitted byte back into the receiver.
//!
//! Interrupts (`irqs: [rxc, dre, txc]`, the part's vector numbers): RXC while
//! RXCIE is set and a byte waits; TXC when TXCIE is set and TXCIF is; DRE
//! when DREIE is set (DREIF is always 1). DRE is raised on each event that
//! (re)asserts it — DREIE going 1, and every TXDATAL write while DREIE is set
//! — which is every path ArduinoCore-megaavr's HardwareSerial takes; a DRE
//! handler that returns with DREIE still set and without writing TXDATAL is
//! not re-entered here, though silicon would.
//!
//! Refused rather than ignored: RS-485 mode, synchronous / IrDA / master-SPI
//! CMODE, and the auto-baud RXMODEs. Not modelled: parity / frame errors,
//! 9-bit frames, start-of-frame detection, multiprocessor mode and the event
//! input.

use crate::bus::bus_trace::{BusDir, BusPayload, BusTrace};
use crate::peripherals::device::{UartStreamDevice, UartStreamHost};
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

const CONSOLE: usize = 64 * 1024;
const EVENT_TOKEN: u32 = 1;
const RX_TOKEN: u32 = 2;

const RXDATAL: u64 = 0x00;
const RXDATAH: u64 = 0x01;
const TXDATAL: u64 = 0x02;
const TXDATAH: u64 = 0x03;
const STATUS: u64 = 0x04;
const CTRLA: u64 = 0x05;
const CTRLB: u64 = 0x06;
const CTRLC: u64 = 0x07;
const BAUDL: u64 = 0x08;
const BAUDH: u64 = 0x09;
const CTRLD: u64 = 0x0A;
const DBGCTRL: u64 = 0x0B;
const EVCTRL: u64 = 0x0C;
const TXPLCTRL: u64 = 0x0D;
const RXPLCTRL: u64 = 0x0E;
const WINDOW: u64 = 0x0F;

const STATUS_RXCIF: u8 = 1 << 7;
const STATUS_TXCIF: u8 = 1 << 6;
const STATUS_DREIF: u8 = 1 << 5;
const CTRLA_RXCIE: u8 = 1 << 7;
const CTRLA_TXCIE: u8 = 1 << 6;
const CTRLA_DREIE: u8 = 1 << 5;
const CTRLA_LBME: u8 = 1 << 3;
const CTRLA_RS485: u8 = 0x03;
const CTRLB_RXEN: u8 = 1 << 7;
const CTRLB_TXEN: u8 = 1 << 6;
const CTRLB_RXMODE: u8 = 0x06;
const CTRLB_RXMODE_CLK2X: u8 = 0x02;
const CTRLC_CMODE: u8 = 0xC0;

#[derive(Default)]
struct State {
    ctrla: u8,
    ctrlb: u8,
    ctrlc: u8,
    baud: u16,
    ctrld: u8,
    dbgctrl: u8,
    evctrl: u8,
    txplctrl: u8,
    rxplctrl: u8,
    txcif: bool,
    /// A DRE / TXC assertion waits to be raised.
    dre_event: bool,
    txc_event: bool,
    /// Scheduler wakes queued or live.
    event_live: bool,
    rx_live: bool,
    pending_events: Vec<(u64, u32)>,
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo: bool,
    streams: Vec<Box<dyn UartStreamDevice>>,
}

pub struct MegaAvrUsart {
    state: Mutex<State>,
    rx: Arc<Mutex<VecDeque<u8>>>,
    /// Vector numbers: receive complete, data register empty, transmit
    /// complete.
    irqs: [u32; 3],
    trace: BusTrace,
    trace_name: String,
    clock: Option<CycleClock>,
}

impl std::fmt::Debug for MegaAvrUsart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MegaAvrUsart")
            .field("irqs", &self.irqs)
            .finish_non_exhaustive()
    }
}

impl MegaAvrUsart {
    pub fn new(irqs: [u32; 3]) -> Self {
        Self {
            state: Mutex::new(State {
                ctrlc: 0x03,
                ..State::default()
            }),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            irqs,
            trace: crate::bus::bus_trace::new_log(),
            trace_name: String::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn rx_enabled(s: &State) -> bool {
        s.ctrlb & CTRLB_RXEN != 0
    }

    fn rx_waiting(&self, s: &State) -> bool {
        Self::rx_enabled(s) && !self.rx.lock().unwrap().is_empty()
    }

    fn rx_due(&self, s: &State) -> bool {
        s.ctrla & CTRLA_RXCIE != 0 && self.rx_waiting(s)
    }

    /// CPU cycles per character (start + 8 data + stop): a bit is
    /// S * BAUD / 64 peripheral clocks, S = 16 (8 with CLK2X). CLK_PER is
    /// taken as the CPU clock.
    fn char_cycles(s: &State) -> u64 {
        let samples: u64 = if s.ctrlb & CTRLB_RXMODE == CTRLB_RXMODE_CLK2X {
            8
        } else {
            16
        };
        (10 * samples * u64::from(s.baud.max(64)) / 64).max(1)
    }

    fn status(&self, s: &State) -> u8 {
        let mut v = STATUS_DREIF;
        if s.txcif {
            v |= STATUS_TXCIF;
        }
        if self.rx_waiting(s) {
            v |= STATUS_RXCIF;
        }
        v
    }

    fn want_wake(&self, s: &mut State) {
        if !self.scheduler_mode() {
            return;
        }
        if (s.dre_event || s.txc_event) && !s.event_live {
            s.event_live = true;
            s.pending_events.push((1, EVENT_TOKEN));
        }
        if Self::rx_enabled(s) && s.ctrla & CTRLA_RXCIE != 0 && !s.rx_live {
            s.rx_live = true;
            let delay = if self.rx_due(s) {
                1
            } else {
                Self::char_cycles(s)
            };
            s.pending_events.push((delay, RX_TOKEN));
        }
    }

    /// The vectors due now.
    fn service(&self, s: &mut State) -> Vec<u32> {
        let mut irqs = Vec::new();
        if self.rx_due(s) {
            irqs.push(self.irqs[0]);
        }
        if std::mem::take(&mut s.dre_event) && s.ctrla & CTRLA_DREIE != 0 {
            irqs.push(self.irqs[1]);
        }
        if std::mem::take(&mut s.txc_event) && s.ctrla & CTRLA_TXCIE != 0 && s.txcif {
            irqs.push(self.irqs[2]);
        }
        irqs
    }

    fn transmit(&self, s: &mut State, v: u8) {
        self.trace.push(
            &self.trace_name,
            BusPayload::Uart {
                direction: BusDir::Tx,
                byte: v,
            },
        );
        if s.ctrla & CTRLA_LBME != 0 {
            if Self::rx_enabled(s) {
                self.rx.lock().unwrap().push_back(v);
            }
        } else {
            for peer in &mut s.streams {
                peer.on_tx_byte(v);
            }
            if let Some(sink) = &s.sink {
                let mut b = sink.lock().unwrap();
                if b.len() >= CONSOLE {
                    let remove = b.len() - CONSOLE + 1;
                    b.drain(..remove);
                }
                b.push(v);
            }
            if s.echo {
                use std::io::Write;
                let _ = std::io::stdout().write_all(&[v]);
            }
        }
        s.txcif = true;
        s.txc_event = true;
        if s.ctrla & CTRLA_DREIE != 0 {
            s.dre_event = true;
        }
    }

    fn reg(&self, s: &State, o: u64, pop: bool) -> u8 {
        match o {
            RXDATAL => {
                if !Self::rx_enabled(s) {
                    0
                } else if pop {
                    self.rx.lock().unwrap().pop_front().unwrap_or(0)
                } else {
                    self.rx.lock().unwrap().front().copied().unwrap_or(0)
                }
            }
            RXDATAH if self.rx_waiting(s) => STATUS_RXCIF,
            STATUS => self.status(s),
            CTRLA => s.ctrla,
            CTRLB => s.ctrlb,
            CTRLC => s.ctrlc,
            BAUDL => s.baud as u8,
            BAUDH => (s.baud >> 8) as u8,
            CTRLD => s.ctrld,
            DBGCTRL => s.dbgctrl,
            EVCTRL => s.evctrl,
            TXPLCTRL => s.txplctrl,
            RXPLCTRL => s.rxplctrl,
            _ => 0,
        }
    }

    fn refuse(what: String) -> SimResult<()> {
        Err(SimulationError::NotImplemented(format!(
            "megaAVR USART: {what} is not modelled"
        )))
    }

    fn write_reg(&mut self, o: u64, v: u8) -> SimResult<()> {
        let mut s = self.state.lock().unwrap();
        match o {
            TXDATAL if s.ctrlb & CTRLB_TXEN != 0 => self.transmit(&mut s, v),
            // TXEN clear: the transmitter is off and the byte goes nowhere.
            // The ninth data bit: 9-bit frames are not modelled, and an
            // 8-bit frame ignores it.
            TXDATAH => {}
            STATUS if v & STATUS_TXCIF != 0 => s.txcif = false,
            CTRLA => {
                if v & CTRLA_RS485 != 0 {
                    drop(s);
                    return Self::refuse(format!("CTRLA.RS485 ({v:#04x})"));
                }
                let rising = v & !s.ctrla;
                s.ctrla = v;
                if rising & CTRLA_DREIE != 0 {
                    s.dre_event = true;
                }
                if rising & CTRLA_TXCIE != 0 && s.txcif {
                    s.txc_event = true;
                }
            }
            CTRLB => {
                if v & CTRLB_RXMODE > CTRLB_RXMODE_CLK2X {
                    drop(s);
                    return Self::refuse(format!("auto-baud CTRLB.RXMODE ({v:#04x})"));
                }
                s.ctrlb = v & 0xDF;
            }
            CTRLC => {
                if v & CTRLC_CMODE != 0 {
                    drop(s);
                    return Self::refuse(format!("CTRLC.CMODE other than asynchronous ({v:#04x})"));
                }
                s.ctrlc = v;
            }
            BAUDL => s.baud = (s.baud & 0xFF00) | u16::from(v),
            BAUDH => s.baud = (s.baud & 0x00FF) | (u16::from(v) << 8),
            CTRLD => s.ctrld = v & 0xC0,
            DBGCTRL => s.dbgctrl = v & 0x01,
            EVCTRL => s.evctrl = v & 0x01,
            TXPLCTRL => s.txplctrl = v,
            RXPLCTRL => s.rxplctrl = v & 0x7F,
            _ => {}
        }
        self.want_wake(&mut s);
        Ok(())
    }
}

impl UartStreamHost for MegaAvrUsart {
    fn attach_stream_device(&mut self, dev: Box<dyn UartStreamDevice>) {
        assert!(
            dev.stream_generation().is_none(),
            "megaAVR USART has no exclusive protocol attachment"
        );
        self.state.lock().unwrap().streams.push(dev);
    }
    fn detach_console_sink(&mut self) {
        let mut s = self.state.lock().unwrap();
        s.sink = None;
        s.echo = false;
    }
    fn attach_console_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo: bool) -> bool {
        let mut s = self.state.lock().unwrap();
        s.sink = sink;
        s.echo = echo;
        true
    }
    fn hosts_protocol_peer(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .streams
            .iter()
            .any(|p| p.carries_protocol_octets())
    }
    fn peer_ids(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .streams
            .iter()
            .filter_map(|p| p.device_id().map(str::to_owned))
            .collect()
    }
    fn inject_peer_remote(&mut self, id: &str, bytes: &[u8]) -> Result<(), String> {
        crate::peripherals::device::inject_remote_into(
            &mut self.state.lock().unwrap().streams,
            id,
            bytes,
        )
    }
}

impl Peripheral for MegaAvrUsart {
    fn attach_bus_trace(&mut self, name: &str, trace: &BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.into();
    }
    fn bus_trace_handle(&self) -> Option<BusTrace> {
        Some(self.trace.clone())
    }
    fn peek(&self, o: u64) -> Option<u8> {
        (o < WINDOW).then(|| self.reg(&self.state.lock().unwrap(), o, false))
    }
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        self.state
            .lock()
            .unwrap()
            .streams
            .iter()
            .flat_map(|p| p.logs())
            .collect()
    }
    fn inspect(
        &self,
        base: u64,
        name: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::PeripheralInspect {
        let mut pi = crate::inspect::default_inspect(self, base, name, opts);
        pi.kind = "uart".into();
        pi
    }
    fn read(&self, o: u64) -> SimResult<u8> {
        Ok(self.reg(&self.state.lock().unwrap(), o, true))
    }
    fn write(&mut self, o: u64, v: u8) -> SimResult<()> {
        self.write_reg(o, v)
    }
    fn legacy_tick_active(&self) -> bool {
        if self.scheduler_mode() {
            return false;
        }
        let s = self.state.lock().unwrap();
        s.dre_event || s.txc_event || (Self::rx_enabled(&s) && s.ctrla & CTRLA_RXCIE != 0)
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
        let mut s = self.state.lock().unwrap();
        let irqs = self.service(&mut s);
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }
    /// Also called after host bytes land in the RX queue.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        let mut s = self.state.lock().unwrap();
        self.want_wake(&mut s);
        std::mem::take(&mut s.pending_events)
    }
    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let mut s = self.state.lock().unwrap();
        if event_token == EVENT_TOKEN {
            s.event_live = false;
            let mut irqs = Vec::new();
            if std::mem::take(&mut s.dre_event) && s.ctrla & CTRLA_DREIE != 0 {
                irqs.push(self.irqs[1]);
            }
            if std::mem::take(&mut s.txc_event) && s.ctrla & CTRLA_TXCIE != 0 && s.txcif {
                irqs.push(self.irqs[2]);
            }
            return crate::sched::EventResult {
                explicit_irqs: irqs,
                ..Default::default()
            };
        }
        let irqs = if self.rx_due(&s) {
            vec![self.irqs[0]]
        } else {
            Vec::new()
        };
        // The receiver stays polled, one character apart, while armed.
        let again = Self::rx_enabled(&s) && s.ctrla & CTRLA_RXCIE != 0;
        s.rx_live = again;
        crate::sched::EventResult {
            explicit_irqs: irqs,
            reschedule_delay: again.then(|| Self::char_cycles(&s)),
            ..Default::default()
        }
    }
    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }
    fn on_node_reset(&mut self) {
        let mut s = self.state.lock().unwrap();
        let streams = std::mem::take(&mut s.streams);
        let (sink, echo) = (s.sink.take(), s.echo);
        *s = State {
            ctrlc: 0x03,
            streams,
            sink,
            echo,
            ..State::default()
        };
        self.rx.lock().unwrap().clear();
    }
    fn as_uart_stream_host(&mut self) -> Option<&mut dyn UartStreamHost> {
        Some(self)
    }
    fn uart_rx_source(&self) -> Option<Arc<Mutex<VecDeque<u8>>>> {
        Some(self.rx.clone())
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console() -> (MegaAvrUsart, Arc<Mutex<Vec<u8>>>) {
        let mut u = MegaAvrUsart::new([37, 38, 39]);
        let sink = Arc::new(Mutex::new(Vec::new()));
        assert!(u.attach_console_sink(Some(sink.clone()), false));
        (u, sink)
    }

    #[test]
    fn a_byte_leaves_only_with_txen_and_sets_txcif() {
        let (mut u, sink) = console();
        assert_eq!(u.read(STATUS).unwrap(), STATUS_DREIF);
        u.write(TXDATAL, b'x').unwrap();
        assert!(
            sink.lock().unwrap().is_empty(),
            "TXEN clear: nothing leaves"
        );
        u.write(CTRLB, CTRLB_TXEN).unwrap();
        u.write(TXDATAL, b'A').unwrap();
        assert_eq!(&*sink.lock().unwrap(), b"A");
        assert_eq!(u.read(STATUS).unwrap(), STATUS_DREIF | STATUS_TXCIF);
        u.write(STATUS, STATUS_TXCIF).unwrap();
        assert_eq!(u.read(STATUS).unwrap(), STATUS_DREIF);
    }

    #[test]
    fn dreie_raises_dre_once_per_assertion() {
        let (mut u, _sink) = console();
        u.write(CTRLB, CTRLB_TXEN).unwrap();
        u.write(CTRLA, CTRLA_DREIE).unwrap();
        assert_eq!(u.tick().explicit_irqs, Some(vec![38]));
        assert_eq!(u.tick().explicit_irqs, None, "no new assertion");
        u.write(TXDATAL, b'B').unwrap();
        assert_eq!(u.tick().explicit_irqs, Some(vec![38]));
        u.write(CTRLA, 0).unwrap();
        u.write(TXDATAL, b'C').unwrap();
        assert_eq!(u.tick().explicit_irqs, None, "DREIE clear");
    }

    #[test]
    fn host_bytes_wait_in_rxdata_and_raise_rxc() {
        let (mut u, _sink) = console();
        u.uart_rx_source().unwrap().lock().unwrap().push_back(0x5A);
        assert_eq!(u.read(STATUS).unwrap() & STATUS_RXCIF, 0, "RXEN clear");
        u.write(CTRLB, CTRLB_RXEN).unwrap();
        u.write(CTRLA, CTRLA_RXCIE).unwrap();
        assert_eq!(u.read(RXDATAH).unwrap(), STATUS_RXCIF);
        assert_eq!(u.tick().explicit_irqs, Some(vec![37]));
        assert_eq!(u.read(RXDATAL).unwrap(), 0x5A);
        assert_eq!(u.read(STATUS).unwrap() & STATUS_RXCIF, 0);
        assert_eq!(u.tick().explicit_irqs, None);
    }

    #[test]
    fn loopback_feeds_the_receiver_not_the_console() {
        let (mut u, sink) = console();
        u.write(CTRLB, CTRLB_TXEN | CTRLB_RXEN).unwrap();
        u.write(CTRLA, CTRLA_LBME).unwrap();
        u.write(TXDATAL, 0xA5).unwrap();
        assert!(sink.lock().unwrap().is_empty());
        assert_eq!(u.read(RXDATAL).unwrap(), 0xA5);
    }

    #[test]
    fn unmodelled_modes_are_refused() {
        let (mut u, _sink) = console();
        assert!(u.write(CTRLA, 0x01).is_err(), "RS-485");
        assert!(u.write(CTRLC, 0x40).is_err(), "synchronous");
        assert!(u.write(CTRLB, 0x04).is_err(), "generic auto-baud");
        assert_eq!(u.read(CTRLC).unwrap(), 0x03, "reset: 8N1");
    }
}
