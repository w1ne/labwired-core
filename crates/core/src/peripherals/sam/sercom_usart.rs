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
            trace: crate::bus::bus_trace::BusTrace::default(),
            trace_name: String::new(),
        }
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
        if self.enabled() {
            flags |= INT_DRE;
        }
        if self.enabled() && self.ctrlb & CTRLB_RXEN != 0 && self.rx_queued() {
            flags |= INT_RXC;
        } else {
            flags &= !INT_RXC;
        }
        flags
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
        self.emit_byte((value & 0xFF) as u8);
        // Transmission is complete by the time the store returns.
        self.intflag |= INT_TXC;
    }

    /// A DATA read: hand over the oldest queued byte and clear RXC by
    /// consuming it. Reading with nothing queued returns the holding register,
    /// as silicon does — not a fabricated zero.
    fn read_data(&self) -> u16 {
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

    fn tick(&mut self) -> PeripheralTickResult {
        let asserted = self.intenset & self.effective_intflag() != 0;
        let edge = asserted && !self.irq_level;
        self.irq_level = asserted;
        PeripheralTickResult {
            irq: edge,
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
            }
            CTRLB => self.ctrlb = value,
            BAUD => self.baud = (value & 0xFFFF) as u16,
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
