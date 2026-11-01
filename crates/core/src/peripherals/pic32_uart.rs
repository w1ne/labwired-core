// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! PIC32MZ UART (PIC32MZ EF data sheet DS60001320, "UART" chapter; PIC32
//! Family Reference Manual section 21). Registers UxMODE (+0x00), UxSTA
//! (+0x10), UxTXREG (+0x20), UxRXREG (+0x30), UxBRG (+0x40), each 32-bit
//! with the PIC32 CLR/SET/INV aliases at +0x4/+0x8/+0xC.
//!
//! TX is immediate (the 8-deep TX FIFO is always empty, so UTXBF reads 0
//! and TRMT reads 1); RX is the shared host queue. Interrupts are raised
//! as EVIC flags through `explicit_irqs` (`irqs: [fault, rx, tx]`): the TX
//! flag is set each time the TX buffer drains (every byte written while
//! UTXEN is set) and when UTXEN goes 1; the RX flag is set while the queue
//! reaches the URXISEL threshold.
//!
//! Under `event-scheduler` the TX flag is a one-cycle scheduler wake queued
//! by the write that caused it. Host bytes can land in the RX queue without
//! any bus call (a session's `send`, interactive stdin), so while the
//! receiver is on (`ON` and `URXEN`) the model also checks the queue once
//! per character time: ten bit times at the UxBRG/BRGH baud from `pbclk_hz`
//! (PBCLK2). A byte cannot arrive faster than that on the wire, so the RX
//! flag is at most one character late. Without the scheduler the legacy
//! tick does both. Parity/framing errors, address detect, IrDA, flow
//! control and loopback are not modelled; OERR is never set.

use crate::bus::bus_trace::{BusDir, BusPayload, BusTrace};
use crate::peripherals::device::{UartStreamDevice, UartStreamHost};
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

const CONSOLE: usize = 64 * 1024;
const TX_TOKEN: u32 = 1;
const RX_TOKEN: u32 = 2;
/// The PIC32MZ EF on the Curiosity 2.0 under NuttX: SYSCLK 200 MHz,
/// PBCLK2 (the UART clock) 100 MHz.
pub const DEFAULT_CLOCK_HZ: u64 = 200_000_000;
pub const DEFAULT_PBCLK_HZ: u64 = 100_000_000;
const MODE_ON: u32 = 1 << 15;
const MODE_BRGH: u32 = 1 << 3;
const STA_URXDA: u32 = 1 << 0;
const STA_OERR: u32 = 1 << 1;
const STA_RIDLE: u32 = 1 << 4;
const STA_TRMT: u32 = 1 << 8;
const STA_UTXBF: u32 = 1 << 9;
const STA_UTXEN: u32 = 1 << 10;
const STA_URXEN: u32 = 1 << 12;
/// Bits UxSTA reports from the model rather than storage.
const STA_DERIVED: u32 = STA_URXDA | STA_RIDLE | STA_TRMT | STA_UTXBF | (0b11 << 2);

#[derive(Default)]
struct State {
    mode: u32,
    sta: u32,
    brg: u32,
    /// A TX-buffer-empty event waits to be flagged on the EVIC.
    tx_event: bool,
    /// The TX / RX scheduler wakes are queued or live.
    tx_live: bool,
    rx_live: bool,
    pending_events: Vec<(u64, u32)>,
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo: bool,
    streams: Vec<Box<dyn UartStreamDevice>>,
}

pub struct Pic32Uart {
    state: Mutex<State>,
    rx: Arc<Mutex<VecDeque<u8>>>,
    /// EVIC vectors: fault, receive, transmit.
    irqs: [u32; 3],
    clock_hz: u64,
    pbclk_hz: u64,
    trace: BusTrace,
    trace_name: String,
    clock: Option<CycleClock>,
}

impl std::fmt::Debug for Pic32Uart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pic32Uart")
            .field("irqs", &self.irqs)
            .finish_non_exhaustive()
    }
}

impl Pic32Uart {
    pub fn new(irqs: [u32; 3]) -> Self {
        Self::with_clocks(irqs, DEFAULT_CLOCK_HZ, DEFAULT_PBCLK_HZ)
    }

    pub fn with_clocks(irqs: [u32; 3], clock_hz: u64, pbclk_hz: u64) -> Self {
        Self {
            state: Mutex::new(State {
                sta: STA_TRMT,
                ..State::default()
            }),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            irqs,
            clock_hz: clock_hz.max(1),
            pbclk_hz: pbclk_hz.max(1),
            trace: crate::bus::bus_trace::new_log(),
            trace_name: String::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn rx_enabled(s: &State) -> bool {
        Self::on(s) && s.sta & STA_URXEN != 0
    }

    fn rx_due(&self, s: &State) -> bool {
        Self::rx_enabled(s) && self.rx.lock().unwrap().len() >= Self::rx_threshold(s)
    }

    /// CPU cycles in one character (start + 8 data + stop) at the UxBRG
    /// baud: PBCLK / (16 or, with BRGH, 4) / (UxBRG + 1).
    fn char_cycles(&self, s: &State) -> u64 {
        let div: u128 = if s.mode & MODE_BRGH != 0 { 4 } else { 16 };
        let bit = div * (u128::from(s.brg) + 1);
        let cycles = 10 * bit * u128::from(self.clock_hz) / u128::from(self.pbclk_hz);
        (cycles as u64).max(1)
    }

    /// Queue the wakes the state needs: TX after one cycle, the RX check
    /// after one cycle when data already waits, else one character later.
    fn want_wake(&self, s: &mut State) {
        if !self.scheduler_mode() {
            return;
        }
        if s.tx_event && !s.tx_live {
            s.tx_live = true;
            s.pending_events.push((1, TX_TOKEN));
        }
        if Self::rx_enabled(s) && !s.rx_live {
            s.rx_live = true;
            let delay = if self.rx_due(s) {
                1
            } else {
                self.char_cycles(s)
            };
            s.pending_events.push((delay, RX_TOKEN));
        }
    }

    /// The EVIC flags due now.
    fn service(&self, s: &mut State) -> Vec<u32> {
        let mut irqs = Vec::new();
        if std::mem::take(&mut s.tx_event) {
            irqs.push(self.irqs[2]);
        }
        if self.rx_due(s) {
            irqs.push(self.irqs[1]);
        }
        irqs
    }

    fn on(s: &State) -> bool {
        s.mode & MODE_ON != 0
    }

    fn status(&self, s: &State) -> u32 {
        let mut sta = (s.sta & !STA_DERIVED) | STA_TRMT | STA_RIDLE;
        if !self.rx.lock().unwrap().is_empty() {
            sta |= STA_URXDA;
        }
        sta
    }

    /// URXISEL: 00 = any character, 01 = half full (4), 10 = 3/4 full (6).
    fn rx_threshold(s: &State) -> usize {
        match (s.sta >> 6) & 3 {
            0 => 1,
            1 => 4,
            _ => 6,
        }
    }

    fn transmit(&self, s: &mut State, v: u8) {
        self.trace.push(
            &self.trace_name,
            BusPayload::Uart {
                direction: BusDir::Tx,
                byte: v,
            },
        );
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
        s.tx_event = true;
    }

    fn check(o: u64) -> SimResult<()> {
        if o % 4 == 0 && o < 0x50 {
            Ok(())
        } else {
            Err(SimulationError::NotImplemented(format!(
                "PIC32 UART offset {o:#x}"
            )))
        }
    }

    fn read_word(&self, o: u64) -> SimResult<u32> {
        Self::check(o)?;
        let s = self.state.lock().unwrap();
        Ok(match o {
            0x00 => s.mode,
            0x10 => self.status(&s),
            0x30 if Self::on(&s) => u32::from(self.rx.lock().unwrap().pop_front().unwrap_or(0)),
            0x40 => s.brg,
            // TXREG and every CLR/SET/INV alias read as zero.
            _ => 0,
        })
    }

    fn write_word(&mut self, o: u64, v: u32) -> SimResult<()> {
        Self::check(o)?;
        let mut s = self.state.lock().unwrap();
        let alias = o & 0xC;
        let reg = o & !0xF;
        let apply = |old: u32| match alias {
            0x0 => v,
            0x4 => old & !v,
            0x8 => old | v,
            _ => old ^ v,
        };
        match reg {
            0x00 => {
                s.mode = apply(s.mode) & 0x0000_EFFF;
                self.want_wake(&mut s);
            }
            0x10 => {
                let was_txen = s.sta & STA_UTXEN != 0;
                // OERR is the only status flag software can write (clear).
                s.sta = apply(self.status(&s)) & !(STA_DERIVED | STA_OERR);
                if !was_txen && s.sta & STA_UTXEN != 0 && Self::on(&s) {
                    s.tx_event = true;
                }
                self.want_wake(&mut s);
            }
            0x20 if alias == 0 && Self::on(&s) && s.sta & STA_UTXEN != 0 => {
                self.transmit(&mut s, v as u8);
                self.want_wake(&mut s);
            }
            0x40 => s.brg = apply(s.brg) & 0xFFFF,
            _ => {}
        }
        Ok(())
    }
}

impl UartStreamHost for Pic32Uart {
    fn attach_stream_device(&mut self, dev: Box<dyn UartStreamDevice>) {
        assert!(
            dev.stream_generation().is_none(),
            "PIC32 UART has no exclusive protocol attachment"
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

impl Peripheral for Pic32Uart {
    fn attach_bus_trace(&mut self, name: &str, trace: &BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.into();
    }
    fn bus_trace_handle(&self) -> Option<BusTrace> {
        Some(self.trace.clone())
    }
    fn peek(&self, o: u64) -> Option<u8> {
        if o >= 0x50 {
            return None;
        }
        let s = self.state.lock().unwrap();
        let word = match o & !3 {
            0x00 => s.mode,
            0x10 => self.status(&s),
            0x30 => u32::from(self.rx.lock().unwrap().front().copied().unwrap_or(0)),
            0x40 => s.brg,
            _ => 0,
        };
        Some((word >> ((o & 3) * 8)) as u8)
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
        let w = self.read_word(o & !3)?;
        Ok((w >> ((o & 3) * 8)) as u8)
    }
    fn write(&mut self, o: u64, v: u8) -> SimResult<()> {
        // A byte store to the low byte of a register (TXREG) is a register
        // write of that byte; other byte lanes shift into place.
        self.write_word(o & !3, u32::from(v) << ((o & 3) * 8))
    }
    fn read_u32(&self, o: u64) -> SimResult<u32> {
        self.read_word(o)
    }
    fn write_u32(&mut self, o: u64, v: u32) -> SimResult<()> {
        self.write_word(o, v)
    }
    fn legacy_tick_active(&self) -> bool {
        if self.scheduler_mode() {
            return false;
        }
        // Host bytes can arrive without a bus call: tick while receiving.
        let s = self.state.lock().unwrap();
        s.tx_event || Self::rx_enabled(&s)
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
        if event_token == TX_TOKEN {
            s.tx_live = false;
            let irqs = std::mem::take(&mut s.tx_event)
                .then_some(self.irqs[2])
                .into_iter()
                .collect();
            return crate::sched::EventResult {
                explicit_irqs: irqs,
                ..Default::default()
            };
        }
        let irqs = if self.rx_due(&s) {
            vec![self.irqs[1]]
        } else {
            Vec::new()
        };
        // The receiver stays polled, one character apart, while it is on.
        let again = Self::rx_enabled(&s);
        s.rx_live = again;
        crate::sched::EventResult {
            explicit_irqs: irqs,
            reschedule_delay: again.then(|| self.char_cycles(&s)),
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
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

    fn tick_irqs(u: &mut Pic32Uart) -> Vec<u32> {
        u.tick().explicit_irqs.unwrap_or_default()
    }

    #[test]
    fn tx_reaches_console_only_when_on_and_enabled() {
        let mut u = Pic32Uart::new([145, 146, 147]);
        let sink = Arc::new(Mutex::new(Vec::new()));
        assert!(u.attach_console_sink(Some(sink.clone()), false));
        u.write_u32(0x20, u32::from(b'x')).unwrap();
        u.write_u32(0x08, MODE_ON).unwrap();
        u.write_u32(0x18, STA_UTXEN).unwrap();
        assert_eq!(
            tick_irqs(&mut u),
            vec![147],
            "UTXEN 0->1 empties the buffer"
        );
        for &b in b"ok" {
            u.write_u32(0x20, u32::from(b)).unwrap();
        }
        assert_eq!(&*sink.lock().unwrap(), b"ok");
        assert_eq!(tick_irqs(&mut u), vec![147]);
        assert!(tick_irqs(&mut u).is_empty(), "one flag per drain");
        let sta = u.read_u32(0x10).unwrap();
        assert_eq!(
            sta & (STA_TRMT | STA_UTXBF | STA_UTXEN),
            STA_TRMT | STA_UTXEN
        );
    }

    #[test]
    fn rx_flag_holds_until_the_queue_drains() {
        let mut u = Pic32Uart::new([145, 146, 147]);
        u.write_u32(0x00, MODE_ON).unwrap();
        u.write_u32(0x10, STA_URXEN).unwrap();
        u.rx.lock().unwrap().extend(b"ab");
        assert_eq!(u.read_u32(0x10).unwrap() & STA_URXDA, STA_URXDA);
        assert_eq!(tick_irqs(&mut u), vec![146]);
        assert_eq!(u.read_u32(0x30).unwrap(), u32::from(b'a'));
        assert_eq!(tick_irqs(&mut u), vec![146]);
        assert_eq!(u.read_u32(0x30).unwrap(), u32::from(b'b'));
        assert!(tick_irqs(&mut u).is_empty());
        assert_eq!(u.read_u32(0x10).unwrap() & STA_URXDA, 0);
        // URXISEL = 01: four characters before the flag.
        u.write_u32(0x18, 1 << 6).unwrap();
        u.rx.lock().unwrap().extend(b"abc");
        assert!(tick_irqs(&mut u).is_empty());
        u.rx.lock().unwrap().push_back(b'd');
        assert_eq!(tick_irqs(&mut u), vec![146]);
    }

    #[test]
    fn set_clr_inv_aliases_and_read_only_status() {
        let mut u = Pic32Uart::new([0, 1, 2]);
        u.write_u32(0x40, 0x1234).unwrap();
        u.write_u32(0x44, 0x0034).unwrap();
        assert_eq!(u.read_u32(0x40).unwrap(), 0x1200);
        u.write_u32(0x4C, 0x0201).unwrap();
        assert_eq!(u.read_u32(0x40).unwrap(), 0x1001);
        assert_eq!(u.read_u32(0x44).unwrap(), 0, "aliases read as zero");
        // Software cannot set URXDA, TRMT clear or UTXBF.
        u.write_u32(0x10, STA_URXDA | STA_UTXBF).unwrap();
        let sta = u.read_u32(0x10).unwrap();
        assert_eq!(sta & (STA_URXDA | STA_UTXBF | STA_TRMT), STA_TRMT);
        assert!(u.read_u32(0x50).is_err());
    }
}
