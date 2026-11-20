// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Texas Instruments MSP430x2xx Universal Serial Interface (family user's
//! guide SLAU144, chapter 14), as on the MSP430G2x52/G2x12 and F20x2/F20x3.
//!
//! [`Msp430Usi`] (`msp430_usi`): `USICTL0` at offset 0, `USICTL1` 1,
//! `USICKCTL` 2, `USICNT` 3, `USISRL` 4, `USISRH` 5 (0x0078 on every x2xx
//! part). Word access reads and writes the byte pairs (Table 14-2).
//!
//! Shift engine (14.2):
//!
//! * Each shift clock samples one input bit into `USISR` and decrements
//!   `USICNTx`. `USICNTx` reaching 0, by counting or by a write of 0, sets
//!   `USIIFG`; a write of 1 to 31 clears `USIIFG` and `USISTP` unless
//!   `USIIFGCC` is set. `USI16B` shifts all of `USISR`, else only `USISRL`;
//!   `USILSB` shifts right (LSB first) instead of left.
//! * The output latch takes the next output bit (the MSB, or the LSB with
//!   `USILSB`) on the falling edge before each sample; `USIGE` makes it
//!   transparent.
//! * `USICLK` runs in master mode (`USIMST`) while `USIIFG` = 0 and
//!   `USICNTx` > 0. One bit takes `2^USIDIVx` source clocks. `USISSELx` 2
//!   and 3 (SMCLK) count one CPU cycle per clock, as Timer_A does; 1 (ACLK)
//!   counts `aclk_hz` against `cpu_hz` (one CPU cycle per clock with no
//!   `cpu_hz`); 4 shifts one bit on each 0 -> 1 write of `USISWCLK`.
//! * `USISWRST` stops the shift clock and holds `USIIFG` at 1 and
//!   `USISTTIFG`, `USISTP` and `USIAL` at 0 (14.2.1). `USISR` keeps its
//!   value over a reset.
//!
//! SPI (`USII2C` = 0, 14.2.3): with `USIPE5` (SCLK) set, attached SPI
//! devices exchange whole bytes. At each 8-bit boundary of a transfer that
//! has at least 8 bits left, the next 8 output bits go to every device on
//! the bus as one byte (wire order, first bit as the MSB) and the first
//! device's answer is shifted in bit by bit through SDI (`USIPE7`). A
//! device is on the bus while its chip-select GPIO is low or when its
//! chip select does not resolve to a GPIO output. Bits outside such a
//! group, or with no device, sample 0.
//!
//! I2C (`USII2C` = 1, 14.2.4): the module is the master (`USIMST`) of an
//! open-drain bus with the attached I2C devices. SDA is low when the USI
//! drives a 0 (`USIOE` with a 0 in the latch) or a device does. A write
//! with the latch transparent (`USIGE`, as the START and STOP sequences of
//! 14.2.4.5 and 14.2.4.6 do) that lowers SDA between clocks is a START
//! (`USISTTIFG`, and `USISCLREL` clears), one that raises it a STOP
//! (`USISTP`). After a START the devices see the
//! address byte; the addressed device acknowledges it, takes written
//! bytes and acknowledges each, or drives read bytes and stops after the
//! master's NACK. A master that sends a 1 while SDA reads 0 loses
//! arbitration: `USIAL` sets and `USIOE` clears (14.2.4.8).
//!
//! Interrupts: `USIIFG` (`irq_counter_source`) and `USISTTIFG`
//! (`irq_start_source`) share the USI vector (`irq_line`), so accepting it
//! clears neither flag; `USIIE` and `USISTTIE` are the enables.
//!
//! Limits:
//!
//! * Slave mode has no external clock: the shift register never moves in
//!   SPI or I2C slave mode. `USISSELx` 0 (SCLK) and 5 to 7 (Timer_A
//!   CCRx outputs) give no clock.
//! * The SCLK, SDO/SDA and SCL pads are not driven from this block; the
//!   port pins keep their GPIO levels. `USICKPH` and `USICKPL` are
//!   storage: the byte exchange does not depend on the SPI mode.
//! * SDA released or driven by a write with `USIGE` clear is neither a
//!   START nor a STOP: the TI USI I2C code clears `USIOE` before each
//!   acknowledge bit, and the devices must not see that edge.
//! * No clock stretching by devices, no multi-master bus and no SCL hold:
//!   `USISCLREL` is storage apart from its clear on a START.

use crate::peripherals::device::{I2cDevice, I2cSlaveHost, SpiDevice};
use crate::peripherals::msp430::{Msp430Irq, SOURCES};
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};
use std::sync::Arc;

const CTL0: u64 = 0;
const CTL1: u64 = 1;
const CKCTL: u64 = 2;
const CNT: u64 = 3;
const SRL: u64 = 4;
const SRH: u64 = 5;

// USICTL0.
const USIPE7: u8 = 0x80;
const USIPE5: u8 = 0x20;
const USILSB: u8 = 0x10;
const USIMST: u8 = 0x08;
const USIGE: u8 = 0x04;
const USIOE: u8 = 0x02;
const USISWRST: u8 = 0x01;
// USICTL1.
const USII2C: u8 = 0x40;
const USISTTIE: u8 = 0x20;
const USIIE: u8 = 0x10;
const USIAL: u8 = 0x08;
const USISTP: u8 = 0x04;
const USISTTIFG: u8 = 0x02;
const USIIFG: u8 = 0x01;
/// `USICTL1` bits held here; the two interrupt flags live in the IRQ block.
const CTL1_STORED: u8 = !(USISTTIFG | USIIFG);
// USICKCTL.
const USISWCLK: u8 = 0x01;
// USICNT.
const USISCLREL: u8 = 0x80;
const USI16B: u8 = 0x40;
const USIIFGCC: u8 = 0x20;
const USICNTX: u8 = 0x1F;

/// The interrupt sources and the request line of the USI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsiRequest {
    /// `USIIFG`.
    pub counter_source: u32,
    /// `USISTTIFG`.
    pub start_source: u32,
    pub line: u32,
}

/// The descriptor's clock facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsiConfig {
    pub aclk_hz: u64,
    /// CPU cycles per second; 0 counts every clock as one CPU cycle.
    pub cpu_hz: u64,
}

/// The I2C bus as the attached devices see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum I2cPhase {
    /// No transfer for the devices: before a START, after a STOP, or a
    /// transfer nobody acknowledged.
    Idle,
    Address {
        byte: u8,
        bits: u8,
    },
    AddressAck {
        ack: bool,
        read: bool,
    },
    Write {
        byte: u8,
        bits: u8,
    },
    WriteAck,
    Read {
        byte: u8,
        bits: u8,
    },
    ReadAck,
}

pub struct Msp430Usi {
    request: Option<UsiRequest>,
    config: UsiConfig,
    irq: Arc<Msp430Irq>,
    ctl0: u8,
    /// `USICTL1` without the two interrupt flags.
    ctl1: u8,
    ckctl: u8,
    cnt: u8,
    sr: u16,
    /// The flags while no interrupt source is routed.
    ifg: bool,
    sttifg: bool,
    /// The output latch.
    latch: bool,
    /// CPU cycles to the next shift clock while `USICLK` runs.
    next_bit: Option<u64>,
    /// Bits shifted since `USICNT` was written.
    shifted: u32,
    /// The answer of the SPI devices to the current byte and the bits of it
    /// already shifted in.
    miso: Option<(u8, u8)>,
    spi: Vec<Box<dyn SpiDevice>>,
    /// Chip-select state per SPI device: `Some(true)` selected, `Some(false)`
    /// released, `None` when the pin does not resolve.
    cs: Vec<Option<bool>>,
    slaves: Vec<Box<dyn I2cDevice>>,
    phase: I2cPhase,
    /// The addressed I2C device.
    selected: Option<usize>,
    /// SDA as the devices drive it.
    device_sda: bool,
    /// The SDA level after the last change, for START and STOP detection.
    sda: bool,
}

impl std::fmt::Debug for Msp430Usi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Msp430Usi")
            .field("ctl0", &self.ctl0)
            .field("ctl1", &self.ctl1)
            .field("ckctl", &self.ckctl)
            .field("cnt", &self.cnt)
            .field("sr", &self.sr)
            .field("spi_devices", &self.spi.len())
            .field("i2c_devices", &self.slaves.len())
            .field("phase", &self.phase)
            .finish()
    }
}

impl Msp430Usi {
    pub fn new(request: Option<UsiRequest>, config: UsiConfig) -> Self {
        let mut usi = Self {
            request: request
                .filter(|r| r.counter_source < SOURCES && r.start_source < SOURCES && r.line < 64),
            config,
            irq: Msp430Irq::new(),
            ctl0: 0,
            ctl1: 0,
            ckctl: 0,
            cnt: 0,
            sr: 0,
            ifg: false,
            sttifg: false,
            latch: true,
            next_bit: None,
            shifted: 0,
            miso: None,
            spi: Vec::new(),
            cs: Vec::new(),
            slaves: Vec::new(),
            phase: I2cPhase::Idle,
            selected: None,
            device_sda: true,
            sda: true,
        };
        usi.reset_registers();
        usi
    }

    fn reset_registers(&mut self) {
        self.ctl0 = USISWRST;
        self.ctl1 = 0;
        self.ckctl = 0;
        self.cnt = 0;
        self.latch = true;
        self.next_bit = None;
        self.shifted = 0;
        self.miso = None;
        self.set_ifg(true);
        self.set_sttifg(false);
        self.sync_enable();
        self.end_i2c_transfer();
        self.sda = self.sda_level();
    }

    fn ifg(&self) -> bool {
        match self.request {
            Some(r) => self.irq.flag(r.counter_source),
            None => self.ifg,
        }
    }

    fn set_ifg(&mut self, on: bool) {
        match self.request {
            Some(r) => self.irq.set_flag(r.counter_source, on),
            None => self.ifg = on,
        }
    }

    fn sttifg(&self) -> bool {
        match self.request {
            Some(r) => self.irq.flag(r.start_source),
            None => self.sttifg,
        }
    }

    fn set_sttifg(&mut self, on: bool) {
        match self.request {
            Some(r) => self.irq.set_flag(r.start_source, on),
            None => self.sttifg = on,
        }
    }

    fn sync_enable(&self) {
        if let Some(r) = self.request {
            self.irq
                .set_enable(r.counter_source, self.ctl1 & USIIE != 0);
            self.irq
                .set_enable(r.start_source, self.ctl1 & USISTTIE != 0);
        }
    }

    fn in_reset(&self) -> bool {
        self.ctl0 & USISWRST != 0
    }

    fn i2c(&self) -> bool {
        self.ctl1 & USII2C != 0
    }

    fn count(&self) -> u8 {
        self.cnt & USICNTX
    }

    fn source(&self) -> u8 {
        (self.ckctl >> 2) & 7
    }

    /// `USICLK` runs from a free-running source.
    fn clocking(&self) -> bool {
        !self.in_reset()
            && self.ctl0 & USIMST != 0
            && !self.ifg()
            && self.count() > 0
            && (1..=3).contains(&self.source())
    }

    /// CPU cycles per shift clock.
    fn bit_cycles(&self) -> u64 {
        let clocks = 1u64 << (self.ckctl >> 5);
        match self.source() {
            1 if self.config.aclk_hz != 0 && self.config.cpu_hz != 0 => (clocks
                * self.config.cpu_hz)
                .div_ceil(self.config.aclk_hz)
                .max(1),
            _ => clocks,
        }
    }

    fn reschedule(&mut self) {
        if !self.clocking() {
            self.next_bit = None;
        } else if self.next_bit.is_none() {
            self.next_bit = Some(self.bit_cycles());
        }
    }

    fn wide(&self) -> bool {
        self.cnt & USI16B != 0
    }

    fn lsb_first(&self) -> bool {
        self.ctl0 & USILSB != 0
    }

    /// The bit the next shift clock puts out.
    fn out_bit(&self) -> bool {
        if self.lsb_first() {
            self.sr & 1 != 0
        } else if self.wide() {
            self.sr & 0x8000 != 0
        } else {
            self.sr & 0x80 != 0
        }
    }

    fn shift_in(&mut self, bit: bool) {
        let bit = u16::from(bit);
        if self.wide() {
            self.sr = if self.lsb_first() {
                self.sr >> 1 | bit << 15
            } else {
                self.sr << 1 | bit
            };
        } else {
            let low = self.sr & 0xFF;
            let low = if self.lsb_first() {
                low >> 1 | bit << 7
            } else {
                (low << 1 | bit) & 0xFF
            };
            self.sr = (self.sr & 0xFF00) | low;
        }
    }

    /// The next 8 output bits in wire order, the first as the MSB.
    fn next_out_byte(&self) -> u8 {
        if self.lsb_first() {
            (self.sr as u8).reverse_bits()
        } else if self.wide() {
            (self.sr >> 8) as u8
        } else {
            self.sr as u8
        }
    }

    /// One shift clock: the falling edge loads the output latch, the
    /// rising edge samples the input.
    fn clock_bit(&mut self) {
        if self.ctl0 & USIGE == 0 {
            self.latch = self.out_bit();
        }
        let input = if self.i2c() {
            self.i2c_bit()
        } else {
            self.spi_bit()
        };
        self.shift_in(input);
        self.shifted += 1;
        self.cnt = (self.cnt & !USICNTX) | (self.count() - 1);
        if self.count() == 0 {
            self.set_ifg(true);
        }
        if self.ctl0 & USIGE != 0 {
            self.latch = self.out_bit();
        }
        self.sda = self.sda_level();
    }

    // ── SPI ──────────────────────────────────────────────────────────────

    fn spi_on_bus(&self, index: usize) -> bool {
        self.cs.get(index).copied().flatten() != Some(false)
    }

    fn spi_bit(&mut self) -> bool {
        if self.shifted % 8 == 0 {
            self.miso = None;
            if self.count() >= 8 && self.ctl0 & USIPE5 != 0 {
                let mosi = self.next_out_byte();
                let mut answer = None;
                for index in 0..self.spi.len() {
                    if self.spi_on_bus(index) {
                        let byte = self.spi[index].transfer(mosi);
                        answer.get_or_insert(byte);
                    }
                }
                self.miso = answer.map(|byte| (byte, 0));
            }
        }
        let Some((byte, bits)) = self.miso else {
            return false;
        };
        self.miso = Some((byte, bits + 1));
        self.ctl0 & USIPE7 != 0 && byte >> (7 - bits) & 1 != 0
    }

    // ── I2C ──────────────────────────────────────────────────────────────

    /// SDA as the USI drives it: open drain, low for a 0 in the latch.
    fn master_sda(&self) -> bool {
        self.ctl0 & USIOE == 0 || self.latch
    }

    fn sda_level(&self) -> bool {
        self.master_sda() && self.device_sda
    }

    /// What the devices drive on SDA for the next bit.
    fn device_bit(&self) -> bool {
        match self.phase {
            I2cPhase::AddressAck { ack, .. } => !ack,
            I2cPhase::WriteAck => self.selected.is_none(),
            I2cPhase::Read { byte, bits } => byte >> (7 - bits) & 1 != 0,
            _ => true,
        }
    }

    fn i2c_bit(&mut self) -> bool {
        self.device_sda = self.device_bit();
        let sda = self.sda_level();
        if self.ctl0 & (USIMST | USIOE) == (USIMST | USIOE) && self.latch && !sda {
            self.ctl1 |= USIAL;
            self.ctl0 &= !USIOE;
        }
        self.advance_i2c(sda);
        sda
    }

    /// The devices sample `sda` on the rising edge of SCL.
    fn advance_i2c(&mut self, sda: bool) {
        let bit = u8::from(sda);
        self.phase = match self.phase {
            I2cPhase::Idle => I2cPhase::Idle,
            I2cPhase::Address { byte, bits } => {
                let byte = byte << 1 | bit;
                if bits < 7 {
                    I2cPhase::Address {
                        byte,
                        bits: bits + 1,
                    }
                } else {
                    let address = byte >> 1;
                    let found = self.slaves.iter().position(|s| s.claims_address(address));
                    if let Some(old) = self.selected.filter(|&old| Some(old) != found) {
                        self.slaves[old].stop();
                    }
                    self.selected = found;
                    if let Some(index) = found {
                        self.slaves[index].select_address(address);
                        self.slaves[index].start();
                    }
                    I2cPhase::AddressAck {
                        ack: found.is_some(),
                        read: byte & 1 != 0,
                    }
                }
            }
            I2cPhase::AddressAck { ack: false, .. } => I2cPhase::Idle,
            I2cPhase::AddressAck { read: true, .. } => self.next_read(),
            I2cPhase::AddressAck { read: false, .. } | I2cPhase::WriteAck => {
                I2cPhase::Write { byte: 0, bits: 0 }
            }
            I2cPhase::Write { byte, bits } => {
                let byte = byte << 1 | bit;
                if bits < 7 {
                    I2cPhase::Write {
                        byte,
                        bits: bits + 1,
                    }
                } else {
                    if let Some(index) = self.selected {
                        self.slaves[index].write(byte);
                    }
                    I2cPhase::WriteAck
                }
            }
            I2cPhase::Read { byte, bits } if bits < 7 => I2cPhase::Read {
                byte,
                bits: bits + 1,
            },
            I2cPhase::Read { .. } => I2cPhase::ReadAck,
            // The master's ACK asks for another byte, its NACK ends the read.
            I2cPhase::ReadAck if !sda => self.next_read(),
            I2cPhase::ReadAck => I2cPhase::Idle,
        };
    }

    fn next_read(&mut self) -> I2cPhase {
        let byte = match self.selected {
            Some(index) => self.slaves[index].read(),
            None => 0xFF,
        };
        I2cPhase::Read { byte, bits: 0 }
    }

    fn end_i2c_transfer(&mut self) {
        if let Some(index) = self.selected.take() {
            self.slaves[index].stop();
        }
        self.phase = I2cPhase::Idle;
        self.device_sda = true;
    }

    /// SDA moved between clocks, with SCL high, through the transparent
    /// latch: a falling SDA is a START, a rising one a STOP.
    fn sda_changed(&mut self) {
        let sda = self.sda_level();
        if sda == self.sda {
            return;
        }
        self.sda = sda;
        if !self.i2c() || self.in_reset() {
            return;
        }
        if sda {
            self.ctl1 |= USISTP;
            self.end_i2c_transfer();
        } else {
            self.set_sttifg(true);
            self.cnt &= !USISCLREL;
            self.phase = I2cPhase::Address { byte: 0, bits: 0 };
            self.device_sda = true;
        }
    }

    // ── Registers ────────────────────────────────────────────────────────

    fn byte(&self, offset: u64) -> u8 {
        match offset {
            CTL0 => self.ctl0,
            CTL1 => {
                self.ctl1
                    | if self.sttifg() { USISTTIFG } else { 0 }
                    | if self.ifg() { USIIFG } else { 0 }
            }
            CKCTL => self.ckctl,
            CNT => self.cnt,
            SRL => self.sr as u8,
            SRH => (self.sr >> 8) as u8,
            _ => 0,
        }
    }

    fn set_byte(&mut self, offset: u64, value: u8) {
        let transparent = self.ctl0 & USIGE != 0;
        match offset {
            CTL0 => {
                self.ctl0 = value;
                if self.in_reset() {
                    self.hold_flags();
                }
            }
            CTL1 => {
                self.ctl1 = value & CTL1_STORED;
                self.set_ifg(value & USIIFG != 0);
                self.set_sttifg(value & USISTTIFG != 0);
                if self.in_reset() {
                    self.hold_flags();
                }
                self.sync_enable();
            }
            CKCTL => {
                let rising = value & !self.ckctl & USISWCLK != 0;
                self.ckctl = value;
                let manual = !self.in_reset()
                    && self.ctl0 & USIMST != 0
                    && self.source() == 4
                    && !self.ifg()
                    && self.count() > 0;
                if rising && manual {
                    self.clock_bit();
                }
            }
            CNT => {
                self.cnt = value;
                self.shifted = 0;
                self.miso = None;
                if !self.in_reset() {
                    if self.count() == 0 {
                        self.set_ifg(true);
                    } else if value & USIIFGCC == 0 {
                        self.set_ifg(false);
                        self.ctl1 &= !USISTP;
                    }
                }
            }
            SRL => self.sr = (self.sr & 0xFF00) | u16::from(value),
            SRH => self.sr = (self.sr & 0x00FF) | u16::from(value) << 8,
            _ => return,
        }
        if self.ctl0 & USIGE != 0 {
            self.latch = self.out_bit();
        }
        if transparent || self.ctl0 & USIGE != 0 {
            self.sda_changed();
        } else {
            self.sda = self.sda_level();
        }
        self.reschedule();
    }

    fn hold_flags(&mut self) {
        self.set_ifg(true);
        self.set_sttifg(false);
        self.ctl1 &= !(USISTP | USIAL);
    }
}

impl Peripheral for Msp430Usi {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.byte(offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.set_byte(offset, value);
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let offset = offset & !1;
        Ok(u16::from(self.byte(offset)) | u16::from(self.byte(offset + 1)) << 8)
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        let offset = offset & !1;
        self.set_byte(offset, value as u8);
        self.set_byte(offset + 1, (value >> 8) as u8);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some(self.byte(offset))
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn legacy_tick_active(&self) -> bool {
        self.next_bit.is_some()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, mut cycles: u64) -> PeripheralTickResult {
        while let Some(left) = self.next_bit {
            if cycles < left {
                self.next_bit = Some(left - cycles);
                break;
            }
            cycles -= left;
            self.next_bit = None;
            self.clock_bit();
            self.reschedule();
        }
        PeripheralTickResult::default()
    }

    fn attach_msp430_irq(&mut self, irq: &Arc<Msp430Irq>) {
        if let Some(r) = self.request {
            irq.route(r.counter_source, r.line, false);
            irq.route(r.start_source, r.line, false);
        }
        self.irq = irq.clone();
        self.on_system_reset();
    }

    fn on_system_reset(&mut self) {
        self.reset_registers();
    }

    fn spi_attached_devices(&self) -> Option<&Vec<Box<dyn SpiDevice>>> {
        Some(&self.spi)
    }

    fn spi_attached_devices_mut(&mut self) -> Option<&mut Vec<Box<dyn SpiDevice>>> {
        Some(&mut self.spi)
    }

    fn gpio_chip_select_pins(&self) -> Vec<String> {
        self.spi
            .iter()
            .map(|d| d.cs_pin().to_string())
            .filter(|pin| !pin.is_empty())
            .collect()
    }

    fn sync_gpio_chip_selects(&mut self, bus: &dyn Bus) {
        self.cs.resize(self.spi.len(), None);
        for (index, device) in self.spi.iter_mut().enumerate() {
            let pin = device.cs_pin();
            if pin.is_empty() {
                continue;
            }
            let next = bus
                .read_gpio_output_by_label(pin)
                .map(|high| device.cs_selected_at(high));
            let was = self.cs[index] == Some(true);
            let now = next == Some(true);
            if now && !was {
                device.cs_select();
            } else if was && !now {
                device.cs_release();
            }
            self.cs[index] = next;
        }
    }

    fn i2c_slave_host(&mut self) -> Option<&mut dyn I2cSlaveHost> {
        Some(self)
    }

    fn drives_central_i2c_time(&self) -> bool {
        true
    }

    fn advance_attached_i2c_us(&mut self, us: u64) {
        for slave in &mut self.slaves {
            slave.advance_time_us(us);
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "usictl0": self.byte(CTL0),
            "usictl1": self.byte(CTL1),
            "usickctl": self.ckctl,
            "usicnt": self.cnt,
            "usisr": self.sr,
            "spi_devices": self.spi.len(),
            "i2c_devices": self.slaves.len(),
        })
    }
}

impl I2cSlaveHost for Msp430Usi {
    fn attach_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.slaves.push(device);
    }
}

#[cfg(test)]
#[path = "msp430_usi_tests.rs"]
mod tests;
