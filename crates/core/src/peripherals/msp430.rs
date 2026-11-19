// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Texas Instruments MSP430x1xx and MSP430x2xx on-chip modules (family user's
//! guides SLAU049 and SLAU144) and the interrupt sources they share with the
//! [`crate::cpu::Msp430`] core.
//!
//! * [`Msp430Irq`]: the interrupt sources. Each has a flag (IFG) and an
//!   enable (IE) and is routed to one request line; lines are numbered by
//!   priority from the top of the vector table (line `n` vectors through
//!   `0xFFFE - 2n`). The core accepts the highest-priority line with a
//!   source whose flag and enable are both set, and acceptance clears the
//!   flag of a source that is alone on its line, as silicon does.
//! * [`Msp430Sfr`] (`msp430_sfr`): the special function registers IE1, IE2,
//!   IFG1, IFG2, ME1, ME2 at 0x0000-0x0005 and UC1IE, UC1IFG at 0x0006-0x0007.
//!   A descriptor maps each interrupt bit to its line; unmapped bits are
//!   storage.
//! * [`Msp430Usart`] (`msp430_usart`, `msp430_usci_a`): the x1xx USART and
//!   the x2xx USCI_A in UART mode. A byte written to the transmit buffer
//!   leaves at once (the shift register is always empty), so the transmit
//!   flag is set again by the same write. Received bytes move from the host
//!   queue into the receive buffer one character time apart (ten bit times of
//!   the BR1:BR0 divider, counted in CPU cycles) and wait there until
//!   firmware reads them; receive errors are never set. The loopback bit
//!   (`LISTEN`, `UCLISTEN`) feeds transmitted bytes back to the receiver.
//!   The modulation registers, the clock source select, SPI mode, the
//!   multiprocessor modes, automatic baud rate detection and IrDA are storage.
//! * [`Msp430Wdt`] (`msp430_wdt`): WDTCTL with its password. A write without
//!   0x5A in the upper byte requests a reset (PUC). The counter does not run:
//!   firmware that leaves the watchdog active is not reset by a time-out.
//! * [`Msp430Mpy`] (`msp430_mpy`): the 16×16 hardware multiplier (MPY, MPYS,
//!   MAC, MACS, OP2, RESLO, RESHI, SUMEXT).
//! * [`Msp430Bcm`] (`msp430_bcm`, `msp430_bcm_plus`, `msp430_bcsctl3`): the
//!   basic clock module registers as storage with their reset values. No
//!   clock frequency is derived from them.

use crate::bus::bus_trace::{BusDir, BusPayload, BusTrace};
use crate::peripheral::PeripheralResetCause;
use crate::peripherals::device::{UartStreamDevice, UartStreamHost};
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

// ── Interrupt request lines ───────────────────────────────────────────────

/// Interrupt sources per chip. Sources 0-7 are the bits of IE1/IFG1, 8-15
/// the bits of IE2/IFG2 and 16-23 the bits of UC1IE/UC1IFG; modules with
/// their own flag registers use the rest.
pub const SOURCES: u32 = 64;

/// The source behind bit `bit` of IFG1 (`reg` 1), IFG2 (`reg` 2) or UC1IFG
/// (`reg` 3).
pub const fn sfr_source(reg: u32, bit: u32) -> u32 {
    (match reg {
        2 => 8,
        3 => 16,
        _ => 0,
    }) + (bit & 7)
}

/// The interrupt sources of one MSP430, shared between the core and the
/// modules that own the flags. Each source has a flag (IFG) and an enable
/// (IE) and is routed to one request line.
#[derive(Debug)]
pub struct Msp430Irq {
    flag: AtomicU64,
    enable: AtomicU64,
    /// Sources whose flag the core clears when it accepts their line.
    auto_clear: AtomicU64,
    line: [AtomicU8; SOURCES as usize],
    /// ME1, ME2.
    module_enable: [AtomicU8; 2],
}

impl Default for Msp430Irq {
    fn default() -> Self {
        Self {
            flag: AtomicU64::new(0),
            enable: AtomicU64::new(0),
            auto_clear: AtomicU64::new(0),
            line: std::array::from_fn(|_| AtomicU8::new(0)),
            module_enable: [AtomicU8::new(0), AtomicU8::new(0)],
        }
    }
}

impl Msp430Irq {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn put(cell: &AtomicU64, source: u32, on: bool) {
        if source >= SOURCES {
            return;
        }
        if on {
            cell.fetch_or(1 << source, Ordering::Relaxed);
        } else {
            cell.fetch_and(!(1 << source), Ordering::Relaxed);
        }
    }

    fn get(cell: &AtomicU64, source: u32) -> bool {
        source < SOURCES && cell.load(Ordering::Relaxed) >> source & 1 != 0
    }

    /// Route `source` to request line `line` (vector `0xFFFE - 2 * line`).
    /// `auto_clear`: the line has no other source, so accepting it clears
    /// the flag, as silicon does for a single-source interrupt.
    pub fn route(&self, source: u32, line: u32, auto_clear: bool) {
        if source < SOURCES && line < 64 {
            self.line[source as usize].store(line as u8, Ordering::Relaxed);
            Self::put(&self.auto_clear, source, auto_clear);
        }
    }

    pub fn set_flag(&self, source: u32, on: bool) {
        Self::put(&self.flag, source, on);
    }

    pub fn flag(&self, source: u32) -> bool {
        Self::get(&self.flag, source)
    }

    pub fn set_enable(&self, source: u32, on: bool) {
        Self::put(&self.enable, source, on);
    }

    pub fn enabled(&self, source: u32) -> bool {
        Self::get(&self.enable, source)
    }

    fn requesting(&self) -> u64 {
        self.flag.load(Ordering::Relaxed) & self.enable.load(Ordering::Relaxed)
    }

    fn line_of(&self, source: u32) -> u32 {
        u32::from(self.line[source as usize].load(Ordering::Relaxed))
    }

    /// Lines requesting service, one bit per line.
    pub fn pending(&self) -> u64 {
        let mut sources = self.requesting();
        let mut lines = 0;
        while sources != 0 {
            lines |= 1 << self.line_of(sources.trailing_zeros());
            sources &= sources - 1;
        }
        lines
    }

    /// The core accepted `line`: single-source flags on it are cleared.
    /// Accepting the NMI line clears every NMI enable (NMIIE, OFIE, ACCVIE;
    /// SLAU049 2.2.1), which the handler sets again.
    pub fn accept(&self, line: u32) {
        let requesting = self.requesting();
        for source in (0..SOURCES).filter(|s| self.line_of(*s) == line) {
            if requesting >> source & 1 != 0 && Self::get(&self.auto_clear, source) {
                self.set_flag(source, false);
            }
            if line == LINE_NMI {
                self.set_enable(source, false);
            }
        }
    }

    /// Bit `bit` of ME1 (`reg` 1) or ME2 (`reg` 2).
    pub fn module_enabled(&self, reg: u32, bit: u32) -> bool {
        let cell = &self.module_enable[(reg.clamp(1, 2) - 1) as usize];
        cell.load(Ordering::Relaxed) >> (bit & 7) & 1 != 0
    }
}

/// The non-maskable interrupt line (vector 0xFFFC).
pub const LINE_NMI: u32 = 1;

// ── Special function registers ────────────────────────────────────────────

const SFR_ME1: u64 = 4;
const SFR_UC1IE: u64 = 6;
const SFR_UC1IFG: u64 = 7;
/// OFIFG: set by a power-on reset.
const IFG1_RESET: u8 = 0x02;

/// IE1/IE2/IFG1/IFG2/ME1/ME2 and, on parts with a second USCI, UC1IE/UC1IFG.
#[derive(Debug)]
pub struct Msp430Sfr {
    irq: Arc<Msp430Irq>,
    /// The line behind each bit of IE1/IFG1, IE2/IFG2 and UC1IE/UC1IFG
    /// (0 = not an interrupt bit of this part).
    lines: [[u32; 8]; 3],
    /// Bits no line is mapped to, per register pair: enable byte, flag byte.
    stored: [[u8; 2]; 3],
}

/// The register pair (0 = IE1/IFG1, 1 = IE2/IFG2, 2 = UC1IE/UC1IFG) and
/// whether `offset` is its flag byte.
fn sfr_pair(offset: u64) -> Option<(usize, bool)> {
    match offset {
        0..=3 => Some(((offset & 1) as usize, offset >= 2)),
        SFR_UC1IE => Some((2, false)),
        SFR_UC1IFG => Some((2, true)),
        _ => None,
    }
}

impl Msp430Sfr {
    pub fn new(lines: [[u32; 8]; 3]) -> Self {
        let sfr = Self {
            irq: Msp430Irq::new(),
            lines,
            stored: [[0, IFG1_RESET], [0, 0], [0, 0]],
        };
        sfr.route();
        sfr
    }

    /// Route every mapped bit; a bit that is alone on its line is a
    /// single-source interrupt.
    fn route(&self) {
        let all = || self.lines.iter().flatten().filter(|l| **l != 0);
        for pair in 0..3 {
            for bit in 0..8 {
                let line = self.lines[pair][bit];
                if line != 0 {
                    let alone = all().filter(|l| **l == line).count() == 1;
                    self.irq
                        .route(sfr_source(pair as u32 + 1, bit as u32), line, alone);
                }
            }
        }
    }

    fn mapped(&self, pair: usize) -> u8 {
        (0..8).fold(0, |m, bit| m | u8::from(self.lines[pair][bit] != 0) << bit)
    }

    fn source_bits(&self, pair: usize, flags: bool) -> u8 {
        (0..8).fold(0, |v, bit| {
            let source = sfr_source(pair as u32 + 1, bit);
            let on = if flags {
                self.irq.flag(source)
            } else {
                self.irq.enabled(source)
            };
            v | u8::from(on) << bit
        })
    }
}

impl Peripheral for Msp430Sfr {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(match (sfr_pair(offset), offset) {
            (Some((pair, flags)), _) => {
                let mapped = self.mapped(pair);
                (self.stored[pair][usize::from(flags)] & !mapped)
                    | (self.source_bits(pair, flags) & mapped)
            }
            (None, SFR_ME1 | 5) => {
                self.irq.module_enable[(offset - SFR_ME1) as usize].load(Ordering::Relaxed)
            }
            _ => 0,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match (sfr_pair(offset), offset) {
            (Some((pair, flags)), _) => {
                self.stored[pair][usize::from(flags)] = value;
                for bit in (0..8).filter(|b| self.lines[pair][*b as usize] != 0) {
                    let source = sfr_source(pair as u32 + 1, bit);
                    let on = value >> bit & 1 != 0;
                    if flags {
                        self.irq.set_flag(source, on);
                    } else {
                        self.irq.set_enable(source, on);
                    }
                }
            }
            (None, SFR_ME1 | 5) => {
                self.irq.module_enable[(offset - SFR_ME1) as usize].store(value, Ordering::Relaxed)
            }
            _ => {}
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn attach_msp430_irq(&mut self, irq: &Arc<Msp430Irq>) {
        self.irq = irq.clone();
        self.route();
    }

    fn on_system_reset(&mut self) {
        // A PUC clears the enables; the module flags belong to their
        // modules, which reset them.
        for pair in 0..3 {
            self.stored[pair][0] = 0;
            for bit in (0..8).filter(|b| self.lines[pair][*b as usize] != 0) {
                self.irq.set_enable(sfr_source(pair as u32 + 1, bit), false);
            }
        }
        for me in &self.irq.module_enable {
            me.store(0, Ordering::Relaxed);
        }
    }
}

// ── USART and USCI_A (UART mode) ──────────────────────────────────────────

const CONSOLE: usize = 64 * 1024;
const RX_TOKEN: u32 = 1;
const MAX_REGS: usize = 9;

/// Where a UART module keeps the bits this model acts on.
#[derive(Debug)]
struct Layout {
    /// Control and status registers before the two data buffers.
    regs: usize,
    reset: [u8; MAX_REGS],
    /// Register index and mask of the software reset bit.
    swrst: (usize, u8),
    /// Register index and mask of the loopback bit.
    listen: (usize, u8),
    /// Register index of the low divider byte; the high byte follows.
    divider: usize,
    /// A register whose `set` bits always read 1 and `clear` bits 0.
    forced: (usize, u8, u8),
    /// Receive and transmit need their ME bits.
    module_enable: bool,
}

/// MSP430x1xx USART: UxCTL, UxTCTL, UxRCTL, UxMCTL, UxBR0, UxBR1, UxRXBUF,
/// UxTXBUF. TXEPT always reads 1; the receive error flags are never set.
const USART: Layout = Layout {
    regs: 6,
    reset: [0x01, 0x01, 0, 0, 0, 0, 0, 0, 0],
    swrst: (0, 0x01),
    listen: (0, 0x08),
    divider: 4,
    forced: (1, 0x01, 0),
    module_enable: true,
};
/// The USART receive error flags in UxRCTL (FE, PE, OE, BRK, RXERR).
const USART_RCTL: (usize, u8) = (2, 0xF1);

/// MSP430x2xx USCI_A: UCAxABCTL, UCAxIRTCTL, UCAxIRRCTL, UCAxCTL0, UCAxCTL1,
/// UCAxBR0, UCAxBR1, UCAxMCTL, UCAxSTAT, UCAxRXBUF, UCAxTXBUF. UCAxSTAT keeps
/// UCLISTEN; its error and busy flags are never set.
const USCI_A: Layout = Layout {
    regs: 9,
    reset: [0, 0, 0, 0, 0x01, 0, 0, 0, 0],
    swrst: (4, 0x01),
    listen: (8, 0x80),
    divider: 5,
    forced: (8, 0, 0x7F),
    module_enable: false,
};

struct UsartState {
    regs: [u8; MAX_REGS],
    rxbuf: u8,
    rx_full: bool,
    rx_live: bool,
    /// Legacy tick: CPU cycles until the byte on the line has arrived.
    rx_wait: Option<u64>,
    pending_events: Vec<(u64, u32)>,
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo: bool,
    streams: Vec<Box<dyn UartStreamDevice>>,
}

/// An MSP430 UART: the x1xx USART or the x2xx USCI_A, in UART mode.
pub struct Msp430Usart {
    layout: &'static Layout,
    state: Mutex<UsartState>,
    rx: Arc<Mutex<VecDeque<u8>>>,
    irq: Arc<Msp430Irq>,
    /// Interrupt sources: receive flag, transmit flag.
    sources: [u32; 2],
    /// Module enable bits: register (1 = ME1, 2 = ME2), URXE bit, UTXE bit.
    me: [u32; 3],
    trace: BusTrace,
    trace_name: String,
    clock: Option<CycleClock>,
}

impl std::fmt::Debug for Msp430Usart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Msp430Usart")
            .field("sources", &self.sources)
            .finish_non_exhaustive()
    }
}

impl Msp430Usart {
    /// An x1xx USART. `flags`: the IFG register (1 or 2) and its URXIFG and
    /// UTXIFG bits; `me`: the ME register (1 or 2) and its URXE and UTXE bits.
    pub fn new(flags: [u32; 3], me: [u32; 3]) -> Self {
        Self::with_layout(&USART, flags, me)
    }

    /// An x2xx USCI_A. `flags`: the flag register (2 = IFG2, 3 = UC1IFG) and
    /// its UCAxRXIFG and UCAxTXIFG bits.
    pub fn usci_a(flags: [u32; 3]) -> Self {
        Self::with_layout(&USCI_A, flags, [0; 3])
    }

    fn with_layout(layout: &'static Layout, flags: [u32; 3], me: [u32; 3]) -> Self {
        let usart = Self {
            layout,
            state: Mutex::new(UsartState {
                regs: layout.reset,
                rxbuf: 0,
                rx_full: false,
                rx_live: false,
                rx_wait: None,
                pending_events: Vec::new(),
                sink: None,
                echo: false,
                streams: Vec::new(),
            }),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            irq: Msp430Irq::new(),
            sources: [
                sfr_source(flags[0], flags[1]),
                sfr_source(flags[0], flags[2]),
            ],
            me,
            trace: crate::bus::bus_trace::new_log(),
            trace_name: String::new(),
            clock: None,
        };
        usart.reset_flags();
        usart
    }

    crate::cycle_clock::scheduler_mode!();

    fn rxbuf_offset(&self) -> u64 {
        self.layout.regs as u64
    }

    fn txbuf_offset(&self) -> u64 {
        self.layout.regs as u64 + 1
    }

    /// The transmit flag is set and the receive flag cleared by a PUC and
    /// while the software reset bit is set.
    fn reset_flags(&self) {
        self.irq.set_flag(self.sources[0], false);
        self.irq.set_flag(self.sources[1], true);
    }

    fn running(&self, s: &UsartState) -> bool {
        s.regs[self.layout.swrst.0] & self.layout.swrst.1 == 0
    }

    fn rx_enabled(&self, s: &UsartState) -> bool {
        self.running(s)
            && (!self.layout.module_enable || self.irq.module_enabled(self.me[0], self.me[1]))
    }

    fn tx_enabled(&self, s: &UsartState) -> bool {
        self.running(s)
            && (!self.layout.module_enable || self.irq.module_enabled(self.me[0], self.me[2]))
    }

    /// CPU cycles in one character: start, eight data and one stop bit.
    fn char_cycles(&self, s: &UsartState) -> u64 {
        let low = self.layout.divider;
        let divider = u64::from(s.regs[low + 1]) << 8 | u64::from(s.regs[low]);
        (10 * divider).max(1)
    }

    /// Move the next received byte into an empty receive buffer.
    fn receive(&self, s: &mut UsartState) {
        if !self.rx_enabled(s) || s.rx_full {
            return;
        }
        if let Some(byte) = self.rx.lock().unwrap().pop_front() {
            s.rxbuf = byte;
            s.rx_full = true;
            self.irq.set_flag(self.sources[0], true);
        }
    }

    /// Keep one receive wake live while the receiver is on.
    fn want_wake(&self, s: &mut UsartState) {
        if self.scheduler_mode() && self.rx_enabled(s) && !s.rx_live {
            s.rx_live = true;
            let waiting = !s.rx_full && !self.rx.lock().unwrap().is_empty();
            let delay = if waiting { 1 } else { self.char_cycles(s) };
            s.pending_events.push((delay, RX_TOKEN));
        }
    }

    fn transmit(&self, s: &mut UsartState, v: u8) {
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
        if s.regs[self.layout.listen.0] & self.layout.listen.1 != 0 {
            self.rx.lock().unwrap().push_back(v);
        }
    }

    fn store(&self, s: &mut UsartState, index: usize, value: u8) {
        let (forced, set, clear) = self.layout.forced;
        let mut v = value;
        if index == forced {
            v = (v | set) & !clear;
        }
        if self.layout.module_enable && index == USART_RCTL.0 {
            v &= !USART_RCTL.1;
        }
        s.regs[index] = v;
        if index == self.layout.swrst.0 && v & self.layout.swrst.1 != 0 {
            s.rx_full = false;
            self.reset_flags();
        }
    }
}

impl UartStreamHost for Msp430Usart {
    fn attach_stream_device(&mut self, dev: Box<dyn UartStreamDevice>) {
        assert!(
            dev.stream_generation().is_none(),
            "MSP430 USART has no exclusive protocol attachment"
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

impl Peripheral for Msp430Usart {
    fn attach_bus_trace(&mut self, name: &str, trace: &BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.into();
    }

    fn bus_trace_handle(&self) -> Option<BusTrace> {
        Some(self.trace.clone())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let s = self.state.lock().unwrap();
        Some(match offset {
            o if (o as usize) < self.layout.regs => s.regs[o as usize],
            o if o == self.rxbuf_offset() => s.rxbuf,
            _ => 0,
        })
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

    fn read(&self, offset: u64) -> SimResult<u8> {
        let mut s = self.state.lock().unwrap();
        Ok(match offset {
            o if (o as usize) < self.layout.regs => s.regs[o as usize],
            o if o == self.rxbuf_offset() => {
                // Reading the receive buffer clears the receive flag.
                s.rx_full = false;
                self.irq.set_flag(self.sources[0], false);
                s.rxbuf
            }
            _ => 0,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let mut s = self.state.lock().unwrap();
        match offset {
            o if (o as usize) < self.layout.regs => self.store(&mut s, o as usize, value),
            o if o == self.txbuf_offset() && self.tx_enabled(&s) => {
                self.transmit(&mut s, value);
                // The byte is gone at once, so the buffer is empty again.
                self.irq.set_flag(self.sources[1], true);
            }
            _ => {}
        }
        self.want_wake(&mut s);
        Ok(())
    }

    fn legacy_tick_active(&self) -> bool {
        // Host bytes can arrive without a bus call: tick while receiving.
        !self.scheduler_mode() && self.rx_enabled(&self.state.lock().unwrap())
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_elapsed(1)
    }

    /// Without the scheduler the walk counts the character time.
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        let mut s = self.state.lock().unwrap();
        if !self.rx_enabled(&s) || s.rx_full || self.rx.lock().unwrap().is_empty() {
            s.rx_wait = None;
            return PeripheralTickResult::default();
        }
        let left = s
            .rx_wait
            .unwrap_or_else(|| self.char_cycles(&s))
            .saturating_sub(cycles);
        if left == 0 {
            s.rx_wait = None;
            self.receive(&mut s);
        } else {
            s.rx_wait = Some(left);
        }
        PeripheralTickResult::default()
    }

    /// Also called after host bytes land in the RX queue.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        let mut s = self.state.lock().unwrap();
        self.want_wake(&mut s);
        std::mem::take(&mut s.pending_events)
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let mut s = self.state.lock().unwrap();
        self.receive(&mut s);
        // The receiver stays polled, one character apart, while it is on.
        let again = self.rx_enabled(&s);
        s.rx_live = again;
        crate::sched::EventResult {
            reschedule_delay: again.then(|| self.char_cycles(&s)),
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn attach_msp430_irq(&mut self, irq: &Arc<Msp430Irq>) {
        self.irq = irq.clone();
        self.reset_flags();
    }

    fn on_system_reset(&mut self) {
        let mut s = self.state.lock().unwrap();
        s.regs = self.layout.reset;
        s.rx_full = false;
        self.reset_flags();
    }

    fn as_uart_stream_host(&mut self) -> Option<&mut dyn UartStreamHost> {
        Some(self)
    }

    fn uart_rx_source(&self) -> Option<Arc<Mutex<VecDeque<u8>>>> {
        Some(self.rx.clone())
    }
}

// ── Watchdog timer ────────────────────────────────────────────────────────

const WDT_PASSWORD: u16 = 0x5A00;
const WDT_READ_KEY: u16 = 0x6900;
/// WDTCNTCL: clears the counter and reads as zero.
const WDT_CNTCL: u16 = 1 << 3;

/// WDTCTL.
#[derive(Debug, Default)]
pub struct Msp430Wdt {
    ctl: u8,
    reset_latched: bool,
    irq: Option<Arc<Msp430Irq>>,
}

/// WDTIFG is IFG1.0.
const WDTIFG: u32 = sfr_source(1, 0);

impl Msp430Wdt {
    pub fn new() -> Self {
        Self::default()
    }

    /// A security key violation sets WDTIFG and requests a PUC.
    fn violation(&mut self) {
        self.reset_latched = true;
        if let Some(irq) = &self.irq {
            irq.set_flag(WDTIFG, true);
        }
    }
}

impl Peripheral for Msp430Wdt {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(match offset {
            0 => self.ctl,
            1 => (WDT_READ_KEY >> 8) as u8,
            _ => 0,
        })
    }

    /// WDTCTL is a word register: a byte store cannot carry the password.
    fn write(&mut self, offset: u64, _value: u8) -> SimResult<()> {
        if offset < 2 {
            self.violation();
        }
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(if offset == 0 {
            WDT_READ_KEY | u16::from(self.ctl)
        } else {
            0
        })
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if offset != 0 {
            return Ok(());
        }
        if value & 0xFF00 == WDT_PASSWORD {
            self.ctl = (value & !WDT_CNTCL) as u8;
        } else {
            self.violation();
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn reset_request_capable(&self) -> bool {
        true
    }

    fn latched_reset_cause(&self) -> Option<PeripheralResetCause> {
        self.reset_latched.then_some(PeripheralResetCause::Watchdog)
    }

    fn take_reset_request(&mut self) -> bool {
        std::mem::take(&mut self.reset_latched)
    }

    fn on_system_reset(&mut self) {
        self.ctl = 0;
    }

    fn attach_msp430_irq(&mut self, irq: &Arc<Msp430Irq>) {
        self.irq = Some(irq.clone());
    }
}

// ── Hardware multiplier ───────────────────────────────────────────────────

const MPY_OP2: u64 = 0x8;
const MPY_RESLO: u64 = 0xA;
const MPY_RESHI: u64 = 0xC;
const MPY_SUMEXT: u64 = 0xE;

/// The 16×16 hardware multiplier. Writing the first operand to MPY, MPYS,
/// MAC or MACS selects the operation; writing OP2 runs it.
#[derive(Debug, Default)]
pub struct Msp430Mpy {
    op1: u16,
    op2: u16,
    /// 0 = MPY, 1 = MPYS, 2 = MAC, 3 = MACS.
    mode: u8,
    result: u32,
    sumext: u16,
}

impl Msp430Mpy {
    pub fn new() -> Self {
        Self::default()
    }

    fn run(&mut self) {
        let signed = self.mode & 1 != 0;
        let product = if signed {
            (i32::from(self.op1 as i16) * i32::from(self.op2 as i16)) as u32
        } else {
            u32::from(self.op1) * u32::from(self.op2)
        };
        let (result, carry) = if self.mode & 2 != 0 {
            self.result.overflowing_add(product)
        } else {
            (product, false)
        };
        self.result = result;
        self.sumext = match self.mode {
            0 => 0,
            2 => u16::from(carry),
            // Signed: the sign of the result, extended.
            _ if result & 0x8000_0000 != 0 => 0xFFFF,
            _ => 0,
        };
    }

    fn word(&self, offset: u64) -> u16 {
        match offset & !1 {
            0x0..=0x6 => self.op1,
            MPY_OP2 => self.op2,
            MPY_RESLO => self.result as u16,
            MPY_RESHI => (self.result >> 16) as u16,
            MPY_SUMEXT => self.sumext,
            _ => 0,
        }
    }
}

impl Peripheral for Msp430Mpy {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.word(offset) >> ((offset & 1) * 8)) as u8)
    }

    /// A byte store writes the low byte and clears the upper one.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset & 1 == 0 {
            self.write_u16(offset, u16::from(value))?;
        }
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(self.word(offset))
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        match offset & !1 {
            0x0..=0x6 => {
                self.op1 = value;
                self.mode = (offset >> 1) as u8;
            }
            MPY_OP2 => {
                self.op2 = value;
                self.run();
            }
            MPY_RESLO => self.result = (self.result & 0xFFFF_0000) | u32::from(value),
            MPY_RESHI => self.result = (self.result & 0xFFFF) | u32::from(value) << 16,
            _ => {}
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }
}

// ── Basic clock module ────────────────────────────────────────────────────

/// The basic clock module registers as storage with their reset values.
#[derive(Debug)]
pub struct Msp430Bcm {
    reset: &'static [u8],
    regs: Vec<u8>,
}

impl Msp430Bcm {
    /// x1xx basic clock module at 0x56: DCOCTL, BCSCTL1, BCSCTL2 (SLAU049 4.3).
    pub fn new() -> Self {
        Self::with_reset(&[0x60, 0x84, 0x00])
    }

    /// x2xx basic clock module+ at 0x56: DCOCTL, BCSCTL1, BCSCTL2 (SLAU144 5.3).
    pub fn plus() -> Self {
        Self::with_reset(&[0x60, 0x87, 0x00])
    }

    /// x2xx BCSCTL3 at 0x53.
    pub fn bcsctl3() -> Self {
        Self::with_reset(&[0x05])
    }

    fn with_reset(reset: &'static [u8]) -> Self {
        Self {
            reset,
            regs: reset.to_vec(),
        }
    }
}

impl Default for Msp430Bcm {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Msp430Bcm {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.regs.get(offset as usize).copied().unwrap_or(0))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if let Some(reg) = self.regs.get_mut(offset as usize) {
            *reg = value;
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn on_system_reset(&mut self) {
        self.regs = self.reset.to_vec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SFR_IE1: u64 = 0;
    const SFR_IFG1: u64 = 2;
    const U_CTL: u64 = 0;
    const U_TCTL: u64 = 1;
    const U_RXBUF: u64 = 6;
    const U_TXBUF: u64 = 7;
    const CTL_SWRST: u8 = 0x01;
    const CTL_LISTEN: u8 = 0x08;
    const TCTL_TXEPT: u8 = 0x01;

    const F149_IE1: [u32; 8] = [5, 0, 0, 0, 0, 0, 6, 7];
    const URXIFG0: u32 = sfr_source(1, 6);
    const UTXIFG0: u32 = sfr_source(1, 7);

    fn sfr_on(irq: &Arc<Msp430Irq>) -> Msp430Sfr {
        let mut sfr = Msp430Sfr::new([F149_IE1, [0; 8], [0; 8]]);
        sfr.attach_msp430_irq(irq);
        sfr
    }

    fn usart_on(irq: &Arc<Msp430Irq>) -> Msp430Usart {
        let mut u = Msp430Usart::new([1, 6, 7], [1, 6, 7]);
        u.attach_msp430_irq(irq);
        sfr_on(irq).write(SFR_ME1, 0xC0).unwrap();
        u.write(U_CTL, 0x10).unwrap();
        u
    }

    #[test]
    fn sfr_bits_follow_their_sources_and_unmapped_bits_are_storage() {
        let irq = Msp430Irq::new();
        let mut sfr = sfr_on(&irq);
        assert_eq!(sfr.read(SFR_IFG1).unwrap(), IFG1_RESET);
        sfr.write(SFR_IE1, 0x42).unwrap();
        assert!(irq.enabled(URXIFG0) && !irq.enabled(UTXIFG0));
        assert_eq!(sfr.read(SFR_IE1).unwrap(), 0x42);
        irq.set_flag(URXIFG0, true);
        assert_eq!(sfr.read(SFR_IFG1).unwrap(), IFG1_RESET | 0x40);
        assert_eq!(irq.pending(), 1 << 6, "USART0 receive is line 6");
        // Firmware clears OFIFG and the receive flag.
        sfr.write(SFR_IFG1, 0).unwrap();
        assert_eq!(sfr.read(SFR_IFG1).unwrap(), 0);
        assert_eq!(irq.pending(), 0);
    }

    #[test]
    fn only_a_single_source_line_clears_its_flag_on_acceptance() {
        let irq = Msp430Irq::new();
        // USCI_A0 and USCI_B0 receive share line 8 on a 2xx part.
        let mut sfr = Msp430Sfr::new([F149_IE1, [8, 9, 8, 9, 0, 0, 0, 0], [0; 8]]);
        sfr.attach_msp430_irq(&irq);
        sfr.write(SFR_IE1, 0x40).unwrap();
        sfr.write(1, 0x05).unwrap();
        irq.set_flag(URXIFG0, true);
        irq.set_flag(sfr_source(2, 0), true);
        irq.set_flag(sfr_source(2, 2), true);
        assert_eq!(irq.pending(), 1 << 6 | 1 << 8);
        irq.accept(6);
        irq.accept(8);
        assert!(!irq.flag(URXIFG0));
        assert_eq!(sfr.read(3).unwrap(), 0x05, "shared flags stay set");
    }

    #[test]
    fn accepting_the_nmi_clears_its_enables() {
        let irq = Msp430Irq::new();
        let mut sfr = Msp430Sfr::new([[0, 1, 0, 0, 1, 1, 0, 0], [0; 8], [0; 8]]);
        sfr.attach_msp430_irq(&irq);
        sfr.write(SFR_IFG1, 0x02).unwrap();
        sfr.write(SFR_IE1, 0x12).unwrap();
        assert_eq!(irq.pending(), 1 << LINE_NMI);
        irq.accept(LINE_NMI);
        assert_eq!(sfr.read(SFR_IE1).unwrap(), 0, "OFIE and NMIIE cleared");
        assert_eq!(
            sfr.read(SFR_IFG1).unwrap(),
            0x02,
            "OFIFG stays for the handler"
        );
    }

    #[test]
    fn usart_transmits_only_while_enabled_and_keeps_utxifg_set() {
        let irq = Msp430Irq::new();
        let mut u = Msp430Usart::new([1, 6, 7], [1, 6, 7]);
        u.attach_msp430_irq(&irq);
        let sink = Arc::new(Mutex::new(Vec::new()));
        u.attach_console_sink(Some(sink.clone()), false);
        assert!(irq.flag(UTXIFG0), "UTXIFG is set by a PUC");
        // SWRST still set and UTXE clear: nothing leaves.
        u.write(U_TXBUF, b'x').unwrap();
        assert!(sink.lock().unwrap().is_empty());
        let mut u = usart_on(&irq);
        u.attach_console_sink(Some(sink.clone()), false);
        irq.set_flag(UTXIFG0, false);
        u.write(U_TXBUF, b'A').unwrap();
        assert_eq!(*sink.lock().unwrap(), b"A");
        assert!(irq.flag(UTXIFG0), "the buffer is empty again");
        assert_eq!(u.read(U_TCTL).unwrap() & TCTL_TXEPT, TCTL_TXEPT);
    }

    #[test]
    fn usart_receives_one_byte_at_a_time_and_a_read_clears_urxifg() {
        let irq = Msp430Irq::new();
        let mut u = usart_on(&irq);
        u.uart_rx_source().unwrap().lock().unwrap().extend(b"ab");
        u.tick();
        assert!(irq.flag(URXIFG0));
        u.tick();
        assert_eq!(u.read(U_RXBUF).unwrap(), b'a', "the second byte waits");
        assert!(!irq.flag(URXIFG0));
        u.tick();
        assert_eq!(u.read(U_RXBUF).unwrap(), b'b');
        u.tick();
        assert!(!irq.flag(URXIFG0));
    }

    #[test]
    fn listen_feeds_transmitted_bytes_back() {
        let irq = Msp430Irq::new();
        let mut u = usart_on(&irq);
        u.write(U_CTL, 0x10 | CTL_LISTEN).unwrap();
        u.write(U_TXBUF, 0x5A).unwrap();
        u.tick();
        assert!(irq.flag(URXIFG0));
        assert_eq!(u.read(U_RXBUF).unwrap(), 0x5A);
    }

    #[test]
    fn swrst_restores_the_flag_reset_state() {
        let irq = Msp430Irq::new();
        let mut u = usart_on(&irq);
        u.uart_rx_source().unwrap().lock().unwrap().push_back(1);
        u.tick();
        irq.set_flag(UTXIFG0, false);
        u.write(U_CTL, CTL_SWRST).unwrap();
        assert!(!irq.flag(URXIFG0) && irq.flag(UTXIFG0));
    }

    #[test]
    fn wdt_needs_its_password() {
        let irq = Msp430Irq::new();
        let mut w = Msp430Wdt::new();
        w.attach_msp430_irq(&irq);
        assert_eq!(w.read_u16(0).unwrap(), 0x6900);
        w.write_u16(0, 0x5A80 | WDT_CNTCL).unwrap();
        assert_eq!(w.read_u16(0).unwrap(), 0x6980, "WDTCNTCL reads as zero");
        assert!(!w.take_reset_request());
        w.write_u16(0, 0x0080).unwrap();
        assert_eq!(
            w.latched_reset_cause(),
            Some(PeripheralResetCause::Watchdog)
        );
        assert!(w.take_reset_request());
        assert!(!w.take_reset_request(), "taken once");
        assert!(irq.flag(WDTIFG), "WDTIFG records the violation");
        w.write(0, 0x80).unwrap();
        assert!(w.take_reset_request(), "a byte store has no password");
    }

    #[test]
    fn multiplier_modes() {
        let mut m = Msp430Mpy::new();
        m.write_u16(0x0, 0xFFFF).unwrap();
        m.write_u16(MPY_OP2, 0xFFFF).unwrap();
        assert_eq!(m.read_u16(MPY_RESHI).unwrap(), 0xFFFE);
        assert_eq!(m.read_u16(MPY_RESLO).unwrap(), 0x0001);
        assert_eq!(m.read_u16(MPY_SUMEXT).unwrap(), 0);
        // MPYS: -1 * 2 = -2.
        m.write_u16(0x2, 0xFFFF).unwrap();
        m.write_u16(MPY_OP2, 2).unwrap();
        assert_eq!(m.read_u16(MPY_RESHI).unwrap(), 0xFFFF);
        assert_eq!(m.read_u16(MPY_RESLO).unwrap(), 0xFFFE);
        assert_eq!(m.read_u16(MPY_SUMEXT).unwrap(), 0xFFFF);
        // MAC: 0xFFFFFFFE + 2 * 2 carries out.
        m.write_u16(0x4, 2).unwrap();
        m.write_u16(MPY_OP2, 2).unwrap();
        assert_eq!(m.read_u16(MPY_RESHI).unwrap(), 0);
        assert_eq!(m.read_u16(MPY_RESLO).unwrap(), 2);
        assert_eq!(m.read_u16(MPY_SUMEXT).unwrap(), 1);
        // MACS: 2 + (-3 * 1) = -1.
        m.write_u16(0x6, 0xFFFD).unwrap();
        m.write_u16(MPY_OP2, 1).unwrap();
        assert_eq!(m.read_u16(MPY_RESHI).unwrap(), 0xFFFF);
        assert_eq!(m.read_u16(MPY_RESLO).unwrap(), 0xFFFF);
        assert_eq!(m.read_u16(MPY_SUMEXT).unwrap(), 0xFFFF);
    }

    #[test]
    fn usci_a_runs_without_module_enables_and_keeps_its_flags_on_acceptance() {
        let irq = Msp430Irq::new();
        // F2619: UCA0RXIFG/UCA0TXIFG are IFG2.0/IFG2.1, sharing lines 8 and 9
        // with UCB0 (IFG2.2/IFG2.3).
        let mut sfr = Msp430Sfr::new([[5, 0, 0, 0, 0, 0, 0, 0], [8, 9, 8, 9, 0, 0, 0, 0], [0; 8]]);
        sfr.attach_msp430_irq(&irq);
        let mut u = Msp430Usart::usci_a([2, 0, 1]);
        u.attach_msp430_irq(&irq);
        let sink = Arc::new(Mutex::new(Vec::new()));
        u.attach_console_sink(Some(sink.clone()), false);
        assert_eq!(u.read(4).unwrap(), 0x01, "UCSWRST is set after a PUC");
        assert_eq!(sfr.read(3).unwrap(), 0x02, "UCA0TXIFG is set after a PUC");
        u.write(10, b'x').unwrap();
        assert!(sink.lock().unwrap().is_empty(), "held in reset");
        u.write(4, 0x80).unwrap();
        u.write(10, b'U').unwrap();
        assert_eq!(*sink.lock().unwrap(), b"U");
        // The transmit line is shared with UCB0: acceptance keeps the flag.
        sfr.write(1, 0x02).unwrap();
        assert_eq!(irq.pending(), 1 << 9);
        irq.accept(9);
        assert_eq!(sfr.read(3).unwrap(), 0x02);
        // UCLISTEN (UCA0STAT.7) loops a byte back; reading RXBUF clears RXIFG.
        u.write(8, 0xFF).unwrap();
        assert_eq!(u.read(8).unwrap(), 0x80, "the status flags are never set");
        u.write(10, 0x5A).unwrap();
        u.tick();
        assert_eq!(sfr.read(3).unwrap(), 0x03);
        assert_eq!(u.read(9).unwrap(), 0x5A);
        assert_eq!(sfr.read(3).unwrap(), 0x02);
    }

    #[test]
    fn the_second_usci_uses_uc1ie_and_uc1ifg() {
        let irq = Msp430Irq::new();
        let mut sfr = Msp430Sfr::new([[0; 8], [0; 8], [14, 15, 14, 15, 0, 0, 0, 0]]);
        sfr.attach_msp430_irq(&irq);
        let mut u = Msp430Usart::usci_a([3, 0, 1]);
        u.attach_msp430_irq(&irq);
        assert_eq!(sfr.read(7).unwrap(), 0x02, "UCA1TXIFG");
        sfr.write(6, 0x02).unwrap();
        assert_eq!(irq.pending(), 1 << 15);
        assert_eq!(sfr.read(6).unwrap(), 0x02);
    }

    #[test]
    fn clock_module_reset_values() {
        assert_eq!(Msp430Bcm::new().read(1).unwrap(), 0x84);
        let mut plus = Msp430Bcm::plus();
        assert_eq!(plus.read(1).unwrap(), 0x87);
        plus.write(1, 0x0D).unwrap();
        plus.on_system_reset();
        assert_eq!(plus.read(1).unwrap(), 0x87);
        assert_eq!(Msp430Bcm::bcsctl3().read(0).unwrap(), 0x05);
    }
}
