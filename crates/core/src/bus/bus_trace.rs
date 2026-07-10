//! Universal bus-transaction trace: tracing device wrappers + a shared log.
//!
//! A wrapper sits between any I²C/SPI master and its attached device, forwards
//! every trait call (so behaviour is unchanged, including `as_any` downcasts),
//! and records each transacted byte into a shared `BusTraceLog`. Because every
//! family attaches through `I2c::attach` / `Spi::attach`, one wrapper covers
//! all chips.

use std::collections::VecDeque;
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
    pub bus: String,
    pub payload: BusPayload,
}

#[derive(Debug, Default)]
pub struct BusTraceRing {
    seq: u64,
    events: VecDeque<BusTraceEvent>,
}

impl BusTraceRing {
    pub fn push(&mut self, bus: &str, payload: BusPayload) {
        self.seq = self.seq.wrapping_add(1);
        if self.events.len() >= BUS_TRACE_LIMIT {
            self.events.pop_front();
        }
        self.events.push_back(BusTraceEvent {
            seq: self.seq,
            bus: bus.to_string(),
            payload,
        });
    }
    pub fn snapshot(&self) -> Vec<BusTraceEvent> {
        self.events.iter().cloned().collect()
    }
}

pub type BusTraceLog = Arc<Mutex<BusTraceRing>>;
pub fn new_log() -> BusTraceLog {
    Arc::new(Mutex::new(BusTraceRing::default()))
}

pub struct TracingI2cDevice {
    bus: String,
    log: BusTraceLog,
    inner: Box<dyn I2cDevice>,
    expect_address: bool, // next write is the address byte (set on start())
}

impl TracingI2cDevice {
    pub fn new(bus: String, log: BusTraceLog, inner: Box<dyn I2cDevice>) -> Self {
        Self {
            bus,
            log,
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
        self.log.lock().unwrap().push(
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
            self.log.lock().unwrap().push(
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
            self.log.lock().unwrap().push(
                &self.bus,
                BusPayload::I2c {
                    kind: I2cSym::AddrRead,
                    byte: addr_byte,
                    ack: true,
                },
            );
        }
        let b = self.inner.read();
        self.log.lock().unwrap().push(
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
    log: BusTraceLog,
    inner: Box<dyn SpiDevice>,
}

impl TracingSpiDevice {
    pub fn new(bus: String, log: BusTraceLog, inner: Box<dyn SpiDevice>) -> Self {
        Self { bus, log, inner }
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
        self.log
            .lock()
            .unwrap()
            .push(&self.bus, BusPayload::Spi { mosi, miso });
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
        let snap = log.lock().unwrap().snapshot();
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
}
