//! Universal bus-transaction trace: tracing device wrappers + a shared log.
//!
//! A wrapper sits between any I²C/SPI master and its attached device, forwards
//! every trait call (so behaviour is unchanged, including `as_any` downcasts),
//! and records each transacted byte into a shared [`BusTrace`].
//!
//! ## One choke point (no per-callsite `set_bus_trace`)
//!
//! A slave is wrapped in exactly ONE place — the free functions [`wrap_i2c`] /
//! [`wrap_spi`] below — and those are reached through a single funnel:
//! [`crate::bus::SystemBus::attach_i2c_slave`] /
//! [`crate::bus::SystemBus::attach_spi_device`]. Controllers no longer carry a
//! trace handle and their raw `push_slave` / `push_device` methods do NOT wrap;
//! the only way to hand a controller a slave is through the bus funnel, which
//! always wraps. That makes it impossible to attach a device that bypasses the
//! trace: a controller family the funnel does not recognise is a hard error, not
//! a silently untraced bus (the failure mode that once shipped on ESP32-C3).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::peripherals::i2c::I2cDevice;
use crate::peripherals::spi::SpiDevice;

const BUS_TRACE_LIMIT: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum I2cSym {
    AddrWrite,
    AddrRead,
    Data,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum BusPayload {
    I2c { kind: I2cSym, byte: u8, ack: bool },
    Spi { mosi: u8, miso: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BusTraceEvent {
    pub seq: u64,
    /// Engine cycle counter at the moment the byte transacted, mirrored from the
    /// bus's `current_cycle` via the shared clock (see [`BusTrace::set_cycle`]).
    /// Lets the UI time-align protocol decode with sampled waveforms on one
    /// cycle axis. `0` until the machine has stepped at least once.
    pub cycle: u64,
    pub bus: String,
    pub payload: BusPayload,
}

#[derive(Debug, Default)]
pub struct BusTraceRing {
    seq: u64,
    events: VecDeque<BusTraceEvent>,
}

impl BusTraceRing {
    fn push(&mut self, cycle: u64, bus: &str, payload: BusPayload) {
        self.seq = self.seq.wrapping_add(1);
        if self.events.len() >= BUS_TRACE_LIMIT {
            self.events.pop_front();
        }
        self.events.push_back(BusTraceEvent {
            seq: self.seq,
            cycle,
            bus: bus.to_string(),
            payload,
        });
    }
    pub fn snapshot(&self) -> Vec<BusTraceEvent> {
        self.events.iter().cloned().collect()
    }
}

/// Shared bus-trace handle: a ring-buffered event log plus a shared cycle clock
/// the bus advances once per step. Cloning shares both (Arc), so every wrapper
/// stamping into the log reads the same "now". Cheap to clone.
#[derive(Clone)]
pub struct BusTrace {
    ring: Arc<Mutex<BusTraceRing>>,
    clock: Arc<AtomicU64>,
}

impl Default for BusTrace {
    fn default() -> Self {
        Self::new()
    }
}

impl BusTrace {
    pub fn new() -> Self {
        Self {
            ring: Arc::new(Mutex::new(BusTraceRing::default())),
            clock: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Publish the current engine cycle so subsequent trace events are stamped
    /// with it. Called by the bus once per step from `current_cycle`; a plain
    /// atomic store, no lock.
    pub fn set_cycle(&self, cycle: u64) {
        self.clock.store(cycle, Ordering::Relaxed);
    }

    /// Record one transacted symbol, stamped with the shared clock's "now".
    pub fn push(&self, bus: &str, payload: BusPayload) {
        let cycle = self.clock.load(Ordering::Relaxed);
        self.ring.lock().unwrap().push(cycle, bus, payload);
    }

    pub fn snapshot(&self) -> Vec<BusTraceEvent> {
        self.ring.lock().unwrap().snapshot()
    }
}

/// Retained name for the shared handle type; every reference means [`BusTrace`].
pub type BusTraceLog = BusTrace;

pub fn new_log() -> BusTrace {
    BusTrace::new()
}

/// The single point at which an I²C slave is wrapped for tracing. Reached only
/// through [`crate::bus::SystemBus::attach_i2c_slave`] (and the nRF52 serial mux,
/// which must attach at build time) — never from a controller's own attach.
pub fn wrap_i2c(bus: &str, trace: &BusTrace, dev: Box<dyn I2cDevice>) -> Box<dyn I2cDevice> {
    Box::new(TracingI2cDevice::new(bus.to_string(), trace.clone(), dev))
}

/// The single point at which a SPI device is wrapped for tracing (see
/// [`wrap_i2c`]).
pub fn wrap_spi(bus: &str, trace: &BusTrace, dev: Box<dyn SpiDevice>) -> Box<dyn SpiDevice> {
    Box::new(TracingSpiDevice::new(bus.to_string(), trace.clone(), dev))
}

pub struct TracingI2cDevice {
    bus: String,
    trace: BusTrace,
    inner: Box<dyn I2cDevice>,
    expect_address: bool, // next write is the address byte (set on start())
}

impl TracingI2cDevice {
    pub fn new(bus: String, trace: BusTrace, inner: Box<dyn I2cDevice>) -> Self {
        Self {
            bus,
            trace,
            inner,
            expect_address: false,
        }
    }
}

impl I2cDevice for TracingI2cDevice {
    fn address(&self) -> u8 {
        self.inner.address()
    }
    fn start(&mut self) {
        self.expect_address = true;
        self.inner.start();
    }
    fn stop(&mut self) {
        self.inner.stop();
    }
    /// Forward the wall-clock advance to the wrapped device. Without this the
    /// trace wrapper swallows the master's `advance_time_us` call (the trait
    /// default is a no-op), so a real sensor behind the trace — e.g. a MAX30102
    /// on the nRF54L TWIM — would never advance its own sample clock and its
    /// FIFO could never overrun under CPU starvation. Completing the wall-clock
    /// change means the wrapper must be transparent to it.
    fn advance_time_us(&mut self, us: u64) {
        self.inner.advance_time_us(us);
    }
    fn write(&mut self, data: u8) {
        // The master selects this device by address, then calls start() + write()/read().
        // The wrapper reconstructs the framing universally: the FIRST transfer after a
        // start() is the address (direction inferred: write => AddrWrite), using the
        // device's own address(); subsequent transfers are Data. No master cooperation
        // needed, so this works identically for every chip family.
        let addr_byte = self.inner.address() << 1; // write (R/W bit = 0)
        let kind = if self.expect_address {
            I2cSym::AddrWrite
        } else {
            I2cSym::Data
        };
        self.expect_address = false;
        self.inner.write(data);
        let byte = if matches!(kind, I2cSym::AddrWrite) {
            addr_byte
        } else {
            data
        };
        self.trace.push(
            &self.bus,
            BusPayload::I2c {
                kind,
                byte,
                ack: true,
            },
        );
        // When the first write IS the address frame, the data byte still flows to the
        // device; emit it as a following Data event so no payload byte is lost.
        if matches!(kind, I2cSym::AddrWrite) {
            self.trace.push(
                &self.bus,
                BusPayload::I2c {
                    kind: I2cSym::Data,
                    byte: data,
                    ack: true,
                },
            );
        }
    }
    fn read(&mut self) -> u8 {
        if self.expect_address {
            // A read transaction: synthesize the address frame (R) before the first byte.
            self.expect_address = false;
            let addr_byte = (self.inner.address() << 1) | 1; // read
            self.trace.push(
                &self.bus,
                BusPayload::I2c {
                    kind: I2cSym::AddrRead,
                    byte: addr_byte,
                    ack: true,
                },
            );
        }
        let b = self.inner.read();
        self.trace.push(
            &self.bus,
            BusPayload::I2c {
                kind: I2cSym::Data,
                byte: b,
                ack: true,
            },
        );
        b
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.inner.as_any()
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        self.inner.as_any_mut()
    }
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        self.inner.as_sim_input_mut()
    }
}

pub struct TracingSpiDevice {
    bus: String,
    trace: BusTrace,
    inner: Box<dyn SpiDevice>,
}

impl TracingSpiDevice {
    pub fn new(bus: String, trace: BusTrace, inner: Box<dyn SpiDevice>) -> Self {
        Self { bus, trace, inner }
    }
}

impl SpiDevice for TracingSpiDevice {
    fn cs_select(&mut self) {
        self.inner.cs_select();
    }
    fn cs_release(&mut self) {
        self.inner.cs_release();
    }
    fn transfer(&mut self, mosi: u8) -> u8 {
        let miso = self.inner.transfer(mosi);
        self.trace.push(&self.bus, BusPayload::Spi { mosi, miso });
        miso
    }
    fn cs_pin(&self) -> &str {
        self.inner.cs_pin()
    }
    fn dc_pin(&self) -> Option<&str> {
        self.inner.dc_pin()
    }
    fn set_dc_level(&mut self, level: bool) {
        self.inner.set_dc_level(level);
    }
    fn dc_source(&self) -> Option<(u64, u8)> {
        self.inner.dc_source()
    }
    fn set_dc_source(&mut self, odr_addr: u64, bit: u8) {
        self.inner.set_dc_source(odr_addr, bit);
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.inner.as_any()
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        self.inner.as_any_mut()
    }
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        self.inner.as_sim_input_mut()
    }
    fn runtime_snapshot(&self) -> Vec<u8> {
        self.inner.runtime_snapshot()
    }
    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> crate::SimResult<()> {
        self.inner.restore_runtime_snapshot(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::i2c::I2cDevice;

    struct Dev {
        addr: u8,
    }
    impl I2cDevice for Dev {
        fn address(&self) -> u8 {
            self.addr
        }
        fn read(&mut self) -> u8 {
            0xC7
        }
        fn write(&mut self, _b: u8) {}
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            Some(self)
        }
    }

    #[test]
    fn tracing_i2c_wrapper_records_writes_and_is_transparent_to_downcast() {
        let log = new_log();
        let mut w = TracingI2cDevice::new("i2c1".into(), log.clone(), Box::new(Dev { addr: 0x1E }));
        // simulate a master write of one data byte
        I2cDevice::write(&mut w, 0xAF);
        let snap = log.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].bus, "i2c1");
        match &snap[0].payload {
            BusPayload::I2c { byte, .. } => assert_eq!(*byte, 0xAF),
            _ => panic!("wrong payload"),
        }
        // transparency: downcast through the wrapper still finds the inner Dev
        let any = I2cDevice::as_any(&w).expect("wrapper forwards as_any");
        assert!(
            any.downcast_ref::<Dev>().is_some(),
            "as_any must forward to inner"
        );
    }

    /// Regression guard: the trace wrapper MUST forward `advance_time_us` to the
    /// inner device. Every I²C slave on the nRF54L TWIM is wrapped here, so a
    /// wrapper that swallows the master's wall-clock advance leaves a real
    /// sensor's sample clock frozen — its FIFO can never overrun under CPU
    /// starvation, and the whole BLE-contention model silently reads zero. The
    /// production path is `factory → wrap_i2c → TWIM`, which the TWIM's own
    /// unit test (an *unwrapped* mock) does not exercise; this closes that gap.
    #[test]
    fn tracing_i2c_wrapper_forwards_advance_time_us() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

        struct Timed {
            advanced_us: Arc<AtomicU64>,
        }
        impl I2cDevice for Timed {
            fn address(&self) -> u8 {
                0x57
            }
            fn read(&mut self) -> u8 {
                0
            }
            fn write(&mut self, _b: u8) {}
            fn advance_time_us(&mut self, us: u64) {
                self.advanced_us.fetch_add(us, Ordering::Relaxed);
            }
        }

        let advanced = Arc::new(AtomicU64::new(0));
        let mut w = wrap_i2c(
            "twi21",
            &new_log(),
            Box::new(Timed {
                advanced_us: advanced.clone(),
            }),
        );

        I2cDevice::advance_time_us(&mut *w, 1234);

        assert_eq!(
            advanced.load(Ordering::Relaxed),
            1234,
            "the trace wrapper must be transparent to advance_time_us"
        );
    }

    #[test]
    fn events_are_stamped_with_the_shared_clock_cycle() {
        let log = new_log();
        let mut w = TracingI2cDevice::new("i2c1".into(), log.clone(), Box::new(Dev { addr: 0x1E }));
        // First byte transacts at cycle 0 (clock never advanced).
        I2cDevice::write(&mut w, 0x01);
        // Advance the shared clock, then transact again.
        log.set_cycle(4242);
        I2cDevice::write(&mut w, 0x02);
        let snap = log.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].cycle, 0, "first event predates any step");
        assert_eq!(
            snap[1].cycle, 4242,
            "second event carries the advanced cycle"
        );
    }
}
