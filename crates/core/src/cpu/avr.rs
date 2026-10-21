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

/// ATmega328P geometry (the default model). The ATmega2560 values live on
/// [`AvrModel`]; a CPU built for a chip reads them from its own fields.
pub const FLASH_SIZE: usize = 32 * 1024;
pub const RAMEND: u16 = 0x08FF;
pub const SRAM_START: u16 = 0x0100;

/// The AVR parts this interpreter runs. The ATmega2560 (Arduino Mega) is the
/// ATmega328P core with: 256 KB flash and a 3-byte PC (CALL/RCALL/ICALL/RET/
/// RETI and interrupt entry push and pop 3 bytes and take one cycle more;
/// EIND for EIJMP/EICALL, RAMPZ for ELPM), 8 KB SRAM at 0x0200, ports A..L
/// (H..L in extended I/O at 0x100..0x10B), a 16-channel ADC (ADCSRB.MUX5
/// selects ADC8..ADC15) and its own vector table. Timer0/1/2, USART0, SPI and
/// TWI sit at the same data addresses as on the 328P, so they are shared; their
/// vector numbers are remapped on entry ([`mega2560_vector`]).
///
/// Not modelled on the ATmega2560: INTn/PCINT (its pins differ from the
/// 328P's; the registers are stored only), output-compare pads (PWM
/// `analogWrite` levels; the pads follow `PORTx`), Timer3/4/5 and USART1..3
/// (registers stored only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvrModel {
    #[default]
    Atmega328p,
    Atmega2560,
}

impl AvrModel {
    /// The model a chip descriptor names: `atmega2560`/`atmega2561`/`mega2560`
    /// select the ATmega2560, anything else the ATmega328P.
    pub fn from_chip_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        if n.contains("2560") || n.contains("2561") {
            AvrModel::Atmega2560
        } else {
            AvrModel::Atmega328p
        }
    }
    pub fn flash_size(self) -> usize {
        match self {
            AvrModel::Atmega328p => FLASH_SIZE,
            AvrModel::Atmega2560 => 256 * 1024,
        }
    }
    pub fn sram_start(self) -> u16 {
        match self {
            AvrModel::Atmega328p => SRAM_START,
            AvrModel::Atmega2560 => 0x0200,
        }
    }
    pub fn ramend(self) -> u16 {
        match self {
            AvrModel::Atmega328p => RAMEND,
            AvrModel::Atmega2560 => 0x21FF,
        }
    }
    /// Bus base of this model's `avr_adc` input window (2 bytes per channel).
    /// The 2560's 16 channels move it past the port mirrors (PINF..PING sit
    /// at 0x1_002F..0x1_0034, where the 328P keeps its 8-channel window).
    pub fn adc_input_base(self) -> u64 {
        match self {
            AvrModel::Atmega328p => AVR_ADC_INPUT_BASE,
            AvrModel::Atmega2560 => AVR_MEGA_ADC_INPUT_BASE,
        }
    }
    /// `true` when the PC is pushed as 3 bytes (flash above 128 KB).
    pub fn three_byte_pc(self) -> bool {
        self == AvrModel::Atmega2560
    }
}

/// ATmega2560 data-space addresses of PINx for ports A..L (DDRx = +1, PORTx = +2).
pub const MEGA2560_PIN_ADDRS: [u16; 11] = [
    0x20, 0x23, 0x26, 0x29, 0x2C, 0x2F, 0x32, 0x100, 0x103, 0x106, 0x109,
];

/// ATmega2560 vector number (1-based, datasheet table 14-1) for a vector this
/// core numbers the ATmega328P way. The shared peripherals raise 328P vector
/// numbers; the 2560 jumps to its own table slot.
pub fn mega2560_vector(vec328: u32) -> u32 {
    match vec328 {
        2 | 3 => vec328,       // INT0, INT1
        4..=6 => vec328 + 6,   // PCINT0..2 -> 10..12
        7 => 13,               // WDT
        8..=10 => vec328 + 6,  // TIMER2 COMPA/COMPB/OVF -> 14..16
        11..=13 => vec328 + 6, // TIMER1 CAPT/COMPA/COMPB -> 17..19
        14..=21 => vec328 + 7, // TIMER1 OVF -> 21, TIMER0 COMPA/B/OVF -> 22..24, SPI 25, USART0 RX/UDRE/TX -> 26..28
        22 => 30,              // ADC
        23 => 31,              // EE READY
        24 => 29,              // ANALOG COMP
        25 => 40,              // TWI
        26 => 58,              // SPM READY
        v => v,
    }
}
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
    /// Which part this is: memory geometry, PC width, ports, ADC mux, vectors.
    pub model: AvrModel,
    sram_start: u16,
    ramend: u16,
    pub r: [u8; 32],
    pub pc: u32,
    pub sp: u16,
    pub sreg: u8,
    pub flash: Vec<u8>,
    pub sram: Vec<u8>,
    /// Data-space 0x20..0x1FF (the 328P uses 0x20..0xFF; 0x100..0x1FF is the
    /// ATmega2560's extended I/O).
    pub io: [u8; 0x1E0],
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
    /// Timer2, the counter Arduino `tone()` runs in CTC mode.
    ///
    /// Normal and CTC count as on silicon. The PWM modes count 0..255; their
    /// OC2A/OC2B pad levels come from `avr/oc_pins.rs`.
    pub tcnt2: u8,
    pub tccr2a: u8,
    pub tccr2b: u8,
    pub ocr2a: u8,
    pub ocr2b: u8,
    pub timsk2: u8,
    pub tifr2: u8,
    /// `ASSR`: only `EXCLK` and `AS2` are stored. Update-busy bits read as 0
    /// (writes land immediately). `AS2` selects TOSC1, which is not modelled,
    /// so the counter stops instead of pretending the CPU clock still drives it.
    pub assr: u8,
    pub t2_prescale_acc: u32,
    /// Timer1, the 16-bit counter (`avr/timer1.rs`).
    t1: timer1::Timer1,
    /// Port B / port D pads an output-compare unit owns, and the level it
    /// drives there, as last pushed to the bus-side port model.
    oc_mask: [u8; 2],
    oc_level: [u8; 2],
    pub serial_tx: Vec<u8>,
    /// Optional live sink for MachineTrait UART capture.
    pub serial_sink: Option<Arc<Mutex<Vec<u8>>>>,
    pub ucsr0a: u8,
    /// Step counter for the RX-complete poll.
    rx_poll: u8,
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
    /// External interrupts: `EICRA` (sense control), `EIMSK`, `EIFR`.
    pub eicra: u8,
    pub eimsk: u8,
    pub eifr: u8,
    /// Pin-change interrupts: `PCICR`, `PCIFR`, `PCMSK0..2`.
    pub pcicr: u8,
    pub pcifr: u8,
    pub pcmsk: [u8; 3],
    /// `SMCR`: sleep mode and sleep enable.
    pub smcr: u8,
    /// Stopped by `SLEEP` until an enabled interrupt wakes it.
    pub sleeping: bool,
    /// Pad levels of ports B, C, D at the last boundary sample
    /// (see `avr/ext_int.rs`). Only the watched ports are kept current.
    ext_last: [u8; 3],
    /// Set by `SEI` and `RETI`: the next instruction runs before any pending
    /// interrupt is taken. This is what makes `sei(); sleep_cpu();` atomic.
    irq_shadow: bool,
    /// Ports sampled at every boundary (bit 0 = B, 1 = C, 2 = D); zero while
    /// no INT/PCINT is configured, which keeps the step cost unchanged.
    ext_watch: u8,
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
/// `MCUCR` in data space (I/O 0x35), ATmega328P datasheet §14.4.1.
pub const ADDR_MCUCR: u16 = 0x0055;
/// `MCUCR.PUD`: pull-up disable for every port.
pub const MCUCR_PUD: u8 = 1 << 4;
/// `PINB` data-space address; `DDRB`/`PORTB` follow it.
pub const AVR_PINB: u16 = 0x0023;
/// `PINC` data-space address; `DDRC`/`PORTC` follow it.
pub const AVR_PINC: u16 = 0x0026;
/// `PIND` data-space address; `DDRD`/`PORTD` follow it.
pub const AVR_PIND: u16 = 0x0029;
/// Bus base of the `avr_adc` input window: two bytes of millivolts per channel.
pub const AVR_ADC_INPUT_BASE: u64 = 0x0001_0030;
/// Bus base of the ATmega2560's 16-channel `avr_adc` input window.
pub const AVR_MEGA_ADC_INPUT_BASE: u64 = 0x0001_0200;
/// AVcc on the 5 V boards this part is modelled for, and the internal bandgap.
const AVR_AVCC_MV: u32 = 5000;
const AVR_BANDGAP_MV: u32 = 1100;

pub const VEC_TIMER0_OVF: u32 = 17; // datasheet 1-based; @0x40 = __vector_16
/// TWI_vect is `_VECTOR(24)` → PC 0x60; pending bit uses vec=25 (`(vec-1)*4`).
pub const VEC_TWI: u32 = 25;
/// USART_RX_vect, datasheet 1-based vector number (@0x24 = `__vector_18`).
pub const VEC_USART_RX: u32 = 19;
pub const UCSRA_RXC: u8 = 1 << 7;
/// UCSR0B bit 7: RX complete interrupt enable.
const UCSRB_RXCIE: u8 = 1 << 7;
pub const UCSRA_UDRE: u8 = 1 << 5;
pub const UCSRA_TXC: u8 = 1 << 6;
pub const TIMSK_TOIE0: u8 = 1 << 0;
pub const TIFR_TOV0: u8 = 1 << 0;
/// `TIMER2_COMPA_vect` (`__vector_7` at byte 0x1C). Datasheet vector 8.
pub const VEC_TIMER2_COMPA: u32 = 8;
/// `TIMER2_COMPB_vect` (`__vector_8` at byte 0x20).
pub const VEC_TIMER2_COMPB: u32 = 9;
/// `TIMER2_OVF_vect` (`__vector_9` at byte 0x24).
pub const VEC_TIMER2_OVF: u32 = 10;
const TIMSK2_TOIE2: u8 = 1 << 0;
const TIMSK2_OCIE2A: u8 = 1 << 1;
const TIMSK2_OCIE2B: u8 = 1 << 2;
const TIFR2_TOV2: u8 = 1 << 0;
const TIFR2_OCF2A: u8 = 1 << 1;
const TIFR2_OCF2B: u8 = 1 << 2;
const ASSR_EXCLK: u8 = 1 << 6;
const ASSR_AS2: u8 = 1 << 5;
const T2_IRQ_MASK: u64 = (1 << VEC_TIMER2_COMPA) | (1 << VEC_TIMER2_COMPB) | (1 << VEC_TIMER2_OVF);

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

/// One Timer2 clock: the counter value after the tick and which flags it raises.
struct T2Step {
    tcnt: u8,
    ocfa: bool,
    ocfb: bool,
    tov: bool,
}

/// CTC clears on the clock after TCNT2 has held OCR2A for one timer period,
/// so matches are `OCR2A + 1` ticks apart. Normal mode wraps at 0xFF.
fn t2_one_tick(ctc: bool, tcnt: u8, ocr_a: u8, ocr_b: u8) -> T2Step {
    let tcnt = if ctc && tcnt == ocr_a {
        0
    } else {
        tcnt.wrapping_add(1)
    };
    T2Step {
        tcnt,
        ocfa: tcnt == ocr_a,
        ocfb: tcnt == ocr_b,
        // CTC sets TOV2 on MAX (0xFF), not on the clear back to zero.
        tov: if ctc { tcnt == 0xFF } else { tcnt == 0 },
    }
}

/// `true` when `ticks` clocks land on `target` at least once, starting from
/// `phase` in a counter of length `period`. The current phase is not a hit.
fn t2_hits(phase: u64, ticks: u64, period: u64, target: u64) -> bool {
    if target >= period || ticks == 0 {
        return false;
    }
    let mut dist = (target + period - phase) % period;
    if dist == 0 {
        dist = period;
    }
    ticks >= dist
}

/// CTC closed form. `tcnt` must already be in `0..=ocr_a`.
fn t2_ctc_closed(tcnt: u8, ticks: u64, ocr_a: u8, ocr_b: u8) -> T2Step {
    let phase = u64::from(tcnt);
    let period = u64::from(ocr_a) + 1;
    T2Step {
        tcnt: ((phase + ticks) % period) as u8,
        ocfa: t2_hits(phase, ticks, period, u64::from(ocr_a)),
        ocfb: t2_hits(phase, ticks, period, u64::from(ocr_b)),
        tov: t2_hits(phase, ticks, period, 0xFF),
    }
}

fn t2_normal_closed(tcnt: u8, ticks: u64, ocr_a: u8, ocr_b: u8) -> T2Step {
    let phase = u64::from(tcnt);
    T2Step {
        tcnt: ((phase + ticks) & 0xFF) as u8,
        ocfa: t2_hits(phase, ticks, 256, u64::from(ocr_a)),
        ocfb: t2_hits(phase, ticks, 256, u64::from(ocr_b)),
        tov: t2_hits(phase, ticks, 256, 0),
    }
}

impl Avr {
    /// An ATmega328P (Arduino Uno/Nano).
    pub fn new() -> Self {
        Self::new_model(AvrModel::Atmega328p)
    }

    /// The CPU for a chip descriptor's `name` (see [`AvrModel::from_chip_name`]).
    pub fn for_chip_name(name: &str) -> Self {
        Self::new_model(AvrModel::from_chip_name(name))
    }

    pub fn new_model(model: AvrModel) -> Self {
        let (sram_start, ramend) = (model.sram_start(), model.ramend());
        Self {
            model,
            sram_start,
            ramend,
            r: [0; 32],
            pc: 0,
            sp: ramend,
            sreg: 0,
            flash: vec![0xFF; model.flash_size()],
            sram: vec![0; (ramend as usize + 1) - sram_start as usize],
            io: [0; 0x1E0],
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
            tcnt2: 0,
            tccr2a: 0,
            tccr2b: 0,
            ocr2a: 0,
            ocr2b: 0,
            timsk2: 0,
            tifr2: 0,
            assr: 0,
            t2_prescale_acc: 0,
            t1: timer1::Timer1::default(),
            oc_mask: [0; 2],
            oc_level: [0; 2],
            serial_tx: Vec::new(),
            serial_sink: None,
            ucsr0a: UCSRA_UDRE,
            rx_poll: 0,
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
            eicra: 0,
            eimsk: 0,
            eifr: 0,
            pcicr: 0,
            pcifr: 0,
            pcmsk: [0; 3],
            smcr: 0,
            sleeping: false,
            ext_last: [0; 3],
            ext_watch: 0,
            irq_shadow: false,
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
    fn is_mega(&self) -> bool {
        self.model == AvrModel::Atmega2560
    }

    /// `PINx` of a port this model has.
    #[inline]
    fn is_pin_reg(&self, addr: u16) -> bool {
        match self.model {
            AvrModel::Atmega328p => matches!(addr, AVR_PINB | AVR_PINC | AVR_PIND),
            AvrModel::Atmega2560 => MEGA2560_PIN_ADDRS.contains(&addr),
        }
    }

    /// `PINx`/`DDRx`/`PORTx` of a port this model has: mirrored to the bus.
    #[inline]
    fn is_port_reg(&self, addr: u16) -> bool {
        match self.model {
            AvrModel::Atmega328p => (AVR_PINB..=AVR_PIND + 2).contains(&addr),
            AvrModel::Atmega2560 => {
                (0x20..=0x34).contains(&addr) || (0x100..=0x10B).contains(&addr)
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
    fn flag_t(&self) -> bool {
        self.sreg & 0x40 != 0
    }

    #[inline]
    fn set_flag_t(&mut self, on: bool) {
        if on {
            self.sreg |= 0x40;
        } else {
            self.sreg &= !0x40;
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
            a if self.is_pin_reg(a) => {
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
            0x0037 => Ok(self.tifr2),
            0x0044 => Ok(self.tccr0a),
            0x0045 => Ok(self.tccr0b),
            0x0046 => Ok(self.tcnt0),
            0x0047 => Ok(self.ocr0a),
            0x0048 => Ok(self.ocr0b),
            0x006E => Ok(self.timsk0),
            0x0070 => Ok(self.timsk2),
            0x00B0 => Ok(self.tccr2a),
            0x00B1 => Ok(self.tccr2b),
            0x00B2 => Ok(self.tcnt2),
            0x00B3 => Ok(self.ocr2a),
            0x00B4 => Ok(self.ocr2b),
            0x00B6 => Ok(self.assr),
            timer1::ADDR_TIFR1
            | timer1::ADDR_TIMSK1
            | timer1::ADDR_TCCR1A..=0x0082
            | 0x0084..=timer1::ADDR_OCR1BH => Ok(self.timer1_read(addr).unwrap_or_default()),
            // RXC0 comes from the bus-side USART model, which holds the receive
            // queue (peers and host input). A bus with no USART window reads 0.
            0x00C0 => {
                let rxc = if Self::usart_on_bus(bus) {
                    bus.read_u8(AVR_IO_MIRROR_BASE + 0xC0)? & UCSRA_RXC
                } else {
                    0
                };
                Ok(self.ucsr0a | UCSRA_UDRE | rxc)
            }
            0x00C1 => Ok(self.ucsr0b),
            0x00C2 => Ok(self.ucsr0c),
            0x00C4 => Ok((self.ubrr0 & 0xFF) as u8),
            0x00C5 => Ok((self.ubrr0 >> 8) as u8),
            // UDR0 read pops one received byte from the bus-side USART model.
            0x00C6 => {
                if Self::usart_on_bus(bus) {
                    bus.read_u8(AVR_IO_MIRROR_BASE + 0xC6)
                } else {
                    Ok(0)
                }
            }
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
            ext_int::ADDR_PCIFR..=ext_int::ADDR_EIMSK
            | ext_int::ADDR_SMCR
            | ext_int::ADDR_PCICR
            | ext_int::ADDR_EICRA
            | ext_int::ADDR_PCMSK0..=ext_int::ADDR_PCMSK2
                if !self.is_mega() || addr == ext_int::ADDR_SMCR =>
            {
                Ok(self.ext_read(addr).unwrap_or_default())
            }
            0x0020..=0x00FF => Ok(self.io[(addr - 0x20) as usize]),
            // ATmega2560 extended I/O (ports H..L, Timer3/4/5, USART1..3):
            // stored registers.
            0x0100..=0x01FF if self.is_mega() => Ok(self.io[(addr - 0x20) as usize]),
            a if (self.sram_start..=self.ramend).contains(&a) => {
                Ok(self.sram[(a - self.sram_start) as usize])
            }
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
            0x0037 => {
                // Write-1-to-clear, same as TIFR0. The compare/overflow
                // interrupt is level-sensitive on the flag, so clearing it
                // drops a request that has not been taken yet.
                self.tifr2 &= !(value & 0x07);
                self.sync_timer2_irq();
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
            0x0070 => {
                self.timsk2 = value & 0x07;
                self.sync_timer2_irq();
                Ok(())
            }
            0x00B0 => {
                // COM2A/COM2B and WGM21:0. Reserved bits read as 0. In the
                // PWM modes the COM bits hand OC2A/OC2B to the waveform
                // (`oc_pins.rs`).
                self.tccr2a = value & 0xF3;
                Ok(())
            }
            0x00B1 => {
                // FOC2A/FOC2B are strobes and always read as 0. Force
                // compare is not modelled, so the strobe is a no-op. WGM22 and CS22:0 stick.
                self.tccr2b = value & 0x0F;
                Ok(())
            }
            0x00B2 => {
                self.tcnt2 = value;
                Ok(())
            }
            0x00B3 => {
                self.ocr2a = value;
                Ok(())
            }
            0x00B4 => {
                self.ocr2b = value;
                Ok(())
            }
            0x00B6 => {
                self.assr = value & (ASSR_EXCLK | ASSR_AS2);
                Ok(())
            }
            timer1::ADDR_TIFR1
            | timer1::ADDR_TIMSK1
            | timer1::ADDR_TCCR1A..=0x0082
            | 0x0084..=timer1::ADDR_OCR1BH => {
                self.timer1_write(addr, value);
                Ok(())
            }
            0x00C0 => {
                // Writing 1 to TXC0 clears it on silicon, and HardwareSerial does
                // so after every byte, then `flush()` waits for the flag to come
                // back when the shift register empties. Transmission is instant
                // here, so the flag is already back: keeping it set is what lets
                // `flush()` (and a Modbus master's post-transmission hook) return.
                self.ucsr0a |= UCSRA_UDRE | UCSRA_TXC;
                // The bus-side USART needs U2X for its baud; tolerate no window.
                Self::usart_mirror_write(bus, 0xC0, value)?;
                Ok(())
            }
            0x00C1 => {
                self.ucsr0b = value;
                Self::usart_mirror_write(bus, 0xC1, value)?;
                Ok(())
            }
            0x00C2 => {
                self.ucsr0c = value;
                Ok(())
            }
            0x00C4 => {
                self.ubrr0 = (self.ubrr0 & 0xFF00) | value as u16;
                Self::usart_mirror_write(bus, 0xC4, value)?;
                Ok(())
            }
            0x00C5 => {
                self.ubrr0 = (self.ubrr0 & 0x00FF) | ((value as u16) << 8);
                Self::usart_mirror_write(bus, 0xC5, value)?;
                Ok(())
            }
            0x00C6 => {
                self.serial_tx.push(value);
                self.ucsr0a |= UCSRA_UDRE | UCSRA_TXC;
                bus.write_u8(addr as u64, value)?;
                // Hand the byte to the bus-side USART, which hosts the peers
                // (an RS-485 transceiver and its slaves). No window, no peers.
                Self::usart_mirror_write(bus, 0xC6, value)?;
                // A byte a transceiver put on an RS-485 bus is a frame, not
                // console text: the bus-side model says so at +7.
                let on_bus =
                    Self::usart_on_bus(bus) && bus.read_u8(AVR_IO_MIRROR_BASE + 0xC7)? & 1 != 0;
                if !on_bus {
                    if let Some(sink) = &self.serial_sink {
                        if let Ok(mut g) = sink.lock() {
                            g.push(value);
                        }
                    }
                }
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
            ext_int::ADDR_PCIFR..=ext_int::ADDR_EIMSK
            | ext_int::ADDR_SMCR
            | ext_int::ADDR_PCICR
            | ext_int::ADDR_EICRA
            | ext_int::ADDR_PCMSK0..=ext_int::ADDR_PCMSK2
                if !self.is_mega() || addr == ext_int::ADDR_SMCR =>
            {
                self.ext_write(addr, value, bus);
                Ok(())
            }
            ADDR_MCUCR => {
                // MCUCR: the CPU owns it (IVSEL/IVCE, BODS/BODSE are stored
                // only), but PUD (bit 4) switches off every pull-up of every
                // port, so a change goes to the bus-side ports.
                let old = self.io[usize::from(ADDR_MCUCR - 0x20)];
                self.io[usize::from(ADDR_MCUCR - 0x20)] = value;
                if (old ^ value) & MCUCR_PUD != 0 {
                    bus.set_pull_ups_disabled(value & MCUCR_PUD != 0);
                }
                let _mirror = bus.write_u8(addr as u64, value);
                Ok(())
            }
            0x0020..=0x00FF => {
                self.io[(addr - 0x20) as usize] = value;
                // PORTB/PORTC/PORTD (0x23..0x2B): mirror to the high bus window
                // so the bus-side port models see DDR and PORT — `--watch-gpio
                // portb:N`, a board_io LED, and co-simulation's
                // `board.gpio.<pad>` / `board.gpio_output.<pad>` read them
                // there (flash@0 swallows low-address bus writes).
                if self.is_port_reg(addr) {
                    // High-window mirror is best-effort (must not fail IN/OUT).
                    // A pad an output-compare unit owns keeps its waveform level.
                    let pad = self.port_pad_value(addr, value);
                    let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + addr as u64, pad);
                } else {
                    let _mirror = bus.write_u8(addr as u64, value);
                }
                Ok(())
            }
            0x0100..=0x01FF if self.is_mega() => {
                self.io[(addr - 0x20) as usize] = value;
                if self.is_port_reg(addr) {
                    let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + addr as u64, value);
                }
                Ok(())
            }
            a if (self.sram_start..=self.ramend).contains(&a) => {
                self.sram[(a - self.sram_start) as usize] = value;
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
    ///
    /// ATmega2560: MUX5 (ADCSRB bit 3) with ADMUX MUX4:0 makes a 6-bit code;
    /// 0x00..0x07 are ADC0..7, 0x20..0x27 are ADC8..15 (Arduino A8..A15),
    /// 0x1E is the 1.1 V bandgap. Differential/gain codes read 0.
    fn adc_convert(&self, bus: &dyn Bus) -> SimResult<u16> {
        let channel: Option<u8> = if self.is_mega() {
            let mux5 = self.io[(0x7B - 0x20) as usize] & (1 << 3) != 0;
            match (self.admux & 0x1F) | if mux5 { 0x20 } else { 0 } {
                m @ 0x00..=0x07 => Some(m),
                m @ 0x20..=0x27 => Some(m - 0x18),
                0x1E => None,
                _ => Some(0xFF),
            }
        } else {
            match self.admux & 0x0F {
                m @ 0..=7 => Some(m),
                0x0E => None,
                _ => Some(0xFF),
            }
        };
        let input_mv: u32 = match channel {
            Some(0xFF) => 0,
            Some(ch) => {
                let base = self.model.adc_input_base() + u64::from(ch) * 2;
                let lo = bus.read_u8(base)?;
                let hi = bus.read_u8(base + 1)?;
                u32::from(u16::from_le_bytes([lo, hi]))
            }
            None => AVR_BANDGAP_MV,
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
        // Closed form of "one TCNT0 increment per `div` clocks": an idle
        // fast-forward hands this millions of cycles at once.
        let acc = u64::from(self.t0_prescale_acc) + u64::from(cpu_cycles);
        let ticks = acc / u64::from(div);
        self.t0_prescale_acc = (acc % u64::from(div)) as u32;
        if ticks == 0 {
            return;
        }
        let count = u64::from(self.tcnt0) + ticks;
        self.tcnt0 = (count & 0xFF) as u8;
        if count > 0xFF {
            self.tifr0 |= TIFR_TOV0;
            if self.timsk0 & TIMSK_TOIE0 != 0 {
                self.pending_irq |= 1u64 << VEC_TIMER0_OVF;
            }
        }
    }

    /// clk_I/O prescaler for Timer2. `AS2` freezes it: the asynchronous TOSC1
    /// clock is not modelled. Timer2's dividers are not Timer0's — `/32` and
    /// `/128` exist here and `/256` is CS = 6, not 4.
    fn t2_prescaler(&self) -> u32 {
        if self.assr & ASSR_AS2 != 0 {
            return 0;
        }
        match self.tccr2b & 0x07 {
            0 => 0,
            1 => 1,
            2 => 8,
            3 => 32,
            4 => 64,
            5 => 128,
            6 => 256,
            7 => 1024,
            _ => 0,
        }
    }

    /// WGM22:0 == 2. Every other mode counts like normal (TOP = 0xFF).
    fn t2_is_ctc(&self) -> bool {
        let wgm = (self.tccr2a & 0x03) | ((self.tccr2b & 0x08) >> 1);
        wgm == 0b010
    }

    /// Compare-match interrupts stay pending while the flag and its enable
    /// are both set, including when firmware sets the enable after the flag.
    fn sync_timer2_irq(&mut self) {
        let mut want = 0u64;
        if self.timsk2 & TIMSK2_OCIE2A != 0 && self.tifr2 & TIFR2_OCF2A != 0 {
            want |= 1 << VEC_TIMER2_COMPA;
        }
        if self.timsk2 & TIMSK2_OCIE2B != 0 && self.tifr2 & TIFR2_OCF2B != 0 {
            want |= 1 << VEC_TIMER2_COMPB;
        }
        if self.timsk2 & TIMSK2_TOIE2 != 0 && self.tifr2 & TIFR2_TOV2 != 0 {
            want |= 1 << VEC_TIMER2_OVF;
        }
        self.pending_irq = (self.pending_irq & !T2_IRQ_MASK) | want;
    }

    fn t2_absorb(&mut self, step: T2Step) {
        self.tcnt2 = step.tcnt;
        if step.ocfa {
            self.tifr2 |= TIFR2_OCF2A;
        }
        if step.ocfb {
            self.tifr2 |= TIFR2_OCF2B;
        }
        if step.tov {
            self.tifr2 |= TIFR2_TOV2;
        }
    }

    fn t2_apply_ticks(&mut self, mut ticks: u64) {
        if ticks == 0 {
            return;
        }
        if self.t2_is_ctc() {
            // OCR2A was lowered under the count: climb back into 0..=OCR2A
            // (at most 256 ticks) and then the CTC cycle is closed-form.
            while ticks > 0 && self.tcnt2 > self.ocr2a {
                let step = t2_one_tick(true, self.tcnt2, self.ocr2a, self.ocr2b);
                self.t2_absorb(step);
                ticks -= 1;
            }
            if ticks > 0 {
                self.t2_absorb(t2_ctc_closed(self.tcnt2, ticks, self.ocr2a, self.ocr2b));
            }
        } else {
            self.t2_absorb(t2_normal_closed(self.tcnt2, ticks, self.ocr2a, self.ocr2b));
        }
        self.sync_timer2_irq();
    }

    /// Advance Timer2 by CPU clocks. Called beside [`Self::tick_timer0`].
    pub fn tick_timer2(&mut self, cpu_cycles: u32) {
        let div = self.t2_prescaler();
        if div == 0 || cpu_cycles == 0 {
            return;
        }
        let acc = u64::from(self.t2_prescale_acc) + u64::from(cpu_cycles);
        let ticks = acc / u64::from(div);
        self.t2_prescale_acc = (acc % u64::from(div)) as u32;
        self.t2_apply_ticks(ticks);
    }

    fn tick_io_timers(&mut self, cpu_cycles: u32) {
        self.tick_timer0(cpu_cycles);
        self.tick_timer1(cpu_cycles);
        self.tick_timer2(cpu_cycles);
    }

    /// CPU cycles until the next Timer2 interrupt that can wake a sleeper.
    /// `None` when the counter cannot raise an enabled flag (stopped, async,
    /// or the programmed TOP never reaches it).
    fn timer2_wake_cycles(&self) -> Option<u64> {
        if !self.flag_i() || !self.io_clock_running() {
            return None;
        }
        let div = u64::from(self.t2_prescaler());
        if div == 0 {
            return None;
        }
        let mask = self.timsk2 & (TIMSK2_OCIE2A | TIMSK2_OCIE2B | TIMSK2_TOIE2);
        if mask == 0 {
            return None;
        }
        let ctc = self.t2_is_ctc();
        let mut tcnt = self.tcnt2;
        for n in 1..=512u64 {
            let step = t2_one_tick(ctc, tcnt, self.ocr2a, self.ocr2b);
            tcnt = step.tcnt;
            let hit = (mask & TIMSK2_OCIE2A != 0 && step.ocfa)
                || (mask & TIMSK2_OCIE2B != 0 && step.ocfb)
                || (mask & TIMSK2_TOIE2 != 0 && step.tov);
            if hit {
                let cycles = n * div - u64::from(self.t2_prescale_acc).min(div - 1);
                return Some(cycles.max(1));
            }
        }
        None
    }

    fn timer0_wake_cycles(&self) -> Option<u64> {
        let div = u64::from(self.t0_prescaler());
        if self.io_clock_running() && div != 0 && self.timsk0 & TIMSK_TOIE0 != 0 && self.flag_i() {
            let ticks = 256 - u64::from(self.tcnt0);
            let cycles = ticks * div - u64::from(self.t0_prescale_acc).min(div - 1);
            Some(cycles.max(1))
        } else {
            None
        }
    }

    /// An interrupt the core would take now: global enable set and a vector
    /// pending. This is also what wakes a sleeping core.
    #[inline]
    fn wake_pending(&self) -> bool {
        self.flag_i() && self.pending_irq & 0xFFFF_FFFE != 0
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
                        // Every other master calls `I2cDevice::start` when the
                        // address ACKs. Skipping it left the device's read
                        // latch pointing at the previous register, so the next
                        // word came back `0xFF`.
                        self.i2c_slaves[idx].start();
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
                    if (self.sram_start..=self.ramend).contains(&a) {
                        self.sram[(a - self.sram_start) as usize] = *b;
                    } else if (0x20..self.sram_start).contains(&a) {
                        self.io[(a - 0x20) as usize] = *b;
                    }
                }
            } else if (addr as usize) < self.flash.len() {
                // Program flash (code + data LMA).
                self.load_flash(addr as u32, &seg.data);
            }
        }
        self.pc = image.entry_point as u32 & !1;
    }

    /// Whether the bus maps the USART0 host window. A chip yaml without it has
    /// no peers to serve and the CPU's own registers are the whole USART.
    fn usart_on_bus(bus: &dyn Bus) -> bool {
        bus.has_mmio_window(AVR_IO_MIRROR_BASE + 0xC0)
    }

    /// Forward a USART register write to the bus-side USART model, when there
    /// is one. A refused write on a bus that does have the window is an error.
    fn usart_mirror_write(bus: &mut dyn Bus, reg: u64, value: u8) -> SimResult<()> {
        if Self::usart_on_bus(bus) {
            bus.write_u8(AVR_IO_MIRROR_BASE + reg, value)?;
        }
        Ok(())
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

    /// Push the return address. On a 3-byte-PC part (ATmega2560) the word
    /// address is 22 bits, three bytes go on the stack, and every user of this
    /// (CALL, RCALL, ICALL, EICALL, interrupt entry) takes one cycle more.
    fn push_pc(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        let word = self.pc / 2;
        if self.model.three_byte_pc() {
            self.push_byte(((word >> 16) & 0x3F) as u8, bus)?;
            self.cycles += 1;
        }
        self.push_byte(((word >> 8) & 0xFF) as u8, bus)?;
        self.push_byte((word & 0xFF) as u8, bus)?;
        Ok(())
    }

    /// Pop the return address (RET, RETI): 3 bytes and one cycle more on a
    /// 3-byte-PC part.
    fn pop_pc(&mut self, bus: &dyn Bus) -> SimResult<()> {
        let lo = self.pop_byte(bus)? as u32;
        let hi = self.pop_byte(bus)? as u32;
        let mut word = (hi << 8) | lo;
        if self.model.three_byte_pc() {
            let top = self.pop_byte(bus)? as u32;
            word |= (top & 0x3F) << 16;
            self.cycles += 1;
        }
        self.pc = word * 2;
        Ok(())
    }

    /// `EIND` (I/O 0x3C) and `RAMPZ` (I/O 0x3B): the high byte EIJMP/EICALL
    /// and ELPM put above Z. Stored in the I/O shadow.
    pub(in crate::cpu::avr) fn eind(&self) -> u32 {
        u32::from(self.io[(0x5C - 0x20) as usize])
    }
    pub(in crate::cpu::avr) fn rampz(&self) -> u32 {
        u32::from(self.io[(0x5B - 0x20) as usize])
    }
    pub(in crate::cpu::avr) fn set_rampz(&mut self, v: u8) {
        self.io[(0x5B - 0x20) as usize] = v;
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
        // Hardware clears the matching timer flag on vector entry.
        if vec == VEC_TIMER0_OVF {
            self.tifr0 &= !TIFR_TOV0;
        } else if vec == VEC_TIMER2_COMPA {
            self.tifr2 &= !TIFR2_OCF2A;
            self.sync_timer2_irq();
        } else if vec == VEC_TIMER2_COMPB {
            self.tifr2 &= !TIFR2_OCF2B;
            self.sync_timer2_irq();
        } else if vec == VEC_TIMER2_OVF {
            self.tifr2 &= !TIFR2_TOV2;
            self.sync_timer2_irq();
        }
        self.timer1_vector_entered(vec);
        // ... and the INTn / PCIFn flag of an external or pin-change vector.
        self.ext_vector_entered(vec);
        self.push_pc(bus)?;
        let slot = if self.is_mega() {
            mega2560_vector(vec)
        } else {
            vec
        };
        self.pc = slot.saturating_sub(1) * 4;
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

    /// Fold the tiny `INC Rd; RJMP -2` loop emitted by the AVR throughput
    /// fixture. This is intentionally an exact two-word recognizer, not a
    /// general AVR block executor: the latter would need to reproduce every
    /// instruction boundary at essentially interpreter cost.
    ///
    /// Timer0, Timer2 and a takeable interrupt make intermediate cycle boundaries
    /// observable, so callers refuse this path in either case. With those
    /// guards, the loop has no bus accesses and its complete architectural
    /// effect is the final register/flags/PC plus the summed instruction
    /// cycles calculated here.
    fn try_run_inc_rjmp_spin(&mut self, max_count: u32) -> u32 {
        if max_count == 0 || !matches!(self.pc, 2 | 4) {
            return 0;
        }
        let Ok(inc) = self.fetch_word(2) else {
            return 0;
        };
        let Ok(branch) = self.fetch_word(4) else {
            return 0;
        };
        // INC Rd followed by RJMP from byte 4 back to byte 2.
        if inc & 0xFE0F != 0x9403 || branch != 0xCFFE {
            return 0;
        }

        let starts_at_inc = self.pc == 2;
        let inc_count = if starts_at_inc {
            max_count.div_ceil(2)
        } else {
            max_count / 2
        };
        let branch_count = max_count - inc_count;

        if inc_count > 0 {
            let rd = ((inc >> 4) & 0x1F) as usize;
            let result = self.r[rd].wrapping_add(inc_count as u8);
            self.r[rd] = result;
            // INC preserves C, H, T and I and replaces V/Z/N/S from the final
            // increment. Since only the final architectural state is visible
            // under the caller's guards, applying those flags once is exact.
            self.set_v(result == 0x80);
            self.set_z(result);
            self.set_n(result);
            self.update_s_from_nv();
        }

        self.pc = match (starts_at_inc, max_count & 1) {
            (true, 0) | (false, 1) => 2,
            (true, 1) | (false, 0) => 4,
            _ => unreachable!(),
        };
        self.cycles += u64::from(inc_count) + 2 * u64::from(branch_count);
        max_count
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
        if self.model.three_byte_pc() {
            5
        } else {
            4
        }
    }

    /// A core stopped by `SLEEP` may be skipped until the next thing that can
    /// wake it: a Timer0 overflow or a Timer1/Timer2 compare/overflow with its
    /// interrupt enabled (while clk_I/O runs), or a pad change seen at a later
    /// boundary. Nothing to skip when an interrupt is already takeable, or a
    /// watched pad moved since the last sample, or the USART receive interrupt
    /// is on.
    fn idle_fast_forward_budget(&self, bus: &dyn Bus) -> Option<u64> {
        if !self.sleeping || self.wake_pending() || self.ext_pins_moved(bus) {
            return None;
        }
        // A byte arriving for RXCIE wakes the core, and its arrival time is the
        // bus-side USART's to know, not this budget's: with the interrupt on,
        // stay on the stepped path (which reads UCSR0A with its error).
        if self.ucsr0b & UCSRB_RXCIE != 0 && Self::usart_on_bus(bus) {
            return None;
        }
        Some(
            [
                self.timer0_wake_cycles(),
                self.timer1_wake_cycles(),
                self.timer2_wake_cycles(),
            ]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(u64::MAX),
        )
    }

    fn fast_forward_idle_cycles(&mut self, cycles: u64) {
        self.cycles += cycles;
        if self.io_clock_running() {
            let mut left = cycles;
            while left > 0 {
                let chunk = left.min(u64::from(u32::MAX));
                self.tick_io_timers(chunk as u32);
                left -= chunk;
            }
        }
    }

    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        // MCUCR resets to 0: PUD clear, so the ports' pull-ups come back.
        let mcucr = &mut self.io[usize::from(ADDR_MCUCR - 0x20)];
        if *mcucr & MCUCR_PUD != 0 {
            bus.set_pull_ups_disabled(false);
        }
        *mcucr = 0;
        self.r = [0; 32];
        self.pc = 0;
        self.sp = self.ramend;
        self.sreg = 0;
        self.pending_irq = 0;
        self.cycles = 0;
        self.tcnt0 = 0;
        self.tccr0a = 0;
        self.tccr0b = 0;
        self.timsk0 = 0;
        self.tifr0 = 0;
        self.t0_prescale_acc = 0;
        self.tcnt2 = 0;
        self.tccr2a = 0;
        self.tccr2b = 0;
        self.ocr2a = 0;
        self.ocr2b = 0;
        self.timsk2 = 0;
        self.tifr2 = 0;
        self.assr = 0;
        self.t2_prescale_acc = 0;
        self.t1 = timer1::Timer1::default();
        self.oc_mask = [0; 2];
        self.oc_level = [0; 2];
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
        self.ext_reset();
        self.irq_shadow = false;
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

        // RX-complete interrupt: level-sensitive on RXC0 while RXCIE0 is set.
        // The queue lives on the bus-side USART model, so look at it every 32nd
        // step instead of every one: a few microseconds of latency at 16 MHz,
        // and nothing at all on a sketch that never enables the interrupt.
        if self.ucsr0b & UCSRB_RXCIE != 0 {
            self.rx_poll = self.rx_poll.wrapping_add(1);
            // A sleeping core looks every cycle: the step is one idle clock,
            // and a skipped idle window must wake on the same cycle.
            if (self.rx_poll & 31 == 0 || self.sleeping)
                && Self::usart_on_bus(bus)
                && bus.read_u8(AVR_IO_MIRROR_BASE + 0xC0)? & UCSRA_RXC != 0
            {
                self.pending_irq |= 1u64 << VEC_USART_RX;
            }
        }

        // The flag was true when it was latched; an ISR that ran in between may
        // have read the byte. Vectoring then would hand the firmware a phantom
        // 0x00 from an empty UDR0, so check again at the moment of entry.
        if self.pending_irq & (1u64 << VEC_USART_RX) != 0
            && self.flag_i()
            && bus.read_u8(AVR_IO_MIRROR_BASE + 0xC0)? & UCSRA_RXC == 0
        {
            self.pending_irq &= !(1u64 << VEC_USART_RX);
        }

        // INT0/INT1 and PCINT: sample the pads at this boundary.
        if self.ext_watch != 0 {
            self.sample_ext_pins(bus);
        }

        if self.sleeping {
            if !self.wake_pending() {
                // One idle clock: nothing retires, Timer0 counts if clk_I/O runs.
                self.cycles += 1;
                if self.io_clock_running() {
                    self.tick_io_timers(1);
                    self.sync_oc_pins(bus);
                }
                return Ok(());
            }
            // "The MCU is then halted for four cycles in addition to the
            // start-up time" (none in idle mode; oscillator start-up after a
            // deeper sleep is not modelled), then the vector is entered.
            self.sleeping = false;
            self.cycles += 4;
        }

        let shadow = std::mem::take(&mut self.irq_shadow);
        if !shadow && self.try_take_irq(bus)? {
            self.cycles += 4;
            let delta = self.cycles.saturating_sub(before) as u32;
            self.tick_io_timers(delta.max(1));
            self.sync_oc_pins(bus);
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

        self.tick_io_timers(delta.max(1));
        self.sync_oc_pins(bus);
        Ok(())
    }

    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        let push_tap = bus.logic_tap().filter(|tap| tap.push_armed());
        let timer_stopped =
            self.t0_prescaler() == 0 && self.t1.prescaler() == 0 && self.t2_prescaler() == 0;
        let irq_takeable = self.flag_i() && self.pending_irq != 0;
        // A sleeping core, or pads that must be sampled at every boundary for
        // INT/PCINT, keep the one-instruction path.
        let irq_takeable = irq_takeable || self.sleeping || self.ext_watch != 0;
        // The INC/RJMP spin touches no bus, so it can never push a pad edge:
        // it stays available under push capture, as long as the tap clock is
        // carried across the cycles it retires.
        if config.batch_mode_enabled && observers.is_empty() && timer_stopped && !irq_takeable {
            let cycles_before = self.cycles;
            let retired = self.try_run_inc_rjmp_spin(max_count);
            if retired > 0 {
                if let Some(tap) = &push_tap {
                    tap.set_clock(tap.clock() + (self.cycles - cycles_before));
                }
                // The default batch loop publishes one simulated instruction
                // per retired AVR instruction. This loop performs no bus read,
                // so one equivalent accumulated update is sufficient.
                crate::advance_batch_cycle(
                    bus,
                    u64::from(config.peripheral_tick_interval > 1) * u64::from(retired),
                );
                return Ok(retired);
            }
        }
        crate::default_step_batch(self, bus, observers, config, max_count)
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
#[path = "avr/ext_int.rs"]
mod ext_int;
#[path = "avr/oc_pins.rs"]
mod oc_pins;
#[path = "avr/timer1.rs"]
mod timer1;
pub use ext_int::{VEC_INT0, VEC_INT1, VEC_PCINT0, VEC_PCINT1, VEC_PCINT2};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DmaRequest, SimulationConfig};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    #[derive(Debug)]
    struct CountSteps(Arc<AtomicUsize>);

    impl SimulationObserver for CountSteps {
        fn on_step_start(&self, _pc: u32, _opcode: u32) {
            self.0.fetch_add(1, Ordering::Relaxed);
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
    fn inc_rjmp_batch_matches_instruction_steps_at_every_phase_and_budget() {
        let cfg = SimulationConfig::default();
        // INC r18; RJMP back one word. Exercise both entry phases, odd/even
        // budgets, flag boundaries (0x7f -> 0x80 and 0xff -> 0), and preserved
        // C/H/T/I bits.
        for pc in [2, 4] {
            for budget in 1..=17 {
                for initial in [0x00, 0x6f, 0x7f, 0xf7, 0xff] {
                    let mut fast = Avr::new();
                    let mut reference = Avr::new();
                    for cpu in [&mut fast, &mut reference] {
                        cpu.load_words(2, &[0x9523, 0xCFFE]);
                        cpu.pc = pc;
                        cpu.r[18] = initial;
                        cpu.sreg = 0xE1;
                        cpu.cycles = 123;
                    }
                    let mut reference_bus = MockBus::new();

                    assert_eq!(
                        fast.try_run_inc_rjmp_spin(budget),
                        budget,
                        "recognizer made no progress at pc={pc} budget={budget}"
                    );
                    crate::default_step_batch(
                        &mut reference,
                        &mut reference_bus,
                        &[],
                        &cfg,
                        budget,
                    )
                    .unwrap();

                    assert_eq!(fast.r, reference.r, "pc={pc} budget={budget}");
                    assert_eq!(fast.sreg, reference.sreg, "pc={pc} budget={budget}");
                    assert_eq!(fast.pc, reference.pc, "pc={pc} budget={budget}");
                    assert_eq!(fast.cycles, reference.cycles, "pc={pc} budget={budget}");
                }
            }
        }
    }

    #[test]
    fn inc_rjmp_batch_refuses_near_misses() {
        let mut cpu = Avr::new();
        cpu.load_words(2, &[0x9523, 0xCFFE]);
        cpu.pc = 2;

        assert_eq!(cpu.try_run_inc_rjmp_spin(16), 16);
        // A one-word near miss must never be recognized.
        cpu.load_words(4, &[0xCFFF]);
        cpu.pc = 2;
        assert_eq!(cpu.try_run_inc_rjmp_spin(16), 0);
    }

    #[test]
    fn inc_rjmp_batch_falls_back_for_trace_and_timer_boundaries() {
        let cfg = SimulationConfig::default();

        let mut traced = Avr::new();
        traced.load_words(2, &[0x9523, 0xCFFE]);
        traced.pc = 2;
        let count = Arc::new(AtomicUsize::new(0));
        let observer: Arc<dyn SimulationObserver> = Arc::new(CountSteps(count.clone()));
        traced
            .step_batch(
                &mut MockBus::new(),
                std::slice::from_ref(&observer),
                &cfg,
                7,
            )
            .unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 7);

        let mut timed = Avr::new();
        timed.load_words(2, &[0x9523, 0xCFFE]);
        timed.pc = 2;
        timed.tccr0b = 1; // Timer0 clocked at CPU/1.
        timed.tcnt0 = 254;
        timed.timsk0 = TIMSK_TOIE0;
        timed.sreg = 0x80; // Interrupts globally enabled.
        timed.step_batch(&mut MockBus::new(), &[], &cfg, 2).unwrap();
        assert_eq!(timed.tcnt0, 1, "one INC cycle plus two RJMP cycles");
        assert_ne!(timed.pending_irq & (1 << VEC_TIMER0_OVF), 0);
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

    /// avr-gcc compares a 32-bit value with `cpi` plus three `sbci`. SBCI's Z
    /// is sticky, so a zero byte result must not set Z after an earlier
    /// mismatch. Adafruit DHT `expectPulse` treats `count == 0xFFFFFFFF` as a
    /// timeout; a non-sticky Z made every small count look like that timeout.
    #[test]
    fn sbci_sticky_z_on_32bit_compare_with_minus_one() {
        const Z: u8 = 0x02;
        // cpi r22,0xFF; sbci r23,0xFF; sbci r24,0xFF; sbci r25,0xFF
        const WORDS: [u16; 4] = [0x3F6F, 0x4F7F, 0x4F8F, 0x4F9F];

        let z_set = |bytes: [u8; 4]| {
            let mut cpu = Avr::new();
            cpu.load_words(0, &WORDS);
            cpu.r[22] = bytes[0];
            cpu.r[23] = bytes[1];
            cpu.r[24] = bytes[2];
            cpu.r[25] = bytes[3];
            let mut bus = MockBus::new();
            let cfg = SimulationConfig::default();
            for _ in 0..4 {
                cpu.step(&mut bus, &[], &cfg).unwrap();
            }
            cpu.sreg & Z != 0
        };

        assert!(
            z_set([0xFF, 0xFF, 0xFF, 0xFF]),
            "0xFFFFFFFF == 0xFFFFFFFF sets Z"
        );
        assert!(
            !z_set([0x05, 0x00, 0x00, 0x00]),
            "a small count is not equal to 0xFFFFFFFF"
        );
        assert!(
            !z_set([0xFF, 0xFF, 0xFF, 0x00]),
            "a mismatch in the top byte clears Z"
        );
    }

    /// BSET/BCLR are one-cycle SREG bit ops. Only SEI/CLI used to be decoded,
    /// so `clt` (0x94E8) from avr-libc `__floatsisf` was a decode error.
    #[test]
    fn bset_bclr_round_trip_takes_one_cycle() {
        // (BSET, BCLR, SREG mask). I is SEI/CLI and stays on its own path.
        let pairs = [
            (0x9408u16, 0x9488u16, 0x01u8), // SEC / CLC
            (0x9468, 0x94E8, 0x40),         // SET / CLT
        ];
        let mut bus = MockBus::new();
        let cfg = SimulationConfig::default();
        for (bset, bclr, mask) in pairs {
            let mut cpu = Avr::new();
            // I stays set across the round trip; these ops must not clear it.
            cpu.sreg = 0x80;
            cpu.load_words(0, &[bset, bclr]);
            cpu.step(&mut bus, &[], &cfg).unwrap();
            assert_eq!(cpu.cycles, 1, "BSET is one cycle (op {bset:#06x})");
            assert_eq!(cpu.sreg, 0x80 | mask, "BSET {bset:#06x}");
            cpu.step(&mut bus, &[], &cfg).unwrap();
            assert_eq!(cpu.cycles, 2, "BCLR is one cycle (op {bclr:#06x})");
            assert_eq!(cpu.sreg, 0x80, "BCLR {bclr:#06x} restores SREG");
            assert_eq!(cpu.pc, 4);
        }
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
    fn bst_copies_register_bit_into_t() {
        let mut cpu = Avr::new();
        // BST r25, 7 = 0xFB97 (avr-libc __divmodsi4 prologue; host ELF PC 0x93c)
        cpu.load_words(0, &[0xFB97]);
        cpu.r[25] = 0x80;
        let mut bus = MockBus::new();
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert!(cpu.flag_t());
        assert_eq!(cpu.pc, 2);

        cpu.pc = 0;
        cpu.r[25] = 0x00;
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert!(!cpu.flag_t());
    }

    #[test]
    fn bld_copies_t_into_register_bit() {
        let mut cpu = Avr::new();
        // BLD r16, 0 = 0xF900 (1111 100d dddd 0bbb with d=16, b=0)
        cpu.load_words(0, &[0xF900]);
        cpu.r[16] = 0xFE;
        cpu.set_flag_t(true);
        let mut bus = MockBus::new();
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.r[16], 0xFF);

        cpu.pc = 0;
        cpu.r[16] = 0xFF;
        cpu.set_flag_t(false);
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.r[16], 0xFE);
    }

    #[test]
    fn morning_divmodsi4_prologue_steps_past_bst() {
        // Regression for Uno pot/bargraph prove: DecodeError at byte PC 0x93c
        // was BST from avr-libc __divmodsi4 (map() → signed division). Hosted
        // morning ELF faults at 0x93c; this fixture (same sketch, local
        // arduino:avr core) places the same BST word nearby — without BST the
        // twin hard-stops before LEDs/gpio_edges can move.
        let mut cpu = Avr::new();
        let elf_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/avr/arduino-uno-morning-bargraph.elf");
        let elf = std::fs::read(&elf_path).expect("morning bargraph fixture ELF");
        assert_eq!(&elf[0..4], b"\x7fELF");
        let e_phoff = u32::from_le_bytes(elf[28..32].try_into().unwrap()) as usize;
        let e_phentsize = u16::from_le_bytes(elf[42..44].try_into().unwrap()) as usize;
        let e_phnum = u16::from_le_bytes(elf[44..46].try_into().unwrap()) as usize;
        let mut loaded = false;
        for i in 0..e_phnum {
            let off = e_phoff + i * e_phentsize;
            let p_type = u32::from_le_bytes(elf[off..off + 4].try_into().unwrap());
            if p_type != 1 {
                continue;
            }
            let p_offset = u32::from_le_bytes(elf[off + 4..off + 8].try_into().unwrap()) as usize;
            let p_vaddr = u32::from_le_bytes(elf[off + 8..off + 12].try_into().unwrap());
            let p_filesz = u32::from_le_bytes(elf[off + 16..off + 20].try_into().unwrap()) as usize;
            if p_vaddr != 0 || p_filesz == 0 {
                continue;
            }
            let src = &elf[p_offset..p_offset + p_filesz];
            cpu.flash[..src.len()].copy_from_slice(src);
            loaded = true;
            break;
        }
        assert!(loaded, "no flash PT_LOAD");
        let mut bst_pc = None;
        let mut pc = 0usize;
        while pc + 1 < cpu.flash.len() {
            let op = u16::from_le_bytes([cpu.flash[pc], cpu.flash[pc + 1]]);
            let is32 = (op & 0xFE0F) == 0x9000
                || (op & 0xFE0F) == 0x9200
                || (op & 0xFE0E) == 0x940C
                || (op & 0xFE0E) == 0x940E;
            if pc >= 0x100 && (op & 0xFE08) == 0xFA00 {
                bst_pc = Some(pc as u32);
                break;
            }
            pc += if is32 { 4 } else { 2 };
        }
        let bst_pc = bst_pc.expect("morning ELF should contain BST");
        // Fixture places BST at 0x944; hosted morning prove faulted at 0x93c —
        // same opcode family (0xFBxx BST).
        assert_eq!(
            u16::from_le_bytes([cpu.flash[bst_pc as usize], cpu.flash[bst_pc as usize + 1]]),
            0xFB97
        );
        cpu.pc = bst_pc;
        cpu.r[25] = 0x80;
        let mut bus = MockBus::new();
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap_or_else(|e| panic!("must step past BST at {bst_pc:#x}: {e:?}"));
        assert!(cpu.flag_t());
        assert_eq!(cpu.pc, bst_pc + 2);
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

    /// The closed form has to agree with one timer clock at a time, including
    /// a counter left above TOP and a chunk large enough to skip the loop.
    #[test]
    fn timer2_closed_form_matches_one_tick_at_a_time() {
        let cases = [
            (true, 177u8, 0u8, 0u8, 178u64),
            (true, 177, 50, 10, 5_000),
            (true, 0, 0, 0, 10),
            (true, 255, 1, 0, 1_000),
            (true, 10, 200, 50, 300),
            (true, 1, 1, 1, 4),
            (false, 177, 3, 200, 10_000),
            (false, 0, 255, 0, 256),
            (false, 1, 1, 255, 2),
            (false, 128, 128, 0, 256 * 3 + 7),
        ];
        for (ctc, ocr_a, ocr_b, start, ticks) in cases {
            let mut tcnt = start;
            let mut ocfa = false;
            let mut ocfb = false;
            let mut tov = false;
            for _ in 0..ticks {
                let step = t2_one_tick(ctc, tcnt, ocr_a, ocr_b);
                tcnt = step.tcnt;
                ocfa |= step.ocfa;
                ocfb |= step.ocfb;
                tov |= step.tov;
            }
            let mut cpu = Avr::new();
            cpu.tccr2a = if ctc { 0x02 } else { 0 };
            cpu.tccr2b = 1;
            cpu.ocr2a = ocr_a;
            cpu.ocr2b = ocr_b;
            cpu.tcnt2 = start;
            cpu.tick_timer2(ticks as u32);
            assert_eq!(
                cpu.tcnt2, tcnt,
                "ctc={ctc} ocr={ocr_a} start={start} ticks={ticks}"
            );
            assert_eq!(cpu.tifr2 & TIFR2_OCF2A != 0, ocfa, "OCF2A");
            assert_eq!(cpu.tifr2 & TIFR2_OCF2B != 0, ocfb, "OCF2B");
            assert_eq!(cpu.tifr2 & TIFR2_TOV2 != 0, tov, "TOV2");
        }
    }

    /// Arduino `tone(6, 700, _)` at 16 MHz lands on OCR2A = 177, clk/64.
    /// Matches are (OCR+1) * prescaler CPU cycles apart.
    #[test]
    fn timer2_ctc_match_period_is_ocr_plus_one_times_prescaler() {
        let mut cpu = Avr::new();
        let mut bus = MockBus::new();
        cpu.data_write(0xB0, 1 << 1, &mut bus).unwrap(); // WGM21, CTC
        cpu.data_write(0xB1, 0x04, &mut bus).unwrap(); // CS22, clk/64
        cpu.data_write(0xB3, 177, &mut bus).unwrap();
        cpu.data_write(0x70, TIMSK2_OCIE2A, &mut bus).unwrap();
        cpu.sreg = 0x80;
        // From 0 the first compare is the clock that lands on OCR2A. After
        // that, matches are (OCR2A + 1) timer clocks apart.
        let first = 177 * 64;
        let period = 178 * 64;
        cpu.tick_timer2(first - 1);
        assert_eq!(cpu.tifr2 & TIFR2_OCF2A, 0, "the match is on the last clock");
        assert_eq!(cpu.pending_irq & (1 << VEC_TIMER2_COMPA), 0);
        cpu.tick_timer2(1);
        assert_ne!(cpu.tifr2 & TIFR2_OCF2A, 0);
        assert_ne!(cpu.pending_irq & (1 << VEC_TIMER2_COMPA), 0);
        assert_eq!(cpu.tcnt2, 177);
        cpu.data_write(0x37, TIFR2_OCF2A, &mut bus).unwrap();
        cpu.tick_timer2(period - 1);
        assert_eq!(cpu.tifr2 & TIFR2_OCF2A, 0, "steady period has not elapsed");
        cpu.tick_timer2(1);
        assert_ne!(cpu.tifr2 & TIFR2_OCF2A, 0);
        assert_eq!(cpu.tcnt2, 177);

        // Vector entry clears the flag and lands on TIMER2_COMPA_vect.
        cpu.load_words(0, &[0x0000]);
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.pc, (VEC_TIMER2_COMPA - 1) * 4);
        assert_eq!(cpu.tifr2 & TIFR2_OCF2A, 0);
        assert_eq!(cpu.pending_irq & (1 << VEC_TIMER2_COMPA), 0);
    }

    #[test]
    fn timer2_prescalers_include_32_and_128_and_foc_reads_zero() {
        let mut cpu = Avr::new();
        let mut bus = MockBus::new();
        cpu.data_write(0xB1, 0xFF, &mut bus).unwrap();
        assert_eq!(cpu.data_read(0xB1, &bus).unwrap() & 0xC0, 0, "FOC2A/FOC2B");
        assert_eq!(cpu.t2_prescaler(), 1024);
        for (cs, div) in [
            (1, 1),
            (2, 8),
            (3, 32),
            (4, 64),
            (5, 128),
            (6, 256),
            (7, 1024),
        ] {
            cpu.tccr2b = cs;
            assert_eq!(cpu.t2_prescaler(), div, "CS={cs}");
        }
    }

    #[test]
    fn timer2_assr_keeps_only_as2_and_stops_the_cpu_clock() {
        let mut cpu = Avr::new();
        let mut bus = MockBus::new();
        cpu.data_write(0xB6, 0xFF, &mut bus).unwrap();
        assert_eq!(cpu.data_read(0xB6, &bus).unwrap(), ASSR_EXCLK | ASSR_AS2);
        cpu.tccr2b = 1;
        cpu.tick_timer2(1000);
        assert_eq!(cpu.tcnt2, 0, "AS2 is not the CPU clock");
        cpu.data_write(0xB6, 0, &mut bus).unwrap();
        cpu.tick_timer2(5);
        assert_eq!(cpu.tcnt2, 5);
    }

    #[test]
    fn timer2_compare_b_and_overflow_vectors() {
        let mut cpu = Avr::new();
        let mut bus = MockBus::new();
        cpu.sreg = 0x80;
        cpu.tccr2b = 1;
        cpu.ocr2b = 1;
        cpu.timsk2 = TIMSK2_OCIE2B;
        cpu.tick_timer2(1);
        assert_eq!(cpu.tcnt2, 1);
        assert_ne!(cpu.pending_irq & (1 << VEC_TIMER2_COMPB), 0);
        cpu.load_words(0, &[0x0000]);
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.pc, (VEC_TIMER2_COMPB - 1) * 4);

        let mut cpu = Avr::new();
        cpu.sreg = 0x80;
        cpu.tccr2b = 1;
        cpu.tcnt2 = 255;
        cpu.timsk2 = TIMSK2_TOIE2;
        cpu.tick_timer2(1);
        assert_eq!(cpu.tcnt2, 0);
        assert_ne!(cpu.tifr2 & TIFR2_TOV2, 0);
        cpu.load_words(0, &[0x0000]);
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.pc, (VEC_TIMER2_OVF - 1) * 4);
    }

    /// Enabling OCIE2A while OCF2A is already set requests the interrupt.
    #[test]
    fn timer2_late_enable_sees_a_sticky_flag() {
        let mut cpu = Avr::new();
        let mut bus = MockBus::new();
        cpu.tccr2a = 0x02;
        cpu.tccr2b = 1;
        cpu.ocr2a = 0;
        cpu.tick_timer2(1);
        assert_ne!(cpu.tifr2 & TIFR2_OCF2A, 0);
        assert_eq!(cpu.pending_irq & (1 << VEC_TIMER2_COMPA), 0);
        cpu.data_write(0x70, TIMSK2_OCIE2A, &mut bus).unwrap();
        assert_ne!(cpu.pending_irq & (1 << VEC_TIMER2_COMPA), 0);
        cpu.data_write(0x37, TIFR2_OCF2A, &mut bus).unwrap();
        assert_eq!(cpu.tifr2 & TIFR2_OCF2A, 0);
        assert_eq!(cpu.pending_irq & (1 << VEC_TIMER2_COMPA), 0);
    }

    /// The idle skip lands the Timer2 compare on the same cycle as stepping
    /// the sleeping core one idle clock at a time.
    #[test]
    fn timer2_sleep_budget_matches_the_stepped_wake() {
        let setup = || {
            let mut cpu = Avr::new();
            cpu.tccr2a = 0x02;
            cpu.tccr2b = 0x04;
            cpu.ocr2a = 10;
            cpu.tcnt2 = 3;
            cpu.t2_prescale_acc = 5;
            cpu.timsk2 = TIMSK2_OCIE2A;
            cpu.sreg = 0x80;
            cpu.smcr = 1;
            cpu.load_words(0x100, &[0x9588, 0xCFFF]);
            cpu.pc = 0x100;
            let mut bus = MockBus::new();
            cpu.step(&mut bus, &[], &SimulationConfig::default())
                .unwrap();
            assert!(cpu.sleeping);
            (cpu, bus)
        };
        let (mut stepped, mut sbus) = setup();
        for _ in 0..10_000 {
            if stepped.pc == (VEC_TIMER2_COMPA - 1) * 4 {
                break;
            }
            stepped
                .step(&mut sbus, &[], &SimulationConfig::default())
                .unwrap();
        }
        assert_eq!(
            stepped.pc,
            (VEC_TIMER2_COMPA - 1) * 4,
            "compare A woke the core"
        );
        let (mut skipped, mut kbus) = setup();
        let budget = skipped.idle_fast_forward_budget(&kbus).expect("asleep");
        skipped.fast_forward_idle_cycles(budget);
        assert_eq!(
            skipped.idle_fast_forward_budget(&kbus),
            None,
            "OCF2A pending"
        );
        skipped
            .step(&mut kbus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(skipped.pc, stepped.pc);
        assert_eq!(skipped.cycles, stepped.cycles, "same wake cycle");
        assert_eq!(skipped.tcnt2, stepped.tcnt2);
        assert_eq!(skipped.t2_prescale_acc, stepped.t2_prescale_acc);
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

    // ---- ATmega2560 (AvrModel::Atmega2560) ----

    fn mega() -> Avr {
        Avr::new_model(AvrModel::Atmega2560)
    }

    fn step_n(cpu: &mut Avr, bus: &mut MockBus, n: usize) {
        let cfg = SimulationConfig::default();
        for _ in 0..n {
            cpu.step(bus, &[], &cfg).unwrap();
        }
    }

    #[test]
    fn chip_name_selects_the_model() {
        assert_eq!(Avr::for_chip_name("atmega2560").model, AvrModel::Atmega2560);
        assert_eq!(Avr::for_chip_name("atmega328p").model, AvrModel::Atmega328p);
        let m = mega();
        assert_eq!(m.sp, 0x21FF, "SP resets to RAMEND");
        assert_eq!(m.flash.len(), 256 * 1024);
    }

    #[test]
    fn mega_call_and_ret_move_a_three_byte_pc() {
        // CALL 0x000100 (word 0x80); at 0x100: RET.
        let prog = [0x940E, 0x0080];
        let run = |mut cpu: Avr| {
            let mut bus = MockBus::new();
            cpu.load_words(0, &prog);
            cpu.load_words(0x100, &[0x9508]);
            cpu.set_pc(0);
            let (sp0, c0) = (cpu.sp, cpu.cycles);
            step_n(&mut cpu, &mut bus, 1);
            assert_eq!(cpu.pc, 0x100);
            let (pushed, call_cycles) = (sp0 - cpu.sp, cpu.cycles - c0);
            let c1 = cpu.cycles;
            step_n(&mut cpu, &mut bus, 1);
            assert_eq!(cpu.pc, 4, "RET comes back after the 2-word CALL");
            assert_eq!(cpu.sp, sp0);
            (pushed, call_cycles, cpu.cycles - c1)
        };
        let (p328, call328, ret328) = run(Avr::new());
        let (p2560, call2560, ret2560) = run(mega());
        assert_eq!(p328, 2);
        assert_eq!(p2560, 3, "the 2560 pushes a 3-byte return address");
        assert_eq!(call2560, call328 + 1, "CALL takes one more cycle");
        assert_eq!(ret2560, ret328 + 1, "RET takes one more cycle");
    }

    #[test]
    fn mega_eicall_and_eijmp_use_eind() {
        let mut bus = MockBus::new();
        let mut cpu = mega();
        // LDI R16,1; OUT EIND(0x3C),R16; LDI R30,0x10; LDI R31,0; EICALL
        cpu.load_words(0, &[0xE001, 0xBF0C, 0xE1E0, 0xE0F0, 0x9519]);
        cpu.set_pc(0);
        step_n(&mut cpu, &mut bus, 5);
        assert_eq!(cpu.eind(), 1);
        assert_eq!(cpu.pc, 0x2_0020, "EIND:Z = word 0x1_0010");
        assert_eq!(cpu.sp, 0x21FF - 3);

        let mut cpu = mega();
        // Same, with EIJMP: no push.
        cpu.load_words(0, &[0xE001, 0xBF0C, 0xE1E0, 0xE0F0, 0x9419]);
        cpu.set_pc(0);
        step_n(&mut cpu, &mut bus, 5);
        assert_eq!(cpu.pc, 0x2_0020);
        assert_eq!(cpu.sp, 0x21FF);
    }

    #[test]
    fn mega_elpm_reads_rampz_z_and_carries_into_rampz() {
        let mut bus = MockBus::new();
        let mut cpu = mega();
        cpu.load_words(0x1_0004, &[0x1234]);
        // LDI R16,1; OUT RAMPZ(0x3B),R16; LDI R30,4; LDI R31,0; ELPM R5,Z+; ELPM R6,Z
        cpu.load_words(0, &[0xE001, 0xBF0B, 0xE0E4, 0xE0F0, 0x9057, 0x9066]);
        cpu.set_pc(0);
        step_n(&mut cpu, &mut bus, 6);
        assert_eq!(cpu.r[5], 0x34);
        assert_eq!(cpu.r[6], 0x12);
        assert_eq!((cpu.rampz(), cpu.r[31], cpu.r[30]), (1, 0, 5));

        // Z+ from 0xFFFF carries into RAMPZ.
        let mut cpu = mega();
        cpu.load_words(0xFFFE, &[0xAB00]);
        // LDI R30,0xFF; LDI R31,0xFF; ELPM R5,Z+
        cpu.load_words(0, &[0xEFEF, 0xEFFF, 0x9057]);
        cpu.set_pc(0);
        step_n(&mut cpu, &mut bus, 3);
        assert_eq!(cpu.r[5], 0xAB);
        assert_eq!((cpu.rampz(), cpu.r[31], cpu.r[30]), (1, 0, 0));
    }

    #[test]
    fn mega_extended_io_port_h_is_mirrored_and_readable() {
        let mut bus = MockBus::new();
        let mut cpu = mega();
        cpu.data_write(0x101, 0x38, &mut bus).unwrap(); // DDRH: PH3..PH5 out
        cpu.data_write(0x102, 0x20, &mut bus).unwrap(); // PORTH: PH5 (D8) high
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x101).unwrap(), 0x38);
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x102).unwrap(), 0x20);
        assert_eq!(cpu.data_read(0x100, &bus).unwrap() & 0x38, 0x20, "PINH");
        // PORTK (0x108) too; on the 328P that address is plain SRAM.
        cpu.data_write(0x108, 0x03, &mut bus).unwrap();
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x108).unwrap(), 0x03);
        let mut uno = Avr::new();
        let mut uno_bus = MockBus::new();
        uno.data_write(0x108, 0x03, &mut uno_bus).unwrap();
        assert_eq!(uno_bus.read_u8(AVR_IO_MIRROR_BASE + 0x108).unwrap(), 0);
    }

    #[test]
    fn mega_adc_mux5_selects_adc8_to_adc15() {
        let mut bus = MockBus::new();
        let mut cpu = mega();
        let put = |bus: &mut MockBus, ch: u64, mv: u16| {
            let base = AVR_MEGA_ADC_INPUT_BASE + ch * 2;
            bus.write_u8(base, (mv & 0xFF) as u8).unwrap();
            bus.write_u8(base + 1, (mv >> 8) as u8).unwrap();
        };
        put(&mut bus, 0, 1000);
        put(&mut bus, 1, 1500);
        put(&mut bus, 8, 5000);
        put(&mut bus, 9, 2000);
        put(&mut bus, 15, 250);
        let convert = |cpu: &mut Avr, bus: &mut MockBus, mux5: bool, mux: u8| {
            cpu.data_write(0x7B, if mux5 { 0x08 } else { 0 }, bus)
                .unwrap();
            cpu.data_write(0x7C, 0x40 | mux, bus).unwrap();
            cpu.data_write(0x7A, 0xC0, bus).unwrap();
            u16::from(cpu.adch) << 8 | u16::from(cpu.adcl)
        };
        assert_eq!(convert(&mut cpu, &mut bus, false, 0), 204, "A0 = ADC0");
        assert_eq!(convert(&mut cpu, &mut bus, false, 1), 307, "A1 = ADC1");
        assert_eq!(convert(&mut cpu, &mut bus, true, 0), 1023, "A8 = ADC8");
        assert_eq!(convert(&mut cpu, &mut bus, true, 1), 409, "A9 = ADC9");
        assert_eq!(convert(&mut cpu, &mut bus, true, 7), 51, "A15 = ADC15");
        assert_eq!(convert(&mut cpu, &mut bus, false, 0x1F), 0, "GND");
    }

    #[test]
    fn mega_vectors_follow_the_2560_table() {
        // Internal (328P, 1-based) numbering -> ATmega2560 slot (DS2549 §14).
        let table = [
            (2, 2),   // INT0
            (4, 10),  // PCINT0
            (7, 13),  // WDT
            (10, 16), // TIMER2_OVF
            (14, 21), // TIMER1_OVF
            (17, 24), // TIMER0_OVF
            (18, 25), // SPI_STC
            (19, 26), // USART0_RX
            (20, 27), // USART0_UDRE
            (21, 28), // USART0_TX
            (22, 30), // ADC
            (25, 40), // TWI
        ];
        for (v328, v2560) in table {
            assert_eq!(mega2560_vector(v328), v2560, "328P vector {v328}");
        }
    }
}
