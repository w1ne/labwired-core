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
//! single-byte TXD path used by the Adafruit/Arduino nRF52 core.
//!
//! RX: host-injected serial input arrives through the shared `rx_source` queue
//! (see `Bus::attach_uart_rx_source_named`, which downcasts to this model).
//! UARTE personality: TASKS_STARTRX arms an EasyDMA drain — up to RXD.MAXCNT
//! queued bytes are written to RAM at RXD.PTR, RXD.AMOUNT is set and ENDRX
//! raised. Bytes that arrive AFTER STARTRX are picked up by a periodic re-arm
//! (scheduler path, ~1024-cycle poll); the bare-bus `tick_with_bus` path only
//! drains when the queue is non-empty. Legacy personality: RXD (0x518) pops one
//! queued byte per read and RXDRDY reflects queue-non-empty. Baud-rate timing
//! is not modelled; transfers complete at the next scheduler event.
//!
//! EVENTS: hardware-generated. SW write-1 is ignored; write-0 clears.

use crate::peripherals::nrf52::pin_select::{NrfPinClaim, NrfPinClaims};
use crate::peripherals::pad_lines::PadLines;
use crate::peripherals::uart_waveform::{Parity, UartFraming, UartNarrator};
use crate::{Bus, Peripheral, SimResult};
use std::collections::{BTreeMap, VecDeque};
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

// BAUDRATE — silicon reset is Baud250000 = 0x0400_0000 (nRF52840 PS v1.11
// §6.34.9.27 p847 for UARTE; same value on the legacy UART at p830).
const OFF_BAUDRATE: u64 = 0x524;
/// Silicon reset / documented `Baud250000` encoding.
const BAUDRATE_RESET: u32 = 0x0400_0000;

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

// ── CONFIG fields (nRF52840 PS v1.11 §6.34.9.30, p849) ───────────────────────
// bit 0     HWFC   0 = Disabled, 1 = Enabled
// bits[3:1] PARITY 0x0 = Excluded, 0x7 = Include EVEN parity bit
// bit 4     STOP   0 = One stop bit, 1 = Two stop bits
const CONFIG_PARITY_SHIFT: u32 = 1;
const CONFIG_PARITY_MASK: u32 = 0x7;
const CONFIG_PARITY_INCLUDED: u32 = 0x7;
const CONFIG_STOP_TWO: u32 = 1 << 4;

// ── ENABLE personalities (PS v1.11 §6.34.9.24 / §6.33.10.21) ─────────────────
// 4 selects the legacy UART, 8 selects UARTE. Both drive TXD through PSEL.TXD,
// so both make this instance's pin claim live.
const ENABLE_MASK: u32 = 0xF;

/// Line order for this UARTE's [`PadLines`]. TXD ONLY — see
/// [`Nrf52Uarte::pad_lines_arc`].
pub(crate) const UARTE_LINES: &[&str] = &["TXD"];
pub(crate) const LINE_TXD: usize = 0;

/// nRF52 core / HFCLK frequency. `BAUDRATE` is defined as
/// `round(baud · 2^32 / 16 MHz)`, so one bit period in core cycles is
/// `64 MHz · 2^32 / (BAUDRATE · 16 MHz)` = `2^34 / BAUDRATE`.
const BIT_TIME_NUMERATOR: u64 = 1u64 << 34;

/// Buffered TX bytes past which a transfer's narration is dropped rather than
/// truncated. `TXD.MAXCNT` is 16 bits, so one EasyDMA transfer can be 64 KiB.
const WIRE_BYTE_CAP: usize = 2_048;

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
    /// Set by a STARTTX task write; consumed by the EasyDMA engine
    /// (`do_easydma_tx`) via either `tick_with_bus` (bare-bus unit tests /
    /// bus_tick_indices) or `on_event` (Machine + event-scheduler, delay-0).
    /// Deferred because `write_u32` has no bus handle for the RAM read.
    tx_pending: bool,
    /// Set by a STARTRX task write (UARTE personality); consumed by
    /// `do_easydma_rx` once the RX queue has bytes to drain.
    rx_pending: bool,
    /// Scheduler-side singleton: a TX wake is already queued (or in
    /// `reschedule_delay` flight). `collect_scheduled_events` runs after
    /// every MMIO write; without this guard each STARTTX-pending poll arms
    /// a new absolute deadline and trips
    /// [`MAX_LIVE_EVENTS_PER_PERIPHERAL`](crate::sched::MAX_LIVE_EVENTS_PER_PERIPHERAL)
    /// on Zephyr hello_world (nRF5340 UARTE console).
    tx_chain_live: bool,
    /// Scheduler-side singleton for the RX EasyDMA / empty-queue poll chain.
    /// Same contract as `tx_chain_live`. An empty-queue poll may upgrade to
    /// delay-0 once when bytes arrive (`rx_poll_upgrade_live`).
    rx_chain_live: bool,
    /// True after an empty-queue RX poll has been upgraded to an immediate
    /// drain while the original poll event is still resident. Caps the
    /// upgrade pile-up at one extra live event.
    rx_poll_upgrade_live: bool,
    /// Host-injected serial input. Shared with the runner via
    /// `Bus::attach_uart_rx_source_named`; bytes pushed there sit in the
    /// queue until firmware reads them (RXD pop or EasyDMA drain).
    rx_source: Arc<Mutex<VecDeque<u8>>>,
    /// Captured TX bytes for `test`-mode assertions (`uart_contains`).
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    /// Echo transmitted bytes to the process stdout (console behaviour).
    echo_stdout: bool,
    /// The machine's ONE bus trace and this instance's name in it; see
    /// [`crate::bus::bus_trace`]. Private until `attach_bus_trace` hands over
    /// the shared handle at registration.
    trace: crate::bus::bus_trace::BusTrace,
    trace_name: String,
    /// Live TXD level published to whichever pad `PSEL.TXD` selects, so a probe
    /// clipped there measures the serial waveform instead of the GPIO output
    /// latch. Created lazily by [`Self::pad_lines_arc`] at bus wiring time;
    /// `None` when no GPIO port routes this UARTE's pad, and then nothing below
    /// is buffered or narrated at all.
    lines: Option<Arc<PadLines>>,
    /// This instance's standing claim on the pad `PSEL.TXD` names. See
    /// [`crate::peripherals::nrf52::pin_select`].
    claim_txd: NrfPinClaim,
    /// Bytes of the EasyDMA transfer in flight, buffered so the whole burst is
    /// narrated as one contiguous waveform. See [`Self::wire_flush`].
    wire_bytes: Vec<u8>,
    /// Set when a transfer blew past [`WIRE_BYTE_CAP`], so its narration is
    /// dropped whole rather than published truncated.
    wire_overflow: bool,
    /// The cycle the previous narration ran to, so the next one cannot reach
    /// back over cycles it already painted.
    ///
    /// A UART is the one bus that really does flush burst after burst — a
    /// `printk` loop is exactly that — and [`UartNarrator::emit_ending_at`]
    /// would reach as far back as it liked, re-driving levels the capture layer
    /// has already recorded and inventing transitions that never happened. The
    /// I²C and SPI narrators here use the unbounded form because their
    /// transactions are separated by idle bus; this one is not.
    wire_cursor: u64,
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
            // BAUDRATE silicon reset: Baud250000 (PS v1.11 p847 / p830).
            baudrate: BAUDRATE_RESET,
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

    /// Shared handle to the RX injection queue, mirroring `Uart::rx_buffer`
    /// so `Bus::attach_uart_rx_source_named` can drive nRF52 serial input.
    pub fn rx_buffer(&self) -> Arc<Mutex<VecDeque<u8>>> {
        self.rx_source.clone()
    }

    fn rx_queued(&self) -> usize {
        self.rx_source.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// The shared pad-line cell for this UARTE's TXD, created on first use.
    /// Called at bus wiring time by `wire_nrf52_pads`; an idle serial line
    /// rests HIGH (mark).
    ///
    /// TXD ONLY. `PSEL.RXD`, `PSEL.CTS` and `PSEL.RTS` are tracked registers
    /// but nothing in this engine DRIVES those wires — RX arrives as queued
    /// bytes with no timing, and flow control is not modelled at all. A pad
    /// routed to one of them would report a confident constant idle level right
    /// through incoming traffic, which is worse than the GPIO-latch fallback it
    /// replaced because it looks authoritative. Same call as
    /// `wire_rp2040_uart_pads`. They join the table when something drives them.
    pub(crate) fn pad_lines_arc(&mut self) -> Arc<PadLines> {
        self.lines
            .get_or_insert_with(|| Arc::new(PadLines::new(UARTE_LINES, &[true])))
            .clone()
    }

    /// Join the chip's pin-claim table so `PSEL.TXD` decides which pad reads
    /// this UARTE's wire. Config-build time only.
    pub(crate) fn install_pin_claims(&mut self, claims: &Arc<NrfPinClaims>, txd_token: u32) {
        self.claim_txd.install(claims.clone(), txd_token);
        self.sync_pin_claims();
    }

    /// Republish the TXD claim from the live registers, after any write that
    /// can move the pad (`PSEL.TXD`, `ENABLE`).
    ///
    /// Live under BOTH personalities. One MMIO window hosts UART0 and UARTE0,
    /// selected by `ENABLE` (4 = legacy UART, 8 = UARTE), and both of them
    /// drive the pin `PSEL.TXD` names — so gating on the UARTE value alone
    /// would leave the Adafruit/Arduino nRF52 core, which uses the legacy path,
    /// invisible.
    fn sync_pin_claims(&mut self) {
        let live = matches!(self.enable & ENABLE_MASK, ENABLE_UART_LEGACY | 8);
        self.claim_txd.update(self.psel_txd, live);
    }

    /// One bit period in core cycles, from `BAUDRATE`.
    ///
    /// `BAUDRATE` is `round(baud · 2^32 / 16 MHz)` (nRF52840 PS v1.11
    /// §6.34.9.27, p847 — `Baud115200 = 0x01D60000`, `Baud250000 =
    /// 0x04000000`, `Baud1M = 0x10000000`), and the core runs at 64 MHz, so one
    /// bit is `2^34 / BAUDRATE` cycles. At 115200 baud that is 555 cycles,
    /// which is 64 MHz / 115 200 to within a cycle.
    fn bit_time_cycles(&self) -> u64 {
        if self.baudrate == 0 {
            // A zero BAUDRATE transmits nothing on silicon. Nothing sane can be
            // narrated from it; fall back to the silicon reset rate rather than
            // divide by zero or draw a waveform of zero-length bits.
            return BIT_TIME_NUMERATOR / u64::from(BAUDRATE_RESET);
        }
        (BIT_TIME_NUMERATOR / u64::from(self.baudrate)).max(2)
    }

    /// Character framing as `CONFIG` programs it (PS v1.11 §6.34.9.30, p849).
    /// UARTE is 8 data bits, LSB first, always — the part has no word-length
    /// field.
    fn framing(&self) -> UartFraming {
        let parity = if (self.config >> CONFIG_PARITY_SHIFT) & CONFIG_PARITY_MASK
            == CONFIG_PARITY_INCLUDED
        {
            // "Include EVEN parity bit" — the only parity this part generates.
            Parity::Even
        } else {
            Parity::None
        };
        UartFraming {
            data_bits: 8,
            parity,
            stop_bits: if self.config & CONFIG_STOP_TWO != 0 {
                2
            } else {
                1
            },
        }
    }

    /// Publish the buffered transfer's waveform onto the claimed TXD pad.
    ///
    /// The EasyDMA model completes a whole buffer in one shot, so the burst is
    /// narrated as one contiguous run ending at the present cycle — the same
    /// arrangement, and for the same reason, as every other transaction-level
    /// narrator in the engine: the capture layer accepts stamps in the PAST
    /// only, so a character that has not yet had time to cross has nowhere to
    /// go.
    fn wire_flush(&mut self) {
        let overflowed = std::mem::take(&mut self.wire_overflow);
        let Some(lines) = self.lines.clone() else {
            self.wire_bytes.clear();
            return;
        };
        if overflowed {
            // The bytes really went out; we simply cannot draw that many edges.
            // A truncated character list would decode to a message nobody sent.
            self.wire_bytes.clear();
            return;
        }
        if self.wire_bytes.is_empty() {
            return;
        }
        let framing = self.framing();
        let mut wave =
            UartNarrator::with_lines(LINE_TXD, &[lines.level(LINE_TXD)], self.bit_time_cycles());
        for byte in std::mem::take(&mut self.wire_bytes) {
            wave.frame(byte, framing);
        }
        let now = lines.tap_clock().unwrap_or(0);
        // A transfer early in a run has less history behind it than the
        // waveform needs; the narrator compresses into what IS available rather
        // than emitting a spike. The characters still decode; only the timebase
        // gives, so the verdict is deliberately dropped.
        let _fit = wave.emit_between(&lines, self.wire_cursor, now);
        self.wire_cursor = now;
    }

    /// Buffer one transmitted byte for narration. Costs one branch when no pad
    /// routes to this UARTE.
    fn wire_push(&mut self, byte: u8) {
        if self.lines.is_none() || self.wire_overflow {
            return;
        }
        if self.wire_bytes.len() >= WIRE_BYTE_CAP {
            self.wire_overflow = true;
            self.wire_bytes.clear();
            return;
        }
        self.wire_bytes.push(byte);
    }

    fn emit_byte(&mut self, byte: u8) {
        self.wire_push(byte);
        // The one place a TX byte leaves this UARTE, so the one place it is
        // traced. See `attach_bus_trace` for what this model recorded before:
        // nothing, anywhere.
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
}

impl Peripheral for Nrf52Uarte {
    fn line_names(&self) -> &'static [&'static str] {
        UARTE_LINES
    }

    fn wire_lines(&self) -> Option<&PadLines> {
        self.lines.as_deref()
    }

    fn bus_trace_handle(&self) -> Option<crate::bus::bus_trace::BusTrace> {
        Some(self.trace.clone())
    }

    /// Join the machine's one bus trace. This model previously recorded no
    /// trace at all — the browser located UARTs by `downcast_ref::<Uart>()`,
    /// which a UARTE is not, so every nRF52 lab's UART analyzer was silently
    /// empty.
    fn attach_bus_trace(&mut self, name: &str, trace: &crate::bus::bus_trace::BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.to_string();
    }

    /// Dual-path EasyDMA: scheduler delay-0 (`on_event`) under Machine +
    /// walk-free + batched `peripheral_tick_interval`, and `tick_with_bus`
    /// (`bus_tick_indices`) for bare-bus unit tests / feature-off. No
    /// time-driven `tick()` / `tick_elapsed()`, so the legacy walk is not
    /// required. Under `rec_tick=512` the scheduler path completes STARTTX on
    /// the next cycle (not at the 512-cycle bus-tick quantum).
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    /// Not in the per-cycle walk: no time-driven `tick()` / `tick_elapsed()`.
    /// EasyDMA completion rides the dual-path scheduler + bus_tick engines.
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
            // RXDRDY additionally reflects queued-but-unread injection bytes,
            // so a legacy driver polling RXDRDY-then-RXD sees data-ready.
            OFF_EVENTS_RXDRDY => self.events_rxdrdy | (self.rx_queued() > 0) as u32,
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
            // Legacy UART data: TXD is write-only (reads 0). RXD pops one
            // byte from the injection queue per read (0 when empty), matching
            // a polling legacy driver that waits on RXDRDY first.
            OFF_TXD_LEGACY => 0,
            OFF_RXD_LEGACY => self
                .rx_source
                .lock()
                .ok()
                .and_then(|mut q| q.pop_front())
                .unwrap_or(0) as u32,
            _ => self.extra.get(&offset).copied().unwrap_or(0),
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // STARTTX arms an EasyDMA TX; the actual buffer read + emit happens
            // in `do_easydma_tx` via tick_with_bus / on_event (write_u32 has no
            // bus handle). A 1-byte poll_out and a multi-byte buffered write
            // both land here. Only the UARTE personality uses EasyDMA — in
            // legacy UART mode STARTTX just enables the transmitter and bytes
            // flow through the TXD register.
            OFF_TASKS_STARTTX if self.enable != ENABLE_UART_LEGACY => self.tx_pending = true,
            OFF_TASKS_STARTTX => {}
            // STOPTX completes immediately in this model: raise TXSTOPPED so a
            // driver waiting on it (nrfx is_tx_ready) makes progress.
            OFF_TASKS_STOPTX => self.events_txstopped = 1,
            // STARTRX (UARTE personality) arms an EasyDMA drain of the RX
            // injection queue; the RAM write happens in `do_easydma_rx`.
            OFF_TASKS_STARTRX if self.enable != ENABLE_UART_LEGACY => self.rx_pending = true,
            // Legacy-personality STARTRX and the stop/flush tasks: accepted,
            // no modelled effect (legacy RX is byte-pull driven by RXD reads).
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
            // Enable. Moves which pad this instance drives: while disabled the
            // TXD pin "behaves as a regular GPIO" (PS v1.11 §6.34.8, p836).
            OFF_ENABLE => {
                self.enable = value & 0xF;
                self.sync_pin_claims();
            }
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
                // The legacy path has no burst: one byte per register write, so
                // it is narrated immediately rather than accumulated.
                self.wire_flush();
                self.events_txdrdy = 1;
            }
            // RXD is a read-only receive register; writes are ignored.
            OFF_RXD_LEGACY => {}
            // PSEL
            OFF_PSEL_RTS => self.psel_rts = value,
            OFF_PSEL_TXD => {
                self.psel_txd = value;
                self.sync_pin_claims();
            }
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

    /// Dual path: bus_tick for bare-bus tests; on_event for scheduler.
    /// The bare-bus path only fires when there is drainable work: a pending
    /// TX, or a pending RX with bytes already queued (no wake-on-inject here —
    /// the scheduler path's periodic re-arm covers late-arriving bytes).
    fn needs_bus_tick(&self) -> bool {
        self.tx_pending || (self.rx_pending && self.rx_queued() > 0)
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if self.tx_pending {
            self.do_easydma_tx(bus);
        }
        if self.rx_pending && self.rx_queued() > 0 {
            self.do_easydma_rx(bus);
        }
    }

    fn uses_scheduler(&self) -> bool {
        true
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        let mut events = Vec::new();
        // Layer-2 singleton (see `event_scheduler` cancellation contract):
        // arm at most one TX / one RX wake. Re-arming at a new absolute
        // deadline on every MMIO collect is what tripped the live-event
        // ceiling on nRF5340 Zephyr hello_world (peripheral idx = UARTE0).
        if self.tx_pending && !self.tx_chain_live {
            self.tx_chain_live = true;
            events.push((0, 1)); // STARTTX EasyDMA drain (delay-0 → next cycle)
        }
        if self.rx_pending {
            let delay = if self.rx_queued() > 0 { 0 } else { 1023 };
            if !self.rx_chain_live {
                self.rx_chain_live = true;
                self.rx_poll_upgrade_live = false;
                events.push((delay, 2));
            } else if delay == 0 && !self.rx_poll_upgrade_live {
                // Bytes arrived while an empty-queue poll is still resident:
                // schedule one immediate drain. The stale poll still fires
                // later and is a no-op once `rx_pending` is cleared.
                self.rx_poll_upgrade_live = true;
                events.push((0, 2));
            }
        }
        events
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token == 1 {
            // Drain decrements live; clear our singleton so a later STARTTX
            // can arm again. Bare-bus `tick_with_bus` also clears this.
            self.tx_chain_live = false;
            if self.tx_pending {
                self.do_easydma_tx(bus);
            }
        }
        if event_token == 2 {
            if self.rx_pending {
                if self.rx_queued() > 0 {
                    self.do_easydma_rx(bus);
                    self.rx_chain_live = false;
                    self.rx_poll_upgrade_live = false;
                } else {
                    // Nothing to receive yet: stay armed via reschedule so
                    // `take_scheduled_events` does not pile a second poll.
                    // Keep `rx_chain_live` set — the reschedule path arms the
                    // next poll under the same token after live is briefly
                    // decremented by drain.
                    self.rx_poll_upgrade_live = false;
                    return crate::sched::EventResult {
                        reschedule_delay: Some(1023),
                        ..Default::default()
                    };
                }
            } else {
                // Stale poll after an upgrade already drained RX.
                self.rx_chain_live = false;
                self.rx_poll_upgrade_live = false;
            }
        }
        crate::sched::EventResult::default()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl Nrf52Uarte {
    /// EasyDMA TX engine shared by `tick_with_bus` and `on_event` so the two
    /// paths cannot drift. Instantaneous whole-buffer completion (modelled).
    fn do_easydma_tx(&mut self, bus: &mut dyn Bus) {
        if !self.tx_pending {
            return;
        }
        self.tx_pending = false;
        self.tx_chain_live = false;

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
        // The whole buffer has crossed as far as this model is concerned, so
        // this is where the burst becomes narratable.
        self.wire_flush();

        // Raise the TX-path events a polling driver waits on. The transfer is
        // modelled as instantaneous (whole buffer in one shot), so all of the
        // begin→drain→stop events fire together: TXSTARTED, then TXDRDY/ENDTX,
        // then TXSTOPPED. nrfx's poll_out enables the ENDTX_STOPTX short and
        // waits on TXSTOPPED, so that one must be set or it spins forever.
        self.events_txstarted = 1;
        self.events_txdrdy = 1;
        self.events_endtx = 1;
        self.events_txstopped = 1;
    }

    /// EasyDMA RX engine shared by `tick_with_bus` and `on_event` so the two
    /// paths cannot drift. Drains up to RXD.MAXCNT bytes from the injection
    /// queue into RAM at RXD.PTR in one shot (instantaneous, like TX), sets
    /// RXD.AMOUNT and raises RXSTARTED/RXDRDY/ENDRX. Callers must guarantee
    /// the queue is non-empty; `rx_pending` is consumed here.
    fn do_easydma_rx(&mut self, bus: &mut dyn Bus) {
        if !self.rx_pending {
            return;
        }
        self.rx_pending = false;
        self.rx_chain_live = false;
        self.rx_poll_upgrade_live = false;

        let max = (self.rxd_maxcnt & 0xFFFF) as usize;
        let mut n = 0usize;
        if let Ok(mut q) = self.rx_source.lock() {
            while n < max {
                let Some(b) = q.pop_front() else { break };
                if bus.write_u8(self.rxd_ptr as u64 + n as u64, b).is_ok() {
                    n += 1;
                }
            }
        }
        self.rxd_amount = n as u32;

        self.events_rxstarted = 1;
        if n > 0 {
            self.events_rxdrdy = 1;
        }
        self.events_endrx = 1;
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
    fn starttx_schedules_delay0_event() {
        let mut u = Nrf52Uarte::new();
        u.write_u32(OFF_ENABLE, ENABLE_UARTE).unwrap();
        assert!(u.uses_scheduler());
        assert!(u.take_scheduled_events().is_empty());
        u.write_u32(OFF_TASKS_STARTTX, 1).unwrap();
        assert_eq!(u.take_scheduled_events(), vec![(0, 1)]);
    }

    #[test]
    fn on_event_completes_easydma_tx() {
        use crate::bus::SystemBus;
        use crate::memory::LinearMemory;
        use crate::sched::EventScheduler;
        use crate::Bus;

        let mut bus = SystemBus::empty();
        bus.ram = LinearMemory::new(256, 0x2000_0000);
        bus.write_u8(0x2000_0010, b'O').unwrap();
        bus.write_u8(0x2000_0011, b'K').unwrap();

        let mut u = Nrf52Uarte::new();
        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        u.set_sink(Some(sink.clone()), false);

        u.write_u32(OFF_ENABLE, ENABLE_UARTE).unwrap();
        u.write_u32(OFF_TXD_PTR, 0x2000_0010).unwrap();
        u.write_u32(OFF_TXD_MAXCNT, 2).unwrap();
        u.write_u32(OFF_TASKS_STARTTX, 1).unwrap();

        let mut sched = EventScheduler::new();
        let res = u.on_event(1, &mut sched, &mut bus);
        let _ = res;

        assert_eq!(&*sink.lock().unwrap(), b"OK");
        assert_eq!(u.read_u32(OFF_EVENTS_ENDTX).unwrap(), 1);
        assert_eq!(u.read_u32(OFF_EVENTS_TXSTOPPED).unwrap(), 1);
        assert!(!u.tx_pending, "on_event consumes pending");
        assert!(u.take_scheduled_events().is_empty());
    }

    #[test]
    fn easydma_rx_writes_queued_bytes_to_ram() {
        use crate::bus::SystemBus;
        use crate::memory::LinearMemory;
        use crate::Bus;

        let mut bus = SystemBus::empty();
        bus.ram = LinearMemory::new(256, 0x2000_0000);

        let mut u = Nrf52Uarte::new();
        u.rx_buffer()
            .lock()
            .unwrap()
            .extend([0xDE, 0xAD, 0xBE, 0xEF]);

        u.write_u32(OFF_ENABLE, ENABLE_UARTE).unwrap();
        u.write_u32(OFF_RXD_PTR, 0x2000_0020).unwrap();
        u.write_u32(OFF_RXD_MAXCNT, 3).unwrap(); // MAXCNT < queued: drains 3
        u.write_u32(OFF_TASKS_STARTRX, 1).unwrap();
        assert!(u.needs_bus_tick(), "STARTRX with queued bytes arms RX DMA");

        u.tick_with_bus(&mut bus);

        assert_eq!(bus.read_u8(0x2000_0020).unwrap(), 0xDE);
        assert_eq!(bus.read_u8(0x2000_0021).unwrap(), 0xAD);
        assert_eq!(bus.read_u8(0x2000_0022).unwrap(), 0xBE);
        assert_eq!(u.read_u32(OFF_RXD_AMOUNT).unwrap(), 3);
        assert_eq!(u.read_u32(OFF_EVENTS_ENDRX).unwrap(), 1);
        assert_eq!(u.read_u32(OFF_EVENTS_RXSTARTED).unwrap(), 1);
        assert_eq!(u.rx_queued(), 1, "one byte beyond MAXCNT stays queued");
        assert!(!u.rx_pending, "drain consumes pending");
    }

    #[test]
    fn easydma_rx_empty_queue_stays_armed() {
        let mut u = Nrf52Uarte::new();
        u.write_u32(OFF_ENABLE, ENABLE_UARTE).unwrap();
        u.write_u32(OFF_TASKS_STARTRX, 1).unwrap();
        // Bare-bus path: no work until bytes exist (no busy-spin).
        assert!(!u.needs_bus_tick());
        // Scheduler path: one empty-queue poll, not delay-0.
        assert_eq!(u.take_scheduled_events(), vec![(1023, 2)]);
        // Singleton: further collects while still empty do not pile wakes.
        assert!(u.take_scheduled_events().is_empty());
        // Bytes arrive → one upgrade to immediate drain (still ≤2 live).
        u.rx_buffer().lock().unwrap().push_back(0x42);
        assert_eq!(u.take_scheduled_events(), vec![(0, 2)]);
        assert!(
            u.take_scheduled_events().is_empty(),
            "upgrade is one-shot; further collects must not pile"
        );
    }

    #[test]
    fn starttx_does_not_pile_identical_scheduler_wakes() {
        let mut u = Nrf52Uarte::new();
        u.write_u32(OFF_ENABLE, ENABLE_UARTE).unwrap();
        u.write_u32(OFF_TASKS_STARTTX, 1).unwrap();
        assert_eq!(u.take_scheduled_events(), vec![(0, 1)]);
        // Simulate many MMIO-side collects while STARTTX is still pending.
        for _ in 0..16 {
            assert!(
                u.take_scheduled_events().is_empty(),
                "TX chain is a singleton under the event-scheduler contract"
            );
        }
    }

    #[test]
    fn legacy_rxd_pops_injection_queue() {
        let mut u = Nrf52Uarte::new();
        u.rx_buffer().lock().unwrap().extend(*b"OK");
        u.write_u32(OFF_ENABLE, ENABLE_UART_LEGACY).unwrap();
        assert_eq!(u.read_u32(OFF_EVENTS_RXDRDY).unwrap(), 1);
        assert_eq!(u.read_u32(OFF_RXD_LEGACY).unwrap(), b'O' as u32);
        assert_eq!(u.read_u32(OFF_RXD_LEGACY).unwrap(), b'K' as u32);
        assert_eq!(u.read_u32(OFF_EVENTS_RXDRDY).unwrap(), 0);
        assert_eq!(u.read_u32(OFF_RXD_LEGACY).unwrap(), 0);
    }

    #[test]
    fn legacy_startrx_does_not_arm_easydma() {
        let mut u = Nrf52Uarte::new();
        u.write_u32(OFF_ENABLE, ENABLE_UART_LEGACY).unwrap();
        u.write_u32(OFF_TASKS_STARTRX, 1).unwrap();
        assert!(!u.rx_pending);
    }

    #[test]
    fn psel_defaults_to_disconnected() {
        let u = Nrf52Uarte::new();
        assert_eq!(u.read_u32(OFF_PSEL_TXD).unwrap(), 0xFFFF_FFFF);
        assert_eq!(u.read_u32(OFF_PSEL_RXD).unwrap(), 0xFFFF_FFFF);
    }
    #[test]
    fn baudrate_reset_is_baud250000() {
        // nRF52840 PS v1.11 §6.34.9.27 p847 (UARTE) / p830 (UART): BAUDRATE
        // resets to 0x0400_0000 (Baud250000), not 115200.
        let u = Nrf52Uarte::new();
        assert_eq!(u.read_u32(OFF_BAUDRATE).unwrap(), BAUDRATE_RESET);
        assert_eq!(u.read_u32(OFF_BAUDRATE).unwrap(), 0x0400_0000);
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
