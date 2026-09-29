// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! i.MX RT1060 LPUART (LPUART1..8, IMXRT1060RM §49).
//!
//! The RT LPUART carries VERID/PARAM/GLOBAL/PINCFG in front of the Kinetis
//! register block, so BAUD/STAT/CTRL/DATA sit at 0x10/0x14/0x18/0x1C (the
//! generic `Uart` model's Kinetis layout has them at 0x00..0x0C).
//!
//! Modelled: 4-entry TX and RX FIFOs (`PARAM` = 0x0202), `FIFO`/`WATER`
//! watermarks and counts, a transmitter that shifts one character per frame
//! time derived from `BAUD` (SBR/OSR) and the LPUART functional clock, `STAT`
//! TDRE/TC/RDRF/IDLE/OR with the write-1-to-clear flags, the level interrupt
//! (`CTRL` TIE/TCIE/RIE/ILIE/ORIE) and the TX/RX DMA requests (`BAUD`
//! TDMAE/RDMAE). Transmitted characters go to the console sink and the bus
//! trace; received characters are injected by the host (`push_rx`).
//!
//! External devices on the pins ([`UartStreamDevice`] peers, e.g. a Bluetooth
//! module) attach through [`UartStreamHost`]. A peer gets each transmitted
//! character when its stop bit ends, and its device time is brought forward to
//! that cycle first. What a peer sends goes onto the RX wire and takes one
//! frame time per character at the programmed baud rate. A character that
//! completes while the receiver is off (`CTRL.RE` = 0) is lost, as on the pin;
//! one that completes with the FIFO full sets `STAT.OR` and is lost.
//! Test-script injections (`rx_buffer`) wait until the receiver is on.

use super::{byte_of, Timebase};
use crate::bus::bus_trace::{BusDir, BusPayload, BusTrace};
use crate::peripherals::device::{UartStreamDevice, UartStreamHost};
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const VERID: u32 = 0x00;
const PARAM: u32 = 0x04;
const GLOBAL: u32 = 0x08;
const PINCFG: u32 = 0x0C;
const BAUD: u32 = 0x10;
const STAT: u32 = 0x14;
const CTRL: u32 = 0x18;
const DATA: u32 = 0x1C;
const MATCH: u32 = 0x20;
const MODIR: u32 = 0x24;
const FIFO: u32 = 0x28;
const WATER: u32 = 0x2C;

const FIFO_DEPTH: usize = 4;

/// Longest time between two polls of the attached peers, µs.
const PEER_POLL_US: u64 = 1_000;

// STAT
const ST_TDRE: u32 = 1 << 23;
const ST_TC: u32 = 1 << 22;
const ST_RDRF: u32 = 1 << 21;
const ST_IDLE: u32 = 1 << 20;
const ST_OR: u32 = 1 << 19;
/// Write-1-to-clear flags: LBKDIF, RXEDGIF, IDLE, OR, NF, FE, PF, MA1F, MA2F.
const ST_W1C: u32 = (1 << 31) | (1 << 30) | (0x1F << 16) | (1 << 15) | (1 << 14);
/// Plain read-write STAT bits: LBKDE, BRK13, RWUID, RXINV, MSBF.
const ST_RW: u32 = (1 << 25) | (1 << 26) | (1 << 27) | (1 << 28) | (1 << 29);

// CTRL
const CT_TIE: u32 = 1 << 23;
const CT_TCIE: u32 = 1 << 22;
const CT_RIE: u32 = 1 << 21;
const CT_ILIE: u32 = 1 << 20;
const CT_TE: u32 = 1 << 19;
const CT_RE: u32 = 1 << 18;
const CT_ORIE: u32 = 1 << 27;

// BAUD
const BD_TDMAE: u32 = 1 << 23;
const BD_RDMAE: u32 = 1 << 21;

// FIFO
const FF_RXFE: u32 = 1 << 3;
const FF_TXFE: u32 = 1 << 7;
const FF_RXFLUSH: u32 = 1 << 14;
const FF_TXFLUSH: u32 = 1 << 15;
const FF_RXEMPT: u32 = 1 << 22;
const FF_TXEMPT: u32 = 1 << 23;

/// LPUART functional clock the baud divider runs from when the chip YAML
/// does not say otherwise: PLL3 / 6 = 80 MHz (`CSCDR1.UART_CLK_SEL` = 0,
/// `UART_CLK_PODF` = 0), the MCUXpresso SDK board default.
pub const DEFAULT_UART_CLK_HZ: u64 = 80_000_000;

#[derive(Debug)]
struct Inner {
    baud: u32,
    stat: u32,
    ctrl: u32,
    matchr: u32,
    modir: u32,
    fifo: u32,
    water: u32,
    global: u32,
    pincfg: u32,
    tx_fifo: VecDeque<u8>,
    /// Character on the wire and the cycle its stop bit ends.
    shifting: Option<(u8, u64)>,
    rx_fifo: VecDeque<u8>,
    /// Last time the transmitter state was brought forward.
    synced: u64,
    /// Bytes completed but not yet handed to the sink (drained on `&mut`),
    /// with the cycle their stop bit ended.
    tx_done: Vec<(u8, u64)>,
    /// Characters a peer has sent that are not on the RX wire yet, with the
    /// cycle each was sent.
    rx_wire: VecDeque<(u8, u64)>,
    /// Character on the RX wire and the cycle its stop bit ends.
    rx_shifting: Option<(u8, u64)>,
    /// Characters that completed while the receiver was off or the FIFO was
    /// full.
    rx_lost: u64,
    /// Characters that completed into the RX FIFO, for the bus trace
    /// (drained on `&mut`).
    rx_done: Vec<u8>,
}

impl Inner {
    fn new() -> Self {
        Self {
            baud: 0x0F00_0004,
            stat: 0x00C0_0000,
            ctrl: 0,
            matchr: 0,
            modir: 0,
            fifo: 0x00C0_0011,
            water: 0,
            global: 0,
            pincfg: 0,
            tx_fifo: VecDeque::new(),
            shifting: None,
            rx_fifo: VecDeque::new(),
            synced: 0,
            tx_done: Vec::new(),
            rx_wire: VecDeque::new(),
            rx_shifting: None,
            rx_lost: 0,
            rx_done: Vec::new(),
        }
    }

    fn tx_depth(&self) -> usize {
        if self.fifo & FF_TXFE != 0 {
            FIFO_DEPTH
        } else {
            1
        }
    }
    fn rx_depth(&self) -> usize {
        if self.fifo & FF_RXFE != 0 {
            FIFO_DEPTH
        } else {
            1
        }
    }
    fn tdre(&self) -> bool {
        if self.fifo & FF_TXFE != 0 {
            self.tx_fifo.len() <= (self.water & 0x3) as usize
        } else {
            self.tx_fifo.is_empty()
        }
    }
    fn tc(&self) -> bool {
        self.tx_fifo.is_empty() && self.shifting.is_none()
    }
    fn rdrf(&self) -> bool {
        if self.fifo & FF_RXFE != 0 {
            self.rx_fifo.len() > ((self.water >> 16) & 0x3) as usize
        } else {
            !self.rx_fifo.is_empty()
        }
    }
    fn stat_view(&self) -> u32 {
        let mut s = self.stat & (ST_W1C | ST_RW);
        if self.tdre() {
            s |= ST_TDRE;
        }
        if self.tc() {
            s |= ST_TC;
        }
        if self.rdrf() {
            s |= ST_RDRF;
        }
        s
    }
    fn irq(&self) -> bool {
        let s = self.stat_view();
        (self.ctrl & CT_TIE != 0 && s & ST_TDRE != 0)
            || (self.ctrl & CT_TCIE != 0 && s & ST_TC != 0)
            || (self.ctrl & CT_RIE != 0 && s & ST_RDRF != 0)
            || (self.ctrl & CT_ILIE != 0 && s & ST_IDLE != 0)
            || (self.ctrl & CT_ORIE != 0 && s & ST_OR != 0)
    }
}

pub struct ImxrtLpuart {
    inner: RefCell<Inner>,
    time: Timebase,
    uart_clk_hz: u64,
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,
    trace: BusTrace,
    trace_name: String,
    /// Every byte this instance transmitted (for inspection and tests).
    tx_log: Vec<u8>,
    /// External devices on the TX/RX pins.
    streams: Vec<Box<dyn UartStreamDevice>>,
    /// The cycle up to which the peers' device time has been brought.
    peer_cycles: u64,
    /// Host injection queue (test scripts, interactive input). Its bytes go
    /// onto the RX wire when the receiver is on.
    rx_inject: Arc<Mutex<VecDeque<u8>>>,
}

impl std::fmt::Debug for ImxrtLpuart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImxrtLpuart")
            .field("inner", &self.inner)
            .field("uart_clk_hz", &self.uart_clk_hz)
            .field("trace_name", &self.trace_name)
            .field("tx_bytes", &self.tx_log.len())
            .field("streams", &self.streams.len())
            .finish()
    }
}

impl Default for ImxrtLpuart {
    fn default() -> Self {
        Self::new(DEFAULT_UART_CLK_HZ)
    }
}

impl ImxrtLpuart {
    pub fn new(uart_clk_hz: u64) -> Self {
        Self {
            inner: RefCell::new(Inner::new()),
            time: Timebase::default(),
            uart_clk_hz: uart_clk_hz.max(1),
            sink: None,
            echo_stdout: false,
            trace: BusTrace::default(),
            trace_name: "lpuart".into(),
            tx_log: Vec::new(),
            streams: Vec::new(),
            peer_cycles: 0,
            rx_inject: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// The host injection queue (see the module doc).
    pub fn rx_buffer(&self) -> Arc<Mutex<VecDeque<u8>>> {
        self.rx_inject.clone()
    }

    /// Attach an external device to the TX/RX pins.
    pub fn attach_stream(&mut self, dev: Box<dyn UartStreamDevice>) {
        self.peer_cycles = self.time.now();
        self.streams.push(dev);
    }

    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }

    /// Everything this LPUART has put on its TX line so far.
    pub fn tx_log(&self) -> &[u8] {
        &self.tx_log
    }

    /// Host -> device: a character arrives on RX. Returns false (overrun)
    /// when the receiver is off or the FIFO is full.
    pub fn push_rx(&mut self, byte: u8) -> bool {
        let mut i = self.inner.borrow_mut();
        if i.ctrl & CT_RE == 0 {
            return false;
        }
        if i.rx_fifo.len() >= i.rx_depth() {
            i.stat |= ST_OR;
            return false;
        }
        i.rx_fifo.push_back(byte);
        drop(i);
        self.refresh_irq();
        self.trace.push(
            &self.trace_name,
            BusPayload::Uart {
                direction: BusDir::Rx,
                byte,
            },
        );
        true
    }

    /// CPU cycles one character occupies on the wire:
    /// (start + 8/9/10 data + parity + stop) bits * (OSR+1) * SBR / clk.
    fn frame_cycles(&self, i: &Inner) -> u64 {
        let sbr = (i.baud & 0x1FFF).max(1) as u64;
        let osr = (((i.baud >> 24) & 0x1F) as u64).max(3) + 1;
        let data_bits = if i.ctrl & (1 << 4) != 0 { 9 } else { 8 };
        let parity = (i.ctrl >> 1) & 1;
        let stop = if i.baud & (1 << 13) != 0 { 2 } else { 1 };
        let bits = 1 + data_bits + parity as u64 + stop;
        let per_bit = osr * sbr * self.time.cpu_hz() / self.uart_clk_hz;
        (bits * per_bit).max(1)
    }

    /// Bring the transmitter forward to `now`.
    fn sync(&self) {
        let now = self.time.now();
        let mut i = self.inner.borrow_mut();
        let frame = self.frame_cycles(&i);
        loop {
            match i.shifting {
                Some((b, end)) if end <= now => {
                    i.tx_done.push((b, end));
                    i.shifting = None;
                    if let Some(next) = i.tx_fifo.pop_front() {
                        i.shifting = Some((next, end + frame));
                    }
                }
                None => {
                    if i.ctrl & CT_TE != 0 {
                        if let Some(next) = i.tx_fifo.pop_front() {
                            i.shifting = Some((next, now + frame));
                            continue;
                        }
                    }
                    break;
                }
                _ => break,
            }
        }
        // The receive side: injected bytes join the wire while RE is on.
        if i.ctrl & CT_RE != 0 {
            if let Ok(mut q) = self.rx_inject.try_lock() {
                while let Some(b) = q.pop_front() {
                    i.rx_wire.push_back((b, now));
                }
            }
        }
        loop {
            match i.rx_shifting {
                Some((b, end)) if end <= now => {
                    i.rx_shifting = None;
                    if i.ctrl & CT_RE == 0 {
                        i.rx_lost += 1;
                    } else if i.rx_fifo.len() >= i.rx_depth() {
                        i.stat |= ST_OR;
                        i.rx_lost += 1;
                    } else {
                        i.rx_fifo.push_back(b);
                        i.rx_done.push(b);
                    }
                    if let Some((next, sent)) = i.rx_wire.pop_front() {
                        i.rx_shifting = Some((next, end.max(sent) + frame));
                    }
                }
                None => match i.rx_wire.pop_front() {
                    Some((next, sent)) => i.rx_shifting = Some((next, sent + frame)),
                    None => break,
                },
                _ => break,
            }
        }
        i.synced = now;
    }

    /// Bring the peers' device time forward to `until` (a cycle) and put
    /// what they send onto the RX wire.
    fn service_peers(&mut self, until: u64) {
        if self.streams.is_empty() {
            self.peer_cycles = until;
            return;
        }
        let per_us = (self.time.cpu_hz() / 1_000_000).max(1);
        let us = until.saturating_sub(self.peer_cycles) / per_us;
        self.peer_cycles += us * per_us;
        let mut credit = us;
        let mut sent = Vec::new();
        loop {
            let step = credit.min(u64::from(u32::MAX)) as u32;
            credit -= u64::from(step);
            for stream in self.streams.iter_mut() {
                let mut elapsed = step;
                while let Some(b) = stream.poll(elapsed) {
                    sent.push(b);
                    elapsed = 0;
                }
            }
            if credit == 0 {
                break;
            }
        }
        if !sent.is_empty() {
            let mut i = self.inner.borrow_mut();
            let at = self.peer_cycles;
            i.rx_wire.extend(sent.into_iter().map(|b| (b, at)));
        }
    }

    fn flush_rx_done(&mut self) {
        let done = std::mem::take(&mut self.inner.borrow_mut().rx_done);
        for byte in done {
            self.trace.push(
                &self.trace_name,
                BusPayload::Uart {
                    direction: BusDir::Rx,
                    byte,
                },
            );
        }
    }

    fn flush_tx_done(&mut self) {
        let done = std::mem::take(&mut self.inner.borrow_mut().tx_done);
        for (b, end) in done {
            if !self.streams.is_empty() {
                self.service_peers(end);
                for stream in self.streams.iter_mut() {
                    stream.on_tx_byte(b);
                }
            }
            self.tx_log.push(b);
            if let Some(sink) = &self.sink {
                if let Ok(mut s) = sink.lock() {
                    s.push(b);
                }
            }
            if self.echo_stdout {
                use std::io::Write;
                let _ = std::io::stdout().write_all(&[b]);
            }
            self.trace.push(
                &self.trace_name,
                BusPayload::Uart {
                    direction: BusDir::Tx,
                    byte: b,
                },
            );
        }
    }

    pub fn read_reg(&self, off: u32) -> u32 {
        self.sync();
        let mut i = self.inner.borrow_mut();
        match off & !3 {
            VERID => 0x0401_0003,
            PARAM => 0x0000_0202,
            GLOBAL => i.global,
            PINCFG => i.pincfg,
            BAUD => i.baud,
            STAT => i.stat_view(),
            CTRL => i.ctrl,
            DATA => match i.rx_fifo.pop_front() {
                Some(b) => b as u32,
                None => 1 << 12, // RXEMPT
            },
            MATCH => i.matchr,
            MODIR => i.modir,
            FIFO => {
                let mut v = i.fifo & !(FF_RXEMPT | FF_TXEMPT);
                if i.rx_fifo.is_empty() {
                    v |= FF_RXEMPT;
                }
                if i.tx_fifo.is_empty() {
                    v |= FF_TXEMPT;
                }
                v
            }
            WATER => {
                (i.water & 0x0003_0003)
                    | ((i.tx_fifo.len() as u32) << 8)
                    | ((i.rx_fifo.len() as u32) << 24)
            }
            _ => 0,
        }
    }

    pub fn write_reg(&mut self, off: u32, value: u32, mask: u32) {
        self.sync();
        {
            let mut i = self.inner.borrow_mut();
            let v = value & mask;
            let merge = |old: u32| (old & !mask) | v;
            match off & !3 {
                GLOBAL => {
                    i.global = merge(i.global) & 0x2;
                    if i.global & 0x2 != 0 {
                        // Software reset: every register but GLOBAL to reset.
                        let g = i.global;
                        *i = Inner::new();
                        i.global = g;
                    }
                }
                PINCFG => i.pincfg = merge(i.pincfg) & 0x3,
                BAUD => i.baud = merge(i.baud),
                STAT => {
                    let clr = v & ST_W1C;
                    i.stat &= !clr;
                    i.stat = (i.stat & !(ST_RW & mask)) | (v & ST_RW);
                }
                CTRL => {
                    i.ctrl = merge(i.ctrl);
                    if i.ctrl & CT_TE == 0 {
                        // Transmitter off: the queue does not drain.
                    }
                }
                DATA if mask & 0xFF != 0 => {
                    let depth = i.tx_depth();
                    if i.tx_fifo.len() < depth {
                        i.tx_fifo.push_back(v as u8);
                    } else {
                        // TXOF: TX FIFO overflow.
                        i.fifo |= 1 << 17;
                    }
                }
                MATCH => i.matchr = merge(i.matchr),
                MODIR => i.modir = merge(i.modir),
                FIFO => {
                    // TXOF/RXUF are w1c; flush bits act and read as zero.
                    let w1c = v & ((1 << 16) | (1 << 17));
                    let keep =
                        merge(i.fifo) & !(FF_RXFLUSH | FF_TXFLUSH) & !((1 << 16) | (1 << 17));
                    i.fifo = keep | (i.fifo & ((1 << 16) | (1 << 17)) & !w1c);
                    if v & FF_TXFLUSH != 0 {
                        i.tx_fifo.clear();
                    }
                    if v & FF_RXFLUSH != 0 {
                        i.rx_fifo.clear();
                    }
                }
                WATER => i.water = merge(i.water) & 0x0003_0003,
                _ => {}
            }
        }
        self.sync();
        self.flush_tx_done();
        self.flush_rx_done();
    }

    /// DMA request line: 0 = TX (TDMAE & TDRE), 1 = RX (RDMAE & RDRF).
    fn dma_line(&self, line: u8) -> bool {
        self.sync();
        let i = self.inner.borrow();
        match line {
            0 => i.baud & BD_TDMAE != 0 && i.ctrl & CT_TE != 0 && i.tdre(),
            1 => i.baud & BD_RDMAE != 0 && i.rdrf(),
            _ => false,
        }
    }
}

impl ImxrtLpuart {
    /// Recompute the cached interrupt line (see `Timebase::level`).
    fn refresh_irq(&self) {
        self.time.set_level({
            self.sync();
            self.inner.borrow().irq()
        });
    }

    fn tick_inner(&mut self, cycles: u64) -> PeripheralTickResult {
        self.time.advance(cycles);
        self.sync();
        self.flush_tx_done();
        let now = self.time.now();
        self.service_peers(now);
        // What the peers just sent may already be due (a zero-delay reply).
        self.sync();
        self.flush_tx_done();
        self.flush_rx_done();
        let until = {
            let i = self.inner.borrow();
            let rx_next = match (i.rx_shifting, i.rx_wire.is_empty()) {
                (Some((_, end)), _) => Some(end),
                (None, false) => Some(now + 1),
                (None, true) => None,
            };
            let injected =
                i.ctrl & CT_RE != 0 && self.rx_inject.try_lock().is_ok_and(|q| !q.is_empty());
            let per_us = (self.time.cpu_hz() / 1_000_000).max(1);
            // A peer can get work between two ticks that its last
            // `next_wake_us` did not know about (a test-script stimulus, data
            // from its far side), and only the bus can move a wake-up. So a
            // host with peers wakes at least once per millisecond of device
            // time, the pace of the generic UART.
            let peer_next = (!self.streams.is_empty()).then(|| {
                let us = self
                    .streams
                    .iter()
                    .filter_map(|s| s.next_wake_us())
                    .min()
                    .unwrap_or(PEER_POLL_US)
                    .clamp(1, PEER_POLL_US);
                self.peer_cycles + us * per_us
            });
            [
                i.shifting.map(|(_, end)| end),
                rx_next,
                injected.then_some(now + 1),
                peer_next,
            ]
            .into_iter()
            .flatten()
            .min()
        };
        super::wake_hint(now, until)
    }
}

impl UartStreamHost for ImxrtLpuart {
    fn attach_stream_device(&mut self, dev: Box<dyn UartStreamDevice>) {
        self.attach_stream(dev);
    }

    fn detach_console_sink(&mut self) {
        self.set_sink(None, false);
    }

    fn hosts_protocol_peer(&self) -> bool {
        self.streams.iter().any(|s| s.carries_protocol_octets())
    }

    fn peer_ids(&self) -> Vec<String> {
        self.streams
            .iter()
            .filter_map(|s| s.device_id().map(str::to_string))
            .collect()
    }

    fn inject_peer_remote(&mut self, device: &str, bytes: &[u8]) -> Result<(), String> {
        crate::peripherals::device::inject_remote_into(&mut self.streams, device, bytes)
    }
}

impl Peripheral for ImxrtLpuart {
    /// Walked only while timed work is in flight or the interrupt line is
    /// asserted (so its deassert is reconciled); MMIO re-arms it.
    fn legacy_tick_active(&self) -> bool {
        ({
            let i = self.inner.borrow();
            i.shifting.is_some()
                || !i.tx_fifo.is_empty()
                || i.rx_shifting.is_some()
                || !i.rx_wire.is_empty()
        }) || self.time.level()
            || !self.streams.is_empty()
            || self.rx_inject.try_lock().is_ok_and(|q| !q.is_empty())
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Byte reads of DATA pop the FIFO once (the low byte).
        let v = byte_of(self.read_reg(offset as u32), offset);
        self.refresh_irq();
        Ok(v)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        self.write_reg(offset as u32, (value as u32) << shift, 0xFF << shift);
        self.refresh_irq();
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = self.read_reg(offset as u32);
        self.refresh_irq();
        Ok(v)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset as u32, value, u32::MAX);
        self.refresh_irq();
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        let off = offset as u32 & !3;
        if off == DATA {
            let i = self.inner.borrow();
            return Some(i.rx_fifo.front().copied().unwrap_or(0));
        }
        Some(byte_of(self.read_reg(off), offset))
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        let r = self.tick_inner(cycles);
        self.refresh_irq();
        r
    }
    fn dma_request_active(&self, line: u8) -> bool {
        self.dma_line(line)
    }
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.time.level())
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.time.attach_clock(clock);
    }
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.time.attach_cpu_hz(hz);
    }
    fn attach_bus_trace(&mut self, name: &str, trace: &BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.to_string();
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn as_uart_stream_host(&mut self) -> Option<&mut dyn UartStreamHost> {
        Some(self)
    }
    fn uart_rx_source(&self) -> Option<Arc<Mutex<VecDeque<u8>>>> {
        Some(self.rx_buffer())
    }
    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for stream in self.streams.iter_mut() {
            if let Some(si) = stream.as_sim_input_mut() {
                if f(si) {
                    return true;
                }
            }
        }
        false
    }
    /// The logs of the attached peers (the LPUART records none itself).
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        self.streams.iter().flat_map(|s| s.logs()).collect()
    }
    fn snapshot(&self) -> serde_json::Value {
        let i = self.inner.borrow();
        serde_json::json!({
            "peripheral": "imxrt_lpuart",
            "baud": i.baud,
            "ctrl": i.ctrl,
            "stat": i.stat_view(),
            "tx_bytes": self.tx_log.len(),
            "rx_lost": i.rx_lost,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CycleClock;

    fn uart() -> (ImxrtLpuart, CycleClock) {
        let mut u = ImxrtLpuart::new(DEFAULT_UART_CLK_HZ);
        let c = CycleClock::default();
        u.attach_cycle_clock(c.clone());
        (u, c)
    }

    #[test]
    fn transmit_takes_a_frame_time_then_sets_tc() {
        let (mut u, c) = uart();
        // 115200 baud at 80 MHz: OSR=15 (16x), SBR = 43.
        u.write_reg(BAUD, (15 << 24) | 43, u32::MAX);
        u.write_reg(CTRL, CT_TE | CT_RE, u32::MAX);
        assert_ne!(u.read_reg(STAT) & ST_TC, 0);
        u.write_reg(DATA, b'A' as u32, u32::MAX);
        assert_eq!(u.read_reg(STAT) & ST_TC, 0, "shifting");
        assert_ne!(u.read_reg(STAT) & ST_TDRE, 0, "buffer free again");
        let frame = 10 * 16 * 43 * 600 / 80;
        c.publish(frame as u64);
        assert_ne!(u.read_reg(STAT) & ST_TC, 0);
        u.write_reg(CTRL, CT_TE | CT_RE, u32::MAX);
        assert_eq!(u.tx_log(), b"A");
    }

    /// A peer that answers every byte with the byte + 1 after 100 µs, and
    /// records the device time each byte arrived.
    struct Plus1 {
        now_us: u64,
        seen: std::sync::Arc<std::sync::Mutex<Vec<(u8, u64)>>>,
        due: VecDeque<(u64, u8)>,
        out: VecDeque<u8>,
    }

    impl UartStreamDevice for Plus1 {
        fn poll(&mut self, elapsed_us: u32) -> Option<u8> {
            self.now_us += u64::from(elapsed_us);
            while self.due.front().is_some_and(|(t, _)| *t <= self.now_us) {
                let (_, b) = self.due.pop_front().unwrap();
                self.out.push_back(b);
            }
            self.out.pop_front()
        }
        fn on_tx_byte(&mut self, byte: u8) {
            self.seen.lock().unwrap().push((byte, self.now_us));
            self.due
                .push_back((self.now_us + 100, byte.wrapping_add(1)));
        }
        fn next_wake_us(&self) -> Option<u64> {
            self.due
                .front()
                .map(|(t, _)| t.saturating_sub(self.now_us).max(1))
        }
        fn device_id(&self) -> Option<&str> {
            Some("plus1")
        }
    }

    type Seen = std::sync::Arc<std::sync::Mutex<Vec<(u8, u64)>>>;

    /// 115200 baud LPUART at 600 MHz with a `Plus1` peer; returns the frame
    /// time in cycles.
    fn with_peer() -> (ImxrtLpuart, CycleClock, Seen, u64) {
        let (mut u, c) = uart();
        let seen: Seen = Default::default();
        u.attach_stream(Box::new(Plus1 {
            now_us: 0,
            seen: seen.clone(),
            due: VecDeque::new(),
            out: VecDeque::new(),
        }));
        u.write_reg(BAUD, (15 << 24) | 43, u32::MAX);
        u.write_reg(CTRL, CT_TE | CT_RE, u32::MAX);
        (u, c, seen, 10 * 16 * 43 * 600 / 80)
    }

    /// Tick the model up to cycle `to` the way the bus does, following its
    /// wake hints.
    fn tick_to(u: &mut ImxrtLpuart, c: &CycleClock, to: u64) {
        loop {
            let now = c.now();
            if now >= to {
                break;
            }
            let hint = u.tick_elapsed(0).ticks_until_next.unwrap_or(1);
            c.publish((now + hint).min(to));
        }
        u.tick_elapsed(0);
    }

    #[test]
    fn peer_gets_tx_bytes_at_stop_bit_and_answers_at_baud_rate() {
        let (mut u, c, seen, frame) = with_peer();
        u.write_reg(DATA, 0x41, u32::MAX);
        tick_to(&mut u, &c, frame - 1);
        assert!(seen.lock().unwrap().is_empty(), "not before the stop bit");
        tick_to(&mut u, &c, frame + 10);
        let got = seen.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, 0x41);
        // Device time at delivery = the stop-bit cycle, in µs.
        assert_eq!(got[0].1, frame / 600);
        // Reply: 100 µs later, then one frame on the RX wire.
        let reply_done = (got[0].1 + 100) * 600 + frame;
        tick_to(&mut u, &c, reply_done - 600);
        assert_eq!(u.read_reg(DATA), 1 << 12, "still on the wire");
        tick_to(&mut u, &c, reply_done + 600);
        assert_eq!(u.read_reg(DATA), 0x42);
        assert_eq!(u.peer_ids(), vec!["plus1".to_string()]);
    }

    /// Work a peer gets between ticks (a stimulus, far-side data) must not
    /// wait for the wake-up the peer asked for before it had that work.
    #[test]
    fn a_peer_is_polled_at_least_every_millisecond() {
        let (mut u, _c, _, _) = with_peer();
        let hint = u.tick_elapsed(0).ticks_until_next.unwrap();
        assert!(hint <= 600_000, "hint {hint} cycles at 600 MHz");
    }

    #[test]
    fn peer_bytes_are_lost_while_the_receiver_is_off() {
        let (mut u, c, _, frame) = with_peer();
        u.write_reg(DATA, 0x10, u32::MAX);
        tick_to(&mut u, &c, frame + 10);
        u.write_reg(CTRL, CT_TE, u32::MAX);
        tick_to(&mut u, &c, frame * 3 + 200 * 600);
        u.write_reg(CTRL, CT_TE | CT_RE, u32::MAX);
        assert_eq!(u.read_reg(DATA), 1 << 12);
        assert_eq!(u.snapshot()["rx_lost"], 1);
    }

    #[test]
    fn injected_bytes_wait_for_the_receiver() {
        let (mut u, c) = uart();
        u.write_reg(BAUD, (15 << 24) | 43, u32::MAX);
        // RX FIFO on (4 deep), so both characters fit without a read.
        u.write_reg(FIFO, FF_RXFE, u32::MAX);
        u.rx_buffer().lock().unwrap().extend([1u8, 2]);
        tick_to(&mut u, &c, 1_000_000);
        assert_eq!(u.read_reg(DATA), 1 << 12, "RE off: held");
        u.write_reg(CTRL, CT_RE, u32::MAX);
        tick_to(&mut u, &c, 2_000_000);
        assert_eq!(u.read_reg(DATA), 1);
        assert_eq!(u.read_reg(DATA), 2);
    }

    #[test]
    fn a_full_fifo_sets_overrun() {
        let (mut u, c) = uart();
        u.write_reg(BAUD, (15 << 24) | 43, u32::MAX);
        u.write_reg(CTRL, CT_RE, u32::MAX);
        // FIFO off: depth 1.
        u.rx_buffer().lock().unwrap().extend([1u8, 2, 3]);
        tick_to(&mut u, &c, 1_000_000);
        assert_ne!(u.read_reg(STAT) & ST_OR, 0);
        assert_eq!(u.read_reg(DATA), 1);
    }

    #[test]
    fn rx_fifo_and_rxempt() {
        let (mut u, _) = uart();
        u.write_u32(CTRL as u64, CT_RE | CT_RIE).unwrap();
        assert_eq!(u.read_reg(DATA), 1 << 12);
        assert!(!u.irq_line_level().unwrap());
        assert!(u.push_rx(0x42));
        assert!(u.irq_line_level().unwrap());
        assert_ne!(u.read_reg(STAT) & ST_RDRF, 0);
        assert_eq!(u.read_u32(DATA as u64).unwrap(), 0x42);
        assert!(!u.irq_line_level().unwrap());
    }
}
