// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 UARTE (UART with EasyDMA) — and the legacy UART it superseded.
//!
//! Source: nRF52840 PS rev 1.7 §6.33 (UARTE) and §6.34 (UART). Both instances
//! share one MMIO window on silicon (UART0/UARTE0 at 0x4000_2000): a firmware
//! selects the personality with ENABLE (4 = legacy UART, 8 = UARTE). Their
//! register maps overlap except for the data path — legacy UART has single-byte
//! RXD (0x518) / TXD (0x51C) shift registers, whereas UARTE uses EasyDMA
//! pointer/maxcnt/amount blocks. This one model serves both so an image built
//! against either driver boots.
//!
//! Models the full register surface including PSEL, BAUDRATE, CONFIG and the DMA
//! pointer/maxcnt/amount registers used by zephyr/nrfx drivers, plus the legacy
//! single-byte TXD path used by the Adafruit/Arduino nRF52 core. Dynamic RX is
//! not modelled — firmware that programs the peripheral and reads config
//! registers back will see its writes round-trip.
//!
//! EVENTS: hardware-generated. SW write-1 is ignored; write-0 clears.

use crate::{Bus, Peripheral, SimResult};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

// Task offsets (read-as-0, task starts on write-1)
const OFF_TASKS_STARTRX: u64 = 0x000;
const OFF_TASKS_STOPRX: u64 = 0x004;
const OFF_TASKS_STARTTX: u64 = 0x008;
const OFF_TASKS_STOPTX: u64 = 0x00C;
const OFF_TASKS_FLUSHRX: u64 = 0x02C;

// Event offsets (0x100..0x17C)
const OFF_EVENTS_CTS: u64 = 0x100;
const OFF_EVENTS_NCTS: u64 = 0x104;
const OFF_EVENTS_RXDRDY: u64 = 0x108;
const OFF_EVENTS_ENDRX: u64 = 0x110;
const OFF_EVENTS_TXDRDY: u64 = 0x11C;
const OFF_EVENTS_ENDTX: u64 = 0x120;
const OFF_EVENTS_ERROR: u64 = 0x124;
const OFF_EVENTS_RXTO: u64 = 0x144;
const OFF_EVENTS_RXSTARTED: u64 = 0x14C;
const OFF_EVENTS_TXSTARTED: u64 = 0x150;
const OFF_EVENTS_TXSTOPPED: u64 = 0x158;

// Interrupt registers
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;

// Error source (write-1-clear)
const OFF_ERRORSRC: u64 = 0x480;

// Enable — 4 selects the legacy UART personality, 8 selects UARTE (EasyDMA).
const OFF_ENABLE: u64 = 0x500;
const ENABLE_UART_LEGACY: u32 = 4;
#[cfg(test)]
const ENABLE_UARTE: u32 = 8;

// Legacy UART single-byte data registers (PS §6.34.13). Present only in the
// legacy personality; UARTE reuses this address range for EasyDMA and never
// touches these two words.
const OFF_RXD_LEGACY: u64 = 0x518;
const OFF_TXD_LEGACY: u64 = 0x51C;

// PSEL block (0x508..0x518): RTS, TXD, CTS, RXD — reset value = 0xFFFF_FFFF (disconnected)
const OFF_PSEL_RTS: u64 = 0x508;
const OFF_PSEL_TXD: u64 = 0x50C;
const OFF_PSEL_CTS: u64 = 0x510;
const OFF_PSEL_RXD: u64 = 0x514;

// BAUDRATE — reset value is BAUD_115200 = 0x01D7E000
const OFF_BAUDRATE: u64 = 0x524;

// RXD EasyDMA block
const OFF_RXD_PTR: u64 = 0x534;
const OFF_RXD_MAXCNT: u64 = 0x538;
const OFF_RXD_AMOUNT: u64 = 0x53C;

// TXD EasyDMA block
const OFF_TXD_PTR: u64 = 0x544;
const OFF_TXD_MAXCNT: u64 = 0x548;
const OFF_TXD_AMOUNT: u64 = 0x54C;

// CONFIG: bits [3:0] = hwfc|parity, bit 4 = paritytype; reset = 0
const OFF_CONFIG: u64 = 0x56C;

#[derive(Default)]
pub struct Nrf52Uarte {
    // Events (TASKS always read 0)
    events_cts: u32,
    events_ncts: u32,
    events_rxdrdy: u32,
    events_endrx: u32,
    events_txdrdy: u32,
    events_endtx: u32,
    events_error: u32,
    events_rxto: u32,
    events_rxstarted: u32,
    events_txstarted: u32,
    events_txstopped: u32,
    // Config / status
    inten: u32,
    errorsrc: u32,
    enable: u32,
    psel_rts: u32,
    psel_txd: u32,
    psel_cts: u32,
    psel_rxd: u32,
    baudrate: u32,
    // DMA registers (all read-write, no side effects in sim)
    rxd_ptr: u32,
    rxd_maxcnt: u32,
    rxd_amount: u32,
    txd_ptr: u32,
    txd_maxcnt: u32,
    txd_amount: u32,
    config: u32,
    // Overflow bucket for any unmodelled register
    extra: BTreeMap<u64, u32>,
    // ── Dynamic EasyDMA TX state (not part of the register surface) ──────
    /// Set by a STARTTX task write; consumed by the next `tick_with_bus`,
    /// which DMA-reads the TXD buffer from RAM and emits it. The transfer is
    /// deferred to the bus-aware tick because `write_u32` has no bus handle.
    tx_pending: bool,
    /// Captured TX bytes for `test`-mode assertions (`uart_contains`).
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    /// Echo transmitted bytes to the process stdout (console behaviour).
    echo_stdout: bool,
}

impl std::fmt::Debug for Nrf52Uarte {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Nrf52Uarte")
            .field("enable", &self.enable)
            .field("txd_ptr", &self.txd_ptr)
            .field("txd_maxcnt", &self.txd_maxcnt)
            .field("tx_pending", &self.tx_pending)
            .finish()
    }
}

impl Nrf52Uarte {
    pub fn new() -> Self {
        Self {
            // PSELs reset to disconnected (all bits set = 0xFFFF_FFFF)
            psel_rts: 0xFFFF_FFFF,
            psel_txd: 0xFFFF_FFFF,
            psel_cts: 0xFFFF_FFFF,
            psel_rxd: 0xFFFF_FFFF,
            // BAUDRATE reset: BAUD_115200
            baudrate: 0x01D7_E000,
            // Default to console echo; capture sink attached on demand.
            echo_stdout: true,
            ..Self::default()
        }
    }

    /// Attach a capture sink and/or toggle stdout echo. Mirrors `Uart::set_sink`
    /// so `Bus::attach_uart_tx_sink` can wire a UARTE console the same way it
    /// wires the legacy UART.
    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }

    fn emit_byte(&mut self, byte: u8) {
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
}

impl Peripheral for Nrf52Uarte {

    /// Not in the per-cycle walk: this model overrides neither `tick()` nor
    /// `tick_elapsed()`, so every visit ran the default no-op and returned a
    /// default `PeripheralTickResult`. Skipping it removes dispatch, never an
    /// effect — byte-identical by construction.
    ///
    /// Safe against the "sleeps and never wakes" trap: the bus calls
    /// `refresh_legacy_tick_index()` on every MMIO write, so if this model ever
    /// gains a tick and a state-dependent condition, a firmware write re-arms it.
    fn legacy_tick_active(&self) -> bool {
        false
    }
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // Tasks: always read 0
            OFF_TASKS_STARTRX | OFF_TASKS_STOPRX | OFF_TASKS_STARTTX | OFF_TASKS_STOPTX
            | OFF_TASKS_FLUSHRX => 0,
            // Events
            OFF_EVENTS_CTS => self.events_cts,
            OFF_EVENTS_NCTS => self.events_ncts,
            OFF_EVENTS_RXDRDY => self.events_rxdrdy,
            OFF_EVENTS_ENDRX => self.events_endrx,
            OFF_EVENTS_TXDRDY => self.events_txdrdy,
            OFF_EVENTS_ENDTX => self.events_endtx,
            OFF_EVENTS_ERROR => self.events_error,
            OFF_EVENTS_RXTO => self.events_rxto,
            OFF_EVENTS_RXSTARTED => self.events_rxstarted,
            OFF_EVENTS_TXSTARTED => self.events_txstarted,
            OFF_EVENTS_TXSTOPPED => self.events_txstopped,
            // Interrupts
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            // Status
            OFF_ERRORSRC => self.errorsrc,
            OFF_ENABLE => self.enable & 0xF,
            // PSEL
            OFF_PSEL_RTS => self.psel_rts,
            OFF_PSEL_TXD => self.psel_txd,
            OFF_PSEL_CTS => self.psel_cts,
            OFF_PSEL_RXD => self.psel_rxd,
            // BAUDRATE
            OFF_BAUDRATE => self.baudrate,
            // DMA
            OFF_RXD_PTR => self.rxd_ptr,
            OFF_RXD_MAXCNT => self.rxd_maxcnt & 0xFFFF,
            OFF_RXD_AMOUNT => self.rxd_amount & 0xFFFF,
            OFF_TXD_PTR => self.txd_ptr,
            OFF_TXD_MAXCNT => self.txd_maxcnt & 0xFFFF,
            OFF_TXD_AMOUNT => self.txd_amount & 0xFFFF,
            // CONFIG: bits [4:0]
            OFF_CONFIG => self.config & 0x1F,
            // Legacy UART data: TXD is write-only (reads 0), RXD has no modelled
            // receiver so it reads 0 (no byte pending).
            OFF_TXD_LEGACY | OFF_RXD_LEGACY => 0,
            _ => self.extra.get(&offset).copied().unwrap_or(0),
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // STARTTX arms an EasyDMA TX; the actual buffer read + emit happens
            // in `tick_with_bus` (write_u32 has no bus handle). A 1-byte
            // poll_out and a multi-byte buffered write both land here. Only the
            // UARTE personality uses EasyDMA — in legacy UART mode STARTTX just
            // enables the transmitter and bytes flow through the TXD register.
            OFF_TASKS_STARTTX if self.enable != ENABLE_UART_LEGACY => self.tx_pending = true,
            OFF_TASKS_STARTTX => {}
            // STOPTX completes immediately in this model: raise TXSTOPPED so a
            // driver waiting on it (nrfx is_tx_ready) makes progress.
            OFF_TASKS_STOPTX => self.events_txstopped = 1,
            // Remaining tasks are RX/flush — no TX-path effect yet.
            OFF_TASKS_STARTRX | OFF_TASKS_STOPRX | OFF_TASKS_FLUSHRX => {}
            // EVENTS: hardware-generated; SW write-1 ignored, write-0 clears
            OFF_EVENTS_CTS if value == 0 => self.events_cts = 0,
            OFF_EVENTS_NCTS if value == 0 => self.events_ncts = 0,
            OFF_EVENTS_RXDRDY if value == 0 => self.events_rxdrdy = 0,
            OFF_EVENTS_ENDRX if value == 0 => self.events_endrx = 0,
            OFF_EVENTS_TXDRDY if value == 0 => self.events_txdrdy = 0,
            OFF_EVENTS_ENDTX if value == 0 => self.events_endtx = 0,
            OFF_EVENTS_ERROR if value == 0 => self.events_error = 0,
            OFF_EVENTS_RXTO if value == 0 => self.events_rxto = 0,
            OFF_EVENTS_RXSTARTED if value == 0 => self.events_rxstarted = 0,
            OFF_EVENTS_TXSTARTED if value == 0 => self.events_txstarted = 0,
            OFF_EVENTS_TXSTOPPED if value == 0 => self.events_txstopped = 0,
            // Interrupts
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            // ERRORSRC: write-1-clear
            OFF_ERRORSRC => self.errorsrc &= !value,
            // Enable
            OFF_ENABLE => self.enable = value & 0xF,
            // Legacy UART TXD (PS §6.34): writing a byte transmits it through the
            // shift register and, once the shifter is free for the next byte,
            // raises EVENTS_TXDRDY. The Adafruit/Arduino nRF52 Uart::write does
            // `TXD = byte; while (EVENTS_TXDRDY == 0); EVENTS_TXDRDY = 0`, so the
            // byte must land in the sink and TXDRDY must go high or it spins
            // forever.
            // FIDELITY: modeled, NOT HW-validated (2026-07-04) — legacy UART
            // TXD (0x51C) → EVENTS_TXDRDY (0x11C). nRF52840 PS rev 1.7 §6.34.
            // Transfer is instantaneous (byte out, TXDRDY immediately); real
            // silicon raises TXDRDY only after the stop bit at the configured
            // baud, and TX must have been armed by TASKS_STARTTX.
            OFF_TXD_LEGACY => {
                self.emit_byte(value as u8);
                self.events_txdrdy = 1;
            }
            // RXD is a read-only receive register; writes are ignored.
            OFF_RXD_LEGACY => {}
            // PSEL
            OFF_PSEL_RTS => self.psel_rts = value,
            OFF_PSEL_TXD => self.psel_txd = value,
            OFF_PSEL_CTS => self.psel_cts = value,
            OFF_PSEL_RXD => self.psel_rxd = value,
            // BAUDRATE
            OFF_BAUDRATE => self.baudrate = value,
            // DMA
            OFF_RXD_PTR => self.rxd_ptr = value,
            OFF_RXD_MAXCNT => self.rxd_maxcnt = value & 0xFFFF,
            OFF_RXD_AMOUNT => {} // RO, driven by DMA hardware
            OFF_TXD_PTR => self.txd_ptr = value,
            OFF_TXD_MAXCNT => self.txd_maxcnt = value & 0xFFFF,
            OFF_TXD_AMOUNT => {} // RO
            // CONFIG
            OFF_CONFIG => self.config = value & 0x1F,
            _ => {
                self.extra.insert(offset, value);
            }
        }
        Ok(())
    }

    /// EasyDMA needs to read the firmware-owned TX buffer out of RAM, which is
    /// only reachable with a bus handle — so the transfer is performed here,
    /// in the bus-aware pre-tick pass, rather than in `write_u32`.
    fn needs_bus_tick(&self) -> bool {
        self.tx_pending
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if !self.tx_pending {
            return;
        }
        self.tx_pending = false;

        // EasyDMA reads MAXCNT bytes starting at TXD.PTR. A disconnected pin or
        // a disabled peripheral still completes the transfer on real silicon
        // (the bytes just go nowhere), so we don't gate on PSEL.
        let len = (self.txd_maxcnt & 0xFFFF) as usize;
        for i in 0..len {
            let addr = self.txd_ptr as u64 + i as u64;
            if let Ok(b) = bus.read_u8(addr) {
                self.emit_byte(b);
            }
        }
        self.txd_amount = len as u32;

        // Raise the TX-path events a polling driver waits on. The transfer is
        // modelled as instantaneous (whole buffer in one tick), so all of the
        // begin→drain→stop events fire together: TXSTARTED, then TXDRDY/ENDTX,
        // then TXSTOPPED. nrfx's poll_out enables the ENDTX_STOPTX short and
        // waits on TXSTOPPED, so that one must be set or it spins forever.
        self.events_txstarted = 1;
        self.events_txdrdy = 1;
        self.events_endtx = 1;
        self.events_txstopped = 1;
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

    #[test]
    fn easydma_tx_emits_buffer_and_raises_completion_events() {
        use crate::bus::SystemBus;
        use crate::memory::LinearMemory;
        use crate::Bus;

        // RAM-backed bus holding the TX buffer "Hi" at 0x2000_0010.
        let mut bus = SystemBus::empty();
        bus.ram = LinearMemory::new(256, 0x2000_0000);
        bus.write_u8(0x2000_0010, b'H').unwrap();
        bus.write_u8(0x2000_0011, b'i').unwrap();

        let mut u = Nrf52Uarte::new();
        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        u.set_sink(Some(sink.clone()), false); // capture only, no stdout echo

        u.write_u32(OFF_ENABLE, 8).unwrap(); // UARTE mode
        u.write_u32(OFF_TXD_PTR, 0x2000_0010).unwrap();
        u.write_u32(OFF_TXD_MAXCNT, 2).unwrap();
        assert!(!u.needs_bus_tick(), "no DMA armed before STARTTX");

        u.write_u32(OFF_TASKS_STARTTX, 1).unwrap();
        assert!(u.needs_bus_tick(), "STARTTX arms the EasyDMA");
        u.tick_with_bus(&mut bus);

        assert_eq!(&*sink.lock().unwrap(), b"Hi", "buffer DMAed out of RAM");
        assert_eq!(u.read_u32(OFF_TXD_AMOUNT).unwrap(), 2);
        // poll_out (ENDTX_STOPTX short) waits on these — all must be set.
        assert_eq!(u.read_u32(OFF_EVENTS_ENDTX).unwrap(), 1);
        assert_eq!(u.read_u32(OFF_EVENTS_TXSTARTED).unwrap(), 1);
        assert_eq!(u.read_u32(OFF_EVENTS_TXSTOPPED).unwrap(), 1);
        assert!(!u.needs_bus_tick(), "transfer consumes the pending flag");
    }

    #[test]
    fn legacy_uart_txd_emits_byte_and_raises_txdrdy() {
        let mut u = Nrf52Uarte::new();
        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        u.set_sink(Some(sink.clone()), false);

        // Legacy personality: ENABLE = 4.
        u.write_u32(OFF_ENABLE, ENABLE_UART_LEGACY).unwrap();
        // TXDRDY starts clear; the write must set it (matching the poll loop).
        assert_eq!(u.read_u32(OFF_EVENTS_TXDRDY).unwrap(), 0);
        u.write_u32(OFF_TXD_LEGACY, b'A' as u32).unwrap();
        assert_eq!(&*sink.lock().unwrap(), b"A", "TXD byte reached the sink");
        assert_eq!(u.read_u32(OFF_EVENTS_TXDRDY).unwrap(), 1, "TXDRDY raised");
        assert_eq!(u.read_u32(OFF_TXD_LEGACY).unwrap(), 0, "TXD reads as 0");

        // Driver clears TXDRDY (write-0) before the next byte.
        u.write_u32(OFF_EVENTS_TXDRDY, 0).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTS_TXDRDY).unwrap(), 0);
        u.write_u32(OFF_TXD_LEGACY, b'B' as u32).unwrap();
        assert_eq!(&*sink.lock().unwrap(), b"AB");
        assert_eq!(u.read_u32(OFF_EVENTS_TXDRDY).unwrap(), 1);
    }

    #[test]
    fn legacy_starttx_does_not_arm_easydma() {
        let mut u = Nrf52Uarte::new();
        u.write_u32(OFF_ENABLE, ENABLE_UART_LEGACY).unwrap();
        u.write_u32(OFF_TASKS_STARTTX, 1).unwrap();
        assert!(
            !u.needs_bus_tick(),
            "legacy mode STARTTX must not trigger an EasyDMA transfer"
        );
    }

    #[test]
    fn uarte_starttx_still_arms_easydma() {
        let mut u = Nrf52Uarte::new();
        u.write_u32(OFF_ENABLE, ENABLE_UARTE).unwrap();
        u.write_u32(OFF_TASKS_STARTTX, 1).unwrap();
        assert!(u.needs_bus_tick(), "UARTE mode STARTTX arms EasyDMA");
    }

    #[test]
    fn psel_defaults_to_disconnected() {
        let u = Nrf52Uarte::new();
        assert_eq!(u.read_u32(OFF_PSEL_TXD).unwrap(), 0xFFFF_FFFF);
        assert_eq!(u.read_u32(OFF_PSEL_RXD).unwrap(), 0xFFFF_FFFF);
    }

    #[test]
    fn baudrate_reset_is_115200() {
        let u = Nrf52Uarte::new();
        assert_eq!(u.read_u32(OFF_BAUDRATE).unwrap(), 0x01D7_E000);
    }

    #[test]
    fn psel_txd_roundtrips() {
        let mut u = Nrf52Uarte::new();
        u.write_u32(OFF_PSEL_TXD, 6).unwrap();
        assert_eq!(u.read_u32(OFF_PSEL_TXD).unwrap(), 6);
    }

    #[test]
    fn events_write_1_ignored() {
        let mut u = Nrf52Uarte::new();
        u.write_u32(OFF_EVENTS_TXDRDY, 1).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTS_TXDRDY).unwrap(), 0);
    }

    #[test]
    fn events_write_0_clears() {
        let mut u = Nrf52Uarte::new();
        // Simulate HW setting event (by direct field access in test)
        u.events_txdrdy = 1;
        u.write_u32(OFF_EVENTS_TXDRDY, 0).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTS_TXDRDY).unwrap(), 0);
    }
}
