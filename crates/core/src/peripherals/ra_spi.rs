// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RA serial peripheral interface (RSPI), master mode.
//!
//! Register layout from FSP `R7FA4M1AB.h` (`R_SPI0_Type`, 0x40 bytes; SPI0 at
//! 0x40072000, SPI1 at +0x100): byte registers `SPCR` +0x00, `SSLP` +0x01,
//! `SPPCR` +0x02, `SPSR` +0x03, `SPDR` +0x04 (32-bit, with the halfword
//! `SPDR_HA` and byte `SPDR_BY` views at the same offset), `SPSCR` +0x08,
//! `SPSSR` +0x09, `SPBR` +0x0A, `SPDCR` +0x0B, `SPCKD` +0x0C, `SSLND` +0x0D,
//! `SPND` +0x0E, `SPCR2` +0x0F, `SPCMD0`..`SPCMD7` +0x10 (16-bit), `SPDCR2`
//! +0x20, `SPCR3` +0x21, `SPPR` +0x3E. Behaviour is what ArduinoCore-renesas
//! 1.x `SPI.cpp` drives with direct register access (no FSP driver, no
//! interrupts): byte frames through `SPDR_BY` polling `SPSR.SPRF`, and
//! `transfer(buf, n)` as 32-bit frames with `SPDCR.SPLW` and `SPDCR2.BYSW`,
//! two words queued ahead on `SPSR.SPTEF`.
//!
//! Modelled, against attached byte-level `SpiDevice`s:
//! - An `SPDR` store with `SPCR.SPE` and `MSTR` set exchanges one frame of
//!   `SPCMD0.SPB` bits (8, 16 or 32) with every attached device, at once.
//!   `SPCMD0.LSBF` sends the frame least-significant bit first; the device
//!   sees each byte in wire order (first bit = bit 7, the `megaavr_spi`
//!   convention). `SPDCR2.BYSW` swaps the bytes of a 16- or 32-bit frame on
//!   the way out and back, so a little-endian buffer goes out in memory
//!   order.
//! - The receive buffer, the shift register and the transmit buffer hold
//!   one frame each. `SPRF` is set while a received frame is unread; an
//!   `SPDR` load returns it and lets the next queued frame land. `SPTEF`
//!   is 0 only when all three are occupied; a store then is dropped, as on
//!   silicon. `SPSR.IDLNF` reads 1 while a frame is held behind the
//!   receive buffer.
//! - `SPSR` error flags (`OVRF`, `MODF`, `PERF`, `UDRF`) clear on a 0
//!   write; none of them ever sets.
//!
//! Timing is not modelled: `SPBR` / `SPCMDn.BRDV`, the clock phase and
//! polarity, and the `SPCKD` / `SSLND` / `SPND` delays are stored only. A
//! frame that would finish while `SPRF` is still set waits for the `SPDR`
//! load instead, so the model never overruns (`OVRF`) where a slow reader
//! would on silicon. The SSL pins are not driven: every attached device
//! sees every frame (the soft-CS broadcast convention), and a device
//! answering non-zero drives MISO.
//!
//! Refused rather than ignored: slave mode (`SPE` without `MSTR`),
//! transmit-only mode (`TXMD`), every interrupt enable (`SPRIE`, `SPTIE`,
//! `SPEIE`, `SPCR2.SPIIE`, `SPCR3.CENDIE`), and, at the transfer, a
//! command sequence (`SPSCR` != 0), multi-frame mode (`SPDCR.SPFC`),
//! transmit-buffer reads (`SPRDTD`), loopback (`SPPCR.SPLP` / `SPLP2`),
//! parity (`SPCR2.SPPE`), and a frame length that is not 8, 16 or 32.

use crate::peripherals::device::SpiDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

const SPCR: u64 = 0x00;
const SPPCR: u64 = 0x02;
const SPSR: u64 = 0x03;
const SPDR: u64 = 0x04;
const SPSCR: u64 = 0x08;
const SPSSR: u64 = 0x09;
const SPBR: u64 = 0x0A;
const SPDCR: u64 = 0x0B;
const SPCR2: u64 = 0x0F;
const SPCMD0: u64 = 0x10;
const SPDCR2: u64 = 0x20;
const SPCR3: u64 = 0x21;
const SPPR: u64 = 0x3E;
const SIZE: usize = 0x40;

const SPCR_TXMD: u8 = 1 << 1;
const SPCR_MSTR: u8 = 1 << 3;
const SPCR_SPEIE: u8 = 1 << 4;
const SPCR_SPTIE: u8 = 1 << 5;
const SPCR_SPE: u8 = 1 << 6;
const SPCR_SPRIE: u8 = 1 << 7;
const SPPCR_LOOPBACK: u8 = 0x03;
const SPSR_IDLNF: u8 = 1 << 1;
const SPSR_SPTEF: u8 = 1 << 5;
const SPSR_SPRF: u8 = 1 << 7;
/// OVRF, MODF, PERF, UDRF: the write-0-to-clear error flags.
const SPSR_ERRORS: u8 = 0x1D;
const SPDCR_SPFC: u8 = 0x03;
const SPDCR_SPRDTD: u8 = 1 << 4;
const SPCR2_SPPE: u8 = 1 << 0;
const SPCR2_SPIIE: u8 = 1 << 2;
const SPCR3_CENDIE: u8 = 1 << 4;
const SPDCR2_BYSW: u8 = 1 << 0;
/// Receive buffer + shift register + transmit buffer.
const DEPTH: usize = 3;

pub struct RaSpi {
    /// Every byte register except SPSR and SPDR, by offset.
    regs: [u8; SIZE],
    /// SPSR error flags (never set by the model; stored for the 0-write).
    errors: u8,
    /// Received frames not yet read, oldest first. Index 0 is the receive
    /// buffer; the rest are frames held behind it.
    rx: RefCell<VecDeque<u32>>,
    /// The value an SPDR load returns with nothing received.
    last: Cell<u32>,
    devices: Vec<Box<dyn SpiDevice>>,
    /// Evidence: the first frames sent (capped) and the frame count.
    mosi: Vec<u32>,
    frames: u64,
}

impl std::fmt::Debug for RaSpi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RaSpi")
            .field("spcr", &self.regs[SPCR as usize])
            .field("devices", &self.devices.len())
            .finish_non_exhaustive()
    }
}

impl Default for RaSpi {
    fn default() -> Self {
        Self::new()
    }
}

impl RaSpi {
    pub fn new() -> Self {
        let mut regs = [0u8; SIZE];
        regs[SPBR as usize] = 0xFF;
        // SPCMDn reset 0x070D: SPB = 7 (8 bits), BRDV = 3, CPOL = 0, CPHA = 1.
        for n in 0..8 {
            regs[SPCMD0 as usize + 2 * n] = 0x0D;
            regs[SPCMD0 as usize + 2 * n + 1] = 0x07;
        }
        Self {
            regs,
            errors: 0,
            rx: RefCell::new(VecDeque::new()),
            last: Cell::new(0),
            devices: Vec::new(),
            mosi: Vec::new(),
            frames: 0,
        }
    }

    pub fn push_device(&mut self, dev: Box<dyn SpiDevice>) {
        self.devices.push(dev);
    }

    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "RA SPI: {what} is not modelled"
        )))
    }

    fn r(&self, o: u64) -> u8 {
        self.regs[o as usize]
    }

    fn spcmd0(&self) -> u16 {
        u16::from_le_bytes([self.r(SPCMD0), self.r(SPCMD0 + 1)])
    }

    /// Frame length from SPCMD0.SPB: 0100-0111 = 8, 1111 = 16, 0010/0011 = 32.
    fn frame_bits(&self) -> Option<u32> {
        match (self.spcmd0() >> 8) & 0xF {
            0x4..=0x7 => Some(8),
            0xF => Some(16),
            0x2 | 0x3 => Some(32),
            _ => None,
        }
    }

    fn spsr(&self) -> u8 {
        let held = self.rx.borrow().len();
        let mut v = self.errors;
        if held > 0 {
            v |= SPSR_SPRF;
        }
        if held > 1 {
            v |= SPSR_IDLNF;
        }
        if held < DEPTH {
            v |= SPSR_SPTEF;
        }
        v
    }

    /// An SPDR load: pop the receive buffer, letting a held frame land.
    fn load_spdr(&self) -> u32 {
        match self.rx.borrow_mut().pop_front() {
            Some(v) => {
                self.last.set(v);
                v
            }
            None => self.last.get(),
        }
    }

    fn store_spdr(&mut self, v: u32) -> SimResult<()> {
        let spcr = self.r(SPCR);
        if spcr & SPCR_SPE == 0 {
            return Self::refuse("an SPDR store with SPCR.SPE = 0");
        }
        if self.r(SPSCR) != 0 {
            return Self::refuse("a command sequence (SPSCR != 0)");
        }
        let spdcr = self.r(SPDCR);
        if spdcr & SPDCR_SPFC != 0 {
            return Self::refuse("multi-frame transfers (SPDCR.SPFC)");
        }
        if spdcr & SPDCR_SPRDTD != 0 {
            return Self::refuse("transmit-buffer reads (SPDCR.SPRDTD)");
        }
        if self.r(SPPCR) & SPPCR_LOOPBACK != 0 {
            return Self::refuse("loopback (SPPCR.SPLP / SPLP2)");
        }
        if self.r(SPCR2) & SPCR2_SPPE != 0 {
            return Self::refuse("parity (SPCR2.SPPE)");
        }
        let Some(bits) = self.frame_bits() else {
            return Self::refuse("a frame length other than 8, 16 or 32 bits (SPCMD0.SPB)");
        };
        if self.rx.borrow().len() >= DEPTH {
            // SPTEF = 0: the transmit buffer is full and the store is lost.
            return Ok(());
        }
        let rx = self.exchange(v, bits);
        self.rx.borrow_mut().push_back(rx);
        if self.mosi.len() < 64 {
            self.mosi.push(v);
        }
        self.frames += 1;
        Ok(())
    }

    /// Clock one frame through every device; returns the received frame.
    fn exchange(&mut self, v: u32, bits: u32) -> u32 {
        let n = bits / 8;
        let swap = self.r(SPDCR2) & SPDCR2_BYSW != 0 && n > 1;
        let swap_frame = |x: u32| match n {
            2 => (x as u16).swap_bytes() as u32,
            4 => x.swap_bytes(),
            _ => x,
        };
        let mask = if bits == 32 {
            u32::MAX
        } else {
            (1u32 << bits) - 1
        };
        let tx = if swap { swap_frame(v & mask) } else { v & mask };
        let lsb_first = self.spcmd0() & (1 << 12) != 0;
        let mut rx = 0u32;
        for i in 0..n {
            // MSB first: the most significant byte leaves first, bit 7 first.
            // LSB first: the least significant byte leaves first, bit 0 first.
            let shift = if lsb_first { 8 * i } else { 8 * (n - 1 - i) };
            let byte = (tx >> shift) as u8;
            let wire = if lsb_first { byte.reverse_bits() } else { byte };
            let mut miso = 0u8;
            for dev in &mut self.devices {
                let resp = dev.transfer(wire);
                if resp != 0 {
                    miso = resp;
                }
            }
            let back = if lsb_first { miso.reverse_bits() } else { miso };
            rx |= (back as u32) << shift;
        }
        if swap {
            swap_frame(rx)
        } else {
            rx
        }
    }

    fn write_reg(&mut self, o: u64, v: u8) -> SimResult<()> {
        match o {
            SPCR => {
                if v & (SPCR_SPRIE | SPCR_SPTIE | SPCR_SPEIE) != 0 {
                    return Self::refuse("SPI interrupts (SPCR.SPRIE / SPTIE / SPEIE)");
                }
                if v & SPCR_TXMD != 0 {
                    return Self::refuse("transmit-only mode (SPCR.TXMD)");
                }
                if v & SPCR_SPE != 0 && v & SPCR_MSTR == 0 {
                    return Self::refuse("slave mode (SPCR.SPE without MSTR)");
                }
                self.regs[SPCR as usize] = v;
            }
            SPSR => self.errors &= v | !SPSR_ERRORS,
            SPDR..=0x07 => return Self::refuse("a byte store into the middle of SPDR"),
            SPSSR => {}
            SPCR2 if v & SPCR2_SPIIE != 0 => {
                return Self::refuse("the SPI idle interrupt (SPCR2.SPIIE)")
            }
            SPCR3 if v & SPCR3_CENDIE != 0 => {
                return Self::refuse("the communication-end interrupt (SPCR3.CENDIE)")
            }
            SPPR | 0x3F => {}
            0x01 | SPPCR | SPSCR | SPBR | SPDCR | 0x0C..=0x0F | SPCMD0..=0x1F | SPDCR2 | SPCR3 => {
                self.regs[o as usize] = v
            }
            _ => return Self::refuse("a store outside the register map"),
        }
        Ok(())
    }

    fn reg(&self, o: u64) -> Option<u8> {
        Some(match o {
            SPSR => self.spsr(),
            SPDR..=0x07 => (self.last.get() >> (8 * (o - SPDR))) as u8,
            // SPPR (buffer width / number / command number check) reads 0
            // and drops stores; neither FSP nor the core reads it.
            0x00..=0x02 | SPSCR..=0x21 | SPPR | 0x3F => self.regs[o as usize],
            _ => return None,
        })
    }
}

impl Peripheral for RaSpi {
    fn spi_attached_devices(&self) -> Option<&Vec<Box<dyn SpiDevice>>> {
        Some(&self.devices)
    }
    fn spi_attached_devices_mut(&mut self) -> Option<&mut Vec<Box<dyn SpiDevice>>> {
        Some(&mut self.devices)
    }
    fn read(&self, o: u64) -> SimResult<u8> {
        if o == SPDR {
            return Ok(self.load_spdr() as u8);
        }
        if (SPDR + 1..=0x07).contains(&o) {
            return Self::refuse("a byte load from the middle of SPDR");
        }
        match self.reg(o) {
            Some(v) => Ok(v),
            None => Self::refuse("a load outside the register map"),
        }
    }
    fn read_u16(&self, o: u64) -> SimResult<u16> {
        if o == SPDR {
            return Ok(self.load_spdr() as u16);
        }
        Ok(self.read(o)? as u16 | (self.read(o + 1)? as u16) << 8)
    }
    fn read_u32(&self, o: u64) -> SimResult<u32> {
        if o == SPDR {
            return Ok(self.load_spdr());
        }
        let mut v = 0u32;
        for i in 0..4 {
            v |= (self.read(o + i)? as u32) << (8 * i);
        }
        Ok(v)
    }
    fn peek(&self, o: u64) -> Option<u8> {
        if o == SPDR {
            return self
                .rx
                .borrow()
                .front()
                .map(|v| *v as u8)
                .or(Some(self.last.get() as u8));
        }
        self.reg(o)
    }
    fn write(&mut self, o: u64, v: u8) -> SimResult<()> {
        if o == SPDR {
            return self.store_spdr(v as u32);
        }
        self.write_reg(o, v)
    }
    fn write_u16(&mut self, o: u64, v: u16) -> SimResult<()> {
        if o == SPDR {
            return self.store_spdr(v as u32);
        }
        self.write_reg(o, v as u8)?;
        self.write_reg(o + 1, (v >> 8) as u8)
    }
    fn write_u32(&mut self, o: u64, v: u32) -> SimResult<()> {
        if o == SPDR {
            return self.store_spdr(v);
        }
        for i in 0..4 {
            self.write_reg(o + i, (v >> (8 * i)) as u8)?;
        }
        Ok(())
    }
    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }
    fn on_node_reset(&mut self) {
        let devices = std::mem::take(&mut self.devices);
        let (mosi, frames) = (std::mem::take(&mut self.mosi), self.frames);
        *self = Self::new();
        self.devices = devices;
        self.mosi = mosi;
        self.frames = frames;
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "ra_spi", "spcr": self.r(SPCR),
            "spsr": self.spsr(), "spcmd0": self.spcmd0(), "spdcr": self.r(SPDCR),
            "spdcr2": self.r(SPDCR2), "mosi": self.mosi, "frames": self.frames,
            "devices": self.devices.len()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A sensor that clocks out a fixed byte stream and logs MOSI.
    struct Stream {
        out: Vec<u8>,
        idx: usize,
        log: Arc<Mutex<Vec<u8>>>,
    }
    impl SpiDevice for Stream {
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.log.lock().unwrap().push(mosi);
            let v = self.out[self.idx % self.out.len()];
            self.idx += 1;
            v
        }
        fn cs_pin(&self) -> &str {
            "P103"
        }
    }

    fn spi(out: &[u8]) -> (RaSpi, Arc<Mutex<Vec<u8>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut s = RaSpi::new();
        s.push_device(Box::new(Stream {
            out: out.to_vec(),
            idx: 0,
            log: log.clone(),
        }));
        (s, log)
    }

    /// ArduinoCore-renesas configSpi(): SPCR = 0, MSTR, SPBYT, BYSW = 1 for
    /// MSB first, SPCMD0 = 8-bit frame, SPSR read + clear, SPE = 1.
    fn arduino_config(s: &mut RaSpi, lsb_first: bool) {
        s.write(SPCR, 0).unwrap();
        s.write(0x01, 0).unwrap();
        s.write(SPPCR, 0x20).unwrap();
        s.write(SPBR, 1).unwrap();
        s.write(SPDCR, 0x40).unwrap();
        s.write(SPDCR2, if lsb_first { 0 } else { 1 }).unwrap();
        s.write(SPCR2, 0x10).unwrap();
        s.write(SPCR, SPCR_MSTR).unwrap();
        let lsbf = if lsb_first { 1 << 12 } else { 0 };
        s.write_u16(SPCMD0, 0x0700 | lsbf).unwrap();
        s.read(SPSR).unwrap();
        s.write(SPSR, 0).unwrap();
        s.write(SPCR, SPCR_MSTR | SPCR_SPE).unwrap();
    }

    /// The core's byte transfer: SPDR_BY = b; spin on SPRF; read SPDR_BY.
    fn transfer(s: &mut RaSpi, b: u8) -> u8 {
        s.write(SPDR, b).unwrap();
        assert_ne!(s.read(SPSR).unwrap() & SPSR_SPRF, 0);
        let v = s.read(SPDR).unwrap();
        assert_eq!(s.read(SPSR).unwrap() & SPSR_SPRF, 0);
        v
    }

    #[test]
    fn reset_values() {
        let s = RaSpi::new();
        assert_eq!(s.read(SPSR).unwrap(), SPSR_SPTEF);
        assert_eq!(s.read(SPBR).unwrap(), 0xFF);
        assert_eq!(s.read_u16(SPCMD0 + 14).unwrap(), 0x070D);
    }

    #[test]
    fn byte_frames_read_a_max31855_word() {
        let (mut s, log) = spi(&[0x01, 0x90, 0x16, 0x00]);
        arduino_config(&mut s, false);
        let mut frame = 0u32;
        for _ in 0..4 {
            frame = (frame << 8) | transfer(&mut s, 0) as u32;
        }
        assert_eq!(frame, 0x0190_1600);
        assert_eq!(*log.lock().unwrap(), vec![0; 4]);
    }

    #[test]
    fn lsb_first_reverses_the_wire_both_ways() {
        let (mut s, log) = spi(&[0x80]);
        arduino_config(&mut s, true);
        assert_eq!(transfer(&mut s, 0x01), 0x01);
        assert_eq!(*log.lock().unwrap(), vec![0x80]);
    }

    /// The core's transfer(buf, 8): SPLW + SPB = 32, two words queued on
    /// SPTEF, then read-one-write-one. BYSW = 1 sends memory order.
    #[test]
    fn word_frames_with_byte_swap_keep_memory_order() {
        let out: Vec<u8> = (0xA0..0xA8).collect();
        let (mut s, log) = spi(&out);
        arduino_config(&mut s, false);
        s.write(SPCR, SPCR_MSTR).unwrap();
        s.write(SPDCR, 0x20).unwrap();
        s.write_u16(SPCMD0, 0x0200).unwrap();
        s.write(SPCR, SPCR_MSTR | SPCR_SPE).unwrap();
        let buf = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let words = [
            u32::from_le_bytes(buf[0..4].try_into().unwrap()),
            u32::from_le_bytes(buf[4..8].try_into().unwrap()),
        ];
        for w in words {
            assert_ne!(s.read(SPSR).unwrap() & SPSR_SPTEF, 0);
            s.write_u32(SPDR, w).unwrap();
        }
        let mut got = Vec::new();
        for _ in 0..2 {
            assert_ne!(s.read(SPSR).unwrap() & SPSR_SPRF, 0);
            got.extend_from_slice(&s.read_u32(SPDR).unwrap().to_le_bytes());
        }
        assert_eq!(*log.lock().unwrap(), buf.to_vec());
        assert_eq!(got, out);
        assert_eq!(s.read(SPSR).unwrap(), SPSR_SPTEF);
    }

    #[test]
    fn a_full_pipeline_clears_sptef_and_drops_the_store() {
        let (mut s, _) = spi(&[0x5A]);
        arduino_config(&mut s, false);
        for _ in 0..DEPTH {
            s.write(SPDR, 0).unwrap();
        }
        assert_eq!(s.read(SPSR).unwrap() & SPSR_SPTEF, 0);
        s.write(SPDR, 0).unwrap();
        assert_eq!(s.frames, DEPTH as u64);
    }

    #[test]
    fn refusals() {
        let mut s = RaSpi::new();
        assert!(s.write(SPCR, SPCR_SPE).is_err());
        assert!(s.write(SPCR, SPCR_MSTR | SPCR_SPRIE).is_err());
        assert!(s.write(SPCR, SPCR_MSTR | SPCR_TXMD).is_err());
        assert!(s.write(SPDR, 0).is_err());
        s.write(SPCR, SPCR_MSTR | SPCR_SPE).unwrap();
        s.write_u16(SPCMD0, 0x0900).unwrap();
        assert!(s.write(SPDR, 0).is_err());
    }
}
