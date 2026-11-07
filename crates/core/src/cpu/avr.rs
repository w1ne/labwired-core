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

/// The AVR parts this interpreter runs. The ATmega1281 and ATmega2560 family
/// cores share mega peripheral behavior, with per-device flash and PC geometry.
/// The ATmega2560 (Arduino Mega) has 256 KB flash and a 3-byte PC; the
/// ATmega1281 has 128 KB flash and a 2-byte PC. Both have 8 KB SRAM at 0x0200.
/// EIND for EIJMP/EICALL is available on the ATmega2560/2561 model, while
/// RAMPZ for ELPM is shared. The mega peripheral map includes ports A..L
/// (H..L in extended I/O at 0x100..0x10B), a 16-channel ADC (ADCSRB.MUX5
/// selects ADC8..ADC15) and its own vector table. Timer0/1/2, USART0, SPI and
/// TWI sit at the same data addresses as on the 328P, so they are shared; their
/// vector numbers are remapped on entry ([`mega2560_vector`]).
///
/// Not modelled on the ATmega2560: INTn/PCINT (its pins differ from the
/// 328P's; the registers are stored only), output-compare pads (PWM
/// `analogWrite` levels; the pads follow `PORTx`), Timer3/4/5 and USART1..3
/// (registers stored only).
///
/// The classic ATmega8/16/32 keep every peripheral in the 64-byte I/O space
/// (data 0x20..0x5F) and start SRAM at 0x60, so their map shares nothing with
/// the 328P's beyond SP/SREG: data 0x26 is ADCSRA there, not PINC, and 0x35 is
/// PORTC, not TIFR0. Checked against avr-libc 2.0.0 iom8.h/iom16.h/iom32.h:
/// ADCL 0x24, ADCH 0x25, ADCSRA 0x26, ADMUX 0x27; PINx/DDRx/PORTx ascending at
/// 0x30 (D), 0x33 (C), 0x36 (B) and, on the 16/32, 0x39 (A); SFIOR.PUD (bit 2)
/// at 0x50. The ADC is on port C on the ATmega8 (ADC6/ADC7 are TQFP-only pins)
/// and on port A on the 16/32. Only that map, the ADC and the ports are
/// modelled on these cores; their timers, USART, SPI and TWI registers are
/// stored only, and no interrupt is taken (the vector tables are not modelled).
///
/// The ATmega164/324/644/1284 (avr-libc `iomxx4.h`, `iom164p.h`, `iom324p.h`,
/// `iom644.h`, `iom1284p.h`) have every ATmega328P register at the 328P's
/// address, plus port A (PINA/DDRA/PORTA at 0x20..0x22), PCMSK3, USART1 and
/// PRR0. So the 328P peripheral handlers are shared, as on the mega. Their ADC
/// is ADC0..7 on PA0..PA7 with MUX4:0 (0x1E the 1.1 V bandgap, 0x1F GND,
/// 0x08..0x1D differential/gain codes, read 0) and REFS = 10 / 11 select the
/// internal 1.1 V / 2.56 V. INT2 and PCINT3 sit early in their vector table,
/// so vectors are remapped on entry ([`xx4_vector`]). Not modelled: INTn/PCINT
/// (PCINT0..7 are port A here, not port B; the registers are stored only),
/// output-compare pads (OC0A/OC0B are PB3/PB4, OC1A/OC1B PD5/PD4, OC2A/OC2B
/// PD7/PD6; the pads follow `PORTx`), USART1 (stored only).
///
/// The ATmega128RFA1 and ATmega128RFR2 (avr-libc `iom128rfa1.h`,
/// `iom128rfr2.h`) have the ATmega1281 data map for ports A..G (no ports
/// H..L: 0x100..0x10B is empty), ADCL..ADMUX and the 1281 vector table for
/// slots 1..56 (the radio, symbol counter, AES and BAT_LOW vectors follow).
/// They run on the mega core with 16 KB SRAM (RAMEND 0x41FF). Their ADC is
/// powered from the internal 1.8 V AVDD regulator (ATmega128RFA1 datasheet
/// 8266F §27, ATmega256RFR2 family 8393C §27): MUX5:0 0..7 are ADC0..7
/// (PF0..PF7), 0x1E the 1.2 V bandgap, 0x1F AVSS; differential, temperature
/// sensor and back-bias codes read 0. REFS = 01 is AVDD (1.8 V), 10 the
/// internal 1.5 V and 11 the internal 1.6 V; 00 (AREF) is taken as AVDD
/// because AREF is not a modelled pin. ADCSRB.AVDDOK and REFOK read 1 while
/// ADEN is set (no start-up delay is modelled). Not modelled: the radio
/// transceiver, the RFA1's REFS latch-until-re-enable, OC pads (they follow
/// `PORTx`).
///
/// The ATtiny25/45/85 (avr-libc `iotn25.h`/`iotn45.h`/`iotn85.h`, all
/// `iotnx5.h`) share the classic parts' I/O-space layout for what is modelled:
/// ADCL/ADCH 0x24/0x25, ADCSRA 0x26, ADMUX 0x27, PINB/DDRB/PORTB 0x36..0x38,
/// SP/SREG 0x5D..0x5F and SRAM from 0x60 (RAMEND 0xDF/0x15F/0x25F, flash
/// 2/4/8 KB). Port B is their only port. ADCSRB (0x23) is stored only (MUX5
/// does not exist), and PUD is MCUCR bit 6 (0x55), not SFIOR. The ADC is
/// ADC0..3 on PB5, PB2, PB4, PB3 with MUX3:0 and a third reference bit, REFS2,
/// at ADMUX bit 4 (see [`Avr::tiny_x5_adc_input`]). As on the classic cores,
/// timers, USI and the watchdog are stored only and no interrupt is taken.
///
/// The ATtiny24/44/84 (avr-libc `iotn24.h`/`iotn44.h`/`iotn84.h`, all
/// `iotnx4.h`) keep the same I/O-space ADC registers (ADCSRB 0x23, ADCL/ADCH
/// 0x24/0x25, ADCSRA 0x26, ADMUX 0x27), the same SRAM and flash sizes as the
/// x5 parts and PUD in MCUCR bit 6 (0x55). They have two ports:
/// PINB/DDRB/PORTB 0x36..0x38 (PB0..PB3) and PINA/DDRA/PORTA 0x39..0x3B. The
/// ADC is ADC0..7 on PA0..PA7 with MUX5:0 in ADMUX bits 5:0, so ADLAR moves to
/// ADCSRB bit 4 (see [`Avr::tiny_x4_adc_input`]). Timers, USI and the
/// watchdog are stored only and no interrupt is taken.
///
/// The ATtiny13/13A (avr-libc `iotn13.h`/`iotn13a.h`) keep the x5 parts'
/// I/O-space ADC registers (ADCSRB 0x23, ADCL/ADCH 0x24/0x25, ADCSRA 0x26,
/// ADMUX 0x27), PINB/DDRB/PORTB 0x36..0x38 as the only port and PUD in MCUCR
/// bit 6 (0x55), with 1 KB flash and 64 B SRAM 0x60..0x9F (no SPH). The ADC is
/// ADC0..3 on PB5, PB2, PB4, PB3 with only MUX1:0 and one reference bit,
/// REFS0 (see [`Avr::tiny13_adc_input`]). Timer0, the watchdog and the EEPROM
/// are stored only and no interrupt is taken.
///
/// The ATmega128 (avr-libc `iom128.h`) keeps the classic I/O-space layout for
/// ports A..D (PINx/DDRx/PORTx at 0x30, 0x33, 0x36, 0x39), ADCL/ADCH
/// 0x24/0x25, ADCSRA 0x26, ADMUX 0x27 (no ADCSRB), SP/SREG 0x5D..0x5F and
/// SFIOR.PUD (bit 2) at 0x40, RAMPZ at 0x5B. Port E is PINE/DDRE/PORTE
/// 0x21..0x23 and port G PING/DDRG/PORTG 0x63..0x65 in the extended I/O space
/// (0x60..0xFF, stored), while port F is split: PINF is 0x20, DDRF/PORTF are
/// 0x61/0x62. Its bus mirror is therefore remapped to [`MEGA128_PORTF_MIRROR`].
/// SRAM is 0x100..0x10FF and flash 128 KB, the map avr-gcc links for (the
/// ATmega103-compatibility fuse, which moves SRAM to 0x60, is taken as
/// unprogrammed). The ADC is ADC0..7 on PF0..PF7 with MUX4:0: 0x1E is the
/// 1.23 V bandgap, 0x1F GND, the differential/gain codes 0x08..0x1D read 0;
/// REFS = 11 is the internal 2.56 V (ATmega128 datasheet 2467X §23.8.1,
/// Tables 96/97). As on the classic cores, timers, USARTs, SPI and TWI are
/// stored only and no interrupt is taken.
///
/// The ATmega32U4 (avr-libc `iom32u4.h`; Arduino Leonardo/Micro) keeps the
/// ATmega328P's data map for what this core shares: Timer0/1, SPI, TWI,
/// MCUCR.PUD (bit 4) and ADCL..ADMUX at 0x78..0x7C, with SRAM 0x100..0xAFF
/// (2.5 KB) and 32 KB flash. Ports B..F are PINx/DDRx/PORTx at 0x23..0x31 (no
/// port A). Its vector table is the ATmega1281's from INT0 to EE READY, with
/// TWI at slot 37 and SPM READY at 38 ([`u4_vector`]). The ADC has no ADC2/ADC3
/// and puts MUX5 in ADCSRB bit 5, not the 2560's bit 3 (see
/// [`Avr::u4_adc_input`]). Not modelled: USB, USART1 at 0xC8..0xCE, Timer3/4,
/// INTn/PCINT (INT0/INT1 are PD0/PD1 here; the registers are stored only) and
/// the OC pads (OC0A PB7, OC1A..C PB5..PB7; they follow `PORTx`).
///
/// The ATtiny1634 (avr-libc `iotn1634.h`) puts the ADC at the bottom of the
/// I/O space: ADCL/ADCH 0x20/0x21, ADCSRB 0x22, ADCSRA 0x23, ADMUX 0x24. Its
/// three ports are PINx/DDRx/PORTx/PUEx groups of four, port C at 0x27..0x2A,
/// B at 0x2B..0x2E and A at 0x2F..0x32. A pull-up is enabled by PUEx, not by
/// PORTx on an input, and there is no PUD bit (ATtiny1634 datasheet 8303 §12.2).
/// So the PORTx byte mirrored to the bus-side `avr_gpio` window is
/// `(PORTx & DDRx) | (PUEx & !DDRx)`, which that model reads as the drive on
/// outputs and the pull-up on inputs. Extended I/O is 0x60..0xFF (stored),
/// SRAM 0x100..0x4FF and flash 16 KB. ADLAR is ADCSRB bit 3 and the ADC is
/// ADC0..11 on PA3..PA7, PB0..PB3, PC0..PC2 with MUX3:0 (see
/// [`Avr::tiny1634_adc_input`]). Timers, USARTs, USI/TWI and the analog
/// comparator are stored only and no interrupt is taken.
///
/// The ATtiny2313/2313A and 4313 (avr-libc `iotn2313.h`, `iotn2313a.h`,
/// `iotn4313.h`) have no ADC. Their analog input is the comparator: ACSR at
/// I/O 0x08 (data 0x28) with ACO (bit 5) high while AIN0 (PB0) is above AIN1
/// (PB1), and ACBG (bit 6) putting the 1.1 V bandgap on the positive input
/// (see [`Avr::comparator_acsr`]). Ports D, B, A are PINx/DDRx/PORTx at
/// 0x30, 0x36, 0x39 (as on the classic parts, with no port C), MCUCR.PUD is
/// bit 7, SRAM is 0x60..0xDF (4313: 0x60..0x15F) and flash 2 KB (4313:
/// 4 KB). 0x24..0x27 are PCMSK1/PCMSK2 and timer registers, not an ADC, so
/// they are stored. Timers, the USART, USI, PCINT and the comparator's
/// ACI/ACIE/ACIC are stored only and no interrupt is taken.
///
/// The AT90USB162 (avr-libc `iousb162.h`) keeps the ATmega328P's data map for
/// what this core shares: ports B..D at 0x23..0x2B, Timer0/1, SPI and
/// MCUCR.PUD (bit 4), with SRAM 0x100..0x2FF and 16 KB flash. It has no ADC,
/// so 0x78..0x7C are stored, not converted. Its analog input is the
/// comparator: ACSR at 0x50 (I/O 0x30, ACD 7, ACBG 6, ACO 5, ACI 4) with AIN0
/// on PD1 and AIN1 on PD2 (datasheet 7707 §2.2.4), composed the same way as
/// the ATtiny2313's (see [`Avr::comparator_acsr`]). Its vector table is
/// [`usb162_vector`]. Not modelled: USB, USART1 at 0xC8..0xCE, INT0..7 and
/// PCINT0/1 (INT0 is PD0 here; the registers are stored only), and the
/// Timer1 channel C. The 328P's Timer2, USART0 and TWI are not gated off.
///
/// The ATmega16M1/64M1 (avr-libc `iom16m1.h`/`iom64m1.h`, which differ only
/// in memory sizes) keep the ATmega328P's data map for what this core shares:
/// Timer0/1, SPI, MCUCR.PUD (bit 4) and ADCL..ADMUX at 0x78..0x7C, with
/// ports B..E at 0x23..0x2E (no port A; port E is PE0..PE2), SRAM from 0x100
/// to 0x4FF (16M1) or 0x10FF (64M1) and 16/64 KB flash. Their ADC is
/// ADC0..10 on PE2, PD4, PD5, PD6, PB7, PB2, PB5, PB6, PC4, PC5, PC6 with
/// MUX4:0 (see [`Avr::m1_adc_input`]). Their vector table is [`m1_vector`].
/// Not modelled: the PSC, CAN, LIN/UART, the four analog comparators and the
/// amplifiers, the DAC and the current source; INT0..3 (on PD6, PB2, PB5,
/// PC0) and PCINT0..3 are stored only, and so are the OC pads (OC0A PD3,
/// OC0B PE1, OC1A PD2, OC1B PC1; they follow `PORTx`). The 328P's Timer2,
/// USART0 and TWI are not gated off.
///
/// The ATmega169P/169PA (avr-libc `iom169p.h`/`iom169pa.h`) keep the
/// ATmega328P's data map for Timer1, USART0, SPI, EEPROM, MCUCR.PUD (bit 4)
/// and ADCL..ADMUX at 0x78..0x7C, with ports A..G at 0x20..0x34 (the
/// ATmega1281's addresses; port G is PG0..PG5), SRAM 0x100..0x4FF and 16 KB
/// flash. Their ADC is ADC0..7 on PF0..PF7 with MUX4:0, including the gain-1
/// differential codes (see [`Avr::m169p_adc_code`]). Their vector table is
/// [`m169p_vector`]. Not modelled: the LCD controller (0xE4..0xFE, stored),
/// USI (0xB8..0xBA, where the 328P's TWI is not gated off), and Timer0/2,
/// whose clock select is in TCCR0A/TCCR2A here, not in the 328P's TCCRnB
/// (0x45/0xB1), so they do not count. INT0 (PD1) and PCINT0..15 (ports E
/// and B) are stored only, and so are the OC pads (OC0A PB4, OC1A/B PB5/PB6,
/// OC2A PB7; they follow `PORTx`).
///
/// The ATmega328PB (avr-libc `iom328pb.h`, simavr `sim_mega328pb.c`) is the
/// ATmega328P's data map, memory sizes, ADC (MUX3:0, 0x0E bandgap, 0x0F GND,
/// REFS = 11 the internal 1.1 V) and vectors 1..25, plus port E (PE0..PE3)
/// at 0x2C..0x2E. ADC6/ADC7 are bonded to PE2/PE3 rather than to dedicated
/// pads. Not modelled: USART1, SPI1, TWI1, Timer3/4, the PTC and the vectors
/// 26..44 they add; their registers are stored only. PCINT3 (port E) is
/// stored only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvrModel {
    #[default]
    Atmega328p,
    Atmega1281,
    Atmega2560,
    Atmega8,
    Atmega16,
    Atmega32,
    Atmega164,
    Atmega324,
    Atmega644,
    Atmega1284,
    Atmega128Rf,
    Attiny25,
    Attiny45,
    Attiny85,
    Attiny24,
    Attiny44,
    Attiny84,
    Atmega128,
    Attiny13,
    Atmega32u4,
    Attiny1634,
    Attiny2313,
    Attiny4313,
    At90usb162,
    Atmega16m1,
    Atmega64m1,
    Atmega169p,
    Atmega328pb,
}

impl AvrModel {
    /// The model a chip descriptor names. atmega1280/atmega1281 select the
    /// 128 KB two-byte-PC mega core (the 1280 is the 100-pin die: same data
    /// map as the 2560, ports H..L included, which this core already has);
    /// atmega2560/atmega2561/mega2560 select the 256 KB three-byte-PC mega
    /// core. atmega8/atmega16/atmega32 (and their `a` variants) select the
    /// classic I/O-space cores; the part name must match exactly, because
    /// atmega88, atmega168, atmega16m1 and atmega328 are other maps. The
    /// ATmega164/324/644/1284 (with their `a`/`p`/`pa` variants) select the
    /// port-A cores, again by exact name (atmega644rfr2 is another map).
    /// atmega128rfa1/atmega128rfr2 select the 128 KB RF core (exact name:
    /// the 64/256/644/1284/2564RFR2 have other memory sizes). attiny25/45/85
    /// select the ATtiny x5 I/O-space cores and attiny24/44/84 (with their
    /// `a` variants) the ATtiny x4 cores (exact name: the ATtiny261/461/861
    /// are other maps); attiny13/attiny13a select the ATtiny13 core and
    /// atmega32u4 the ATmega32U4 core (exact name: the 16U4 has less SRAM);
    /// attiny1634 selects the ATtiny1634 core; attiny2313/attiny2313a and
    /// attiny4313 the comparator-only ATtiny2313/4313 cores, and at90usb162
    /// the comparator-only AT90USB162 core (exact name: the AT90USB82 has
    /// less flash and SRAM). atmega16m1 and atmega64m1 select the M1 cores
    /// (exact name: the 32M1 has other memory sizes and the 16M1 is not the
    /// classic ATmega16). atmega169p/atmega169pa select the ATmega169P core
    /// (exact name: the original ATmega169 is another revision), and
    /// atmega328pb the ATmega328PB core (the 328P plus port E).
    /// atmega128/atmega128a select the ATmega128
    /// core (exact name: atmega1280, atmega1281, atmega1284 and atmega128rfa1
    /// are other maps). Other names default to the ATmega328P.
    pub fn from_chip_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        let part = n.rsplit(['-', '_', ' ']).next().unwrap_or(&n);
        match part {
            "atmega8" | "atmega8a" => return AvrModel::Atmega8,
            "atmega16" | "atmega16a" => return AvrModel::Atmega16,
            "atmega32" | "atmega32a" => return AvrModel::Atmega32,
            "atmega164" | "atmega164a" | "atmega164p" | "atmega164pa" => {
                return AvrModel::Atmega164
            }
            "atmega324" | "atmega324a" | "atmega324p" | "atmega324pa" => {
                return AvrModel::Atmega324
            }
            "atmega644" | "atmega644a" | "atmega644p" | "atmega644pa" => {
                return AvrModel::Atmega644
            }
            "atmega1284" | "atmega1284p" => return AvrModel::Atmega1284,
            "atmega128rfa1" | "atmega128rfr2" => return AvrModel::Atmega128Rf,
            "attiny25" => return AvrModel::Attiny25,
            "attiny45" => return AvrModel::Attiny45,
            "attiny85" => return AvrModel::Attiny85,
            "attiny24" | "attiny24a" => return AvrModel::Attiny24,
            "attiny44" | "attiny44a" => return AvrModel::Attiny44,
            "attiny84" | "attiny84a" => return AvrModel::Attiny84,
            "atmega128" | "atmega128a" => return AvrModel::Atmega128,
            "attiny13" | "attiny13a" => return AvrModel::Attiny13,
            "atmega32u4" => return AvrModel::Atmega32u4,
            "attiny1634" => return AvrModel::Attiny1634,
            "attiny2313" | "attiny2313a" => return AvrModel::Attiny2313,
            "attiny4313" => return AvrModel::Attiny4313,
            "at90usb162" => return AvrModel::At90usb162,
            "atmega16m1" => return AvrModel::Atmega16m1,
            "atmega64m1" => return AvrModel::Atmega64m1,
            "atmega169p" | "atmega169pa" => return AvrModel::Atmega169p,
            "atmega328pb" => return AvrModel::Atmega328pb,
            _ => {}
        }
        if n.contains("1280") || n.contains("1281") {
            AvrModel::Atmega1281
        } else if n.contains("2560") || n.contains("2561") {
            AvrModel::Atmega2560
        } else {
            AvrModel::Atmega328p
        }
    }
    pub fn flash_size(self) -> usize {
        match self {
            AvrModel::Atmega328p | AvrModel::Atmega328pb => FLASH_SIZE,
            AvrModel::Atmega1281 => 128 * 1024,
            AvrModel::Atmega2560 => 256 * 1024,
            AvrModel::Atmega8 => 8 * 1024,
            AvrModel::Atmega16 => 16 * 1024,
            AvrModel::Atmega32 => 32 * 1024,
            AvrModel::Atmega164 => 16 * 1024,
            AvrModel::Atmega324 => 32 * 1024,
            AvrModel::Atmega644 => 64 * 1024,
            AvrModel::Atmega1284 | AvrModel::Atmega128Rf | AvrModel::Atmega128 => 128 * 1024,
            AvrModel::Attiny25 | AvrModel::Attiny24 => 2 * 1024,
            AvrModel::Attiny45 | AvrModel::Attiny44 => 4 * 1024,
            AvrModel::Attiny85 | AvrModel::Attiny84 => 8 * 1024,
            AvrModel::Attiny13 => 1024,
            AvrModel::Atmega32u4 => 32 * 1024,
            AvrModel::Attiny1634 => 16 * 1024,
            AvrModel::Attiny2313 => 2 * 1024,
            AvrModel::Attiny4313 => 4 * 1024,
            AvrModel::At90usb162 => 16 * 1024,
            AvrModel::Atmega16m1 => 16 * 1024,
            AvrModel::Atmega64m1 => 64 * 1024,
            AvrModel::Atmega169p => 16 * 1024,
        }
    }
    pub fn sram_start(self) -> u16 {
        match self {
            AvrModel::Atmega328p
            | AvrModel::Atmega328pb
            | AvrModel::Atmega164
            | AvrModel::Atmega324
            | AvrModel::Atmega644
            | AvrModel::Atmega1284
            | AvrModel::Atmega128
            | AvrModel::Atmega32u4
            | AvrModel::Attiny1634
            | AvrModel::At90usb162
            | AvrModel::Atmega16m1
            | AvrModel::Atmega64m1
            | AvrModel::Atmega169p => SRAM_START,
            AvrModel::Atmega1281 | AvrModel::Atmega2560 | AvrModel::Atmega128Rf => 0x0200,
            AvrModel::Atmega8
            | AvrModel::Atmega16
            | AvrModel::Atmega32
            | AvrModel::Attiny25
            | AvrModel::Attiny45
            | AvrModel::Attiny85
            | AvrModel::Attiny24
            | AvrModel::Attiny44
            | AvrModel::Attiny84
            | AvrModel::Attiny13
            | AvrModel::Attiny2313
            | AvrModel::Attiny4313 => 0x0060,
        }
    }
    pub fn ramend(self) -> u16 {
        match self {
            AvrModel::Atmega328p | AvrModel::Atmega328pb => RAMEND,
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => 0x21FF,
            AvrModel::Atmega8 | AvrModel::Atmega16 => 0x045F,
            AvrModel::Atmega32 => 0x085F,
            AvrModel::Atmega164 => 0x04FF,
            AvrModel::Atmega324 => 0x08FF,
            AvrModel::Atmega644 | AvrModel::Atmega128 => 0x10FF,
            AvrModel::Atmega1284 => 0x40FF,
            AvrModel::Atmega128Rf => 0x41FF,
            AvrModel::Attiny25 | AvrModel::Attiny24 => 0x00DF,
            AvrModel::Attiny45 | AvrModel::Attiny44 => 0x015F,
            AvrModel::Attiny85 | AvrModel::Attiny84 => 0x025F,
            AvrModel::Attiny13 => 0x009F,
            AvrModel::Atmega32u4 => 0x0AFF,
            AvrModel::Attiny1634 => 0x04FF,
            AvrModel::Attiny2313 => 0x00DF,
            AvrModel::Attiny4313 => 0x015F,
            AvrModel::At90usb162 => 0x02FF,
            AvrModel::Atmega16m1 => 0x04FF,
            AvrModel::Atmega64m1 => 0x10FF,
            AvrModel::Atmega169p => 0x04FF,
        }
    }
    /// The classic ATmega8/16/32 I/O-space map (see [`AvrModel`]).
    pub fn is_classic(self) -> bool {
        matches!(
            self,
            AvrModel::Atmega8 | AvrModel::Atmega16 | AvrModel::Atmega32
        )
    }
    /// The ATtiny25/45/85 map (see [`AvrModel`]).
    pub fn is_tiny_x5(self) -> bool {
        matches!(
            self,
            AvrModel::Attiny25 | AvrModel::Attiny45 | AvrModel::Attiny85
        )
    }
    /// The ATtiny24/44/84 map (see [`AvrModel`]).
    pub fn is_tiny_x4(self) -> bool {
        matches!(
            self,
            AvrModel::Attiny24 | AvrModel::Attiny44 | AvrModel::Attiny84
        )
    }
    /// The ATtiny13/13A map (see [`AvrModel`]).
    pub fn is_tiny13(self) -> bool {
        self == AvrModel::Attiny13
    }
    /// The ATtiny1634 map (see [`AvrModel`]).
    pub fn is_tiny1634(self) -> bool {
        self == AvrModel::Attiny1634
    }
    /// The ATtiny2313/2313A/4313 map, comparator and no ADC (see [`AvrModel`]).
    pub fn is_tiny2313(self) -> bool {
        matches!(self, AvrModel::Attiny2313 | AvrModel::Attiny4313)
    }
    /// The AT90USB162 map, comparator and no ADC (see [`AvrModel`]).
    pub fn is_usb162(self) -> bool {
        self == AvrModel::At90usb162
    }
    /// The ATmega16M1/64M1 map (see [`AvrModel`]).
    pub fn is_m1(self) -> bool {
        matches!(self, AvrModel::Atmega16m1 | AvrModel::Atmega64m1)
    }
    /// The ATmega169P/169PA map (see [`AvrModel`]).
    pub fn is_m169p(self) -> bool {
        self == AvrModel::Atmega169p
    }
    /// Data address of ACSR on a part whose only analog input is the analog
    /// comparator: 0x28 on the ATtiny2313/4313, 0x50 on the AT90USB162.
    pub fn comparator_acsr(self) -> Option<u16> {
        if self.is_tiny2313() {
            Some(TINY2313_ACSR)
        } else if self.is_usb162() {
            Some(USB162_ACSR)
        } else {
            None
        }
    }
    /// The ATmega128 map (see [`AvrModel`]).
    pub fn is_mega128(self) -> bool {
        self == AvrModel::Atmega128
    }
    /// The ADC and ports in the 64-byte I/O space, no 328P register at its
    /// 328P address: the classic ATmega8/16/32, the ATtiny25/45/85 and
    /// 24/44/84 and the ATtiny13 (SRAM from 0x60), the ATmega128 and the
    /// ATtiny1634 (extended I/O 0x60..0xFF, SRAM from 0x100), and the
    /// ATtiny2313/4313 (SRAM from 0x60, no ADC).
    pub fn is_io_space(self) -> bool {
        self.is_classic()
            || self.is_tiny_x5()
            || self.is_tiny_x4()
            || self.is_tiny13()
            || self.is_mega128()
            || self.is_tiny1634()
            || self.is_tiny2313()
    }
    /// An I/O-space core with ADCL/ADCH/ADCSRA/ADMUX: every one but the
    /// ATtiny2313/4313.
    fn has_io_adc(self) -> bool {
        self.is_io_space() && !self.is_tiny2313()
    }
    /// Extended I/O 0x60..0xFF on an I/O-space core: stored registers.
    fn has_ext_io(self) -> bool {
        self.is_mega128() || self.is_tiny1634()
    }
    /// The ATmega164/324/644/1284 port-A map (see [`AvrModel`]).
    pub fn is_xx4(self) -> bool {
        matches!(
            self,
            AvrModel::Atmega164 | AvrModel::Atmega324 | AvrModel::Atmega644 | AvrModel::Atmega1284
        )
    }
    /// The ATmega32U4 map (see [`AvrModel`]).
    pub fn is_u4(self) -> bool {
        self == AvrModel::Atmega32u4
    }
    /// The ATmega128RFA1/128RFR2 1.8 V-AVDD ADC map (see [`AvrModel`]).
    pub fn is_rf(self) -> bool {
        self == AvrModel::Atmega128Rf
    }
    /// Bus base of this model's `avr_adc` input window (2 bytes per channel).
    /// The 2560's 16 channels move it past the port mirrors (PINF..PING sit
    /// at 0x1_002F..0x1_0034, where the 328P keeps its 8-channel window).
    pub fn adc_input_base(self) -> u64 {
        match self {
            AvrModel::Atmega328p
            | AvrModel::Atmega328pb
            | AvrModel::Atmega164
            | AvrModel::Atmega324
            | AvrModel::Atmega644
            | AvrModel::Atmega1284
            | AvrModel::At90usb162 => AVR_ADC_INPUT_BASE,
            AvrModel::Atmega1281
            | AvrModel::Atmega2560
            | AvrModel::Atmega128Rf
            | AvrModel::Atmega128
            | AvrModel::Atmega32u4
            | AvrModel::Atmega16m1
            | AvrModel::Atmega64m1
            | AvrModel::Atmega169p => AVR_MEGA_ADC_INPUT_BASE,
            AvrModel::Atmega8
            | AvrModel::Atmega16
            | AvrModel::Atmega32
            | AvrModel::Attiny25
            | AvrModel::Attiny45
            | AvrModel::Attiny85
            | AvrModel::Attiny24
            | AvrModel::Attiny44
            | AvrModel::Attiny84
            | AvrModel::Attiny13
            | AvrModel::Attiny1634
            | AvrModel::Attiny2313
            | AvrModel::Attiny4313 => AVR_CLASSIC_ADC_INPUT_BASE,
        }
    }
    /// `true` when the PC is pushed as 3 bytes (flash above 128 KB).
    pub fn three_byte_pc(self) -> bool {
        self == AvrModel::Atmega2560
    }
    /// Whether this model implements EIND, EIJMP and EICALL.
    pub fn supports_eind(self) -> bool {
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
/// ATmega164/324/644/1284 vector number (1-based, avr-libc `_VECTOR(n)` + 1)
/// for a vector this core numbers the ATmega328P way. INT2 (4) and PCINT3 (8)
/// push PCINT0..2 up one slot and WDT..USART0 TX up two; ADC and EE READY
/// move past ANALOG COMP, which keeps its slot.
pub fn xx4_vector(vec328: u32) -> u32 {
    match vec328 {
        2 | 3 => vec328,       // INT0, INT1
        4..=6 => vec328 + 1,   // PCINT0..2 -> 5..7
        7..=21 => vec328 + 2,  // WDT 9 .. TIMER0 OVF 19, SPI 20, USART0 RX/UDRE/TX 21..23
        22 | 23 => vec328 + 3, // ADC 25, EE READY 26
        24 => 24,              // ANALOG COMP
        25 | 26 => vec328 + 2, // TWI 27, SPM READY 28
        v => v,
    }
}
/// ATmega32U4 vector number (1-based, avr-libc `_VECTOR(n)` + 1) for a vector
/// this core numbers the ATmega328P way. INT0 through EE READY sit where the
/// ATmega1281 has them ([`mega2560_vector`]); TWI is slot 37 and SPM READY 38.
/// The 32U4 has no Timer2, PCINT1 or PCINT2, so their 328P vectors are never
/// raised.
pub fn u4_vector(vec328: u32) -> u32 {
    match vec328 {
        25 => 37, // TWI
        26 => 38, // SPM READY
        v => mega2560_vector(v),
    }
}
/// AT90USB162 vector number (1-based, avr-libc `_VECTOR(n)` + 1) for a vector
/// this core numbers the ATmega328P way. INT2..7 push PCINT0/1 to 10/11, the
/// two USB vectors put WDT at 14, and Timer1 COMPC (18) moves Timer1 OVF
/// through SPI to 19..23. The part has no PCINT2, Timer2, USART0, ADC or TWI;
/// firmware for it never enables them, and their 328P numbers pass through.
pub fn usb162_vector(vec328: u32) -> u32 {
    match vec328 {
        4 | 5 => vec328 + 6,   // PCINT0/1 -> 10/11
        7 => 14,               // WDT
        11..=13 => vec328 + 4, // TIMER1 CAPT/COMPA/COMPB -> 15..17
        14..=18 => vec328 + 5, // TIMER1 OVF 19, TIMER0 COMPA/B/OVF 20..22, SPI 23
        23 => 28,              // EE READY
        24 => 27,              // ANALOG COMP
        26 => 29,              // SPM READY
        v => v,
    }
}
/// ATmega16M1/64M1 vector number (1-based, avr-libc `_VECTOR(n)` + 1) for a
/// vector this core numbers the ATmega328P way. The four comparators, the PSC
/// and INT0..3 come first, so Timer1 starts at 12; CAN and LIN push PCINT0..2
/// to 23..25, and SPI, ADC, WDT, EE READY and SPM READY are 27..31. The part
/// has no Timer2, USART0 or TWI, and its comparators are not the 328P's;
/// firmware for it never enables them, and their 328P numbers pass through.
pub fn m1_vector(vec328: u32) -> u32 {
    match vec328 {
        2 | 3 => vec328 + 6,   // INT0, INT1 -> 8, 9
        4..=6 => vec328 + 19,  // PCINT0..2 -> 23..25
        7 => 29,               // WDT
        11..=17 => vec328 + 1, // TIMER1 CAPT 12 .. TIMER0 OVF 18
        18 => 27,              // SPI STC
        22 => 28,              // ADC
        23 => 30,              // EE READY
        26 => 31,              // SPM READY
        v => v,
    }
}
/// ATmega169P/169PA vector number (1-based, avr-libc `_VECTOR(n)` + 1) for a
/// vector this core numbers the ATmega328P way. The part has one INT, two
/// PCINT groups and one compare channel on Timer0/2, so PCINT0/1 are 3/4,
/// Timer2 COMP/OVF 5/6, Timer1 7..10, Timer0 COMP/OVF 11/12, SPI 13, USART
/// RX/UDRE/TX 14..16; after USI START/OVF come ANALOG COMP 19, ADC 20, EE
/// READY 21 and SPM READY 22. INT1, PCINT2, WDT, the B compare channels of
/// Timer0/2 and TWI do not exist; their 328P numbers pass through.
pub fn m169p_vector(vec328: u32) -> u32 {
    match vec328 {
        4 | 5 => vec328 - 1,   // PCINT0, PCINT1 -> 3, 4
        8 => 5,                // TIMER2 COMP
        10 => 6,               // TIMER2 OVF
        11..=14 => vec328 - 4, // TIMER1 CAPT/COMPA/COMPB/OVF -> 7..10
        15 => 11,              // TIMER0 COMP
        17..=21 => vec328 - 5, // TIMER0 OVF 12, SPI 13, USART RX/UDRE/TX 14..16
        22 => 20,              // ADC
        23 => 21,              // EE READY
        24 => 19,              // ANALOG COMP
        26 => 22,              // SPM READY
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
    /// SPI slaves attached directly to the CPU (unit tests / legacy take).
    ///
    /// Not `Clone`/`Debug`: trait objects. Production kits stay on the bus
    /// parking SPI controller and are clocked from SPDR via
    /// [`crate::bus::SystemBus::clock_parking_spi`] so inspect can see them.
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
/// `PINA` data-space address on the ATmega164/324/644/1284 (DDRA/PORTA follow).
pub const AVR_PINA: u16 = 0x0020;
/// `PINC` data-space address; `DDRC`/`PORTC` follow it.
pub const AVR_PINC: u16 = 0x0026;
/// `PIND` data-space address; `DDRD`/`PORTD` follow it.
pub const AVR_PIND: u16 = 0x0029;
/// Bus base of the `avr_adc` input window: two bytes of millivolts per channel.
pub const AVR_ADC_INPUT_BASE: u64 = 0x0001_0030;
/// Bus base of the ATmega2560's 16-channel `avr_adc` input window.
pub const AVR_MEGA_ADC_INPUT_BASE: u64 = 0x0001_0200;
/// Bus base of the classic ATmega8/16/32 `avr_adc` input window: just past
/// their I/O space, clear of the port mirrors at 0x1_0030..0x1_003B.
pub const AVR_CLASSIC_ADC_INPUT_BASE: u64 = 0x0001_0060;
/// AT90USB162 `ACSR` (I/O 0x30, avr-libc `iousb162.h`): the bits sit where
/// the ATtiny2313's do.
const USB162_ACSR: u16 = 0x0050;
/// Classic ATmega8/16/32 PINx data addresses, ports D, C, B, A.
pub const CLASSIC_PIN_ADDRS: [u16; 4] = [0x30, 0x33, 0x36, 0x39];
/// ATmega128 bus mirror of port F (PINF, DDRF, PORTF at +0, +1, +2): PINF
/// (0x20) and DDRF/PORTF (0x61/0x62) are not adjacent, and 0x1_0021 is PINE.
/// Past the 0x00..0xFF data I/O, so no other register mirror lands on it.
pub const MEGA128_PORTF_MIRROR: u64 = 0x0001_0100;
/// ATmega128 PINx data addresses, ports F, E, D, C, B, A, G.
pub const MEGA128_PIN_ADDRS: [u16; 7] = [0x20, 0x21, 0x30, 0x33, 0x36, 0x39, 0x63];
/// ATtiny1634 PINx data addresses, ports C, B, A (DDRx, PORTx, PUEx follow).
pub const TINY1634_PIN_ADDRS: [u16; 3] = [0x27, 0x2B, 0x2F];
/// ATtiny2313/2313A/4313 `ACSR` (I/O 0x08) and its bits (avr-libc
/// `iotn2313.h`): ACD 7, ACBG 6, ACO 5, ACI 4.
const TINY2313_ACSR: u16 = 0x0028;
const ACSR_ACD: u8 = 1 << 7;
const ACSR_ACBG: u8 = 1 << 6;
const ACSR_ACO: u8 = 1 << 5;
const ACSR_ACI: u8 = 1 << 4;
/// ATtiny2313/4313 `MCUCR.PUD` (MCUCR is I/O 0x35, as on the ATtiny x5).
const TINY2313_MCUCR_PUD: u8 = 1 << 7;
/// ATmega128 `SFIOR` (I/O 0x20); PUD is bit 2, as on the classic parts.
const MEGA128_SFIOR: u16 = 0x0040;
/// ATmega128 bandgap mux input (2467X Table 133, V_BG typ).
const MEGA128_BANDGAP_MV: u32 = 1230;
/// Classic `SFIOR` (I/O 0x30) and its pull-up-disable bit.
const CLASSIC_SFIOR: u16 = 0x0050;
const CLASSIC_SFIOR_PUD: u8 = 1 << 2;
/// ATtiny25/45/85 and 24/44/84 `MCUCR` (I/O 0x35) and its pull-up-disable bit.
const TINY_X5_MCUCR: u16 = 0x0055;
const TINY_X5_MCUCR_PUD: u8 = 1 << 6;
/// Internal references on the classic parts (ATmega8 datasheet §23, ATmega16/32
/// §21): REFS = 11 is 2.56 V (also on the ATmega164/324/644/1284); the bandgap mux input is 1.30 V on the ATmega8 and
/// 1.22 V on the 16/32.
const CLASSIC_INTERNAL_REF_MV: u32 = 2560;
const MEGA8_BANDGAP_MV: u32 = 1300;
const MEGA16_BANDGAP_MV: u32 = 1220;
/// AVcc on the 5 V boards this part is modelled for, and the internal bandgap.
const AVR_AVCC_MV: u32 = 5000;
const AVR_BANDGAP_MV: u32 = 1100;
/// ATmega128RFA1/128RFR2 ADC: AVDD from the internal regulator, the internal
/// references and the VBG mux input (8266F §27.11.1, Tables 27-10/27-11).
const RF_AVDD_MV: u32 = 1800;
const RF_REF_1V5_MV: u32 = 1500;
const RF_REF_1V6_MV: u32 = 1600;
const RF_BANDGAP_MV: u32 = 1200;

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
        matches!(
            self.model,
            AvrModel::Atmega1281 | AvrModel::Atmega2560 | AvrModel::Atmega128Rf
        )
    }

    /// INT0/INT1 and PCINT0..2 wired to the 328P's pins. The mega, the port-A
    /// parts and the 32U4 route them elsewhere, so their registers are stored
    /// only.
    #[inline]
    fn has_328p_ext_int(&self) -> bool {
        !self.is_mega()
            && !self.model.is_xx4()
            && !self.model.is_u4()
            && !self.model.is_usb162()
            && !self.model.is_m1()
            && !self.model.is_m169p()
    }

    /// `PINx` of a port this model has.
    #[inline]
    fn is_pin_reg(&self, addr: u16) -> bool {
        match self.model {
            AvrModel::Atmega328p | AvrModel::At90usb162 => {
                matches!(addr, AVR_PINB | AVR_PINC | AVR_PIND)
            }
            AvrModel::Atmega164
            | AvrModel::Atmega324
            | AvrModel::Atmega644
            | AvrModel::Atmega1284 => {
                matches!(addr, AVR_PINA | AVR_PINB | AVR_PINC | AVR_PIND)
            }
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => MEGA2560_PIN_ADDRS.contains(&addr),
            AvrModel::Atmega128Rf | AvrModel::Atmega169p => MEGA2560_PIN_ADDRS[..7].contains(&addr),
            AvrModel::Atmega8 => CLASSIC_PIN_ADDRS[..3].contains(&addr),
            AvrModel::Atmega16 | AvrModel::Atmega32 => CLASSIC_PIN_ADDRS.contains(&addr),
            AvrModel::Atmega128 => MEGA128_PIN_ADDRS.contains(&addr),
            AvrModel::Attiny25 | AvrModel::Attiny45 | AvrModel::Attiny85 | AvrModel::Attiny13 => {
                addr == 0x36
            }
            AvrModel::Attiny24 | AvrModel::Attiny44 | AvrModel::Attiny84 => {
                matches!(addr, 0x36 | 0x39)
            }
            AvrModel::Atmega32u4 => MEGA2560_PIN_ADDRS[1..6].contains(&addr),
            AvrModel::Atmega16m1 | AvrModel::Atmega64m1 | AvrModel::Atmega328pb => {
                MEGA2560_PIN_ADDRS[1..5].contains(&addr)
            }
            AvrModel::Attiny1634 => TINY1634_PIN_ADDRS.contains(&addr),
            AvrModel::Attiny2313 | AvrModel::Attiny4313 => {
                matches!(addr, 0x30 | 0x36 | 0x39)
            }
        }
    }

    /// `PINx`/`DDRx`/`PORTx` of a port this model has: mirrored to the bus.
    #[inline]
    fn is_port_reg(&self, addr: u16) -> bool {
        match self.model {
            AvrModel::Atmega328p | AvrModel::At90usb162 => {
                (AVR_PINB..=AVR_PIND + 2).contains(&addr)
            }
            AvrModel::Atmega164
            | AvrModel::Atmega324
            | AvrModel::Atmega644
            | AvrModel::Atmega1284 => (AVR_PINA..=AVR_PIND + 2).contains(&addr),
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => {
                (0x20..=0x34).contains(&addr) || (0x100..=0x10B).contains(&addr)
            }
            AvrModel::Atmega128Rf | AvrModel::Atmega169p => (0x20..=0x34).contains(&addr),
            AvrModel::Atmega8 => (0x30..=0x38).contains(&addr),
            AvrModel::Atmega16 | AvrModel::Atmega32 => (0x30..=0x3B).contains(&addr),
            AvrModel::Atmega128 => {
                (0x20..=0x23).contains(&addr)
                    || (0x30..=0x3B).contains(&addr)
                    || (0x61..=0x65).contains(&addr)
            }
            AvrModel::Attiny25 | AvrModel::Attiny45 | AvrModel::Attiny85 | AvrModel::Attiny13 => {
                (0x36..=0x38).contains(&addr)
            }
            AvrModel::Attiny24 | AvrModel::Attiny44 | AvrModel::Attiny84 => {
                (0x36..=0x3B).contains(&addr)
            }
            AvrModel::Atmega32u4 => (0x23..=0x31).contains(&addr),
            AvrModel::Atmega16m1 | AvrModel::Atmega64m1 | AvrModel::Atmega328pb => {
                (0x23..=0x2E).contains(&addr)
            }
            // PUEx (PINx + 3) is not part of the `avr_gpio` window.
            AvrModel::Attiny1634 => (0x27..=0x32).contains(&addr) && (addr - 0x27) % 4 != 3,
            AvrModel::Attiny2313 | AvrModel::Attiny4313 => {
                (0x30..=0x32).contains(&addr) || (0x36..=0x3B).contains(&addr)
            }
        }
    }

    /// DDRx and PORTx of the port whose PINx is `pin`: the next two addresses,
    /// except the ATmega128's port F (PINF 0x20, DDRF 0x61, PORTF 0x62).
    #[inline]
    fn ddr_port_of(&self, pin: u16) -> (u16, u16) {
        if self.model.is_mega128() && pin == 0x20 {
            (0x61, 0x62)
        } else {
            (pin + 1, pin + 2)
        }
    }

    /// Bus address mirroring the port register at data address `addr`:
    /// `AVR_IO_MIRROR_BASE + addr`, except the ATmega128's port F, which is
    /// gathered at [`MEGA128_PORTF_MIRROR`] so the `avr_gpio` window sees
    /// PINF/DDRF/PORTF at +0/+1/+2.
    #[inline]
    fn port_mirror_addr(&self, addr: u16) -> u64 {
        match addr {
            0x20 if self.model.is_mega128() => MEGA128_PORTF_MIRROR,
            0x61 | 0x62 if self.model.is_mega128() => MEGA128_PORTF_MIRROR + u64::from(addr - 0x60),
            _ => AVR_IO_MIRROR_BASE + u64::from(addr),
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
                let (ddr_addr, port_addr) = self.ddr_port_of(a);
                let ddr = self.io[(ddr_addr - 0x20) as usize];
                let port = self.io[(port_addr - 0x20) as usize];
                // Propagated, not discarded: a chip yaml that maps no window
                // for this port has no input state for this register at all,
                // and a swallowed error would answer with a fabricated low — a
                // released button reading as pressed forever, green.
                let external = bus.read_u8(self.port_mirror_addr(addr))?;
                Ok((port & ddr) | (external & !ddr))
            }
            a if self.model.comparator_acsr() == Some(a) => self.comparator_acsr(a, bus),
            a if self.model.is_io_space() => self.classic_io_read(a),
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
            // The AT90USB162 has no ADC: 0x78..0x7C are stored below.
            0x0078 if !self.model.is_usb162() => Ok(self.adcl),
            0x0079 if !self.model.is_usb162() => Ok(self.adch),
            0x007A if !self.model.is_usb162() => Ok(self.adcsra),
            0x007C if !self.model.is_usb162() => Ok(self.admux),
            0x007B if self.model.is_rf() => Ok(self.rf_adcsrb()),
            ext_int::ADDR_PCIFR..=ext_int::ADDR_EIMSK
            | ext_int::ADDR_SMCR
            | ext_int::ADDR_PCICR
            | ext_int::ADDR_EICRA
            | ext_int::ADDR_PCMSK0..=ext_int::ADDR_PCMSK2
                if self.has_328p_ext_int() || addr == ext_int::ADDR_SMCR =>
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
            a if self.model.is_io_space() => self.classic_io_write(a, value, bus),
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
                // Sample D/C for every device that declares a `dc_source`
                // (same semantics as SystemBus::maybe_latch_dc) — generic for
                // any SPI panel with a DC line, not device-specific.
                let mosi = value;
                let mut miso = 0u8;
                {
                    let sources: Vec<(usize, u64, u8)> = self
                        .spi_devices
                        .iter()
                        .enumerate()
                        .filter_map(|(i, d)| d.dc_source().map(|(a, b)| (i, a, b)))
                        .collect();
                    for (i, addr, bit) in sources {
                        let level = bus
                            .read_u32(addr)
                            .map(|v| (v >> bit) & 1 != 0)
                            .unwrap_or(false);
                        self.spi_devices[i].set_dc_level(level);
                    }
                }
                for dev in &mut self.spi_devices {
                    let resp = dev.transfer(mosi);
                    if resp != 0 {
                        miso = resp;
                    }
                }
                // Kits left on the parking SPI controller (not taken): latch
                // D/C and clock them so inspect / display_artifact still see
                // the panel. Bus::clock_parking_spi — no SystemBus downcast.
                let bus_miso = bus.clock_parking_spi(mosi);
                if bus_miso != 0 {
                    miso = bus_miso;
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
            // ACO is the comparator's output and ACI is write-one-to-clear
            // (never set here: no interrupt is taken), so neither is stored.
            USB162_ACSR if self.model.is_usb162() => {
                self.io[usize::from(USB162_ACSR - 0x20)] = value & !(ACSR_ACO | ACSR_ACI);
                Ok(())
            }
            0x0078..=0x007C if self.model.is_usb162() => {
                self.io[usize::from(addr - 0x20)] = value;
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
            0x007A => self.write_adcsra(value, bus),
            0x007C => {
                self.admux = value;
                Ok(())
            }
            ext_int::ADDR_PCIFR..=ext_int::ADDR_EIMSK
            | ext_int::ADDR_SMCR
            | ext_int::ADDR_PCICR
            | ext_int::ADDR_EICRA
            | ext_int::ADDR_PCMSK0..=ext_int::ADDR_PCMSK2
                if self.has_328p_ext_int() || addr == ext_int::ADDR_SMCR =>
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

    /// The I/O-space cores' I/O, the ATmega128's extended I/O, and SRAM (see
    /// [`AvrModel`]). PINx is composed by the shared arm in [`Self::data_read`].
    fn classic_io_read(&self, addr: u16) -> SimResult<u8> {
        let adc = self.model.has_io_adc();
        let adcl = self.io_adcl();
        match addr {
            a if adc && a == adcl => Ok(self.adcl),
            a if adc && a == adcl + 1 => Ok(self.adch),
            a if adc && a == self.io_adcsra() => Ok(self.adcsra),
            a if adc && a == self.io_admux() => Ok(self.admux),
            0x005D => Ok((self.sp & 0xFF) as u8),
            0x005E => Ok((self.sp >> 8) as u8),
            0x005F => Ok(self.sreg),
            0x0020..=0x005F => Ok(self.io[(addr - 0x20) as usize]),
            0x0060..=0x00FF if self.model.has_ext_io() => Ok(self.io[(addr - 0x20) as usize]),
            a if (self.sram_start..=self.ramend).contains(&a) => {
                Ok(self.sram[(a - self.sram_start) as usize])
            }
            _ => Err(SimulationError::MemoryViolation(addr as u64)),
        }
    }

    fn classic_io_write(&mut self, addr: u16, value: u8, bus: &mut dyn Bus) -> SimResult<()> {
        let adc = self.model.has_io_adc();
        let adcl = self.io_adcl();
        match addr {
            a if adc && a == adcl => self.adcl = value,
            a if adc && a == adcl + 1 => self.adch = value,
            a if adc && a == self.io_adcsra() => return self.write_adcsra(value, bus),
            a if adc && a == self.io_admux() => self.admux = value,
            // ACO is the comparator's output and ACI is write-one-to-clear
            // (never set here: no interrupt is taken), so neither is stored.
            TINY2313_ACSR if self.model.is_tiny2313() => {
                self.io[usize::from(TINY2313_ACSR - 0x20)] = value & !(ACSR_ACO | ACSR_ACI);
            }
            0x005D => self.sp = (self.sp & 0xFF00) | value as u16,
            0x005E => self.sp = (self.sp & 0x00FF) | ((value as u16) << 8),
            0x005F => self.sreg = value,
            0x0027..=0x0032 if self.model.is_tiny1634() => {
                self.io[(addr - 0x20) as usize] = value;
                self.tiny1634_mirror_port(addr, value, bus);
            }
            0x0020..=0x005F => {
                let old = self.io[(addr - 0x20) as usize];
                self.io[(addr - 0x20) as usize] = value;
                if self.is_port_reg(addr) {
                    // Best-effort mirror, as on the 328P: IN/OUT must not fail.
                    let _mirror = bus.write_u8(self.port_mirror_addr(addr), value);
                } else if let Some(pud) = self.io_space_pud(addr) {
                    if (old ^ value) & pud != 0 {
                        bus.set_pull_ups_disabled(value & pud != 0);
                    }
                }
            }
            0x0060..=0x00FF if self.model.has_ext_io() => {
                self.io[(addr - 0x20) as usize] = value;
                if self.is_port_reg(addr) {
                    let _mirror = bus.write_u8(self.port_mirror_addr(addr), value);
                }
            }
            a if (self.sram_start..=self.ramend).contains(&a) => {
                self.sram[(a - self.sram_start) as usize] = value;
                bus.write_u8(a as u64, value)?;
            }
            _ => return Err(SimulationError::MemoryViolation(addr as u64)),
        }
        Ok(())
    }

    /// ADCL on the I/O-space cores (ADCH follows it): 0x20 on the ATtiny1634,
    /// 0x24 elsewhere.
    fn io_adcl(&self) -> u16 {
        if self.model.is_tiny1634() {
            0x20
        } else {
            0x24
        }
    }

    /// ADCSRA on the I/O-space cores: 0x23 on the ATtiny1634, 0x26 elsewhere.
    fn io_adcsra(&self) -> u16 {
        if self.model.is_tiny1634() {
            0x23
        } else {
            0x26
        }
    }

    /// ADMUX on the I/O-space cores: 0x24 on the ATtiny1634, 0x27 elsewhere.
    fn io_admux(&self) -> u16 {
        if self.model.is_tiny1634() {
            0x24
        } else {
            0x27
        }
    }

    /// ACSR at `acsr` as firmware reads it on a comparator-only part: the
    /// stored ACD, ACBG, ACIE, ACIC and ACIS1:0 with ACO (bit 5) composed from
    /// the pins (ATtiny2313 datasheet 2543 §16.1, ATtiny2313A/4313 8246 §15.2,
    /// AT90USB162 7707 §19.1).
    ///
    /// The positive input is AIN0 (PB0 on the ATtiny2313/4313, PD1 on the
    /// AT90USB162), or the 1.1 V bandgap when ACBG (bit 6) is set; the
    /// negative input is AIN1 (PB1, or PD2). Both pin voltages are
    /// the millivolts the bus-side `avr_adc` input window holds for channels
    /// 0 and 1, so any routed analog device reaches them. ACO is 1 while the
    /// positive input is strictly above the negative one; there is no modelled
    /// hysteresis or offset. ACD (bit 7) powers the comparator off and ACO
    /// then reads 0. ACI never sets: the output is evaluated on read, not
    /// watched for edges, and no interrupt is taken.
    fn comparator_acsr(&self, acsr: u16, bus: &dyn Bus) -> SimResult<u8> {
        let stored = self.io[usize::from(acsr - 0x20)];
        if stored & ACSR_ACD != 0 {
            return Ok(stored);
        }
        let pin_mv = |channel: u64| -> SimResult<u32> {
            let base = self.model.adc_input_base() + channel * 2;
            let lo = bus.read_u8(base)?;
            let hi = bus.read_u8(base + 1)?;
            Ok(u32::from(u16::from_le_bytes([lo, hi])))
        };
        let positive_mv = if stored & ACSR_ACBG != 0 {
            AVR_BANDGAP_MV
        } else {
            pin_mv(0)?
        };
        let negative_mv = pin_mv(1)?;
        Ok(if positive_mv > negative_mv {
            stored | ACSR_ACO
        } else {
            stored
        })
    }

    /// ATtiny1634 port write at `addr` (PINx, DDRx, PORTx or PUEx) to the
    /// bus-side `avr_gpio` window at `AVR_IO_MIRROR_BASE + PINx`. PINx and
    /// DDRx go through as written; the PORTx byte the window holds is
    /// `(PORTx & DDRx) | (PUEx & !DDRx)`, so the pull-up follows PUEx and a
    /// PORTx bit on an input does not enable one (datasheet 8303 §12.2).
    /// Best-effort, as on the other cores: IN/OUT must not fail.
    fn tiny1634_mirror_port(&self, addr: u16, value: u8, bus: &mut dyn Bus) {
        let pin = 0x27 + (addr - 0x27) / 4 * 4;
        if addr == pin {
            let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + u64::from(pin), value);
            return;
        }
        let ddr = self.io[usize::from(pin + 1 - 0x20)];
        let port = self.io[usize::from(pin + 2 - 0x20)];
        let pue = self.io[usize::from(pin + 3 - 0x20)];
        if addr == pin + 1 {
            let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + u64::from(pin + 1), ddr);
        }
        let effective = (port & ddr) | (pue & !ddr);
        let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + u64::from(pin + 2), effective);
    }

    /// The pull-up-disable bit when `addr` is the register holding it:
    /// SFIOR.PUD (bit 2) on the classic parts (SFIOR 0x50) and the ATmega128
    /// (SFIOR 0x40), MCUCR.PUD (bit 6, data 0x55) on the ATtiny25/45/85 and
    /// 24/44/84 and the ATtiny13, and MCUCR.PUD (bit 7) on the ATtiny2313/4313.
    fn io_space_pud(&self, addr: u16) -> Option<u8> {
        match addr {
            TINY_X5_MCUCR if self.model.is_tiny2313() => Some(TINY2313_MCUCR_PUD),
            CLASSIC_SFIOR if self.model.is_classic() => Some(CLASSIC_SFIOR_PUD),
            MEGA128_SFIOR if self.model.is_mega128() => Some(CLASSIC_SFIOR_PUD),
            TINY_X5_MCUCR
                if self.model.is_tiny_x5() || self.model.is_tiny_x4() || self.model.is_tiny13() =>
            {
                Some(TINY_X5_MCUCR_PUD)
            }
            _ => None,
        }
    }

    /// ADSC (bit 6): write 1 starts a conversion. It completes immediately (no
    /// conversion time is modelled), converting the millivolts the bus-side
    /// `avr_adc` model holds for the channel ADMUX selects, so a potentiometer
    /// on A0 moves analogRead(A0). ADCSRA has this layout on every modelled
    /// part (ADATE is ADFR on the ATmega8, same bit). ADLAR is ADMUX bit 5,
    /// except on the ATtiny24/44/84, where it is ADCSRB (0x23) bit 4, and the
    /// ATtiny1634, where it is ADCSRB (0x22) bit 3.
    fn write_adcsra(&mut self, value: u8, bus: &dyn Bus) -> SimResult<()> {
        const ADSC: u8 = 1 << 6;
        const ADIF: u8 = 1 << 4;
        const ADEN: u8 = 1 << 7;
        self.adcsra = value;
        if value & ADEN != 0 && value & ADSC != 0 {
            let code = self.adc_convert(bus)?;
            let adlar = if self.model.is_tiny_x4() {
                self.io[(0x23 - 0x20) as usize] & (1 << 4) != 0
            } else if self.model.is_tiny1634() {
                self.io[(0x22 - 0x20) as usize] & (1 << 3) != 0
            } else {
                self.admux & (1 << 5) != 0
            };
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

    /// Classic ATmega8/16/32 and ATmega128 ADMUX: the input millivolts (an
    /// unmodelled code reads 0 V) and the reference.
    ///
    /// ATmega8: MUX3:0, 0..7 single-ended, 0x0E the 1.30 V bandgap, 0x0F GND.
    /// ATmega16/32/128: MUX4:0, 0..7 single-ended, 0x08..0x1D differential/gain
    /// (not modelled), 0x1E the bandgap (1.22 V on the 16/32, 1.23 V on the
    /// 128), 0x1F GND.
    /// REFS = 11 is the internal 2.56 V; 00 (AREF) is taken as 5 V AVcc, as on
    /// the 328P, because AREF is not a modelled pin.
    fn classic_adc_input(&self, bus: &dyn Bus) -> SimResult<(u32, u32)> {
        let (mux, bandgap_code, bandgap_mv) = if self.model == AvrModel::Atmega8 {
            (self.admux & 0x0F, 0x0E, MEGA8_BANDGAP_MV)
        } else if self.model.is_mega128() {
            (self.admux & 0x1F, 0x1E, MEGA128_BANDGAP_MV)
        } else {
            (self.admux & 0x1F, 0x1E, MEGA16_BANDGAP_MV)
        };
        let input_mv = match mux {
            ch @ 0..=7 => {
                let base = self.model.adc_input_base() + u64::from(ch) * 2;
                let lo = bus.read_u8(base)?;
                let hi = bus.read_u8(base + 1)?;
                u32::from(u16::from_le_bytes([lo, hi]))
            }
            m if m == bandgap_code => bandgap_mv,
            _ => 0,
        };
        let reference_mv = if self.admux >> 6 == 0b11 {
            CLASSIC_INTERNAL_REF_MV
        } else {
            AVR_AVCC_MV
        };
        Ok((input_mv, reference_mv))
    }

    /// ATtiny25/45/85 ADMUX: the input millivolts and the reference
    /// (ATtiny25/45/85 datasheet 2586Q §17.13, Tables 17-3/17-4).
    ///
    /// MUX3:0: 0..3 are ADC0..3 (PB5, PB2, PB4, PB3), 0x0C the 1.1 V bandgap,
    /// 0x0D GND; the differential/gain codes 0x04..0x0B and the temperature
    /// sensor (0x0F) are not modelled and read 0. REFS2 is ADMUX bit 4 (MUX4
    /// on the ATmega16/32): REFS2:0 = 010 is the internal 1.1 V and 11x the
    /// internal 2.56 V; x00 (VCC), x01 (AREF on PB0, not a modelled pin) and
    /// the reserved 011 are taken as 5 V VCC.
    fn tiny_x5_adc_input(&self, bus: &dyn Bus) -> SimResult<(u32, u32)> {
        let input_mv = match self.admux & 0x0F {
            ch @ 0..=3 => {
                let base = self.model.adc_input_base() + u64::from(ch) * 2;
                let lo = bus.read_u8(base)?;
                let hi = bus.read_u8(base + 1)?;
                u32::from(u16::from_le_bytes([lo, hi]))
            }
            0x0C => AVR_BANDGAP_MV,
            _ => 0,
        };
        let refs2 = self.admux & (1 << 4) != 0;
        let reference_mv = match (refs2, self.admux >> 6) {
            (false, 0b10) => AVR_BANDGAP_MV,
            (true, 0b10 | 0b11) => CLASSIC_INTERNAL_REF_MV,
            _ => AVR_AVCC_MV,
        };
        Ok((input_mv, reference_mv))
    }

    /// ATtiny13/13A ADMUX: the input millivolts and the reference (ATtiny13A
    /// datasheet 8126F §14.12.1, Tables 14-2/14-3).
    ///
    /// MUX1:0 are ADC0..3 (PB5, PB2, PB4, PB3); there is no bandgap, GND or
    /// differential input, and bits 4:2 and 7 are reserved. REFS0 (bit 6)
    /// selects the internal 1.1 V, otherwise VCC (5 V) is the reference: the
    /// part has no AREF and no REFS1/REFS2.
    fn tiny13_adc_input(&self, bus: &dyn Bus) -> SimResult<(u32, u32)> {
        let base = self.model.adc_input_base() + u64::from(self.admux & 0x03) * 2;
        let lo = bus.read_u8(base)?;
        let hi = bus.read_u8(base + 1)?;
        let input_mv = u32::from(u16::from_le_bytes([lo, hi]));
        let reference_mv = if self.admux & (1 << 6) != 0 {
            AVR_BANDGAP_MV
        } else {
            AVR_AVCC_MV
        };
        Ok((input_mv, reference_mv))
    }

    /// ATtiny24/44/84 ADMUX: the input millivolts and the reference
    /// (ATtiny24A/44A/84A datasheet 8183F §16.13, Tables 16-3/16-4).
    ///
    /// MUX5:0: 0..7 are ADC0..7 (PA0..PA7), 0x20 is AGND, 0x21 the 1.1 V
    /// bandgap; the differential/gain codes (0x08..0x1F, 0x23..0x3F) and the
    /// temperature sensor (0x22) are not modelled and read 0. REFS = 10 is the
    /// internal 1.1 V; 00 (VCC), 01 (AREF on PA0, not a modelled pin) and the
    /// reserved 11 are taken as 5 V VCC.
    fn tiny_x4_adc_input(&self, bus: &dyn Bus) -> SimResult<(u32, u32)> {
        let input_mv = match self.admux & 0x3F {
            ch @ 0..=7 => {
                let base = self.model.adc_input_base() + u64::from(ch) * 2;
                let lo = bus.read_u8(base)?;
                let hi = bus.read_u8(base + 1)?;
                u32::from(u16::from_le_bytes([lo, hi]))
            }
            0x21 => AVR_BANDGAP_MV,
            _ => 0,
        };
        let reference_mv = if self.admux >> 6 == 0b10 {
            AVR_BANDGAP_MV
        } else {
            AVR_AVCC_MV
        };
        Ok((input_mv, reference_mv))
    }

    /// ATtiny1634 ADMUX: the input millivolts and the reference (avr-libc
    /// `iotn1634.h`, simavr `sim_tiny1634.h`, ATtiny1634 datasheet 8303
    /// §19.13).
    ///
    /// MUX3:0: 0..11 are ADC0..11 (PA3..PA7, PB0..PB3, PC0..PC2), 0x0F is
    /// GND; the temperature sensor (0x0E) is not modelled and 0x0C/0x0D are
    /// reserved, so they read 0. ADMUX bits 5:4 (REFEN, ADC0EN) do not select
    /// the input. REFS = 10 is the internal 1.1 V; 00 (VCC), 01 (AREF, not a
    /// modelled pin) and the reserved 11 are taken as 5 V VCC.
    fn tiny1634_adc_input(&self, bus: &dyn Bus) -> SimResult<(u32, u32)> {
        let input_mv = match self.admux & 0x0F {
            ch @ 0..=11 => {
                let base = self.model.adc_input_base() + u64::from(ch) * 2;
                let lo = bus.read_u8(base)?;
                let hi = bus.read_u8(base + 1)?;
                u32::from(u16::from_le_bytes([lo, hi]))
            }
            _ => 0,
        };
        let reference_mv = if self.admux >> 6 == 0b10 {
            AVR_BANDGAP_MV
        } else {
            AVR_AVCC_MV
        };
        Ok((input_mv, reference_mv))
    }

    /// ATmega164/324/644/1284 ADMUX: the input millivolts and the reference.
    ///
    /// MUX4:0: 0..7 are ADC0..7 (PA0..PA7), 0x1E the 1.1 V bandgap, 0x1F GND;
    /// the differential/gain codes 0x08..0x1D are not modelled and read 0.
    /// REFS = 10 is the internal 1.1 V and 11 the internal 2.56 V; 00 (AREF)
    /// is taken as 5 V AVcc, as on the 328P, because AREF is not a modelled pin.
    fn xx4_adc_input(&self, bus: &dyn Bus) -> SimResult<(u32, u32)> {
        let input_mv = match self.admux & 0x1F {
            ch @ 0..=7 => {
                let base = self.model.adc_input_base() + u64::from(ch) * 2;
                let lo = bus.read_u8(base)?;
                let hi = bus.read_u8(base + 1)?;
                u32::from(u16::from_le_bytes([lo, hi]))
            }
            0x1E => AVR_BANDGAP_MV,
            _ => 0,
        };
        let reference_mv = match self.admux >> 6 {
            0b10 => AVR_BANDGAP_MV,
            0b11 => CLASSIC_INTERNAL_REF_MV,
            _ => AVR_AVCC_MV,
        };
        Ok((input_mv, reference_mv))
    }

    /// ATmega128RFA1/128RFR2 ADCSRB: AVDDOK (bit 7) and REFOK (bit 5) are
    /// read-only status bits, 1 once ADEN has powered AVDD and the reference.
    fn rf_adcsrb(&self) -> u8 {
        const AVDDOK_REFOK: u8 = (1 << 7) | (1 << 5);
        let stored = self.io[(0x7B - 0x20) as usize] & !AVDDOK_REFOK;
        if self.adcsra & (1 << 7) != 0 {
            stored | AVDDOK_REFOK
        } else {
            stored
        }
    }

    /// ATmega128RFA1/128RFR2 ADMUX: the input millivolts and the reference.
    ///
    /// MUX5:0 (ADCSRB.MUX5 with MUX4:0): 0..7 are ADC0..7 (PF0..PF7), 0x1E
    /// the 1.2 V bandgap, 0x1F AVSS; differential, temperature-sensor (0x29)
    /// and back-bias codes are not modelled and read 0. REFS = 01 is AVDD
    /// (1.8 V), 10 the internal 1.5 V, 11 the internal 1.6 V; 00 (AREF) is
    /// taken as AVDD because AREF is not a modelled pin.
    fn rf_adc_input(&self, bus: &dyn Bus) -> SimResult<(u32, u32)> {
        let mux5 = self.io[(0x7B - 0x20) as usize] & (1 << 3) != 0;
        let input_mv = match (self.admux & 0x1F) | if mux5 { 0x20 } else { 0 } {
            ch @ 0..=7 => {
                let base = self.model.adc_input_base() + u64::from(ch) * 2;
                let lo = bus.read_u8(base)?;
                let hi = bus.read_u8(base + 1)?;
                u32::from(u16::from_le_bytes([lo, hi]))
            }
            0x1E => RF_BANDGAP_MV,
            _ => 0,
        };
        let reference_mv = match self.admux >> 6 {
            0b10 => RF_REF_1V5_MV,
            0b11 => RF_REF_1V6_MV,
            _ => RF_AVDD_MV,
        };
        Ok((input_mv, reference_mv))
    }

    /// ATmega32U4 ADMUX: the input millivolts and the reference (avr-libc
    /// `iom32u4.h`, ATmega16U4/32U4 datasheet 7766 §24.9, Table 24-4).
    ///
    /// MUX5 is ADCSRB bit 5 (bit 3 on the 1281/2560). MUX5:0 0x00, 0x01 and
    /// 0x04..0x07 are ADC0, ADC1, ADC4..7 (PF0, PF1, PF4..PF7; there is no
    /// ADC2/ADC3), 0x20..0x25 are ADC8..13 (PD4, PD6, PD7, PB4, PB5, PB6),
    /// 0x1E the 1.1 V bandgap, 0x1F GND; the differential/gain codes and the
    /// temperature sensor (0x27) are not modelled and read 0. The `avr_adc`
    /// window is indexed by ADC number. REFS = 11 is the internal 2.56 V; 00
    /// (AREF, not a modelled pin), 01 (AVcc) and the reserved 10 are taken as
    /// 5 V AVcc.
    fn u4_adc_input(&self, bus: &dyn Bus) -> SimResult<(u32, u32)> {
        let mux5 = self.io[(0x7B - 0x20) as usize] & (1 << 5) != 0;
        let channel_mv = |ch: u8| -> SimResult<u32> {
            let base = self.model.adc_input_base() + u64::from(ch) * 2;
            let lo = bus.read_u8(base)?;
            let hi = bus.read_u8(base + 1)?;
            Ok(u32::from(u16::from_le_bytes([lo, hi])))
        };
        let input_mv = match (self.admux & 0x1F) | if mux5 { 0x20 } else { 0 } {
            ch @ (0x00 | 0x01 | 0x04..=0x07) => channel_mv(ch)?,
            code @ 0x20..=0x25 => channel_mv(code - 0x18)?,
            0x1E => AVR_BANDGAP_MV,
            _ => 0,
        };
        let reference_mv = if self.admux >> 6 == 0b11 {
            CLASSIC_INTERNAL_REF_MV
        } else {
            AVR_AVCC_MV
        };
        Ok((input_mv, reference_mv))
    }

    /// ATmega16M1/64M1 ADMUX: the input millivolts and the reference (avr-libc
    /// `iom16m1.h`, datasheet 8209F §22.11.1, Tables 22-4 and 22-5).
    ///
    /// MUX4:0 0x00..0x0A are ADC0..10 (PE2, PD4, PD5, PD6, PB7, PB2, PB5,
    /// PB6, PC4, PC5, PC6), 0x0C is VCC/4, 0x11 the 1.1 V bandgap and 0x12
    /// GND. The temperature sensor (0x0B), ISRC (0x0D), the amplifiers
    /// (0x0E..0x10) and the reserved codes are not modelled and read 0.
    /// ADCSRB has no MUX5 (bit 5 is AREFEN). REFS = 11 is the internal
    /// 2.56 V; 00 (AREF, not a modelled pin), 01 (AVcc) and the reserved 10
    /// are taken as 5 V AVcc.
    fn m1_adc_input(&self, bus: &dyn Bus) -> SimResult<(u32, u32)> {
        let input_mv = match self.admux & 0x1F {
            ch @ 0x00..=0x0A => {
                let base = self.model.adc_input_base() + u64::from(ch) * 2;
                let lo = bus.read_u8(base)?;
                let hi = bus.read_u8(base + 1)?;
                u32::from(u16::from_le_bytes([lo, hi]))
            }
            0x0C => AVR_AVCC_MV / 4,
            0x11 => AVR_BANDGAP_MV,
            _ => 0,
        };
        let reference_mv = if self.admux >> 6 == 0b11 {
            CLASSIC_INTERNAL_REF_MV
        } else {
            AVR_AVCC_MV
        };
        Ok((input_mv, reference_mv))
    }

    /// ATmega169P/169PA ADMUX: the 10-bit result register value (avr-libc
    /// `iom169p.h`, datasheet 8018P §22.7 and §22.9.1, Tables 22-3/22-4).
    ///
    /// MUX4:0 0x00..0x07 are ADC0..7 (PF0..PF7), single-ended:
    /// `Vin * 1024 / Vref`. 0x10..0x17 are ADC0..7 minus ADC1 and 0x18..0x1D
    /// ADC0..5 minus ADC2, gain 1: `(Vpos - Vneg) * 512 / Vref` as a 10-bit
    /// two's complement value, -512..511, truncated toward zero (the
    /// datasheet's ADC3 - ADC2 example, 300 mV - 500 mV at 1.1 V, is -93 =
    /// 0x3A3). 0x1E is the 1.1 V bandgap and 0x1F GND; 0x08..0x0F are N/A
    /// and read 0. ADCSRB has no MUX5 (bit 6 is ACME, bits 2:0 ADTS). REFS =
    /// 11 is the internal 1.1 V; 00 (AREF, not a modelled pin), 01 (AVcc)
    /// and the reserved 10 are taken as 5 V AVcc.
    fn m169p_adc_code(&self, bus: &dyn Bus) -> SimResult<u16> {
        let pin_mv = |ch: u8| -> SimResult<i32> {
            let base = self.model.adc_input_base() + u64::from(ch) * 2;
            let lo = bus.read_u8(base)?;
            let hi = bus.read_u8(base + 1)?;
            Ok(i32::from(u16::from_le_bytes([lo, hi])))
        };
        let reference_mv = if self.admux >> 6 == 0b11 {
            AVR_BANDGAP_MV
        } else {
            AVR_AVCC_MV
        } as i32;
        let single = |mv: i32| (mv * 1024 / reference_mv).clamp(0, 1023) as u16;
        let differential = |pos: u8, neg: u8| -> SimResult<u16> {
            let code = ((pin_mv(pos)? - pin_mv(neg)?) * 512 / reference_mv).clamp(-512, 511);
            Ok((code & 0x3FF) as u16)
        };
        match self.admux & 0x1F {
            ch @ 0x00..=0x07 => Ok(single(pin_mv(ch)?)),
            m @ 0x10..=0x17 => differential(m - 0x10, 1),
            m @ 0x18..=0x1D => differential(m - 0x18, 2),
            0x1E => Ok(single(AVR_BANDGAP_MV as i32)),
            _ => Ok(0),
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
        if self.model.is_classic() || self.model.is_mega128() {
            let (input_mv, reference_mv) = self.classic_adc_input(bus)?;
            return Ok((input_mv * 1024 / reference_mv).min(1023) as u16);
        }
        if self.model.is_tiny_x5() {
            let (input_mv, reference_mv) = self.tiny_x5_adc_input(bus)?;
            return Ok((input_mv * 1024 / reference_mv).min(1023) as u16);
        }
        if self.model.is_tiny13() {
            let (input_mv, reference_mv) = self.tiny13_adc_input(bus)?;
            return Ok((input_mv * 1024 / reference_mv).min(1023) as u16);
        }
        if self.model.is_tiny_x4() {
            let (input_mv, reference_mv) = self.tiny_x4_adc_input(bus)?;
            return Ok((input_mv * 1024 / reference_mv).min(1023) as u16);
        }
        if self.model.is_tiny1634() {
            let (input_mv, reference_mv) = self.tiny1634_adc_input(bus)?;
            return Ok((input_mv * 1024 / reference_mv).min(1023) as u16);
        }
        if self.model.is_xx4() {
            let (input_mv, reference_mv) = self.xx4_adc_input(bus)?;
            return Ok((input_mv * 1024 / reference_mv).min(1023) as u16);
        }
        if self.model.is_rf() {
            let (input_mv, reference_mv) = self.rf_adc_input(bus)?;
            return Ok((input_mv * 1024 / reference_mv).min(1023) as u16);
        }
        if self.model.is_u4() {
            let (input_mv, reference_mv) = self.u4_adc_input(bus)?;
            return Ok((input_mv * 1024 / reference_mv).min(1023) as u16);
        }
        if self.model.is_m1() {
            let (input_mv, reference_mv) = self.m1_adc_input(bus)?;
            return Ok((input_mv * 1024 / reference_mv).min(1023) as u16);
        }
        if self.model.is_m169p() {
            return self.m169p_adc_code(bus);
        }
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
        // The spin fixture (and most sketches between timer uses) has every
        // prescaler stopped. Skip the three outlined tick bodies then.
        if self.t0_prescaler() != 0 {
            self.tick_timer0(cpu_cycles);
        }
        if self.t1.prescaler() != 0 {
            self.tick_timer1(cpu_cycles);
        }
        if self.t2_prescaler() != 0 {
            self.tick_timer2(cpu_cycles);
        }
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
        if self.model.is_io_space() {
            // No vector table is modelled for the I/O-space cores (see
            // `AvrModel`), and none of their peripherals raises a request.
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
        } else if self.model.is_xx4() {
            xx4_vector(vec)
        } else if self.model.is_u4() {
            u4_vector(vec)
        } else if self.model.is_usb162() {
            usb162_vector(vec)
        } else if self.model.is_m1() {
            m1_vector(vec)
        } else if self.model.is_m169p() {
            m169p_vector(vec)
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

    fn for_each_attached_device(
        &self,
        f: &mut dyn FnMut(Option<&str>, crate::inspect::AttachedDeviceRef<'_>),
    ) {
        for device in &self.spi_devices {
            crate::inspect::visit_spi_device(&**device, &mut |d| f(Some("spi"), d));
        }
        for slave in &self.i2c_slaves {
            crate::inspect::visit_i2c_device(&**slave, &mut |d| f(Some("i2c"), d));
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
        assert_eq!(
            Avr::for_chip_name("simavr-atmega1280").model,
            AvrModel::Atmega1281
        );
        let m = mega();
        assert_eq!(m.sp, 0x21FF, "SP resets to RAMEND");
        assert_eq!(m.flash.len(), 256 * 1024);
    }

    #[test]
    fn mega_128k_names_use_the_128k_two_byte_pc_core() {
        let cpu = Avr::for_chip_name("atmega1281");
        assert_eq!(cpu.model, AvrModel::Atmega1281);
        assert_eq!(cpu.flash.len(), 128 * 1024);
        assert_eq!(cpu.sram.len(), 8 * 1024);
        assert_eq!(cpu.sp, 0x21FF);
        assert!(!cpu.model.three_byte_pc());
    }

    #[test]
    fn atmega1281_does_not_decode_eind_control_flow_opcodes() {
        for opcode in [0x9419, 0x9519] {
            let mut cpu = Avr::for_chip_name("atmega1281");
            let mut bus = MockBus::new();
            cpu.load_words(0, &[opcode]);
            let err = cpu.step(&mut bus, &[], &SimulationConfig::default());
            assert!(
                matches!(err, Err(SimulationError::DecodeError(0))),
                "opcode {opcode:#06x}"
            );
        }
    }

    #[test]
    fn atmega1281_call_and_ret_use_a_two_byte_return_address() {
        let mut cpu = Avr::for_chip_name("atmega1281");
        let mut bus = MockBus::new();
        cpu.load_words(0, &[0x940E, 0x0080]);
        cpu.load_words(0x100, &[0x9508]);
        let initial_sp = cpu.sp;

        step_n(&mut cpu, &mut bus, 1);
        assert_eq!(cpu.pc, 0x100);
        assert_eq!(initial_sp - cpu.sp, 2, "128 KB part pushes 2 PC bytes");

        step_n(&mut cpu, &mut bus, 1);
        assert_eq!(cpu.pc, 4);
        assert_eq!(cpu.sp, initial_sp);
    }

    // ---- classic ATmega8/16/32 (AvrModel::is_classic) ----

    #[test]
    fn classic_cores_need_the_exact_part_name() {
        for (name, model) in [
            ("simavr-atmega8", AvrModel::Atmega8),
            ("atmega8a", AvrModel::Atmega8),
            ("simavr-atmega16", AvrModel::Atmega16),
            ("simavr-atmega32", AvrModel::Atmega32),
            // Other maps whose names contain a classic part's name.
            ("simavr-atmega88", AvrModel::Atmega328p),
            ("simavr-atmega168", AvrModel::Atmega328p),
            ("simavr-atmega16m1", AvrModel::Atmega16m1),
            ("simavr-atmega328", AvrModel::Atmega328p),
            ("simavr-atmega32u4", AvrModel::Atmega32u4),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
        }
    }

    #[test]
    fn classic_map_has_sram_at_0x60_and_its_ports_in_io_space() {
        for (model, ramend, flash) in [
            (AvrModel::Atmega8, 0x045F, 8 * 1024),
            (AvrModel::Atmega16, 0x045F, 16 * 1024),
            (AvrModel::Atmega32, 0x085F, 32 * 1024),
        ] {
            let mut cpu = Avr::new_model(model);
            let mut bus = MockBus::new();
            assert_eq!(cpu.sp, ramend, "{model:?}: SP resets to RAMEND");
            assert_eq!(cpu.flash.len(), flash);
            // 0x60 is SRAM here, not the 328P's extended I/O.
            cpu.data_write(0x60, 0xA5, &mut bus).unwrap();
            assert_eq!(cpu.sram[0], 0xA5);
            assert_eq!(cpu.data_read(0x60, &bus).unwrap(), 0xA5);
            // DDRB (0x37) and PORTB (0x38) mirror to the bus-side port.
            cpu.data_write(0x37, 0x0F, &mut bus).unwrap();
            cpu.data_write(0x38, 0x05, &mut bus).unwrap();
            assert_eq!(bus.mem[&(AVR_IO_MIRROR_BASE + 0x37)], 0x0F);
            assert_eq!(bus.mem[&(AVR_IO_MIRROR_BASE + 0x38)], 0x05);
            // PINB: driven bits from PORTB, input bits from the outside level.
            bus.mem.insert(AVR_IO_MIRROR_BASE + 0x36, 0xF0);
            assert_eq!(cpu.data_read(0x36, &bus).unwrap(), 0xF5);
            // The 328P's TIFR0 (0x35) is PORTC here: stored and mirrored.
            cpu.data_write(0x35, 0x3C, &mut bus).unwrap();
            assert_eq!(cpu.data_read(0x35, &bus).unwrap(), 0x3C);
            assert_eq!(cpu.tifr0, 0);
            assert!(
                cpu.data_read(ramend + 1, &bus).is_err(),
                "{model:?}: past RAMEND"
            );
        }
        // Port A exists on the 16/32 only.
        let mut bus = MockBus::new();
        let mut m16 = Avr::new_model(AvrModel::Atmega16);
        m16.data_write(0x3B, 0x81, &mut bus).unwrap();
        assert_eq!(bus.mem[&(AVR_IO_MIRROR_BASE + 0x3B)], 0x81);
        let mut bus = MockBus::new();
        let mut m8 = Avr::new_model(AvrModel::Atmega8);
        m8.data_write(0x3B, 0x81, &mut bus).unwrap();
        assert!(!bus.mem.contains_key(&(AVR_IO_MIRROR_BASE + 0x3B)));
    }

    #[test]
    fn classic_adc_converts_from_its_io_space_window() {
        let mut cpu = Avr::new_model(AvrModel::Atmega16);
        let mut bus = MockBus::new();
        // ADC5 holds 2000 mV in the window at 0x1_0060.
        let base = AVR_CLASSIC_ADC_INPUT_BASE + 5 * 2;
        bus.mem.insert(base, (2000u16 & 0xFF) as u8);
        bus.mem.insert(base + 1, (2000u16 >> 8) as u8);
        // OUT ADMUX(0x07),R16 with REFS0|MUX5; OUT ADCSRA(0x06),R17 = ADEN|ADSC.
        cpu.load_words(0, &[0xE405, 0xB907, 0xEC10, 0xB916]);
        step_n(&mut cpu, &mut bus, 4);
        assert_eq!(u16::from(cpu.adch) << 8 | u16::from(cpu.adcl), 409);
        // ADLAR left-adjusts; REFS = 11 is the internal 2.56 V.
        cpu.data_write(0x27, 0xE5, &mut bus).unwrap();
        cpu.data_write(0x26, 0xC0, &mut bus).unwrap();
        let code = 2000 * 1024 / 2560;
        assert_eq!(cpu.data_read(0x25, &bus).unwrap(), (code >> 2) as u8);
        assert_eq!(cpu.data_read(0x24, &bus).unwrap(), ((code & 3) << 6) as u8);
        // MUX 0x1F is GND; a differential code is not modelled and reads 0.
        for mux in [0x1F, 0x09] {
            cpu.data_write(0x27, 0x40 | mux, &mut bus).unwrap();
            cpu.data_write(0x26, 0xC0, &mut bus).unwrap();
            assert_eq!(u16::from(cpu.adch) << 8 | u16::from(cpu.adcl), 0);
        }
    }

    #[test]
    fn classic_sfior_pud_disables_pull_ups_and_mcucr_does_not() {
        struct PudBus {
            inner: MockBus,
            pud: Vec<bool>,
        }
        impl Bus for PudBus {
            fn read_u8(&self, addr: u64) -> SimResult<u8> {
                self.inner.read_u8(addr)
            }
            fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
                self.inner.write_u8(addr, value)
            }
            fn tick_peripherals(&mut self) -> Vec<u32> {
                Vec::new()
            }
            fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
                Ok(())
            }
            fn config(&self) -> &SimulationConfig {
                &self.inner.config
            }
            fn set_pull_ups_disabled(&mut self, disabled: bool) {
                self.pud.push(disabled);
            }
        }
        let mut bus = PudBus {
            inner: MockBus::new(),
            pud: Vec::new(),
        };
        let mut cpu = Avr::new_model(AvrModel::Atmega32);
        // MCUCR bit 4 is SM0 on the classic parts, not PUD.
        cpu.data_write(ADDR_MCUCR, MCUCR_PUD, &mut bus).unwrap();
        assert!(bus.pud.is_empty());
        cpu.data_write(CLASSIC_SFIOR, CLASSIC_SFIOR_PUD, &mut bus)
            .unwrap();
        cpu.data_write(CLASSIC_SFIOR, 0, &mut bus).unwrap();
        assert_eq!(bus.pud, [true, false]);
    }

    // ---- ATmega128 (AvrModel::is_mega128) ----

    #[test]
    fn mega128_core_needs_the_exact_part_name_and_has_the_iom128_map() {
        for name in ["simavr-atmega128", "atmega128a"] {
            assert_eq!(AvrModel::from_chip_name(name), AvrModel::Atmega128);
        }
        for (name, model) in [
            ("atmega1280", AvrModel::Atmega1281),
            ("atmega1281", AvrModel::Atmega1281),
            ("atmega1284p", AvrModel::Atmega1284),
            ("atmega128rfa1", AvrModel::Atmega128Rf),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
        }
        let model = AvrModel::Atmega128;
        assert!(model.is_io_space() && !model.is_classic());
        assert_eq!(model.flash_size(), 128 * 1024);
        assert_eq!((model.sram_start(), model.ramend()), (0x0100, 0x10FF));

        let mut cpu = Avr::new_model(model);
        let mut bus = MockBus::new();
        // Port F is split: PINF 0x20, DDRF 0x61, PORTF 0x62, gathered on the
        // bus at MEGA128_PORTF_MIRROR, not at 0x1_0061 (0x1_0021 is PINE).
        cpu.data_write(0x61, 0x0F, &mut bus).unwrap();
        cpu.data_write(0x62, 0x05, &mut bus).unwrap();
        assert_eq!(bus.mem[&(MEGA128_PORTF_MIRROR + 1)], 0x0F);
        assert_eq!(bus.mem[&(MEGA128_PORTF_MIRROR + 2)], 0x05);
        assert!(!bus.mem.contains_key(&(AVR_IO_MIRROR_BASE + 0x61)));
        bus.mem.insert(MEGA128_PORTF_MIRROR, 0xA0);
        assert_eq!(cpu.data_read(0x20, &bus).unwrap(), 0xA5);
        // Port E (0x21..0x23) and port G (0x63..0x65) are contiguous.
        cpu.data_write(0x22, 0x01, &mut bus).unwrap();
        cpu.data_write(0x23, 0x01, &mut bus).unwrap();
        bus.mem.insert(AVR_IO_MIRROR_BASE + 0x21, 0x80);
        assert_eq!(cpu.data_read(0x21, &bus).unwrap(), 0x81);
        cpu.data_write(0x64, 0x1F, &mut bus).unwrap();
        assert_eq!(bus.mem[&(AVR_IO_MIRROR_BASE + 0x64)], 0x1F);
        // Extended I/O is stored (TWBR 0x70), not mirrored and not SRAM.
        cpu.data_write(0x70, 0x48, &mut bus).unwrap();
        assert_eq!(cpu.data_read(0x70, &bus).unwrap(), 0x48);
        assert!(!bus.mem.contains_key(&(AVR_IO_MIRROR_BASE + 0x70)));
        assert_eq!(cpu.sram[0], 0);
        cpu.data_write(0x100, 0x5A, &mut bus).unwrap();
        assert_eq!(cpu.sram[0], 0x5A);
        assert!(cpu.data_read(0x1100, &bus).is_err(), "past RAMEND");
        // SFIOR.PUD is bit 2 at 0x40 here, not at the ATmega16/32's 0x50.
        assert_eq!(cpu.io_space_pud(MEGA128_SFIOR), Some(CLASSIC_SFIOR_PUD));
        assert_eq!(cpu.io_space_pud(CLASSIC_SFIOR), None);
    }

    #[test]
    fn mega128_adc_converts_port_f_against_its_references() {
        let mut cpu = Avr::new_model(AvrModel::Atmega128);
        let mut bus = MockBus::new();
        // ADC7 (PF7) holds 2000 mV in the window at 0x1_0200.
        let base = AVR_MEGA_ADC_INPUT_BASE + 7 * 2;
        bus.mem.insert(base, (2000u16 & 0xFF) as u8);
        bus.mem.insert(base + 1, (2000u16 >> 8) as u8);
        // OUT ADMUX(0x07),R16 with REFS0|MUX7; OUT ADCSRA(0x06),R17 = ADEN|ADSC.
        cpu.load_words(0, &[0xE407, 0xB907, 0xEC10, 0xB916]);
        step_n(&mut cpu, &mut bus, 4);
        assert_eq!(u16::from(cpu.adch) << 8 | u16::from(cpu.adcl), 409);
        // REFS = 11 is the internal 2.56 V.
        cpu.data_write(0x27, 0xC7, &mut bus).unwrap();
        cpu.data_write(0x26, 0xC0, &mut bus).unwrap();
        assert_eq!(u16::from(cpu.adch) << 8 | u16::from(cpu.adcl), 800);
        // MUX 0x1E is the 1.23 V bandgap; 0x1F is GND and 0x09 differential.
        cpu.data_write(0x27, 0x5E, &mut bus).unwrap();
        cpu.data_write(0x26, 0xC0, &mut bus).unwrap();
        assert_eq!(
            u16::from(cpu.adch) << 8 | u16::from(cpu.adcl),
            (1230 * 1024 / 5000) as u16
        );
        for mux in [0x1F, 0x09] {
            cpu.data_write(0x27, 0x40 | mux, &mut bus).unwrap();
            cpu.data_write(0x26, 0xC0, &mut bus).unwrap();
            assert_eq!(u16::from(cpu.adch) << 8 | u16::from(cpu.adcl), 0);
        }
    }

    // ---- ATtiny25/45/85 (AvrModel::is_tiny_x5) ----

    #[test]
    fn tiny_x5_cores_need_the_exact_part_name_and_have_the_iotnx5_map() {
        for (name, model) in [
            ("simavr-attiny25", AvrModel::Attiny25),
            ("simavr-attiny45", AvrModel::Attiny45),
            ("simavr-attiny85", AvrModel::Attiny85),
            // The x4 tinies and the 2313 have their own cores.
            ("simavr-attiny24", AvrModel::Attiny24),
            ("simavr-attiny84", AvrModel::Attiny84),
            ("simavr-attiny2313", AvrModel::Attiny2313),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
        }
        // RAMEND/FLASHEND + 1 per avr-libc iotn25.h/iotn45.h/iotn85.h.
        for (model, ramend, flash) in [
            (AvrModel::Attiny25, 0x00DF, 2 * 1024),
            (AvrModel::Attiny45, 0x015F, 4 * 1024),
            (AvrModel::Attiny85, 0x025F, 8 * 1024),
        ] {
            let mut cpu = Avr::new_model(model);
            let mut bus = MockBus::new();
            assert_eq!(cpu.sp, ramend, "{model:?}: SP resets to RAMEND");
            assert_eq!(cpu.flash.len(), flash);
            cpu.data_write(0x60, 0x5A, &mut bus).unwrap();
            assert_eq!(cpu.sram[0], 0x5A);
            assert!(cpu.data_read(ramend + 1, &bus).is_err(), "{model:?}");
            // Port B is the only port: PINB composes, DDRB/PORTB mirror.
            cpu.data_write(0x37, 0x0F, &mut bus).unwrap();
            cpu.data_write(0x38, 0x05, &mut bus).unwrap();
            bus.mem.insert(AVR_IO_MIRROR_BASE + 0x36, 0xF0);
            assert_eq!(cpu.data_read(0x36, &bus).unwrap(), 0xF5);
            // The classic parts' PORTC/PORTA addresses are other registers here.
            cpu.data_write(0x35, 0x11, &mut bus).unwrap();
            cpu.data_write(0x3B, 0x22, &mut bus).unwrap();
            assert!(!bus.mem.contains_key(&(AVR_IO_MIRROR_BASE + 0x35)));
            assert!(!bus.mem.contains_key(&(AVR_IO_MIRROR_BASE + 0x3B)));
        }
    }

    #[test]
    fn tiny_x5_adc_has_four_channels_and_refs2() {
        let mut cpu = Avr::new_model(AvrModel::Attiny85);
        let mut bus = MockBus::new();
        // ADC3 (PB3) holds 1000 mV in the window at 0x1_0060.
        let base = AVR_CLASSIC_ADC_INPUT_BASE + 3 * 2;
        bus.mem.insert(base, (1000u16 & 0xFF) as u8);
        bus.mem.insert(base + 1, (1000u16 >> 8) as u8);
        let convert = |cpu: &mut Avr, bus: &mut MockBus, admux: u8| {
            cpu.data_write(0x27, admux, bus).unwrap();
            cpu.data_write(0x26, 0xC0, bus).unwrap();
            u32::from(cpu.adch) << 8 | u32::from(cpu.adcl)
        };
        assert_eq!(convert(&mut cpu, &mut bus, 0x03), 1000 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x83), 1000 * 1024 / 1100);
        assert_eq!(convert(&mut cpu, &mut bus, 0x93), 1000 * 1024 / 2560);
        assert_eq!(convert(&mut cpu, &mut bus, 0xD3), 1000 * 1024 / 2560);
        // REFS2 with REFS1:0 = 00 is still VCC; 011 is reserved.
        assert_eq!(convert(&mut cpu, &mut bus, 0x13), 1000 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0xC3), 1000 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x0C), 1100 * 1024 / 5000);
        for mux in [0x04, 0x0D, 0x0E, 0x0F] {
            assert_eq!(convert(&mut cpu, &mut bus, mux), 0, "MUX {mux:#04x}");
        }
    }

    #[test]
    fn tiny_x5_mcucr_pud_disables_pull_ups_and_sfior_does_not() {
        struct PudBus {
            inner: MockBus,
            pud: Vec<bool>,
        }
        impl Bus for PudBus {
            fn read_u8(&self, addr: u64) -> SimResult<u8> {
                self.inner.read_u8(addr)
            }
            fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
                self.inner.write_u8(addr, value)
            }
            fn tick_peripherals(&mut self) -> Vec<u32> {
                Vec::new()
            }
            fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
                Ok(())
            }
            fn config(&self) -> &SimulationConfig {
                &self.inner.config
            }
            fn set_pull_ups_disabled(&mut self, disabled: bool) {
                self.pud.push(disabled);
            }
        }
        let mut bus = PudBus {
            inner: MockBus::new(),
            pud: Vec::new(),
        };
        let mut cpu = Avr::new_model(AvrModel::Attiny45);
        // 0x50 is TCCR1 here, not SFIOR.
        cpu.data_write(CLASSIC_SFIOR, CLASSIC_SFIOR_PUD, &mut bus)
            .unwrap();
        assert!(bus.pud.is_empty());
        cpu.data_write(TINY_X5_MCUCR, TINY_X5_MCUCR_PUD, &mut bus)
            .unwrap();
        cpu.data_write(TINY_X5_MCUCR, 0, &mut bus).unwrap();
        assert_eq!(bus.pud, [true, false]);
    }

    // ---- ATtiny13/13A (AvrModel::is_tiny13) ----

    #[test]
    fn tiny13_core_needs_the_exact_part_name_and_has_the_iotn13_map() {
        for (name, model) in [
            ("simavr-attiny13", AvrModel::Attiny13),
            ("attiny13a", AvrModel::Attiny13),
            // The 1634 is another map, with its own core.
            ("simavr-attiny1634", AvrModel::Attiny1634),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
        }
        // RAMEND 0x9F, FLASHEND 0x3FF per avr-libc iotn13.h.
        let mut cpu = Avr::new_model(AvrModel::Attiny13);
        let mut bus = MockBus::new();
        assert_eq!(cpu.sp, 0x009F);
        assert_eq!(cpu.flash.len(), 1024);
        cpu.data_write(0x9F, 0x5A, &mut bus).unwrap();
        assert_eq!(cpu.sram[0x3F], 0x5A);
        assert!(cpu.data_read(0xA0, &bus).is_err());
        // Port B is the only port.
        cpu.data_write(0x37, 0x0F, &mut bus).unwrap();
        cpu.data_write(0x38, 0x05, &mut bus).unwrap();
        bus.mem.insert(AVR_IO_MIRROR_BASE + 0x36, 0xF0);
        assert_eq!(cpu.data_read(0x36, &bus).unwrap(), 0xF5);
        cpu.data_write(0x39, 0x22, &mut bus).unwrap();
        assert!(!bus.mem.contains_key(&(AVR_IO_MIRROR_BASE + 0x39)));
    }

    #[test]
    fn tiny13_adc_has_mux1_0_and_refs0_only() {
        let mut cpu = Avr::new_model(AvrModel::Attiny13);
        let mut bus = MockBus::new();
        for (ch, mv) in [(0u64, 400u16), (3, 1000)] {
            let base = AVR_CLASSIC_ADC_INPUT_BASE + ch * 2;
            bus.mem.insert(base, (mv & 0xFF) as u8);
            bus.mem.insert(base + 1, (mv >> 8) as u8);
        }
        let convert = |cpu: &mut Avr, bus: &mut MockBus, admux: u8| {
            cpu.data_write(0x27, admux, bus).unwrap();
            cpu.data_write(0x26, 0xC0, bus).unwrap();
            u32::from(cpu.adch) << 8 | u32::from(cpu.adcl)
        };
        assert_eq!(convert(&mut cpu, &mut bus, 0x03), 1000 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x43), 1000 * 1024 / 1100);
        // REFS1/REFS2 do not exist: bits 7 and 4 change nothing.
        assert_eq!(convert(&mut cpu, &mut bus, 0x83), 1000 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x53), 1000 * 1024 / 1100);
        // MUX3:2 are reserved: 0x0C (the x5 bandgap) is still ADC0.
        assert_eq!(convert(&mut cpu, &mut bus, 0x0C), 400 * 1024 / 5000);
    }

    // ---- ATmega32U4 (AvrModel::is_u4) ----

    #[test]
    fn u4_core_needs_the_exact_part_name_and_has_the_iom32u4_map() {
        for (name, model) in [
            ("simavr-atmega32u4", AvrModel::Atmega32u4),
            ("atmega32u4", AvrModel::Atmega32u4),
            // The 16U4 has 1.25 KB SRAM; the 32U2 has no ADC.
            ("atmega16u4", AvrModel::Atmega328p),
            ("atmega32u2", AvrModel::Atmega328p),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
        }
        // RAMSTART 0x100, RAMEND 0xAFF, FLASHEND 0x7FFF per avr-libc iom32u4.h.
        let mut cpu = Avr::new_model(AvrModel::Atmega32u4);
        let mut bus = MockBus::new();
        assert_eq!(cpu.sp, 0x0AFF);
        assert_eq!(cpu.flash.len(), 32 * 1024);
        cpu.data_write(0x0AFF, 0x5A, &mut bus).unwrap();
        assert_eq!(cpu.sram[0x09FF], 0x5A);
        assert!(cpu.data_read(0x0B00, &bus).is_err());
        // 0x100 is SRAM here, not the mega's extended I/O.
        cpu.data_write(0x0100, 0xA5, &mut bus).unwrap();
        assert_eq!(cpu.sram[0], 0xA5);
        // Ports B..F; 0x20..0x22 (the mega's port A) is not a port.
        assert!(!cpu.is_pin_reg(0x20) && !cpu.is_port_reg(0x22));
        assert!(cpu.is_pin_reg(0x23) && cpu.is_pin_reg(0x2F) && cpu.is_port_reg(0x31));
        assert!(!cpu.is_pin_reg(0x32) && !cpu.is_port_reg(0x32));
        cpu.data_write(0x30, 0x0F, &mut bus).unwrap();
        cpu.data_write(0x31, 0x05, &mut bus).unwrap();
        bus.mem.insert(AVR_IO_MIRROR_BASE + 0x2F, 0xF0);
        assert_eq!(cpu.data_read(0x2F, &bus).unwrap(), 0xF5);
    }

    #[test]
    fn u4_adc_has_mux5_in_adcsrb_bit5_and_no_adc2_or_adc3() {
        let mut cpu = Avr::new_model(AvrModel::Atmega32u4);
        let mut bus = MockBus::new();
        for (ch, mv) in [(0u64, 400u16), (2, 700), (5, 900), (13, 1000)] {
            let base = AVR_MEGA_ADC_INPUT_BASE + ch * 2;
            bus.mem.insert(base, (mv & 0xFF) as u8);
            bus.mem.insert(base + 1, (mv >> 8) as u8);
        }
        let convert = |cpu: &mut Avr, bus: &mut MockBus, adcsrb: u8, admux: u8| {
            cpu.data_write(0x7B, adcsrb, bus).unwrap();
            cpu.data_write(0x7C, admux, bus).unwrap();
            cpu.data_write(0x7A, 0xC0, bus).unwrap();
            u32::from(cpu.adch) << 8 | u32::from(cpu.adcl)
        };
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x40), 400 * 1024 / 5000);
        // MUX5 (bit 5) with MUX4:0 = 5 is ADC13 (PB6) ...
        assert_eq!(convert(&mut cpu, &mut bus, 0x20, 0x45), 1000 * 1024 / 5000);
        // ... while the 2560's MUX5 position (bit 3) leaves ADC5.
        assert_eq!(convert(&mut cpu, &mut bus, 0x08, 0x45), 900 * 1024 / 5000);
        // There is no ADC2: its window slot is never converted.
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x42), 0);
        // REFS = 11 is 2.56 V; the reserved 10 is taken as AVcc.
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0xC0), 400 * 1024 / 2560);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x80), 400 * 1024 / 5000);
        // 0x1E is the 1.1 V bandgap, 0x1F GND, 0x27 the unmodelled temperature
        // sensor and 0x3E (0x1E with MUX5) a differential code.
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x5E), 1100 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x5F), 0);
        assert_eq!(convert(&mut cpu, &mut bus, 0x20, 0x47), 0);
        assert_eq!(convert(&mut cpu, &mut bus, 0x20, 0x5E), 0);
    }

    #[test]
    fn u4_vectors_follow_iom32u4() {
        // avr-libc iom32u4.h `_VECTOR(n)` + 1 for each 328P vector this core raises.
        for (vec328, slot) in [
            (2, 2),   // INT0
            (3, 3),   // INT1
            (4, 10),  // PCINT0
            (7, 13),  // WDT
            (11, 17), // TIMER1 CAPT
            (12, 18), // TIMER1 COMPA
            (13, 19), // TIMER1 COMPB
            (14, 21), // TIMER1 OVF
            (15, 22), // TIMER0 COMPA
            (17, 24), // TIMER0 OVF
            (18, 25), // SPI STC
            (19, 26), // USART RX
            (22, 30), // ADC
            (23, 31), // EE READY
            (24, 29), // ANALOG COMP
            (25, 37), // TWI
            (26, 38), // SPM READY
        ] {
            assert_eq!(u4_vector(vec328), slot, "328P vector {vec328}");
        }
    }

    // ---- ATtiny1634 (AvrModel::is_tiny1634) ----

    #[test]
    fn tiny1634_core_has_the_iotn1634_map() {
        for (name, model) in [
            ("simavr-attiny1634", AvrModel::Attiny1634),
            ("attiny1634", AvrModel::Attiny1634),
            // The 13 is another map.
            ("simavr-attiny13", AvrModel::Attiny13),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
        }
        // RAMSTART 0x100, RAMEND 0x4FF, FLASHEND 0x3FFF per avr-libc iotn1634.h.
        let mut cpu = Avr::new_model(AvrModel::Attiny1634);
        let mut bus = MockBus::new();
        assert_eq!(cpu.sp, 0x04FF);
        assert_eq!(cpu.flash.len(), 16 * 1024);
        cpu.data_write(0x100, 0x11, &mut bus).unwrap();
        cpu.data_write(0x4FF, 0x5A, &mut bus).unwrap();
        assert_eq!(cpu.sram[0], 0x11);
        assert_eq!(cpu.sram[0x3FF], 0x5A);
        assert!(cpu.data_read(0x500, &bus).is_err());
        // DIDR0 (0x60) is extended I/O: stored, not SRAM.
        cpu.data_write(0x60, 0x0F, &mut bus).unwrap();
        assert_eq!(cpu.data_read(0x60, &bus).unwrap(), 0x0F);
        // 0x24 is ADMUX and 0x27 PINC here, not the x4/x5 ADCL and ADMUX.
        cpu.data_write(0x24, 0x8B, &mut bus).unwrap();
        assert_eq!(cpu.admux, 0x8B);
        assert_eq!(cpu.adcl, 0);
        // Port A: PINA 0x2F, DDRA 0x30, PORTA 0x31, PUEA 0x32.
        cpu.data_write(0x30, 0x0F, &mut bus).unwrap();
        cpu.data_write(0x31, 0x05, &mut bus).unwrap();
        bus.mem.insert(AVR_IO_MIRROR_BASE + 0x2F, 0xF0);
        assert_eq!(cpu.data_read(0x2F, &bus).unwrap(), 0xF5);
    }

    #[test]
    fn tiny1634_pull_ups_follow_puex_not_portx() {
        let mut cpu = Avr::new_model(AvrModel::Attiny1634);
        let mut bus = MockBus::new();
        let mirrored_portb = |bus: &MockBus| bus.mem[&(AVR_IO_MIRROR_BASE + 0x2D)];
        // PB0 output high, PB1 input with PORTB1 set: no pull-up on the 1634.
        cpu.data_write(0x2C, 0x01, &mut bus).unwrap();
        cpu.data_write(0x2D, 0x03, &mut bus).unwrap();
        assert_eq!(mirrored_portb(&bus), 0x01);
        assert_eq!(bus.mem[&(AVR_IO_MIRROR_BASE + 0x2C)], 0x01);
        // PUEB2 pulls the PB2 input up; PUEB0 on the output changes nothing.
        cpu.data_write(0x2E, 0x05, &mut bus).unwrap();
        assert_eq!(mirrored_portb(&bus), 0x05);
        // PB2 becomes an output driven low: its pull-up drops.
        cpu.data_write(0x2C, 0x05, &mut bus).unwrap();
        assert_eq!(mirrored_portb(&bus), 0x01);
        // PUEx is not mirrored: 0x1_002E is not part of any window.
        assert!(!bus.mem.contains_key(&(AVR_IO_MIRROR_BASE + 0x2E)));
    }

    #[test]
    fn tiny1634_adc_has_twelve_inputs_refs_10_and_adlar_in_adcsrb_bit_3() {
        let mut cpu = Avr::new_model(AvrModel::Attiny1634);
        let mut bus = MockBus::new();
        for (ch, mv) in [(0u64, 400u16), (11, 1000)] {
            let base = AVR_CLASSIC_ADC_INPUT_BASE + ch * 2;
            bus.mem.insert(base, (mv & 0xFF) as u8);
            bus.mem.insert(base + 1, (mv >> 8) as u8);
        }
        let convert = |cpu: &mut Avr, bus: &mut MockBus, admux: u8| {
            cpu.data_write(0x24, admux, bus).unwrap();
            cpu.data_write(0x23, 0xC0, bus).unwrap();
            u32::from(cpu.data_read(0x21, bus).unwrap()) << 8
                | u32::from(cpu.data_read(0x20, bus).unwrap())
        };
        assert_eq!(convert(&mut cpu, &mut bus, 0x0B), 1000 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00), 400 * 1024 / 5000);
        // REFS = 10 is the internal 1.1 V; 01 (AREF) and 11 stay VCC.
        assert_eq!(convert(&mut cpu, &mut bus, 0x8B), 1000 * 1024 / 1100);
        assert_eq!(convert(&mut cpu, &mut bus, 0x4B), 1000 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0xCB), 1000 * 1024 / 5000);
        // REFEN/ADC0EN (bits 5:4) do not move the mux off ADC11.
        assert_eq!(convert(&mut cpu, &mut bus, 0x3B), 1000 * 1024 / 5000);
        // 0x0C..0x0E have no modelled input and 0x0F is GND.
        for code in 0x0C..=0x0F {
            assert_eq!(convert(&mut cpu, &mut bus, code), 0, "MUX {code:#04x}");
        }
        // ADLAR is ADCSRB (0x22) bit 3; ADMUX bit 5 (ADLAR on the megas) is REFEN.
        cpu.data_write(0x22, 1 << 3, &mut bus).unwrap();
        convert(&mut cpu, &mut bus, 0x0B);
        let code = 1000 * 1024 / 5000;
        assert_eq!(cpu.adch, (code >> 2) as u8);
        assert_eq!(cpu.adcl, ((code & 0x03) << 6) as u8);
    }

    // ---- ATtiny2313/2313A/4313 (AvrModel::is_tiny2313) ----

    #[test]
    fn tiny2313_cores_have_the_iotn2313_map_and_no_adc() {
        for (name, model, ramend, flash) in [
            ("simavr-attiny2313", AvrModel::Attiny2313, 0x00DF, 2 * 1024),
            ("simavr-attiny2313a", AvrModel::Attiny2313, 0x00DF, 2 * 1024),
            ("simavr-attiny4313", AvrModel::Attiny4313, 0x015F, 4 * 1024),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
            let mut cpu = Avr::for_chip_name(name);
            let mut bus = MockBus::new();
            // RAMSTART 0x60, RAMEND 0xDF / 0x15F, FLASHEND 0x7FF / 0xFFF.
            assert_eq!(cpu.sp, ramend, "{name}");
            assert_eq!(cpu.flash.len(), flash, "{name}");
            cpu.data_write(0x60, 0x11, &mut bus).unwrap();
            cpu.data_write(ramend, 0x5A, &mut bus).unwrap();
            assert_eq!(cpu.sram[0], 0x11);
            assert!(cpu.data_read(ramend + 1, &bus).is_err(), "{name}");
            // 0x24..0x27 are PCMSK1/PCMSK2 and timer registers: stored, no ADC.
            cpu.data_write(0x27, 0x45, &mut bus).unwrap();
            cpu.data_write(0x26, 0xC0, &mut bus).unwrap();
            assert_eq!((cpu.admux, cpu.adcsra), (0, 0), "{name}");
            assert_eq!(cpu.data_read(0x27, &bus).unwrap(), 0x45);
            assert_eq!(cpu.data_read(0x26, &bus).unwrap(), 0xC0);
        }
        // Ports D, B, A at PINx 0x30, 0x36, 0x39; 0x33 (PINC elsewhere) is not a port.
        let mut cpu = Avr::new_model(AvrModel::Attiny2313);
        let mut bus = MockBus::new();
        for pin in [0x30u16, 0x36, 0x39] {
            cpu.data_write(pin + 1, 0x0F, &mut bus).unwrap();
            cpu.data_write(pin + 2, 0x05, &mut bus).unwrap();
            bus.mem.insert(AVR_IO_MIRROR_BASE + u64::from(pin), 0xF0);
            assert_eq!(cpu.data_read(pin, &bus).unwrap(), 0xF5, "PINx {pin:#x}");
        }
        cpu.data_write(0x34, 0x01, &mut bus).unwrap();
        assert!(!bus.mem.contains_key(&(AVR_IO_MIRROR_BASE + 0x34)));
    }

    #[test]
    fn tiny2313_pud_is_mcucr_bit_7() {
        struct PudBus {
            inner: MockBus,
            pud: Vec<bool>,
        }
        impl Bus for PudBus {
            fn read_u8(&self, addr: u64) -> SimResult<u8> {
                self.inner.read_u8(addr)
            }
            fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
                self.inner.write_u8(addr, value)
            }
            fn tick_peripherals(&mut self) -> Vec<u32> {
                Vec::new()
            }
            fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
                Ok(())
            }
            fn config(&self) -> &SimulationConfig {
                &self.inner.config
            }
            fn set_pull_ups_disabled(&mut self, disabled: bool) {
                self.pud.push(disabled);
            }
        }
        let mut bus = PudBus {
            inner: MockBus::new(),
            pud: Vec::new(),
        };
        let mut cpu = Avr::new_model(AvrModel::Attiny4313);
        // Bit 6 is SM1 here (PUD on the x5/x4/13), bit 7 is PUD.
        cpu.data_write(TINY_X5_MCUCR, TINY_X5_MCUCR_PUD, &mut bus)
            .unwrap();
        assert!(bus.pud.is_empty());
        cpu.data_write(TINY_X5_MCUCR, TINY2313_MCUCR_PUD, &mut bus)
            .unwrap();
        cpu.data_write(TINY_X5_MCUCR, 0, &mut bus).unwrap();
        assert_eq!(bus.pud, [true, false]);
    }

    #[test]
    fn tiny2313_comparator_compares_ain0_or_bandgap_against_ain1() {
        let mut cpu = Avr::new_model(AvrModel::Attiny2313);
        let mut bus = MockBus::new();
        let drive = |bus: &mut MockBus, ch: u64, mv: u16| {
            let base = AVR_CLASSIC_ADC_INPUT_BASE + ch * 2;
            bus.mem.insert(base, (mv & 0xFF) as u8);
            bus.mem.insert(base + 1, (mv >> 8) as u8);
        };
        let aco = |cpu: &Avr, bus: &MockBus| cpu.data_read(0x28, bus).unwrap() & ACSR_ACO != 0;
        drive(&mut bus, 0, 900);
        drive(&mut bus, 1, 600);
        assert!(aco(&cpu, &bus), "AIN0 900 mV above AIN1 600 mV");
        drive(&mut bus, 0, 400);
        assert!(!aco(&cpu, &bus), "AIN0 400 mV below AIN1 600 mV");
        drive(&mut bus, 0, 600);
        assert!(
            !aco(&cpu, &bus),
            "equal inputs: ACO needs AIN0 strictly above"
        );
        // ACBG replaces AIN0 with the 1.1 V bandgap.
        cpu.data_write(0x28, ACSR_ACBG, &mut bus).unwrap();
        assert!(aco(&cpu, &bus), "1.1 V above AIN1 600 mV");
        drive(&mut bus, 1, 1200);
        assert!(!aco(&cpu, &bus), "1.1 V below AIN1 1.2 V");
        // ACD powers it off; ACO and ACI written by firmware are not stored.
        drive(&mut bus, 1, 0);
        cpu.data_write(
            0x28,
            ACSR_ACD | ACSR_ACBG | ACSR_ACO | ACSR_ACI | 0x0B,
            &mut bus,
        )
        .unwrap();
        assert_eq!(
            cpu.data_read(0x28, &bus).unwrap(),
            ACSR_ACD | ACSR_ACBG | 0x0B
        );
        // Other cores keep 0x28 as a plain register (the 328P's TCNT0 area etc.).
        let mut x5 = Avr::new_model(AvrModel::Attiny85);
        x5.data_write(0x28, 0x00, &mut bus).unwrap();
        assert_eq!(x5.data_read(0x28, &bus).unwrap(), 0x00);
    }

    // ---- AT90USB162 (AvrModel::is_usb162) ----

    #[test]
    fn usb162_core_has_the_iousb162_map_and_no_adc() {
        assert_eq!(
            AvrModel::from_chip_name("simavr-at90usb162"),
            AvrModel::At90usb162
        );
        // The AT90USB82 has 8 KB flash and SRAM to 0x1FF: not this core.
        assert_ne!(
            AvrModel::from_chip_name("simavr-at90usb82"),
            AvrModel::At90usb162
        );
        let mut cpu = Avr::for_chip_name("simavr-at90usb162");
        let mut bus = MockBus::new();
        // RAMSTART 0x100, RAMEND 0x2FF, FLASHEND 0x3FFF.
        assert_eq!(cpu.sp, 0x02FF);
        assert_eq!(cpu.flash.len(), 16 * 1024);
        cpu.data_write(0x0100, 0x11, &mut bus).unwrap();
        cpu.data_write(0x02FF, 0x5A, &mut bus).unwrap();
        assert_eq!(cpu.sram[0], 0x11);
        assert!(cpu.data_read(0x0300, &bus).is_err());
        // No ADC: ADMUX/ADCSRA are plain registers, no conversion starts.
        bus.mem.insert(AVR_ADC_INPUT_BASE, 0x88);
        bus.mem.insert(AVR_ADC_INPUT_BASE + 1, 0x13);
        cpu.data_write(0x7C, 0x40, &mut bus).unwrap();
        cpu.data_write(0x7A, 0xC0, &mut bus).unwrap();
        assert_eq!((cpu.admux, cpu.adcsra, cpu.adcl, cpu.adch), (0, 0, 0, 0));
        assert_eq!(cpu.data_read(0x7A, &bus).unwrap(), 0xC0);
        assert_eq!(cpu.data_read(0x78, &bus).unwrap(), 0x00);
        // Ports B, C, D at PINx 0x23, 0x26, 0x29, as on the 328P.
        for pin in [0x23u16, 0x26, 0x29] {
            cpu.data_write(pin + 1, 0x0F, &mut bus).unwrap();
            cpu.data_write(pin + 2, 0x05, &mut bus).unwrap();
            bus.mem.insert(AVR_IO_MIRROR_BASE + u64::from(pin), 0xF0);
            assert_eq!(cpu.data_read(pin, &bus).unwrap(), 0xF5, "PINx {pin:#x}");
        }
        // INT0..7/PCINT are wired to other pins: stored only.
        assert!(!cpu.has_328p_ext_int());
    }

    #[test]
    fn usb162_vectors_follow_iousb162() {
        // (328P vector, AT90USB162 `_VECTOR(n)` + 1)
        for (vec328, slot) in [
            (2, 2),   // INT0
            (3, 3),   // INT1
            (4, 10),  // PCINT0
            (5, 11),  // PCINT1
            (7, 14),  // WDT
            (11, 15), // TIMER1 CAPT
            (12, 16), // TIMER1 COMPA
            (13, 17), // TIMER1 COMPB
            (14, 19), // TIMER1 OVF
            (15, 20), // TIMER0 COMPA
            (16, 21), // TIMER0 COMPB
            (17, 22), // TIMER0 OVF
            (18, 23), // SPI STC
            (24, 27), // ANALOG COMP
            (23, 28), // EE READY
            (26, 29), // SPM READY
        ] {
            assert_eq!(usb162_vector(vec328), slot, "328P vector {vec328}");
        }
    }

    #[test]
    fn oc_pads_follow_port_where_the_328p_pins_are_not_oc() {
        // COM0A = 10 in fast PWM drives OC0A, which is PD6 only on the 328P.
        // The AT90USB162 (OC0A PB7), the 16M1/64M1 (OC0A PD3) and the 169P
        // (OC0A PB4) leave
        // their pads to PORTx rather than toggle the 328P's PD6.
        for (model, overlaid) in [
            (AvrModel::Atmega328p, [0, 1 << 6]),
            (AvrModel::At90usb162, [0, 0]),
            (AvrModel::Atmega16m1, [0, 0]),
            (AvrModel::Atmega64m1, [0, 0]),
            (AvrModel::Atmega169p, [0, 0]),
        ] {
            let mut cpu = Avr::new_model(model);
            let mut bus = MockBus::new();
            cpu.data_write(0x2A, 0xFF, &mut bus).unwrap(); // DDRD
            cpu.data_write(0x47, 0x80, &mut bus).unwrap(); // OCR0A
            cpu.data_write(0x44, 0x83, &mut bus).unwrap(); // TCCR0A
            cpu.sync_oc_pins(&mut bus);
            assert_eq!(cpu.oc_mask, overlaid, "{model:?}");
        }
    }

    // ---- ATmega16M1/64M1 (AvrModel::is_m1) ----

    #[test]
    fn m1_cores_need_the_exact_part_name_and_have_the_iom16m1_map() {
        for (name, model) in [
            ("simavr-atmega16m1", AvrModel::Atmega16m1),
            ("simavr-atmega64m1", AvrModel::Atmega64m1),
            ("atmega16m1", AvrModel::Atmega16m1),
            // The 32M1 has 2 KB SRAM and 32 KB flash: not either core.
            ("atmega32m1", AvrModel::Atmega328p),
            // The classic ATmega16 is another map.
            ("atmega16", AvrModel::Atmega16),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
        }
        // RAMSTART 0x100; RAMEND 0x4FF / 0x10FF, FLASHEND 0x3FFF / 0xFFFF.
        for (model, ramend, flash) in [
            (AvrModel::Atmega16m1, 0x04FFu16, 16 * 1024),
            (AvrModel::Atmega64m1, 0x10FF, 64 * 1024),
        ] {
            let mut cpu = Avr::new_model(model);
            let mut bus = MockBus::new();
            assert_eq!(cpu.sp, ramend, "{model:?}");
            assert_eq!(cpu.flash.len(), flash, "{model:?}");
            cpu.data_write(0x0100, 0xA5, &mut bus).unwrap();
            cpu.data_write(ramend, 0x5A, &mut bus).unwrap();
            assert_eq!(cpu.sram[0], 0xA5);
            assert!(cpu.data_read(ramend + 1, &bus).is_err(), "{model:?}");
            // Ports B..E; 0x20..0x22 (port A) and 0x2F (the 32U4's PINF) are not.
            assert!(!cpu.is_pin_reg(0x20) && !cpu.is_port_reg(0x22));
            assert!(cpu.is_pin_reg(0x23) && cpu.is_pin_reg(0x2C) && cpu.is_port_reg(0x2E));
            assert!(!cpu.is_pin_reg(0x2F) && !cpu.is_port_reg(0x2F));
            cpu.data_write(0x2D, 0x0F, &mut bus).unwrap();
            cpu.data_write(0x2E, 0x05, &mut bus).unwrap();
            bus.mem.insert(AVR_IO_MIRROR_BASE + 0x2C, 0xF0);
            assert_eq!(cpu.data_read(0x2C, &bus).unwrap(), 0xF5);
            // INT0..3 are PD6, PB2, PB5, PC0 here: stored only.
            assert!(!cpu.has_328p_ext_int());
        }
    }

    #[test]
    fn m1_adc_has_eleven_inputs_vcc_over_4_and_the_bandgap_on_0x11() {
        let mut cpu = Avr::new_model(AvrModel::Atmega16m1);
        let mut bus = MockBus::new();
        for (ch, mv) in [(0u64, 400u16), (2, 700), (8, 900), (10, 1000)] {
            let base = AVR_MEGA_ADC_INPUT_BASE + ch * 2;
            bus.mem.insert(base, (mv & 0xFF) as u8);
            bus.mem.insert(base + 1, (mv >> 8) as u8);
        }
        let convert = |cpu: &mut Avr, bus: &mut MockBus, adcsrb: u8, admux: u8| {
            cpu.data_write(0x7B, adcsrb, bus).unwrap();
            cpu.data_write(0x7C, admux, bus).unwrap();
            cpu.data_write(0x7A, 0xC0, bus).unwrap();
            u32::from(cpu.adch) << 8 | u32::from(cpu.adcl)
        };
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x40), 400 * 1024 / 5000);
        // MUX4:0 reaches ADC8 and ADC10 (PC4, PC6) with no MUX5 ...
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x48), 900 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x4A), 1000 * 1024 / 5000);
        // ... and ADCSRB bits 3/5 (MUX5 elsewhere; AREFEN here) change nothing.
        assert_eq!(convert(&mut cpu, &mut bus, 0x28, 0x42), 700 * 1024 / 5000);
        // ADLAR is ADMUX bit 5, as on the 328P.
        assert_eq!(
            convert(&mut cpu, &mut bus, 0x00, 0x60),
            (400 * 1024 / 5000) << 6
        );
        // REFS = 11 is 2.56 V; the reserved 10 and AREF 00 are taken as AVcc.
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0xC0), 400 * 1024 / 2560);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x80), 400 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x00), 400 * 1024 / 5000);
        // 0x0C is VCC/4, 0x11 the 1.1 V bandgap, 0x12 GND; the temperature
        // sensor (0x0B), ISRC (0x0D), AMP0..2 (0x0E..0x10) and the 328P/mega
        // bandgap code 0x1E are not modelled inputs.
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x4C), 1250 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x51), 1100 * 1024 / 5000);
        for mux in [0x0B, 0x0D, 0x0E, 0x0F, 0x10, 0x12, 0x13, 0x1E] {
            assert_eq!(
                convert(&mut cpu, &mut bus, 0x00, 0x40 | mux),
                0,
                "MUX {mux:#x}"
            );
        }
    }

    #[test]
    fn m1_vectors_follow_iom16m1() {
        // (328P vector, ATmega16M1/64M1 `_VECTOR(n)` + 1)
        for (vec328, slot) in [
            (2, 8),   // INT0
            (3, 9),   // INT1
            (4, 23),  // PCINT0
            (5, 24),  // PCINT1
            (6, 25),  // PCINT2
            (7, 29),  // WDT
            (11, 12), // TIMER1 CAPT
            (12, 13), // TIMER1 COMPA
            (13, 14), // TIMER1 COMPB
            (14, 15), // TIMER1 OVF
            (15, 16), // TIMER0 COMPA
            (16, 17), // TIMER0 COMPB
            (17, 18), // TIMER0 OVF
            (18, 27), // SPI STC
            (22, 28), // ADC
            (23, 30), // EE READY
            (26, 31), // SPM READY
        ] {
            assert_eq!(m1_vector(vec328), slot, "328P vector {vec328}");
        }
    }

    // ---- ATmega328PB (AvrModel::Atmega328pb) ----

    #[test]
    fn m328pb_core_needs_the_exact_part_name_and_adds_port_e() {
        for (name, model) in [
            ("simavr-atmega328pb", AvrModel::Atmega328pb),
            ("atmega328pb", AvrModel::Atmega328pb),
            // The 328P has no port E: 0x2C..0x2E stay stored there.
            ("atmega328p", AvrModel::Atmega328p),
            ("simavr-atmega328", AvrModel::Atmega328p),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
        }
        let mut cpu = Avr::new_model(AvrModel::Atmega328pb);
        let mut bus = MockBus::new();
        // iom328pb.h: RAMEND 0x8FF and FLASHEND 0x7FFF, as on the 328P.
        assert_eq!(cpu.sp, 0x08FF);
        assert_eq!(cpu.flash.len(), 32 * 1024);
        // PINE/DDRE/PORTE at 0x2C..0x2E; 0x2F is reserved, not a port.
        assert!(cpu.is_pin_reg(0x23) && cpu.is_pin_reg(0x2C));
        assert!(cpu.is_port_reg(0x2E) && !cpu.is_port_reg(0x2F));
        // PORTE/DDRE drive the mirror; PINE reads the bus-side pads.
        cpu.data_write(0x2E, 0x0C, &mut bus).unwrap();
        cpu.data_write(0x2D, 0x03, &mut bus).unwrap();
        assert_eq!(bus.mem[&(AVR_IO_MIRROR_BASE + 0x2D)], 0x03);
        bus.mem.insert(AVR_IO_MIRROR_BASE + 0x2C, 0x04);
        assert_eq!(cpu.data_read(0x2C, &bus).unwrap() & 0x0C, 0x04);
        // The 328P core leaves the same writes off the bus.
        let mut plain = Avr::new_model(AvrModel::Atmega328p);
        let mut plain_bus = MockBus::new();
        plain.data_write(0x2D, 0x03, &mut plain_bus).unwrap();
        assert!(!plain_bus.mem.contains_key(&(AVR_IO_MIRROR_BASE + 0x2D)));
        // INT0/INT1 (PD2/PD3) and PCINT0..2 are the 328P's pins.
        assert!(cpu.has_328p_ext_int());
    }

    #[test]
    fn m328pb_adc_converts_adc6_adc7_with_the_328p_mux_table() {
        let mut cpu = Avr::new_model(AvrModel::Atmega328pb);
        let mut bus = MockBus::new();
        let drive = |bus: &mut MockBus, ch: u64, mv: u16| {
            let base = AVR_ADC_INPUT_BASE + ch * 2;
            bus.mem.insert(base, (mv & 0xFF) as u8);
            bus.mem.insert(base + 1, (mv >> 8) as u8);
        };
        let convert = |cpu: &mut Avr, bus: &mut MockBus, admux: u8| {
            cpu.data_write(0x7C, admux, bus).unwrap();
            cpu.data_write(0x7A, 0xC0, bus).unwrap();
            u32::from(cpu.adch) << 8 | u32::from(cpu.adcl)
        };
        // ADC6/ADC7 (PE2/PE3) against AVcc and the internal 1.1 V.
        drive(&mut bus, 6, 2500);
        drive(&mut bus, 7, 700);
        assert_eq!(convert(&mut cpu, &mut bus, 0x46), 2500 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x47), 700 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0xC7), 700 * 1024 / 1100);
        // sim_mega328pb.c muxmode: 0x0E is the 1.1 V bandgap, 0x0F GND, 0x08
        // the temperature sensor (not modelled).
        assert_eq!(convert(&mut cpu, &mut bus, 0x4E), 1100 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x4F), 0);
        assert_eq!(convert(&mut cpu, &mut bus, 0x48), 0);
    }

    // ---- ATmega169P/169PA (AvrModel::is_m169p) ----

    #[test]
    fn m169p_core_needs_the_exact_part_name_and_has_the_iom169p_map() {
        for (name, model) in [
            ("simavr-atmega169p", AvrModel::Atmega169p),
            ("atmega169pa", AvrModel::Atmega169p),
            // The original ATmega169 is another revision: not this core.
            ("atmega169", AvrModel::Atmega328p),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
        }
        let mut cpu = Avr::new_model(AvrModel::Atmega169p);
        let mut bus = MockBus::new();
        // RAMSTART 0x100, RAMEND 0x4FF, FLASHEND 0x3FFF.
        assert_eq!(cpu.sp, 0x04FF);
        assert_eq!(cpu.flash.len(), 16 * 1024);
        cpu.data_write(0x0100, 0xA5, &mut bus).unwrap();
        cpu.data_write(0x04FF, 0x5A, &mut bus).unwrap();
        assert_eq!(cpu.sram[0], 0xA5);
        assert!(cpu.data_read(0x0500, &bus).is_err());
        // Ports A..G at 0x20..0x34; 0x35 is TIFR0, not a port.
        assert!(cpu.is_pin_reg(0x20) && cpu.is_pin_reg(0x2F) && cpu.is_pin_reg(0x32));
        assert!(cpu.is_port_reg(0x34) && !cpu.is_port_reg(0x35));
        assert!(!cpu.is_pin_reg(0x100));
        // PORTF/DDRF drive the mirror; PINF reads the bus-side pads.
        cpu.data_write(0x30, 0x0F, &mut bus).unwrap();
        cpu.data_write(0x31, 0x05, &mut bus).unwrap();
        bus.mem.insert(AVR_IO_MIRROR_BASE + 0x2F, 0xF0);
        assert_eq!(cpu.data_read(0x2F, &bus).unwrap(), 0xF5);
        // INT0 is PD1 and PCINT0..15 are ports E and B: stored only.
        assert!(!cpu.has_328p_ext_int());
    }

    #[test]
    fn m169p_adc_has_single_ended_differential_bandgap_and_gnd_codes() {
        let mut cpu = Avr::new_model(AvrModel::Atmega169p);
        let mut bus = MockBus::new();
        let drive = |bus: &mut MockBus, ch: u64, mv: u16| {
            let base = AVR_MEGA_ADC_INPUT_BASE + ch * 2;
            bus.mem.insert(base, (mv & 0xFF) as u8);
            bus.mem.insert(base + 1, (mv >> 8) as u8);
        };
        let convert = |cpu: &mut Avr, bus: &mut MockBus, adcsrb: u8, admux: u8| {
            cpu.data_write(0x7B, adcsrb, bus).unwrap();
            cpu.data_write(0x7C, admux, bus).unwrap();
            cpu.data_write(0x7A, 0xC0, bus).unwrap();
            u32::from(cpu.adch) << 8 | u32::from(cpu.adcl)
        };
        // Datasheet 8018P §22.7: ADMUX = 0xFB is ADC3 - ADC2 at 1.1 V; with
        // 300 mV and 500 mV, ADC = 512 * (300 - 500) / 1100 = -93 = 0x3A3,
        // ADCL 0xA3 / ADCH 0x03 right-adjusted.
        drive(&mut bus, 2, 500);
        drive(&mut bus, 3, 300);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0xDB), 0x3A3);
        assert_eq!((cpu.adch, cpu.adcl), (0x03, 0xA3));
        // ADLAR left-adjusts the same 10 bits.
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0xFB), 0x3A3 << 6);
        // Swapped sign: ADC2 - ADC2 is 0, ADC3 - ADC1 positive.
        drive(&mut bus, 1, 100);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0xDA), 0);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0xD3), 200 * 512 / 1100);
        // Differential saturates at -512 (0x200) and +511 (0x1FF).
        drive(&mut bus, 7, 5000);
        drive(&mut bus, 0, 0);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0xD7), 0x1FF);
        // AVcc reference: ADC0 - ADC1 = -100 mV is -10.24, truncated to -10.
        assert_eq!(
            convert(&mut cpu, &mut bus, 0x00, 0x50),
            (-10i32 & 0x3FF) as u32
        );
        drive(&mut bus, 1, 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0xD0), 0x200);
        // Single-ended ADC0..7 against AVcc, and REFS = 11 the internal 1.1 V.
        drive(&mut bus, 6, 900);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x46), 900 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0xC6), 900 * 1024 / 1100);
        // REFS = 10 is reserved and 00 is AREF: both taken as AVcc.
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x86), 900 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x06), 900 * 1024 / 5000);
        // No MUX5: ADCSRB bit 3 (the 2560's MUX5) leaves ADC6.
        assert_eq!(convert(&mut cpu, &mut bus, 0x08, 0x46), 900 * 1024 / 5000);
        // 0x1E is the 1.1 V bandgap, 0x1F GND, 0x08..0x0F N/A; 0x0E (the
        // 328P's bandgap code) has no input here.
        assert_eq!(convert(&mut cpu, &mut bus, 0x00, 0x5E), 1100 * 1024 / 5000);
        for mux in [0x08, 0x0B, 0x0E, 0x0F, 0x1F] {
            assert_eq!(
                convert(&mut cpu, &mut bus, 0x00, 0x40 | mux),
                0,
                "MUX {mux:#x}"
            );
        }
    }

    #[test]
    fn m169p_vectors_follow_iom169p() {
        // (328P vector, ATmega169P `_VECTOR(n)` + 1)
        for (vec328, slot) in [
            (2, 2),   // INT0
            (4, 3),   // PCINT0
            (5, 4),   // PCINT1
            (8, 5),   // TIMER2 COMP
            (10, 6),  // TIMER2 OVF
            (11, 7),  // TIMER1 CAPT
            (12, 8),  // TIMER1 COMPA
            (13, 9),  // TIMER1 COMPB
            (14, 10), // TIMER1 OVF
            (15, 11), // TIMER0 COMP
            (17, 12), // TIMER0 OVF
            (18, 13), // SPI STC
            (19, 14), // USART0 RX
            (20, 15), // USART0 UDRE
            (21, 16), // USART0 TX
            (24, 19), // ANALOG COMP
            (22, 20), // ADC
            (23, 21), // EE READY
            (26, 22), // SPM READY
        ] {
            assert_eq!(m169p_vector(vec328), slot, "328P vector {vec328}");
        }
    }

    #[test]
    fn m169p_core_enters_its_own_vector_slots() {
        // A pending Timer0 overflow (328P 17) enters TIMER0_OVF_vect, which
        // is slot 12 (word 11 * 2) on the 169P, not the 328P's 17.
        let mut cpu = Avr::new_model(AvrModel::Atmega169p);
        let mut bus = MockBus::new();
        cpu.pending_irq = 1 << VEC_TIMER0_OVF;
        cpu.sreg = 0x80;
        step_n(&mut cpu, &mut bus, 1);
        assert_eq!(cpu.pc, 11 * 4);
    }

    #[test]
    fn usb162_comparator_compares_ain0_or_bandgap_against_ain1() {
        let mut cpu = Avr::new_model(AvrModel::At90usb162);
        let mut bus = MockBus::new();
        assert_eq!(AvrModel::At90usb162.comparator_acsr(), Some(0x50));
        assert_eq!(AvrModel::Attiny2313.comparator_acsr(), Some(0x28));
        assert_eq!(AvrModel::Atmega328p.comparator_acsr(), None);
        let drive = |bus: &mut MockBus, ch: u64, mv: u16| {
            let base = AVR_ADC_INPUT_BASE + ch * 2;
            bus.mem.insert(base, (mv & 0xFF) as u8);
            bus.mem.insert(base + 1, (mv >> 8) as u8);
        };
        let aco = |cpu: &Avr, bus: &MockBus| cpu.data_read(0x50, bus).unwrap() & ACSR_ACO != 0;
        drive(&mut bus, 0, 900);
        drive(&mut bus, 1, 600);
        assert!(aco(&cpu, &bus), "AIN0 900 mV above AIN1 600 mV");
        drive(&mut bus, 0, 400);
        assert!(!aco(&cpu, &bus), "AIN0 400 mV below AIN1 600 mV");
        cpu.data_write(0x50, ACSR_ACBG, &mut bus).unwrap();
        assert!(aco(&cpu, &bus), "1.1 V above AIN1 600 mV");
        drive(&mut bus, 1, 1200);
        assert!(!aco(&cpu, &bus), "1.1 V below AIN1 1.2 V");
        drive(&mut bus, 1, 0);
        cpu.data_write(
            0x50,
            ACSR_ACD | ACSR_ACBG | ACSR_ACO | ACSR_ACI | 0x0B,
            &mut bus,
        )
        .unwrap();
        assert_eq!(
            cpu.data_read(0x50, &bus).unwrap(),
            ACSR_ACD | ACSR_ACBG | 0x0B
        );
        // The 328P keeps 0x50 as a stored register: no composed ACO.
        let mut m328 = Avr::new_model(AvrModel::Atmega328p);
        drive(&mut bus, 0, 900);
        m328.data_write(0x50, 0x00, &mut bus).unwrap();
        assert_eq!(m328.data_read(0x50, &bus).unwrap(), 0x00);
    }

    // ---- ATtiny24/44/84 (AvrModel::is_tiny_x4) ----

    #[test]
    fn tiny_x4_cores_need_the_exact_part_name_and_have_the_iotnx4_map() {
        for (name, model) in [
            ("simavr-attiny24", AvrModel::Attiny24),
            ("simavr-attiny44", AvrModel::Attiny44),
            ("simavr-attiny84", AvrModel::Attiny84),
            ("attiny84a", AvrModel::Attiny84),
            // The 261/461/861, 13 and 2313 are other maps.
            ("attiny861", AvrModel::Atmega328p),
            ("attiny13", AvrModel::Attiny13),
            ("attiny4313", AvrModel::Attiny4313),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
        }
        // RAMEND/FLASHEND + 1 per avr-libc iotn24.h/iotn44.h/iotn84.h.
        for (model, ramend, flash) in [
            (AvrModel::Attiny24, 0x00DF, 2 * 1024),
            (AvrModel::Attiny44, 0x015F, 4 * 1024),
            (AvrModel::Attiny84, 0x025F, 8 * 1024),
        ] {
            let mut cpu = Avr::new_model(model);
            let mut bus = MockBus::new();
            assert_eq!(cpu.sp, ramend, "{model:?}: SP resets to RAMEND");
            assert_eq!(cpu.flash.len(), flash);
            cpu.data_write(0x60, 0x5A, &mut bus).unwrap();
            assert_eq!(cpu.sram[0], 0x5A);
            assert!(cpu.data_read(ramend + 1, &bus).is_err(), "{model:?}");
            // Ports A (0x39..0x3B) and B (0x36..0x38) compose and mirror.
            for pin in [0x36, 0x39] {
                cpu.data_write(pin + 1, 0x0F, &mut bus).unwrap();
                cpu.data_write(pin + 2, 0x05, &mut bus).unwrap();
                assert_eq!(
                    bus.mem.get(&(AVR_IO_MIRROR_BASE + u64::from(pin) + 2)),
                    Some(&0x05)
                );
                bus.mem.insert(AVR_IO_MIRROR_BASE + u64::from(pin), 0xF0);
                assert_eq!(cpu.data_read(pin, &bus).unwrap(), 0xF5, "{pin:#x}");
            }
            // The classic parts' PORTC address is another register here.
            cpu.data_write(0x35, 0x11, &mut bus).unwrap();
            assert!(!bus.mem.contains_key(&(AVR_IO_MIRROR_BASE + 0x35)));
        }
    }

    #[test]
    fn tiny_x4_adc_has_eight_channels_six_mux_bits_and_adlar_in_adcsrb() {
        let mut cpu = Avr::new_model(AvrModel::Attiny84);
        let mut bus = MockBus::new();
        // ADC7 (PA7) holds 1000 mV in the window at 0x1_0060.
        let base = AVR_CLASSIC_ADC_INPUT_BASE + 7 * 2;
        bus.mem.insert(base, (1000u16 & 0xFF) as u8);
        bus.mem.insert(base + 1, (1000u16 >> 8) as u8);
        let convert = |cpu: &mut Avr, bus: &mut MockBus, admux: u8| {
            cpu.data_write(0x27, admux, bus).unwrap();
            cpu.data_write(0x26, 0xC0, bus).unwrap();
            u32::from(cpu.adch) << 8 | u32::from(cpu.adcl)
        };
        assert_eq!(convert(&mut cpu, &mut bus, 0x07), 1000 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x87), 1000 * 1024 / 1100);
        // AREF and the reserved 11 are taken as VCC.
        assert_eq!(convert(&mut cpu, &mut bus, 0x47), 1000 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0xC7), 1000 * 1024 / 5000);
        assert_eq!(convert(&mut cpu, &mut bus, 0x21), 1100 * 1024 / 5000);
        // ADMUX bit 5 is MUX5, not ADLAR: 0x27 is a differential code.
        for mux in [0x08, 0x0E, 0x1E, 0x20, 0x22, 0x27] {
            assert_eq!(convert(&mut cpu, &mut bus, mux), 0, "MUX {mux:#04x}");
        }
        // ADLAR is ADCSRB bit 4.
        cpu.data_write(0x23, 1 << 4, &mut bus).unwrap();
        let code = 1000 * 1024 / 5000;
        convert(&mut cpu, &mut bus, 0x07);
        assert_eq!(u32::from(cpu.adch), code >> 2);
        assert_eq!(u32::from(cpu.adcl), (code & 3) << 6);
    }

    #[test]
    fn tiny_x4_mcucr_pud_disables_pull_ups() {
        let mut cpu = Avr::new_model(AvrModel::Attiny24);
        assert_eq!(cpu.io_space_pud(TINY_X5_MCUCR), Some(TINY_X5_MCUCR_PUD));
        assert_eq!(cpu.io_space_pud(CLASSIC_SFIOR), None);
        cpu.model = AvrModel::Atmega8;
        assert_eq!(cpu.io_space_pud(TINY_X5_MCUCR), None);
    }

    // ---- ATmega128RFA1/128RFR2 (AvrModel::is_rf) ----

    #[test]
    fn rf_core_needs_the_exact_part_name_and_has_the_rf_memory_map() {
        for (name, model) in [
            ("simavr-atmega128rfa1", AvrModel::Atmega128Rf),
            ("simavr-atmega128rfr2", AvrModel::Atmega128Rf),
            ("atmega128rfa1", AvrModel::Atmega128Rf),
            // Other RFR2 sizes are other memory maps.
            ("atmega256rfr2", AvrModel::Atmega328p),
            ("atmega2564rfr2", AvrModel::Atmega328p),
            ("atmega64rfr2", AvrModel::Atmega328p),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
        }
        // RAMSTART/RAMEND/FLASHEND + 1 per avr-libc iom128rfa1.h/iom128rfr2.h.
        let rf = AvrModel::Atmega128Rf;
        assert_eq!((rf.sram_start(), rf.ramend()), (0x0200, 0x41FF));
        assert_eq!(rf.flash_size(), 128 * 1024);
        assert!(!rf.three_byte_pc() && !rf.supports_eind());
        let cpu = Avr::new_model(rf);
        // Ports A..G only: PING (0x32) is a port, 0x100 (PINH on the 2560) is not.
        assert!(cpu.is_pin_reg(0x32) && cpu.is_port_reg(0x34));
        assert!(!cpu.is_pin_reg(0x100) && !cpu.is_port_reg(0x100));
    }

    // ---- ATmega164/324/644/1284 (AvrModel::is_xx4) ----

    #[test]
    fn port_a_cores_need_the_exact_part_name() {
        for (name, model) in [
            ("simavr-atmega164", AvrModel::Atmega164),
            ("atmega164pa", AvrModel::Atmega164),
            ("simavr-atmega324", AvrModel::Atmega324),
            ("atmega324p", AvrModel::Atmega324),
            ("simavr-atmega644", AvrModel::Atmega644),
            ("atmega644a", AvrModel::Atmega644),
            ("simavr-atmega1284p", AvrModel::Atmega1284),
            ("atmega1284", AvrModel::Atmega1284),
            // RF parts with the same digits are another map.
            ("atmega644rfr2", AvrModel::Atmega328p),
            ("atmega1284rfr2", AvrModel::Atmega328p),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name}");
            assert_eq!(model.is_xx4(), model != AvrModel::Atmega328p, "{name}");
        }
    }

    #[test]
    fn port_a_map_keeps_328p_sram_start_and_mirrors_port_a() {
        // RAMEND and FLASHEND + 1 per avr-libc iom164.h/iom324p.h/iom644.h/iom1284p.h.
        for (model, ramend, flash) in [
            (AvrModel::Atmega164, 0x04FF, 16 * 1024),
            (AvrModel::Atmega324, 0x08FF, 32 * 1024),
            (AvrModel::Atmega644, 0x10FF, 64 * 1024),
            (AvrModel::Atmega1284, 0x40FF, 128 * 1024),
        ] {
            let mut cpu = Avr::new_model(model);
            let mut bus = MockBus::new();
            assert_eq!(cpu.sp, ramend, "{model:?}: SP resets to RAMEND");
            assert_eq!(cpu.flash.len(), flash);
            assert!(!model.three_byte_pc(), "{model:?}: 2-byte PC up to 128 KB");
            // 0x100 is SRAM, not the mega's extended I/O.
            cpu.data_write(0x100, 0x5A, &mut bus).unwrap();
            assert_eq!(cpu.sram[0], 0x5A);
            // DDRA (0x21) and PORTA (0x22) mirror to the bus-side port.
            cpu.data_write(0x21, 0x0F, &mut bus).unwrap();
            cpu.data_write(0x22, 0x05, &mut bus).unwrap();
            assert_eq!(bus.mem[&(AVR_IO_MIRROR_BASE + 0x21)], 0x0F);
            assert_eq!(bus.mem[&(AVR_IO_MIRROR_BASE + 0x22)], 0x05);
            // PINA: driven bits from PORTA, input bits from the outside level.
            bus.mem.insert(AVR_IO_MIRROR_BASE + 0x20, 0xF0);
            assert_eq!(cpu.data_read(0x20, &bus).unwrap(), 0xF5);
            assert!(
                cpu.data_read(ramend + 1, &bus).is_err(),
                "{model:?}: past RAMEND"
            );
        }
        // The 328P has no port A: 0x22 is stored, not mirrored as a port.
        let mut bus = MockBus::new();
        let mut cpu = Avr::new_model(AvrModel::Atmega328p);
        cpu.data_write(0x22, 0x81, &mut bus).unwrap();
        assert!(!bus.mem.contains_key(&(AVR_IO_MIRROR_BASE + 0x22)));
    }

    #[test]
    fn port_a_adc_mux_and_references_follow_the_xx4_admux() {
        let mut cpu = Avr::new_model(AvrModel::Atmega644);
        let mut bus = MockBus::new();
        // ADC7 (PA7) holds 2000 mV in the 0x1_0030 window.
        let base = AVR_ADC_INPUT_BASE + 7 * 2;
        bus.mem.insert(base, (2000u16 & 0xFF) as u8);
        bus.mem.insert(base + 1, (2000u16 >> 8) as u8);
        let convert = |cpu: &mut Avr, bus: &mut MockBus, admux: u8| {
            cpu.data_write(0x7C, admux, bus).unwrap();
            cpu.data_write(0x7A, 0xC0, bus).unwrap();
            u16::from(cpu.data_read(0x79, bus).unwrap()) << 8
                | u16::from(cpu.data_read(0x78, bus).unwrap())
        };
        // REFS = 01 AVcc, 10 internal 1.1 V (clamps), 11 internal 2.56 V.
        assert_eq!(
            convert(&mut cpu, &mut bus, 0x47),
            (2000 * 1024 / 5000) as u16
        );
        assert_eq!(convert(&mut cpu, &mut bus, 0x87), 1023);
        assert_eq!(
            convert(&mut cpu, &mut bus, 0xC7),
            (2000 * 1024 / 2560) as u16
        );
        // MUX 0x1E is the 1.1 V bandgap, 0x1F GND; 0x0F (the 328P's GND) is a
        // differential code here and 0x0E (the 328P's bandgap) is too.
        assert_eq!(
            convert(&mut cpu, &mut bus, 0x5E),
            (1100 * 1024 / 5000) as u16
        );
        for mux in [0x1F, 0x0E, 0x0F, 0x09] {
            assert_eq!(convert(&mut cpu, &mut bus, 0x40 | mux), 0, "MUX {mux:#04x}");
        }
    }

    #[test]
    fn port_a_vectors_follow_the_xx4_table() {
        // (328P-numbered vector, avr-libc iomxx4.h `_VECTOR(n)` + 1).
        for (vec328, slot) in [
            (2, 2),                 // INT0
            (3, 3),                 // INT1
            (4, 5),                 // PCINT0
            (6, 7),                 // PCINT2
            (7, 9),                 // WDT
            (VEC_TIMER2_COMPA, 10), // TIMER2_COMPA
            (VEC_TIMER0_OVF, 19),   // TIMER0_OVF
            (VEC_USART_RX, 21),     // USART0_RX
            (21, 23),               // USART0_TX
            (22, 25),               // ADC
            (23, 26),               // EE_READY
            (24, 24),               // ANALOG_COMP
            (VEC_TWI, 27),          // TWI
            (26, 28),               // SPM_READY
        ] {
            assert_eq!(xx4_vector(vec328), slot, "328P vector {vec328}");
        }
        // A pending Timer0 overflow enters TIMER0_OVF_vect at word 18 * 2.
        for (model, entry) in [
            (AvrModel::Atmega1284, 18 * 4),
            (AvrModel::Atmega328p, 16 * 4),
        ] {
            let mut cpu = Avr::new_model(model);
            let mut bus = MockBus::new();
            cpu.pending_irq = 1 << VEC_TIMER0_OVF;
            cpu.sreg = 0x80;
            step_n(&mut cpu, &mut bus, 1);
            assert_eq!(cpu.pc, entry, "{model:?}");
        }
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

    /// Recording SPI slave: captures (dc_level, mosi) at each transfer so the
    /// SPDR path's D/C sample can be asserted without a full panel model.
    struct DcProbe {
        src: Option<(u64, u8)>,
        level: bool,
        frames: std::sync::Arc<std::sync::Mutex<Vec<(bool, u8)>>>,
    }

    impl SpiDevice for DcProbe {
        fn transfer(&mut self, mosi_byte: u8) -> u8 {
            self.frames.lock().unwrap().push((self.level, mosi_byte));
            0
        }
        fn cs_pin(&self) -> &str {
            "PB2"
        }
        fn set_dc_level(&mut self, level: bool) {
            self.level = level;
        }
        fn dc_source(&self) -> Option<(u64, u8)> {
            self.src
        }
        fn set_dc_source(&mut self, odr_addr: u64, bit: u8) {
            self.src = Some((odr_addr, bit));
        }
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            Some(self)
        }
    }

    #[test]
    fn spdr_write_samples_dc_pin_per_byte() {
        let mut cpu = Avr::new();
        let frames = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut probe = DcProbe {
            src: None,
            level: false,
            frames: frames.clone(),
        };
        // Fake PORT latch at 0x1000 bit 3 — whatever the kit's dc_source is.
        const ODR: u64 = 0x1000;
        const BIT: u8 = 3;
        probe.set_dc_source(ODR, BIT);
        cpu.push_spi_device(Box::new(probe));

        let mut bus = MockBus::new();
        // Assert the sampled D/C level rides with the GPIO latch per byte.
        bus.write_u8(ODR, 1 << BIT).unwrap();
        cpu.data_write(0x4E, 0xAA, &mut bus).unwrap();
        bus.write_u8(ODR, 0).unwrap();
        cpu.data_write(0x4E, 0x55, &mut bus).unwrap();
        bus.write_u8(ODR, 1 << BIT).unwrap();
        cpu.data_write(0x4E, 0xFF, &mut bus).unwrap();

        let got = frames.lock().unwrap().clone();
        assert_eq!(
            got,
            vec![(true, 0xAA), (false, 0x55), (true, 0xFF)],
            "SPDR must sample D/C per byte, in order"
        );
    }

    /// End to end: SPDR writes from the CPU clock a parking (not taken)
    /// ST7789 on a real SystemBus, with D/C from PORTB.1, and the panel paints.
    #[test]
    fn spdr_writes_paint_parking_st7789() {
        use crate::bus::SystemBus;
        use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chip_path = root.join("configs/chips/atmega328p.yaml");
        let chip = ChipDescriptor::from_file(&chip_path).expect("atmega328p");
        let mut cfg = std::collections::HashMap::new();
        cfg.insert("cs_pin".into(), serde_yaml::Value::String("PB2".into()));
        cfg.insert("dc_pin".into(), serde_yaml::Value::String("PB1".into()));
        let manifest = SystemManifest {
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "spdr-e2e".to_string(),
            chip: chip_path.to_string_lossy().to_string(),
            cpu_hz: Some(16_000_000),
            external_devices: vec![ExternalDevice {
                id: "DS1".to_string(),
                r#type: "st7789-170x320".to_string(),
                connection: "spi".to_string(),
                channel: None,
                route: Default::default(),
                config: cfg,
            }],
            board_io: vec![],
            debug_uart: None,
            wifi_ap: None,
            peripherals: vec![],
            parts: Default::default(),
            memory_overrides: Default::default(),
        };
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus");
        if let Some(idx) = bus.find_peripheral_index_by_name("spi") {
            if let Some(devs) = bus.peripherals[idx].dev.spi_attached_devices_mut() {
                for d in devs.iter_mut() {
                    d.cs_select();
                }
            }
        }
        let mut cpu = Avr::for_chip_name("atmega328p");
        let portb = 0x0001_0023u64;
        bus.write_u8(portb + 1, 1 << 1).unwrap(); // DDRB.1 output
        let frames: &[(bool, u8)] = &[
            (false, 0x01),
            (false, 0x11),
            (false, 0x3A),
            (true, 0x55),
            (false, 0x29),
            (false, 0x2A),
            (true, 0),
            (true, 0),
            (true, 0),
            (true, 1),
            (false, 0x2B),
            (true, 0),
            (true, 0),
            (true, 0),
            (true, 1),
            (false, 0x2C),
            (true, 0xFF),
            (true, 0xFF),
            (true, 0xFF),
            (true, 0xFF),
            (true, 0xFF),
            (true, 0xFF),
            (true, 0xFF),
            (true, 0xFF),
        ];
        for &(dc, b) in frames {
            bus.write_u8(portb + 2, if dc { 1 << 1 } else { 0 })
                .unwrap();
            cpu.data_write(0x4E, b, &mut bus).unwrap();
        }
        let art = bus
            .display_artifact("DS1", &crate::inspect::InspectOpts::default())
            .expect("DS1 artifact");
        let painted = art.meta["painted_bytes"].as_u64().unwrap_or(0);
        assert!(
            painted > 0,
            "SPDR-driven parking ST7789 must paint; meta={:?}",
            art.meta
        );
    }
}
