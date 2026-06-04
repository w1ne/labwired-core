// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::any::Any;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

/// Phase 2B.3b (issue #192): the UART uses a single self-perpetuating event
/// token — it has only one kind of wakeup ("do one tick of work"), so the
/// value is arbitrary and never disambiguated in `on_event`.
const UART_WAKE_TOKEN: u32 = 0;
const UART_TRACE_LIMIT: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UartTraceEvent {
    pub seq: u64,
    pub direction: &'static str,
    pub byte: u8,
}

/// A device that emits bytes through the UART's RX path (e.g. a GPS module).
pub trait UartStreamDevice: Send {
    /// Called periodically by the bus tick. Returns the next byte to push into UART RX,
    /// or None if no byte is pending. Implementations should respect `elapsed_us` to
    /// pace output (e.g. 9600 baud → ~1 ms/byte → emit one byte per ~1000 us tick).
    fn poll(&mut self, elapsed_us: u32) -> Option<u8>;
    /// Observe a byte transmitted by firmware on the TX path. Default: ignore.
    /// Bidirectional peers (e.g. an IO-Link master) override this to receive the
    /// device's responses, complementing `poll` which drives the RX path.
    fn on_tx_byte(&mut self, _byte: u8) {}
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UartRegisterLayout {
    #[default]
    Stm32F1,
    Stm32V2,
    Nrf52,
}

impl FromStr for UartRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32v2" | "v2" | "modern" | "stm32-modern" | "h5" | "stm32h5" => Ok(Self::Stm32V2),
            "nrf52" | "nordic" => Ok(Self::Nrf52),
            _ => Err(format!(
                "unsupported UART register layout '{}'; supported: stm32f1, stm32v2",
                value
            )),
        }
    }
}

/// The complete per-family UART register map: register offsets plus the
/// interrupt-enable bit masks. **Every** family difference lives in this one
/// descriptor — the TX sink / RX buffer / stream / scheduler engine on `Uart`
/// is architecture-independent and shared. Adding or changing a family touches
/// only its arm of `regmap`, never another family's.
#[derive(Debug, Clone, Copy)]
struct UartRegMap {
    status: u64,
    tx: u64,
    rx: u64,
    cr3: u64,
    /// CR1 base offset, or `None` for families with no CR1 interrupt concept.
    cr1: Option<u64>,
    txeie_mask: u32,
    tcie_mask: u32,
}

impl UartRegisterLayout {
    fn regmap(self) -> UartRegMap {
        match self {
            UartRegisterLayout::Stm32F1 => UartRegMap {
                status: 0x00, // SR
                tx: 0x04,     // DR
                rx: 0x04,     // DR
                cr3: 0x14,
                cr1: Some(0x0C),
                txeie_mask: 1 << 7, // TXEIE
                tcie_mask: 1 << 6,  // TCIE
            },
            UartRegisterLayout::Stm32V2 => UartRegMap {
                status: 0x1C, // ISR
                tx: 0x28,     // TDR
                rx: 0x24,     // RDR
                cr3: 0x08,
                cr1: Some(0x00),
                txeie_mask: 1 << 3, // TXEIE/TXFNFIE
                tcie_mask: 1 << 6,  // TCIE
            },
            UartRegisterLayout::Nrf52 => UartRegMap {
                status: 0x400, // EVENTS_TXDRDY
                tx: 0x51C,     // TXD
                rx: 0x518,     // RXD
                cr3: 0x500,    // ENABLE
                cr1: None,
                txeie_mask: 0,
                tcie_mask: 0,
            },
        }
    }
}

/// Minimal UART mock with selectable register layout.
#[derive(serde::Serialize)]
pub struct Uart {
    layout: UartRegisterLayout,
    #[serde(skip)]
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    #[serde(skip)]
    rx_buf: Arc<Mutex<VecDeque<u8>>>,
    echo_stdout: bool,
    /// CR1 register (tracks TXEIE and TE bits for interrupt-driven TX simulation).
    cr1: u32,
    cr3: u32,
    dma_tx_pending: bool,
    /// Stream devices attached to the RX path (e.g. GPS modules).
    #[serde(skip)]
    pub attached_streams: Vec<Box<dyn UartStreamDevice>>,
    #[serde(skip)]
    trace: VecDeque<UartTraceEvent>,
    #[serde(skip)]
    trace_seq: u64,
    /// Microseconds accumulated since last stream tick.
    elapsed_us: u32,
    /// Phase 2B.3b (issue #192): whether a self-perpetuating scheduler WAKE
    /// event is currently in flight. Guards against double-arming. Only used
    /// under the `event-scheduler` feature (flag-off drives via `tick()`).
    #[serde(skip)]
    scheduled: bool,
}

impl core::fmt::Debug for Uart {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Uart")
            .field("layout", &self.layout)
            .field("cr1", &self.cr1)
            .field("streams", &self.attached_streams.len())
            .finish()
    }
}

impl Default for Uart {
    fn default() -> Self {
        Self::new()
    }
}

impl Uart {
    pub fn new() -> Self {
        Self::new_with_layout(UartRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: UartRegisterLayout) -> Self {
        Self {
            layout,
            sink: None,
            rx_buf: Arc::new(Mutex::new(VecDeque::new())),
            echo_stdout: true,
            cr1: 0,
            cr3: 0,
            dma_tx_pending: false,
            attached_streams: Vec::new(),
            trace: VecDeque::new(),
            trace_seq: 0,
            elapsed_us: 0,
            scheduled: false,
        }
    }

    /// Attach a stream device to the UART RX path.
    pub fn attach_stream(&mut self, dev: Box<dyn UartStreamDevice>) {
        self.attached_streams.push(dev);
    }

    /// Get a shared handle to the RX buffer for external data injection.
    pub fn rx_buffer(&self) -> Arc<Mutex<VecDeque<u8>>> {
        self.rx_buf.clone()
    }

    /// Phase 2B.3b (issue #192): does the UART have anything that needs a
    /// per-tick wakeup? Level-triggered TXEIE/TCIE, an attached RX stream, or
    /// a pending DMA TX. Drives both the initial scheduler arm and the
    /// self-reschedule decision so the event path matches the legacy `tick()`.
    fn has_active_work(&self) -> bool {
        let txeie_set = (self.cr1 & self.txeie_mask()) != 0 && self.txeie_mask() != 0;
        let tcie_set = (self.cr1 & self.tcie_mask()) != 0 && self.tcie_mask() != 0;
        txeie_set || tcie_set || !self.attached_streams.is_empty() || self.dma_tx_pending
    }

    /// Phase 2B.3b: one tick-equivalent of work, shared verbatim by the legacy
    /// `tick()` and the scheduler `on_event` so both paths are identical.
    /// Returns `(raise_irq, dma_signals)`.
    fn advance_one_tick(&mut self) -> (bool, Vec<u32>) {
        let mut dma_signals = Vec::new();
        if self.dma_tx_pending {
            dma_signals.push(1); // 1 = TX Signal
            self.dma_tx_pending = false;
        }

        // Poll attached stream devices and push emitted bytes into the RX
        // buffer. Each tick represents ~1000 µs (1 ms) of simulated time. At
        // 9600 baud that is about 1 byte/ms, which matches the GPS pacing.
        if !self.attached_streams.is_empty() {
            const TICK_US: u32 = 1000;
            self.elapsed_us = self.elapsed_us.saturating_add(TICK_US);
            let elapsed = self.elapsed_us;
            self.elapsed_us = 0; // consumed this tick

            let rx_trace = if let Ok(mut rx_guard) = self.rx_buf.lock() {
                let mut rx_trace = Vec::new();
                for stream in &mut self.attached_streams {
                    if let Some(byte) = stream.poll(elapsed) {
                        rx_guard.push_back(byte);
                        rx_trace.push(byte);
                    }
                }
                rx_trace
            } else {
                Vec::new()
            };
            for byte in rx_trace {
                self.record_trace("rx", byte);
            }
        }

        // Fire while either TXEIE or TCIE is set:
        // - TXEIE lets HAL push bytes into DR
        // - TCIE delivers the final completion interrupt after the last byte
        let txeie_set = (self.cr1 & self.txeie_mask()) != 0 && self.txeie_mask() != 0;
        let tcie_set = (self.cr1 & self.tcie_mask()) != 0 && self.tcie_mask() != 0;
        (txeie_set || tcie_set, dma_signals)
    }

    // The 7 accessors below all read from the single per-family `regmap()`
    // descriptor, so a family's register map lives in exactly one place.
    fn status_offset(&self) -> u64 {
        self.layout.regmap().status
    }
    fn tx_offset(&self) -> u64 {
        self.layout.regmap().tx
    }
    fn rx_offset(&self) -> u64 {
        self.layout.regmap().rx
    }
    fn cr3_offset(&self) -> u64 {
        self.layout.regmap().cr3
    }
    /// Offset of the CR1 register. `None` for layouts without a CR1 interrupt concept.
    fn cr1_offset(&self) -> Option<u64> {
        self.layout.regmap().cr1
    }
    /// Bitmask of the TXEIE bit within CR1 for interrupt-driven TX detection.
    fn txeie_mask(&self) -> u32 {
        self.layout.regmap().txeie_mask
    }
    /// Bitmask of the transmission-complete interrupt enable bit within CR1.
    fn tcie_mask(&self) -> u32 {
        self.layout.regmap().tcie_mask
    }

    fn status_ready_value(&self) -> u8 {
        0xC0 // TX-ready + TC-ready in low byte for both layouts.
    }

    fn push_tx(&mut self, value: u8) {
        self.record_trace("tx", value);

        if let Some(sink) = &self.sink {
            if let Ok(mut guard) = sink.lock() {
                guard.push(value);
            }
        }

        for stream in &mut self.attached_streams {
            stream.on_tx_byte(value);
        }

        if self.echo_stdout {
            #[allow(unused_must_use)]
            {
                print!("{}", value as char);
                io::stdout().flush();
            }
        }
    }

    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }

    fn record_trace(&mut self, direction: &'static str, byte: u8) {
        self.trace_seq = self.trace_seq.wrapping_add(1);
        if self.trace.len() >= UART_TRACE_LIMIT {
            self.trace.pop_front();
        }
        self.trace.push_back(UartTraceEvent {
            seq: self.trace_seq,
            direction,
            byte,
        });
    }

    pub fn trace_snapshot(&self) -> Vec<UartTraceEvent> {
        self.trace.iter().cloned().collect()
    }
}

impl crate::Peripheral for Uart {
    fn read(&self, offset: u64) -> SimResult<u8> {
        if offset == self.status_offset() {
            let mut val = self.status_ready_value();
            // Set RXNE bit (bit 5) when RX buffer has data
            if let Ok(guard) = self.rx_buf.lock() {
                if !guard.is_empty() {
                    val |= 1 << 5; // RXNE
                }
            }
            return Ok(val);
        }
        if offset == self.rx_offset() {
            // Pop one byte from RX buffer
            if let Ok(mut guard) = self.rx_buf.lock() {
                return Ok(guard.pop_front().unwrap_or(0x00));
            }
            return Ok(0x00);
        }
        if offset == self.cr3_offset() {
            return Ok(self.cr3 as u8);
        }
        // Return CR1 bytes so interrupt-driven firmware can read back TXEIE state.
        if let Some(cr1_base) = self.cr1_offset() {
            let byte_offset = offset.wrapping_sub(cr1_base);
            if byte_offset < 4 {
                return Ok(((self.cr1 >> (byte_offset * 8)) & 0xFF) as u8);
            }
        }
        Ok(0)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let is_legacy_tx_alias =
            matches!(self.layout, UartRegisterLayout::Stm32F1) && offset == 0x00;

        if offset == self.tx_offset() || is_legacy_tx_alias {
            self.push_tx(value);
            // If DMAT bit is set, we might be in a DMA sequence.
            if (self.cr3 & (1 << 7)) != 0 {
                self.dma_tx_pending = true;
            }
        } else if offset == self.cr3_offset() {
            self.cr3 = value as u32;
            if (self.cr3 & (1 << 7)) != 0 {
                self.dma_tx_pending = true;
            }
        } else if let Some(cr1_base) = self.cr1_offset() {
            // Track CR1 byte-by-byte so TXEIE state is visible to tick().
            let byte_offset = offset.wrapping_sub(cr1_base);
            if byte_offset < 4 {
                let shift = byte_offset * 8;
                self.cr1 = (self.cr1 & !(0xFF << shift)) | ((value as u32) << shift);
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        let (irq, dma_signals) = self.advance_one_tick();
        crate::PeripheralTickResult {
            irq,
            dma_signals: (!dma_signals.is_empty()).then_some(dma_signals),
            ..Default::default()
        }
    }

    /// Phase 2B.3b (issue #192): the shared `Uart` is migrated to the event
    /// scheduler. With the feature on, the bus stops calling `tick()` every
    /// cycle; `take_scheduled_events` / `on_event` drive it instead. With the
    /// feature off this is ignored and `tick()` still runs.
    fn uses_scheduler(&self) -> bool {
        true
    }

    /// Hand the bus a single self-perpetuating WAKE event when the UART has
    /// active work and none is already in flight. Called after an MMIO write
    /// (TXEIE/TCIE arm, DMA trigger) and once at scheduler bootstrap (so an
    /// RX stream attached before firmware runs gets polled).
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.has_active_work() && !self.scheduled {
            self.scheduled = true;
            vec![(0, UART_WAKE_TOKEN)]
        } else {
            Vec::new()
        }
    }

    /// Fire one tick-equivalent of work and re-arm while there's still work.
    /// `raise_own_irq` mirrors the legacy `tick()` returning `irq: true` (the
    /// bus pends the UART's configured NVIC line); `dma_signals` route exactly
    /// as the legacy path; `reschedule_delay` keeps the level-triggered IRQ
    /// (and stream pacing) going at one event per tick until work drains.
    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let (irq, dma_signals) = self.advance_one_tick();
        let keep_going = self.has_active_work();
        self.scheduled = keep_going;
        crate::sched::EventResult {
            raise_own_irq: irq,
            dma_signals,
            reschedule_delay: keep_going.then_some(1),
            ..Default::default()
        }
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        if offset == self.status_offset() {
            let mut val = self.status_ready_value();
            if let Ok(guard) = self.rx_buf.lock() {
                if !guard.is_empty() {
                    val |= 1 << 5; // RXNE
                }
            }
            return Some(val);
        }
        if offset == self.rx_offset() {
            // Peek without consuming
            if let Ok(guard) = self.rx_buf.lock() {
                return Some(*guard.front().unwrap_or(&0x00));
            }
            return Some(0x00);
        }
        if offset == self.cr3_offset() {
            return Some(self.cr3 as u8);
        }
        if let Some(cr1_base) = self.cr1_offset() {
            let byte_offset = offset.wrapping_sub(cr1_base);
            if byte_offset < 4 {
                return Some(((self.cr1 >> (byte_offset * 8)) & 0xFF) as u8);
            }
        }
        Some(0)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::{Uart, UartRegisterLayout};
    use crate::Peripheral;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_uart_f1_transmit_offsets() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        uart.set_sink(Some(sink.clone()), false);

        // DR offset
        uart.write(0x04, b'A').unwrap();
        // Legacy alias for compatibility in existing fixtures
        uart.write(0x00, b'B').unwrap();

        let data = sink.lock().unwrap().clone();
        assert_eq!(data, vec![b'A', b'B']);
    }

    #[test]
    fn test_uart_v2_transmit_uses_tdr_only() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32V2);
        let sink = Arc::new(Mutex::new(Vec::new()));
        uart.set_sink(Some(sink.clone()), false);

        // Wrong offset for v2 should not transmit.
        uart.write(0x04, b'X').unwrap();
        // TDR offset
        uart.write(0x28, b'Y').unwrap();

        let data = sink.lock().unwrap().clone();
        assert_eq!(data, vec![b'Y']);
        assert_eq!(uart.read(0x1C).unwrap(), 0xC0); // ISR ready flags
    }

    #[test]
    fn test_uart_tick_raises_irq_for_tcie() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);

        // CR1 bit 6 = TCIE for STM32F1.
        uart.write(0x0C, 1 << 6).unwrap();

        assert!(uart.tick().irq);
    }

    // ── Phase 2B.3b: event-scheduler path ────────────────────────────────
    // These exercise the same `advance_one_tick` core as `tick()` but via the
    // scheduler hooks. The hooks aren't feature-gated (only the Machine/bus
    // *callers* are), so they run in both build configs.

    #[test]
    fn event_path_arms_and_raises_own_irq_for_tcie() {
        use crate::bus::SystemBus;
        use crate::sched::EventScheduler;

        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        uart.write(0x0C, 1 << 6).unwrap(); // TCIE → active work

        // Arms exactly one WAKE; a second take is a no-op (already scheduled).
        assert_eq!(
            uart.take_scheduled_events(),
            vec![(0, super::UART_WAKE_TOKEN)]
        );
        assert!(uart.take_scheduled_events().is_empty());

        // on_event raises the UART's *own* IRQ and re-arms while TCIE is set.
        let mut sched = EventScheduler::new();
        let mut bus = SystemBus::empty();
        let r = uart.on_event(super::UART_WAKE_TOKEN, &mut sched, &mut bus);
        assert!(r.raise_own_irq, "event path must request the own-IRQ pend");
        assert_eq!(r.reschedule_delay, Some(1), "re-arm while interrupt is set");
    }

    #[test]
    fn event_path_stops_when_interrupt_cleared() {
        use crate::bus::SystemBus;
        use crate::sched::EventScheduler;

        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        uart.write(0x0C, 1 << 6).unwrap(); // TCIE on
        let _ = uart.take_scheduled_events();

        // Clear TCIE → next event raises no IRQ and does not re-arm.
        uart.write(0x0C, 0).unwrap();
        let mut sched = EventScheduler::new();
        let mut bus = SystemBus::empty();
        let r = uart.on_event(super::UART_WAKE_TOKEN, &mut sched, &mut bus);
        assert!(!r.raise_own_irq);
        assert_eq!(r.reschedule_delay, None, "idle UART stops scheduling");
        // And it won't re-arm itself with no active work.
        assert!(uart.take_scheduled_events().is_empty());
    }

    #[test]
    fn event_path_paces_attached_stream_rx() {
        use super::UartStreamDevice;
        use crate::bus::SystemBus;
        use crate::sched::EventScheduler;

        struct OneByte(u8);
        impl UartStreamDevice for OneByte {
            fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
                Some(self.0)
            }
        }

        let mut uart = Uart::new();
        uart.attach_stream(Box::new(OneByte(b'G')));

        // A stream attached at setup is "active work" → arms at bootstrap.
        assert_eq!(
            uart.take_scheduled_events(),
            vec![(0, super::UART_WAKE_TOKEN)]
        );

        let mut sched = EventScheduler::new();
        let mut bus = SystemBus::empty();
        let rx = uart.rx_buffer();
        uart.on_event(super::UART_WAKE_TOKEN, &mut sched, &mut bus);
        assert_eq!(rx.lock().unwrap().front().copied(), Some(b'G'));
    }

    #[test]
    fn attached_stream_observes_firmware_tx_bytes() {
        use super::UartStreamDevice;
        use std::sync::{Arc, Mutex};

        struct Recorder(Arc<Mutex<Vec<u8>>>);
        impl UartStreamDevice for Recorder {
            fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
                None
            }
            fn on_tx_byte(&mut self, byte: u8) {
                self.0.lock().unwrap().push(byte);
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut uart = Uart::new(); // Stm32F1 layout
        uart.set_sink(None, false); // disable stdout echo
        uart.attach_stream(Box::new(Recorder(seen.clone())));

        // Stm32F1: writing the DR alias at offset 0x00 transmits a byte.
        uart.write(0x00, 0x42).unwrap();

        assert_eq!(*seen.lock().unwrap(), vec![0x42]);
    }

    #[test]
    fn uart_trace_snapshot_records_tx_and_rx_without_draining_buffers() {
        use super::UartStreamDevice;

        struct OneByte(Option<u8>);
        impl UartStreamDevice for OneByte {
            fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
                self.0.take()
            }
        }

        let mut uart = Uart::new();
        let sink = Arc::new(Mutex::new(Vec::new()));
        uart.set_sink(Some(sink.clone()), false);
        uart.attach_stream(Box::new(OneByte(Some(0x33))));

        uart.write(0x04, 0x42).unwrap();
        uart.tick();

        let trace = uart.trace_snapshot();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].direction, "tx");
        assert_eq!(trace[0].byte, 0x42);
        assert_eq!(trace[1].direction, "rx");
        assert_eq!(trace[1].byte, 0x33);
        assert_eq!(sink.lock().unwrap().as_slice(), &[0x42]);
        assert_eq!(uart.read(0x04).unwrap(), 0x33);
    }
}
