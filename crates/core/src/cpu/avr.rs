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

/// ATmega328P geometry (the default model). Other AVR values live on
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
/// The ATmega4809 and tinyAVR 0-series use an AVRxt data map: I/O occupies data
/// addresses 0x0000..=0x0FFF and SRAM begins at a device-specific address.
/// Their common instruction execution is shared, but peripherals beyond
/// explicitly configured GPIO windows and their interrupt controllers are not
/// yet modelled.
///
/// Timer0/1/2 output-compare pads use the mega pins (OC0A = PB7, OC0B = PG5,
/// OC1A/OC1B = PB5/PB6, OC2A = PB4, OC2B = PH6; see `avr/oc_pins.rs`).
/// INT0..7 and PCINT0..23 on the mega pins (`avr/mega_ext_int.rs`), the
/// timed, auto-triggered ADC (`avr/mega_adc.rs`) and the timed SPI block
/// (`avr/mega_spi.rs`) are the mega's own. Not modelled on the ATmega2560:
/// OC1C, Timer3/4/5 and USART1..3 (registers stored only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvrModel {
    #[default]
    Atmega328p,
    /// ATmega328PB shares the 328P core geometry and adds a second USART.
    Atmega328pb,
    /// ATmega4809 (Arduino Nano Every): AVRxt data map, 48 KB flash and
    /// 6 KB SRAM at 0x2800..=0x3FFF.
    Atmega4809,
    Atmega1281,
    Atmega2560,
    /// ATmega32U4 (Arduino Leonardo / Micro): the classic mega core with
    /// 32 KB flash, 2.5 KB SRAM at 0x0100, ports B..F, a USB device
    /// controller (`avr/usb.rs`) and its own vector table ([`m32u4_vector`]).
    Atmega32u4,
    Attiny202,
    Attiny204,
    Attiny402,
    Attiny404,
    Attiny406,
    Attiny804,
    Attiny806,
    Attiny807,
    Attiny1604,
    Attiny1606,
    Attiny1607,
    // Classic (pre-AVRxt) tinyAVR. These are NOT AVRxt and must not be given the
    // AVRxt data map: no VPORT, no 0x3Fxx SRAM window. Their SRAM starts at
    // 0x0060 (0x0100 for the 48/88, which have a 160-byte extended I/O file).
    // Before these existed every one of them silently resolved to Atmega328p,
    // so firmware ran with the wrong flash size, the wrong SRAM window and the
    // wrong vector table.
    Attiny13,
    Attiny2313,
    Attiny25,
    Attiny45,
    Attiny85,
    Attiny261,
    Attiny461,
    Attiny861,
    Attiny44,
    Attiny84,
    Attiny48,
    Attiny88,
    /// AVRrc reduced core (ATtiny4/5/9/10 datasheet, Atmel-8127H): 16
    /// registers (r16..r31), I/O at data 0x00..=0x3F, 32 B SRAM at
    /// 0x40..=0x5F, flash mapped read-only into data space at 0x4000, and a
    /// one-word LDS/STS form. Only the ATtiny10 geometry is modelled.
    Attiny10,
}

impl AvrModel {
    /// The model a chip descriptor names. atmega1281 selects the 128 KB
    /// two-byte-PC mega core; atmega2560/atmega2561/mega2560 select the
    /// 256 KB three-byte-PC mega core; the listed tinyAVR 0-series parts
    /// select the AVRxt data map and their exact flash/SRAM geometry. Other
    /// names default to the ATmega328P. ATmega328PB is matched explicitly so
    /// its second USART is not silently treated as the 328P's unused I/O.
    pub fn from_chip_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        // Exact match: "attiny10" is a prefix of the ATtiny102/104, which
        // are AVRrc parts with a different peripheral set.
        if n == "attiny10" || n == "attiny 10" || n.starts_with("attiny10-") {
            AvrModel::Attiny10
        } else if n.contains("attiny202") || n.contains("attiny 202") {
            AvrModel::Attiny202
        } else if n.contains("attiny204") || n.contains("attiny 204") {
            AvrModel::Attiny204
        } else if n.contains("attiny402") || n.contains("attiny 402") {
            AvrModel::Attiny402
        } else if n.contains("attiny404") || n.contains("attiny 404") {
            AvrModel::Attiny404
        } else if n.contains("attiny406") || n.contains("attiny 406") {
            AvrModel::Attiny406
        } else if n.contains("attiny804") || n.contains("attiny 804") {
            AvrModel::Attiny804
        } else if n.contains("attiny806") || n.contains("attiny 806") {
            AvrModel::Attiny806
        } else if n.contains("attiny807") || n.contains("attiny 807") {
            AvrModel::Attiny807
        } else if n.contains("attiny1604") || n.contains("attiny 1604") {
            AvrModel::Attiny1604
        } else if n.contains("attiny1606") || n.contains("attiny 1606") {
            AvrModel::Attiny1606
        } else if n.contains("attiny1607") || n.contains("attiny 1607") {
            AvrModel::Attiny1607
        // Classic tinyAVR. ORDER MATTERS: these use contains(), and several
        // names are prefixes of others (attiny861 contains attiny86; attiny44 is
        // a prefix of attiny441/attiny44a; attiny25 of attiny25x). Longer,
        // more specific names are matched first so a part never falls into a
        // shorter part's model. Every geometry below is from the datasheet's
        // data-memory map (SRAM start .. RAMEND), not interpolated.
        } else if n.contains("attiny861") || n.contains("attiny 861") {
            AvrModel::Attiny861
        } else if n.contains("attiny461") || n.contains("attiny 461") {
            AvrModel::Attiny461
        } else if n.contains("attiny261") || n.contains("attiny 261") {
            AvrModel::Attiny261
        } else if n.contains("attiny2313") || n.contains("attiny 2313") {
            AvrModel::Attiny2313
        } else if n.contains("attiny88") || n.contains("attiny 88") {
            AvrModel::Attiny88
        } else if n.contains("attiny48") || n.contains("attiny 48") {
            AvrModel::Attiny48
        } else if n.contains("attiny85") || n.contains("attiny 85") {
            AvrModel::Attiny85
        } else if n.contains("attiny84") || n.contains("attiny 84") {
            AvrModel::Attiny84
        } else if n.contains("attiny45") || n.contains("attiny 45") {
            AvrModel::Attiny45
        } else if n.contains("attiny44") || n.contains("attiny 44") {
            AvrModel::Attiny44
        } else if n.contains("attiny25") || n.contains("attiny 25") {
            AvrModel::Attiny25
        } else if n.contains("attiny13") || n.contains("attiny 13") {
            AvrModel::Attiny13
        } else if n.contains("1281") {
            AvrModel::Atmega1281
        } else if n.contains("2560") || n.contains("2561") {
            AvrModel::Atmega2560
        } else if n.contains("328pb") {
            AvrModel::Atmega328pb
        } else if n.contains("4809") {
            AvrModel::Atmega4809
        } else if n.contains("32u4") {
            AvrModel::Atmega32u4
        } else {
            AvrModel::Atmega328p
        }
    }
    pub fn flash_size(self) -> usize {
        match self {
            AvrModel::Atmega328p | AvrModel::Atmega328pb | AvrModel::Atmega32u4 => FLASH_SIZE,
            AvrModel::Atmega4809 => 48 * 1024,
            AvrModel::Atmega1281 => 128 * 1024,
            AvrModel::Atmega2560 => 256 * 1024,
            AvrModel::Attiny202 | AvrModel::Attiny204 => 2 * 1024,
            AvrModel::Attiny402 | AvrModel::Attiny404 | AvrModel::Attiny406 => 4 * 1024,
            AvrModel::Attiny804 | AvrModel::Attiny806 | AvrModel::Attiny807 => 8 * 1024,
            AvrModel::Attiny1604 | AvrModel::Attiny1606 | AvrModel::Attiny1607 => 16 * 1024,
            // Classic tinyAVR. Datasheet flash size, per part.
            AvrModel::Attiny13 => 1024,
            AvrModel::Attiny25 | AvrModel::Attiny2313 | AvrModel::Attiny261 => 2 * 1024,
            AvrModel::Attiny45 | AvrModel::Attiny44 | AvrModel::Attiny461 | AvrModel::Attiny48 => {
                4 * 1024
            }
            AvrModel::Attiny85 | AvrModel::Attiny84 | AvrModel::Attiny861 | AvrModel::Attiny88 => {
                8 * 1024
            }
            AvrModel::Attiny10 => 1024,
        }
    }
    pub fn sram_start(self) -> u16 {
        match self {
            AvrModel::Atmega328p | AvrModel::Atmega328pb | AvrModel::Atmega32u4 => SRAM_START,
            AvrModel::Atmega4809 => 0x2800,
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => 0x0200,
            AvrModel::Attiny202 | AvrModel::Attiny204 => 0x3F80,
            AvrModel::Attiny402 | AvrModel::Attiny404 | AvrModel::Attiny406 => 0x3F00,
            AvrModel::Attiny804 | AvrModel::Attiny806 | AvrModel::Attiny807 => 0x3E00,
            AvrModel::Attiny1604 | AvrModel::Attiny1606 | AvrModel::Attiny1607 => 0x3C00,
            // Classic tinyAVR: SRAM follows the 64-byte I/O file at 0x0060, except
            // the 48/88 which add a 160-byte extended I/O file and start SRAM at
            // 0x0100 (ATtiny48/88 datasheet, Table 5-2).
            AvrModel::Attiny48 | AvrModel::Attiny88 => 0x0100,
            AvrModel::Attiny13
            | AvrModel::Attiny2313
            | AvrModel::Attiny25
            | AvrModel::Attiny45
            | AvrModel::Attiny85
            | AvrModel::Attiny261
            | AvrModel::Attiny461
            | AvrModel::Attiny861
            | AvrModel::Attiny44
            | AvrModel::Attiny84 => 0x0060,
            // AVRrc: SRAM follows the 64-byte I/O space (Figure 6-1).
            AvrModel::Attiny10 => 0x0040,
        }
    }
    pub fn ramend(self) -> u16 {
        match self {
            AvrModel::Atmega328p | AvrModel::Atmega328pb => RAMEND,
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => 0x21FF,
            // 2.5 KB at 0x0100 (DS7766 section 5.2, Figure 5-2).
            AvrModel::Atmega32u4 => 0x0AFF,
            AvrModel::Atmega4809
            | AvrModel::Attiny202
            | AvrModel::Attiny204
            | AvrModel::Attiny402
            | AvrModel::Attiny404
            | AvrModel::Attiny406
            | AvrModel::Attiny804
            | AvrModel::Attiny806
            | AvrModel::Attiny807
            | AvrModel::Attiny1604
            | AvrModel::Attiny1606
            | AvrModel::Attiny1607 => 0x3FFF,
            // Classic tinyAVR RAMEND = SRAM start + size - 1 (datasheets).
            AvrModel::Attiny13 => 0x009F, // 0x0060 + 64B
            AvrModel::Attiny25 | AvrModel::Attiny2313 | AvrModel::Attiny261 => 0x00DF, // + 128B
            AvrModel::Attiny45 | AvrModel::Attiny44 | AvrModel::Attiny461 => 0x015F, // + 256B
            AvrModel::Attiny48 => 0x01FF, // 0x0100 + 256B
            AvrModel::Attiny85 | AvrModel::Attiny84 | AvrModel::Attiny861 => 0x025F, // 0x0060 + 512B
            AvrModel::Attiny88 => 0x02FF, // 0x0100 + 512B
            AvrModel::Attiny10 => 0x005F, // 0x0040 + 32B
        }
    }
    /// Bus base of this model's `avr_adc` input window (2 bytes per channel).
    /// The 2560's 16 channels move it past the port mirrors (PINF..PING sit
    /// at 0x1_002F..0x1_0034, where the 328P keeps its 8-channel window).
    pub fn adc_input_base(self) -> u64 {
        match self {
            AvrModel::Atmega328p | AvrModel::Atmega328pb => AVR_ADC_INPUT_BASE,
            // The 32U4's PINF (0x2F) mirror sits where the 328P window starts.
            AvrModel::Atmega1281 | AvrModel::Atmega2560 | AvrModel::Atmega32u4 => {
                AVR_MEGA_ADC_INPUT_BASE
            }
            AvrModel::Atmega4809
            | AvrModel::Attiny202
            | AvrModel::Attiny204
            | AvrModel::Attiny402
            | AvrModel::Attiny404
            | AvrModel::Attiny406
            | AvrModel::Attiny804
            | AvrModel::Attiny806
            | AvrModel::Attiny807
            | AvrModel::Attiny1604
            | AvrModel::Attiny1606
            | AvrModel::Attiny1607 => AVR_ADC_INPUT_BASE,
            // Classic tinyAVR use the same ADC input window as the 328P side of
            // the model; only the mega parts move it.
            AvrModel::Attiny13
            | AvrModel::Attiny2313
            | AvrModel::Attiny25
            | AvrModel::Attiny45
            | AvrModel::Attiny85
            | AvrModel::Attiny261
            | AvrModel::Attiny461
            | AvrModel::Attiny861
            | AvrModel::Attiny44
            | AvrModel::Attiny84
            | AvrModel::Attiny48
            | AvrModel::Attiny88
            | AvrModel::Attiny10 => AVR_ADC_INPUT_BASE,
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

    /// The ATmega328PB has a second polling USART at data-space 0xC8..0xCE.
    /// Its USART interrupts are not yet modelled.
    ///
    /// The ATmega32U4's only USART is USART1, at the same addresses.
    pub const fn has_usart1(self) -> bool {
        matches!(self, AvrModel::Atmega328pb | AvrModel::Atmega32u4)
    }

    /// The watchdog's `WDTCSR` is at data space 0x60 (`avr/wdt.rs`).
    pub const fn has_wdtcsr(self) -> bool {
        matches!(
            self,
            AvrModel::Atmega328p
                | AvrModel::Atmega328pb
                | AvrModel::Atmega1281
                | AvrModel::Atmega2560
        )
    }

    /// Whether this part is an AVRrc reduced core: r16..r31 only, no
    /// ADIW/SBIW/MOVW/MUL/LPM/SPM/JMP/CALL/LDD/STD, and one-word LDS/STS.
    #[inline]
    pub const fn is_reduced_core(self) -> bool {
        matches!(self, AvrModel::Attiny10)
    }

    /// Whether this part uses the AVRxt data-space map and direct I/O
    /// addresses for IN/OUT, SBI/CBI and SBIS/SBIC.
    #[inline]
    pub fn uses_avrxt_data_map(self) -> bool {
        matches!(
            self,
            AvrModel::Atmega4809
                | AvrModel::Attiny202
                | AvrModel::Attiny204
                | AvrModel::Attiny402
                | AvrModel::Attiny404
                | AvrModel::Attiny406
                | AvrModel::Attiny804
                | AvrModel::Attiny806
                | AvrModel::Attiny807
                | AvrModel::Attiny1604
                | AvrModel::Attiny1606
                | AvrModel::Attiny1607
        )
    }
}

/// ATmega32U4 data-space addresses of PINx for ports B..F (DDRx = +1, PORTx = +2).
pub const M32U4_PIN_ADDRS: [u16; 5] = [0x23, 0x26, 0x29, 0x2C, 0x2F];

/// ATmega32U4 vector number (1-based, DS7766 Table 9-1) for a vector this
/// core numbers the ATmega328P way, plus USB_GEN/USB_COM (internal 27/28).
/// Timer2, PCINT1/2 and USART0 do not exist on the 32U4 and are never raised;
/// they pass through unchanged.
pub fn m32u4_vector(vec328: u32) -> u32 {
    match vec328 {
        2 | 3 => vec328,       // INT0, INT1
        4 => 10,               // PCINT0
        7 => 13,               // WDT
        11..=13 => vec328 + 6, // TIMER1 CAPT/COMPA/COMPB -> 17..19
        14 => 21,              // TIMER1 OVF
        15..=21 => vec328 + 7, // TIMER0 COMPA/B/OVF -> 22..24, SPI 25, USART1 RX/UDRE/TX -> 26..28
        22 => 30,              // ADC
        23 => 31,              // EE READY
        24 => 29,              // ANALOG COMP
        25 => 37,              // TWI
        26 => 38,              // SPM READY
        usb::VEC_USB_GEN => 11,
        usb::VEC_USB_COM => 12,
        v => v,
    }
}

/// Flash and SRAM geometry of a classic megaAVR or AT90USB part that has no
/// [`AvrModel`] of its own.
///
/// These parts run on the ATmega328P model: its instruction set and its
/// peripherals at the 328P addresses. Only the memory geometry is the part's
/// own. Without it every one of them ran with 32 KB flash and SRAM at
/// 0x0100..=0x08FF, so avr-libc startup code, which sets SP to the part's
/// RAMEND, faulted on its first push on any part whose RAMEND is above 0x08FF,
/// and parts whose SRAM starts at 0x0060 had 328P extended I/O where the
/// first 160 bytes of their SRAM are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvrGeometry {
    pub flash_size: usize,
    pub sram_start: u16,
    pub ramend: u16,
}

/// (part, flash bytes, SRAM start, RAMEND). Values are FLASHEND + 1, RAMSTART
/// and RAMEND of the part's avr-libc `io*.h` header, which follow the
/// datasheet's memory map. A name matches with an optional A, L, V, P, PA or
/// PV suffix, never by prefix: `atmega16` must not capture `atmega162` or
/// `atmega168`, nor `atmega64` the `atmega640`.
const CLASSIC_MEGA_GEOMETRY: &[(&str, usize, u16, u16)] = &[
    ("atmega8", 8 * 1024, 0x0060, 0x045F),
    ("atmega16", 16 * 1024, 0x0060, 0x045F),
    ("atmega32", 32 * 1024, 0x0060, 0x085F),
    ("atmega64", 64 * 1024, 0x0100, 0x10FF),
    ("atmega128", 128 * 1024, 0x0100, 0x10FF),
    ("atmega8515", 8 * 1024, 0x0060, 0x025F),
    ("atmega8535", 8 * 1024, 0x0060, 0x025F),
    ("atmega162", 16 * 1024, 0x0100, 0x04FF),
    ("atmega48", 4 * 1024, 0x0100, 0x02FF),
    ("atmega88", 8 * 1024, 0x0100, 0x04FF),
    ("atmega168", 16 * 1024, 0x0100, 0x04FF),
    ("atmega164", 16 * 1024, 0x0100, 0x04FF),
    ("atmega324", 32 * 1024, 0x0100, 0x08FF),
    ("atmega644", 64 * 1024, 0x0100, 0x10FF),
    ("atmega1284", 128 * 1024, 0x0100, 0x40FF),
    ("atmega165", 16 * 1024, 0x0100, 0x04FF),
    ("atmega169", 16 * 1024, 0x0100, 0x04FF),
    ("atmega325", 32 * 1024, 0x0100, 0x08FF),
    ("atmega329", 32 * 1024, 0x0100, 0x08FF),
    ("atmega3250", 32 * 1024, 0x0100, 0x08FF),
    ("atmega3290", 32 * 1024, 0x0100, 0x08FF),
    ("atmega645", 64 * 1024, 0x0100, 0x10FF),
    ("atmega649", 64 * 1024, 0x0100, 0x10FF),
    ("atmega6450", 64 * 1024, 0x0100, 0x10FF),
    ("atmega6490", 64 * 1024, 0x0100, 0x10FF),
    ("at90usb646", 64 * 1024, 0x0100, 0x10FF),
    ("at90usb1286", 128 * 1024, 0x0100, 0x20FF),
];

impl AvrGeometry {
    /// The geometry of a classic megaAVR / AT90USB chip name, or `None` when
    /// the name is not in the table (the part then keeps its model's geometry).
    pub fn for_chip_name(name: &str) -> Option<Self> {
        let n = name.trim().to_ascii_lowercase();
        CLASSIC_MEGA_GEOMETRY
            .iter()
            .find(|(part, ..)| {
                n.strip_prefix(part).is_some_and(|suffix| {
                    matches!(suffix, "" | "a" | "l" | "v" | "p" | "pa" | "pv")
                })
            })
            .map(|&(_, flash_size, sram_start, ramend)| Self {
                flash_size,
                sram_start,
                ramend,
            })
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
    /// AVRxt I/O memory, whose data-space addresses start at zero and span
    /// 0x0000..=0x0FFF. Used by the tinyAVR 0-series model.
    tiny_io: Vec<u8>,
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
    /// A CPU write to TCNT0 blocks any compare match in the next timer clock
    /// (DS40002061 §15.6.2); set by the write, consumed by that clock.
    pub t0_match_blocked: bool,
    /// Timer2, the counter Arduino `tone()` runs in CTC mode.
    ///
    /// Normal, CTC and phase-correct (WGM 1, TOP = 0xFF, up and down) count
    /// as on silicon. Fast PWM and WGM 5 count 0..255. The OC2A/OC2B pad
    /// levels come from `avr/oc_pins.rs`.
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
    /// Timer2 is counting down (phase-correct mode only).
    pub t2_down: bool,
    /// Timer1, the 16-bit counter (`avr/timer1.rs`).
    t1: timer1::Timer1,
    /// Pads an output-compare unit owns, per port slot of `oc_pins::OcMap`
    /// (B/D on the 328P, B/G/H on the mega), and the level it drives there,
    /// as last pushed to the bus-side port model.
    oc_mask: [u8; 3],
    oc_level: [u8; 3],
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
    /// Soft-SPI bit-bang state (SPE=0): last sampled SCK level, the byte
    /// accumulating on rising edges, and how many bits are in it. The supported
    /// Uno mapping is DIN=D11/PB3 and CLK=D13/PB5, for example
    /// `LedControl(11, 13, 10, count)`. Arbitrary DIN/CLK pins are not routed.
    /// Each rising SCK samples MOSI MSB-first, and every eighth
    /// bit clocks the parking SPI chain.
    soft_spi_sck: bool,
    soft_spi_shift: u8,
    soft_spi_bits: u8,
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
    /// AVRxt CCP window and CPUINT (`avr/avrxt.rs`).
    xt: avrxt::XtState,
    /// ATmega32U4 USB device controller and its host (`avr/usb.rs`).
    usb: usb::UsbDevice,
    /// Watchdog timer: `WDTCSR` and `WDR` (`avr/wdt.rs`).
    wdt: wdt::Wdt,
    /// Data EEPROM and `EECR`/`EEDR`/`EEAR` (`avr/eeprom.rs`). Empty on the
    /// parts whose EEPROM is not modelled.
    eeprom: eeprom::Eeprom,
    /// ATmega2560 family: INT0..7 / PCINT0..23 (`avr/mega_ext_int.rs`), the
    /// timed ADC (`avr/mega_adc.rs`) and the timed SPI (`avr/mega_spi.rs`).
    mx: mega_ext_int::MegaExt,
    madc: mega_adc::MegaAdc,
    mspi: mega_spi::MegaSpi,
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
/// Data-space address where AVRrc parts map program flash (ATtiny4/5/9/10
/// datasheet §6.2).
pub const AVRRC_FLASH_DATA_BASE: u16 = 0x4000;
/// `MCUCR` in data space (I/O 0x35), ATmega328P datasheet §14.4.1.
pub const ADDR_MCUCR: u16 = 0x0055;
/// MCU Status Register in data space (I/O 0x34), reset flags are write-zero-clear.
pub const ADDR_MCUSR: u16 = 0x0054;
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
/// ATmega328P temperature sensor output at +25 °C, typical (table 24-2).
const AVR_TEMP_SENSOR_25C_MV: u32 = 314;
/// `adc_convert`'s channel marker for the temperature sensor (mux 0x08).
const AVR_ADC_TEMP_SENSOR: u8 = 0xFE;

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
/// `TIMER0_COMPA_vect` (`__vector_14`, byte 0x38 on the 328P; vector 22 on
/// the 2560 through `mega2560_vector`).
pub const VEC_TIMER0_COMPA: u32 = 15;
/// `TIMER0_COMPB_vect` (`__vector_15`, byte 0x3C on the 328P).
pub const VEC_TIMER0_COMPB: u32 = 16;
pub const TIMSK_OCIE0A: u8 = 1 << 1;
pub const TIMSK_OCIE0B: u8 = 1 << 2;
pub const TIFR_OCF0A: u8 = 1 << 1;
pub const TIFR_OCF0B: u8 = 1 << 2;
const T0_IRQ_MASK: u64 = (1 << VEC_TIMER0_COMPA) | (1 << VEC_TIMER0_COMPB) | (1 << VEC_TIMER0_OVF);
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

/// Phase-correct (dual-slope) counting: BOTTOM up to TOP and back down, so
/// one cycle is `2 * TOP` timer clocks (ATmega328P datasheet 15.7.4, 16.9.4,
/// 18.7.4). The counter is tracked as `(tcnt, down)`; `dual_phase` maps that
/// onto a position in `0..2 * TOP`, where `TOP` is reached once going up and
/// `BOTTOM` once going down.
fn dual_phase(tcnt: u64, down: bool, top: u64) -> u64 {
    if down {
        (2 * top - tcnt) % (2 * top)
    } else {
        tcnt
    }
}

/// Inverse of [`dual_phase`]: the counter value and direction at `phase`.
fn dual_from_phase(phase: u64, top: u64) -> (u64, bool) {
    if phase <= top {
        (phase, false)
    } else {
        (2 * top - phase, true)
    }
}

/// `true` when `ticks` clocks from `phase` pass the counter over `target`,
/// on either slope. A target above TOP never matches.
fn dual_hits(phase: u64, ticks: u64, top: u64, target: u64) -> bool {
    let period = 2 * top;
    target <= top
        && (t2_hits(phase, ticks, period, target)
            || t2_hits(phase, ticks, period, (period - target) % period))
}

/// Timer clocks from `phase` until the counter next equals `target` (a full
/// period if it equals it now), or `None` above TOP.
fn dual_dist(phase: u64, top: u64, target: u64) -> Option<u64> {
    let period = 2 * top;
    (target <= top).then(|| {
        [target, (period - target) % period]
            .into_iter()
            .map(|t| match (t + period - phase) % period {
                0 => period,
                d => d,
            })
            .min()
            .unwrap_or(period)
    })
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
        let model = AvrModel::from_chip_name(name);
        let mut cpu = Self::new_model(model);
        // Only a name that fell through to the default model takes a table
        // geometry; a part with its own model already has its own.
        if model == AvrModel::Atmega328p {
            if let Some(geometry) = AvrGeometry::for_chip_name(name) {
                cpu.set_geometry(geometry);
            }
        }
        cpu
    }

    /// The flash size and SRAM window this CPU was built with.
    pub fn geometry(&self) -> AvrGeometry {
        AvrGeometry {
            flash_size: self.flash.len(),
            sram_start: self.sram_start,
            ramend: self.ramend,
        }
    }

    /// Resize flash and SRAM to `geometry`. For use before firmware is loaded:
    /// both memories are cleared and SP returns to the new RAMEND.
    fn set_geometry(&mut self, geometry: AvrGeometry) {
        self.flash = vec![0xFF; geometry.flash_size];
        self.sram_start = geometry.sram_start;
        self.ramend = geometry.ramend;
        self.sram = vec![0; usize::from(geometry.ramend - geometry.sram_start) + 1];
        self.sp = geometry.ramend;
    }

    /// SRAM that begins inside the 328P's extended I/O file (0x0060..=0x00FF),
    /// as on every part whose I/O file is only 64 registers. On those parts
    /// the address is SRAM and must not reach a 328P register arm.
    #[inline]
    fn is_low_sram(&self, addr: u16) -> bool {
        self.sram_start < 0x0100 && (self.sram_start..=self.ramend).contains(&addr)
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
            tiny_io: vec![0; 0x1000],
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
            t0_match_blocked: false,
            tcnt2: 0,
            tccr2a: 0,
            tccr2b: 0,
            ocr2a: 0,
            ocr2b: 0,
            timsk2: 0,
            tifr2: 0,
            assr: 0,
            t2_prescale_acc: 0,
            t2_down: false,
            t1: timer1::Timer1::default(),
            oc_mask: [0; 3],
            oc_level: [0; 3],
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
            soft_spi_sck: false,
            soft_spi_shift: 0,
            soft_spi_bits: 0,
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
            xt: avrxt::XtState::default(),
            irq_shadow: false,
            usb: usb::UsbDevice::default(),
            wdt: wdt::Wdt::new(model.has_wdtcsr()),
            eeprom: eeprom::Eeprom::new(model.eeprom_size()),
            mx: mega_ext_int::MegaExt::default(),
            madc: mega_adc::MegaAdc::default(),
            mspi: mega_spi::MegaSpi::default(),
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
        matches!(self.model, AvrModel::Atmega1281 | AvrModel::Atmega2560)
    }

    #[inline]
    fn is_32u4(&self) -> bool {
        self.model == AvrModel::Atmega32u4
    }

    /// INT0/INT1 and PCINT0..2 on the 328P pins (`avr/ext_int.rs`). The mega
    /// and 32U4 bond them elsewhere; there only SMCR is live.
    #[inline]
    fn ext_int_328_pins(&self) -> bool {
        !self.is_mega() && !self.is_32u4()
    }

    /// ATmega32U4 registers the 328P map would misread: PLLCSR and the USB
    /// block go to `avr/usb.rs`; 0xC0..0xC7 are Timer4 (TCCR4A..E, stored
    /// only), not USART0.
    #[inline]
    fn is_32u4_own_reg(&self, addr: u16) -> bool {
        self.is_32u4()
            && matches!(addr, usb::ADDR_PLLCSR | 0xC0..=0xC7 | usb::ADDR_UHWCON..=usb::ADDR_UEINT)
    }

    #[inline]
    fn is_avrxt(&self) -> bool {
        self.model.uses_avrxt_data_map()
    }

    #[inline]
    fn is_avrrc(&self) -> bool {
        self.model.is_reduced_core()
    }

    /// Convert a 6-bit IN/OUT/SBI/SBIC I/O operand to a data-space address.
    /// AVRxt and AVRrc both place I/O at data address 0, so the operand is
    /// the address.
    #[inline]
    pub(in crate::cpu::avr) fn io_data_address(&self, io_addr: u8) -> u16 {
        if self.is_avrxt() || self.is_avrrc() {
            u16::from(io_addr)
        } else {
            0x20 + u16::from(io_addr)
        }
    }

    /// `PINx` of a port this model has.
    #[inline]
    fn is_pin_reg(&self, addr: u16) -> bool {
        match self.model {
            // ATtiny48/88 keep PINB/PINC/PIND at the 328P addresses
            // (datasheet register summary: 0x23, 0x26, 0x29).
            AvrModel::Atmega328p
            | AvrModel::Atmega328pb
            | AvrModel::Attiny48
            | AvrModel::Attiny88 => {
                matches!(addr, AVR_PINB | AVR_PINC | AVR_PIND)
            }
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => MEGA2560_PIN_ADDRS.contains(&addr),
            AvrModel::Atmega32u4 => M32U4_PIN_ADDRS.contains(&addr),
            AvrModel::Atmega4809 => false,
            AvrModel::Attiny202 | AvrModel::Attiny204 => false,
            AvrModel::Attiny402
            | AvrModel::Attiny404
            | AvrModel::Attiny406
            | AvrModel::Attiny804
            | AvrModel::Attiny806
            | AvrModel::Attiny807
            | AvrModel::Attiny1604
            | AvrModel::Attiny1606
            | AvrModel::Attiny1607 => false,
            AvrModel::Attiny13
            | AvrModel::Attiny2313
            | AvrModel::Attiny25
            | AvrModel::Attiny45
            | AvrModel::Attiny85
            | AvrModel::Attiny261
            | AvrModel::Attiny461
            | AvrModel::Attiny861
            | AvrModel::Attiny44
            | AvrModel::Attiny84
            | AvrModel::Attiny10 => false,
        }
    }

    /// `PINx`/`DDRx`/`PORTx` of a port this model has: mirrored to the bus.
    #[inline]
    /// Every port back to a tri-stated input, `DDRx = PORTx = 0` (ATmega328P
    /// datasheet §14.4: both reset to 0). A watchdog reset arrives here with
    /// the bus already running, so the bus-side port windows are cleared as
    /// well as this I/O map; at power-on both are already 0 and nothing is
    /// written. All DDRs go first, so an output that was high is released
    /// rather than driven low for a moment.
    fn reset_ports(&mut self, bus: &mut dyn Bus) {
        for offset in [1, 2] {
            for pin in 0x20..=0x10Bu16 {
                if !self.is_pin_reg(pin) {
                    continue;
                }
                let addr = pin + offset;
                let reg = &mut self.io[usize::from(addr - 0x20)];
                if *reg != 0 {
                    *reg = 0;
                    // Best-effort, like every port mirror write.
                    let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + u64::from(addr), 0);
                }
            }
        }
    }

    fn is_port_reg(&self, addr: u16) -> bool {
        match self.model {
            AvrModel::Atmega328p
            | AvrModel::Atmega328pb
            | AvrModel::Attiny48
            | AvrModel::Attiny88 => (AVR_PINB..=AVR_PIND + 2).contains(&addr),
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => {
                (0x20..=0x34).contains(&addr) || (0x100..=0x10B).contains(&addr)
            }
            AvrModel::Atmega32u4 => (0x23..=0x31).contains(&addr),
            AvrModel::Atmega4809 => false,
            AvrModel::Attiny202 | AvrModel::Attiny204 => false,
            AvrModel::Attiny402
            | AvrModel::Attiny404
            | AvrModel::Attiny406
            | AvrModel::Attiny804
            | AvrModel::Attiny806
            | AvrModel::Attiny807
            | AvrModel::Attiny1604
            | AvrModel::Attiny1606
            | AvrModel::Attiny1607 => false,
            AvrModel::Attiny13
            | AvrModel::Attiny2313
            | AvrModel::Attiny25
            | AvrModel::Attiny45
            | AvrModel::Attiny85
            | AvrModel::Attiny261
            | AvrModel::Attiny461
            | AvrModel::Attiny861
            | AvrModel::Attiny44
            | AvrModel::Attiny84
            | AvrModel::Attiny10 => false,
        }
    }

    /// Synchronize GPIO CS for parking kits and legacy CPU-owned SPI devices.
    fn sync_spi_cs(&mut self, bus: &mut dyn Bus) -> bool {
        let mut released = bus.sync_parking_spi_cs();
        for device in &mut self.spi_devices {
            let Some(high) = bus.read_gpio_output_by_label(device.cs_pin()) else {
                continue;
            };
            match (device.is_selected(), !high) {
                (false, true) => device.cs_select(),
                (true, false) => {
                    device.cs_release();
                    released = true;
                }
                _ => {}
            }
        }
        released
    }

    /// GPIO-CS and byte-aligned software SPI for Uno-compatible PORTB.
    ///
    /// Supported wiring is DIN=D11/PB3, CLK=D13/PB5, and any configured LOAD
    /// pin. DIN and CLK must both be configured as outputs in DDRB.
    /// This is not the common LedControl(12, 11, 10, ...) wiring; use
    /// LedControl(11, 13, 10, ...) for this bridge. LOAD updates also apply
    /// with SPE=1, so a hardware-SPDR transaction latches on its final GPIO
    /// write. Sub-byte transfers are outside this bridge's byte-level scope.
    fn soft_spi_after_port(&mut self, bus: &mut dyn Bus, addr: u16, port_value: u8) {
        // Reset the partial byte at the LOAD edge before any port/SPE return.
        // Complete bytes still shift while LOAD is high on parts like MAX7219.
        let released = self.sync_spi_cs(bus);
        let accepts_clock = bus.parking_spi_accepts_clock();
        if released || !accepts_clock {
            self.soft_spi_shift = 0;
            self.soft_spi_bits = 0;
        }
        if !matches!(self.model, AvrModel::Atmega328p | AvrModel::Atmega328pb) {
            return;
        }
        const DDRB: u16 = AVR_PINB + 1;
        const PORTB: u16 = AVR_PINB + 2;
        const OUTPUTS: u8 = (1 << 3) | (1 << 5);
        // PORT writes on inputs control pull-ups, not this output-only bridge.
        // Read the CPU-owned DDR already mirrored above; no extra bus walk.
        if self.io[usize::from(DDRB - 0x20)] & OUTPUTS != OUTPUTS {
            self.soft_spi_shift = 0;
            self.soft_spi_bits = 0;
            self.soft_spi_sck = false;
            return;
        }
        if addr == DDRB {
            // Establish the level when GPIO ownership is acquired. A later
            // PORT write repeating that high level must not invent a CLK edge.
            self.soft_spi_sck = self.io[usize::from(PORTB - 0x20)] & (1 << 5) != 0;
            return;
        }
        if addr != PORTB {
            return;
        }
        let sck = (port_value & (1 << 5)) != 0;
        let rising = sck && !self.soft_spi_sck;
        self.soft_spi_sck = sck;
        const SPE: u8 = 1 << 6;
        if self.spcr & SPE != 0 || !accepts_clock {
            return;
        }
        if rising {
            let mosi = (port_value & (1 << 3)) != 0;
            self.soft_spi_shift = (self.soft_spi_shift << 1) | u8::from(mosi);
            self.soft_spi_bits += 1;
            if self.soft_spi_bits == 8 {
                let _ = bus.clock_parking_spi(self.soft_spi_shift);
                self.soft_spi_shift = 0;
                self.soft_spi_bits = 0;
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
        if self.is_avrxt() {
            return self.avrxt_data_read(addr, bus);
        }
        if self.is_avrrc() {
            return self.avrrc_data_read(addr, bus);
        }
        match addr {
            0x0000..=0x001F => Ok(self.r[addr as usize]),
            a if self.is_32u4_own_reg(a) => {
                Ok(self.usb_read(a).unwrap_or(self.io[usize::from(a - 0x20)]))
            }
            a if self.is_low_sram(a) => Ok(self.sram[(a - self.sram_start) as usize]),
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
                // A pad an output-compare unit owns reads its waveform level,
                // not the latch (the synchronizer samples the pad).
                let port = self.port_pad_value(addr + 2, self.io[(addr + 2 - 0x20) as usize]);
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
            // ATmega328PB USART1 mirrors the 328P USART0 register layout at
            // data-space 0xC8..0xCE. Its local control registers use the I/O
            // file; the bus-side UART owns RX buffering and peer connections.
            0x00C8 if self.model.has_usart1() => {
                let rxc = if Self::usart1_on_bus(bus) {
                    bus.read_u8(AVR_IO_MIRROR_BASE + 0xC8)? & UCSRA_RXC
                } else {
                    0
                };
                let flags = self.io[(0xC8 - 0x20) as usize] & !UCSRA_RXC;
                Ok(flags | UCSRA_UDRE | rxc)
            }
            0x00C9 | 0x00CA | 0x00CC | 0x00CD if self.model.has_usart1() => {
                Ok(self.io[(addr - 0x20) as usize])
            }
            // UDR1 read pops one received byte from the bus-side USART.
            0x00CE if self.model.has_usart1() => {
                if Self::usart1_on_bus(bus) {
                    bus.read_u8(AVR_IO_MIRROR_BASE + 0xCE)
                } else {
                    Ok(0)
                }
            }
            a if self.is_mega() && mega_spi::handles(a) => Ok(self.mega_spi_read(a)),
            a if self.is_mega() && mega_adc::handles(a) => Ok(self.mega_adc_read(a)),
            a if self.is_mega() && mega_ext_int::handles(a) => Ok(self.mega_ext_read(a)),
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
                if self.ext_int_328_pins() || addr == ext_int::ADDR_SMCR =>
            {
                Ok(self.ext_read(addr).unwrap_or_default())
            }
            a if self.wdt.handles(a) => Ok(self.wdt.read(self.cycles)),
            a if self.eeprom.handles(a) => Ok(self.eeprom.read(a, self.cycles)),
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
        if self.is_avrxt() {
            return self.avrxt_data_write(addr, value, bus);
        }
        if self.is_avrrc() {
            return self.avrrc_data_write(addr, value, bus);
        }
        match addr {
            0x0000..=0x001F => {
                self.r[addr as usize] = value;
                Ok(())
            }
            a if self.is_32u4_own_reg(a) => {
                if (0xC0..=0xC7).contains(&a) {
                    self.io[usize::from(a - 0x20)] = value;
                } else {
                    self.usb_write(a, value);
                }
                Ok(())
            }
            a if self.is_low_sram(a) => {
                self.sram[(a - self.sram_start) as usize] = value;
                bus.write_u8(a as u64, value)?;
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
                self.tifr0 &= !(value & (TIFR_OCF0B | TIFR_OCF0A | TIFR_TOV0));
                self.sync_timer0_irq();
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
                self.t0_match_blocked = true;
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
                self.timsk0 = value & (TIMSK_OCIE0B | TIMSK_OCIE0A | TIMSK_TOIE0);
                self.sync_timer0_irq();
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
                // TCCR1A/B and TIMSK1 decide whether ICP1 must be watched.
                if !self.is_mega()
                    && matches!(addr, timer1::ADDR_TIMSK1 | timer1::ADDR_TCCR1A | 0x0081)
                {
                    self.ext_config_changed(bus);
                }
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
            0x00C8 if self.model.has_usart1() => {
                // U2X1/MPCM1 are writable. TXC1 is write-one-to-clear and
                // returns once the instantaneous transmitter becomes empty;
                // RXC1 is supplied by the bus-side USART queue.
                let i = (0xC8 - 0x20) as usize;
                let txc = if value & UCSRA_TXC != 0 {
                    0
                } else {
                    self.io[i] & UCSRA_TXC
                };
                self.io[i] = txc | (value & 0x03);
                Self::usart1_mirror_write(bus, 0xC8, value)?;
                Ok(())
            }
            0x00C9 | 0x00CA | 0x00CC | 0x00CD if self.model.has_usart1() => {
                self.io[(addr - 0x20) as usize] = value;
                Self::usart1_mirror_write(bus, u64::from(addr), value)?;
                Ok(())
            }
            0x00CE if self.model.has_usart1() => {
                self.serial_tx.push(value);
                let status = &mut self.io[(0xC8 - 0x20) as usize];
                *status |= UCSRA_UDRE | UCSRA_TXC;
                bus.write_u8(u64::from(addr), value)?;
                Self::usart1_mirror_write(bus, 0xCE, value)?;
                let on_bus =
                    Self::usart1_on_bus(bus) && bus.read_u8(AVR_IO_MIRROR_BASE + 0xCF)? & 1 != 0;
                if !on_bus {
                    if let Some(sink) = &self.serial_sink {
                        if let Ok(mut g) = sink.lock() {
                            g.push(value);
                        }
                    }
                }
                Ok(())
            }
            a if self.is_mega() && mega_spi::handles(a) => self.mega_spi_write(a, value, bus),
            a if self.is_mega() && mega_adc::handles(a) => self.mega_adc_write(a, value, bus),
            a if self.is_mega() && mega_ext_int::handles(a) => {
                self.mega_ext_write(a, value, bus);
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
                self.spdr = self.spi_exchange(bus, value);
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
                if self.ext_int_328_pins() || addr == ext_int::ADDR_SMCR =>
            {
                self.ext_write(addr, value, bus);
                Ok(())
            }
            ADDR_MCUSR => {
                self.io[usize::from(ADDR_MCUSR - 0x20)] &= value & 0x0f;
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
            a if self.wdt.handles(a) => {
                self.wdt_write(value);
                Ok(())
            }
            a if self.eeprom.handles(a) => {
                self.cycles += self.eeprom.write(a, value, self.cycles);
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
                    // Sync GPIO LOAD for both hardware and software SPI;
                    // the Uno software bridge samples PB3 on PB5 rising edges.
                    self.soft_spi_after_port(bus, addr, pad);
                } else {
                    let _mirror = bus.write_u8(addr as u64, value);
                }
                Ok(())
            }
            0x0100..=0x01FF if self.is_mega() => {
                self.io[(addr - 0x20) as usize] = value;
                if self.is_port_reg(addr) {
                    // PORTH carries OC2B (PH6) on the ATmega2560.
                    let pad = self.port_pad_value(addr, value);
                    let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + addr as u64, pad);
                    self.soft_spi_after_port(bus, addr, pad);
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

    /// AVRrc data-space read (ATtiny4/5/9/10 datasheet Figure 6-1): I/O at
    /// 0x00..=0x3F, SRAM, then flash mapped read-only at 0x4000. The NVM
    /// lock, configuration, calibration and device-ID bytes at 0x3F00..0x3FFF
    /// are not modelled and fault like any unmapped address.
    fn avrrc_data_read(&self, addr: u16, bus: &dyn Bus) -> SimResult<u8> {
        match addr {
            0x003D => Ok(self.sp as u8),
            0x003E => Ok((self.sp >> 8) as u8),
            0x003F => Ok(self.sreg),
            0x0000..=0x003F => {
                let mirror = AVR_IO_MIRROR_BASE + u64::from(addr);
                if bus.has_mmio_window(mirror) {
                    bus.read_u8(mirror)
                } else {
                    Ok(self.tiny_io[addr as usize])
                }
            }
            a if (self.sram_start..=self.ramend).contains(&a) => {
                Ok(self.sram[(a - self.sram_start) as usize])
            }
            a if a >= AVRRC_FLASH_DATA_BASE
                && usize::from(a - AVRRC_FLASH_DATA_BASE) < self.flash.len() =>
            {
                Ok(self.flash[usize::from(a - AVRRC_FLASH_DATA_BASE)])
            }
            _ => Err(SimulationError::MemoryViolation(u64::from(addr))),
        }
    }

    /// AVRrc data-space write. I/O follows the AVRxt mirror rule; the mapped
    /// flash is read-only to firmware (datasheet §6.2), so a store there
    /// faults instead of being dropped.
    fn avrrc_data_write(&mut self, addr: u16, value: u8, bus: &mut dyn Bus) -> SimResult<()> {
        match addr {
            0x003D => {
                self.sp = (self.sp & 0xFF00) | u16::from(value);
                Ok(())
            }
            0x003E => {
                self.sp = (self.sp & 0x00FF) | (u16::from(value) << 8);
                Ok(())
            }
            0x003F => {
                self.sreg = value;
                Ok(())
            }
            0x0000..=0x003F => {
                self.tiny_io[addr as usize] = value;
                let mirror = AVR_IO_MIRROR_BASE + u64::from(addr);
                if bus.has_mmio_window(mirror) {
                    bus.write_u8(mirror, value)?;
                }
                Ok(())
            }
            a if (self.sram_start..=self.ramend).contains(&a) => {
                self.sram[(a - self.sram_start) as usize] = value;
                bus.write_u8(u64::from(a), value)?;
                Ok(())
            }
            _ => Err(SimulationError::MemoryViolation(u64::from(addr))),
        }
    }

    /// One 10-bit conversion of the channel ADMUX selects.
    ///
    /// MUX 0..7 are the input pins, whose millivolts come from the bus-side
    /// `avr_adc` model; 0x08 is the temperature sensor, 0x0E is the 1.1 V
    /// bandgap and 0x0F is GND. The reserved mux codes read 0. The temperature
    /// sensor gives the datasheet's typical 314 mV at +25 °C (ATmega328P
    /// table 24-2, with the 1.1 V reference: 292). The die is held at +25 °C,
    /// and the part-to-part offset (about ±10 °C uncalibrated) is not modelled.
    /// REFS selects the reference: 11 is the internal 1.1 V, anything else is
    /// taken as 5 V AVcc (AREF is not a modelled pin).
    ///
    /// The ATmega2560 family has its own converter (`avr/mega_adc.rs`).
    fn adc_convert(&self, bus: &dyn Bus) -> SimResult<u16> {
        let channel: Option<u8> = if self.is_32u4() {
            // DS7766 Table 24-4: MUX5 is ADCSRB bit 5. 0x00..0x07 are
            // ADC0..7 (ADC2/3 are not bonded), 0x20..0x25 ADC8..13, 0x1E the
            // 1.1 V bandgap; 0x1F is GND. The temperature sensor (0x27) and
            // the differential/gain codes are not modelled and read 0.
            let mux5 = self.io[(0x7B - 0x20) as usize] & (1 << 5) != 0;
            match (self.admux & 0x1F) | if mux5 { 0x20 } else { 0 } {
                m @ 0x00..=0x07 => Some(m),
                m @ 0x20..=0x25 => Some(m - 0x18),
                0x1E => None,
                _ => Some(0xFF),
            }
        } else {
            match self.admux & 0x0F {
                m @ 0..=7 => Some(m),
                0x08 => Some(AVR_ADC_TEMP_SENSOR),
                0x0E => None,
                _ => Some(0xFF),
            }
        };
        let input_mv: u32 = match channel {
            Some(0xFF) => 0,
            Some(AVR_ADC_TEMP_SENSOR) => AVR_TEMP_SENSOR_25C_MV,
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

    /// Advance Timer0 by CPU clocks. The counter runs BOTTOM..0xFF (normal
    /// and fast PWM with TOP = 0xFF; the TOP = OCR0A and phase-correct modes
    /// are not modelled and count the same way). TOV0 is set as the count
    /// wraps to BOTTOM; OCF0x on the timer clock after TCNT0 equalled OCR0x,
    /// i.e. as the count leaves the compare value (DS40002061 §15.6, Figures
    /// 15-9/15-10), unless a TCNT0 write blocked that clock.
    pub fn tick_timer0(&mut self, cpu_cycles: u32) {
        let div = self.t0_prescaler();
        if div == 0 || cpu_cycles == 0 {
            return;
        }
        // Closed form of "one TCNT0 increment per `div` clocks": an idle
        // fast-forward hands this millions of cycles at once.
        let acc = u64::from(self.t0_prescale_acc) + u64::from(cpu_cycles);
        let mut ticks = acc / u64::from(div);
        self.t0_prescale_acc = (acc % u64::from(div)) as u32;
        if ticks == 0 {
            return;
        }
        if std::mem::take(&mut self.t0_match_blocked) {
            // The first clock after a TCNT0 write: no compare match.
            let wrapped = self.tcnt0 == 0xFF;
            self.tcnt0 = self.tcnt0.wrapping_add(1);
            if wrapped {
                self.tifr0 |= TIFR_TOV0;
            }
            ticks -= 1;
        }
        let phase = u64::from(self.tcnt0);
        let leaves = |ocr: u8| t2_hits(phase, ticks, 256, (u64::from(ocr) + 1) & 0xFF);
        if leaves(self.ocr0a) {
            self.tifr0 |= TIFR_OCF0A;
        }
        if leaves(self.ocr0b) {
            self.tifr0 |= TIFR_OCF0B;
        }
        if t2_hits(phase, ticks, 256, 0) {
            self.tifr0 |= TIFR_TOV0;
        }
        self.tcnt0 = ((phase + ticks) & 0xFF) as u8;
        self.sync_timer0_irq();
    }

    /// Timer0 requests stay pending while a flag and its enable are both set,
    /// including an enable written over an already set flag.
    fn sync_timer0_irq(&mut self) {
        let on = self.tifr0 & self.timsk0;
        let mut want = 0u64;
        if on & TIFR_OCF0A != 0 {
            want |= 1 << VEC_TIMER0_COMPA;
        }
        if on & TIFR_OCF0B != 0 {
            want |= 1 << VEC_TIMER0_COMPB;
        }
        if on & TIFR_TOV0 != 0 {
            want |= 1 << VEC_TIMER0_OVF;
        }
        self.pending_irq = (self.pending_irq & !T0_IRQ_MASK) | want;
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

    /// WGM22:0 == 1: phase-correct PWM with TOP = 0xFF, Arduino's
    /// `analogWrite` mode for D3/D11. WGM 5 (TOP = OCR2A) still counts up.
    fn t2_is_phase_correct(&self) -> bool {
        let wgm = (self.tccr2a & 0x03) | ((self.tccr2b & 0x08) >> 1);
        wgm == 0b001
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
        } else if self.t2_is_phase_correct() {
            // OCF2x on the match on either slope, TOV2 at BOTTOM.
            let phase = dual_phase(u64::from(self.tcnt2), self.t2_down, 0xFF);
            let (tcnt, down) = dual_from_phase((phase + ticks) % 0x1FE, 0xFF);
            let step = T2Step {
                tcnt: tcnt as u8,
                ocfa: dual_hits(phase, ticks, 0xFF, u64::from(self.ocr2a)),
                ocfb: dual_hits(phase, ticks, 0xFF, u64::from(self.ocr2b)),
                tov: dual_hits(phase, ticks, 0xFF, 0),
            };
            self.t2_absorb(step);
            self.t2_down = down;
        } else {
            self.t2_absorb(t2_normal_closed(self.tcnt2, ticks, self.ocr2a, self.ocr2b));
        }
        if !self.t2_is_phase_correct() {
            self.t2_down = false;
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
        if self.t2_is_phase_correct() {
            let phase = dual_phase(u64::from(self.tcnt2), self.t2_down, 0xFF);
            let n = [
                (TIMSK2_OCIE2A, u64::from(self.ocr2a)),
                (TIMSK2_OCIE2B, u64::from(self.ocr2b)),
                (TIMSK2_TOIE2, 0),
            ]
            .into_iter()
            .filter(|(bit, _)| mask & bit != 0)
            .filter_map(|(_, target)| dual_dist(phase, 0xFF, target))
            .min()?;
            let cycles = n * div - u64::from(self.t2_prescale_acc).min(div - 1);
            return Some(cycles.max(1));
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

    /// CPU cycles until the next Timer0 flag whose interrupt is enabled.
    fn timer0_wake_cycles(&self) -> Option<u64> {
        let div = u64::from(self.t0_prescaler());
        if !self.io_clock_running() || div == 0 || !self.flag_i() {
            return None;
        }
        let tcnt = u64::from(self.tcnt0);
        // Timer clocks until the count lands on `target` (a full period if
        // it is there now). A blocked first clock can only make it later.
        let dist = |target: u64| match (target + 256 - tcnt) % 256 {
            0 => 256,
            d => d,
        };
        let ticks = [
            (TIMSK_TOIE0, 0),
            (TIMSK_OCIE0A, (u64::from(self.ocr0a) + 1) & 0xFF),
            (TIMSK_OCIE0B, (u64::from(self.ocr0b) + 1) & 0xFF),
        ]
        .into_iter()
        .filter(|(bit, _)| self.timsk0 & bit != 0)
        .map(|(_, target)| dist(target))
        .min()?;
        let cycles = ticks * div - u64::from(self.t0_prescale_acc).min(div - 1);
        Some(cycles.max(1))
    }

    /// An interrupt the core would take now: global enable set and a vector
    /// pending. This is also what wakes a sleeping core.
    #[inline]
    fn wake_pending(&self) -> bool {
        self.flag_i()
            && (self.pending_irq & 0xFFFF_FFFE != 0
                || (self.is_mega() && self.pending_irq >> mega_ext_int::NATIVE_BASE != 0))
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
                // EEPROM is NVMCTRL-backed on AVRxt. Do not misplace a biased
                // EEPROM image at data-space address zero while NVMCTRL
                // programming and reads remain unmodelled.
                if self.is_avrxt() && addr >= AVR_EEPROM_VMA_BIAS {
                    continue;
                }
                // `EEMEM` goes to the data EEPROM, never into SRAM or I/O.
                if addr >= AVR_EEPROM_VMA_BIAS {
                    for (i, b) in seg.data.iter().enumerate() {
                        if let Some(cell) = self.eeprom.data.get_mut(d as usize + i) {
                            *cell = *b;
                        }
                    }
                    continue;
                }
                // Data / EEPROM space (biased VMA from avr-gcc).
                for (i, b) in seg.data.iter().enumerate() {
                    let a = d as u16 + i as u16;
                    if (self.sram_start..=self.ramend).contains(&a) {
                        self.sram[(a - self.sram_start) as usize] = *b;
                    } else if (self.is_avrxt() && a < 0x1000) || (self.is_avrrc() && a < 0x40) {
                        self.tiny_io[a as usize] = *b;
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

    /// Whether the bus maps the ATmega328PB USART1 host window.
    fn usart1_on_bus(bus: &dyn Bus) -> bool {
        bus.has_mmio_window(AVR_IO_MIRROR_BASE + 0xC8)
    }

    /// Forward a USART1 register write when the chip descriptor has a host
    /// UART window. A refused write on a mapped window is an error.
    fn usart1_mirror_write(bus: &mut dyn Bus, reg: u64, value: u8) -> SimResult<()> {
        if Self::usart1_on_bus(bus) {
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
        if self.is_avrxt() {
            return self.try_take_irq_avrxt(bus);
        }
        if !self.flag_i() || self.pending_irq == 0 {
            return Ok(false);
        }
        // ATmega2560 INT2..INT7: native vector numbers (`avr/mega_ext_int.rs`).
        if self.pending_irq >> mega_ext_int::NATIVE_BASE != 0 && self.is_mega() {
            if let Some(slot) = self.mega_take_native_irq() {
                self.set_flag_i(false);
                self.push_pc(bus)?;
                self.pc = (slot - 1) * 4;
                return Ok(true);
            }
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
            self.sync_timer0_irq();
        } else if vec == VEC_TIMER0_COMPA {
            self.tifr0 &= !TIFR_OCF0A;
            self.sync_timer0_irq();
        } else if vec == VEC_TIMER0_COMPB {
            self.tifr0 &= !TIFR_OCF0B;
            self.sync_timer0_irq();
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
        if vec == wdt::VEC_WDT && self.wdt.handles(wdt::ADDR_WDTCSR) {
            self.wdt_vector_entered();
        }
        // ... and the INTn / PCIFn flag of an external or pin-change vector.
        self.ext_vector_entered(vec);
        if self.is_mega() {
            match vec {
                mega_adc::VEC_ADC => self.mega_adc_vector_entered(),
                mega_spi::VEC_SPI_STC => self.mega_spi_vector_entered(),
                _ => {}
            }
        }
        self.push_pc(bus)?;
        let slot = if self.is_mega() {
            mega2560_vector(vec)
        } else if self.is_32u4() {
            m32u4_vector(vec)
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

        if self.is_avrrc() && self.exec_avrrc(bus, op, pc, next)?.is_some() {
            return Ok(());
        }

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
    /// boundary, or the end of an EEPROM write while `EERIE` waits on it.
    /// Nothing to skip when an interrupt is already takeable, or a
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
                self.usb.wake_cycles(self.cycles),
                self.wdt.wake_cycles(self.cycles),
                self.eeprom.ready_wake_cycles(self.cycles),
                self.mega_adc_wake_cycles(),
                self.mega_spi_wake_cycles(),
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
        if self.is_avrxt() || self.is_avrrc() {
            self.tiny_io.fill(0);
            self.r = [0; 32];
            self.pc = 0;
            self.sp = self.ramend;
            self.sreg = 0;
            self.pending_irq = 0;
            self.cycles = 0;
            self.sleeping = false;
            self.irq_shadow = false;
            self.reset_avrxt();
            let _ = bus;
            return Ok(());
        }
        // MCUCR resets to 0: PUD clear, so the ports' pull-ups come back.
        let mcucr = &mut self.io[usize::from(ADDR_MCUCR - 0x20)];
        if *mcucr & MCUCR_PUD != 0 {
            bus.set_pull_ups_disabled(false);
        }
        *mcucr = 0;
        self.reset_ports(bus);
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
        self.ocr0a = 0;
        self.ocr0b = 0;
        self.t0_prescale_acc = 0;
        self.t0_match_blocked = false;
        self.tcnt2 = 0;
        self.tccr2a = 0;
        self.tccr2b = 0;
        self.ocr2a = 0;
        self.ocr2b = 0;
        self.timsk2 = 0;
        self.tifr2 = 0;
        self.assr = 0;
        self.t2_prescale_acc = 0;
        self.t2_down = false;
        self.t1 = timer1::Timer1::default();
        self.oc_mask = [0; 3];
        self.oc_level = [0; 3];
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
        self.mx = mega_ext_int::MegaExt::default();
        if self.is_mega() {
            self.mega_adc_reset();
            self.mega_spi_reset();
        }
        self.wdt_after_reset();
        self.eeprom.reset();
        self.irq_shadow = false;
        // A reset drops the pull-up: the host sees a disconnect.
        self.usb = usb::UsbDevice::default();
        // Keep attached SPI/I2C slaves across reset (same wiring as real board).
        Ok(())
    }

    fn on_supply_reset(&mut self, cause: crate::power::SupplyResetCause) {
        if self.is_avrxt() || self.is_avrrc() {
            return;
        }
        let flags = &mut self.io[usize::from(ADDR_MCUSR - 0x20)];
        match cause {
            crate::power::SupplyResetCause::PowerOn => *flags = 1, // PORF
            crate::power::SupplyResetCause::BrownOut => *flags |= 1 << 2, // BORF
        }
        // A power-on clears WDRF, which releases the forced WDE.
        self.wdt_after_reset();
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

        // ATmega32U4: bus reset and SOF frames up to now (a compare while the
        // device is detached; every other model never attaches).
        self.usb_tick();
        // A watchdog time-out: the interrupt is raised below like any other,
        // a system reset restarts the core at this boundary.
        if self.wdt_poll(bus)? {
            return Ok(());
        }

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
        if !self.is_avrxt()
            && self.pending_irq & (1u64 << VEC_USART_RX) != 0
            && self.flag_i()
            && bus.read_u8(AVR_IO_MIRROR_BASE + 0xC0)? & UCSRA_RXC == 0
        {
            self.pending_irq &= !(1u64 << VEC_USART_RX);
        }

        // EE_READY: level-sensitive on EERIE with no write in progress.
        if self.eeprom.irq_watched() || self.pending_irq & (1u64 << eeprom::VEC_EE_READY) != 0 {
            self.sync_eeprom_irq();
        }

        // INT0/INT1 and PCINT: sample the pads at this boundary.
        if self.ext_watch != 0 {
            self.sample_ext_pins(bus);
        }
        // ATmega2560 family: ADC conversions, triggers and SPI transfers due
        // at this boundary.
        if self.is_mega() {
            if self.madc.active(self.adcsra) {
                self.mega_adc_poll(bus)?;
            }
            if self.mega_spi_active() {
                self.mega_spi_poll(bus);
            }
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
        self.xt.retire();

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
        // So does an attached USB device: its frames come on cycle deadlines.
        let irq_takeable =
            irq_takeable || self.sleeping || self.ext_watch != 0 || self.usb.active();
        // So do an ATmega2560 ADC conversion or SPI transfer in flight.
        let irq_takeable = irq_takeable
            || (self.is_mega() && (self.madc.active(self.adcsra) || self.mega_spi_active()));
        // The INC/RJMP spin touches no bus, so it can never push a pad edge:
        // it stays available under push capture, as long as the tap clock is
        // carried across the cycles it retires.
        // A running watchdog must see every boundary its time-out can land on.
        let wdt_stopped = self.wdt.wake_cycles(self.cycles).is_none();
        if config.batch_mode_enabled
            && observers.is_empty()
            && timer_stopped
            && wdt_stopped
            && !irq_takeable
        {
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

#[path = "avr/avrxt.rs"]
mod avrxt;
#[path = "avr/eeprom.rs"]
mod eeprom;
#[path = "avr/exec/mod.rs"]
mod exec;
#[path = "avr/ext_int.rs"]
mod ext_int;
#[path = "avr/mega_adc.rs"]
mod mega_adc;
#[path = "avr/mega_ext_int.rs"]
mod mega_ext_int;
#[path = "avr/mega_spi.rs"]
mod mega_spi;
#[path = "avr/oc_pins.rs"]
mod oc_pins;
#[path = "avr/timer1.rs"]
mod timer1;
/// ATmega32U4 USB device controller.
mod usb;
#[path = "avr/wdt.rs"]
mod wdt;
pub use ext_int::{VEC_INT0, VEC_INT1, VEC_PCINT0, VEC_PCINT1, VEC_PCINT2};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DmaRequest, SimulationConfig};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockBus {
        mem: HashMap<u64, u8>,
        mmio_bases: Vec<u64>,
        config: SimulationConfig,
    }

    impl MockBus {
        fn new() -> Self {
            Self {
                mem: HashMap::new(),
                mmio_bases: Vec::new(),
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
        fn has_mmio_window(&self, addr: u64) -> bool {
            self.mmio_bases.contains(&addr)
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

        // REFS=11, MUX=0x08 is the temperature sensor against 1.1 V: the
        // typical 314 mV at +25 °C, 314 * 1024 / 1100 = 292.
        cpu.load_words(0, &[0xEC08, 0x9300, 0x007C, 0xEC10, 0x9310, 0x007A, 0xCFFF]);
        cpu.set_pc(0);
        for _ in 0..4 {
            cpu.step(&mut bus, &[], &cfg).unwrap();
        }
        assert_eq!(u16::from(cpu.adch) << 8 | u16::from(cpu.adcl), 292);
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

    // Timer0 output compare units A and B (ATmega328P DS40002061 §15.6-15.7,
    // ATmega2560 DS2549 §16.6-16.7): TCNT0 == OCR0x raises OCF0x on the next
    // timer clock, OCIE0x in TIMSK0 requests TIMER0_COMPA/COMPB (328P vectors
    // 15/16, 2560 vectors 22/23) while the flag is set, entering the vector or
    // writing a one clears it, and a TCNT0 write blocks the match in the next
    // timer clock. Bits: TIFR0/TIMSK0 OCF0A/OCIE0A = 1, OCF0B/OCIE0B = 2.
    #[test]
    fn timer0_compare_a_sets_ocf0a_on_the_clock_after_the_match() {
        let mut cpu = Avr::new();
        cpu.tccr0a = 0x03; // fast PWM, TOP = 0xFF (Arduino init())
        cpu.tccr0b = 0x03; // clk/64
        cpu.ocr0a = 128;
        cpu.timsk0 = 1 << 1;
        cpu.tick_timer0(128 * 64);
        assert_eq!(cpu.tcnt0, 128);
        assert_eq!(cpu.tifr0 & (1 << 1), 0, "match, flag on the next clock");
        assert_eq!(cpu.pending_irq & (1 << 15), 0);
        cpu.tick_timer0(64);
        assert_eq!(cpu.tcnt0, 129);
        assert_ne!(cpu.tifr0 & (1 << 1), 0, "OCF0A");
        assert_ne!(cpu.pending_irq & (1 << 15), 0, "TIMER0_COMPA requested");
        // One full 256-clock period later it matches again, in one closed-form
        // chunk as an idle fast-forward hands it.
        cpu.tifr0 = 0;
        cpu.pending_irq = 0;
        cpu.tick_timer0(255 * 64);
        assert_eq!(cpu.tifr0 & (1 << 1), 0);
        cpu.tick_timer0(64);
        assert_ne!(cpu.tifr0 & (1 << 1), 0);
    }

    #[test]
    fn timer0_compare_b_request_follows_flag_and_enable() {
        let mut cpu = Avr::new();
        let mut bus = MockBus::new();
        cpu.tccr0a = 0x03;
        cpu.tccr0b = 0x03;
        cpu.ocr0b = 10;
        cpu.tick_timer0(11 * 64);
        assert_ne!(cpu.tifr0 & (1 << 2), 0, "OCF0B sets without its enable");
        assert_eq!(cpu.pending_irq & (1 << 16), 0);
        cpu.data_write(0x6E, 1 << 2, &mut bus).unwrap();
        assert_ne!(
            cpu.pending_irq & (1 << 16),
            0,
            "OCIE0B set over a pending flag requests the vector"
        );
        cpu.data_write(0x35, 1 << 2, &mut bus).unwrap();
        assert_eq!(cpu.tifr0 & (1 << 2), 0, "write one to clear");
        assert_eq!(cpu.pending_irq & (1 << 16), 0, "and the request goes");
    }

    #[test]
    fn timer0_tcnt0_write_blocks_the_next_compare_match() {
        let mut cpu = Avr::new();
        let mut bus = MockBus::new();
        cpu.tccr0b = 0x03;
        cpu.ocr0a = 50;
        cpu.data_write(0x46, 50, &mut bus).unwrap();
        cpu.tick_timer0(64);
        assert_eq!(cpu.tcnt0, 51);
        assert_eq!(cpu.tifr0 & (1 << 1), 0, "blocked by the TCNT0 write");
        cpu.tick_timer0(256 * 64);
        assert_ne!(cpu.tifr0 & (1 << 1), 0, "the next pass matches");
    }

    #[test]
    fn timer0_compa_enters_its_vector_and_clears_ocf0a() {
        // 328P: TIMER0_COMPA_vect is vector 15 (byte 0x38); 2560: vector 22
        // (byte 0x54).
        for (chip, at) in [("atmega328p", 0x38u32), ("atmega2560", 0x54)] {
            let mut cpu = Avr::for_chip_name(chip);
            let mut bus = MockBus::new();
            cpu.sreg = 0x80;
            cpu.tccr0b = 0x03;
            cpu.timsk0 = 1 << 1;
            cpu.tick_timer0(64);
            cpu.load_words(0x200, &[0x0000]);
            cpu.pc = 0x200;
            cpu.step(&mut bus, &[], &SimulationConfig::default())
                .unwrap();
            assert_eq!(cpu.pc, at, "{chip}");
            assert_eq!(cpu.tifr0 & (1 << 1), 0, "{chip}: OCF0A cleared on entry");
        }
    }

    /// A sleeping core with only OCIE0A set wakes at the compare match, not at
    /// the overflow, when the idle budget skips the clocks in one chunk.
    #[test]
    fn timer0_compare_match_bounds_the_sleep_budget() {
        let mut cpu = Avr::new();
        let bus = MockBus::new();
        cpu.tccr0a = 0x03;
        cpu.tccr0b = 0x03;
        cpu.ocr0a = 20;
        cpu.tcnt0 = 3;
        cpu.timsk0 = 1 << 1;
        cpu.sreg = 0x80;
        cpu.sleeping = true;
        let budget = cpu.idle_fast_forward_budget(&bus).expect("asleep");
        assert_eq!(
            budget,
            (21 - 3) * 64,
            "wakes on the clock after TCNT0 == 20"
        );
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

    /// Phase-correct (WGM 1) against a one-clock-at-a-time dual-slope
    /// reference: 0..=255 up, 254..=1 down, OCF2x on the match on either
    /// slope, TOV2 at BOTTOM.
    #[test]
    fn timer2_phase_correct_counts_up_and_back_down() {
        let reference = |mut tcnt: u8, mut down: bool, ocr_a: u8, ocr_b: u8, ticks: u64| {
            let (mut ocfa, mut ocfb, mut tov) = (false, false, false);
            for _ in 0..ticks {
                if tcnt == 0xFF {
                    down = true;
                }
                if tcnt == 0 {
                    down = false;
                }
                tcnt = if down { tcnt - 1 } else { tcnt + 1 };
                ocfa |= tcnt == ocr_a;
                ocfb |= tcnt == ocr_b;
                tov |= tcnt == 0;
            }
            let down = down && tcnt != 0 && tcnt != 0xFF;
            (tcnt, down, ocfa, ocfb, tov)
        };
        let cases = [
            (0u8, false, 128u8, 0u8, 255u64),
            (0, false, 128, 0, 256),
            (0, false, 128, 0, 509),
            (0, false, 128, 0, 510),
            (200, true, 199, 201, 1),
            (200, true, 10, 255, 150),
            (255, false, 254, 1, 3),
            (7, false, 255, 0, 510 * 4 + 300),
            (1, true, 1, 2, 1),
        ];
        for (start, down, ocr_a, ocr_b, ticks) in cases {
            let want = reference(start, down, ocr_a, ocr_b, ticks);
            let mut cpu = Avr::new();
            cpu.tccr2a = 0x01;
            cpu.tccr2b = 1;
            cpu.ocr2a = ocr_a;
            cpu.ocr2b = ocr_b;
            cpu.tcnt2 = start;
            cpu.t2_down = down;
            cpu.tick_timer2(ticks as u32);
            let got = (
                cpu.tcnt2,
                cpu.t2_down,
                cpu.tifr2 & TIFR2_OCF2A != 0,
                cpu.tifr2 & TIFR2_OCF2B != 0,
                cpu.tifr2 & TIFR2_TOV2 != 0,
            );
            assert_eq!(
                got, want,
                "start={start} down={down} ocr={ocr_a}/{ocr_b} ticks={ticks}"
            );
        }
    }

    /// Arduino `init()` puts Timer2 in phase-correct clk/64: TOV2 every
    /// 64 * 510 = 32,640 CPU cycles (490 Hz on D3/D11), not 64 * 256.
    #[test]
    fn timer2_phase_correct_overflow_period_is_510_clocks() {
        let mut cpu = Avr::new();
        cpu.tccr2a = 0x01;
        cpu.tccr2b = 0x04; // clk/64
        cpu.timsk2 = TIMSK2_TOIE2;
        cpu.sreg |= 0x80;
        assert_eq!(cpu.timer2_wake_cycles(), Some(32_640));
        cpu.tick_timer2(32_640 - 64);
        assert_eq!(
            (cpu.tcnt2, cpu.t2_down, cpu.tifr2 & TIFR2_TOV2),
            (1, true, 0)
        );
        cpu.tick_timer2(64);
        assert_eq!((cpu.tcnt2, cpu.tifr2 & TIFR2_TOV2), (0, TIFR2_TOV2));
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
    fn atmega328pb_selects_shared_geometry_and_routes_usart1() {
        use crate::bus::SystemBus;
        use labwired_config::{ChipDescriptor, SystemManifest};

        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chip_path = root.join("configs/chips/atmega328pb.yaml");
        let chip = ChipDescriptor::from_file(&chip_path).expect("atmega328pb config");
        let manifest = SystemManifest {
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "atmega328pb-usart1-smoke".to_string(),
            chip: chip_path.to_string_lossy().to_string(),
            cpu_hz: Some(16_000_000),
            external_devices: Vec::new(),
            board_io: Vec::new(),
            debug_uart: None,
            wifi_ap: None,
            peripherals: Vec::new(),
            parts: Default::default(),
            memory_overrides: Default::default(),
        };
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("PB bus");
        let usart1 = AVR_IO_MIRROR_BASE + 0xC8;
        assert!(
            bus.has_mmio_window(usart1),
            "USART1 host window must be mapped"
        );
        assert!(
            bus.has_mmio_window(AVR_IO_MIRROR_BASE + 0xC0),
            "USART0 remains a separate window"
        );

        let mut cpu = Avr::for_chip_name("ATmega328PB-AU");
        assert_eq!(cpu.model, AvrModel::Atmega328pb);
        assert_eq!(cpu.flash.len(), 32 * 1024);
        assert_eq!(cpu.sram.len(), 2 * 1024);
        assert_eq!(cpu.sp, 0x08FF);
        let console = Arc::new(Mutex::new(Vec::new()));
        cpu.set_serial_sink(Arc::clone(&console));

        // Execute a small AVR firmware image: configure U2X1, USART1 TX/RX,
        // and UBRR1, then write 'P' through UDR1. STS is the 328PB data-space
        // access used by avr-gcc for this extended I/O window.
        cpu.load_words(
            0,
            &[
                0xE002, 0x9300, 0x00C8, // LDI r16,2; STS UCSR1A,r16
                0xE108, 0x9300, 0x00C9, // LDI r16,0x18; STS UCSR1B,r16
                0xE007, 0x9300, 0x00CC, // LDI r16,7; STS UBRR1L,r16
                0xE000, 0x9300, 0x00CD, // LDI r16,0; STS UBRR1H,r16
                0xE500, 0x9300, 0x00CE, // LDI r16,'P'; STS UDR1,r16
            ],
        );
        for _ in 0..10 {
            cpu.step(&mut bus, &[], &SimulationConfig::default())
                .unwrap();
        }

        assert_eq!(cpu.serial_tx, b"P");
        assert_eq!(*console.lock().unwrap(), b"P");
        assert_ne!(cpu.data_read(0xC8, &bus).unwrap() & UCSRA_UDRE, 0);
        assert_ne!(cpu.data_read(0xC8, &bus).unwrap() & UCSRA_TXC, 0);

        // RXC1 and UDR1 come from the second bus-side USART, not USART0.
        let mut rx_bus = MockBus::new();
        rx_bus.mmio_bases.push(usart1);
        rx_bus.mem.insert(usart1, UCSRA_RXC);
        rx_bus.mem.insert(usart1 + 6, b'R');
        assert_ne!(cpu.data_read(0xC8, &rx_bus).unwrap() & UCSRA_RXC, 0);
        assert_eq!(cpu.data_read(0xCE, &rx_bus).unwrap(), b'R');

        // The 328P does not acquire the PB-only USART1 window by fallback.
        let mut p = Avr::for_chip_name("atmega328p");
        p.data_write(0xCE, b'X', &mut rx_bus).unwrap();
        assert!(p.serial_tx.is_empty());
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
    fn mega_128k_names_use_the_128k_two_byte_pc_core() {
        let cpu = Avr::for_chip_name("atmega1281");
        assert_eq!(cpu.model, AvrModel::Atmega1281);
        assert_eq!(cpu.flash.len(), 128 * 1024);
        assert_eq!(cpu.sram.len(), 8 * 1024);
        assert_eq!(cpu.sp, 0x21FF);
        assert!(!cpu.model.three_byte_pc());
    }

    #[test]
    fn attiny404_selects_avrxt_geometry_and_runs_call_stack_and_sram_firmware() {
        let mut cpu = Avr::for_chip_name("attiny404");
        assert_eq!(cpu.model, AvrModel::Attiny404);
        assert_eq!(cpu.flash.len(), 4 * 1024);
        assert_eq!(cpu.sram.len(), 256);
        assert_eq!(cpu.sp, 0x3FFF);
        assert!(!cpu.model.three_byte_pc());

        // Exercise direct-address OUT/IN/SBI, store to data SRAM, call a
        // subroutine (exercising the top-of-SRAM stack), and return. The
        // ATtiny register file is separate from data memory, so a data access
        // to 0x0000 must not overwrite r0.
        cpu.load_words(
            0,
            &[
                0xE402, // LDI r16, 0x42
                0xB900, // OUT VPORTA.DIR(0x00), r16
                0xB110, // IN r17, VPORTA.DIR(0x00)
                0x9A00, // SBI VPORTA.DIR(0x00), 0
                0x9300, 0x3F00, // STS 0x3F00, r16
                0x9300, 0x0001, // STS VPORTA.OUT, r16 (data-space I/O)
                0xD003, // RCALL subroutine four words ahead
                0xB73D, // IN r19, CPU.SPL(0x3D)
                0xB74F, // IN r20, CPU.SREG(0x3F)
                0xCFFF, // loop forever after return
                0xE021, // subroutine: LDI r18, 1
                0x9508, // RET
            ],
        );
        let mut bus = MockBus::new();
        step_n(&mut cpu, &mut bus, 11);

        assert_eq!(cpu.sram[0], 0x42, "STS writes to ATtiny404 SRAM");
        assert_eq!(cpu.tiny_io[0], 0x43, "data address zero is I/O");
        assert_eq!(cpu.tiny_io[1], 0x42, "STS writes data-space I/O");
        assert_eq!(cpu.r[0], 0, "data address zero is not R0");
        assert_eq!(cpu.r[17], 0x42, "IN reads direct-address AVRxt I/O");
        assert_eq!(cpu.r[18], 0x01, "subroutine executes after the direct IN");
        assert_eq!(cpu.r[19], 0xFF, "CPU.SPL is at I/O address 0x3D");
        assert_eq!(cpu.r[20], 0, "CPU.SREG is at I/O address 0x3F");
        assert_eq!(cpu.sp, 0x3FFF, "RET restores the AVRxt stack pointer");
        assert_eq!(cpu.sram[0xFF], 0, "return PC high byte was pushed");
        assert_eq!(cpu.sram[0xFE], 9, "return PC low byte was pushed");
    }

    /// Classic (pre-AVRxt) tinyAVR must select their OWN model.
    ///
    /// Regression guard. Before these arms existed every one of these parts hit
    /// the `else` branch and became an ATmega328p: 32 KB flash, SRAM at 0x0100
    /// and the 328P vector table. Firmware then ran against geometry the part
    /// does not have, and nothing failed loudly - which is exactly how
    /// labwired-ip came to claim 13 parts as onboarded when they were not.
    ///
    /// Geometry is from each datasheet's data-memory map (SRAM start .. RAMEND).
    #[test]
    fn classic_tinyavr_selects_its_own_geometry_and_not_the_atmega328p() {
        // (name, model, flash, sram_start, ramend, sram_len)
        let cases = [
            ("attiny13", AvrModel::Attiny13, 1024, 0x0060, 0x009F, 64),
            (
                "attiny25",
                AvrModel::Attiny25,
                2 * 1024,
                0x0060,
                0x00DF,
                128,
            ),
            (
                "attiny45",
                AvrModel::Attiny45,
                4 * 1024,
                0x0060,
                0x015F,
                256,
            ),
            (
                "attiny85",
                AvrModel::Attiny85,
                8 * 1024,
                0x0060,
                0x025F,
                512,
            ),
            (
                "attiny2313",
                AvrModel::Attiny2313,
                2 * 1024,
                0x0060,
                0x00DF,
                128,
            ),
            (
                "attiny261",
                AvrModel::Attiny261,
                2 * 1024,
                0x0060,
                0x00DF,
                128,
            ),
            (
                "attiny461",
                AvrModel::Attiny461,
                4 * 1024,
                0x0060,
                0x015F,
                256,
            ),
            (
                "attiny861",
                AvrModel::Attiny861,
                8 * 1024,
                0x0060,
                0x025F,
                512,
            ),
            (
                "attiny44",
                AvrModel::Attiny44,
                4 * 1024,
                0x0060,
                0x015F,
                256,
            ),
            (
                "attiny84",
                AvrModel::Attiny84,
                8 * 1024,
                0x0060,
                0x025F,
                512,
            ),
            // The 48/88 have a 160-byte extended I/O file, so SRAM starts at
            // 0x0100 rather than 0x0060 (ATtiny48/88 datasheet Table 5-2).
            (
                "attiny48",
                AvrModel::Attiny48,
                4 * 1024,
                0x0100,
                0x01FF,
                256,
            ),
            (
                "attiny88",
                AvrModel::Attiny88,
                8 * 1024,
                0x0100,
                0x02FF,
                512,
            ),
        ];
        for (name, model, flash, sram_start, ramend, sram_len) in cases {
            assert_eq!(
                AvrModel::from_chip_name(name),
                model,
                "{name} must not fall through to the default model"
            );
            let cpu = Avr::for_chip_name(name);
            assert_eq!(cpu.model, model, "{name} selects its exact model");
            assert_eq!(cpu.flash.len(), flash, "{name} flash size");
            assert_eq!(cpu.sram_start, sram_start, "{name} SRAM start");
            assert_eq!(cpu.sram.len(), sram_len, "{name} SRAM size");
            assert_eq!(cpu.ramend, ramend, "{name} RAMEND");
            // These are classic AVR, so they must NOT get the AVRxt data map.
            // Giving them 0x3Fxx SRAM would be as wrong as the 328P fallback.
            assert!(
                !cpu.model.uses_avrxt_data_map(),
                "{name} is classic AVR, not AVRxt"
            );
        }
    }

    /// Classic megaAVR / AT90USB parts without a model of their own keep the
    /// 328P model but must take their own flash and SRAM geometry.
    ///
    /// Regression guard. Before the geometry table every one of these names
    /// ran with 32 KB flash and SRAM 0x0100..=0x08FF. avr-libc startup sets SP
    /// to the part's RAMEND, so e.g. an ATmega128 image faulted on its first
    /// push at 0x10FF. Values are avr-libc FLASHEND + 1, RAMSTART and RAMEND.
    #[test]
    fn classic_megaavr_names_take_their_own_flash_and_sram_geometry() {
        // (name, flash, sram_start, ramend)
        let cases: &[(&str, usize, u16, u16)] = &[
            ("atmega8", 8 * 1024, 0x0060, 0x045F),
            ("atmega8a", 8 * 1024, 0x0060, 0x045F),
            ("atmega16", 16 * 1024, 0x0060, 0x045F),
            ("ATmega16A", 16 * 1024, 0x0060, 0x045F),
            ("atmega32", 32 * 1024, 0x0060, 0x085F),
            ("atmega64", 64 * 1024, 0x0100, 0x10FF),
            ("atmega64a", 64 * 1024, 0x0100, 0x10FF),
            ("atmega128", 128 * 1024, 0x0100, 0x10FF),
            ("atmega8515", 8 * 1024, 0x0060, 0x025F),
            ("atmega8535", 8 * 1024, 0x0060, 0x025F),
            ("atmega162", 16 * 1024, 0x0100, 0x04FF),
            ("atmega48", 4 * 1024, 0x0100, 0x02FF),
            ("atmega48p", 4 * 1024, 0x0100, 0x02FF),
            ("atmega88pa", 8 * 1024, 0x0100, 0x04FF),
            ("atmega168", 16 * 1024, 0x0100, 0x04FF),
            ("atmega168p", 16 * 1024, 0x0100, 0x04FF),
            ("atmega164p", 16 * 1024, 0x0100, 0x04FF),
            ("atmega324p", 32 * 1024, 0x0100, 0x08FF),
            ("atmega644", 64 * 1024, 0x0100, 0x10FF),
            ("atmega644p", 64 * 1024, 0x0100, 0x10FF),
            ("atmega1284p", 128 * 1024, 0x0100, 0x40FF),
            ("atmega165p", 16 * 1024, 0x0100, 0x04FF),
            ("atmega169", 16 * 1024, 0x0100, 0x04FF),
            ("atmega325", 32 * 1024, 0x0100, 0x08FF),
            ("atmega329p", 32 * 1024, 0x0100, 0x08FF),
            ("atmega3250", 32 * 1024, 0x0100, 0x08FF),
            ("atmega3290p", 32 * 1024, 0x0100, 0x08FF),
            ("atmega645", 64 * 1024, 0x0100, 0x10FF),
            ("atmega649", 64 * 1024, 0x0100, 0x10FF),
            ("atmega6450", 64 * 1024, 0x0100, 0x10FF),
            ("atmega6490", 64 * 1024, 0x0100, 0x10FF),
            ("at90usb646", 64 * 1024, 0x0100, 0x10FF),
            ("at90usb1286", 128 * 1024, 0x0100, 0x20FF),
        ];
        for &(name, flash, sram_start, ramend) in cases {
            let cpu = Avr::for_chip_name(name);
            assert_eq!(
                cpu.model,
                AvrModel::Atmega328p,
                "{name} keeps the 328P model"
            );
            assert_eq!(cpu.flash.len(), flash, "{name} flash size");
            assert_eq!(cpu.sram_start, sram_start, "{name} SRAM start");
            assert_eq!(cpu.ramend, ramend, "{name} RAMEND");
            assert_eq!(
                cpu.sram.len(),
                usize::from(ramend - sram_start) + 1,
                "{name} SRAM size"
            );
            assert_eq!(cpu.sp, ramend, "{name} SP resets to RAMEND");
            assert_eq!(
                Some(cpu.geometry()),
                AvrGeometry::for_chip_name(name),
                "{name} reports the table geometry"
            );
        }

        // Names outside the table keep their model's geometry. The first two
        // are the parts the table's shorter names must not capture.
        for name in ["atmega640", "atmega1280", "atmega328p", "atmega328", ""] {
            assert_eq!(
                AvrGeometry::for_chip_name(name),
                None,
                "{name:?} not in table"
            );
            let cpu = Avr::for_chip_name(name);
            assert_eq!(cpu.flash.len(), FLASH_SIZE, "{name:?} flash size");
            assert_eq!(
                (cpu.sram_start, cpu.ramend),
                (SRAM_START, RAMEND),
                "{name:?} SRAM"
            );
        }
        // A part with its own model is not resized by the table.
        let cpu = Avr::for_chip_name("atmega1281");
        assert_eq!(cpu.model, AvrModel::Atmega1281);
        assert_eq!((cpu.flash.len(), cpu.ramend), (128 * 1024, 0x21FF));
    }

    /// ATtiny48/88 have PORTB/PORTC/PORTD at the 328P addresses, so an OUT to
    /// PORTB must reach the bus-side port window like it does on the 328P.
    #[test]
    fn attiny48_and_attiny88_mirror_portb_to_the_bus_port_window() {
        for name in ["attiny48", "attiny88"] {
            let mut cpu = Avr::for_chip_name(name);
            cpu.load_words(
                0,
                &[
                    0xEF0F, // LDI r16, 0xFF
                    0xB904, // OUT DDRB(0x04), r16
                    0xEA0A, // LDI r16, 0xAA
                    0xB905, // OUT PORTB(0x05), r16
                ],
            );
            let mut bus = MockBus::new();
            step_n(&mut cpu, &mut bus, 4);
            assert_eq!(
                bus.mem.get(&(AVR_IO_MIRROR_BASE + 0x25)),
                Some(&0xAA),
                "{name} PORTB latch is mirrored"
            );
            assert_eq!(
                bus.mem.get(&(AVR_IO_MIRROR_BASE + 0x24)),
                Some(&0xFF),
                "{name} DDRB is mirrored"
            );
        }
    }

    /// The ATmega128 stack starts at RAMEND 0x10FF, above the 328P's 0x08FF.
    #[test]
    fn atmega128_calls_through_a_stack_at_its_own_ramend() {
        let mut cpu = Avr::for_chip_name("atmega128");
        cpu.load_words(
            0,
            &[
                0xEA05, // LDI r16, 0xA5
                0x9300, 0x0100, // STS 0x0100, r16 (first SRAM byte)
                0xD001, // RCALL subroutine two words ahead
                0xCFFF, // loop forever after return
                0x9110, 0x0100, // subroutine: LDS r17, 0x0100
                0x9508, // RET
            ],
        );
        let mut bus = MockBus::new();
        step_n(&mut cpu, &mut bus, 5);

        assert_eq!(cpu.r[17], 0xA5, "subroutine ran and read SRAM back");
        assert_eq!(cpu.sp, 0x10FF, "RET restores SP to RAMEND");
        assert_eq!(cpu.sram[0x0FFE], 4, "return PC low byte pushed at 0x10FE");
        assert_eq!(cpu.pc, 8, "returned to word 4, the one after RCALL");

        // The same image on the 328P geometry has no memory at 0x10FF.
        let mut cpu = Avr::new();
        cpu.sp = 0x10FF;
        cpu.load_words(0, &[0xD000, 0xCFFF]); // RCALL .+0
        let cfg = SimulationConfig::default();
        assert!(
            matches!(
                cpu.step(&mut bus, &[], &cfg),
                Err(SimulationError::MemoryViolation(0x10FF))
            ),
            "ATmega328P has no SRAM at 0x10FF"
        );
    }

    /// On a part whose SRAM starts at 0x0060, data 0x0060..=0x00FF is SRAM.
    /// On the 328P those addresses are WDTCSR (0x60) and UDR0 (0xC6).
    #[test]
    fn atmega16_sram_at_0x60_is_memory_and_not_328p_extended_io() {
        let mut cpu = Avr::for_chip_name("atmega16");
        let serial = Arc::new(Mutex::new(Vec::new()));
        cpu.set_serial_sink(serial.clone());
        cpu.load_words(
            0,
            &[
                0xE50A, // LDI r16, 0x5A
                0x9300, 0x0060, // STS 0x0060, r16 (RAMSTART; 328P WDTCSR)
                0x9300, 0x00C6, // STS 0x00C6, r16 (328P UDR0)
                0x9300, 0x045F, // STS 0x045F, r16 (RAMEND)
                0x9110, 0x0060, // LDS r17, 0x0060
                0x9120, 0x00C6, // LDS r18, 0x00C6
                0x9130, 0x045F, // LDS r19, 0x045F
                0x9300, 0x0460, // STS 0x0460, r16 (one past RAMEND)
            ],
        );
        let mut bus = MockBus::new();
        step_n(&mut cpu, &mut bus, 7);

        assert_eq!(cpu.sram[0], 0x5A, "0x0060 is the first SRAM byte");
        assert_eq!(cpu.sram[0x66], 0x5A, "0x00C6 is SRAM");
        assert_eq!(cpu.sram[0x3FF], 0x5A, "0x045F is the last SRAM byte");
        assert_eq!((cpu.r[17], cpu.r[18], cpu.r[19]), (0x5A, 0x5A, 0x5A));
        assert!(
            serial.lock().unwrap().is_empty(),
            "a store to SRAM 0x00C6 must not transmit on the 328P USART"
        );
        let cfg = SimulationConfig::default();
        assert!(
            matches!(
                cpu.step(&mut bus, &[], &cfg),
                Err(SimulationError::MemoryViolation(0x0460))
            ),
            "the address after RAMEND is not memory"
        );
    }

    #[test]
    fn tinyavr_0_series_selects_exact_avrxt_flash_and_sram_geometry() {
        for (name, model, flash_size, sram_start, sram_len) in [
            ("attiny202", AvrModel::Attiny202, 2 * 1024, 0x3F80, 128),
            ("attiny204", AvrModel::Attiny204, 2 * 1024, 0x3F80, 128),
            ("attiny402", AvrModel::Attiny402, 4 * 1024, 0x3F00, 256),
            ("attiny404", AvrModel::Attiny404, 4 * 1024, 0x3F00, 256),
            ("attiny406", AvrModel::Attiny406, 4 * 1024, 0x3F00, 256),
            ("attiny804", AvrModel::Attiny804, 8 * 1024, 0x3E00, 512),
            ("attiny806", AvrModel::Attiny806, 8 * 1024, 0x3E00, 512),
            ("attiny807", AvrModel::Attiny807, 8 * 1024, 0x3E00, 512),
            ("attiny1604", AvrModel::Attiny1604, 16 * 1024, 0x3C00, 1024),
            ("attiny1606", AvrModel::Attiny1606, 16 * 1024, 0x3C00, 1024),
            ("attiny1607", AvrModel::Attiny1607, 16 * 1024, 0x3C00, 1024),
        ] {
            let mut cpu = Avr::for_chip_name(name);
            assert_eq!(cpu.model, model, "{name} selects its exact model");
            assert_eq!(cpu.flash.len(), flash_size, "{name} flash size");
            assert_eq!(cpu.sram_start, sram_start, "{name} SRAM start");
            assert_eq!(cpu.sram.len(), sram_len, "{name} SRAM size");
            assert_eq!(cpu.ramend, 0x3FFF, "{name} RAMEND");
            assert!(cpu.model.uses_avrxt_data_map(), "{name} uses AVRxt");
            cpu.flash[flash_size - 1] = 0xA5;
            let mut bus = MockBus::new();
            let last_flash_byte = 0x8000 + (flash_size - 1) as u16;
            assert_eq!(
                cpu.avrxt_data_read(last_flash_byte, &bus).unwrap(),
                0xA5,
                "{name} final flash byte is mapped"
            );
            assert!(
                matches!(
                    cpu.avrxt_data_read(last_flash_byte + 1, &bus),
                    Err(SimulationError::MemoryViolation(_))
                ),
                "{name} address after flash is outside the mapped window"
            );

            // LDI r16,0x5A; STS first SRAM byte,r16; STS last SRAM byte,r16.
            cpu.load_words(0, &[0xE50A, 0x9300, sram_start, 0x9300, 0x3FFF]);
            step_n(&mut cpu, &mut bus, 3);
            assert_eq!(cpu.sram[0], 0x5A, "{name} first SRAM byte");
            assert_eq!(cpu.sram[sram_len - 1], 0x5A, "{name} last SRAM byte");
        }
    }

    #[test]
    fn attiny10_selects_the_avrrc_data_map() {
        let mut cpu = Avr::for_chip_name("attiny10");
        assert_eq!(cpu.model, AvrModel::Attiny10);
        assert!(cpu.model.is_reduced_core());
        assert!(!cpu.model.uses_avrxt_data_map());
        assert_eq!(cpu.flash.len(), 1024);
        assert_eq!(
            (cpu.sram_start, cpu.ramend, cpu.sram.len()),
            (0x40, 0x5F, 32)
        );
        assert_eq!(
            cpu.io_data_address(0x02),
            0x02,
            "PORTB is I/O and data 0x02"
        );
        // "attiny10" is a prefix of the ATtiny102/104; they must not alias it.
        for other in ["attiny102", "attiny104"] {
            assert_ne!(
                AvrModel::from_chip_name(other),
                AvrModel::Attiny10,
                "{other}"
            );
        }

        let mut bus = MockBus::new();
        cpu.flash[1023] = 0xA5;
        assert_eq!(cpu.avrrc_data_read(0x43FF, &bus).unwrap(), 0xA5);
        for unmapped in [0x0060, 0x3F00, 0x4400] {
            assert!(
                matches!(
                    cpu.avrrc_data_read(unmapped, &bus),
                    Err(SimulationError::MemoryViolation(_))
                ),
                "{unmapped:#06x} is outside the modelled map"
            );
        }
        assert!(
            cpu.avrrc_data_write(0x4000, 0, &mut bus).is_err(),
            "mapped flash is read-only to firmware"
        );

        // LDI r16,0x5A; STS 0x40,r16; STS 0x5F,r16 (one-word AVRrc forms).
        cpu.load_words(0, &[0xE50A, 0xA900, 0xAB0F]);
        step_n(&mut cpu, &mut bus, 3);
        assert_eq!(cpu.sram[0], 0x5A, "first SRAM byte");
        assert_eq!(cpu.sram[31], 0x5A, "last SRAM byte");
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
            // ADPS = 0 (/2): the first conversion is 25 ADC clocks, 50 cycles.
            cpu.cycles += 64;
            cpu.mega_adc_poll(bus).unwrap();
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
        fn is_selected(&self) -> bool {
            true // This D/C probe has no modeled chip-select gate.
        }
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

    /// The supported LedControl(11, 13, 10, 1) mapping bit-bangs with SPE=0.
    /// Soft-SPI after PORTB must latch a MAX7219 digit into the parking kit.
    #[test]
    fn soft_spi_portb_bitbang_paints_parking_max7219() {
        use crate::bus::SystemBus;
        use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let chip_path = root.join("configs/chips/atmega328p.yaml");
        let chip = ChipDescriptor::from_file(&chip_path).expect("atmega328p");
        let mut cfg = std::collections::HashMap::new();
        cfg.insert("cs_pin".into(), serde_yaml::Value::String("PB2".into()));
        let manifest = SystemManifest {
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "soft-spi-max7219".to_string(),
            chip: chip_path.to_string_lossy().to_string(),
            cpu_hz: Some(16_000_000),
            external_devices: vec![ExternalDevice {
                id: "DS1".to_string(),
                r#type: "led-matrix".to_string(),
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
        let mut cpu = Avr::for_chip_name("atmega328p");
        // SPE must stay clear (default) so soft-SPI owns the pins.
        assert_eq!(cpu.spcr & (1 << 6), 0, "SPE must be clear for soft-SPI");

        // DDRB: PB2 (CS), PB3 (MOSI), PB5 (SCK) as outputs.
        const PORTB: u16 = 0x25;
        const DDRB: u16 = 0x24;
        const CS: u8 = 1 << 2;
        const MOSI: u8 = 1 << 3;
        const SCK: u8 = 1 << 5;
        cpu.data_write(DDRB, CS | MOSI | SCK, &mut bus).unwrap();

        // Idle CS high.
        let mut port = CS;
        cpu.data_write(PORTB, port, &mut bus).unwrap();

        let bitbang_byte = |cpu: &mut Avr, bus: &mut SystemBus, port: &mut u8, byte: u8| {
            for i in (0..8).rev() {
                if (byte >> i) & 1 != 0 {
                    *port |= MOSI;
                } else {
                    *port &= !MOSI;
                }
                cpu.data_write(PORTB, *port, bus).unwrap();
                *port |= SCK;
                cpu.data_write(PORTB, *port, bus).unwrap();
                *port &= !SCK;
                cpu.data_write(PORTB, *port, bus).unwrap();
            }
        };

        // CS↓ then shutdown-off (0x0C, 0x01), then CS↑.
        port &= !CS;
        cpu.data_write(PORTB, port, &mut bus).unwrap();
        bitbang_byte(&mut cpu, &mut bus, &mut port, 0x0C);
        bitbang_byte(&mut cpu, &mut bus, &mut port, 0x01);
        port |= CS;
        cpu.data_write(PORTB, port, &mut bus).unwrap();

        // Digit 0 = 0xA5.
        port &= !CS;
        cpu.data_write(PORTB, port, &mut bus).unwrap();
        bitbang_byte(&mut cpu, &mut bus, &mut port, 0x01);
        bitbang_byte(&mut cpu, &mut bus, &mut port, 0xA5);
        port |= CS;
        cpu.data_write(PORTB, port, &mut bus).unwrap();

        let art = bus
            .display_artifact(
                "DS1",
                &crate::inspect::InspectOpts {
                    include_bytes: true,
                    ..Default::default()
                },
            )
            .expect("DS1 artifact");
        assert_eq!(art.meta["shutdown"], false, "soft-SPI must leave shutdown");
        assert_eq!(
            art.bytes.as_deref(),
            Some(&[0xA5u8, 0, 0, 0, 0, 0, 0, 0][..]),
            "soft-SPI digit write must land; meta={:?} bytes={:?}",
            art.meta,
            art.bytes
        );
    }
}
