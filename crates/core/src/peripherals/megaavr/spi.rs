// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! megaAVR 0-series SPI host (master) mode, normal (unbuffered) mode
//! (ATmega4808/4809 data sheet DS40002173 section 24; register map and reset
//! values from the ATmega4809 ATDF `SPI` module).
//!
//! Registers: CTRLA 0x00, CTRLB 0x01, INTCTRL 0x02, INTFLAGS 0x03, DATA 0x04.
//!
//! Modelled, against attached byte-level `SpiDevice`s:
//! - A DATA write while CTRLA.ENABLE and CTRLA.MASTER are set clocks one byte
//!   out and one byte in, and sets INTFLAGS.IF. CTRLA.DORD = 1 (LSB first)
//!   reverses the bit order on the wire in both directions, so the device sees
//!   the bits in the order they would be shifted.
//! - IF clears the way the data sheet gives for normal mode: by reading
//!   INTFLAGS with IF set and then accessing DATA. WRCOL never sets, because
//!   a transfer has finished before the next instruction can write DATA.
//! - A DATA read returns the last byte received.
//!
//! Every transfer completes at once: PRESC / CLK2X and the CTRLB.MODE clock
//! phase and polarity are stored, not timed or sampled. The SS pin is not
//! observed, so neither CTRLB.SSD = 0 multi-master detection nor chip select
//! reaches the devices: every attached device sees every byte (soft-CS
//! broadcast, the AVR path's convention), and a device answering non-zero
//! drives MISO. Refused rather than ignored: buffer mode (CTRLB.BUFEN), the
//! client (slave) side (ENABLE without MASTER), and any interrupt enable
//! (INTCTRL), since the vector's hardware clear of IF is not modelled.

use crate::peripherals::device::SpiDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;
use std::cell::Cell;

const CTRLA: u64 = 0x00;
const CTRLB: u64 = 0x01;
const INTCTRL: u64 = 0x02;
const INTFLAGS: u64 = 0x03;
const DATA: u64 = 0x04;

const CTRLA_DORD: u8 = 1 << 6;
const CTRLA_MASTER: u8 = 1 << 5;
const CTRLA_ENABLE: u8 = 1 << 0;
const CTRLB_BUFEN: u8 = 1 << 7;
const IF: u8 = 1 << 7;

pub struct MegaAvrSpi {
    ctrla: u8,
    ctrlb: u8,
    /// IF only; a Cell because the clearing DATA access may be a load.
    intflags: Cell<u8>,
    data: u8,
    /// INTFLAGS was read with IF set: the next DATA access clears IF.
    if_seen: Cell<bool>,
    devices: Vec<Box<dyn SpiDevice>>,
    /// Evidence: bytes sent (capped) and completed byte transfers.
    mosi: Vec<u8>,
    bytes: u64,
}

impl std::fmt::Debug for MegaAvrSpi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MegaAvrSpi")
            .field("ctrla", &self.ctrla)
            .field("devices", &self.devices.len())
            .finish_non_exhaustive()
    }
}

impl Default for MegaAvrSpi {
    fn default() -> Self {
        Self::new()
    }
}

impl MegaAvrSpi {
    pub fn new() -> Self {
        Self {
            ctrla: 0,
            ctrlb: 0,
            intflags: Cell::new(0),
            data: 0,
            if_seen: Cell::new(false),
            devices: Vec::new(),
            mosi: Vec::new(),
            bytes: 0,
        }
    }

    pub fn push_device(&mut self, dev: Box<dyn SpiDevice>) {
        self.devices.push(dev);
    }

    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "megaAVR SPI: {what} is not modelled"
        )))
    }

    /// Normal mode: reading INTFLAGS with IF set, then accessing DATA.
    fn data_access(&self) {
        if self.if_seen.take() {
            self.intflags.set(self.intflags.get() & !IF);
        }
    }

    fn transfer(&mut self, v: u8) {
        let lsb_first = self.ctrla & CTRLA_DORD != 0;
        let wire = if lsb_first { v.reverse_bits() } else { v };
        let mut miso = 0u8;
        for dev in &mut self.devices {
            let resp = dev.transfer(wire);
            if resp != 0 {
                miso = resp;
            }
        }
        self.data = if lsb_first { miso.reverse_bits() } else { miso };
        if self.mosi.len() < 64 {
            self.mosi.push(v);
        }
        self.bytes += 1;
        self.intflags.set(self.intflags.get() | IF);
    }

    fn reg(&self, o: u64) -> Option<u8> {
        Some(match o {
            CTRLA => self.ctrla,
            CTRLB => self.ctrlb,
            INTCTRL => 0,
            INTFLAGS => self.intflags.get(),
            DATA => self.data,
            _ => return None,
        })
    }

    fn write_reg(&mut self, o: u64, v: u8) -> SimResult<()> {
        match o {
            CTRLA => {
                if v & CTRLA_ENABLE != 0 && v & CTRLA_MASTER == 0 {
                    return Self::refuse("the SPI client (CTRLA.ENABLE without MASTER)");
                }
                self.ctrla = v & 0x77;
            }
            CTRLB => {
                if v & CTRLB_BUFEN != 0 {
                    return Self::refuse("buffer mode (CTRLB.BUFEN)");
                }
                self.ctrlb = v & 0xC7;
            }
            INTCTRL if v != 0 => return Self::refuse("SPI interrupts (INTCTRL)"),
            INTCTRL => {}
            // Normal mode: IF and WRCOL are cleared by the DATA access
            // sequence, not by writing INTFLAGS.
            INTFLAGS => {}
            DATA => {
                self.data_access();
                if self.ctrla & CTRLA_ENABLE != 0 {
                    self.transfer(v);
                } else {
                    self.data = v;
                }
            }
            _ => return Self::refuse("a store outside the register map"),
        }
        Ok(())
    }
}

impl Peripheral for MegaAvrSpi {
    fn spi_attached_devices(&self) -> Option<&Vec<Box<dyn SpiDevice>>> {
        Some(&self.devices)
    }
    fn spi_attached_devices_mut(&mut self) -> Option<&mut Vec<Box<dyn SpiDevice>>> {
        Some(&mut self.devices)
    }
    fn read(&self, o: u64) -> SimResult<u8> {
        let Some(v) = self.reg(o) else {
            return Self::refuse("a load outside the register map");
        };
        match o {
            INTFLAGS if v & IF != 0 => self.if_seen.set(true),
            DATA => self.data_access(),
            _ => {}
        }
        Ok(v)
    }
    fn peek(&self, o: u64) -> Option<u8> {
        self.reg(o)
    }
    fn write(&mut self, o: u64, v: u8) -> SimResult<()> {
        self.write_reg(o, v)
    }
    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }
    fn on_node_reset(&mut self) {
        let devices = std::mem::take(&mut self.devices);
        let (mosi, bytes) = (std::mem::take(&mut self.mosi), self.bytes);
        *self = Self::new();
        self.devices = devices;
        self.mosi = mosi;
        self.bytes = bytes;
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "megaavr_spi", "ctrla": self.ctrla,
            "ctrlb": self.ctrlb, "intflags": self.intflags.get(), "mosi": self.mosi,
            "bytes": self.bytes, "devices": self.devices.len()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A read-only sensor that clocks out a fixed frame and logs MOSI.
    struct Frame {
        out: Vec<u8>,
        idx: usize,
        log: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }
    impl SpiDevice for Frame {
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.log.lock().unwrap().push(mosi);
            let v = self.out[self.idx % self.out.len()];
            self.idx += 1;
            v
        }
        fn cs_pin(&self) -> &str {
            "PE3"
        }
    }

    fn spi(out: &[u8]) -> (MegaAvrSpi, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut s = MegaAvrSpi::new();
        s.push_device(Box::new(Frame {
            out: out.to_vec(),
            idx: 0,
            log: log.clone(),
        }));
        (s, log)
    }

    /// ArduinoCore-megaavr SPIClass::transfer: DATA = b; spin on RXCIF; read DATA.
    fn arduino_transfer(s: &mut MegaAvrSpi, b: u8) -> u8 {
        s.write(DATA, b).unwrap();
        assert_ne!(s.read(INTFLAGS).unwrap() & IF, 0);
        s.read(DATA).unwrap()
    }

    #[test]
    fn arduino_begin_then_four_byte_frame() {
        let (mut s, log) = spi(&[0x01, 0x90, 0x16, 0x00]);
        // SPI.begin(): CTRLB |= SSD; CTRLA |= ENABLE | MASTER; then
        // config(DEFAULT_SPI_SETTINGS) at 4 MHz, MSB first, mode 0.
        s.write(CTRLB, 1 << 2).unwrap();
        s.write(CTRLA, CTRLA_ENABLE | CTRLA_MASTER).unwrap();
        s.write(CTRLA, CTRLA_ENABLE | CTRLA_MASTER | (1 << 4))
            .unwrap();
        s.write(CTRLB, 1 << 2).unwrap();
        let mut frame = 0u32;
        for _ in 0..4 {
            frame = (frame << 8) | u32::from(arduino_transfer(&mut s, 0));
        }
        assert_eq!(frame, 0x0190_1600);
        assert_eq!(*log.lock().unwrap(), [0, 0, 0, 0]);
        assert_eq!(s.read(INTFLAGS).unwrap(), 0, "the DATA read cleared IF");
        assert_eq!(s.bytes, 4);
    }

    #[test]
    fn if_clears_only_after_intflags_was_read() {
        let (mut s, _) = spi(&[0x5A]);
        s.write(CTRLA, CTRLA_ENABLE | CTRLA_MASTER).unwrap();
        s.write(DATA, 0x11).unwrap();
        assert_eq!(s.read(DATA).unwrap(), 0x5A);
        assert_eq!(s.peek(INTFLAGS), Some(IF), "DATA without INTFLAGS first");
        assert_eq!(s.read(INTFLAGS).unwrap(), IF);
        assert_eq!(s.peek(INTFLAGS), Some(IF), "peek has no side effect");
        s.read(DATA).unwrap();
        assert_eq!(s.read(INTFLAGS).unwrap(), 0);
    }

    #[test]
    fn lsb_first_reverses_the_wire_both_ways() {
        let (mut s, log) = spi(&[0x80]);
        s.write(CTRLA, CTRLA_ENABLE | CTRLA_MASTER | CTRLA_DORD)
            .unwrap();
        assert_eq!(arduino_transfer(&mut s, 0x01), 0x01);
        assert_eq!(*log.lock().unwrap(), [0x80]);
    }

    #[test]
    fn disabled_data_write_does_not_clock() {
        let (mut s, log) = spi(&[0xFF]);
        s.write(DATA, 0x42).unwrap();
        assert!(log.lock().unwrap().is_empty());
        assert_eq!(s.read(INTFLAGS).unwrap(), 0);
        assert_eq!(s.read(DATA).unwrap(), 0x42);
    }

    #[test]
    fn modes_outside_the_model_are_refused() {
        let (mut s, _) = spi(&[0]);
        assert!(s.write(CTRLA, CTRLA_ENABLE).is_err(), "client side");
        assert!(s.write(CTRLB, CTRLB_BUFEN).is_err());
        assert!(s.write(INTCTRL, 1).is_err());
        assert!(s.read(0x05).is_err());
        assert!(s.write(0x05, 0).is_err());
        // SPIClass::end() clears ENABLE.
        s.write(CTRLA, CTRLA_MASTER).unwrap();
        s.write(INTCTRL, 0).unwrap();
    }
}
