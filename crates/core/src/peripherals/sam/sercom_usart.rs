// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Microchip SERCOM in **USART** mode (SAM D21 / D51 / E5x).
//!
//! SERCOM is one block that becomes a UART, an SPI controller or an I²C
//! controller depending on `CTRLA.MODE`. This model implements the USART mode
//! only; a SERCOM configured into another mode is left inert rather than
//! answering an SPI driver out of a USART register map. `CTRLA.MODE` is
//! checked, not assumed — see [`SamSercomUsart::usart_mode`].
//!
//! Register map — `ATSAMD21G18A.svd` (Microchip Technology Inc., Apache-2.0),
//! peripheral SERCOM0, cluster USART. Every offset and field below was read
//! off that file:
//!
//! | Offset | Reg | Width | Notes |
//! |--------|-----|-------|-------|
//! | 0x00 | CTRLA | 32 | SWRST[0], ENABLE[1], MODE[4:2] |
//! | 0x04 | CTRLB | 32 | CHSIZE[2:0], TXEN[16], RXEN[17] |
//! | 0x0C | BAUD | 16 | |
//! | 0x0E | RXPL | 8 | |
//! | 0x14 | INTENCLR | 8 | write 1 to clear an enable |
//! | 0x16 | INTENSET | 8 | write 1 to set an enable |
//! | 0x18 | INTFLAG | 8 | DRE[0], TXC[1], RXC[2], RXS[3], CTSIC[4], RXBRK[5], ERROR[7] |
//! | 0x1A | STATUS | 16 | |
//! | 0x1C | SYNCBUSY | 32 | read-only |
//! | 0x28 | DATA | 16 | |
//! | 0x30 | DBGCTRL | 8 | |
//!
//! ## The two flags that decide whether firmware boots at all
//!
//! **DRE** (data register empty) is not stored. `Serial.write()` in the Arduino
//! SAMD core, `usart_serial_putchar()` in ASF and Zephyr's `uart_poll_out` all
//! spin on `while (!INTFLAG.DRE)` before every byte. This model transmits with
//! no latency — the byte is gone by the time the store returns — so the honest
//! answer is that the holding register is empty whenever the peripheral is
//! enabled, and DRE is computed from `CTRLA.ENABLE` on every read. Storing it
//! as a bit that something has to remember to set is how that loop becomes a
//! silent hang.
//!
//! **SYNCBUSY** reads 0, always. On silicon it holds while a write crosses into
//! the peripheral's clock domain, and firmware spins on it after SWRST, after
//! ENABLE and after CTRLB. Synchronisation here is instantaneous, so 0 is the
//! true answer for this model — but it is a *modelling* truth, not a silicon
//! one, and a firmware that measures the sync delay will not see it.
//!
//! ## Not modelled
//!
//! Documented rather than faked: the fractional baud generators (BAUD is stored
//! and read back, and no byte is paced by it), 9-bit frames (`CTRLB.CHSIZE` is
//! stored; DATA carries 8 bits), parity and the FORM field, the ERROR/PERR/
//! FERR/BUFOVF paths, collision detection, CTS/RTS flow control, RXS/RXBRK/
//! CTSIC, DBGCTRL.DBGSTOP, and the SERCOM SPI and I²C modes.
//!
//! **Byte pacing (SAMD51, opt-in).** An instance declared with its pads and
//! its core clock channel ([`SamSercomUsart::with_pacing`]) paces bytes at the
//! configured baud on the live clock tree: one start bit, CHSIZE data bits, a
//! parity bit when FORM selects one and one or two stop bits, each lasting
//! `S * 65536 / (65536 - BAUD)` core-clock cycles for the arithmetic SAMPR
//! settings (S = 16, 8, 3) or `S * (BAUD + FP / 8)` for the fractional ones
//! (DS60001507F 34.6.2.3). DATA feeds a one-byte holding register in front of
//! the shift register, so DRE drops while both are full and TXC rises when the
//! last stop bit has left. Host bytes arrive one frame apart into a two-byte
//! receive buffer; a third sets STATUS.BUFOVF and is lost. Without pacing,
//! the instance keeps the zero-latency behaviour above.
//!
//! Also absent: a `PadLines` wire cell. TX bytes reach the console sink and the
//! bus trace, but no narration wire is published, so a logic-analyzer WIRE
//! probe on this SERCOM has nothing to bind to. Declared rather than faked —
//! see the note on the `Peripheral` impl.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use crate::{Peripheral, PeripheralTickResult, SimResult};

// ── Register offsets ─────────────────────────────────────────────────────────
const CTRLA: u64 = 0x00;
const CTRLB: u64 = 0x04;
const BAUD: u64 = 0x0C;
const RXPL: u64 = 0x0E;
const INTENCLR: u64 = 0x14;
const INTENSET: u64 = 0x16;
const INTFLAG: u64 = 0x18;
const STATUS: u64 = 0x1A;
const SYNCBUSY: u64 = 0x1C;
const DATA: u64 = 0x28;
const DBGCTRL: u64 = 0x30;

// ── CTRLA ────────────────────────────────────────────────────────────────────
const CTRLA_SWRST: u32 = 1 << 0;
const CTRLA_ENABLE: u32 = 1 << 1;
const CTRLA_MODE_SHIFT: u32 = 2;
const CTRLA_MODE_MASK: u32 = 0b111;
/// `CTRLA.MODE` 0x0 = USART with an external clock, 0x1 = USART with the
/// internal clock. Both are USART; everything else is SPI or I²C.
const MODE_USART_EXT_CLK: u32 = 0x0;
const MODE_USART_INT_CLK: u32 = 0x1;

// ── CTRLB ────────────────────────────────────────────────────────────────────
const CTRLB_TXEN: u32 = 1 << 16;
const CTRLB_RXEN: u32 = 1 << 17;

// ── INTFLAG / INTENSET bits ──────────────────────────────────────────────────
const INT_DRE: u8 = 1 << 0;
const INT_TXC: u8 = 1 << 1;
const INT_RXC: u8 = 1 << 2;

/// The SERCOM's single self-perpetuating scheduler event. One token suffices:
/// this model has no timers and no deferred transfer — TX happens inside the
/// DATA write — only "the IRQ level may have moved".
const SERCOM_WAKE_TOKEN: u32 = 0;

/// TX and RX pads of a SAMD51 SERCOM USART: (PORT group, pin) each, their PMUX
/// function and the CTRLA.TXPO / RXPO values that select them.
#[derive(Clone, Copy, Debug)]
pub struct UsartPads {
    pub tx: (u64, u8),
    pub rx: (u64, u8),
    pub mux: u8,
    pub txpo: u8,
    pub rxpo: u8,
}

/// Byte pacing state of a SAMD51 instance (see the module docs). Times are
/// CPU cycles on the bus cycle clock.
#[derive(Debug, Default)]
struct UsartPace {
    /// GCLK peripheral channel of the SERCOM core clock (PCHCTRL index).
    core_gclk: u64,
    /// CPU cycles per bit, resolved from BAUD, SAMPR and the clock tree; `None`
    /// (no clock, external clock mode, reserved SAMPR) moves no byte.
    bit: Option<f64>,
    /// BAUD, CTRLA or CTRLB changed since `bit` was resolved.
    dirty: bool,
    /// DATA holding register: the byte and the cycle it was written.
    hold: Option<(u8, u64)>,
    /// The byte in the shift register and the cycle its stop bit ends.
    shift: Option<(u8, f64)>,
    /// When the transmitter's last frame ended.
    tx_free: f64,
    /// Receive buffer (two bytes on silicon) and the end of the frame being
    /// received from the host.
    rx_fifo: std::cell::RefCell<VecDeque<u8>>,
    rx_end: Option<f64>,
}

/// SERCOM in USART mode.
pub struct SamSercomUsart {
    ctrla: u32,
    ctrlb: u32,
    baud: u16,
    rxpl: u8,
    dbgctrl: u8,
    status: u16,
    /// Interrupt enables (INTENSET/INTENCLR are two views of one register).
    intenset: u8,
    /// The STORED flags only — TXC. DRE and RXC are derived, so they cannot go
    /// stale against the state they describe.
    intflag: u8,
    /// Bytes the outside world has queued for this UART, shared with
    /// `Bus::attach_uart_rx_source_named`.
    rx_source: Arc<Mutex<VecDeque<u8>>>,
    /// The byte most recently popped by a DATA read, so a re-read before the
    /// next arrival returns the same value silicon would hold.
    ///
    /// A `Cell` because reading DATA CONSUMES a byte and `Peripheral::read*`
    /// take `&self` — the same interior mutability the generic `Uart` gets
    /// from popping its RX buffer through a `Mutex` on a shared read.
    rx_holding: std::cell::Cell<u16>,
    /// Captured TX bytes for `test`-mode assertions (`uart_contains`).
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    /// Echo transmitted bytes to the process stdout (console behaviour).
    echo_stdout: bool,
    /// Level of `intenset & flags` at the last tick, so the IRQ is raised on
    /// the 0→1 edge instead of every cycle the flag stays set.
    irq_level: bool,
    /// SAMD51 layout: DRE, TXC, RXC and the remaining flags on four
    /// consecutive NVIC lines from the entry's IRQ (DS60001507F 10.2.2), not
    /// the SAMD21 single line. `Some(first line)` once the bus attached it.
    samd51_lines: Option<Option<u32>>,
    /// Board-declared TX/RX pads (SAMD51 instances). When declared, a byte
    /// reaches the wire only while its pad is muxed to this SERCOM and
    /// CTRLA.TXPO/RXPO select it; the routing is latched on the bus tick.
    pads: Option<UsartPads>,
    routed: (bool, bool),
    pace: Option<UsartPace>,
    /// Cycle clock handed over by the bus at registration on an
    /// `event-scheduler` build. Its presence — not the feature alone — is what
    /// [`Self::scheduler_mode`] reads, so a hand-built bus that never attaches
    /// one keeps the exact historical walk semantics.
    clock: Option<crate::cycle_clock::CycleClock>,
    /// True while a WAKE event is in flight, so a driver poking several
    /// registers in a row does not stack redundant wakeups.
    scheduled: bool,
    /// Test-only lever: pin this model back onto the per-cycle walk even
    /// though the bus attached a clock, so the walk-differential oracle can
    /// build the SAME machine twice and differ only in which path drives it.
    legacy_walk_forced: bool,
    trace: crate::bus::bus_trace::BusTrace,
    trace_name: String,
}

impl std::fmt::Debug for SamSercomUsart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SamSercomUsart")
            .field("ctrla", &self.ctrla)
            .field("ctrlb", &self.ctrlb)
            .field("baud", &self.baud)
            .field("intenset", &self.intenset)
            .field("intflag", &self.effective_intflag())
            .finish()
    }
}

impl Default for SamSercomUsart {
    fn default() -> Self {
        Self::new()
    }
}

impl SamSercomUsart {
    pub fn new() -> Self {
        Self {
            ctrla: 0,
            ctrlb: 0,
            baud: 0,
            rxpl: 0,
            dbgctrl: 0,
            status: 0,
            intenset: 0,
            intflag: 0,
            rx_source: Arc::new(Mutex::new(VecDeque::new())),
            rx_holding: std::cell::Cell::new(0),
            sink: None,
            // Default to console echo; a capture sink is attached on demand.
            echo_stdout: true,
            irq_level: false,
            samd51_lines: None,
            pads: None,
            routed: (false, false),
            pace: None,
            clock: None,
            scheduled: false,
            legacy_walk_forced: false,
            trace: crate::bus::bus_trace::BusTrace::default(),
            trace_name: String::new(),
        }
    }

    /// Declare the TX/RX pads (see [`UsartPads`]).
    pub fn with_pads(mut self, pads: UsartPads) -> Self {
        self.pads = Some(pads);
        self
    }

    /// Pace bytes at the configured baud from the SERCOM core clock on GCLK
    /// channel `core_gclk` (see the module docs). Needs declared pads.
    pub fn with_pacing(mut self, core_gclk: u64) -> Self {
        self.pace = Some(UsartPace {
            core_gclk,
            dirty: true,
            ..Default::default()
        });
        self
    }

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, |c| c.now())
    }

    /// Bits per frame: start, CHSIZE data bits, parity when FORM has one, stop.
    fn frame_bits(&self) -> f64 {
        let data = match self.ctrlb & 7 {
            1 => 9,
            n @ 5..=7 => n,
            _ => 8,
        };
        let parity = matches!((self.ctrla >> 24) & 0xf, 1 | 5) as u32;
        let stop = 1 + (self.ctrlb >> 6 & 1);
        f64::from(1 + data + parity + stop)
    }

    /// CPU cycles per bit from BAUD and CTRLA.SAMPR at the live clock rates.
    fn resolve_bit(&self, bus: &dyn crate::Bus, core_gclk: u64) -> Option<f64> {
        if (self.ctrla >> CTRLA_MODE_SHIFT) & CTRLA_MODE_MASK != MODE_USART_INT_CLK {
            return None;
        }
        let gclk = super::clock_tree::peripheral_hz(bus, core_gclk)? as f64;
        let cpu = super::clock_tree::cpu_hz(bus)? as f64;
        let core = bit_clocks(self.ctrla, self.baud)?;
        (gclk > 0.0).then_some(core * cpu / gclk)
    }

    /// Advance the paced transmitter and receiver to cycle `now`.
    fn settle(&mut self, now: u64) {
        let frame = match self.pace.as_ref().and_then(|p| p.bit) {
            Some(bit) if self.enabled() => bit * self.frame_bits(),
            _ => return,
        };
        let (routed_tx, routed_rx) = self.routed;
        let receiving = self.ctrlb & CTRLB_RXEN != 0 && routed_rx;
        let now_f = now as f64;
        let mut emitted = Vec::new();
        let mut txc = false;
        {
            let pace = self.pace.as_mut().unwrap();
            loop {
                if let Some((byte, end)) = pace.shift {
                    if end > now_f {
                        break;
                    }
                    pace.shift = None;
                    pace.tx_free = end;
                    if routed_tx {
                        emitted.push(byte);
                    }
                    txc = pace.hold.is_none();
                }
                // The holding register moves into an idle shift register.
                match pace.hold.take() {
                    Some((byte, written)) => {
                        let start = pace.tx_free.max(written as f64);
                        pace.shift = Some((byte, start + frame));
                        txc = false;
                    }
                    None => break,
                }
            }
            if receiving {
                loop {
                    let queued = self
                        .rx_source
                        .lock()
                        .map(|q| !q.is_empty())
                        .unwrap_or(false);
                    if !queued {
                        pace.rx_end = None;
                        break;
                    }
                    let end = *pace.rx_end.get_or_insert(now_f + frame);
                    if end > now_f {
                        break;
                    }
                    let byte = self.rx_source.lock().ok().and_then(|mut q| q.pop_front());
                    let mut fifo = pace.rx_fifo.borrow_mut();
                    match byte {
                        Some(byte) if fifo.len() < 2 => fifo.push_back(byte),
                        Some(_) => self.status |= 1 << 2, // STATUS.BUFOVF
                        None => {}
                    }
                    drop(fifo);
                    pace.rx_end = Some(end + frame);
                }
            }
        }
        for byte in emitted {
            self.emit_byte(byte);
        }
        if txc {
            self.intflag |= INT_TXC;
        }
    }

    /// The next cycle a paced byte finishes (the shift register's stop bit, a
    /// byte arriving from the host) or, with a byte waiting in the holding
    /// register of an idle transmitter, now.
    fn pace_deadline(&self) -> Option<f64> {
        let pace = self.pace.as_ref()?;
        let waiting = pace.hold.is_some() && pace.shift.is_none();
        let receiving = self.ctrlb & CTRLB_RXEN != 0;
        let ends = [
            pace.shift.map(|s| s.1),
            pace.rx_end.filter(|_| receiving),
            waiting.then_some(self.now() as f64),
        ];
        ends.into_iter().flatten().reduce(f64::min)
    }

    /// Latch the pad routing, resolve the bit time when the clocks or
    /// registers changed, and move the paced transmitter and receiver to now.
    fn advance(&mut self, bus: &dyn crate::Bus) {
        if let Some(p) = self.pads {
            let txpo = (self.ctrla >> 16) & 3 == u32::from(p.txpo);
            let rxpo = (self.ctrla >> 20) & 3 == u32::from(p.rxpo);
            self.routed = (
                txpo && self.pad_routed(bus, p.tx.0, p.tx.1, p.mux),
                rxpo && self.pad_routed(bus, p.rx.0, p.rx.1, p.mux),
            );
        }
        if let Some(core_gclk) = self
            .pace
            .as_ref()
            .filter(|p| p.dirty || p.bit.is_none())
            .map(|p| p.core_gclk)
        {
            let bit = self.resolve_bit(bus, core_gclk);
            let pace = self.pace.as_mut().unwrap();
            pace.bit = bit;
            pace.dirty = false;
        }
        let now = self.now();
        self.settle(now);
    }

    fn pad_routed(&self, bus: &dyn crate::Bus, port: u64, pin: u8, mux: u8) -> bool {
        let cfg = bus.read_u8(port + 0x40 + u64::from(pin)).unwrap_or(0);
        let pmux = bus.read_u8(port + 0x30 + u64::from(pin / 2)).unwrap_or(0);
        let selected = if pin & 1 != 0 { pmux >> 4 } else { pmux & 0xf };
        cfg & 1 != 0 && selected == mux
    }

    /// The SAMD51 four-line interrupt layout (`samd51_sercom_usart`).
    pub fn new_samd51() -> Self {
        Self {
            samd51_lines: Some(None),
            ..Self::new()
        }
    }

    /// SAMD51 lines asserted now: DRE, TXC, RXC, then RXS/CTSIC/RXBRK/ERROR.
    fn samd51_pending(&self) -> Vec<u32> {
        let Some(Some(base)) = self.samd51_lines else {
            return Vec::new();
        };
        let active = self.intenset & self.effective_intflag();
        [INT_DRE, 1 << 1, INT_RXC, !(INT_DRE | (1 << 1) | INT_RXC)]
            .into_iter()
            .enumerate()
            .filter_map(|(line, mask)| (active & mask != 0).then_some(base + line as u32))
            .collect()
    }

    /// Attach a capture sink and/or toggle stdout echo. Mirrors
    /// `Nrf52Uarte::set_sink` so `Bus::attach_uart_tx_sink` wires a SERCOM
    /// console exactly the same way.
    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }

    /// Shared handle to the RX injection queue, for
    /// `Bus::attach_uart_rx_source_named`.
    pub fn rx_buffer(&self) -> Arc<Mutex<VecDeque<u8>>> {
        self.rx_source.clone()
    }

    /// True when `CTRLA.MODE` selects a USART. A SERCOM put into SPI or I²C
    /// mode is NOT this peripheral, and answering its driver from a USART
    /// register map would be a wrong answer rather than a missing one.
    fn usart_mode(&self) -> bool {
        let mode = (self.ctrla >> CTRLA_MODE_SHIFT) & CTRLA_MODE_MASK;
        mode == MODE_USART_INT_CLK || mode == MODE_USART_EXT_CLK
    }

    fn enabled(&self) -> bool {
        self.ctrla & CTRLA_ENABLE != 0 && self.usart_mode()
    }

    fn rx_queued(&self) -> bool {
        if let Some(pace) = &self.pace {
            return !pace.rx_fifo.borrow().is_empty();
        }
        if self.pads.is_some() && !self.routed.1 {
            return false; // bytes on an unrouted RX pad never reach the receiver
        }
        self.rx_source
            .lock()
            .map(|q| !q.is_empty())
            .unwrap_or(false)
    }

    /// INTFLAG as firmware reads it: the stored TXC plus the two derived flags.
    ///
    /// DRE is set whenever the peripheral is enabled (this model's transmit
    /// holding register is never occupied — see the module docs). RXC is set
    /// whenever RXEN is on and a byte is waiting, which is the definition on
    /// silicon: the flag IS "the receive buffer is full", not a latch someone
    /// has to remember to set.
    fn effective_intflag(&self) -> u8 {
        let mut flags = self.intflag;
        if self.enabled() && self.pace.as_ref().is_none_or(|p| p.hold.is_none()) {
            flags |= INT_DRE;
        }
        if self.enabled() && self.ctrlb & CTRLB_RXEN != 0 && self.rx_queued() {
            flags |= INT_RXC;
        } else {
            flags &= !INT_RXC;
        }
        flags
    }

    /// Pin this SERCOM onto the legacy per-cycle walk. See
    /// [`Self::legacy_walk_forced`].
    pub fn force_legacy_walk(&mut self) {
        self.legacy_walk_forced = true;
    }

    // The shared macro, NOT a hand-written `cfg!`. Folding the private
    // `scheduler_mode()` bodies into it is what took the engine's
    // conditional-compilation surface from 208 to 179; a hand-written twin
    // here would put one site back per model, for nothing.
    crate::cycle_clock::scheduler_mode!();

    /// `scheduler_mode()` plus the test-only walk pin, kept OUT of the macro so
    /// the forced-walk escape costs no conditional-compilation site.
    #[inline]
    fn sched_driven(&self) -> bool {
        self.scheduler_mode() && !self.legacy_walk_forced
    }

    /// The level->edge conversion the legacy `tick()` performed, extracted so
    /// the scheduler path produces a bit-identical IRQ stream.
    fn irq_edge(&mut self) -> bool {
        let asserted = self.intenset & self.effective_intflag() != 0;
        let edge = asserted && !self.irq_level;
        self.irq_level = asserted;
        edge
    }

    /// Whether the scheduler must keep waking this SERCOM.
    ///
    /// The second term is the one that is easy to miss. `effective_intflag`
    /// derives `INT_RXC` from `rx_queued()`, and those bytes arrive from the
    /// HOST, on another thread, through `rx_source`. Nothing on the bus fires
    /// when one lands, so an enabled receiver must be POLLED — which the
    /// per-cycle walk did for free. Without this term a SERCOM console would
    /// go deaf the moment the walk was deleted, and the firmware would simply
    /// wait forever: no error, no output, nothing to grep for.
    fn has_active_work(&self) -> bool {
        let level_moved = (self.intenset & self.effective_intflag() != 0) != self.irq_level;
        let receiver_armed =
            self.enabled() && self.ctrlb & CTRLB_RXEN != 0 && self.intenset & INT_RXC != 0;
        // A paced byte in flight changes DRE, TXC and RXC when its frame ends,
        // and nothing on the bus fires then.
        let pacing = self.enabled() && self.pace_deadline().is_some();
        level_moved || receiver_armed || pacing
    }

    /// Reset to power-on state, keeping the wiring (sink, RX queue, trace) that
    /// belongs to the harness rather than to the silicon. `CTRLA.SWRST`
    /// self-clears: on silicon it reads 1 only while the reset is in progress,
    /// and this one completes within the store.
    fn software_reset(&mut self) {
        self.ctrla = 0;
        self.ctrlb = 0;
        self.baud = 0;
        self.rxpl = 0;
        self.dbgctrl = 0;
        self.status = 0;
        self.intenset = 0;
        self.intflag = 0;
        self.rx_holding.set(0);
        self.irq_level = false;
        if let Some(pace) = &mut self.pace {
            *pace = UsartPace {
                core_gclk: pace.core_gclk,
                dirty: true,
                ..Default::default()
            };
        }
    }

    fn emit_byte(&mut self, byte: u8) {
        self.trace.push(
            &self.trace_name,
            crate::bus::bus_trace::BusPayload::Uart {
                direction: crate::bus::bus_trace::BusDir::Tx,
                byte,
            },
        );
        if let Some(sink) = &self.sink {
            if let Ok(mut guard) = sink.lock() {
                guard.push(byte);
            }
        }
        if self.echo_stdout {
            #[allow(unused_must_use)]
            {
                print!("{}", byte as char);
                io::stdout().flush();
            }
        }
    }

    /// A DATA write: transmit, if the peripheral is in a state that can.
    ///
    /// A store to DATA while the SERCOM is disabled, in a non-USART mode, or
    /// with TXEN clear must NOT emit. Silicon drops it, and a model that emits
    /// anyway prints a console line for a UART the firmware never brought up —
    /// which reads as "the port works" in exactly the runs where it does not.
    fn write_data(&mut self, value: u16) {
        if !self.enabled() || self.ctrlb & CTRLB_TXEN == 0 {
            return;
        }
        if self.pace.is_some() {
            let now = self.now();
            self.settle(now);
            let pace = self.pace.as_mut().unwrap();
            if pace.hold.is_none() {
                // A write while DRE is clear is ignored, as on silicon.
                pace.hold = Some(((value & 0xFF) as u8, now));
                self.intflag &= !INT_TXC; // writing DATA clears TXC
                self.settle(now);
            }
            return;
        }
        // The SERCOM transmits either way; an unrouted TX pad carries nothing.
        if self.pads.is_none() || self.routed.0 {
            self.emit_byte((value & 0xFF) as u8);
        }
        // Transmission is complete by the time the store returns.
        self.intflag |= INT_TXC;
    }

    /// A DATA read: hand over the oldest queued byte and clear RXC by
    /// consuming it. Reading with nothing queued returns the holding register,
    /// as silicon does — not a fabricated zero.
    fn read_data(&self) -> u16 {
        if let Some(pace) = &self.pace {
            if let Some(byte) = pace.rx_fifo.borrow_mut().pop_front() {
                self.rx_holding.set(u16::from(byte));
            }
            return self.rx_holding.get();
        }
        if self.enabled() && self.ctrlb & CTRLB_RXEN != 0 {
            if let Ok(mut q) = self.rx_source.lock() {
                if let Some(byte) = q.pop_front() {
                    self.rx_holding.set(u16::from(byte));
                }
            }
        }
        self.rx_holding.get()
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            CTRLA => self.ctrla,
            CTRLB => self.ctrlb,
            BAUD => u32::from(self.baud),
            RXPL => u32::from(self.rxpl),
            // INTENCLR and INTENSET are two views of the same enable register.
            INTENCLR | INTENSET => u32::from(self.intenset),
            INTFLAG => u32::from(self.effective_intflag()),
            STATUS => u32::from(self.status),
            // Synchronisation is instantaneous in this model — see module docs.
            SYNCBUSY => 0,
            DATA => u32::from(self.read_data()),
            DBGCTRL => u32::from(self.dbgctrl),
            _ => {
                crate::census_reg!("sam:SercomUsart", offset, "read");
                0
            }
        }
    }
}

impl Peripheral for SamSercomUsart {
    fn on_system_reset(&mut self) {
        self.software_reset();
    }
    fn needs_bus_tick(&self) -> bool {
        self.pads.is_some() && self.enabled()
    }
    fn tick_with_bus(&mut self, bus: &mut dyn crate::Bus) {
        self.advance(bus);
    }
    fn attach_bus_trace(&mut self, name: &str, trace: &crate::bus::bus_trace::BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.to_string();
    }

    fn bus_trace_handle(&self) -> Option<crate::bus::bus_trace::BusTrace> {
        Some(self.trace.clone())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let base = reg_base(offset);
        let word = self.read_reg(base);
        Ok(((word >> ((offset - base) * 8)) & 0xFF) as u8)
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok((self.read_reg(reg_base(offset)) & 0xFFFF) as u16)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(reg_base(offset)))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let base = reg_base(offset);
        let shift = (offset - base) * 8;
        // 8-bit registers take the byte directly; wider ones are
        // read-modify-written so a byte store lands in the right lane.
        let word = match base {
            INTENCLR | INTENSET | INTFLAG | RXPL | DBGCTRL => u32::from(value),
            _ => {
                let mask = 0xFFu32 << shift;
                (self.read_reg(base) & !mask) | (u32::from(value) << shift)
            }
        };
        // Only the lane carrying the side effect triggers it: a 16-bit DATA
        // store decomposes into two byte writes, and transmitting on both
        // would double every character on the console.
        let triggers = shift == 0;
        self.write_reg(base, word, triggers);
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write_reg(reg_base(offset), u32::from(value), true);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(reg_base(offset), value, true);
        Ok(())
    }

    /// Level-derived peripheral IRQ, emitted on the 0->1 edge.
    ///
    /// Inert in scheduler mode — `on_event` owns the edge there. Letting both
    /// run would not double the IRQ (the second caller sees the level the
    /// first just latched) but it WOULD race for which of them observes the
    /// transition, i.e. which cycle the NVIC is pended on.
    fn tick(&mut self) -> PeripheralTickResult {
        if self.sched_driven() {
            return PeripheralTickResult::default();
        }
        if self.samd51_lines.is_some() {
            self.irq_edge(); // keep the latched level for the work predicate
            let lines = self.samd51_pending();
            return PeripheralTickResult {
                explicit_irqs: (!lines.is_empty()).then_some(lines),
                ..Default::default()
            };
        }
        PeripheralTickResult {
            irq: self.irq_edge(),
            ..Default::default()
        }
    }

    fn attach_irq_line(&mut self, irq: Option<u32>) {
        if self.samd51_lines.is_some() {
            self.samd51_lines = Some(irq.filter(|irq| *irq <= 252));
        }
    }

    fn reads_can_deassert_irq(&self) -> bool {
        // SAMD51 lines are level sources, and reading DATA consumes RX (RXC).
        self.samd51_lines.is_some()
    }

    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        if let Some(Some(base)) = self.samd51_lines {
            let pending = self.samd51_pending();
            for line in 0..4 {
                report(base + line, pending.contains(&(base + line)));
            }
        }
    }

    /// Bare-CPU-oracle twin of [`Self::tick`]: the forced walk settles
    /// peripherals through their historical path even when the scheduler owns
    /// them, so the edge conversion must run here regardless of mode.
    ///
    /// This model needs no `needs_bus_tick_forced` / `tick_with_bus_forced`
    /// pair, unlike its nRF54L cousins: it has no deferred transfer to
    /// re-expose. A SERCOM transmits inside the DATA write.
    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        if self.samd51_lines.is_some() {
            self.irq_edge();
            let lines = self.samd51_pending();
            return PeripheralTickResult {
                explicit_irqs: (!lines.is_empty()).then_some(lines),
                ..Default::default()
            };
        }
        PeripheralTickResult {
            irq: self.irq_edge(),
            ..Default::default()
        }
    }

    /// Only in the walk while there is something to notice — the SAME
    /// predicate the scheduler wakes on, deliberately.
    ///
    /// The narrower "has the IRQ level moved?" test is WRONG here, and wrong
    /// in a way that is silent. `refresh_legacy_tick_index` runs after an MMIO
    /// write to this peripheral, after its own tick, or on a GPIO edge; a host
    /// thread pushing a byte into `rx_source` is none of those. So a SERCOM
    /// that drops out of the walk while idle can never be put back by an
    /// arriving byte, and the legacy path goes deaf — which is exactly why
    /// this model shipped with no override at all and paid a virtual call on
    /// every simulated cycle, on all six instances.
    ///
    /// Keeping an armed receiver in the walk restores that, while a disabled
    /// or interrupt-less SERCOM still drops out. Caught by
    /// `atsamd21_sercom_walk_differential` on its REFERENCE lane.
    fn legacy_tick_active(&self) -> bool {
        self.has_active_work()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn attach_cycle_clock(&mut self, clock: crate::cycle_clock::CycleClock) {
        self.clock = Some(clock);
    }

    fn uses_scheduler(&self) -> bool {
        self.sched_driven()
    }

    fn needs_legacy_walk(&self) -> bool {
        // The only thing the walk did here — convert the IRQ level to an edge
        // — rides the WAKE event instead, so the walk is deletable.
        //
        // `derive_walk_deletable` is an ALL over the bus: while ANY peripheral
        // answers true the whole board stays pinned to
        // `max_safe_tick_interval() == 1`, and at a one-instruction window the
        // Cortex-M hot-loop fast path (budget >= 8) never engages. On
        // atsamd21g18a that clamp costs ~47x (2567.5 Ir/step against
        // nrf52840's 54.7 on the same fixture and ISA), and all six SERCOMs
        // are this one model.
        !self.sched_driven()
    }

    /// Hand the bus one self-perpetuating WAKE when there is work and none is
    /// already in flight. Called after every MMIO write to this peripheral and
    /// once at scheduler bootstrap — the latter matters here, because an RX
    /// stream attached before firmware runs must still get polled.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.has_active_work() && !self.scheduled {
            self.scheduled = true;
            vec![(0, SERCOM_WAKE_TOKEN)]
        } else {
            Vec::new()
        }
    }

    /// Latch the edge, then re-arm while there is still work.
    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        self.advance(bus);
        let irq = self.irq_edge();
        let keep_going = self.has_active_work();
        self.scheduled = keep_going;

        // Anything outstanding here is an armed receiver waiting on a host
        // that has not typed yet — `irq_edge` just latched the level, so the
        // edge term is settled. That makes the re-arm a POLL, and polling at
        // delay 1 would hold the next-event deadline one cycle ahead of the
        // CPU forever, which is the very clamp this migration exists to lift.
        // Wake at the bus tick interval instead: byte VALUES and ORDER are
        // unchanged, only the instant an injected byte becomes visible is
        // quantised, by at most one interval — the same bound every other
        // scheduler-driven peripheral on a walk-deleted bus already carries.
        // No `#[cfg]` pair: `Bus::peripheral_tick_interval` is available in
        // both worlds and defaults to 1, so the featureless build gets the
        // legacy one-tick cadence from the same expression.
        let mut delay = u64::from(bus.peripheral_tick_interval().max(1));
        // A paced frame wakes the SERCOM exactly when it ends. A deadline
        // already behind (no core clock yet, so nothing moves) polls instead.
        if let Some(end) = self.pace_deadline().filter(|&end| end > self.now() as f64) {
            delay = delay.min((end - self.now() as f64).ceil() as u64);
        }

        if self.samd51_lines.is_some() {
            return crate::sched::EventResult {
                explicit_irqs: self.samd51_pending(),
                reschedule_delay: keep_going.then_some(delay),
                ..Default::default()
            };
        }
        crate::sched::EventResult {
            // `raise_own_irq`, not `raise_irq`: this model does not know its
            // own NVIC line — the bus maps it from `PeripheralEntry::irq`,
            // exactly as it mapped the legacy `PeripheralTickResult::irq`.
            raise_own_irq: irq,
            reschedule_delay: keep_going.then_some(delay),
            ..Default::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    // NOTE: `line_names()` is deliberately NOT implemented. Naming TX/RX here
    // without a `wire_lines()` PadLines cell to back them is a claim the model
    // cannot honour: a wire probe would resolve to a channel with no source and
    // read a flat line, which looks like a quiet bus rather than a missing
    // feature. `bus_visibility` catches exactly that pairing, and the honest
    // answer while this model emits into the console sink rather than onto a
    // modelled wire is the default `&[]` — "this model publishes no wire".
    // Publishing one is follow-up work, alongside SERCOM SPI/I2C modes.
}

impl SamSercomUsart {
    fn mark_pace_dirty(&mut self) {
        if let Some(pace) = &mut self.pace {
            pace.dirty = true;
        }
    }

    /// Apply a write to `base`. `triggers` says whether this access carries the
    /// register's side effect (transmit, receive, reset) — false for the upper
    /// byte lanes of a decomposed multi-byte store.
    fn write_reg(&mut self, base: u64, value: u32, triggers: bool) {
        match base {
            CTRLA => {
                if triggers && value & CTRLA_SWRST != 0 {
                    self.software_reset();
                    return;
                }
                // SWRST never reads back as set: it self-clears.
                self.ctrla = value & !CTRLA_SWRST;
                self.mark_pace_dirty();
            }
            CTRLB => {
                self.ctrlb = value;
                self.mark_pace_dirty();
            }
            BAUD => {
                self.baud = (value & 0xFFFF) as u16;
                self.mark_pace_dirty();
            }
            RXPL => self.rxpl = (value & 0xFF) as u8,
            // Write 1 to clear / write 1 to set — two views, one register.
            INTENCLR => self.intenset &= !((value & 0xFF) as u8),
            INTENSET => self.intenset |= (value & 0xFF) as u8,
            // INTFLAG is write-1-to-clear, and only for the flags that LATCH.
            // DRE and RXC are derived from live state, so a W1C store cannot
            // clear them — on silicon it cannot either: DRE clears by writing
            // DATA and RXC by reading it.
            INTFLAG => self.intflag &= !((value & 0xFF) as u8) | INT_DRE | INT_RXC,
            STATUS => {
                // The error flags are W1C. None of them are raised by this
                // model, so this keeps STATUS at zero rather than letting a
                // driver's clear-on-boot store latch a flag that never fired.
                self.status &= !((value & 0xFFFF) as u16);
            }
            // SYNCBUSY is read-only.
            SYNCBUSY => {}
            DATA => {
                if triggers {
                    self.write_data((value & 0xFFFF) as u16);
                }
            }
            DBGCTRL => self.dbgctrl = (value & 0xFF) as u8,
            _ => {
                crate::census_reg!("sam:SercomUsart", base, "write");
            }
        }
    }
}

/// Core-clock cycles per bit for CTRLA.SAMPR and BAUD (DS60001507F 34.6.2.3):
/// arithmetic `S * 65536 / (65536 - BAUD)` for S = 16, 8, 3, fractional
/// `S * (BAUD[12:0] + FP / 8)` for S = 16, 8; reserved settings give `None`.
fn bit_clocks(ctrla: u32, baud: u16) -> Option<f64> {
    let arithmetic = |samples: f64| samples * 65536.0 / (65536.0 - f64::from(baud));
    let fractional =
        |samples: f64| samples * (f64::from(baud & 0x1fff) + f64::from(baud >> 13) / 8.0);
    let clocks = match (ctrla >> 13) & 7 {
        0 => arithmetic(16.0),
        1 => fractional(16.0),
        2 => arithmetic(8.0),
        3 => fractional(8.0),
        4 => arithmetic(3.0),
        _ => return None,
    };
    (clocks > 0.0).then_some(clocks)
}

/// The base offset of the register covering `offset`.
///
/// SERCOM mixes 8-, 16- and 32-bit registers in one window, so an access
/// cannot simply be masked to a word boundary: `INTFLAG` at 0x18 and
/// `STATUS` at 0x1A share a word, and rounding 0x1A down to 0x18 would serve
/// the interrupt flags to a driver reading the error status.
fn reg_base(offset: u64) -> u64 {
    match offset {
        0x00..=0x03 => CTRLA,
        0x04..=0x07 => CTRLB,
        0x0C..=0x0D => BAUD,
        0x0E => RXPL,
        0x14..=0x15 => INTENCLR,
        0x16..=0x17 => INTENSET,
        0x18..=0x19 => INTFLAG,
        0x1A..=0x1B => STATUS,
        0x1C..=0x1F => SYNCBUSY,
        0x28..=0x29 => DATA,
        0x30 => DBGCTRL,
        other => other,
    }
}

/// The scheduler path.
///
/// This model had NO unit tests at all before the migration — 465 lines, six
/// live instances on every ATSAMD21 board, and nothing pinning its behaviour.
/// So these are not just a scheduler-mode supplement; they are the first
/// tests this file has had, and they deliberately cover the legacy path too,
/// so the differential has something to be differential *against*.
/// Gated on the FEATURE as well as `test`, like its `nrf54l/clock.rs` sibling.
/// `SystemBus::add_peripheral` hands every peripheral a `CycleClock`
/// unconditionally, so without the feature `scheduler_mode()` is false while
/// `clock.is_some()` is true — these tests attach a clock and assert
/// `uses_scheduler()`, which only holds in the feature-on world.
#[cfg(all(test, feature = "event-scheduler"))]
mod scheduler_mode_tests {
    use super::*;
    use crate::cycle_clock::CycleClock;
    use crate::sched::EventScheduler;
    use crate::Peripheral;

    /// An enabled USART with the receiver on and RXC interrupts armed — the
    /// configuration a Zephyr or Arduino console driver leaves behind.
    fn console(sched: bool) -> SamSercomUsart {
        let mut u = SamSercomUsart::new();
        u.set_sink(
            Some(std::sync::Arc::new(std::sync::Mutex::new(Vec::new()))),
            false,
        );
        u.write_u32(
            CTRLA,
            CTRLA_ENABLE | (MODE_USART_INT_CLK << CTRLA_MODE_SHIFT),
        )
        .unwrap();
        u.write_u32(CTRLB, CTRLB_TXEN | CTRLB_RXEN).unwrap();
        u.write_u32(INTENSET, u32::from(INT_RXC)).unwrap();
        if sched {
            u.attach_cycle_clock(CycleClock::default());
        }
        u
    }

    #[test]
    fn attaching_a_clock_leaves_the_walk() {
        let u = console(true);
        assert!(u.uses_scheduler());
        assert!(
            !u.needs_legacy_walk(),
            "and the per-cycle walk is deletable — `derive_walk_deletable` is \
             an ALL, so one holdout pins the whole board to \
             max_safe_tick_interval() == 1"
        );
    }

    #[test]
    fn no_clock_stays_on_the_legacy_walk() {
        let u = console(false);
        assert!(!u.uses_scheduler());
        assert!(
            u.needs_legacy_walk(),
            "a hand-built bus never wired the scheduler, so a SERCOM that \
             stopped ticking would never deliver an interrupt at all"
        );
    }

    /// The regression this migration could most easily introduce, and the one
    /// that would be hardest to notice: an armed receiver whose bytes arrive
    /// from a HOST thread. Nothing on the bus fires when one lands, so the
    /// scheduler must keep asking. If it stopped, the console would simply go
    /// deaf — no error, no output, nothing to grep for.
    #[test]
    fn an_armed_receiver_keeps_polling_for_host_bytes() {
        let mut u = console(true);
        assert_eq!(u.take_scheduled_events(), vec![(0, SERCOM_WAKE_TOKEN)]);

        let mut sched = EventScheduler::new();
        let mut bus = crate::bus::SystemBus::empty();
        let r = u.on_event(SERCOM_WAKE_TOKEN, &mut sched, &mut bus);
        assert!(
            r.reschedule_delay.is_some(),
            "an armed receiver with an empty queue must re-arm the poll, not \
             give up — giving up makes the console deaf"
        );

        // Host types. The next wake must see it and raise the edge.
        u.rx_buffer().lock().unwrap().push_back(b'x');
        let r = u.on_event(SERCOM_WAKE_TOKEN, &mut sched, &mut bus);
        assert!(
            r.raise_own_irq,
            "RXC is enabled and a byte is queued → edge"
        );
        assert_eq!(
            u.read_u16(DATA).unwrap(),
            u16::from(b'x'),
            "and the byte itself must still be readable through DATA"
        );
    }

    /// The poll must not be the thing that re-imposes the clamp it lifted.
    #[test]
    fn the_idle_receiver_poll_is_paced_at_the_tick_interval() {
        let mut u = console(true);
        let mut sched = EventScheduler::new();
        let mut bus = crate::bus::SystemBus::empty();
        bus.config.peripheral_tick_interval = 64;

        let r = u.on_event(SERCOM_WAKE_TOKEN, &mut sched, &mut bus);
        assert_eq!(
            r.reschedule_delay,
            Some(64),
            "delay 1 would hold the next-event deadline one cycle ahead of \
             the CPU forever, pinning plan_cpu_window back to a \
             one-instruction quantum"
        );
    }

    /// With the receiver DISABLED there is nothing to poll for, so an idle
    /// SERCOM must schedule nothing at all. Six instances each holding a
    /// one-cycle deadline would make the walk deletion worthless.
    #[test]
    fn a_quiet_sercom_schedules_nothing() {
        let mut u = SamSercomUsart::new();
        u.attach_cycle_clock(CycleClock::default());
        assert!(u.take_scheduled_events().is_empty());
        assert!(!u.legacy_tick_active());
    }

    /// The walk path is untouched for a bus that never attached a clock.
    #[test]
    fn the_legacy_edge_still_fires_without_a_clock() {
        let mut u = console(false);
        u.rx_buffer().lock().unwrap().push_back(b'y');
        assert!(
            u.legacy_tick_active(),
            "the level moved, so the walk has work"
        );
        assert!(u.tick().irq, "0→1 edge");
        assert!(!u.tick().irq, "and only once, until the level drops");
    }

    /// The scheduler and the walk must agree on the edge, byte for byte. This
    /// is the model-level shadow of the walk-differential gate.
    #[test]
    fn both_paths_produce_the_same_edge_sequence() {
        let mut walk = console(false);
        let mut sch = console(true);
        let mut sched = EventScheduler::new();
        let mut bus = crate::bus::SystemBus::empty();

        let mut walk_edges = Vec::new();
        let mut sched_edges = Vec::new();
        for step in 0..8 {
            if step == 2 {
                walk.rx_buffer().lock().unwrap().push_back(b'a');
                sch.rx_buffer().lock().unwrap().push_back(b'a');
            }
            if step == 5 {
                // Firmware reads DATA, consuming the byte and dropping RXC.
                let _ = walk.read_u16(DATA).unwrap();
                let _ = sch.read_u16(DATA).unwrap();
            }
            walk_edges.push(walk.tick().irq);
            sched_edges.push(
                sch.on_event(SERCOM_WAKE_TOKEN, &mut sched, &mut bus)
                    .raise_own_irq,
            );
        }
        assert_eq!(
            walk_edges, sched_edges,
            "the scheduler path must reproduce the walk's edge sequence exactly"
        );
        assert!(
            walk_edges.iter().any(|e| *e),
            "vacuity guard: the fixture must actually produce an edge, or this \
             comparison is two streams of `false`"
        );
    }
}

#[cfg(test)]
mod pacing_tests {
    use super::*;
    use crate::cycle_clock::CycleClock;

    /// An enabled, routed SAMD51 USART (internal clock, TX and RX on) paced at
    /// `bit` CPU cycles per bit, on a host-controlled clock.
    fn paced(bit: f64) -> (SamSercomUsart, CycleClock, Arc<Mutex<Vec<u8>>>) {
        let pads = UsartPads {
            tx: (0, 16),
            rx: (0, 17),
            mux: 2,
            txpo: 0,
            rxpo: 1,
        };
        let mut u = SamSercomUsart::new_samd51().with_pads(pads).with_pacing(35);
        let clock = CycleClock::default();
        u.attach_cycle_clock(clock.clone());
        let sink = Arc::new(Mutex::new(Vec::new()));
        u.set_sink(Some(sink.clone()), false);
        u.write_u32(
            CTRLA,
            CTRLA_ENABLE | (MODE_USART_INT_CLK << CTRLA_MODE_SHIFT),
        )
        .unwrap();
        u.write_u32(CTRLB, CTRLB_TXEN | CTRLB_RXEN).unwrap();
        u.routed = (true, true);
        let pace = u.pace.as_mut().unwrap();
        pace.bit = Some(bit);
        pace.dirty = false;
        (u, clock, sink)
    }

    fn at(u: &mut SamSercomUsart, clock: &CycleClock, cycle: u64) {
        clock.publish(cycle);
        u.settle(cycle);
    }

    #[test]
    fn codal_115200_on_100_mhz_is_the_arithmetic_baud() {
        // BAUD = 65536 - (16 * 115200 << 16) / 100 MHz = 64329.
        let clocks = bit_clocks(0, 64329).unwrap();
        assert!((clocks - 16.0 * 65536.0 / 1207.0).abs() < 1e-9);
        assert!(
            (100e6 / clocks - 115_108.0).abs() < 1.0,
            "the baud it yields"
        );
        // Fractional 16x: BAUD 54, FP 2 is 16 * 54.25 clocks.
        assert_eq!(bit_clocks(1 << 13, 54 | 2 << 13), Some(16.0 * 54.25));
        assert_eq!(bit_clocks(5 << 13, 0), None, "reserved SAMPR");
    }

    #[test]
    fn a_byte_in_flight_keeps_the_scheduler_waking_until_its_frame_ends() {
        // Frame of 100 cycles. Nothing on the bus fires when a frame ends, so
        // the wake predicate itself must cover the paced byte.
        let (mut u, clock, sink) = paced(10.0);
        assert!(!u.has_active_work() || u.intenset & INT_RXC != 0);
        u.write_u32(INTENCLR, 0xff).unwrap();
        u.write_u32(DATA, u32::from(b'A')).unwrap();
        assert!(u.has_active_work(), "held byte");
        assert!(!u.take_scheduled_events().is_empty());
        at(&mut u, &clock, 0);
        assert_eq!(u.pace_deadline(), Some(100.0), "shift register stop bit");
        assert!(u.has_active_work());
        at(&mut u, &clock, 100);
        assert_eq!(sink.lock().unwrap().as_slice(), b"A");
        assert_eq!(u.pace_deadline(), None);
        assert!(!u.has_active_work(), "idle once the frame ended");
    }

    #[test]
    fn frames_count_start_data_parity_and_stop_bits() {
        let (mut u, _, _) = paced(1.0);
        assert_eq!(u.frame_bits(), 10.0, "8N1");
        u.ctrla |= 1 << 24; // FORM: frame with parity
        u.ctrlb |= 1 << 6; // two stop bits
        assert_eq!(u.frame_bits(), 12.0);
    }

    #[test]
    fn transmit_holds_one_byte_behind_the_shift_register() {
        let (mut u, clock, sink) = paced(100.0); // 1000-cycle 8N1 frames
        u.write_u16(DATA, u16::from(b'A')).unwrap();
        assert_ne!(
            u.read(INTFLAG).unwrap() & INT_DRE,
            0,
            "A went straight to the shifter"
        );
        u.write_u16(DATA, u16::from(b'B')).unwrap();
        assert_eq!(u.read(INTFLAG).unwrap() & INT_DRE, 0, "B waits in DATA");
        u.write_u16(DATA, u16::from(b'C')).unwrap(); // ignored: DRE was clear
        at(&mut u, &clock, 999);
        assert!(sink.lock().unwrap().is_empty());
        at(&mut u, &clock, 1000);
        assert_eq!(*sink.lock().unwrap(), b"A");
        assert_ne!(u.read(INTFLAG).unwrap() & INT_DRE, 0);
        assert_eq!(u.read(INTFLAG).unwrap() & INT_TXC, 0, "B is still shifting");
        at(&mut u, &clock, 2000);
        assert_eq!(*sink.lock().unwrap(), b"AB");
        assert_ne!(
            u.read(INTFLAG).unwrap() & INT_TXC,
            0,
            "TXC after the last stop bit"
        );
    }

    #[test]
    fn host_bytes_arrive_a_frame_apart_and_overflow_the_two_byte_buffer() {
        let (mut u, clock, _) = paced(100.0);
        u.rx_buffer().lock().unwrap().extend(b"xyz");
        at(&mut u, &clock, 0);
        assert_eq!(
            u.read(INTFLAG).unwrap() & INT_RXC,
            0,
            "the first frame is on the wire"
        );
        at(&mut u, &clock, 1000);
        assert_ne!(u.read(INTFLAG).unwrap() & INT_RXC, 0);
        at(&mut u, &clock, 3000);
        assert_eq!(
            u.read_u16(STATUS).unwrap() & 4,
            4,
            "z found both buffer bytes full"
        );
        assert_eq!(u.read_u16(DATA).unwrap(), u16::from(b'x'));
        assert_eq!(u.read_u16(DATA).unwrap(), u16::from(b'y'));
        assert_eq!(u.read(INTFLAG).unwrap() & INT_RXC, 0);
    }

    #[test]
    fn an_unrouted_receiver_takes_nothing_off_the_wire() {
        let (mut u, clock, _) = paced(100.0);
        u.routed.1 = false;
        u.rx_buffer().lock().unwrap().push_back(b'x');
        at(&mut u, &clock, 5000);
        assert_eq!(u.read(INTFLAG).unwrap() & INT_RXC, 0);
        assert_eq!(u.rx_buffer().lock().unwrap().len(), 1);
    }
}
