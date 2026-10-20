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
//! The transfer completes in the wake that runs it, not after a baud frame.
//! That wake is shared with the RX poll and the IRQ edge, and the forced walk
//! completes the buffer in one tick. The walk differential requires those two
//! paths to agree at every instruction, so a baud wait on this peripheral
//! would desync them and blow the smart-ring fixture's cycle window.
//!
//! EVENTS: hardware-generated. SW write-1 is ignored; write-0 clears. Each
//! event register at `0x100 + 4*n` is gated by INTEN bit `n` — that mapping is
//! exact on this family (SVD: CTS=0, NCTS=1, TXDRDY=3, RXDRDY=4, ERROR=5,
//! RXTO=9, TXSTOPPED=12, DMARXEND=19, DMATXEND=26, FRAMETIMEOUT=29).
//!
//! Not modelled: DPPI routing. The SUBSCRIBE_* (0x09C..0x0D4) and PUBLISH_*
//! (0x180..0x1F4) windows accept writes and read back, but connecting a
//! channel has no effect.
//!
//! RX: host-injected serial input arrives through the shared `rx_source`
//! queue (see `Bus::attach_uart_rx_source_named`). `TASKS_DMA.RX.START` arms
//! an EasyDMA drain — up to DMA.RX.MAXCNT queued bytes land in RAM at
//! DMA.RX.PTR, DMA.RX.AMOUNT is set and DMA.RX.END raised. RXDRDY reflects
//! queue-non-empty. The bare-bus `tick_with_bus` path only drains when the
//! queue is non-empty (no wake-on-inject there); bytes arriving after START
//! on a live machine are picked up because the bus re-arms the bus-tick
//! index on every MMIO write.

use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};
use std::collections::{BTreeMap, VecDeque};
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
const OFF_EVENTS_RXDRDY: u64 = 0x110;
#[cfg(test)]
const OFF_EVENTS_ERROR: u64 = 0x114;
#[cfg(test)]
const OFF_EVENTS_RXTO: u64 = 0x124;
const OFF_EVENTS_TXSTOPPED: u64 = 0x130;
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

/// The UARTE's single self-perpetuating scheduler event. One token is enough:
/// this model has no independent timers, only "there is work to do" — an armed
/// EasyDMA transfer, an RX buffer waiting for host-injected bytes, or an IRQ
/// level that has moved since the last observation.
const UARTE_WAKE_TOKEN: u32 = 0;

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
    /// Set by a `TASKS_DMA.RX.START` write; consumed by `tick_with_bus` once
    /// the RX queue has bytes to drain.
    rx_pending: bool,
    /// Host-injected serial input, shared with the runner via
    /// `Bus::attach_uart_rx_source_named`.
    rx_source: Arc<Mutex<VecDeque<u8>>>,
    /// Level of `inten & pending_events` at the last tick, so the IRQ is
    /// raised on the 0→1 edge instead of every cycle the event stays set.
    irq_level: bool,
    /// Cycle clock handed over by the bus at registration on an
    /// `event-scheduler` build. Its presence — not the feature alone — is what
    /// [`Self::scheduler_mode`] reads, so a hand-built bus that never attaches
    /// one keeps the exact historical walk semantics.
    clock: Option<crate::cycle_clock::CycleClock>,
    /// True while a WAKE event is in flight. `take_scheduled_events` runs after
    /// EVERY MMIO write to this peripheral, so without this a driver that pokes
    /// four registers between transfers would stack four redundant wakeups.
    scheduled: bool,
    /// Test-only lever: pin this model back onto the per-cycle walk even
    /// though the bus attached a clock. The walk-differential oracle needs the
    /// SAME machine built twice, differing ONLY in which path drives this
    /// peripheral; without a lever like this the reference lane would have to
    /// be a differently-assembled bus, and the comparison would no longer be
    /// about the migration. Mirrors `Uart::force_legacy_walk`.
    legacy_walk_forced: bool,
    /// Captured TX bytes for `test`-mode assertions (`uart_contains`).
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    /// Echo transmitted bytes to the process stdout (console behaviour).
    echo_stdout: bool,
    /// The machine's ONE bus trace and this instance's name in it; see
    /// [`crate::bus::bus_trace`]. Private until `attach_bus_trace` hands over
    /// the shared handle at registration.
    trace: crate::bus::bus_trace::BusTrace,
    trace_name: String,
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

    /// Shared handle to the RX injection queue, mirroring
    /// `Nrf52Uarte::rx_buffer` so `Bus::attach_uart_rx_source_named` can
    /// drive nRF54L serial input the same way.
    pub fn rx_buffer(&self) -> Arc<Mutex<VecDeque<u8>>> {
        self.rx_source.clone()
    }

    fn rx_queued(&self) -> usize {
        self.rx_source.lock().map(|q| q.len()).unwrap_or(0)
    }

    fn emit_byte(&mut self, byte: u8) {
        // The one place a TX byte leaves this UARTE, so the one place it is
        // traced. See `attach_bus_trace`.
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

    /// EasyDMA RX: drain up to DMA.RX.MAXCNT bytes from the injection queue
    /// into RAM at DMA.RX.PTR in one shot (instantaneous, like TX), set
    /// DMA.RX.AMOUNT and raise RXDRDY/DMA.RX.END. Callers guarantee the queue
    /// is non-empty; `rx_pending` is consumed here.
    fn do_dma_rx(&mut self, bus: &mut dyn Bus) {
        self.rx_pending = false;

        let max = (self.dma_rx_maxcnt & 0xFFFF) as usize;
        let mut n = 0usize;
        if let Ok(mut q) = self.rx_source.lock() {
            while n < max {
                let Some(b) = q.pop_front() else { break };
                let addr = self.dma_rx_ptr as u64 + n as u64;
                if crate::with_device_write_bypass(bus, |bus| bus.write_u8(addr, b)).is_ok() {
                    n += 1;
                }
            }
        }
        self.dma_rx_amount = n as u32;

        if n > 0 {
            self.set_event(OFF_EVENTS_RXDRDY);
        }
        self.set_event(OFF_EVENTS_DMA_RX_END);
    }

    /// Pin this UARTE onto the legacy per-cycle walk. See
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

    /// True when an EasyDMA transfer could move bytes RIGHT NOW: a pending TX,
    /// or a pending RX with bytes already queued. This is the historical
    /// `needs_bus_tick` predicate, unchanged.
    fn has_transfer_work(&self) -> bool {
        self.tx_pending || (self.rx_pending && self.rx_queued() > 0)
    }

    /// One tick-equivalent of EasyDMA: the entire body the legacy
    /// `tick_with_bus` used to hold, now shared by the walk, the bare-CPU
    /// oracle and `on_event` so all three move the same bytes in the same
    /// order.
    fn service(&mut self, bus: &mut dyn Bus) {
        if self.rx_pending && self.rx_queued() > 0 {
            self.do_dma_rx(bus);
        }
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
        crate::with_device_write_bypass(bus, |bus| {
            for i in 0..len {
                let addr = self.dma_tx_ptr as u64 + i as u64;
                if let Ok(b) = bus.read_u8(addr) {
                    self.emit_byte(b);
                }
            }
        });
        self.dma_tx_amount = len as u32;

        // Events still fire together. Per-character TXDRDY during the buffer
        // is not modelled. A baud-timed wait lives on the nRF52 UARTE; this
        // wake cannot take one without moving the RX poll and the IRQ edge
        // with it. FIDELITY: modeled, NOT HW-validated (2026-09-30).
        self.set_event(OFF_EVENTS_TXDRDY);
        self.set_event(OFF_EVENTS_DMA_TX_END);
        self.set_event(OFF_EVENTS_DMA_TX_READY);
        self.set_event(OFF_EVENTS_TXSTOPPED);
    }

    /// The level→edge conversion the legacy `tick()` performed, extracted so
    /// the scheduler path produces a bit-identical IRQ stream.
    fn irq_edge(&mut self) -> bool {
        let level = self.pending_mask() & self.inten != 0;
        let irq = level && !self.irq_level;
        self.irq_level = level;
        irq
    }

    /// Whether the scheduler must keep waking this UARTE.
    ///
    /// `rx_pending` counts even with an EMPTY queue, and that is the whole
    /// reason this model needs a self-perpetuating wake rather than events
    /// armed purely by MMIO writes: RX bytes arrive from the HOST, on another
    /// thread, through `rx_source`. Nothing on the bus fires when one lands,
    /// so an armed RX must be polled — which the per-cycle walk used to do for
    /// free. `on_event` paces that poll at the bus tick interval so the poll
    /// does not itself pin the CPU window back to one instruction.
    fn has_active_work(&self) -> bool {
        self.tx_pending
            || self.rx_pending
            || (self.pending_mask() & self.inten != 0) != self.irq_level
    }
}

impl Peripheral for Nrf54lUarte {
    fn bus_trace_handle(&self) -> Option<crate::bus::bus_trace::BusTrace> {
        Some(self.trace.clone())
    }

    /// Join the machine's one bus trace. Like its nRF52 sibling, this model
    /// previously recorded no trace at all.
    fn attach_bus_trace(&mut self, name: &str, trace: &crate::bus::bus_trace::BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.to_string();
    }

    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        // RXDRDY additionally reflects queued-but-unread injection bytes.
        if offset == OFF_EVENTS_RXDRDY {
            return Ok(self.events[event_index(offset).unwrap()] | (self.rx_queued() > 0) as u32);
        }
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
            // TASKS_DMA.RX.START arms an EasyDMA drain of the RX injection
            // queue; the RAM write happens in `tick_with_bus` (same deferred
            // pattern as TX). STOPRX/FLUSHRX accepted, no modelled effect.
            OFF_TASKS_DMA_RX_START => self.rx_pending = true,
            OFF_TASKS_DMA_RX_STOP | OFF_TASKS_FLUSHRX => {}
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
    /// only reachable with a bus handle — so the transfer is performed in a
    /// bus-aware hook rather than in `write_u32`.
    ///
    /// In scheduler mode this is dead: `on_event` calls [`Self::service`]
    /// directly and running it from the walk as well would move every byte
    /// twice. The bare-CPU oracle still reaches the transfer through
    /// `needs_bus_tick_forced` / `tick_with_bus_forced` below.
    fn needs_bus_tick(&self) -> bool {
        !self.sched_driven() && self.has_transfer_work()
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if self.sched_driven() {
            return;
        }
        self.service(bus);
    }

    /// Bare-CPU-oracle twins: re-expose the historical one-tick transfer to the
    /// forced walk, which freezes the CPU and settles peripherals through the
    /// legacy path even when the scheduler owns them.
    fn needs_bus_tick_forced(&self) -> bool {
        self.has_transfer_work()
    }

    fn tick_with_bus_forced(&mut self, bus: &mut dyn Bus) {
        self.service(bus);
    }

    /// Level-derived peripheral IRQ: asserted while any INTEN-enabled event is
    /// pending, emitted on the 0→1 edge so a pending event that the firmware
    /// has not yet cleared does not re-pend the NVIC every cycle.
    ///
    /// Inert in scheduler mode — `on_event` owns the edge there. Letting both
    /// run would not double the IRQ (the second call sees the level it just
    /// latched and reports no edge) but it WOULD race for which of the two
    /// observes the 0→1 transition, i.e. which cycle the NVIC is pended on.
    fn tick(&mut self) -> PeripheralTickResult {
        if self.sched_driven() {
            return PeripheralTickResult::default();
        }
        PeripheralTickResult {
            irq: self.irq_edge(),
            ..Default::default()
        }
    }

    /// Bare-CPU-oracle twin of [`Self::tick`]: the forced walk settles
    /// peripherals through their historical path even when the scheduler owns
    /// them, so the edge conversion must run here regardless of mode.
    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        PeripheralTickResult {
            irq: self.irq_edge(),
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: crate::cycle_clock::CycleClock) {
        self.clock = Some(clock);
    }

    fn uses_scheduler(&self) -> bool {
        self.sched_driven()
    }

    fn needs_legacy_walk(&self) -> bool {
        // Everything the walk did here — poll for an armed EasyDMA transfer and
        // convert the IRQ level to an edge — now rides the WAKE event, so the
        // walk is deletable for this model.
        //
        // `derive_walk_deletable` is an ALL over the bus: while ANY peripheral
        // answers true the whole board stays pinned to
        // `max_safe_tick_interval() == 1`, and at a one-instruction window the
        // Cortex-M hot-loop fast path (budget >= 8) never engages. On nrf54l15
        // that clamp costs ~38x. See `tick_interval_inventory` for the
        // remaining forcers.
        !self.sched_driven()
    }

    /// Hand the bus one self-perpetuating WAKE when there is work and none is
    /// already in flight. Called after every MMIO write to this peripheral —
    /// which is exactly when `tx_pending` / `rx_pending` / INTEN / an EVENTS
    /// clear can have created work — and once at scheduler bootstrap.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.has_active_work() && !self.scheduled {
            self.scheduled = true;
            // Delay 0 on purpose. This token is also the RX poll and the IRQ
            // edge, and the forced walk finishes a TX in one tick. A frame
            // wait here would desync those paths. The idle re-arm below is
            // what paces the RX poll at the bus tick interval.
            vec![(0, UARTE_WAKE_TOKEN)]
        } else {
            Vec::new()
        }
    }

    /// One tick-equivalent of walk work, then re-arm while work remains.
    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        bus: &mut dyn Bus,
    ) -> crate::sched::EventResult {
        self.service(bus);
        let irq = self.irq_edge();
        let keep_going = self.has_active_work();
        self.scheduled = keep_going;

        // Anything still outstanding here is an RX buffer armed against a host
        // that has not typed yet: `service` consumed any transfer that could
        // run, and `irq_edge` just latched the level. So the re-arm is a POLL,
        // and polling at delay 1 would hold the next-event deadline one cycle
        // ahead forever — which is the very clamp this migration exists to
        // lift. Wake at the bus tick interval instead. Byte VALUES and ORDER
        // are unchanged; only the instant an injected byte becomes visible is
        // quantised, by at most one interval — the same bound every other
        // scheduler-driven peripheral on a walk-deleted bus already carries.
        // A bus needing cycle-exact delivery reports interval 1 and gets the
        // old cadence back verbatim.
        // No `#[cfg]` pair: `Bus::peripheral_tick_interval` is available in
        // both worlds and defaults to 1, so the featureless build gets the
        // legacy one-tick cadence from the same expression.
        let delay = u64::from(bus.peripheral_tick_interval().max(1));

        crate::sched::EventResult {
            // `raise_own_irq`, not `raise_irq`: this model does not know its
            // own NVIC line — the bus maps it from `PeripheralEntry::irq`,
            // exactly as it mapped the legacy `PeripheralTickResult::irq`.
            raise_own_irq: irq,
            reschedule_delay: keep_going.then_some(delay),
            ..Default::default()
        }
    }

    /// `tick()` only tracks the level-held IRQ edge; when the level equals what
    /// was last observed it is a genuine no-op. Reporting that (instead of the
    /// always-active default) drops the UARTE out of the per-cycle walk while
    /// idle and — critically — lets idle fast-forward engage during a
    /// tickless-idle WFI window. Walk-identical: every skipped cycle is one
    /// where `tick()` would have recomputed the same level and emitted no IRQ.
    /// NOT gated on `scheduler_mode()`, deliberately.
    ///
    /// This predicate feeds TWO memberships: the production walk, and the
    /// bare-CPU oracle's forced walk (`tick_peripherals_fully_forced` builds
    /// `forced_tick_indices` from it). Gating it would make
    /// `tick_elapsed_forced` unreachable — the twin would be dead code and the
    /// oracle would silently see a peripheral that never ticks. It is also
    /// unnecessary: the production walk already skips every `uses_scheduler()`
    /// peripheral on its own, so reporting work here costs at most one virtual
    /// call on a hybrid bus and nothing at all on a walk-deleted one.
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
    fn dma_rx_writes_queued_bytes_to_ram() {
        let (mut u, mut bus, _sink) = fixture(0x2000_0010, b"");
        u.rx_buffer()
            .lock()
            .unwrap()
            .extend([0xDE, 0xAD, 0xBE, 0xEF]);

        u.write_u32(OFF_DMA_RX_PTR, 0x2000_0020).unwrap();
        u.write_u32(OFF_DMA_RX_MAXCNT, 3).unwrap(); // MAXCNT < queued: drains 3
        u.write_u32(OFF_TASKS_DMA_RX_START, 1).unwrap();
        assert!(u.needs_bus_tick(), "RX.START with queued bytes arms RX DMA");

        u.tick_with_bus(&mut bus);

        assert_eq!(bus.read_u8(0x2000_0020).unwrap(), 0xDE);
        assert_eq!(bus.read_u8(0x2000_0021).unwrap(), 0xAD);
        assert_eq!(bus.read_u8(0x2000_0022).unwrap(), 0xBE);
        assert_eq!(u.read_u32(OFF_DMA_RX_AMOUNT).unwrap(), 3);
        assert_eq!(u.read_u32(OFF_EVENTS_DMA_RX_END).unwrap(), 1);
        assert_eq!(u.rx_queued(), 1, "one byte beyond MAXCNT stays queued");
        assert!(!u.rx_pending, "drain consumes pending");
    }

    #[test]
    fn dma_rx_empty_queue_stays_armed_without_bus_work() {
        let (mut u, _bus, _sink) = fixture(0x2000_0010, b"");
        u.write_u32(OFF_TASKS_DMA_RX_START, 1).unwrap();
        // Bare-bus path: no work until bytes exist (no busy-spin).
        assert!(!u.needs_bus_tick());
        // RXDRDY reads 0 while the queue is empty, 1 once a byte lands.
        assert_eq!(u.read_u32(OFF_EVENTS_RXDRDY).unwrap(), 0);
        u.rx_buffer().lock().unwrap().push_back(0x42);
        assert!(u.needs_bus_tick());
        assert_eq!(u.read_u32(OFF_EVENTS_RXDRDY).unwrap(), 1);
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

/// The scheduler path, which the 13 tests above cannot reach.
///
/// Every one of them builds the model with `Nrf54lUarte::new()` and never
/// attaches a `CycleClock`, so `scheduler_mode()` is false throughout and they
/// all exercise the LEGACY walk — after this migration, the path that a real
/// nrf54l15 board no longer uses. They are still the right tests for what they
/// assert (register semantics, byte order, zero-length TX); they simply cannot
/// witness a regression in the code that replaced them. These can.
/// Gated on the FEATURE as well as `test`, like its `nrf54l/clock.rs` sibling.
/// `SystemBus::add_peripheral` hands every peripheral a `CycleClock`
/// unconditionally, so without the feature `scheduler_mode()` is false while
/// `clock.is_some()` is true — these tests attach a clock and assert
/// `uses_scheduler()`, which only holds in the feature-on world.
#[cfg(all(test, feature = "event-scheduler"))]
mod scheduler_mode_tests {
    use super::*;
    use crate::cycle_clock::CycleClock;
    use crate::memory::LinearMemory;
    use crate::sched::EventScheduler;

    /// Same shape as the legacy `tests::fixture`, plus the `CycleClock` the
    /// bus hands over at registration — the one thing those 13 tests never do.
    fn fixture(
        addr: u64,
        bytes: &[u8],
    ) -> (
        Nrf54lUarte,
        crate::bus::SystemBus,
        std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    ) {
        let mut bus = crate::bus::SystemBus::empty();
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

    fn armed(
        addr: u64,
        bytes: &[u8],
    ) -> (
        Nrf54lUarte,
        crate::bus::SystemBus,
        std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    ) {
        let (mut u, bus, sink) = fixture(addr, bytes);
        u.attach_cycle_clock(CycleClock::default());
        (u, bus, sink)
    }

    #[test]
    fn attaching_a_clock_leaves_the_walk() {
        let (u, _bus, _sink) = armed(0x2000_0010, b"hi");
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
        let (u, _bus, _sink) = fixture(0x2000_0010, b"hi");
        assert!(!u.uses_scheduler());
        assert!(
            u.needs_legacy_walk(),
            "a hand-built bus never wired the scheduler, so a UARTE that \
             stopped ticking would never complete a transfer at all"
        );
    }

    /// The double-transfer guard. In scheduler mode the walk must not ALSO
    /// move the bytes, or every console byte appears twice.
    #[test]
    fn the_walk_hook_is_inert_in_scheduler_mode() {
        let (mut u, mut bus, sink) = armed(0x2000_0010, b"AB");
        u.write_u32(OFF_DMA_TX_PTR, 0x2000_0010).unwrap();
        u.write_u32(OFF_DMA_TX_MAXCNT, 2).unwrap();
        u.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();

        assert!(
            !u.needs_bus_tick(),
            "the bus-tick pass must not select a scheduler-driven UARTE"
        );
        u.tick_with_bus(&mut bus);
        assert!(
            sink.lock().unwrap().is_empty(),
            "the walk moved bytes the scheduler is also going to move"
        );
        assert!(u.tx_pending, "and the transfer is still armed for on_event");
    }

    /// ...but the bare-CPU oracle, which freezes the CPU and settles
    /// peripherals through their historical path, must still see the transfer.
    /// Without the `_forced` twins it would see nothing at all.
    #[test]
    fn the_forced_walk_still_reaches_the_transfer() {
        let (mut u, mut bus, sink) = armed(0x2000_0010, b"AB");
        u.write_u32(OFF_DMA_TX_PTR, 0x2000_0010).unwrap();
        u.write_u32(OFF_DMA_TX_MAXCNT, 2).unwrap();
        u.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();

        assert!(u.needs_bus_tick_forced());
        u.tick_with_bus_forced(&mut bus);
        assert_eq!(&*sink.lock().unwrap(), b"AB");
    }

    #[test]
    fn a_tx_start_arms_exactly_one_wake() {
        let (mut u, _bus, _sink) = armed(0x2000_0010, b"AB");
        u.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();

        assert_eq!(u.take_scheduled_events(), vec![(0, UARTE_WAKE_TOKEN)]);
        assert!(
            u.take_scheduled_events().is_empty(),
            "a second MMIO write before the wake fires must not stack a \
             duplicate transfer"
        );
    }

    /// The end-to-end scheduler transfer: same bytes, same order, same
    /// completion events, plus the own-IRQ the legacy `tick()` used to raise.
    #[test]
    fn on_event_transfers_and_raises_the_own_irq() {
        let (mut u, mut bus, sink) = armed(0x2000_0010, b"Hi!");
        u.write_u32(OFF_INTEN, 1 << event_index(OFF_EVENTS_DMA_TX_END).unwrap())
            .unwrap();
        u.write_u32(OFF_DMA_TX_PTR, 0x2000_0010).unwrap();
        u.write_u32(OFF_DMA_TX_MAXCNT, 3).unwrap();
        u.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();
        let token = u.take_scheduled_events()[0].1;

        let mut sched = EventScheduler::new();
        let r = u.on_event(token, &mut sched, &mut bus);

        assert_eq!(&*sink.lock().unwrap(), b"Hi!");
        assert_eq!(u.read_u32(OFF_EVENTS_DMA_TX_END).unwrap(), 1);
        assert_eq!(u.read_u32(OFF_DMA_TX_AMOUNT).unwrap(), 3);
        assert!(
            r.raise_own_irq,
            "INTEN-enabled event → 0→1 edge → NVIC pend"
        );
        assert_eq!(
            r.reschedule_delay, None,
            "nothing left to do: the transfer is done and the level is latched"
        );
        assert!(!u.scheduled);
    }

    /// The falling edge. Firmware clearing EVENTS_* drops the level, and the
    /// model must observe that — otherwise `irq_level` stays true and the NEXT
    /// transfer's 0→1 edge is swallowed, hanging the driver.
    #[test]
    fn clearing_the_event_rearms_the_edge_detector() {
        let (mut u, mut bus, _sink) = armed(0x2000_0010, b"A");
        u.write_u32(OFF_INTEN, 1 << event_index(OFF_EVENTS_DMA_TX_END).unwrap())
            .unwrap();
        u.write_u32(OFF_DMA_TX_PTR, 0x2000_0010).unwrap();
        u.write_u32(OFF_DMA_TX_MAXCNT, 1).unwrap();
        u.write_u32(OFF_TASKS_DMA_TX_START, 1).unwrap();
        let token = u.take_scheduled_events()[0].1;
        let mut sched = EventScheduler::new();
        assert!(u.on_event(token, &mut sched, &mut bus).raise_own_irq);
        assert!(u.irq_level);

        // ISR clears the completion events (write-0 clears on this family).
        for off in [
            OFF_EVENTS_TXDRDY,
            OFF_EVENTS_DMA_TX_END,
            OFF_EVENTS_DMA_TX_READY,
            OFF_EVENTS_TXSTOPPED,
        ] {
            u.write_u32(off, 0).unwrap();
        }
        assert_eq!(
            u.take_scheduled_events(),
            vec![(0, UARTE_WAKE_TOKEN)],
            "the level fell, so the walk's job (observe it) needs a wake"
        );
        let r = u.on_event(token, &mut sched, &mut bus);
        assert!(!r.raise_own_irq);
        assert!(!u.irq_level, "edge detector re-armed for the next transfer");
        assert_eq!(r.reschedule_delay, None);
    }

    /// An armed RX with an empty queue is the one case that MUST keep waking:
    /// bytes arrive from the host on another thread, and nothing on the bus
    /// fires when one lands. The per-cycle walk polled for free; the scheduler
    /// has to ask.
    #[test]
    fn an_armed_rx_keeps_polling_until_a_byte_lands() {
        let (mut u, mut bus, _sink) = armed(0x2000_0010, b"");
        u.write_u32(OFF_DMA_RX_PTR, 0x2000_0020).unwrap();
        u.write_u32(OFF_DMA_RX_MAXCNT, 4).unwrap();
        u.write_u32(OFF_TASKS_DMA_RX_START, 1).unwrap();
        assert_eq!(u.take_scheduled_events(), vec![(0, UARTE_WAKE_TOKEN)]);

        let mut sched = EventScheduler::new();
        let r = u.on_event(UARTE_WAKE_TOKEN, &mut sched, &mut bus);
        assert!(
            r.reschedule_delay.is_some(),
            "an empty queue must re-arm the poll, not give up — giving up \
             strands every RX-driven driver on this family"
        );

        // Host types.
        u.rx_buffer().lock().unwrap().extend(*b"hey");
        u.on_event(UARTE_WAKE_TOKEN, &mut sched, &mut bus);
        assert_eq!(bus.read_u8(0x2000_0020).unwrap(), b'h');
        assert_eq!(bus.read_u8(0x2000_0022).unwrap(), b'y');
        assert_eq!(u.read_u32(OFF_DMA_RX_AMOUNT).unwrap(), 3);
        assert!(!u.rx_pending, "the drain consumed the arm");
    }

    /// The poll must not be the thing that re-imposes the clamp it lifted.
    /// A delay of 1 would hold the next-event deadline one cycle ahead of the
    /// CPU forever, pinning `plan_cpu_window` back to a one-instruction
    /// quantum — a walk deletion that bought nothing.
    #[test]
    fn the_idle_rx_poll_is_paced_at_the_tick_interval() {
        let (mut u, mut bus, _sink) = armed(0x2000_0010, b"");
        bus.config.peripheral_tick_interval = 64;
        u.write_u32(OFF_TASKS_DMA_RX_START, 1).unwrap();
        let mut sched = EventScheduler::new();

        let r = u.on_event(UARTE_WAKE_TOKEN, &mut sched, &mut bus);
        assert_eq!(r.reschedule_delay, Some(64));
    }
}
