// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Texas Instruments MSP430x1xx, MSP430x2xx and MSP430x5xx on-chip modules
//! (family user's guides SLAU049, SLAU144 and SLAU208) and the interrupt
//! sources they share with the [`crate::cpu::Msp430`] core.
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
//!
//! MSP430x5xx modules (SLAU208):
//!
//! * [`Msp430SfrX5`] (`msp430_sfr_x5`): SFRIE1, SFRIFG1 and SFRRPCR. A
//!   descriptor maps each interrupt bit to its line; a bit with no line keeps
//!   its flag and enable but requests nothing.
//! * [`Msp430Usart`] (`msp430_usci_a_x5`): the USCI_A register window of the
//!   x5xx parts, UART mode, with the module's own UCAxIE, UCAxIFG and UCAxIV.
//! * [`Msp430Wdt`] (`msp430_wdt_a`): WDT_A, the same password scheme.
//! * [`Msp430Mpy32`] (`msp430_mpy32`): the 32-bit hardware multiplier.
//! * [`Msp430Ucs`] (`msp430_ucs`): the unified clock system registers and the
//!   oscillator fault flags of a part with no crystal fitted. No clock
//!   frequency is derived from them and the FLL does not run.
//! * [`Msp430Pmm`] (`msp430_pmm`): the power management module password,
//!   PMMCOREV and the low-side supervisor flags of a core voltage that
//!   settles at once.

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
    /// Sources a module holds set: their condition still exists, so
    /// firmware cannot clear the flag.
    held: AtomicU64,
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
            held: AtomicU64::new(0),
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
        Self::put(&self.flag, source, on || Self::get(&self.held, source));
    }

    /// Hold the flag of `source` set while `on`; when released the flag
    /// stays until firmware clears it.
    pub fn hold(&self, source: u32, on: bool) {
        Self::put(&self.held, source, on);
        if on {
            Self::put(&self.flag, source, true);
        }
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

/// What one byte of a UART register window is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// A control or status register, by index.
    Reg(usize),
    Rxbuf,
    Txbuf,
    /// UCAxIE, UCAxIFG and the low byte of UCAxIV (x5xx).
    Ie,
    Ifg,
    Iv,
    Unused,
}

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
    /// The register window of a module that is not laid out as `regs`
    /// registers followed by the two data buffers. Such a module has its
    /// own interrupt enable, flag and vector registers.
    window: Option<fn(u64) -> Slot>,
}

impl Layout {
    fn slot(&self, offset: u64) -> Slot {
        match self.window {
            Some(window) => window(offset),
            None => match offset as usize {
                o if o < self.regs => Slot::Reg(o),
                o if o == self.regs => Slot::Rxbuf,
                o if o == self.regs + 1 => Slot::Txbuf,
                _ => Slot::Unused,
            },
        }
    }
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
    window: None,
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
    window: None,
};

/// MSP430x5xx USCI_A (SLAU208 36.4): UCAxCTL1, UCAxCTL0, UCAxBR0, UCAxBR1,
/// UCAxMCTL, UCAxSTAT, UCAxABCTL, UCAxIRTCTL, UCAxIRRCTL in a 32-byte window
/// with UCAxRXBUF at 0x0C, UCAxTXBUF at 0x0E and UCAxIE, UCAxIFG, UCAxIV at
/// 0x1C to 0x1F. UCAxSTAT keeps UCLISTEN; its error and busy flags are never
/// set.
const USCI_A_X5: Layout = Layout {
    regs: 9,
    reset: [0x01, 0, 0, 0, 0, 0, 0, 0, 0],
    swrst: (0, 0x01),
    listen: (5, 0x80),
    divider: 2,
    forced: (5, 0, 0x7F),
    module_enable: false,
    window: Some(usci_a_x5_slot),
};

fn usci_a_x5_slot(offset: u64) -> Slot {
    match offset {
        0x00 | 0x01 => Slot::Reg(offset as usize),
        0x06..=0x08 => Slot::Reg(offset as usize - 4),
        0x0A => Slot::Reg(5),
        0x0C => Slot::Rxbuf,
        0x0E => Slot::Txbuf,
        0x10 => Slot::Reg(6),
        0x12 | 0x13 => Slot::Reg(offset as usize - 11),
        0x1C => Slot::Ie,
        0x1D => Slot::Ifg,
        0x1E => Slot::Iv,
        _ => Slot::Unused,
    }
}

/// The first interrupt source of the x5xx USCI modules: module `unit` owns
/// sources `X5_USCI_SOURCE + 2 * unit` (receive) and the next one (transmit).
const X5_USCI_SOURCE: u32 = 24;
/// x5xx USCI modules one chip can declare.
pub const X5_USCI_UNITS: u32 = (SOURCES - X5_USCI_SOURCE) / 2;
/// UCAxIV values.
const UCIV_RX: u8 = 2;
const UCIV_TX: u8 = 4;

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
    /// The request line of a module with its own flag registers.
    line: Option<u32>,
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

    /// An x5xx USCI_A. `unit` numbers the USCI modules of the chip (it
    /// selects the interrupt sources); `line` is the request line both
    /// flags share.
    pub fn usci_a_x5(unit: u32, line: u32) -> Self {
        let mut usart = Self::with_layout(&USCI_A_X5, [0; 3], [0; 3]);
        let rx = X5_USCI_SOURCE + 2 * (unit % X5_USCI_UNITS);
        usart.sources = [rx, rx + 1];
        usart.line = Some(line);
        usart.irq = Msp430Irq::new();
        usart.route();
        usart.reset_flags();
        usart
    }

    /// A module with its own flags shares one line between them, so the
    /// core does not clear either on acceptance.
    fn route(&self) {
        if let Some(line) = self.line {
            for source in self.sources {
                self.irq.route(source, line, false);
            }
        }
    }

    /// UCAxIV: the highest-priority flag that is set and enabled.
    fn vector(&self) -> u8 {
        let pending = |source| self.irq.flag(source) && self.irq.enabled(source);
        if pending(self.sources[0]) {
            UCIV_RX
        } else if pending(self.sources[1]) {
            UCIV_TX
        } else {
            0
        }
    }

    /// An access to UCAxIV clears the flag it reports.
    fn take_vector(&self, s: &mut UsartState) -> u8 {
        let vector = self.vector();
        match vector {
            UCIV_RX => {
                s.rx_full = false;
                self.irq.set_flag(self.sources[0], false);
            }
            UCIV_TX => self.irq.set_flag(self.sources[1], false),
            _ => {}
        }
        vector
    }

    fn own_bits(&self, flags: bool) -> u8 {
        (0..2).fold(0, |v, bit| {
            let source = self.sources[bit];
            let on = if flags {
                self.irq.flag(source)
            } else {
                self.irq.enabled(source)
            };
            v | u8::from(on) << bit
        })
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
            line: None,
            trace: crate::bus::bus_trace::new_log(),
            trace_name: String::new(),
            clock: None,
        };
        usart.reset_flags();
        usart
    }

    crate::cycle_clock::scheduler_mode!();

    /// The transmit flag is set and the receive flag cleared by a PUC and
    /// while the software reset bit is set. A module with its own enable
    /// register loses its enables as well.
    fn reset_flags(&self) {
        self.irq.set_flag(self.sources[0], false);
        self.irq.set_flag(self.sources[1], true);
        if self.line.is_some() {
            for source in self.sources {
                self.irq.set_enable(source, false);
            }
        }
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
        Some(match self.layout.slot(offset) {
            Slot::Reg(index) => s.regs[index],
            Slot::Rxbuf => s.rxbuf,
            Slot::Ie => self.own_bits(false),
            Slot::Ifg => self.own_bits(true),
            Slot::Iv => self.vector(),
            Slot::Txbuf | Slot::Unused => 0,
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
        Ok(match self.layout.slot(offset) {
            Slot::Reg(index) => s.regs[index],
            Slot::Rxbuf => {
                // Reading the receive buffer clears the receive flag.
                s.rx_full = false;
                self.irq.set_flag(self.sources[0], false);
                s.rxbuf
            }
            Slot::Ie => self.own_bits(false),
            Slot::Ifg => self.own_bits(true),
            Slot::Iv => self.take_vector(&mut s),
            Slot::Txbuf | Slot::Unused => 0,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let mut s = self.state.lock().unwrap();
        match self.layout.slot(offset) {
            Slot::Reg(index) => self.store(&mut s, index, value),
            Slot::Txbuf if self.tx_enabled(&s) => {
                self.transmit(&mut s, value);
                // The byte is gone at once, so the buffer is empty again.
                self.irq.set_flag(self.sources[1], true);
            }
            Slot::Ie => {
                for (bit, source) in self.sources.into_iter().enumerate() {
                    self.irq.set_enable(source, value >> bit & 1 != 0);
                }
            }
            // Firmware can set and clear both flags; a cleared receive flag
            // frees the receive buffer for the next byte.
            Slot::Ifg => {
                s.rx_full = value & 1 != 0;
                for (bit, source) in self.sources.into_iter().enumerate() {
                    self.irq.set_flag(source, value >> bit & 1 != 0);
                }
            }
            Slot::Iv => {
                self.take_vector(&mut s);
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
        self.route();
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

/// WDTCTL of the watchdog timer (x1xx), the watchdog timer+ (x2xx) and
/// WDT_A (x5xx).
#[derive(Debug, Default)]
pub struct Msp430Wdt {
    ctl: u8,
    /// The low byte after a PUC.
    reset: u8,
    reset_latched: bool,
    irq: Option<Arc<Msp430Irq>>,
}

/// WDTIFG is IFG1.0.
const WDTIFG: u32 = sfr_source(1, 0);

impl Msp430Wdt {
    pub fn new() -> Self {
        Self::default()
    }

    /// WDT_A: WDTIS resets to 4 (SLAU208 16.3.1).
    pub fn wdt_a() -> Self {
        Self {
            ctl: 0x04,
            reset: 0x04,
            ..Self::default()
        }
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
        self.ctl = self.reset;
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

// ── MSP430x5xx special function registers ─────────────────────────────────

/// Implemented bits of SFRIE1 and SFRIFG1 (SLAU208 1.14.1, 1.14.2).
const SFRIE1_BITS: u8 = 0xFB;
const SFRIFG1_BITS: u8 = 0xDB;
/// OFIFG and JMBOUTIFG: set by a power-on reset.
const SFRIFG1_POR: u8 = 0x82;
/// NMIIFG and VMAIFG: cleared by a PUC.
const SFRIFG1_PUC_CLEARED: u8 = 0x18;
/// SYSRSTRE and SYSRSTUP.
const SFRRPCR_RESET: u8 = 0x0C;
/// OFIFG is SFRIFG1.1, as it is IFG1.1 on the older families.
const OFIFG: u32 = sfr_source(1, 1);

/// SFRIE1, SFRIFG1 and SFRRPCR, three word registers at 0x0100. The bits of
/// SFRIE1/SFRIFG1 are interrupt sources 0 to 7.
#[derive(Debug)]
pub struct Msp430SfrX5 {
    irq: Arc<Msp430Irq>,
    /// The line behind each bit (0 = the bit requests no interrupt).
    lines: [u32; 8],
    rpcr: u8,
}

impl Msp430SfrX5 {
    pub fn new(lines: [u32; 8]) -> Self {
        let sfr = Self {
            irq: Msp430Irq::new(),
            lines,
            rpcr: SFRRPCR_RESET,
        };
        sfr.power_on();
        sfr
    }

    fn power_on(&self) {
        for bit in 0..8 {
            let source = sfr_source(1, bit);
            let line = self.lines[bit as usize];
            if line != 0 {
                let alone = self.lines.iter().filter(|l| **l == line).count() == 1;
                self.irq.route(source, line, alone);
            }
            if SFRIFG1_POR >> bit & 1 != 0 {
                self.irq.set_flag(source, true);
            }
        }
    }

    fn bits(&self, flags: bool) -> u8 {
        (0..8).fold(0, |v, bit| {
            let source = sfr_source(1, bit);
            let on = if flags {
                self.irq.flag(source)
            } else {
                self.irq.enabled(source)
            };
            v | u8::from(on) << bit
        })
    }
}

impl Peripheral for Msp430SfrX5 {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(match offset {
            0 => self.bits(false) & SFRIE1_BITS,
            2 => self.bits(true) & SFRIFG1_BITS,
            4 => self.rpcr,
            _ => 0,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            0 | 2 => {
                let implemented = if offset == 0 {
                    SFRIE1_BITS
                } else {
                    SFRIFG1_BITS
                };
                for bit in (0..8).filter(|b| implemented >> b & 1 != 0) {
                    let on = value >> bit & 1 != 0;
                    if offset == 0 {
                        self.irq.set_enable(sfr_source(1, bit), on);
                    } else {
                        self.irq.set_flag(sfr_source(1, bit), on);
                    }
                }
            }
            4 => self.rpcr = value & 0x0F,
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
        self.power_on();
    }

    fn on_system_reset(&mut self) {
        self.rpcr = SFRRPCR_RESET;
        for bit in 0..8 {
            let source = sfr_source(1, bit);
            self.irq.set_enable(source, false);
            if SFRIFG1_PUC_CLEARED >> bit & 1 != 0 {
                self.irq.set_flag(source, false);
            }
        }
    }
}

// ── 32-bit hardware multiplier ────────────────────────────────────────────

const MPY32_OP1_32: u64 = 0x10;
const MPY32_OP2L: u64 = 0x20;
const MPY32_OP2H: u64 = 0x22;
const MPY32_RES0: u64 = 0x24;
const MPY32_RES3: u64 = 0x2A;
const MPY32_CTL0: u64 = 0x2C;
const MPYC: u16 = 1 << 0;
const MPYM_SHIFT: u16 = 4;
const MPYOP1_32: u16 = 1 << 6;
const MPYOP2_32: u16 = 1 << 7;
/// Writable bits of MPY32CTL0.
const MPY32CTL0_BITS: u16 = 0x03FD;

/// MPY32 (SLAU208 chapter 25): 16-bit and 32-bit operands, MPY, MPYS, MAC and
/// MACS, a 64-bit result in RES0 to RES3. The result is ready at once.
/// MPYFRAC and MPYSAT are stored and do not change the result; the delayed
/// write bits are stored. A byte store to an operand register writes its low
/// byte (sign-extended in a signed mode); a byte store to the upper byte of
/// an operand or result register is ignored. After a 16-bit MAC or MACS the
/// accumulation carries into RES2 and RES3 (the guide does not specify them).
#[derive(Debug, Default)]
pub struct Msp430Mpy32 {
    op1: u32,
    op2: u32,
    result: u64,
    sumext: u16,
    ctl: u16,
    /// OP2L was written: OP2H completes the operand and runs the operation.
    expect_op2h: bool,
}

impl Msp430Mpy32 {
    pub fn new() -> Self {
        Self::default()
    }

    fn mode(&self) -> u16 {
        self.ctl >> MPYM_SHIFT & 3
    }

    fn signed(&self) -> bool {
        self.mode() & 1 != 0
    }

    fn set_low(word: &mut u32, value: u16) {
        *word = (*word & 0xFFFF_0000) | u32::from(value);
    }

    fn set_high(word: &mut u32, value: u16) {
        *word = (*word & 0xFFFF) | u32::from(value) << 16;
    }

    /// An operand at its width, extended to 64 bits.
    fn operand(&self, value: u32, wide: bool) -> u64 {
        match (wide, self.signed()) {
            (true, true) => value as i32 as i64 as u64,
            (true, false) => u64::from(value),
            (false, true) => value as u16 as i16 as i64 as u64,
            (false, false) => u64::from(value as u16),
        }
    }

    fn run(&mut self) {
        let op1_wide = self.ctl & MPYOP1_32 != 0;
        let op2_wide = self.ctl & MPYOP2_32 != 0;
        // With an operand above 16 bits the 64-bit result gives sign and
        // carry, otherwise the 32-bit result does (SLAU208 25.2.2).
        let wide = op1_wide || op2_wide;
        let product = self
            .operand(self.op1, op1_wide)
            .wrapping_mul(self.operand(self.op2, op2_wide));
        let mode = self.mode();
        let accumulate = mode & 2 != 0;
        let carry = if !accumulate {
            self.result = product;
            false
        } else {
            let sum = self.result.wrapping_add(product);
            let carry = if wide {
                sum < self.result
            } else {
                u64::from(self.result as u32) + u64::from(product as u32) > u64::from(u32::MAX)
            };
            self.result = sum;
            carry
        };
        let sign_bit = if wide { 63 } else { 31 };
        let negative = self.result >> sign_bit & 1 != 0;
        self.sumext = match mode {
            0 => 0,
            2 => u16::from(carry),
            _ if negative => 0xFFFF,
            _ => 0,
        };
        let mpyc = match mode {
            0 => false,
            1 => negative,
            _ => carry,
        };
        self.ctl = (self.ctl & !MPYC) | u16::from(mpyc);
    }

    fn word(&self, offset: u64) -> u16 {
        match offset & !1 {
            0x00..=0x06 => self.op1 as u16,
            MPY_OP2 | MPY32_OP2L => self.op2 as u16,
            MPY_RESLO => self.result as u16,
            MPY_RESHI => (self.result >> 16) as u16,
            MPY_SUMEXT => self.sumext,
            o @ MPY32_OP1_32..=0x1E if o & 2 == 0 => self.op1 as u16,
            MPY32_OP1_32..=0x1E => (self.op1 >> 16) as u16,
            MPY32_OP2H => (self.op2 >> 16) as u16,
            o @ MPY32_RES0..=MPY32_RES3 => (self.result >> ((o - MPY32_RES0) * 8)) as u16,
            MPY32_CTL0 => self.ctl,
            _ => 0,
        }
    }

    fn set_mode(&mut self, mode: u64) {
        self.ctl = (self.ctl & !(3 << MPYM_SHIFT)) | (mode as u16 & 3) << MPYM_SHIFT;
    }

    fn set_result_word(&mut self, index: u64, value: u16) {
        let shift = index * 16;
        self.result = (self.result & !(0xFFFF << shift)) | u64::from(value) << shift;
    }
}

impl Peripheral for Msp430Mpy32 {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.word(offset) >> ((offset & 1) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            MPY32_CTL0 => self.write_u16(MPY32_CTL0, (self.ctl & 0xFF00) | u16::from(value))?,
            o if o == MPY32_CTL0 + 1 => {
                self.write_u16(MPY32_CTL0, (self.ctl & 0xFF) | u16::from(value) << 8)?
            }
            o if o & 1 != 0 => {}
            0x00..=MPY32_OP2H if !(MPY_RESLO..=MPY_SUMEXT).contains(&offset) => {
                // The register written selects the mode before the byte is
                // extended: MPYS, MACS and their 32-bit forms are signed.
                let signed = match offset {
                    0x00..=0x06 => offset & 2 != 0,
                    MPY32_OP1_32..=0x1E => offset & 4 != 0,
                    _ => self.signed(),
                };
                let word = if signed {
                    value as i8 as i16 as u16
                } else {
                    u16::from(value)
                };
                self.write_u16(offset, word)?;
            }
            _ => self.write_u16(offset, u16::from(value))?,
        }
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(self.word(offset))
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        match offset & !1 {
            o @ 0x00..=0x06 => {
                Self::set_low(&mut self.op1, value);
                self.set_mode(o >> 1);
                self.ctl &= !MPYOP1_32;
            }
            MPY_OP2 => {
                Self::set_low(&mut self.op2, value);
                self.ctl &= !MPYOP2_32;
                self.expect_op2h = false;
                self.run();
            }
            MPY_RESLO => self.set_result_word(0, value),
            MPY_RESHI => self.set_result_word(1, value),
            o @ MPY32_OP1_32..=0x1E => {
                self.set_mode((o - MPY32_OP1_32) >> 2);
                if o & 2 == 0 {
                    Self::set_low(&mut self.op1, value);
                    self.ctl &= !MPYOP1_32;
                } else {
                    Self::set_high(&mut self.op1, value);
                    self.ctl |= MPYOP1_32;
                }
            }
            MPY32_OP2L => {
                Self::set_low(&mut self.op2, value);
                self.ctl |= MPYOP2_32;
                self.expect_op2h = true;
            }
            // Without a preceding OP2L write the store is ignored.
            MPY32_OP2H if self.expect_op2h => {
                Self::set_high(&mut self.op2, value);
                self.expect_op2h = false;
                self.run();
            }
            o @ MPY32_RES0..=MPY32_RES3 => self.set_result_word((o - MPY32_RES0) >> 1, value),
            MPY32_CTL0 => self.ctl = value & MPY32CTL0_BITS,
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

// ── Unified clock system ──────────────────────────────────────────────────

const UCS_REGS: usize = 9;
/// UCSCTL0 to UCSCTL8 after a power-on reset (SLAU208 5.4).
const UCS_RESET: [u16; UCS_REGS] = [
    0x0000, 0x0020, 0x101F, 0x0000, 0x0044, 0x0000, 0xC1CD, 0x0F03, 0x0707,
];
/// Writable bits of UCSCTL0 to UCSCTL8.
const UCS_BITS: [u16; UCS_REGS] = [
    0x1FF8, 0x0073, 0x73FF, 0x0077, 0x0777, 0x7777, 0xD1FF, 0x3C1F, 0x0717,
];
/// UCSCTL7 and UCSCTL8 keep their value through a PUC.
const UCS_PUC_REGS: usize = 7;
/// Bits 9 and 8 of UCSCTL7 read 1.
const UCSCTL7_ONES: u16 = 0x0300;
const UCSCTL7: usize = 7;
const DCOFFG: u16 = 1 << 0;
const XT1LFOFFG: u16 = 1 << 1;
const XT1HFOFFG: u16 = 1 << 2;
const XT2OFFG: u16 = 1 << 3;
const UCS_FAULTS: u16 = DCOFFG | XT1LFOFFG | XT1HFOFFG | XT2OFFG;

/// UCSCTL0 to UCSCTL8 of a part with no crystal on XT1 or XT2.
///
/// The registers are storage with their reset values and reserved bits. A
/// crystal oscillator that is switched on (its XTxOFF bit is clear) or
/// selected (FLL reference, ACLK, SMCLK or MCLK) has a fault, because no
/// crystal is fitted: its fault flag is set again when firmware clears it.
/// After reset XT1 is the FLL reference and the ACLK source, so XT1LFOFFG
/// stays set until firmware selects other sources. DCOFFG is set by a
/// power-on reset and stays clear once cleared: the FLL does not run and the
/// DCO tap in UCSCTL0 does not move. OFIFG (SFRIFG1.1) is held set while a
/// fault flag is set. No clock frequency follows these registers.
#[derive(Debug)]
pub struct Msp430Ucs {
    regs: [u16; UCS_REGS],
    irq: Option<Arc<Msp430Irq>>,
}

impl Default for Msp430Ucs {
    fn default() -> Self {
        Self {
            regs: UCS_RESET,
            irq: None,
        }
    }
}

impl Msp430Ucs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fault flags of the crystal oscillators that are in use.
    fn crystal_faults(&self) -> u16 {
        let selects = [
            self.regs[3] >> 4 & 7,
            self.regs[4] & 7,
            self.regs[4] >> 4 & 7,
            self.regs[4] >> 8 & 7,
        ];
        let ctl6 = self.regs[6];
        let xt1 = ctl6 & 0x0001 == 0 || selects.contains(&0);
        let xt2 = ctl6 & 0x0100 == 0 || selects.iter().any(|s| matches!(s, 5 | 6));
        let xt1_flag = if ctl6 & 0x0020 != 0 {
            XT1HFOFFG
        } else {
            XT1LFOFFG
        };
        (if xt1 { xt1_flag } else { 0 }) | (if xt2 { XT2OFFG } else { 0 })
    }

    fn update(&mut self) {
        self.regs[UCSCTL7] |= self.crystal_faults() | UCSCTL7_ONES;
        if let Some(irq) = &self.irq {
            irq.hold(OFIFG, self.regs[UCSCTL7] & UCS_FAULTS != 0);
        }
    }
}

impl Peripheral for Msp430Ucs {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.regs.get((offset >> 1) as usize).copied().unwrap_or(0);
        Ok((word >> ((offset & 1) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let index = (offset >> 1) as usize;
        if index < UCS_REGS {
            let shift = (offset & 1) * 8;
            let word = (self.regs[index] & !(0xFF << shift)) | u16::from(value) << shift;
            self.regs[index] = word & UCS_BITS[index];
            self.update();
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
        self.irq = Some(irq.clone());
        self.update();
    }

    fn on_system_reset(&mut self) {
        self.regs[..UCS_PUC_REGS].copy_from_slice(&UCS_RESET[..UCS_PUC_REGS]);
        self.update();
    }
}

// ── Power management module ───────────────────────────────────────────────

const PMM_PASSWORD: u8 = 0xA5;
const PMM_READ_KEY: u8 = 0x96;
/// PMMCTL0 low byte: PMMCOREV, PMMSWBOR, PMMSWPOR, PMMREGOFF.
const PMMCOREV: u8 = 0x03;
const PMMSWBOR: u8 = 1 << 2;
const PMMSWPOR: u8 = 1 << 3;
const PMMREGOFF: u8 = 1 << 4;
/// Word registers after PMMCTL0: PMMCTL1, SVSMHCTL, SVSMLCTL, SVSMIO, the
/// reserved word at 0x0A, PMMIFG, PMMRIE.
const PMM_WORDS: usize = 7;
const PMM_RESET: [u16; PMM_WORDS] = [0, 0x4400, 0x4400, 0x0020, 0, 0, 0x1100];
const PMM_BITS: [u16; PMM_WORDS] = [0x0033, 0xDFD7, 0xDFD7, 0x1838, 0, 0xB777, 0x3377];
const PMM_SVSMHCTL: usize = 1;
const PMM_SVSMLCTL: usize = 2;
const PMM_IFG: usize = 5;
/// SVSMLCTL: SVMLE and the SVSMLRRL level.
const SVMLE: u16 = 1 << 14;
const SVSMLRRL: u16 = 0x0007;
const SVSMLDLYIFG: u16 = 1 << 0;
const SVMLIFG: u16 = 1 << 1;
const SVMLVLRIFG: u16 = 1 << 2;
const SVSMHDLYIFG: u16 = 1 << 4;

/// The PMM registers at 0x0120 (SLAU208 2.3).
///
/// PMMCTL0 has a password: a word store needs 0xA5 in the upper byte or it
/// requests a PUC; a byte store of 0xA5 to the upper byte unlocks the
/// registers and any other byte locks them. Stores to a locked register are
/// ignored. PMMSWBOR and PMMSWPOR request a reset, which this engine runs as
/// a PUC (the brownout and power-on reset state is not restored).
///
/// No voltage is simulated. The core voltage is at its PMMCOREV level at
/// once and DVCC is above every high-side level, so: a store to SVSMHCTL or
/// SVSMLCTL sets its delay flag (SVSMHDLYIFG, SVSMLDLYIFG); SVMLIFG is set
/// when the low-side monitor is enabled with a level above PMMCOREV;
/// SVMLVLRIFG is set when the core voltage level reaches the monitor level.
/// The high-side flags and the reset flags of PMMIFG are never set. PMMIFG
/// requests no interrupt and PMMRIE is storage.
#[derive(Debug)]
pub struct Msp430Pmm {
    ctl0: u8,
    words: [u16; PMM_WORDS],
    unlocked: bool,
    reset_latched: bool,
}

impl Default for Msp430Pmm {
    fn default() -> Self {
        Self {
            ctl0: 0,
            words: PMM_RESET,
            unlocked: false,
            reset_latched: false,
        }
    }
}

impl Msp430Pmm {
    pub fn new() -> Self {
        Self::default()
    }

    /// The core voltage is below the enabled low-side monitor level.
    fn below_monitor(&self) -> bool {
        let ctl = self.words[PMM_SVSMLCTL];
        ctl & SVMLE != 0 && (ctl & SVSMLRRL) > u16::from(self.ctl0 & PMMCOREV)
    }

    /// Apply a change of PMMCOREV or SVSMLCTL to the low-side monitor flags.
    fn change(&mut self, apply: impl FnOnce(&mut Self)) {
        let was_below = self.below_monitor();
        apply(self);
        let below = self.below_monitor();
        if below && !was_below {
            self.words[PMM_IFG] |= SVMLIFG;
        }
        if was_below && !below && self.words[PMM_SVSMLCTL] & SVMLE != 0 {
            self.words[PMM_IFG] |= SVMLVLRIFG;
        }
    }

    fn write_ctl0(&mut self, value: u8) {
        if value & (PMMSWBOR | PMMSWPOR) != 0 {
            self.reset_latched = true;
        }
        self.change(|pmm| pmm.ctl0 = value & (PMMCOREV | PMMREGOFF));
    }

    fn write_word(&mut self, index: usize, value: u16) {
        let value = value & PMM_BITS[index];
        match index {
            PMM_SVSMLCTL => {
                self.change(|pmm| pmm.words[PMM_SVSMLCTL] = value);
                self.words[PMM_IFG] |= SVSMLDLYIFG;
            }
            PMM_SVSMHCTL => {
                self.words[index] = value;
                self.words[PMM_IFG] |= SVSMHDLYIFG;
            }
            _ => self.words[index] = value,
        }
    }
}

impl Peripheral for Msp430Pmm {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(match offset {
            0 => self.ctl0,
            1 => PMM_READ_KEY,
            _ => {
                let word = self.words.get((offset >> 1) as usize - 1).copied();
                (word.unwrap_or(0) >> ((offset & 1) * 8)) as u8
            }
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            0 if self.unlocked => self.write_ctl0(value),
            1 => self.unlocked = value == PMM_PASSWORD,
            _ if self.unlocked && offset >= 2 => {
                let index = (offset >> 1) as usize - 1;
                if index < PMM_WORDS {
                    let shift = (offset & 1) * 8;
                    let word = (self.words[index] & !(0xFF << shift)) | u16::from(value) << shift;
                    self.write_word(index, word);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        match offset {
            0 if (value >> 8) as u8 == PMM_PASSWORD => {
                self.unlocked = true;
                self.write_ctl0(value as u8);
            }
            // A word store without the password is a key violation.
            0 => self.reset_latched = true,
            _ if self.unlocked && offset & 1 == 0 => {
                let index = (offset >> 1) as usize - 1;
                if index < PMM_WORDS {
                    self.write_word(index, value);
                }
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

    fn reset_request_capable(&self) -> bool {
        true
    }

    fn latched_reset_cause(&self) -> Option<PeripheralResetCause> {
        self.reset_latched.then_some(PeripheralResetCause::System)
    }

    fn take_reset_request(&mut self) -> bool {
        std::mem::take(&mut self.reset_latched)
    }

    /// A PUC locks the registers; their contents stay (they are reset by a
    /// brownout or power-on reset only).
    fn on_system_reset(&mut self) {
        self.unlocked = false;
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

    // ── MSP430x5xx ──

    const SFRIE1: u64 = 0;
    const SFRIFG1: u64 = 2;
    /// F5437: only the watchdog interval interrupt (line 5) is delivered.
    const F5437_LINES: [u32; 8] = [5, 0, 0, 0, 0, 0, 0, 0];

    #[test]
    fn x5_sfr_reset_state_reserved_bits_and_puc() {
        let irq = Msp430Irq::new();
        let mut sfr = Msp430SfrX5::new(F5437_LINES);
        sfr.attach_msp430_irq(&irq);
        assert_eq!(sfr.read_u16(SFRIFG1).unwrap(), 0x0082, "OFIFG, JMBOUTIFG");
        assert_eq!(sfr.read_u16(4).unwrap(), 0x000C, "SFRRPCR");
        sfr.write_u16(SFRIE1, 0xFFFF).unwrap();
        assert_eq!(sfr.read_u16(SFRIE1).unwrap(), 0x00FB, "bit 2 is reserved");
        // OFIE is set with OFIFG, but no line is mapped: nothing is requested
        // on a maskable or NMI line.
        assert_eq!(irq.pending() & !1, 0);
        sfr.write_u16(SFRIFG1, 0x0019).unwrap();
        assert_eq!(sfr.read_u16(SFRIFG1).unwrap(), 0x0019);
        assert_eq!(irq.pending() & !1, 1 << 5, "WDTIFG with WDTIE");
        irq.accept(5);
        assert!(!irq.flag(WDTIFG), "alone on its line");
        sfr.write_u16(SFRIFG1, 0x0019).unwrap();
        sfr.on_system_reset();
        assert_eq!(sfr.read_u16(SFRIE1).unwrap(), 0);
        assert_eq!(
            sfr.read_u16(SFRIFG1).unwrap(),
            0x0001,
            "a PUC clears NMIIFG and VMAIFG and keeps WDTIFG"
        );
    }

    const X5_CTL1: u64 = 0x00;
    const X5_BR0: u64 = 0x06;
    const X5_STAT: u64 = 0x0A;
    const X5_RXBUF: u64 = 0x0C;
    const X5_TXBUF: u64 = 0x0E;
    const X5_IE: u64 = 0x1C;
    const X5_IFG: u64 = 0x1D;
    const X5_IV: u64 = 0x1E;

    #[test]
    fn x5_usci_has_its_own_enable_flag_and_vector_registers() {
        let irq = Msp430Irq::new();
        let mut u = Msp430Usart::usci_a_x5(0, 6);
        u.attach_msp430_irq(&irq);
        let sink = Arc::new(Mutex::new(Vec::new()));
        u.attach_console_sink(Some(sink.clone()), false);
        assert_eq!(u.read(X5_CTL1).unwrap(), 0x01, "UCSWRST after a PUC");
        assert_eq!(u.read(X5_IFG).unwrap(), 0x02, "UCTXIFG after a PUC");
        u.write(X5_TXBUF, b'x').unwrap();
        assert!(sink.lock().unwrap().is_empty(), "held in reset");
        u.write(X5_BR0, 9).unwrap();
        u.write(X5_CTL1, 0x80).unwrap();
        u.write(X5_TXBUF, b'U').unwrap();
        assert_eq!(*sink.lock().unwrap(), b"U");
        assert_eq!(u.read(X5_BR0).unwrap(), 9);

        // Disabled flags do not show in UCAxIV and request nothing.
        assert_eq!(u.read_u16(X5_IV).unwrap(), 0);
        assert_eq!(irq.pending(), 0);
        u.write(X5_IE, 0x03).unwrap();
        assert_eq!(u.read(X5_IE).unwrap(), 0x03);
        assert_eq!(irq.pending(), 1 << 6);
        // Both flags share the line, so acceptance keeps them.
        irq.accept(6);
        assert_eq!(u.read(X5_IFG).unwrap(), 0x02);

        // UCLISTEN loops a byte back; the receive flag has priority.
        u.write(X5_STAT, 0xFF).unwrap();
        assert_eq!(u.read(X5_STAT).unwrap(), 0x80);
        u.write(X5_TXBUF, 0x5A).unwrap();
        // One character: ten bit times of the divider.
        u.tick_elapsed(90);
        assert_eq!(u.read(X5_IFG).unwrap(), 0x03);
        assert_eq!(u.peek(X5_IV), Some(2), "a peek clears nothing");
        assert_eq!(u.read(X5_IFG).unwrap(), 0x03);
        assert_eq!(u.read_u16(X5_IV).unwrap(), 2, "UCRXIFG first");
        assert_eq!(u.read(X5_IFG).unwrap(), 0x02, "reading UCAxIV cleared it");
        assert_eq!(u.read_u16(X5_IV).unwrap(), 4, "then UCTXIFG");
        assert_eq!(u.read(X5_IFG).unwrap(), 0);
        assert_eq!(u.read_u16(X5_IV).unwrap(), 0);

        // Reading UCAxRXBUF clears UCRXIFG.
        u.write(X5_TXBUF, 0xA5).unwrap();
        u.tick_elapsed(90);
        assert_eq!(u.read(X5_IFG).unwrap(), 0x03);
        assert_eq!(u.read(X5_RXBUF).unwrap(), 0xA5);
        assert_eq!(u.read(X5_IFG).unwrap(), 0x02);

        // UCSWRST clears the enables and restores the flag reset state.
        u.write(X5_CTL1, 0x81).unwrap();
        assert_eq!(u.read(X5_IE).unwrap(), 0);
        assert_eq!(u.read(X5_IFG).unwrap(), 0x02);
    }

    #[test]
    fn x5_usci_units_do_not_share_sources() {
        let irq = Msp430Irq::new();
        let mut a0 = Msp430Usart::usci_a_x5(0, 6);
        let mut a1 = Msp430Usart::usci_a_x5(1, 17);
        a0.attach_msp430_irq(&irq);
        a1.attach_msp430_irq(&irq);
        a1.write(X5_IE, 0x02).unwrap();
        assert_eq!(a0.read(X5_IE).unwrap(), 0);
        assert_eq!(irq.pending(), 1 << 17);
        a1.write(X5_IFG, 0).unwrap();
        assert_eq!(a0.read(X5_IFG).unwrap(), 0x02);
        assert_eq!(irq.pending(), 0);
    }

    #[test]
    fn wdt_a_resets_to_its_interval_select() {
        let mut w = Msp430Wdt::wdt_a();
        assert_eq!(w.read_u16(0).unwrap(), 0x6904);
        w.write_u16(0, 0x5A80).unwrap();
        assert_eq!(w.read_u16(0).unwrap(), 0x6980);
        w.on_system_reset();
        assert_eq!(w.read_u16(0).unwrap(), 0x6904);
    }

    const M32_MPY32L: u64 = 0x10;
    const M32_MPY32H: u64 = 0x12;
    const M32_MPYS32L: u64 = 0x14;
    const M32_MAC32L: u64 = 0x18;
    const M32_MAC32H: u64 = 0x1A;

    fn result64(m: &Msp430Mpy32) -> u64 {
        (0..4).fold(0, |v, i| {
            v | u64::from(m.read_u16(MPY32_RES0 + 2 * i).unwrap()) << (16 * i)
        })
    }

    #[test]
    fn mpy32_16_bit_operations_match_the_16_bit_multiplier() {
        let mut m = Msp430Mpy32::new();
        m.write_u16(0x0, 0xFFFF).unwrap();
        m.write_u16(MPY_OP2, 0xFFFF).unwrap();
        assert_eq!(result64(&m), 0xFFFE_0001);
        assert_eq!(m.read_u16(MPY_RESHI).unwrap(), 0xFFFE);
        assert_eq!(m.read_u16(MPY_SUMEXT).unwrap(), 0);
        // MPYS: -1 * 2 = -2, sign-extended to 64 bits; MPYC is the sign.
        m.write_u16(0x2, 0xFFFF).unwrap();
        m.write_u16(MPY_OP2, 2).unwrap();
        assert_eq!(result64(&m), 0xFFFF_FFFF_FFFF_FFFE);
        assert_eq!(m.read_u16(MPY_SUMEXT).unwrap(), 0xFFFF);
        assert_eq!(m.read_u16(MPY32_CTL0).unwrap(), 0x0011, "MPYS, MPYC");
        // MAC: 0xFFFFFFFE + 2 * 2 carries out of 32 bits.
        m.write_u16(MPY32_RES0 + 4, 0).unwrap();
        m.write_u16(MPY32_RES3, 0).unwrap();
        m.write_u16(0x4, 2).unwrap();
        m.write_u16(MPY_OP2, 2).unwrap();
        assert_eq!(m.read_u16(MPY_RESLO).unwrap(), 2);
        assert_eq!(m.read_u16(MPY_RESHI).unwrap(), 0);
        assert_eq!(m.read_u16(MPY_SUMEXT).unwrap(), 1);
        assert_eq!(m.read_u16(MPY32_CTL0).unwrap() & MPYC, MPYC);
        // MACS: 2 + (-3 * 1) = -1.
        m.write_u16(MPY32_RES0 + 4, 0).unwrap();
        m.write_u16(0x6, 0xFFFD).unwrap();
        m.write_u16(MPY_OP2, 1).unwrap();
        assert_eq!(m.read_u16(MPY_RESLO).unwrap(), 0xFFFF);
        assert_eq!(m.read_u16(MPY_RESHI).unwrap(), 0xFFFF);
        assert_eq!(m.read_u16(MPY_SUMEXT).unwrap(), 0xFFFF);
    }

    #[test]
    fn mpy32_32_bit_operands() {
        let mut m = Msp430Mpy32::new();
        let op1 = |m: &mut Msp430Mpy32, low: u64, v: u32| {
            m.write_u16(low, v as u16).unwrap();
            m.write_u16(low + 2, (v >> 16) as u16).unwrap();
        };
        let op2 = |m: &mut Msp430Mpy32, v: u32| {
            m.write_u16(MPY32_OP2L, v as u16).unwrap();
            m.write_u16(MPY32_OP2H, (v >> 16) as u16).unwrap();
        };
        // Unsigned 32 x 32.
        op1(&mut m, M32_MPY32L, 0xFFFF_FFFF);
        op2(&mut m, 0xFFFF_FFFF);
        assert_eq!(result64(&m), 0xFFFF_FFFE_0000_0001);
        assert_eq!(m.read_u16(MPY32_CTL0).unwrap(), 0x00C0, "both 32-bit");
        // Signed 32 x 32: -100000 * 70000.
        op1(&mut m, M32_MPYS32L, (-100_000i32) as u32);
        op2(&mut m, 70_000);
        assert_eq!(result64(&m) as i64, -7_000_000_000);
        assert_eq!(m.read_u16(MPY_SUMEXT).unwrap(), 0xFFFF);
        // 32-bit OP1 with a 16-bit OP2.
        op1(&mut m, M32_MPY32L, 0x0001_0000);
        m.write_u16(MPY_OP2, 3).unwrap();
        assert_eq!(result64(&m), 0x0003_0000);
        // The high word alone, then the low word: a 16-bit OP1.
        m.write_u16(M32_MPY32H, 0x1234).unwrap();
        m.write_u16(M32_MPY32L, 5).unwrap();
        m.write_u16(MPY_OP2, 5).unwrap();
        assert_eq!(result64(&m), 25);
        // OP2H without OP2L is ignored.
        m.write_u16(MPY32_OP2H, 0xFFFF).unwrap();
        assert_eq!(result64(&m), 25);
        // MAC with a 64-bit carry.
        for i in 0..4 {
            m.write_u16(MPY32_RES0 + 2 * i, 0xFFFF).unwrap();
        }
        op1(&mut m, M32_MAC32L, 0x0001_0000);
        op2(&mut m, 1);
        assert_eq!(result64(&m), 0xFFFF);
        assert_eq!(m.read_u16(MPY_SUMEXT).unwrap(), 1);
        assert_eq!(m.read_u16(M32_MAC32H).unwrap(), 1, "OP1 reads back");
        // A byte store to a signed operand register is sign-extended.
        m.write(0x2, 0xFE).unwrap();
        m.write_u16(MPY_OP2, 3).unwrap();
        assert_eq!(result64(&m) as i64, -6);
    }

    #[test]
    fn ucs_reset_values_and_reserved_bits() {
        let mut ucs = Msp430Ucs::new();
        let reset = [
            0x0000, 0x0020, 0x101F, 0x0000, 0x0044, 0x0000, 0xC1CD, 0x0F03, 0x0707,
        ];
        for (i, value) in reset.into_iter().enumerate() {
            assert_eq!(ucs.read_u16(2 * i as u64).unwrap(), value, "UCSCTL{i}");
        }
        ucs.write_u16(0x02, 0xFFFF).unwrap();
        assert_eq!(ucs.read_u16(0x02).unwrap(), 0x0073, "UCSCTL1");
        ucs.write(0x05, 0xFF).unwrap();
        assert_eq!(ucs.read_u16(0x04).unwrap(), 0x731F, "UCSCTL2 high byte");
        ucs.on_system_reset();
        assert_eq!(ucs.read_u16(0x02).unwrap(), 0x0020);
    }

    #[test]
    fn ucs_crystal_faults_last_until_the_crystal_is_deselected() {
        let irq = Msp430Irq::new();
        let mut sfr = Msp430SfrX5::new(F5437_LINES);
        sfr.attach_msp430_irq(&irq);
        let mut ucs = Msp430Ucs::new();
        ucs.attach_msp430_irq(&irq);
        let clear = |ucs: &mut Msp430Ucs, sfr: &mut Msp430SfrX5| {
            let ctl7 = ucs.read_u16(0x0E).unwrap();
            ucs.write_u16(0x0E, ctl7 & !UCS_FAULTS).unwrap();
            let ifg = sfr.read_u16(SFRIFG1).unwrap();
            sfr.write_u16(SFRIFG1, ifg & !0x0002).unwrap();
        };
        // After reset XT1 is the FLL reference and the ACLK source, and no
        // crystal is fitted: XT1LFOFFG and OFIFG come back.
        clear(&mut ucs, &mut sfr);
        assert_eq!(ucs.read_u16(0x0E).unwrap() & UCS_FAULTS, XT1LFOFFG);
        assert_eq!(sfr.read_u16(SFRIFG1).unwrap() & 0x0002, 0x0002);
        // FLL reference = REFO: ACLK still asks for XT1.
        ucs.write_u16(0x06, 0x0020).unwrap();
        clear(&mut ucs, &mut sfr);
        assert_eq!(ucs.read_u16(0x0E).unwrap() & UCS_FAULTS, XT1LFOFFG);
        // ACLK = REFO as well: the flags clear and stay clear.
        ucs.write_u16(0x08, 0x0244).unwrap();
        assert_eq!(
            sfr.read_u16(SFRIFG1).unwrap() & 0x0002,
            0x0002,
            "OFIFG is latched"
        );
        clear(&mut ucs, &mut sfr);
        assert_eq!(ucs.read_u16(0x0E).unwrap() & UCS_FAULTS, 0);
        assert_eq!(sfr.read_u16(SFRIFG1).unwrap() & 0x0002, 0);
        // Switching XT2 on with no crystal is a fault again.
        ucs.write_u16(0x0C, 0xC0CD).unwrap();
        assert_eq!(ucs.read_u16(0x0E).unwrap() & UCS_FAULTS, XT2OFFG);
        assert_eq!(sfr.read_u16(SFRIFG1).unwrap() & 0x0002, 0x0002);
        // The fault flags outlast a PUC.
        ucs.on_system_reset();
        assert_eq!(
            ucs.read_u16(0x0E).unwrap() & UCS_FAULTS,
            XT2OFFG | XT1LFOFFG,
            "XT1 is selected again after the PUC"
        );
    }

    const PMMCTL0: u64 = 0x00;
    const SVSMHCTL: u64 = 0x04;
    const SVSMLCTL: u64 = 0x06;
    const PMMIFG: u64 = 0x0C;
    const PMMRIE: u64 = 0x0E;

    #[test]
    fn pmm_password_and_lock() {
        let mut pmm = Msp430Pmm::new();
        assert_eq!(pmm.read_u16(PMMCTL0).unwrap(), 0x9600);
        assert_eq!(pmm.read_u16(SVSMHCTL).unwrap(), 0x4400);
        assert_eq!(pmm.read_u16(SVSMLCTL).unwrap(), 0x4400);
        assert_eq!(pmm.read_u16(PMMRIE).unwrap(), 0x1100);
        // Locked: stores are ignored.
        pmm.write_u16(SVSMLCTL, 0x4401).unwrap();
        pmm.write(PMMCTL0, 0x01).unwrap();
        assert_eq!(pmm.read_u16(SVSMLCTL).unwrap(), 0x4400);
        assert_eq!(pmm.read_u16(PMMCTL0).unwrap(), 0x9600);
        // A byte store of the password unlocks, any other byte locks.
        pmm.write(PMMCTL0 + 1, 0xA5).unwrap();
        pmm.write(PMMCTL0, 0x01).unwrap();
        assert_eq!(pmm.read_u16(PMMCTL0).unwrap(), 0x9601);
        pmm.write(PMMCTL0 + 1, 0x00).unwrap();
        pmm.write(PMMCTL0, 0x02).unwrap();
        assert_eq!(pmm.read_u16(PMMCTL0).unwrap(), 0x9601);
        assert!(!pmm.take_reset_request());
        // A word store needs the password.
        pmm.write_u16(PMMCTL0, 0xA502).unwrap();
        assert_eq!(pmm.read_u16(PMMCTL0).unwrap(), 0x9602);
        assert!(!pmm.take_reset_request());
        pmm.write_u16(PMMCTL0, 0x0003).unwrap();
        assert_eq!(pmm.read_u16(PMMCTL0).unwrap(), 0x9602);
        assert!(pmm.take_reset_request(), "key violation");
        // PMMSWBOR is self-clearing and requests a reset; a PUC locks.
        pmm.write_u16(PMMCTL0, 0xA506).unwrap();
        assert_eq!(pmm.read_u16(PMMCTL0).unwrap(), 0x9602);
        assert_eq!(
            pmm.latched_reset_cause(),
            Some(PeripheralResetCause::System)
        );
        assert!(pmm.take_reset_request());
        pmm.on_system_reset();
        pmm.write(PMMCTL0, 0x00).unwrap();
        assert_eq!(pmm.read_u16(PMMCTL0).unwrap(), 0x9602, "locked, level kept");
    }

    /// The core voltage step of SLAU208 2.2.4 (`SetVCoreUp`).
    #[test]
    fn pmm_core_voltage_step_sets_the_low_side_flags() {
        let mut pmm = Msp430Pmm::new();
        let level = 1u16;
        pmm.write(PMMCTL0 + 1, 0xA5).unwrap();
        // SVSHE | SVSHRVL = level | SVMHE | SVSMHRRL = level.
        pmm.write_u16(SVSMHCTL, 0x4400 | level << 8 | level)
            .unwrap();
        assert_eq!(pmm.read_u16(PMMIFG).unwrap(), SVSMHDLYIFG);
        // SVSLE | SVMLE | SVSMLRRL = level: the monitor is above the core.
        pmm.write_u16(SVSMLCTL, 0x4400 | level).unwrap();
        assert_eq!(
            pmm.read_u16(PMMIFG).unwrap(),
            SVSMHDLYIFG | SVSMLDLYIFG | SVMLIFG
        );
        pmm.write_u16(PMMIFG, 0).unwrap();
        pmm.write(PMMCTL0, level as u8).unwrap();
        assert_eq!(pmm.read_u16(PMMIFG).unwrap(), SVMLVLRIFG, "level reached");
        pmm.write_u16(SVSMLCTL, 0x4400 | level << 8 | level)
            .unwrap();
        pmm.write(PMMCTL0 + 1, 0).unwrap();
        assert_eq!(pmm.read_u16(SVSMLCTL).unwrap(), 0x4501);
        assert_eq!(pmm.read_u16(PMMCTL0).unwrap(), 0x9601);
    }
}
