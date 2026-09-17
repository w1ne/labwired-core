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

/// Where the chip descriptor maps the bus-side mirror of the CPU's IO
/// registers: data-space address `a` is mirrored at `AVR_IO_MIRROR_BASE + a`.
pub const AVR_IO_MIRROR_BASE: u64 = 0x0001_0000;
/// `PINB` data-space address; `DDRB`/`PORTB` follow it.
pub const AVR_PINB: u16 = 0x0023;
/// `PINC` data-space address; `DDRC`/`PORTC` follow it.
pub const AVR_PINC: u16 = 0x0026;
/// `PIND` data-space address; `DDRD`/`PORTD` follow it.
pub const AVR_PIND: u16 = 0x0029;
/// Bus base of the `avr_adc` input window: two bytes of millivolts per channel.
pub const AVR_ADC_INPUT_BASE: u64 = 0x0001_0030;
/// AVcc on the 5 V boards this part is modelled for, and the internal bandgap.
const AVR_AVCC_MV: u32 = 5000;
const AVR_BANDGAP_MV: u32 = 1100;

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

    fn data_read(&self, addr: u16, bus: &dyn Bus) -> SimResult<u8> {
        match addr {
            0x0000..=0x001F => Ok(self.r[addr as usize]),
            // PINB/PINC/PIND — the registers on this part whose value the
            // OUTSIDE WORLD moves, so they are the ones the IO shadow cannot
            // answer alone.
            //
            // `self.io` only ever holds what firmware wrote, and nothing
            // firmware writes lands in PINx (a write there toggles PORTx). So
            // reading the shadow made `digitalRead` on an input pin return 0
            // forever: a `board_io` button attached to a port would drive its
            // level into the bus-side model and the sketch would never see the
            // press.
            //
            // Composed here rather than taken wholesale from the bus so each
            // half comes from the register that owns it: bits the firmware
            // drives (DDR set) read back its own PORT latch — the real chip's
            // behaviour, and what makes "set it, then confirm it" work — while
            // bits left as inputs take the level the bus-side port model
            // holds. Only the input half of that model's answer is consulted,
            // so the two copies of PORT cannot disagree here.
            //
            // Each port is PINx/DDRx/PORTx at base, base+1, base+2 (PORTB 0x23,
            // PORTC 0x26, PORTD 0x29), mirrored to the same offsets in the
            // high window by `data_write`.
            AVR_PINB | AVR_PINC | AVR_PIND => {
                let ddr = self.io[(addr + 1 - 0x20) as usize];
                let port = self.io[(addr + 2 - 0x20) as usize];
                // Propagated, not discarded: a chip yaml that maps no window
                // for this port has no input state for this register at all,
                // and a swallowed error would answer with a fabricated low — a
                // released button reading as pressed forever, green.
                let external = bus.read_u8(AVR_IO_MIRROR_BASE + u64::from(addr))?;
                Ok((port & ddr) | (external & !ddr))
            }
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
                // ADSC (bit 6): write 1 starts a conversion. It completes
                // immediately (no conversion time is modelled), converting the
                // millivolts the bus-side `avr_adc` model holds for the channel
                // ADMUX selects, so a potentiometer on A0 moves analogRead(A0).
                const ADSC: u8 = 1 << 6;
                const ADIF: u8 = 1 << 4;
                const ADEN: u8 = 1 << 7;
                self.adcsra = value;
                if value & ADEN != 0 && value & ADSC != 0 {
                    let code = self.adc_convert(bus)?;
                    let adlar = self.admux & (1 << 5) != 0;
                    if adlar {
                        self.adch = (code >> 2) as u8;
                        self.adcl = ((code & 0x03) << 6) as u8;
                    } else {
                        self.adcl = (code & 0xFF) as u8;
                        self.adch = (code >> 8) as u8;
                    }
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
                // PORTB/PORTC/PORTD (0x23..0x2B): mirror to the high bus window
                // so the bus-side port models see DDR and PORT — `--watch-gpio
                // portb:N`, a board_io LED, and co-simulation's
                // `board.gpio.<pad>` / `board.gpio_output.<pad>` read them
                // there (flash@0 swallows low-address bus writes).
                if (AVR_PINB..=AVR_PIND + 2).contains(&addr) {
                    // High-window mirror is best-effort (must not fail IN/OUT).
                    let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + addr as u64, value);
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

    /// One 10-bit conversion of the channel ADMUX selects.
    ///
    /// MUX 0..7 are the input pins, whose millivolts come from the bus-side
    /// `avr_adc` model; 0x0E is the 1.1 V bandgap and 0x0F is GND. The other mux
    /// codes (the temperature sensor at 0x08) are not modelled and read 0.
    /// REFS selects the reference: 11 is the internal 1.1 V, anything else is
    /// taken as 5 V AVcc (AREF is not a modelled pin).
    fn adc_convert(&self, bus: &dyn Bus) -> SimResult<u16> {
        let mux = self.admux & 0x0F;
        let input_mv: u32 = match mux {
            0..=7 => {
                let base = AVR_ADC_INPUT_BASE + u64::from(mux) * 2;
                let lo = bus.read_u8(base)?;
                let hi = bus.read_u8(base + 1)?;
                u32::from(u16::from_le_bytes([lo, hi]))
            }
            0x0E => AVR_BANDGAP_MV,
            _ => 0,
        };
        let reference_mv = if self.admux >> 6 == 0b11 {
            AVR_BANDGAP_MV
        } else {
            AVR_AVCC_MV
        };
        Ok((input_mv * 1024 / reference_mv).min(1023) as u16)
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
        let next = pc.wrapping_add(2);

        // Every exec_* function is #[inline(always)] and has exactly this one
        // call site, so this dispatch chain compiles down to the same
        // machine code as the pre-split single-function decoder.
        if self.exec_system_a(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_branch_a(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_system_b(bus, op, pc, next)?.is_some() {
            return Ok(());
        }

        if self.exec_branch_b(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_load_store_a(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_system_c(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_bitops_a(bus, op, next)?.is_some() {
            return Ok(());
        }
        if self.exec_load_store_b(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_load_store_c(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_arith_a(op, next)?.is_some() {
            return Ok(());
        }
        if self.exec_mul_a(op, next)?.is_some() {
            return Ok(());
        }
        if self.exec_branch_c(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_arith_b(op, next)?.is_some() {
            return Ok(());
        }
        if self.exec_bitops_b(op, next)?.is_some() {
            return Ok(());
        }
        if self.exec_arith_c(op, next)?.is_some() {
            return Ok(());
        }
        if self.exec_branch_d(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_load_store_d(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_arith_d(op, next)?.is_some() {
            return Ok(());
        }
        if self.exec_load_store_e(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_branch_e(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_load_store_f(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_branch_f(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_branch_g(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_arith_e(op, next)?.is_some() {
            return Ok(());
        }
        if self.exec_load_store_g(bus, op, pc, next)?.is_some() {
            return Ok(());
        }
        if self.exec_mul_b(op, next)?.is_some() {
            return Ok(());
        }

        Err(SimulationError::DecodeError(pc as u64))
    }
}

impl Cpu for Avr {
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    /// Every step charges the datasheet's clock cycles to `cycles`, and Timer0
    /// (`millis()`, `delay()`) counts exactly those, so they are real time.
    fn instruction_cycles_are_time(&self) -> bool {
        true
    }

    fn clock_cycles(&self) -> u64 {
        self.cycles
    }

    /// `CALL`, `RET`, `RETI` and an interrupt entry take 4 clock cycles, the
    /// longest step this core models.
    fn max_step_cycles(&self) -> u32 {
        4
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

#[path = "avr/exec/mod.rs"]
mod exec;

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

    /// `digitalRead` compiles to `IN Rd, PINB` (or an LDS of 0x23). The level a
    /// button holds lives in the bus-side `portb` model, not in the CPU's IO
    /// shadow, so the read has to consult the bus or the sketch never sees the
    /// press — the button attaches, the stimulus reports success, and nothing
    /// moves.
    #[test]
    fn pinb_read_sees_an_externally_driven_input_bit() {
        let mut cpu = Avr::new();
        // IN R16, PINB(io3)
        cpu.load_words(0, &[0xB103, 0xCFFF]);
        let mut bus = MockBus::new();
        let cfg = SimulationConfig::default();

        // PB2 is an input (DDRB bit clear) and the outside world holds it high.
        // The mirror window is where `data_write` already pushes DDRB/PORTB.
        bus.write_u8(0x0001_0023, 1 << 2).unwrap();

        cpu.set_pc(0);
        cpu.step(&mut bus, &[], &cfg).unwrap();
        assert_eq!(
            cpu.r[16] & (1 << 2),
            1 << 2,
            "PINB must show the pressed level"
        );
    }

    /// The other half of the same rule: a pin the firmware drives reads back its
    /// own PORTB latch rather than the external world's level.
    #[test]
    fn pinb_read_prefers_the_port_latch_on_an_output_pin() {
        let mut cpu = Avr::new();
        // LDI R16,0x20; OUT DDRB(io4),R16; OUT PORTB(io5),R16; IN R17,PINB(io3)
        cpu.load_words(0, &[0xE200, 0xB904, 0xB905, 0xB113, 0xCFFF]);
        let mut bus = MockBus::new();
        let cfg = SimulationConfig::default();
        // External source pulling PB5 low — the driver must win.
        bus.write_u8(0x0001_0023, 0).unwrap();
        cpu.set_pc(0);
        for _ in 0..4 {
            cpu.step(&mut bus, &[], &cfg).unwrap();
        }
        assert_eq!(cpu.r[17] & 0x20, 0x20, "driven output reads back its latch");
    }

    /// PORTD is wired the way PORTB is: DDRD/PORTD writes reach the mirror
    /// window, and a PIND read takes input bits from the bus-side model while
    /// output bits read back the latch. `CapacitiveSensor` reads its receive
    /// pin (D2 = PD2) through exactly this register, and the touch circuit
    /// also writes an input level for its send pin (D4 = PD4), which the
    /// firmware drives: that level must not show through the latch.
    #[test]
    fn pind_reads_external_inputs_and_the_latch_of_driven_pads() {
        let mut cpu = Avr::new();
        // LDI R16,0x10; OUT DDRD(io0x0A),R16; IN R17,PIND(io0x09);
        // OUT PORTD(io0x0B),R16; IN R18,PIND
        cpu.load_words(0, &[0xE100, 0xB90A, 0xB119, 0xB90B, 0xB129, 0xCFFF]);
        let mut bus = MockBus::new();
        let cfg = SimulationConfig::default();
        // The outside world holds PD2 and PD4 high.
        bus.write_u8(AVR_IO_MIRROR_BASE + u64::from(AVR_PIND), 0x14)
            .unwrap();
        cpu.set_pc(0);
        for _ in 0..5 {
            cpu.step(&mut bus, &[], &cfg).unwrap();
        }
        assert_eq!(
            bus.read_u8(AVR_IO_MIRROR_BASE + 0x2A).unwrap(),
            0x10,
            "DDRD mirrored"
        );
        assert_eq!(
            bus.read_u8(AVR_IO_MIRROR_BASE + 0x2B).unwrap(),
            0x10,
            "PORTD mirrored"
        );
        assert_eq!(
            cpu.r[17], 0x04,
            "PD4 drives its low latch over the external high; PD2 reads the outside"
        );
        assert_eq!(cpu.r[18], 0x14, "PD4 now drives high");
    }

    /// `ADIW`/`SBIW` set C, V and S per the AVR instruction set. Arduino's
    /// Timer0 ISR propagates `timer0_millis` with `ADIW r24,1; ADC r26,r1`, so a
    /// carry left over from an earlier `CPI` used to be added into the top
    /// half of `millis()` on every interrupt.
    #[test]
    fn adiw_and_sbiw_set_carry_overflow_and_sign() {
        let cfg = SimulationConfig::default();
        let mut bus = MockBus::new();
        let mut run = |r24: u8, r25: u8, op: u16, sreg: u8| {
            let mut cpu = Avr::new();
            cpu.load_words(0, &[op, 0xCFFF]);
            cpu.r[24] = r24;
            cpu.r[25] = r25;
            cpu.sreg = sreg;
            cpu.set_pc(0);
            cpu.step(&mut bus, &[], &cfg).unwrap();
            (u16::from_le_bytes([cpu.r[24], cpu.r[25]]), cpu.sreg & 0x1F)
        };
        const C: u8 = 0x01;
        const Z: u8 = 0x02;
        const N: u8 = 0x04;
        const V: u8 = 0x08;
        const S: u8 = 0x10;
        // ADIW r24,1 (0x9601)
        assert_eq!(
            run(0x34, 0x12, 0x9601, C),
            (0x1235, 0),
            "a stale C is cleared"
        );
        assert_eq!(
            run(0xFF, 0xFF, 0x9601, 0),
            (0x0000, C | Z),
            "carry out of 16 bits"
        );
        assert_eq!(
            run(0xFF, 0x7F, 0x9601, 0),
            (0x8000, N | V),
            "signed overflow"
        );
        // SBIW r24,1 (0x9701)
        assert_eq!(run(0x00, 0x00, 0x9701, 0), (0xFFFF, C | N | S), "borrow");
        assert_eq!(
            run(0x00, 0x80, 0x9701, 0),
            (0x7FFF, V | S),
            "signed overflow"
        );
        assert_eq!(
            run(0x02, 0x00, 0x9701, C),
            (0x0001, 0),
            "a stale C is cleared"
        );
    }

    /// A conversion reads the selected channel's millivolts from the bus-side
    /// `avr_adc` window, honours ADLAR, and answers the bandgap mux code
    /// without touching the bus.
    #[test]
    fn adc_converts_the_selected_channel_and_left_adjusts() {
        let mut cpu = Avr::new();
        let mut bus = MockBus::new();
        let cfg = SimulationConfig::default();
        // 2000 mV on ADC2: 2000 * 1024 / 5000 = 409 = 0b01_1001_1001.
        bus.write_u8(AVR_ADC_INPUT_BASE + 4, (2000u16 & 0xFF) as u8)
            .unwrap();
        bus.write_u8(AVR_ADC_INPUT_BASE + 5, (2000u16 >> 8) as u8)
            .unwrap();
        // LDI R16,0x42 (REFS=AVcc, MUX=2); STS ADMUX,R16; LDI R17,0xC0 (ADEN|ADSC); STS ADCSRA,R17
        cpu.load_words(0, &[0xE402, 0x9300, 0x007C, 0xEC10, 0x9310, 0x007A, 0xCFFF]);
        cpu.set_pc(0);
        for _ in 0..4 {
            cpu.step(&mut bus, &[], &cfg).unwrap();
        }
        assert_eq!(u16::from(cpu.adch) << 8 | u16::from(cpu.adcl), 409);
        assert_eq!(cpu.adcsra & (1 << 6), 0, "ADSC clears when done");
        assert_ne!(cpu.adcsra & (1 << 4), 0, "ADIF sets when done");

        // Same input, ADLAR set (0x62): 409 << 6 split across ADCH:ADCL.
        cpu.load_words(0, &[0xE602, 0x9300, 0x007C, 0xEC10, 0x9310, 0x007A, 0xCFFF]);
        cpu.set_pc(0);
        for _ in 0..4 {
            cpu.step(&mut bus, &[], &cfg).unwrap();
        }
        assert_eq!(u16::from(cpu.adch) << 8 | u16::from(cpu.adcl), 409 << 6);

        // MUX=0x0E is the 1.1 V bandgap: 1100 * 1024 / 5000 = 225.
        cpu.load_words(0, &[0xE40E, 0x9300, 0x007C, 0xEC10, 0x9310, 0x007A, 0xCFFF]);
        cpu.set_pc(0);
        for _ in 0..4 {
            cpu.step(&mut bus, &[], &cfg).unwrap();
        }
        assert_eq!(u16::from(cpu.adch) << 8 | u16::from(cpu.adcl), 225);
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
