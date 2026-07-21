// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF54L UARTE (UART with EasyDMA), the nRF54L-generation layout.
//!
//! Source: Nordic MDK SVD `nrf54l15_application.svd`, peripheral
//! `GLOBAL_UARTE20_S` (register layout derived from `GLOBAL_UARTE00_NS`),
//! cross-checked against the Zephyr `uarte_nrfx_uarte.c` driver for access
//! semantics.
//!
//! **This is NOT the nRF52 UARTE with a different base address.** The nRF54L
//! family moved EasyDMA into a `DMA.{RX,TX}` register cluster and renumbered
//! the whole task/event surface:
//!
//! | function            | nRF52 | nRF54L |
//! |---------------------|-------|--------|
//! | start TX transfer   | 0x008 | 0x050  (`TASKS_DMA.TX.START`)  |
//! | TX complete event   | 0x120 | 0x168  (`EVENTS_DMA.TX.END`)   |
//! | TX stopped event    | 0x158 | 0x130  (`EVENTS_TXSTOPPED`)    |
//! | TX buffer pointer   | 0x544 | 0x73C  (`DMA.TX.PTR`)          |
//! | TX buffer length    | 0x548 | 0x740  (`DMA.TX.MAXCNT`)       |
//!
//! `ENABLE` (0x500) and `BAUDRATE` (0x524) happen to sit at the same offsets
//! on both generations, which is exactly what disguised the incompatibility:
//! an nRF52 model reused here accepts the enable/baud writes and then never
//! sees the DMA start task, so the firmware hangs.
//!
//! Zero-length TX: Zephyr's `uarte_tx_path_init()` deliberately arms a 0-byte
//! transfer to drive the TX path into a known stopped state, then spins on
//! `EVENTS_TXSTOPPED`. A model that only completes transfers with `len > 0`
//! hangs the boot forever, so a zero-length transfer completes and raises the
//! same completion events as any other.
//!
//! EVENTS: hardware-generated. SW write-1 is ignored; write-0 clears. Each
//! event register at `0x100 + 4*n` is gated by INTEN bit `n` — that mapping is
//! exact on this family (SVD: CTS=0, NCTS=1, TXDRDY=3, RXDRDY=4, ERROR=5,
//! RXTO=9, TXSTOPPED=12, DMARXEND=19, DMATXEND=26, FRAMETIMEOUT=29).
//!
//! Not modelled: DPPI routing. The SUBSCRIBE_* (0x09C..0x0D4) and PUBLISH_*
//! (0x180..0x1F4) windows accept writes and read back, but connecting a
//! channel has no effect. RX has no input source — the RX tasks are accepted
//! and leave the RX events clear rather than fabricating received data.

use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

// ── Tasks (read-as-0, task starts on write-1) ───────────────────────────────
const OFF_TASKS_FLUSHRX: u64 = 0x01C;
const OFF_TASKS_DMA_RX_START: u64 = 0x028;
const OFF_TASKS_DMA_RX_STOP: u64 = 0x02C;
const OFF_TASKS_DMA_TX_START: u64 = 0x050;
const OFF_TASKS_DMA_TX_STOP: u64 = 0x054;

// ── Events (0x100..0x17C) ───────────────────────────────────────────────────
// Stored in one array indexed by `(offset - 0x100) / 4`, which is also the
// INTEN bit position for that event (see module docs).
const EVENTS_BASE: u64 = 0x100;
const EVENTS_END: u64 = 0x17C;
const NUM_EVENTS: usize = 32;

#[cfg(test)]
const OFF_EVENTS_CTS: u64 = 0x100;
#[cfg(test)]
const OFF_EVENTS_NCTS: u64 = 0x104;
const OFF_EVENTS_TXDRDY: u64 = 0x10C;
#[cfg(test)]
const OFF_EVENTS_RXDRDY: u64 = 0x110;
#[cfg(test)]
const OFF_EVENTS_ERROR: u64 = 0x114;
#[cfg(test)]
const OFF_EVENTS_RXTO: u64 = 0x124;
const OFF_EVENTS_TXSTOPPED: u64 = 0x130;
#[cfg(test)]
const OFF_EVENTS_DMA_RX_END: u64 = 0x14C;
const OFF_EVENTS_DMA_TX_END: u64 = 0x168;
const OFF_EVENTS_DMA_TX_READY: u64 = 0x16C;
#[cfg(test)]
const OFF_EVENTS_FRAMETIMEOUT: u64 = 0x174;

/// Event index (== INTEN bit) for an event offset, or `None` when the offset
/// is outside the event window.
fn event_index(offset: u64) -> Option<usize> {
    if (EVENTS_BASE..=EVENTS_END).contains(&offset) && offset % 4 == 0 {
        Some(((offset - EVENTS_BASE) / 4) as usize)
    } else {
        None
    }
}

// ── Shorts / interrupts ─────────────────────────────────────────────────────
const OFF_SHORTS: u64 = 0x200;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;

// ── Status / configuration ──────────────────────────────────────────────────
const OFF_ERRORSRC: u64 = 0x480;
/// ENABLE: 8 = UARTE enabled, 0 = disabled. (The nRF52 legacy-UART
/// personality, ENABLE = 4, does not exist on this family.)
const OFF_ENABLE: u64 = 0x500;
const OFF_BAUDRATE: u64 = 0x524;
const OFF_CONFIG: u64 = 0x56C;
const OFF_ADDRESS: u64 = 0x574;
const OFF_FRAMETIMEOUT: u64 = 0x578;

// ── PSEL block (reset value = 0xFFFF_FFFF, disconnected) ────────────────────
const OFF_PSEL_TXD: u64 = 0x604;
const OFF_PSEL_CTS: u64 = 0x608;
const OFF_PSEL_RXD: u64 = 0x60C;
const OFF_PSEL_RTS: u64 = 0x610;

// ── DMA.RX cluster ──────────────────────────────────────────────────────────
const OFF_DMA_RX_PTR: u64 = 0x704;
const OFF_DMA_RX_MAXCNT: u64 = 0x708;
const OFF_DMA_RX_AMOUNT: u64 = 0x70C;
const OFF_DMA_RX_LIST: u64 = 0x714;
const OFF_DMA_RX_TERMINATEONBUSERROR: u64 = 0x71C;
const OFF_DMA_RX_BUSERRORADDRESS: u64 = 0x720;

// ── DMA.TX cluster ──────────────────────────────────────────────────────────
const OFF_DMA_TX_PTR: u64 = 0x73C;
const OFF_DMA_TX_MAXCNT: u64 = 0x740;
const OFF_DMA_TX_AMOUNT: u64 = 0x744;
const OFF_DMA_TX_LIST: u64 = 0x74C;
const OFF_DMA_TX_TERMINATEONBUSERROR: u64 = 0x754;
const OFF_DMA_TX_BUSERRORADDRESS: u64 = 0x758;

#[derive(Default)]
pub struct Nrf54lUarte {
    /// EVENTS_* registers, indexed by INTEN bit position.
    events: [u32; NUM_EVENTS],
    // Config / status
    shorts: u32,
    inten: u32,
    errorsrc: u32,
    enable: u32,
    baudrate: u32,
    config: u32,
    address: u32,
    frametimeout: u32,
    psel_txd: u32,
    psel_cts: u32,
    psel_rxd: u32,
    psel_rts: u32,
    // DMA.RX cluster (stored/read back; no RX source is modelled)
    dma_rx_ptr: u32,
    dma_rx_maxcnt: u32,
    dma_rx_amount: u32,
    dma_rx_list: u32,
    dma_rx_terminate_on_buserror: u32,
    dma_rx_buserroraddress: u32,
    // DMA.TX cluster
    dma_tx_ptr: u32,
    dma_tx_maxcnt: u32,
    dma_tx_amount: u32,
    dma_tx_list: u32,
    dma_tx_terminate_on_buserror: u32,
    dma_tx_buserroraddress: u32,
    /// Overflow bucket for any unmodelled register (SUBSCRIBE/PUBLISH DPPI
    /// windows and reserved words), so a write/readback never faults.
    extra: BTreeMap<u64, u32>,
    // ── Dynamic EasyDMA TX state (not part of the register surface) ─────────
    /// Set by a `TASKS_DMA.TX.START` write; consumed by the next
    /// `tick_with_bus`, which DMA-reads the TX buffer from RAM and emits it.
    /// The transfer is deferred to the bus-aware tick because `write_u32` has
    /// no bus handle.
    tx_pending: bool,
    /// Level of `inten & pending_events` at the last tick, so the IRQ is
    /// raised on the 0→1 edge instead of every cycle the event stays set.
    irq_level: bool,
    /// Captured TX bytes for `test`-mode assertions (`uart_contains`).
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    /// Echo transmitted bytes to the process stdout (console behaviour).
    echo_stdout: bool,
}

impl std::fmt::Debug for Nrf54lUarte {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Nrf54lUarte")
            .field("enable", &self.enable)
            .field("dma_tx_ptr", &self.dma_tx_ptr)
            .field("dma_tx_maxcnt", &self.dma_tx_maxcnt)
            .field("tx_pending", &self.tx_pending)
            .finish()
    }
}

impl Nrf54lUarte {
    pub fn new() -> Self {
        Self {
            // PSELs reset to disconnected (all bits set).
            psel_txd: 0xFFFF_FFFF,
            psel_cts: 0xFFFF_FFFF,
            psel_rxd: 0xFFFF_FFFF,
            psel_rts: 0xFFFF_FFFF,
            // BAUDRATE reset: BAUD_115200 (same encoding as nRF52).
            baudrate: 0x01D7_E000,
            // Default to console echo; capture sink attached on demand.
            echo_stdout: true,
            ..Self::default()
        }
    }

    /// Attach a capture sink and/or toggle stdout echo. Mirrors
    /// `Nrf52Uarte::set_sink` so `Bus::attach_uart_tx_sink` wires an nRF54L
    /// console exactly the same way.
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

    /// Raise an event by offset (hardware side; SW cannot do this).
    fn set_event(&mut self, offset: u64) {
        if let Some(i) = event_index(offset) {
            self.events[i] = 1;
        }
    }

    /// Bitmask of currently-pending events, in INTEN bit order.
    fn pending_mask(&self) -> u32 {
        let mut mask = 0u32;
        for (i, e) in self.events.iter().enumerate() {
            if *e != 0 {
                mask |= 1 << i;
            }
        }
        mask
    }
}

impl Peripheral for Nrf54lUarte {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        // Events first: one contiguous window, index == INTEN bit.
        if let Some(i) = event_index(offset) {
            return Ok(self.events[i]);
        }
        Ok(match offset {
            // Tasks always read 0.
            OFF_TASKS_FLUSHRX
            | OFF_TASKS_DMA_RX_START
            | OFF_TASKS_DMA_RX_STOP
            | OFF_TASKS_DMA_TX_START
            | OFF_TASKS_DMA_TX_STOP => 0,
            OFF_SHORTS => self.shorts,
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_ERRORSRC => self.errorsrc,
            OFF_ENABLE => self.enable & 0xF,
            OFF_BAUDRATE => self.baudrate,
            OFF_CONFIG => self.config,
            OFF_ADDRESS => self.address,
            OFF_FRAMETIMEOUT => self.frametimeout,
            OFF_PSEL_TXD => self.psel_txd,
            OFF_PSEL_CTS => self.psel_cts,
            OFF_PSEL_RXD => self.psel_rxd,
            OFF_PSEL_RTS => self.psel_rts,
            OFF_DMA_RX_PTR => self.dma_rx_ptr,
            OFF_DMA_RX_MAXCNT => self.dma_rx_maxcnt & 0xFFFF,
            OFF_DMA_RX_AMOUNT => self.dma_rx_amount & 0xFFFF,
            OFF_DMA_RX_LIST => self.dma_rx_list,
            OFF_DMA_RX_TERMINATEONBUSERROR => self.dma_rx_terminate_on_buserror & 1,
            OFF_DMA_RX_BUSERRORADDRESS => self.dma_rx_buserroraddress,
            OFF_DMA_TX_PTR => self.dma_tx_ptr,
            OFF_DMA_TX_MAXCNT => self.dma_tx_maxcnt & 0xFFFF,
            OFF_DMA_TX_AMOUNT => self.dma_tx_amount & 0xFFFF,
            OFF_DMA_TX_LIST => self.dma_tx_list,
            OFF_DMA_TX_TERMINATEONBUSERROR => self.dma_tx_terminate_on_buserror & 1,
            OFF_DMA_TX_BUSERRORADDRESS => self.dma_tx_buserroraddress,
            // Everything else (DPPI SUBSCRIBE/PUBLISH, reserved words, and any
            // offset inside the 4 KB window we do not model) reads back what
            // was written, or 0.
            _ => self.extra.get(&offset).copied().unwrap_or(0),
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // EVENTS_*: hardware-generated. SW write-1 ignored, write-0 clears.
        if let Some(i) = event_index(offset) {
            if value == 0 {
                self.events[i] = 0;
            }
            return Ok(());
        }
        match offset {
            // TASKS_DMA.TX.START arms an EasyDMA TX; the buffer read + emit
            // happens in `tick_with_bus` (write_u32 has no bus handle). This is
            // the register the nRF52 model never saw — its STARTTX was 0x008.
            OFF_TASKS_DMA_TX_START => self.tx_pending = true,
            // TASKS_DMA.TX.STOP completes immediately in this model: raise
            // TXSTOPPED so a driver waiting on it makes progress.
            OFF_TASKS_DMA_TX_STOP => self.set_event(OFF_EVENTS_TXSTOPPED),
            // RX tasks are accepted but inert: no RX source is modelled, so
            // fabricating RX events (or an ENDRX with junk data) would be a
            // lie. Leaving them clear is the honest state and does not hang a
            // driver that only ever polls the TX path.
            OFF_TASKS_DMA_RX_START | OFF_TASKS_DMA_RX_STOP | OFF_TASKS_FLUSHRX => {}
            OFF_SHORTS => self.shorts = value,
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            // ERRORSRC: write-1-clear.
            OFF_ERRORSRC => self.errorsrc &= !value,
            OFF_ENABLE => self.enable = value & 0xF,
            OFF_BAUDRATE => self.baudrate = value,
            OFF_CONFIG => self.config = value,
            OFF_ADDRESS => self.address = value,
            OFF_FRAMETIMEOUT => self.frametimeout = value,
            OFF_PSEL_TXD => self.psel_txd = value,
            OFF_PSEL_CTS => self.psel_cts = value,
            OFF_PSEL_RXD => self.psel_rxd = value,
            OFF_PSEL_RTS => self.psel_rts = value,
            OFF_DMA_RX_PTR => self.dma_rx_ptr = value,
            OFF_DMA_RX_MAXCNT => self.dma_rx_maxcnt = value & 0xFFFF,
            OFF_DMA_RX_AMOUNT => {} // RO, driven by DMA hardware
            OFF_DMA_RX_LIST => self.dma_rx_list = value,
            OFF_DMA_RX_TERMINATEONBUSERROR => self.dma_rx_terminate_on_buserror = value & 1,
            OFF_DMA_RX_BUSERRORADDRESS => {} // RO
            OFF_DMA_TX_PTR => self.dma_tx_ptr = value,
            OFF_DMA_TX_MAXCNT => self.dma_tx_maxcnt = value & 0xFFFF,
            OFF_DMA_TX_AMOUNT => {} // RO
            OFF_DMA_TX_LIST => self.dma_tx_list = value,
            OFF_DMA_TX_TERMINATEONBUSERROR => self.dma_tx_terminate_on_buserror = value & 1,
            OFF_DMA_TX_BUSERRORADDRESS => {} // RO
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

        // EasyDMA reads MAXCNT bytes starting at DMA.TX.PTR. A disconnected
        // pin or a disabled peripheral still completes the transfer on real
        // silicon (the bytes just go nowhere), so we don't gate on PSEL.
        //
        // MAXCNT == 0 is a REAL case, not a degenerate one: Zephyr's
        // `uarte_tx_path_init()` arms a 0-byte transfer purely to drive the TX
        // path into a stopped state and then spins on EVENTS_TXSTOPPED. The
        // loop below simply runs zero times and the completion events fire
        // exactly as they do for a non-empty buffer.
        let len = (self.dma_tx_maxcnt & 0xFFFF) as usize;
        for i in 0..len {
            let addr = self.dma_tx_ptr as u64 + i as u64;
            if let Ok(b) = bus.read_u8(addr) {
                self.emit_byte(b);
            }
        }
        self.dma_tx_amount = len as u32;

        // The transfer is modelled as instantaneous (whole buffer in one
        // tick), so the whole TX completion set fires together.
        // FIDELITY: modeled, NOT HW-validated (2026-07-20) — real silicon
        // spaces TXDRDY per character at the configured baud and raises
        // DMA.TX.END only after the last stop bit.
        self.set_event(OFF_EVENTS_TXDRDY);
        self.set_event(OFF_EVENTS_DMA_TX_END);
        self.set_event(OFF_EVENTS_DMA_TX_READY);
        self.set_event(OFF_EVENTS_TXSTOPPED);
    }

    /// Level-derived peripheral IRQ: asserted while any INTEN-enabled event is
    /// pending, emitted on the 0→1 edge so a pending event that the firmware
    /// has not yet cleared does not re-pend the NVIC every cycle.
    fn tick(&mut self) -> PeripheralTickResult {
        let level = self.pending_mask() & self.inten != 0;
        let irq = level && !self.irq_level;
        self.irq_level = level;
        PeripheralTickResult {
            irq,
            ..Default::default()
        }
    }

    /// `tick()` only tracks the level-held IRQ edge; when the level equals what
    /// was last observed it is a genuine no-op. Reporting that (instead of the
    /// always-active default) drops the UARTE out of the per-cycle walk while
    /// idle and — critically — lets idle fast-forward engage during a
    /// tickless-idle WFI window. Walk-identical: every skipped cycle is one
    /// where `tick()` would have recomputed the same level and emitted no IRQ.
    fn legacy_tick_active(&self) -> bool {
        (self.pending_mask() & self.inten != 0) != self.irq_level
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
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

    /// Build a UARTE with a capture sink (no stdout echo) plus a RAM-backed
    /// bus holding `bytes` at `addr`.
    fn fixture(
        addr: u64,
        bytes: &[u8],
    ) -> (
        Nrf54lUarte,
        crate::bus::SystemBus,
        std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    ) {
        use crate::bus::SystemBus;
        use crate::memory::LinearMemory;
        use crate::Bus;

        let mut bus = SystemBus::empty();
        bus.ram = LinearMemory::new(256, 0x2000_0000);
        for (i, b) in bytes.iter().enumerate() {
            bus.write_u8(addr + i as u64, *b).unwrap();
        }
        let mut u = Nrf54lUarte::new();
        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        u.set_sink(Some(sink.clone()), false);
        u.write_u32(OFF_ENABLE, 8).unwrap();
        (u, bus, sink)
    }

    #[test]
    fn easydma_tx_emits_buffer_and_raises_completion_events() {
        let (mut u, mut bus, sink) = fixture(0x2000_0010, b"Hi");

        u.write_u32(OFF_DMA_TX_PTR, 0x2000_0010).unwrap();
        u.write_u32(OFF_DMA_TX_MAXCNT, 2).unwrap();
        assert!(!u.needs_bus_tick(), "no DMA armed before TX.START");

        u.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();
        assert!(u.needs_bus_tick(), "TX.START (0x050) arms the EasyDMA");
        u.tick_with_bus(&mut bus);

        assert_eq!(&*sink.lock().unwrap(), b"Hi", "buffer DMAed out of RAM");
        assert_eq!(u.read_u32(OFF_DMA_TX_AMOUNT).unwrap(), 2);
        assert_eq!(u.read_u32(OFF_EVENTS_DMA_TX_END).unwrap(), 1);
        assert_eq!(u.read_u32(OFF_EVENTS_DMA_TX_READY).unwrap(), 1);
        assert_eq!(u.read_u32(OFF_EVENTS_TXSTOPPED).unwrap(), 1);
        assert!(!u.needs_bus_tick(), "transfer consumes the pending flag");
    }

    /// Regression: Zephyr's `uarte_tx_path_init()` arms MAXCNT = 0 and then
    /// spins on EVENTS_TXSTOPPED. If a zero-length transfer does not complete,
    /// unmodified `hello_world` never prints its banner.
    #[test]
    fn zero_length_tx_still_completes_and_raises_txstopped() {
        let (mut u, mut bus, sink) = fixture(0x2000_0010, b"");

        u.write_u32(OFF_DMA_TX_PTR, 0x2000_0010).unwrap();
        u.write_u32(OFF_DMA_TX_MAXCNT, 0).unwrap();
        u.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();
        u.tick_with_bus(&mut bus);

        assert!(sink.lock().unwrap().is_empty(), "no bytes for a 0-byte DMA");
        assert_eq!(u.read_u32(OFF_DMA_TX_AMOUNT).unwrap(), 0);
        assert_eq!(
            u.read_u32(OFF_EVENTS_DMA_TX_END).unwrap(),
            1,
            "DMA.TX.END must fire for a zero-length transfer"
        );
        assert_eq!(
            u.read_u32(OFF_EVENTS_TXSTOPPED).unwrap(),
            1,
            "TXSTOPPED must fire or uarte_tx_path_init() spins forever"
        );
    }

    #[test]
    fn tasks_dma_tx_stop_raises_txstopped() {
        let mut u = Nrf54lUarte::new();
        assert_eq!(u.read_u32(OFF_EVENTS_TXSTOPPED).unwrap(), 0);
        u.write_u32(OFF_TASKS_DMA_TX_STOP, 1).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTS_TXSTOPPED).unwrap(), 1);
    }

    #[test]
    fn events_write_1_ignored_write_0_clears() {
        let mut u = Nrf54lUarte::new();
        // SW cannot set an event.
        u.write_u32(OFF_EVENTS_TXDRDY, 1).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTS_TXDRDY).unwrap(), 0);
        // Hardware sets it; SW clears with a write of 0.
        u.set_event(OFF_EVENTS_TXDRDY);
        assert_eq!(u.read_u32(OFF_EVENTS_TXDRDY).unwrap(), 1);
        u.write_u32(OFF_EVENTS_TXDRDY, 0).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTS_TXDRDY).unwrap(), 0);
    }

    /// The event window is one contiguous block whose index is the INTEN bit;
    /// pin the offsets the SVD gives so a future edit cannot silently shift
    /// them back onto the nRF52 numbering.
    #[test]
    fn event_offsets_map_to_their_inten_bits() {
        for (off, bit) in [
            (OFF_EVENTS_CTS, 0),
            (OFF_EVENTS_NCTS, 1),
            (OFF_EVENTS_TXDRDY, 3),
            (OFF_EVENTS_RXDRDY, 4),
            (OFF_EVENTS_ERROR, 5),
            (OFF_EVENTS_RXTO, 9),
            (OFF_EVENTS_TXSTOPPED, 12),
            (OFF_EVENTS_DMA_RX_END, 19),
            (OFF_EVENTS_DMA_TX_END, 26),
            (OFF_EVENTS_DMA_TX_READY, 27),
            (OFF_EVENTS_FRAMETIMEOUT, 29),
        ] {
            assert_eq!(event_index(off), Some(bit), "offset {off:#x}");
        }
    }

    #[test]
    fn irq_raised_only_when_event_is_enabled() {
        let mut u = Nrf54lUarte::new();
        // Disabled: TXSTOPPED pends but the line stays low.
        u.write_u32(OFF_TASKS_DMA_TX_STOP, 1).unwrap();
        assert!(!u.tick().irq, "masked event must not raise the IRQ");

        // Enable TXSTOPPED (bit 12) via INTENSET → IRQ on the 0→1 edge, once.
        u.write_u32(OFF_INTENSET, 1 << 12).unwrap();
        assert_eq!(u.read_u32(OFF_INTEN).unwrap(), 1 << 12);
        assert!(u.tick().irq, "enabled pending event raises the IRQ");
        assert!(!u.tick().irq, "level stays high — no repeated pend");

        // Clearing the event drops the line; INTENCLR then masks it again.
        u.write_u32(OFF_EVENTS_TXSTOPPED, 0).unwrap();
        assert!(!u.tick().irq);
        u.write_u32(OFF_INTENCLR, 1 << 12).unwrap();
        assert_eq!(u.read_u32(OFF_INTEN).unwrap(), 0);
        u.write_u32(OFF_TASKS_DMA_TX_STOP, 1).unwrap();
        assert!(!u.tick().irq, "INTENCLR masks the event again");
    }

    #[test]
    fn config_registers_roundtrip() {
        let mut u = Nrf54lUarte::new();
        for (off, val) in [
            (OFF_PSEL_TXD, 0x0000_0106),
            (OFF_PSEL_RXD, 0x0000_0104),
            (OFF_PSEL_CTS, 0x8000_0000),
            (OFF_PSEL_RTS, 0x0000_0105),
            (OFF_BAUDRATE, 0x0EBE_DFA4),
            (OFF_CONFIG, 0x0000_000E),
            (OFF_SHORTS, 0x0000_0020),
            (OFF_ADDRESS, 0x0000_00AA),
            (OFF_FRAMETIMEOUT, 0x0000_1234),
            (OFF_DMA_TX_PTR, 0x2000_1234),
            (OFF_DMA_RX_PTR, 0x2000_5678),
        ] {
            u.write_u32(off, val).unwrap();
            assert_eq!(u.read_u32(off).unwrap(), val, "offset {off:#x}");
        }
        // MAXCNT registers are 16-bit on this family.
        u.write_u32(OFF_DMA_TX_MAXCNT, 0x1234).unwrap();
        assert_eq!(u.read_u32(OFF_DMA_TX_MAXCNT).unwrap(), 0x1234);
        u.write_u32(OFF_DMA_RX_MAXCNT, 0x0042).unwrap();
        assert_eq!(u.read_u32(OFF_DMA_RX_MAXCNT).unwrap(), 0x0042);
    }

    #[test]
    fn psel_defaults_to_disconnected_and_baudrate_to_115200() {
        let u = Nrf54lUarte::new();
        assert_eq!(u.read_u32(OFF_PSEL_TXD).unwrap(), 0xFFFF_FFFF);
        assert_eq!(u.read_u32(OFF_PSEL_RXD).unwrap(), 0xFFFF_FFFF);
        assert_eq!(u.read_u32(OFF_BAUDRATE).unwrap(), 0x01D7_E000);
    }

    #[test]
    fn unimplemented_offsets_read_zero_and_do_not_panic() {
        let mut u = Nrf54lUarte::new();
        // Reserved word, a DPPI SUBSCRIBE slot, a PUBLISH slot, and the very
        // top of the 4 KB window.
        for off in [0x008u64, 0x09C, 0x180, 0x1F4, 0xFFC, 0x400] {
            assert_eq!(u.read_u32(off).unwrap(), 0, "offset {off:#x} reads 0");
        }
        // DPPI windows accept writes and read back (no routing behaviour).
        u.write_u32(0x09C, 0x8000_0003).unwrap();
        assert_eq!(u.read_u32(0x09C).unwrap(), 0x8000_0003);
        u.write_u32(0x1F4, 0x8000_0005).unwrap();
        assert_eq!(u.read_u32(0x1F4).unwrap(), 0x8000_0005);
    }

    #[test]
    fn rx_tasks_are_accepted_and_leave_rx_events_clear() {
        let mut u = Nrf54lUarte::new();
        u.write_u32(OFF_DMA_RX_PTR, 0x2000_0000).unwrap();
        u.write_u32(OFF_DMA_RX_MAXCNT, 16).unwrap();
        u.write_u32(OFF_TASKS_DMA_RX_START, 1).unwrap();
        u.write_u32(OFF_TASKS_DMA_RX_STOP, 1).unwrap();
        u.write_u32(OFF_TASKS_FLUSHRX, 1).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTS_DMA_RX_END).unwrap(), 0);
        assert_eq!(u.read_u32(OFF_EVENTS_RXDRDY).unwrap(), 0);
        assert!(!u.needs_bus_tick(), "RX tasks must not arm the TX EasyDMA");
    }

    #[test]
    fn errorsrc_is_write_1_clear() {
        let mut u = Nrf54lUarte::new();
        u.errorsrc = 0b1111;
        u.write_u32(OFF_ERRORSRC, 0b0101).unwrap();
        assert_eq!(u.read_u32(OFF_ERRORSRC).unwrap(), 0b1010);
    }
}
