// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! AVR8 CPU (ATmega328P-class) — Harvard flash + byte data space.
//!
//! Public PC is a **byte** address (ELF/DWARF). Fetch uses word index
//! `pc_byte / 2`. Data space is separate from program flash.

use crate::peripherals::i2c::I2cDevice;
use crate::peripherals::spi::SpiDevice;
use crate::snapshot::{AvrCpuSnapshot, CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
use std::sync::{Arc, Mutex};

/// Master TWI phase after a completed bus event (status already latched).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum TwiPhase {
    #[default]
    Idle,
    /// START/REP_START sent; next TWDR is SLA+R/W.
    Started,
    /// Master transmitter (write).
    Mt,
    /// Master receiver (read).
    Mr,
}

pub const FLASH_SIZE: usize = 32 * 1024;
pub const RAMEND: u16 = 0x08FF;
pub const SRAM_START: u16 = 0x0100;
pub const AVR_DATA_VMA_BIAS: u64 = 0x0080_0000;
pub const AVR_EEPROM_VMA_BIAS: u64 = 0x0081_0000;

pub fn strip_avr_data_bias(vma: u64) -> Option<u64> {
    if (AVR_EEPROM_VMA_BIAS..AVR_EEPROM_VMA_BIAS + 0x1_0000).contains(&vma) {
        return Some(vma - AVR_EEPROM_VMA_BIAS);
    }
    if (AVR_DATA_VMA_BIAS..AVR_DATA_VMA_BIAS + 0x1_0000).contains(&vma) {
        return Some(vma - AVR_DATA_VMA_BIAS);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvrLoadSpace {
    Flash,
    Data,
    Eeprom,
}

pub fn classify_avr_vma(vma: u64) -> (AvrLoadSpace, u64) {
    if let Some(d) = strip_avr_data_bias(vma) {
        if vma >= AVR_EEPROM_VMA_BIAS {
            (AvrLoadSpace::Eeprom, d)
        } else {
            (AvrLoadSpace::Data, d)
        }
    } else {
        (AvrLoadSpace::Flash, vma)
    }
}

pub struct Avr {
    pub r: [u8; 32],
    pub pc: u32,
    pub sp: u16,
    pub sreg: u8,
    pub flash: Vec<u8>,
    pub sram: Vec<u8>,
    pub io: [u8; 0xE0],
    pub pending_irq: u64,
    pub cycles: u64,
    pub tcnt0: u8,
    pub tccr0a: u8,
    pub tccr0b: u8,
    pub timsk0: u8,
    pub tifr0: u8,
    pub ocr0a: u8,
    pub ocr0b: u8,
    pub t0_prescale_acc: u32,
    pub serial_tx: Vec<u8>,
    /// Optional live sink for MachineTrait UART capture.
    pub serial_sink: Option<Arc<Mutex<Vec<u8>>>>,
    pub ucsr0a: u8,
    pub ucsr0b: u8,
    pub ucsr0c: u8,
    pub ubrr0: u16,
    /// SPI control / status / data (ATmega328P data-space 0x4C..0x4E).
    pub spcr: u8,
    pub spsr: u8,
    pub spdr: u8,
    /// SPI slaves (e.g. matrix MAX31855) attached for L4.
    ///
    /// Not `Clone`/`Debug`: trait objects. Kits land here after
    /// [`crate::bus::SystemBus::take_spi_devices`] moves them off the bus
    /// parking SPI controller (AVR has no MMIO SPI model).
    pub spi_devices: Vec<Box<dyn SpiDevice>>,
    /// TWI (I²C) — TWBR/TWSR/TWAR/TWDR/TWCR (data-space 0xB8..0xBC).
    pub twbr: u8,
    pub twsr: u8,
    pub twar: u8,
    pub twdr: u8,
    pub twcr: u8,
    twi_phase: TwiPhase,
    twi_slave: Option<usize>,
    /// I²C slaves (e.g. matrix INA219) attached for L3.
    pub i2c_slaves: Vec<Box<dyn I2cDevice>>,
    /// ADC (ADMUX/ADCSRA/ADCL/ADCH) — matrix L5 analogRead.
    pub admux: u8,
    pub adcsra: u8,
    pub adcl: u8,
    pub adch: u8,
}

impl std::fmt::Debug for Avr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Avr")
            .field("pc", &self.pc)
            .field("sp", &self.sp)
            .field("sreg", &self.sreg)
            .field("cycles", &self.cycles)
            .field("spcr", &self.spcr)
            .field("spsr", &self.spsr)
            .field("spdr", &self.spdr)
            .field("spi_devices", &self.spi_devices.len())
            .field("twcr", &self.twcr)
            .field("twsr", &self.twsr)
            .field("i2c_slaves", &self.i2c_slaves.len())
            .finish_non_exhaustive()
    }
}

pub const VEC_TIMER0_OVF: u32 = 17; // datasheet 1-based; @0x40 = __vector_16
/// TWI_vect is `_VECTOR(24)` → PC 0x60; pending bit uses vec=25 (`(vec-1)*4`).
pub const VEC_TWI: u32 = 25;
pub const UCSRA_UDRE: u8 = 1 << 5;
pub const UCSRA_TXC: u8 = 1 << 6;
pub const TIMSK_TOIE0: u8 = 1 << 0;
pub const TIFR_TOV0: u8 = 1 << 0;

// TWCR bits (ATmega328P datasheet).
const TWINT: u8 = 1 << 7;
const TWEA: u8 = 1 << 6;
const TWSTA: u8 = 1 << 5;
const TWSTO: u8 = 1 << 4;
const TWWC: u8 = 1 << 3;
const TWEN: u8 = 1 << 2;
const TWIE: u8 = 1 << 0;

// TW_STATUS codes (TWSR & 0xF8).
const TW_START: u8 = 0x08;
const TW_REP_START: u8 = 0x10;
const TW_MT_SLA_ACK: u8 = 0x18;
const TW_MT_SLA_NACK: u8 = 0x20;
const TW_MT_DATA_ACK: u8 = 0x28;
const TW_MT_DATA_NACK: u8 = 0x30;
const TW_MR_SLA_ACK: u8 = 0x40;
const TW_MR_SLA_NACK: u8 = 0x48;
const TW_MR_DATA_ACK: u8 = 0x50;
const TW_MR_DATA_NACK: u8 = 0x58;

impl Default for Avr {
    fn default() -> Self {
        Self::new()
    }
}

impl Avr {
    pub fn new() -> Self {
        Self {
            r: [0; 32],
            pc: 0,
            sp: RAMEND,
            sreg: 0,
            flash: vec![0xFF; FLASH_SIZE],
            sram: vec![0; (RAMEND as usize + 1) - SRAM_START as usize],
            io: [0; 0xE0],
            pending_irq: 0,
            cycles: 0,
            tcnt0: 0,
            tccr0a: 0,
            tccr0b: 0,
            timsk0: 0,
            tifr0: 0,
            ocr0a: 0,
            ocr0b: 0,
            t0_prescale_acc: 0,
            serial_tx: Vec::new(),
            serial_sink: None,
            ucsr0a: UCSRA_UDRE,
            ucsr0b: 0,
            ucsr0c: 0,
            ubrr0: 0,
            spcr: 0,
            spsr: 0,
            spdr: 0,
            spi_devices: Vec::new(),
            twbr: 0,
            twsr: 0xF8, // no-info status with prescaler 0
            twar: 0,
            twdr: 0xFF,
            twcr: 0,
            twi_phase: TwiPhase::Idle,
            twi_slave: None,
            i2c_slaves: Vec::new(),
            admux: 0,
            adcsra: 0,
            adcl: 0,
            adch: 0,
        }
    }

    pub fn load_flash(&mut self, addr: u32, data: &[u8]) {
        let start = addr as usize;
        let end = (start + data.len()).min(self.flash.len());
        if start < end {
            let n = end - start;
            self.flash[start..end].copy_from_slice(&data[..n]);
        }
    }

    pub fn load_words(&mut self, addr: u32, words: &[u16]) {
        for (i, w) in words.iter().enumerate() {
            let b = addr as usize + i * 2;
            if b + 1 < self.flash.len() {
                self.flash[b] = (*w & 0xFF) as u8;
                self.flash[b + 1] = (*w >> 8) as u8;
            }
        }
    }

    #[inline]
    fn flag_i(&self) -> bool {
        self.sreg & 0x80 != 0
    }

    #[inline]
    fn set_flag_i(&mut self, on: bool) {
        if on {
            self.sreg |= 0x80;
        } else {
            self.sreg &= !0x80;
        }
    }

    #[inline]
    fn set_z(&mut self, v: u8) {
        if v == 0 {
            self.sreg |= 0x02;
        } else {
            self.sreg &= !0x02;
        }
    }

    #[inline]
    fn set_n(&mut self, v: u8) {
        if v & 0x80 != 0 {
            self.sreg |= 0x04;
        } else {
            self.sreg &= !0x04;
        }
    }

    #[inline]
    fn set_c(&mut self, on: bool) {
        if on {
            self.sreg |= 0x01;
        } else {
            self.sreg &= !0x01;
        }
    }

    #[inline]
    fn set_v(&mut self, on: bool) {
        if on {
            self.sreg |= 0x08;
        } else {
            self.sreg &= !0x08;
        }
    }

    #[inline]
    fn update_s_from_nv(&mut self) {
        let n = (self.sreg >> 2) & 1;
        let v = (self.sreg >> 3) & 1;
        if n ^ v != 0 {
            self.sreg |= 0x10;
        } else {
            self.sreg &= !0x10;
        }
    }

    fn fetch_word(&self, pc_byte: u32) -> SimResult<u16> {
        if pc_byte % 2 != 0 {
            return Err(SimulationError::DecodeError(pc_byte as u64));
        }
        let i = pc_byte as usize;
        if i + 1 >= self.flash.len() {
            return Err(SimulationError::MemoryViolation(pc_byte as u64));
        }
        Ok(u16::from_le_bytes([self.flash[i], self.flash[i + 1]]))
    }

    fn data_read(&self, addr: u16, _bus: &dyn Bus) -> SimResult<u8> {
        match addr {
            0x0000..=0x001F => Ok(self.r[addr as usize]),
            0x005D => Ok((self.sp & 0xFF) as u8),
            0x005E => Ok((self.sp >> 8) as u8),
            0x005F => Ok(self.sreg),
            0x0035 => Ok(self.tifr0),
            0x0044 => Ok(self.tccr0a),
            0x0045 => Ok(self.tccr0b),
            0x0046 => Ok(self.tcnt0),
            0x0047 => Ok(self.ocr0a),
            0x0048 => Ok(self.ocr0b),
            0x006E => Ok(self.timsk0),
            0x00C0 => Ok(self.ucsr0a | UCSRA_UDRE),
            0x00C1 => Ok(self.ucsr0b),
            0x00C2 => Ok(self.ucsr0c),
            0x00C4 => Ok((self.ubrr0 & 0xFF) as u8),
            0x00C5 => Ok((self.ubrr0 >> 8) as u8),
            0x00C6 => Ok(0),
            // SPI: SPCR/SPSR/SPDR (ATmega328P data space)
            0x004C => Ok(self.spcr),
            0x004D => Ok(self.spsr),
            0x004E => Ok(self.spdr),
            // TWI: TWBR/TWSR/TWAR/TWDR/TWCR
            0x00B8 => Ok(self.twbr),
            0x00B9 => Ok(self.twsr),
            0x00BA => Ok(self.twar),
            0x00BB => Ok(self.twdr),
            0x00BC => Ok(self.twcr),
            // ADC: ADCL/ADCH/ADCSRA/ADMUX (data space)
            0x0078 => Ok(self.adcl),
            0x0079 => Ok(self.adch),
            0x007A => Ok(self.adcsra),
            0x007C => Ok(self.admux),
            0x0020..=0x00FF => Ok(self.io[(addr - 0x20) as usize]),
            a if (SRAM_START..=RAMEND).contains(&a) => Ok(self.sram[(a - SRAM_START) as usize]),
            _ => Err(SimulationError::MemoryViolation(addr as u64)),
        }
    }

    fn data_write(&mut self, addr: u16, value: u8, bus: &mut dyn Bus) -> SimResult<()> {
        match addr {
            0x0000..=0x001F => {
                self.r[addr as usize] = value;
                Ok(())
            }
            0x005D => {
                self.sp = (self.sp & 0xFF00) | value as u16;
                Ok(())
            }
            0x005E => {
                self.sp = (self.sp & 0x00FF) | ((value as u16) << 8);
                Ok(())
            }
            0x005F => {
                self.sreg = value;
                Ok(())
            }
            0x0035 => {
                self.tifr0 &= !value;
                Ok(())
            }
            0x0044 => {
                self.tccr0a = value;
                Ok(())
            }
            0x0045 => {
                self.tccr0b = value;
                Ok(())
            }
            0x0046 => {
                self.tcnt0 = value;
                Ok(())
            }
            0x0047 => {
                self.ocr0a = value;
                Ok(())
            }
            0x0048 => {
                self.ocr0b = value;
                Ok(())
            }
            0x006E => {
                self.timsk0 = value;
                Ok(())
            }
            0x00C0 => {
                if value & UCSRA_TXC != 0 {
                    self.ucsr0a &= !UCSRA_TXC;
                }
                self.ucsr0a |= UCSRA_UDRE;
                Ok(())
            }
            0x00C1 => {
                self.ucsr0b = value;
                Ok(())
            }
            0x00C2 => {
                self.ucsr0c = value;
                Ok(())
            }
            0x00C4 => {
                self.ubrr0 = (self.ubrr0 & 0xFF00) | value as u16;
                Ok(())
            }
            0x00C5 => {
                self.ubrr0 = (self.ubrr0 & 0x00FF) | ((value as u16) << 8);
                Ok(())
            }
            0x00C6 => {
                self.serial_tx.push(value);
                if let Some(sink) = &self.serial_sink {
                    if let Ok(mut g) = sink.lock() {
                        g.push(value);
                    }
                }
                self.ucsr0a |= UCSRA_UDRE | UCSRA_TXC;
                bus.write_u8(addr as u64, value)?;
                Ok(())
            }
            0x004C => {
                self.spcr = value;
                Ok(())
            }
            0x004D => {
                // Writing 1 to SPIF/WCOL clears them (AVR: write 1 then access SPDR).
                self.spsr &= !value;
                Ok(())
            }
            0x004E => {
                // SPI data: master clocks one byte through attached slaves.
                let mosi = value;
                let mut miso = 0u8;
                for dev in &mut self.spi_devices {
                    let resp = dev.transfer(mosi);
                    if resp != 0 {
                        miso = resp;
                    }
                }
                self.spdr = miso;
                self.spsr |= 1 << 7; // SPIF
                Ok(())
            }
            0x00B8 => {
                self.twbr = value;
                Ok(())
            }
            0x00B9 => {
                // Only prescaler bits [1:0] are writable; status is read-only.
                self.twsr = (self.twsr & 0xF8) | (value & 0x03);
                Ok(())
            }
            0x00BA => {
                self.twar = value;
                Ok(())
            }
            0x00BB => {
                self.twdr = value;
                Ok(())
            }
            0x00BC => {
                self.twi_write_cr(value);
                Ok(())
            }
            0x0078 => {
                self.adcl = value;
                Ok(())
            }
            0x0079 => {
                self.adch = value;
                Ok(())
            }
            0x007A => {
                // ADSC (bit 6): write 1 starts a conversion; complete immediately
                // with mid-scale 512 (~Vcc/2) so analogRead() never hangs.
                const ADSC: u8 = 1 << 6;
                const ADIF: u8 = 1 << 4;
                const ADEN: u8 = 1 << 7;
                self.adcsra = value;
                if value & ADEN != 0 && value & ADSC != 0 {
                    self.adcl = 0x00;
                    self.adch = 0x02; // 512
                    self.adcsra = (value & !ADSC) | ADIF;
                }
                Ok(())
            }
            0x007C => {
                self.admux = value;
                Ok(())
            }
            0x0020..=0x00FF => {
                self.io[(addr - 0x20) as usize] = value;
                // PORTB (0x23..0x25): mirror to high bus window so --watch-gpio
                // portb:N works (flash@0 swallows low-address bus writes).
                if (0x0023..=0x0025).contains(&addr) {
                    // High-window mirror is best-effort (must not fail IN/OUT).
                    let _mirror = bus.write_u8(0x0001_0000 + addr as u64, value);
                } else {
                    let _mirror = bus.write_u8(addr as u64, value);
                }
                Ok(())
            }
            a if (SRAM_START..=RAMEND).contains(&a) => {
                self.sram[(a - SRAM_START) as usize] = value;
                bus.write_u8(a as u64, value)?;
                Ok(())
            }
            _ => Err(SimulationError::MemoryViolation(addr as u64)),
        }
    }

    fn t0_prescaler(&self) -> u32 {
        match self.tccr0b & 0x07 {
            0 => 0,
            1 => 1,
            2 => 8,
            3 => 64,
            4 => 256,
            5 => 1024,
            _ => 0,
        }
    }

    pub fn tick_timer0(&mut self, cpu_cycles: u32) {
        let div = self.t0_prescaler();
        if div == 0 || cpu_cycles == 0 {
            return;
        }
        self.t0_prescale_acc = self.t0_prescale_acc.saturating_add(cpu_cycles);
        while self.t0_prescale_acc >= div {
            self.t0_prescale_acc -= div;
            let (next, overflowed) = self.tcnt0.overflowing_add(1);
            self.tcnt0 = next;
            if overflowed {
                self.tifr0 |= TIFR_TOV0;
                if self.timsk0 & TIMSK_TOIE0 != 0 {
                    self.pending_irq |= 1u64 << VEC_TIMER0_OVF;
                }
            }
        }
    }

    pub fn portb(&self) -> u8 {
        self.io[(0x25 - 0x20) as usize]
    }

    pub fn serial_as_str(&self) -> String {
        String::from_utf8_lossy(&self.serial_tx).into_owned()
    }

    pub fn set_serial_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>) {
        self.serial_sink = Some(sink);
    }

    pub fn push_spi_device(&mut self, device: Box<dyn SpiDevice>) {
        self.spi_devices.push(device);
    }

    pub fn push_i2c_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.i2c_slaves.push(device);
    }

    fn find_i2c_slave(&self, addr7: u8) -> Option<usize> {
        self.i2c_slaves.iter().position(|s| s.address() == addr7)
    }

    /// Write TWCR: writing 1 to TWINT clears it and starts the next TWI step
    /// (START / SLA / DATA / STOP). Completes immediately and re-asserts TWINT
    /// (except pure STOP) so Arduino's interrupt-driven `twi.c` advances.
    fn twi_write_cr(&mut self, value: u8) {
        let en = value & TWEN != 0;
        let ie = value & TWIE != 0;
        let start = value & TWSTA != 0;
        let stop = value & TWSTO != 0;
        let clear_int = value & TWINT != 0;
        let ack = value & TWEA != 0;

        // Preserve enable/ie/ea; drop TWINT/TWSTA/TWSTO/TWWC until op completes.
        self.twcr = value & (TWEN | TWIE | TWEA);

        if !en {
            self.twi_phase = TwiPhase::Idle;
            self.twi_slave = None;
            return;
        }

        if !clear_int {
            // Init path: TWCR = TWEN|TWIE|TWEA without starting a transfer.
            return;
        }

        if stop {
            if let Some(idx) = self.twi_slave {
                self.i2c_slaves[idx].stop();
            }
            self.twi_phase = TwiPhase::Idle;
            self.twi_slave = None;
            // STOP auto-clears TWSTO; TWINT is not set after STOP.
            self.twcr = TWEN | (if ie { TWIE } else { 0 }) | (if ack { TWEA } else { 0 });
            return;
        }

        if start {
            let status = if matches!(self.twi_phase, TwiPhase::Idle) {
                TW_START
            } else {
                TW_REP_START
            };
            self.twsr = (self.twsr & 0x03) | status;
            self.twi_phase = TwiPhase::Started;
            self.twcr = TWEN | TWINT | (if ie { TWIE } else { 0 }) | (if ack { TWEA } else { 0 });
            if ie {
                self.pending_irq |= 1u64 << VEC_TWI;
            }
            return;
        }

        // Continue: address or data depending on phase.
        match self.twi_phase {
            TwiPhase::Idle => {
                // Spurious TWINT clear with no START — no-info.
                self.twsr = (self.twsr & 0x03) | 0xF8;
            }
            TwiPhase::Started => {
                let addr7 = self.twdr >> 1;
                let is_read = self.twdr & 1 != 0;
                match self.find_i2c_slave(addr7) {
                    Some(idx) => {
                        self.twi_slave = Some(idx);
                        if is_read {
                            self.twi_phase = TwiPhase::Mr;
                            self.twsr = (self.twsr & 0x03) | TW_MR_SLA_ACK;
                        } else {
                            self.twi_phase = TwiPhase::Mt;
                            self.twsr = (self.twsr & 0x03) | TW_MT_SLA_ACK;
                        }
                    }
                    None => {
                        self.twi_slave = None;
                        self.twsr = (self.twsr & 0x03)
                            | if is_read {
                                TW_MR_SLA_NACK
                            } else {
                                TW_MT_SLA_NACK
                            };
                        self.twi_phase = TwiPhase::Idle;
                    }
                }
            }
            TwiPhase::Mt => {
                if let Some(idx) = self.twi_slave {
                    self.i2c_slaves[idx].write(self.twdr);
                    self.twsr = (self.twsr & 0x03) | TW_MT_DATA_ACK;
                } else {
                    self.twsr = (self.twsr & 0x03) | TW_MT_DATA_NACK;
                }
            }
            TwiPhase::Mr => {
                if let Some(idx) = self.twi_slave {
                    self.twdr = self.i2c_slaves[idx].read();
                    self.twsr =
                        (self.twsr & 0x03) | if ack { TW_MR_DATA_ACK } else { TW_MR_DATA_NACK };
                } else {
                    self.twdr = 0xFF;
                    self.twsr = (self.twsr & 0x03) | TW_MR_DATA_NACK;
                }
            }
        }

        self.twcr = TWEN | TWINT | (if ie { TWIE } else { 0 }) | (if ack { TWEA } else { 0 });
        // Drop TWWC (write collision) — not modelled.
        let _ = TWWC;
        if ie {
            self.pending_irq |= 1u64 << VEC_TWI;
        }
    }

    /// Load a ProgramImage: low addresses → flash; data-space addresses → SRAM.
    pub fn load_program_image(&mut self, image: &crate::memory::ProgramImage) {
        for seg in &image.segments {
            let addr = seg.start_addr;
            if let Some(d) = strip_avr_data_bias(addr) {
                // Data / EEPROM space (biased VMA from avr-gcc).
                for (i, b) in seg.data.iter().enumerate() {
                    let a = d as u16 + i as u16;
                    if (SRAM_START..=RAMEND).contains(&a) {
                        self.sram[(a - SRAM_START) as usize] = *b;
                    } else if (0x20..=0xFF).contains(&a) {
                        self.io[(a - 0x20) as usize] = *b;
                    }
                }
            } else if addr < 0x8000 {
                // Program flash (code + data LMA).
                self.load_flash(addr as u32, &seg.data);
            }
        }
        self.pc = image.entry_point as u32 & !1;
    }

    fn push_byte(&mut self, value: u8, bus: &mut dyn Bus) -> SimResult<()> {
        self.data_write(self.sp, value, bus)?;
        self.sp = self.sp.wrapping_sub(1);
        Ok(())
    }

    fn pop_byte(&mut self, bus: &dyn Bus) -> SimResult<u8> {
        self.sp = self.sp.wrapping_add(1);
        self.data_read(self.sp, bus)
    }

    fn push_pc(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        let word = (self.pc / 2) as u16;
        self.push_byte((word >> 8) as u8, bus)?;
        self.push_byte((word & 0xFF) as u8, bus)?;
        Ok(())
    }

    fn pop_pc(&mut self, bus: &dyn Bus) -> SimResult<()> {
        let lo = self.pop_byte(bus)? as u16;
        let hi = self.pop_byte(bus)? as u16;
        let word = (hi << 8) | lo;
        self.pc = (word as u32) * 2;
        Ok(())
    }

    fn word_size_bytes(op: u16) -> u32 {
        let top = op & 0xFE0E;
        if top == 0x940C || top == 0x940E {
            return 4;
        }
        if (op & 0xFE0F) == 0x9000 || (op & 0xFE0F) == 0x9200 {
            return 4;
        }
        2
    }

    fn try_take_irq(&mut self, bus: &mut dyn Bus) -> SimResult<bool> {
        if !self.flag_i() || self.pending_irq == 0 {
            return Ok(false);
        }
        let vec = self.pending_irq.trailing_zeros();
        if vec == 0 || vec > 31 {
            return Ok(false);
        }
        self.pending_irq &= !(1u64 << vec);
        self.set_flag_i(false);
        // Hardware clears the matching timer overflow flag on vector entry.
        if vec == VEC_TIMER0_OVF {
            self.tifr0 &= !TIFR_TOV0;
        }
        self.push_pc(bus)?;
        self.pc = vec.saturating_sub(1) * 4;
        Ok(true)
    }

    /// Raw encoding at `pc`, for the trace only.
    ///
    /// Read at the SAME widths the fetch path uses, so an observed run touches
    /// exactly the bytes an unobserved one does — a trace that perturbs the run
    /// it measures is useless. Returns the word count too, because AVR mixes
    /// 16- and 32-bit instructions and a trace that reported only the first
    /// word of `JMP` could not be disassembled back.
    ///
    /// The four 32-bit families are recognised by the same masks the decoder
    /// below uses: LDS 0xFE0F/0x9000, STS 0xFE0F/0x9200, JMP 0xFE0E/0x940C,
    /// CALL 0xFE0E/0x940E. A fetch that would fault reports 0 rather than
    /// propagating: the trace must not turn a readable run into an error.
    fn raw_word_for_trace(&self, pc: u32) -> (u32, u32) {
        let Ok(lo) = self.fetch_word(pc) else {
            return (0, 2);
        };
        let is_32 = (lo & 0xFE0F) == 0x9000
            || (lo & 0xFE0F) == 0x9200
            || (lo & 0xFE0E) == 0x940C
            || (lo & 0xFE0E) == 0x940E;
        if !is_32 {
            return (u32::from(lo), 2);
        }
        match self.fetch_word(pc.wrapping_add(2)) {
            // Little-endian in flash, so the second word is the high half —
            // the same order `expected_opcode` is assembled in by
            // cpu_trace_conformance.
            Ok(hi) => ((u32::from(hi) << 16) | u32::from(lo), 4),
            Err(_) => (u32::from(lo), 2),
        }
    }

    fn step_inner(
        &mut self,
        bus: &mut dyn Bus,
        _observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
        let pc = self.pc;
        let op = self.fetch_word(pc)?;
        let mut next = pc.wrapping_add(2);

        if op == 0x0000 {
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }
        if op == 0x9478 {
            self.set_flag_i(true);
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }
        if op == 0x94F8 {
            self.set_flag_i(false);
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }
        if op == 0x9508 {
            self.pop_pc(bus)?;
            self.cycles += 4;
            return Ok(());
        }
        if op == 0x9518 {
            self.pop_pc(bus)?;
            self.set_flag_i(true);
            self.cycles += 4;
            return Ok(());
        }
        if op == 0x9588 {
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }
        if op == 0x9598 {
            return Err(SimulationError::Halt);
        }

        // RJMP
        if (op & 0xF000) == 0xC000 {
            let k = op & 0x0FFF;
            let offset = if k & 0x0800 != 0 {
                (k | 0xF000) as i16
            } else {
                k as i16
            };
            let pc_word = (pc / 2) as i32 + 1 + offset as i32;
            self.pc = (pc_word as u32) * 2;
            self.cycles += 2;
            return Ok(());
        }

        // RCALL
        if (op & 0xF000) == 0xD000 {
            let k = op & 0x0FFF;
            let offset = if k & 0x0800 != 0 {
                (k | 0xF000) as i16
            } else {
                k as i16
            };
            self.pc = next;
            self.push_pc(bus)?;
            let pc_word = (pc / 2) as i32 + 1 + offset as i32;
            self.pc = (pc_word as u32) * 2;
            self.cycles += 3;
            return Ok(());
        }

        // LDI
        if (op & 0xF000) == 0xE000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            self.r[rd] = k;
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // OUT
        if (op & 0xF800) == 0xB800 {
            let a = (((op >> 5) & 0x30) | (op & 0x0F)) as u8;
            let rr = ((op >> 4) & 0x1F) as usize;
            let data_addr = 0x20u16 + a as u16;
            self.data_write(data_addr, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // IN
        if (op & 0xF800) == 0xB000 {
            let a = (((op >> 5) & 0x30) | (op & 0x0F)) as u8;
            let rd = ((op >> 4) & 0x1F) as usize;
            let data_addr = 0x20u16 + a as u16;
            self.r[rd] = self.data_read(data_addr, bus)?;
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SBI
        if (op & 0xFF00) == 0x9A00 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let data_addr = 0x20u16 + a as u16;
            let v = self.data_read(data_addr, bus)? | (1 << b);
            self.data_write(data_addr, v, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // CBI
        if (op & 0xFF00) == 0x9800 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let data_addr = 0x20u16 + a as u16;
            let v = self.data_read(data_addr, bus)? & !(1 << b);
            self.data_write(data_addr, v, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // LDS
        if (op & 0xFE0F) == 0x9000 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let k = self.fetch_word(next)?;
            next = next.wrapping_add(2);
            self.r[rd] = self.data_read(k, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // STS
        if (op & 0xFE0F) == 0x9200 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let k = self.fetch_word(next)?;
            next = next.wrapping_add(2);
            self.data_write(k, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // MOV
        if (op & 0xFC00) == 0x2C00 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            self.r[rd] = self.r[rr];
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // ADD
        if (op & 0xFC00) == 0x0C00 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let a = self.r[rd];
            let b = self.r[rr];
            let (res, c) = a.overflowing_add(b);
            let v = (!(a ^ b) & (a ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }
        // ADC Rd,Rr: 0001 11rd dddd rrrr
        if (op & 0xFC00) == 0x1C00 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let carry = self.sreg & 1;
            let sum = self.r[rd] as u16 + self.r[rr] as u16 + carry as u16;
            let res = sum as u8;
            let c = sum > 0xFF;
            let v = (!(self.r[rd] ^ self.r[rr]) & (self.r[rd] ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // EOR
        if (op & 0xFC00) == 0x2400 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let res = self.r[rd] ^ self.r[rr];
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // AND
        if (op & 0xFC00) == 0x2000 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let res = self.r[rd] & self.r[rr];
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // OR
        if (op & 0xFC00) == 0x2800 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let res = self.r[rd] | self.r[rr];
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // CP
        if (op & 0xFC00) == 0x1400 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let a = self.r[rd];
            let b = self.r[rr];
            let (res, c) = a.overflowing_sub(b);
            let v = ((a ^ b) & (a ^ res)) & 0x80 != 0;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SUB Rd,Rr: 0001 10rd dddd rrrr
        if (op & 0xFC00) == 0x1800 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let a = self.r[rd];
            let b = self.r[rr];
            let (res, c) = a.overflowing_sub(b);
            let v = ((a ^ b) & (a ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // MUL Rd,Rr: 1001 11rd dddd rrrr → R1:R0 = Rd * Rr (unsigned)
        if (op & 0xFC00) == 0x9C00 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let prod = (self.r[rd] as u16) * (self.r[rr] as u16);
            self.r[0] = (prod & 0xFF) as u8;
            self.r[1] = (prod >> 8) as u8;
            self.set_c((prod & 0x8000) != 0);
            self.set_z(if prod == 0 { 0 } else { 1 });
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // MULS Rd,Rr: 0000 0010 dddd rrrr  (Rd,Rr in 16..31)
        if (op & 0xFF00) == 0x0200 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let rr = 16 + (op & 0x0F) as usize;
            let prod = (self.r[rd] as i8 as i16) * (self.r[rr] as i8 as i16);
            let prod_u = prod as u16;
            self.r[0] = (prod_u & 0xFF) as u8;
            self.r[1] = (prod_u >> 8) as u8;
            self.set_c((prod_u & 0x8000) != 0);
            self.set_z(if prod_u == 0 { 0 } else { 1 });
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // MULSU Rd,Rr: 0000 0011 0ddd 0rrr (Rd,Rr in 16..23)
        if (op & 0xFF88) == 0x0300 {
            let rd = 16 + ((op >> 4) & 0x07) as usize;
            let rr = 16 + (op & 0x07) as usize;
            let prod = (self.r[rd] as i8 as i16) * (self.r[rr] as i16);
            let prod_u = prod as u16;
            self.r[0] = (prod_u & 0xFF) as u8;
            self.r[1] = (prod_u >> 8) as u8;
            self.set_c((prod_u & 0x8000) != 0);
            self.set_z(if prod_u == 0 { 0 } else { 1 });
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // ICALL: 1001 0101 0000 1001 — call to Z (word address)
        if op == 0x9509 {
            self.pc = next;
            self.push_pc(bus)?;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.pc = (z as u32) * 2;
            self.cycles += 3;
            return Ok(());
        }

        // NEG Rd: 1001 010d dddd 0001
        if (op & 0xFE0F) == 0x9401 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let a = self.r[rd];
            let res = (0u8).wrapping_sub(a);
            self.r[rd] = res;
            self.set_c(a != 0);
            self.set_z(res);
            self.set_n(res);
            self.set_v(res == 0x80);
            self.update_s_from_nv();
            // H flag: roughly from borrow into bit 3
            if (a & 0x0F) != 0 {
                self.sreg |= 0x20;
            } else {
                self.sreg &= !0x20;
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SWAP Rd: 1001 010d dddd 0010
        if (op & 0xFE0F) == 0x9402 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let v = self.r[rd];
            self.r[rd] = v.rotate_left(4);
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // ASR Rd: 1001 010d dddd 0101
        if (op & 0xFE0F) == 0x9405 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let a = self.r[rd];
            let res = ((a as i8) >> 1) as u8;
            self.r[rd] = res;
            self.set_c(a & 1 != 0);
            self.set_z(res);
            self.set_n(res);
            self.set_v(((res >> 7) ^ (a & 1)) != 0);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // LSR Rd: 1001 010d dddd 0110
        if (op & 0xFE0F) == 0x9406 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let a = self.r[rd];
            let c = a & 1 != 0;
            let res = a >> 1;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.sreg &= !0x04; // N = 0
            self.set_v(c); // V = N⊕C = C
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // ROR Rd: 1001 010d dddd 0111
        if (op & 0xFE0F) == 0x9407 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let a = self.r[rd];
            let c_in = self.sreg & 1;
            let c_out = a & 1 != 0;
            let res = (a >> 1) | (c_in << 7);
            self.r[rd] = res;
            self.set_c(c_out);
            self.set_z(res);
            self.set_n(res);
            let n = (res >> 7) & 1;
            self.set_v((n ^ (c_out as u8)) != 0);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // CPI
        if (op & 0xF000) == 0x3000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            let a = self.r[rd];
            let (res, c) = a.overflowing_sub(k);
            let v = ((a ^ k) & (a ^ res)) & 0x80 != 0;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // BRcc
        if (op & 0xF800) == 0xF000 {
            let k = ((op >> 3) & 0x7F) as i8;
            let offset = if k & 0x40 != 0 { k | !0x7F } else { k };
            let bit = (op & 0x07) as u8;
            let complement = (op & 0x0400) != 0;
            let flag = (self.sreg >> bit) & 1 != 0;
            let take = if complement { !flag } else { flag };
            if take {
                let pc_word = (pc / 2) as i32 + 1 + offset as i32;
                self.pc = (pc_word as u32) * 2;
                self.cycles += 2;
            } else {
                self.pc = next;
                self.cycles += 1;
            }
            return Ok(());
        }

        // PUSH
        if (op & 0xFE0F) == 0x920F {
            let rr = ((op >> 4) & 0x1F) as usize;
            self.push_byte(self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // POP
        if (op & 0xFE0F) == 0x900F {
            let rd = ((op >> 4) & 0x1F) as usize;
            self.r[rd] = self.pop_byte(bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // ADIW
        if (op & 0xFF00) == 0x9600 {
            let d = ((op >> 4) & 0x03) as usize;
            let rd = 24 + d * 2;
            let k = (((op >> 6) & 0x03) << 4) | (op & 0x0F);
            let val = u16::from_le_bytes([self.r[rd], self.r[rd + 1]]).wrapping_add(k);
            self.r[rd] = (val & 0xFF) as u8;
            self.r[rd + 1] = (val >> 8) as u8;
            self.set_z(if val == 0 { 0 } else { 1 });
            self.set_n((val >> 8) as u8);
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // SBIW
        if (op & 0xFF00) == 0x9700 {
            let d = ((op >> 4) & 0x03) as usize;
            let rd = 24 + d * 2;
            let k = (((op >> 6) & 0x03) << 4) | (op & 0x0F);
            let val = u16::from_le_bytes([self.r[rd], self.r[rd + 1]]).wrapping_sub(k);
            self.r[rd] = (val & 0xFF) as u8;
            self.r[rd + 1] = (val >> 8) as u8;
            self.set_z(if val == 0 { 0 } else { 1 });
            self.set_n((val >> 8) as u8);
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // MOVW
        if (op & 0xFF00) == 0x0100 {
            let rd = ((op >> 4) & 0x0F) as usize * 2;
            let rr = (op & 0x0F) as usize * 2;
            self.r[rd] = self.r[rr];
            self.r[rd + 1] = self.r[rr + 1];
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // IJMP
        if op == 0x9409 {
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.pc = (z as u32) * 2;
            self.cycles += 2;
            return Ok(());
        }

        // LPM
        if op == 0x95C8 {
            let z = u16::from_le_bytes([self.r[30], self.r[31]]) as usize;
            self.r[0] = self.flash.get(z).copied().unwrap_or(0xFF);
            self.pc = next;
            self.cycles += 3;
            return Ok(());
        }

        // LPM Rd,Z
        if (op & 0xFE0F) == 0x9004 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]) as usize;
            self.r[rd] = self.flash.get(z).copied().unwrap_or(0xFF);
            self.pc = next;
            self.cycles += 3;
            return Ok(());
        }

        // LPM Rd,Z+
        if (op & 0xFE0F) == 0x9005 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.r[rd] = self.flash.get(z as usize).copied().unwrap_or(0xFF);
            let z2 = z.wrapping_add(1);
            self.r[30] = (z2 & 0xFF) as u8;
            self.r[31] = (z2 >> 8) as u8;
            self.pc = next;
            self.cycles += 3;
            return Ok(());
        }

        // LD Rd,X
        if (op & 0xFE0F) == 0x900C {
            let rd = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]);
            self.r[rd] = self.data_read(x, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // ST X,Rr
        // ST X, Rr: 1001 001r rrrr 1100
        if (op & 0xFE0F) == 0x920C {
            let rr = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]);
            self.data_write(x, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // ST X+, Rr: 1001 001r rrrr 1101
        if (op & 0xFE0F) == 0x920D {
            let rr = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]);
            self.data_write(x, self.r[rr], bus)?;
            let x2 = x.wrapping_add(1);
            self.r[26] = (x2 & 0xFF) as u8;
            self.r[27] = (x2 >> 8) as u8;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // ST -X, Rr: 1001 001r rrrr 1110
        if (op & 0xFE0F) == 0x920E {
            let rr = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]).wrapping_sub(1);
            self.r[26] = (x & 0xFF) as u8;
            self.r[27] = (x >> 8) as u8;
            self.data_write(x, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // LD Rd, X+: 1001 000d dddd 1101
        if (op & 0xFE0F) == 0x900D {
            let rd = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]);
            self.r[rd] = self.data_read(x, bus)?;
            let x2 = x.wrapping_add(1);
            self.r[26] = (x2 & 0xFF) as u8;
            self.r[27] = (x2 >> 8) as u8;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // LD Rd, -X: 1001 000d dddd 1110
        if (op & 0xFE0F) == 0x900E {
            let rd = ((op >> 4) & 0x1F) as usize;
            let x = u16::from_le_bytes([self.r[26], self.r[27]]).wrapping_sub(1);
            self.r[26] = (x & 0xFF) as u8;
            self.r[27] = (x >> 8) as u8;
            self.r[rd] = self.data_read(x, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // JMP
        if (op & 0xFE0E) == 0x940C {
            let k_hi = ((op >> 3) & 0x3E) | (op & 0x01);
            let k_lo = self.fetch_word(next)?;
            let k = ((k_hi as u32) << 16) | k_lo as u32;
            self.pc = k * 2;
            self.cycles += 3;
            return Ok(());
        }

        // CALL
        if (op & 0xFE0E) == 0x940E {
            let k_hi = ((op >> 3) & 0x3E) | (op & 0x01);
            let k_lo = self.fetch_word(next)?;
            let k = ((k_hi as u32) << 16) | k_lo as u32;
            self.pc = next.wrapping_add(2);
            self.push_pc(bus)?;
            self.pc = k * 2;
            self.cycles += 4;
            return Ok(());
        }

        // SBIS
        if (op & 0xFF00) == 0x9B00 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let v = self.data_read(0x20 + a as u16, bus)?;
            if v & (1 << b) != 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SBIC
        if (op & 0xFF00) == 0x9900 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let v = self.data_read(0x20 + a as u16, bus)?;
            if v & (1 << b) == 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SBRS
        if (op & 0xFE08) == 0xFE00 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let b = (op & 0x07) as u8;
            if self.r[rr] & (1 << b) != 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SBRC
        if (op & 0xFE08) == 0xFC00 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let b = (op & 0x07) as u8;
            if self.r[rr] & (1 << b) == 0 {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // CPSE
        if (op & 0xFC00) == 0x1000 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            if self.r[rd] == self.r[rr] {
                let following = self.fetch_word(next)?;
                next = next.wrapping_add(Self::word_size_bytes(following));
            }
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // INC
        if (op & 0xFE0F) == 0x9403 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let res = self.r[rd].wrapping_add(1);
            self.r[rd] = res;
            self.set_v(res == 0x80);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // DEC
        if (op & 0xFE0F) == 0x940A {
            let rd = ((op >> 4) & 0x1F) as usize;
            let res = self.r[rd].wrapping_sub(1);
            self.r[rd] = res;
            self.set_v(res == 0x7F);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // ANDI Rd,K: 0111 KKKK dddd KKKK  (Rd 16..31)
        if (op & 0xF000) == 0x7000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            let res = self.r[rd] & k;
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // ORI Rd,K: 0110 KKKK dddd KKKK
        if (op & 0xF000) == 0x6000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            let res = self.r[rd] | k;
            self.r[rd] = res;
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SUBI Rd,K: 0101 KKKK dddd KKKK
        if (op & 0xF000) == 0x5000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            let a = self.r[rd];
            let (res, c) = a.overflowing_sub(k);
            let v = ((a ^ k) & (a ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SBCI Rd,K: 0100 KKKK dddd KKKK
        if (op & 0xF000) == 0x4000 {
            let rd = 16 + ((op >> 4) & 0x0F) as usize;
            let k = ((op & 0x0F00) >> 4) as u8 | (op & 0x0F) as u8;
            let carry = self.sreg & 1;
            let a = self.r[rd] as u16;
            let sub = k as u16 + carry as u16;
            let (res16, c1) = a.overflowing_sub(sub);
            let res = res16 as u8;
            let v = ((self.r[rd] ^ k) & (self.r[rd] ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c1 || a < sub);
            self.set_z(res);
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // CPC Rd,Rr: 0000 01rd dddd rrrr
        if (op & 0xFC00) == 0x0400 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let carry = self.sreg & 1;
            let a = self.r[rd] as u16;
            let b = self.r[rr] as u16 + carry as u16;
            let (res16, _) = a.overflowing_sub(b);
            let res = res16 as u8;
            let c = a < b;
            let v = ((self.r[rd] ^ self.r[rr]) & (self.r[rd] ^ res)) & 0x80 != 0;
            self.set_c(c);
            // Z is sticky for CPC: only clear if res != 0
            if res != 0 {
                self.sreg &= !0x02;
            }
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // SBC Rd,Rr: 0000 10rd dddd rrrr
        if (op & 0xFC00) == 0x0800 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let rr = (((op >> 5) & 0x10) | (op & 0x0F)) as usize;
            let carry = self.sreg & 1;
            let a = self.r[rd] as u16;
            let b = self.r[rr] as u16 + carry as u16;
            let res = a.wrapping_sub(b) as u8;
            let c = a < b;
            let v = ((self.r[rd] ^ self.r[rr]) & (self.r[rd] ^ res)) & 0x80 != 0;
            self.r[rd] = res;
            self.set_c(c);
            if res != 0 {
                self.sreg &= !0x02;
            } else { /* Z sticky leave */
            }
            self.set_n(res);
            self.set_v(v);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // COM Rd: 1001 010d dddd 0000
        if (op & 0xFE0F) == 0x9400 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let res = !self.r[rd];
            self.r[rd] = res;
            self.set_c(true);
            self.set_v(false);
            self.set_z(res);
            self.set_n(res);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(());
        }

        // LDD Rd, Z+q: 10q0 qq0d dddd 0qqq  (bit3=0 → Z)
        // LDD Rd, Y+q: 10q0 qq0d dddd 1qqq  (bit3=1 → Y)
        // STD Z+q / Y+q: same with bit9=1 (store).
        if (op & 0xD000) == 0x8000 {
            let q = ((op & 0x2000) >> 8) | ((op & 0x0C00) >> 7) | (op & 0x07);
            let reg = ((op >> 4) & 0x1F) as usize;
            let is_st = (op & 0x0200) != 0;
            // ISA: bit 3 clear = Z, set = Y (not the other way around).
            let use_y = (op & 0x0008) != 0;
            let base = if use_y {
                u16::from_le_bytes([self.r[28], self.r[29]])
            } else {
                u16::from_le_bytes([self.r[30], self.r[31]])
            };
            let addr = base.wrapping_add(q);
            if is_st {
                self.data_write(addr, self.r[reg], bus)?;
            } else {
                self.r[reg] = self.data_read(addr, bus)?;
            }
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // LD Rd, Y+ / -Y / Z+ / -Z and ST counterparts
        // LD Rd, Y+: 1001 000d dddd 1001
        // LD Rd, -Y: 1001 000d dddd 1010
        // LD Rd, Z+: 1001 000d dddd 0001
        // LD Rd, -Z: 1001 000d dddd 0010
        // ST Y+, Rr: 1001 001r rrrr 1001 etc.
        if (op & 0xFE0C) == 0x9008
            || (op & 0xFE0C) == 0x9000
            || (op & 0xFE0C) == 0x9208
            || (op & 0xFE0C) == 0x9200
        {
            let is_st = (op & 0x0200) != 0;
            let reg = ((op >> 4) & 0x1F) as usize;
            let mode = op & 0x0F;
            // Y modes: 1001, 1010, 1100(ld Y), Z: 0001, 0010, 0000(ld Z bare handled elsewhere)
            let (base_lo, base_hi, predec, postinc) = match mode {
                0x9 => (28usize, 29usize, false, true),  // Y+
                0xA => (28, 29, true, false),            // -Y
                0x1 => (30, 31, false, true),            // Z+
                0x2 => (30, 31, true, false),            // -Z
                0xC if !is_st => (28, 29, false, false), // LD Rd, Y
                0x8 if is_st => (28, 29, false, false),  // unlikely
                0x0 if !is_st && (op & 0xFE0F) == 0x9000 => {
                    // already LDS
                    (0, 0, false, false)
                }
                _ => (0, 0, false, false),
            };
            if base_lo != 0 {
                let mut base = u16::from_le_bytes([self.r[base_lo], self.r[base_hi]]);
                if predec {
                    base = base.wrapping_sub(1);
                }
                if is_st {
                    self.data_write(base, self.r[reg], bus)?;
                } else {
                    self.r[reg] = self.data_read(base, bus)?;
                }
                if postinc {
                    base = base.wrapping_add(1);
                }
                if predec || postinc {
                    self.r[base_lo] = (base & 0xFF) as u8;
                    self.r[base_hi] = (base >> 8) as u8;
                }
                self.pc = next;
                self.cycles += 2;
                return Ok(());
            }
        }

        // LD Rd, Y: 1000 000d dddd 1000
        // ST Y, Rr: 1000 001r rrrr 1000
        // LD Rd, Z: 1000 000d dddd 0000
        // ST Z, Rr: 1000 001r rrrr 0000
        if (op & 0xD208) == 0x8000 || (op & 0xD208) == 0x8008 {
            // might overlap LDD — already handled with q bits
        }
        if (op & 0xFE0F) == 0x8008 {
            // LD Rd, Y (q=0 form without q bits) — actually 1000 000d dddd 1000
            let rd = ((op >> 4) & 0x1F) as usize;
            let y = u16::from_le_bytes([self.r[28], self.r[29]]);
            self.r[rd] = self.data_read(y, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }
        if (op & 0xFE0F) == 0x8208 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let y = u16::from_le_bytes([self.r[28], self.r[29]]);
            self.data_write(y, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }
        if (op & 0xFE0F) == 0x8000 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.r[rd] = self.data_read(z, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }
        if (op & 0xFE0F) == 0x8200 {
            let rr = ((op >> 4) & 0x1F) as usize;
            let z = u16::from_le_bytes([self.r[30], self.r[31]]);
            self.data_write(z, self.r[rr], bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        // FMUL Rd,Rr: 0000 0011 0ddd 1rrr (Rd,Rr in 16..23)
        if (op & 0xFF88) == 0x0308 {
            let rd = 16 + ((op >> 4) & 0x07) as usize;
            let rr = 16 + (op & 0x07) as usize;
            let prod = (self.r[rd] as u16) * (self.r[rr] as u16);
            let shifted = prod << 1;
            self.r[0] = (shifted & 0xFF) as u8;
            self.r[1] = (shifted >> 8) as u8;
            self.set_c((prod & 0x8000) != 0);
            self.set_z(if shifted == 0 { 0 } else { 1 });
            self.pc = next;
            self.cycles += 2;
            return Ok(());
        }

        Err(SimulationError::DecodeError(pc as u64))
    }
}

impl Cpu for Avr {
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.r = [0; 32];
        self.pc = 0;
        self.sp = RAMEND;
        self.sreg = 0;
        self.pending_irq = 0;
        self.cycles = 0;
        self.tcnt0 = 0;
        self.tccr0a = 0;
        self.tccr0b = 0;
        self.timsk0 = 0;
        self.tifr0 = 0;
        self.t0_prescale_acc = 0;
        self.serial_tx.clear();
        self.ucsr0a = UCSRA_UDRE;
        self.ucsr0b = 0;
        self.ucsr0c = 0;
        self.ubrr0 = 0;
        self.spcr = 0;
        self.spsr = 0;
        self.spdr = 0;
        self.twbr = 0;
        self.twsr = 0xF8;
        self.twar = 0;
        self.twdr = 0xFF;
        self.twcr = 0;
        self.twi_phase = TwiPhase::Idle;
        self.twi_slave = None;
        // Keep attached SPI/I2C slaves across reset (same wiring as real board).
        Ok(())
    }

    /// One instruction, plus the standardized instruction trace.
    ///
    /// The trace contract is documented on `SimulationObserver`:
    /// `on_step_start(pc, opcode)`, then `InstructionRetired`, then
    /// `on_step_end(cycles, registers)` whose register slice ends `[.., SP, PC]`
    /// with PC already advanced. It is proven per core by
    /// `crates/core/tests/cpu_trace_conformance.rs`.
    ///
    /// This core used to ignore `observers` entirely — the same defect that
    /// file was written after finding on Xtensa. `--trace` produced an empty
    /// file for every AVR chip and nothing failed, because nothing checked.
    ///
    /// The interrupt is taken HERE rather than inside `step_inner` so that this
    /// method can tell the two cases apart: vectoring to a handler retires no
    /// instruction, so it must emit no `InstructionRetired`.
    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        let before = self.cycles;

        if self.try_take_irq(bus)? {
            self.cycles += 4;
            let delta = self.cycles.saturating_sub(before) as u32;
            self.tick_timer0(delta.max(1));
            return Ok(());
        }

        // Building the register snapshot is pure waste when nothing observes
        // it, and this runs on every instruction — so all of it is gated.
        let observed = !observers.is_empty();
        let pc = self.pc;
        let opcode = if observed {
            self.raw_word_for_trace(pc).0
        } else {
            0
        };
        if observed {
            for obs in observers {
                obs.on_step_start(pc, opcode);
            }
        }

        self.step_inner(bus, observers, config)?;

        let delta = self.cycles.saturating_sub(before) as u32;

        if observed {
            // 32 general registers, then the standard trailer: SP, then PC.
            let mut registers = [0u32; 34];
            for (slot, value) in registers.iter_mut().zip(self.r.iter()) {
                *slot = u32::from(*value);
            }
            registers[32] = u32::from(self.sp);
            registers[33] = self.pc;

            crate::emit_trace_event(
                observers,
                labwired_hw_trace::TraceEvent::InstructionRetired { pc, opcode },
            );
            for obs in observers {
                obs.on_step_end(delta, &registers);
            }
        }

        self.tick_timer0(delta.max(1));
        Ok(())
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val & !1;
    }
    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_sp(&mut self, val: u32) {
        self.sp = val as u16;
    }
    fn set_exception_pending(&mut self, exception_num: u32) {
        if exception_num > 0 && exception_num < 64 {
            self.pending_irq |= 1u64 << exception_num;
        }
    }

    fn get_register(&self, id: u8) -> u32 {
        match id {
            0..=31 => self.r[id as usize] as u32,
            32 => self.sp as u32,
            33 => self.sreg as u32,
            34 => self.pc,
            _ => 0,
        }
    }

    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0..=31 => self.r[id as usize] = val as u8,
            32 => self.sp = val as u16,
            33 => self.sreg = val as u8,
            34 => self.pc = val & !1,
            _ => {}
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::Avr(AvrCpuSnapshot {
            registers: self.r.to_vec(),
            pc: self.pc,
            sp: self.sp,
            sreg: self.sreg,
        })
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Avr(s) = snapshot {
            for (i, v) in s.registers.iter().take(32).enumerate() {
                self.r[i] = *v;
            }
            self.pc = s.pc & !1;
            self.sp = s.sp;
            self.sreg = s.sreg;
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names: Vec<String> = (0..32).map(|i| format!("R{i}")).collect();
        names.push("SP".into());
        names.push("SREG".into());
        names.push("PC".into());
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        let u = name.to_uppercase();
        if let Some(rest) = u.strip_prefix('R') {
            if let Ok(n) = rest.parse::<u8>() {
                if n < 32 {
                    return Some(n);
                }
            }
        }
        match u.as_str() {
            "SP" => Some(32),
            "SREG" => Some(33),
            "PC" => Some(34),
            "X" => Some(26),
            "Y" => Some(28),
            "Z" => Some(30),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DmaRequest, SimulationConfig};
    use std::collections::HashMap;

    struct MockBus {
        mem: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl MockBus {
        fn new() -> Self {
            Self {
                mem: HashMap::new(),
                config: SimulationConfig::default(),
            }
        }
    }

    impl Bus for MockBus {
        fn read_u8(&self, addr: u64) -> SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    #[test]
    fn rjmp_self_retires_10000_steps() {
        let mut cpu = Avr::new();
        cpu.load_words(0, &[0xCFFF]);
        cpu.set_pc(0);
        let mut bus = MockBus::new();
        let cfg = SimulationConfig::default();
        for _ in 0..10_000 {
            cpu.step(&mut bus, &[], &cfg).unwrap();
        }
        assert_eq!(cpu.get_pc(), 0);
    }

    #[test]
    fn pc_rejects_odd_fetch() {
        let mut cpu = Avr::new();
        cpu.load_words(0, &[0x0000]);
        cpu.pc = 1;
        let mut bus = MockBus::new();
        let err = cpu
            .step(&mut bus, &[], &SimulationConfig::default())
            .unwrap_err();
        assert!(matches!(err, SimulationError::DecodeError(1)));
    }

    #[test]
    fn in_out_and_lds_alias_same_io_register() {
        let mut cpu = Avr::new();
        // LDI R16,0xA5; OUT PORTB(io5),R16; LDS R17,0x25
        cpu.load_words(0, &[0xEA05, 0xB905, 0x9110, 0x0025, 0xCFFF]);
        let mut bus = MockBus::new();
        let cfg = SimulationConfig::default();
        cpu.set_pc(0);
        cpu.step(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.r[16], 0xA5);
        cpu.step(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.io[(0x25 - 0x20) as usize], 0xA5);
        cpu.step(&mut bus, &[], &cfg).unwrap();
        assert_eq!(cpu.r[17], 0xA5);
    }

    #[test]
    fn data_bias_strip() {
        assert_eq!(strip_avr_data_bias(0x0080_0100), Some(0x100));
        assert_eq!(strip_avr_data_bias(0x0081_0010), Some(0x10));
        assert_eq!(strip_avr_data_bias(0x0000_0100), None);
        assert_eq!(classify_avr_vma(0x0080_0200), (AvrLoadSpace::Data, 0x200));
        assert_eq!(classify_avr_vma(0x0000_0040), (AvrLoadSpace::Flash, 0x40));
    }

    #[test]
    fn sbi_sets_portb_bit() {
        let mut cpu = Avr::new();
        cpu.load_words(0, &[0x9A2D, 0xCFFF]);
        let mut bus = MockBus::new();
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.io[0x05], 1 << 5);
    }

    #[test]
    fn st_x_plus_increments() {
        let mut cpu = Avr::new();
        // ST X+, r1 with r1=0, X=0x120
        cpu.r[1] = 0;
        cpu.r[26] = 0x20;
        cpu.r[27] = 0x01;
        cpu.flash[0] = 0x1d; // ST X+, r1 = 0x921d LE
        cpu.flash[1] = 0x92;
        cpu.flash[2] = 0xff; // rjmp .-2
        cpu.flash[3] = 0xcf;
        cpu.set_pc(0);
        let mut bus = MockBus::new();
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.r[26], 0x21);
        assert_eq!(cpu.r[27], 0x01);
        assert_eq!(cpu.sram[0x20], 0); // 0x120-0x100=0x20
    }

    /// Regression: STD Z+q must use Z (bit3=0), not Y — wrong polarity
    /// clobbered SPH when Y held the ctor-table cursor (0x5C) and q hit 0x5E.
    #[test]
    fn std_z_plus_q_does_not_touch_sp_via_y() {
        let mut cpu = Avr::new();
        cpu.sp = 0x08FD;
        // Y = 0x005C (looks like ctor cursor); Z = 0x0129 (Serial object)
        cpu.r[28] = 0x5C;
        cpu.r[29] = 0x00;
        cpu.r[30] = 0x29;
        cpu.r[31] = 0x01;
        cpu.r[1] = 0x00;
        // STD Z+2, r1 = 0x8212 (bit3 clear → Z)
        cpu.load_words(0, &[0x8212, 0xCFFF]);
        let mut bus = MockBus::new();
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.sp, 0x08FD, "SPH/SPL must not change");
        // 0x129+2 = 0x12B → sram index 0x2B
        assert_eq!(cpu.sram[0x2B], 0x00);
    }

    #[test]
    fn lpm_rd_z_r25_encoding() {
        let mut cpu = Avr::new();
        cpu.flash[0] = 0x94;
        cpu.flash[1] = 0x91;
        cpu.flash[100] = 0xAB;
        cpu.r[30] = 100;
        cpu.r[31] = 0;
        cpu.set_pc(0);
        let mut bus = MockBus::new();
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.r[25], 0xAB);
    }

    #[test]
    fn unknown_opcode_decode_error() {
        let mut cpu = Avr::new();
        cpu.load_words(0, &[0xFFFF]);
        let mut bus = MockBus::new();
        let err = cpu
            .step(&mut bus, &[], &SimulationConfig::default())
            .unwrap_err();
        assert!(matches!(err, SimulationError::DecodeError(0)));
    }
    #[test]
    fn timer0_overflow_pends_with_arduino_prescale() {
        let mut cpu = Avr::new();
        cpu.tccr0a = 0x03;
        cpu.tccr0b = 0x03;
        cpu.timsk0 = TIMSK_TOIE0;
        cpu.tick_timer0(16384);
        assert!(cpu.pending_irq & (1 << VEC_TIMER0_OVF) != 0);
    }

    #[test]
    fn usart_udre_tx_never_blocks() {
        let mut cpu = Avr::new();
        let mut bus = MockBus::new();
        for b in b"Hi" {
            cpu.data_write(0xC6, *b, &mut bus).unwrap();
            assert_ne!(cpu.data_read(0xC0, &bus).unwrap() & UCSRA_UDRE, 0);
        }
        assert_eq!(cpu.serial_tx, b"Hi");
    }

    #[test]
    fn hand_blink_toggles_portb5_and_serial() {
        let mut cpu = Avr::new();
        cpu.load_words(0, &[0x9A2D, 0x982D, 0xE508, 0x9300, 0x00C6, 0xCFFA]);
        let mut bus = MockBus::new();
        let cfg = SimulationConfig::default();
        let mut hi = false;
        let mut lo = false;
        for _ in 0..200 {
            cpu.step(&mut bus, &[], &cfg).unwrap();
            if cpu.portb() & 0x20 != 0 {
                hi = true;
            } else {
                lo = true;
            }
            if hi && lo && cpu.serial_tx.contains(&b'X') {
                break;
            }
        }
        assert!(hi && lo);
        assert!(cpu.serial_tx.contains(&b'X'));
    }
}
