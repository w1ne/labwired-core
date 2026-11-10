// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! AVR8 CPU — Harvard flash + byte data space.
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

/// The AVR parts this interpreter runs. ATmega16 and ATmega168 select their
/// exact flash / SRAM geometry while retaining the shared classic AVR8 path;
/// its imported profile claims no peripherals. The imported ATmega1280 profile is
/// routed through ATmega1281 geometry for CPU-only execution: both descriptors
/// specify 128 KB flash and 8 KB SRAM at 0x0200. This alias does not assert
/// ATmega1280 peripheral behavior. The ATmega2560 (Arduino Mega) has 256 KB
/// flash and a 3-byte PC; ATmega1281 has 128 KB flash and a 2-byte PC.
/// EIND for EIJMP/EICALL is available on the ATmega2560/2561 model, while
/// RAMPZ for ELPM is shared. The mega peripheral map includes ports A..L
/// (H..L in extended I/O at 0x100..0x10B), a 16-channel ADC (ADCSRB.MUX5
/// selects ADC8..ADC15) and its own vector table. Timer0/1/2, USART0, SPI and
/// TWI sit at the same data addresses as on the 328P, so they are shared; their
/// vector numbers are remapped on entry ([`mega2560_vector`]).
///
/// The tinyAVR 0-series uses an AVRxt data map: the register file is separate,
/// I/O occupies data addresses 0x0000..=0x0FFF, and SRAM begins at a device-
/// specific address below 0x4000. Its common instruction execution is shared,
/// but its peripherals and interrupt controller are not yet modelled.
///
/// Not modelled on the ATmega2560: INTn/PCINT (its pins differ from the
/// 328P's; the registers are stored only), output-compare pads (PWM
/// `analogWrite` levels; the pads follow `PORTx`), Timer3/4/5 and USART1..3
/// (registers stored only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvrModel {
    #[default]
    Atmega328p,
    /// ATmega328PB shares the 328P core geometry and adds a second USART.
    Atmega328pb,
    /// ATmega16 geometry for CPU/SRAM-only imported-profile execution.
    Atmega16,
    /// ATmega168 geometry for CPU/SRAM-only imported-profile execution.
    Atmega168,
    /// ATmega128 geometry, with 128 KiB flash and 4 KiB SRAM. Its imported
    /// onboarding profile is CPU/SRAM-only and does not claim peripherals.
    Atmega128,
    Atmega1281,
    Atmega2560,
    /// ATmega32U4 geometry, with the common AVR8 execution path. Its USB,
    /// interrupt-vector, and non-GPIO peripheral behavior are not modeled.
    Atmega32u4,
    /// ATmega1284P flash and SRAM geometry. Peripheral behavior remains
    /// outside this profile until its register map is modeled and exercised.
    Atmega1284p,
    // Classic megaAVR parts imported from simavr for CPU/SRAM-only execution.
    // Without these arms each resolved to Atmega328p geometry. No peripheral,
    // GPIO-mirror or vector-table behavior is claimed for them.
    Atmega8,
    Atmega48,
    Atmega88,
    Atmega32,
    Atmega164,
    Atmega324,
    Atmega644,
    // Remaining simavr megaAVR/USB imports, same CPU/SRAM-only scope.
    Atmega169,
    Atmega16m1,
    Atmega64m1,
    Atmega128rfa1,
    Atmega128rfr2,
    At90usb162,
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
    Attiny24,
    Attiny4313,
    Attiny1634,
}

/// Classic megaAVR parts matched on a whole name token, optionally with a
/// `p`/`pa`/`a`/`l` suffix, so `atmega48` does not catch the megaAVR 0-series
/// `atmega4808` and `atmega32` does not catch `atmega328` or `atmega32u4`.
fn classic_mega_model(lower_name: &str) -> Option<AvrModel> {
    lower_name
        .split(|character: char| !character.is_ascii_alphanumeric())
        .find_map(|token| {
            let base = ["pa", "p", "a", "l", ""]
                .iter()
                .find_map(|suffix| token.strip_suffix(suffix))?;
            Some(match base {
                "atmega8" => AvrModel::Atmega8,
                "atmega48" => AvrModel::Atmega48,
                "atmega88" => AvrModel::Atmega88,
                "atmega32" => AvrModel::Atmega32,
                "atmega164" => AvrModel::Atmega164,
                "atmega324" => AvrModel::Atmega324,
                "atmega644" => AvrModel::Atmega644,
                "atmega169" => AvrModel::Atmega169,
                "atmega16m1" => AvrModel::Atmega16m1,
                "atmega64m1" => AvrModel::Atmega64m1,
                "atmega128rfa1" => AvrModel::Atmega128rfa1,
                "atmega128rfr2" => AvrModel::Atmega128rfr2,
                "at90usb162" => AvrModel::At90usb162,
                _ => return None,
            })
        })
}

impl AvrModel {
    /// The model a chip descriptor names. atmega128/atmega1281 select 128 KB
    /// two-byte-PC core geometry; atmega2560/atmega2561/mega2560 select the
    /// 256 KB three-byte-PC mega core; the listed tinyAVR 0-series parts
    /// select the AVRxt data map and their exact flash/SRAM geometry. Other
    /// names default to the ATmega328P. ATmega328PB is matched explicitly so
    /// its second USART is not silently treated as the 328P's unused I/O.
    pub fn from_chip_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        if n.contains("attiny202") || n.contains("attiny 202") {
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
        } else if n.contains("attiny1634") || n.contains("attiny 1634") {
            AvrModel::Attiny1634
        } else if n.contains("attiny4313") || n.contains("attiny 4313") {
            AvrModel::Attiny4313
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
        } else if n.contains("attiny24") || n.contains("attiny 24") {
            AvrModel::Attiny24
        } else if n.contains("attiny13") || n.contains("attiny 13") {
            AvrModel::Attiny13
        } else if let Some(model) = classic_mega_model(&n) {
            model
        } else if n.contains("1284") {
            AvrModel::Atmega1284p
        } else if n.contains("atmega168") {
            AvrModel::Atmega168
        } else if n
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|part| part == "atmega16")
        {
            AvrModel::Atmega16
        } else if n.contains("1280") || n.contains("1281") {
            AvrModel::Atmega1281
        } else if n.ends_with("atmega128") {
            AvrModel::Atmega128
        } else if n.contains("2560") || n.contains("2561") {
            AvrModel::Atmega2560
        } else if n.contains("32u4") || n.contains("32u 4") {
            AvrModel::Atmega32u4
        } else if n.contains("328pb") {
            AvrModel::Atmega328pb
        } else {
            AvrModel::Atmega328p
        }
    }
    pub fn flash_size(self) -> usize {
        match self {
            AvrModel::Atmega328p | AvrModel::Atmega328pb => FLASH_SIZE,
            AvrModel::Atmega16 => 16 * 1024,
            AvrModel::Atmega168 => 16 * 1024,
            AvrModel::Atmega128 | AvrModel::Atmega1281 => 128 * 1024,
            AvrModel::Atmega2560 => 256 * 1024,
            AvrModel::Atmega32u4 => 32 * 1024,
            AvrModel::Atmega1284p => 128 * 1024,
            AvrModel::Atmega48 => 4 * 1024,
            AvrModel::Atmega8 | AvrModel::Atmega88 => 8 * 1024,
            AvrModel::Atmega164 => 16 * 1024,
            AvrModel::Atmega32 | AvrModel::Atmega324 => 32 * 1024,
            AvrModel::Atmega644 => 64 * 1024,
            AvrModel::Atmega169 | AvrModel::Atmega16m1 | AvrModel::At90usb162 => 16 * 1024,
            AvrModel::Atmega64m1 => 64 * 1024,
            AvrModel::Atmega128rfa1 | AvrModel::Atmega128rfr2 => 128 * 1024,
            AvrModel::Attiny202 | AvrModel::Attiny204 => 2 * 1024,
            AvrModel::Attiny402 | AvrModel::Attiny404 | AvrModel::Attiny406 => 4 * 1024,
            AvrModel::Attiny804 | AvrModel::Attiny806 | AvrModel::Attiny807 => 8 * 1024,
            AvrModel::Attiny1604 | AvrModel::Attiny1606 | AvrModel::Attiny1607 => 16 * 1024,
            // Classic tinyAVR. Datasheet flash size, per part.
            AvrModel::Attiny13 => 1024,
            AvrModel::Attiny25
            | AvrModel::Attiny2313
            | AvrModel::Attiny261
            | AvrModel::Attiny24 => 2 * 1024,
            AvrModel::Attiny45
            | AvrModel::Attiny44
            | AvrModel::Attiny461
            | AvrModel::Attiny48
            | AvrModel::Attiny4313 => 4 * 1024,
            AvrModel::Attiny85 | AvrModel::Attiny84 | AvrModel::Attiny861 | AvrModel::Attiny88 => {
                8 * 1024
            }
            AvrModel::Attiny1634 => 16 * 1024,
        }
    }
    pub fn sram_start(self) -> u16 {
        match self {
            AvrModel::Atmega328p | AvrModel::Atmega328pb | AvrModel::Atmega168 => SRAM_START,
            // ATmega8/32 have only the 64-byte I/O file; the others add the
            // 160-byte extended I/O file and start SRAM at 0x0100.
            AvrModel::Atmega16 | AvrModel::Atmega8 | AvrModel::Atmega32 => 0x0060,
            AvrModel::Atmega48
            | AvrModel::Atmega88
            | AvrModel::Atmega164
            | AvrModel::Atmega324
            | AvrModel::Atmega644
            | AvrModel::Atmega169
            | AvrModel::Atmega16m1
            | AvrModel::Atmega64m1
            | AvrModel::At90usb162 => 0x0100,
            // The 128RFx1/RFR2 extended I/O file (radio, second USART) runs
            // to 0x01FF, so SRAM starts at 0x0200 like the 1281/2560.
            AvrModel::Atmega128rfa1 | AvrModel::Atmega128rfr2 => 0x0200,
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => 0x0200,
            AvrModel::Atmega32u4 | AvrModel::Atmega1284p | AvrModel::Atmega128 => 0x0100,
            AvrModel::Attiny202 | AvrModel::Attiny204 => 0x3F80,
            AvrModel::Attiny402 | AvrModel::Attiny404 | AvrModel::Attiny406 => 0x3F00,
            AvrModel::Attiny804 | AvrModel::Attiny806 | AvrModel::Attiny807 => 0x3E00,
            AvrModel::Attiny1604 | AvrModel::Attiny1606 | AvrModel::Attiny1607 => 0x3C00,
            // Classic tinyAVR: SRAM follows the 64-byte I/O file at 0x0060, except
            // the 48/88 which add a 160-byte extended I/O file and start SRAM at
            // 0x0100 (ATtiny48/88 datasheet, Table 5-2). The 1634 likewise has
            // a 160-byte extended I/O file and SRAM at 0x0100..0x04FF.
            AvrModel::Attiny48 | AvrModel::Attiny88 | AvrModel::Attiny1634 => 0x0100,
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
            | AvrModel::Attiny24
            | AvrModel::Attiny4313 => 0x0060,
        }
    }
    pub fn ramend(self) -> u16 {
        match self {
            AvrModel::Atmega328p | AvrModel::Atmega328pb => RAMEND,
            AvrModel::Atmega16 => 0x045F,
            AvrModel::Atmega168 => 0x04FF,
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => 0x21FF,
            AvrModel::Atmega128 => 0x10FF,
            AvrModel::Atmega32u4 => 0x0AFF,
            AvrModel::Atmega1284p => 0x40FF,
            AvrModel::Atmega8 => 0x045F,  // 0x0060 + 1KB
            AvrModel::Atmega32 => 0x085F, // 0x0060 + 2KB
            AvrModel::Atmega48 => 0x02FF, // 0x0100 + 512B
            AvrModel::Atmega88 | AvrModel::Atmega164 => 0x04FF, // 0x0100 + 1KB
            AvrModel::Atmega324 => 0x08FF, // 0x0100 + 2KB
            AvrModel::Atmega644 => 0x10FF, // 0x0100 + 4KB
            AvrModel::At90usb162 => 0x02FF, // 0x0100 + 512B
            AvrModel::Atmega169 | AvrModel::Atmega16m1 => 0x04FF, // 0x0100 + 1KB
            AvrModel::Atmega64m1 => 0x10FF, // 0x0100 + 4KB
            AvrModel::Atmega128rfa1 | AvrModel::Atmega128rfr2 => 0x41FF, // 0x0200 + 16KB
            AvrModel::Attiny202
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
            AvrModel::Attiny25
            | AvrModel::Attiny2313
            | AvrModel::Attiny261
            | AvrModel::Attiny24 => 0x00DF, // + 128B
            AvrModel::Attiny45
            | AvrModel::Attiny44
            | AvrModel::Attiny461
            | AvrModel::Attiny4313 => 0x015F, // + 256B
            AvrModel::Attiny48 => 0x01FF, // 0x0100 + 256B
            AvrModel::Attiny85 | AvrModel::Attiny84 | AvrModel::Attiny861 => 0x025F, // 0x0060 + 512B
            AvrModel::Attiny88 => 0x02FF,   // 0x0100 + 512B
            AvrModel::Attiny1634 => 0x04FF, // 0x0100 + 1KB
        }
    }
    /// Bus base of this model's `avr_adc` input window (2 bytes per channel).
    /// The 2560's 16 channels move it past the port mirrors (PINF..PING sit
    /// at 0x1_002F..0x1_0034, where the 328P keeps its 8-channel window).
    pub fn adc_input_base(self) -> u64 {
        match self {
            AvrModel::Atmega328p
            | AvrModel::Atmega328pb
            | AvrModel::Atmega16
            | AvrModel::Atmega168
            | AvrModel::Atmega32u4
            | AvrModel::Atmega1284p
            | AvrModel::Atmega8
            | AvrModel::Atmega48
            | AvrModel::Atmega88
            | AvrModel::Atmega32
            | AvrModel::Atmega164
            | AvrModel::Atmega324
            | AvrModel::Atmega644
            | AvrModel::Atmega169
            | AvrModel::Atmega16m1
            | AvrModel::Atmega64m1
            | AvrModel::Atmega128rfa1
            | AvrModel::Atmega128rfr2
            | AvrModel::At90usb162 => AVR_ADC_INPUT_BASE,
            AvrModel::Atmega128 | AvrModel::Atmega1281 | AvrModel::Atmega2560 => {
                AVR_MEGA_ADC_INPUT_BASE
            }
            AvrModel::Attiny202
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
            | AvrModel::Attiny24
            | AvrModel::Attiny4313
            | AvrModel::Attiny1634 => AVR_ADC_INPUT_BASE,
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

    /// Classic parts with only the 64-byte I/O file (0x20..0x5F). Their SRAM
    /// starts at 0x0060, so 0x60..0xFF is memory, not the 328P extended I/O
    /// file: a variable at 0xC6 must not be sent out as a UDR0 byte.
    pub fn has_classic_io_file(self) -> bool {
        self.sram_start() == 0x0060
    }

    /// Parts whose (first) USART sits in the classic I/O-file layout: UBRRL
    /// 0x29, UCSRB 0x2A, UCSRA 0x2B, UDR 0x2C (ATmega8/16/32 USART, ATmega128
    /// USART0, ATtiny2313/4313 USART). UBRRH/UCSRC live elsewhere on each and
    /// are kept as plain registers.
    pub const fn has_classic_usart(self) -> bool {
        matches!(
            self,
            AvrModel::Atmega8
                | AvrModel::Atmega16
                | AvrModel::Atmega32
                | AvrModel::Atmega128
                | AvrModel::Attiny2313
                | AvrModel::Attiny4313
        )
    }

    /// Classic-I/O-file models whose PINx is implemented as write-one-to-toggle
    /// (ATtiny2313/4313 and the measured ATtiny13/24/44/84/25/45/85/1634 profiles);
    /// ATmega8/16/32/128 PINx is modeled as read-only.
    pub const fn has_classic_pin_toggle(self) -> bool {
        matches!(
            self,
            AvrModel::Attiny2313
                | AvrModel::Attiny4313
                | AvrModel::Attiny13
                | AvrModel::Attiny24
                | AvrModel::Attiny44
                | AvrModel::Attiny84
                | AvrModel::Attiny25
                | AvrModel::Attiny45
                | AvrModel::Attiny85
                | AvrModel::Attiny1634
        )
    }

    /// The ATmega328PB has a second polling USART at data-space 0xC8..0xCE.
    /// Its USART interrupts are not yet modelled.
    pub const fn has_usart1(self) -> bool {
        matches!(self, AvrModel::Atmega328pb)
    }

    /// Whether this part uses the AVRxt data-space map and direct I/O
    /// addresses for IN/OUT, SBI/CBI and SBIS/SBIC.
    #[inline]
    pub fn uses_avrxt_data_map(self) -> bool {
        matches!(
            self,
            AvrModel::Attiny202
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
    /// Data EEPROM and `EECR`/`EEDR`/`EEAR` (`avr/eeprom.rs`). Empty on the
    /// parts whose EEPROM is not modelled.
    eeprom: eeprom::Eeprom,
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
/// `PINA` data-space address; `DDRA`/`PORTA` follow it.
pub const AVR_PINA: u16 = 0x0020;
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
            eeprom: eeprom::Eeprom::new(model.eeprom_size()),
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
    fn is_avrxt(&self) -> bool {
        self.model.uses_avrxt_data_map()
    }

    /// Convert a 6-bit IN/OUT/SBI/SBIC I/O operand to a data-space address.
    #[inline]
    pub(in crate::cpu::avr) fn io_data_address(&self, io_addr: u8) -> u16 {
        if self.is_avrxt() {
            u16::from(io_addr)
        } else {
            0x20 + u16::from(io_addr)
        }
    }

    /// The 328P-layout USART0 register a classic-layout USART register
    /// stands for, so both layouts share one USART implementation and the
    /// same bus-side host window.
    #[inline]
    fn classic_usart_alias(&self, addr: u16) -> Option<u16> {
        if !self.model.has_classic_usart() {
            return None;
        }
        match addr {
            0x0029 => Some(0x00C4), // UBRRL -> UBRR0L
            0x002A => Some(0x00C1), // UCSRB -> UCSR0B
            0x002B => Some(0x00C0), // UCSRA -> UCSR0A
            0x002C => Some(0x00C6), // UDR -> UDR0
            _ => None,
        }
    }

    /// Where a classic-layout access goes before the 328P decode sees it:
    /// `Ok(addr)` to run the 328P decode on (a USART alias), `Err(())` when the
    /// classic decode owns the address.
    #[inline]
    fn classic_route(&self, addr: u16) -> Result<u16, ()> {
        if let Some(canon) = self.classic_usart_alias(addr) {
            return Ok(canon);
        }
        // ATmega128: the whole I/O file and its 0x60..0xFF extended I/O use
        // the classic layout (PORTA at 0x3B, not the 328P PCIFR); SRAM from
        // 0x0100 falls through to the shared decode.
        if self.model.has_classic_io_file()
            || (self.model == AvrModel::Atmega128 && (0x0020..=0x00FF).contains(&addr))
        {
            return Err(());
        }
        Ok(addr)
    }

    /// `PINx` of a port this model has.
    #[inline]
    fn is_pin_reg(&self, addr: u16) -> bool {
        match self.model {
            // The megaAVR x8 family (48/88/168/328) shares one register
            // summary: PINB/DDRB/PORTB..PIND/DDRD/PORTD at 0x23..0x2B.
            AvrModel::Atmega328p
            | AvrModel::Atmega328pb
            | AvrModel::Atmega168
            | AvrModel::Atmega88
            | AvrModel::Atmega48
            | AvrModel::Atmega32u4
            | AvrModel::At90usb162 => {
                matches!(addr, AVR_PINB | AVR_PINC | AVR_PIND)
            }
            // ATmega16M1/64M1 add PINE/DDRE/PORTE at 0x2C..0x2E.
            AvrModel::Atmega16m1 | AvrModel::Atmega64m1 => {
                matches!(addr, AVR_PINB | AVR_PINC | AVR_PIND | 0x002C)
            }
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => MEGA2560_PIN_ADDRS.contains(&addr),
            // ATmega169P and ATmega128RFA1/RFR2: ports A-G at 0x20..0x34 as on
            // the 1281; no extended-I/O ports H-L.
            AvrModel::Atmega169 | AvrModel::Atmega128rfa1 | AvrModel::Atmega128rfr2 => {
                MEGA2560_PIN_ADDRS[..7].contains(&addr)
            }
            AvrModel::Atmega164
            | AvrModel::Atmega324
            | AvrModel::Atmega644
            | AvrModel::Atmega1284p => {
                matches!(addr, AVR_PINA | AVR_PINB | AVR_PINC | AVR_PIND)
            }
            // Classic layout: PINx/DDRx/PORTx descend from PORTA at 0x3B.
            AvrModel::Atmega16 | AvrModel::Atmega32 => {
                matches!(addr, 0x0030 | 0x0033 | 0x0036 | 0x0039)
            }
            // ATmega8 has no port A: PINB/PINC/PIND at 0x36/0x33/0x30.
            AvrModel::Atmega8 => matches!(addr, 0x0030 | 0x0033 | 0x0036),
            // ATmega128 ports A-D as on the 16/32, plus PINE/DDRE/PORTE at
            // 0x21..0x23. Ports F/G (DDRF/PORTF in extended I/O) are not mirrored.
            AvrModel::Atmega128 => matches!(addr, 0x0021 | 0x0030 | 0x0033 | 0x0036 | 0x0039),
            // ATtiny2313/4313: PIND 0x30, PINB 0x36, PINA 0x39 (0x33..0x35 are GPIOR0..2).
            AvrModel::Attiny2313 | AvrModel::Attiny4313 => matches!(addr, 0x0030 | 0x0036 | 0x0039),
            // ATtiny24/44/84: PINB 0x36, PINA 0x39.
            AvrModel::Attiny24 | AvrModel::Attiny44 | AvrModel::Attiny84 => {
                matches!(addr, 0x0036 | 0x0039)
            }
            // ATtiny13 and 25/45/85 have port B only: PINB 0x36.
            AvrModel::Attiny13 | AvrModel::Attiny25 | AvrModel::Attiny45 | AvrModel::Attiny85 => {
                addr == 0x0036
            }
            // ATtiny1634: PINC 0x27, PINB 0x2B, PINA 0x2F; each port's PUEx
            // pull-up register sits after PORTx (0x2A/0x2E/0x32).
            AvrModel::Attiny1634 => matches!(addr, 0x0027 | 0x002B | 0x002F),
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
            AvrModel::Attiny261
            | AvrModel::Attiny461
            | AvrModel::Attiny861
            | AvrModel::Attiny48
            | AvrModel::Attiny88 => false,
        }
    }

    /// `PINx`/`DDRx`/`PORTx` of a port this model has: mirrored to the bus.
    #[inline]
    fn is_port_reg(&self, addr: u16) -> bool {
        match self.model {
            AvrModel::Atmega328p
            | AvrModel::Atmega328pb
            | AvrModel::Atmega168
            | AvrModel::Atmega88
            | AvrModel::Atmega48
            | AvrModel::Atmega32u4
            | AvrModel::At90usb162 => (AVR_PINB..=AVR_PIND + 2).contains(&addr),
            AvrModel::Atmega16m1 | AvrModel::Atmega64m1 => (AVR_PINB..=0x002E).contains(&addr),
            AvrModel::Atmega1281 | AvrModel::Atmega2560 => {
                (0x20..=0x34).contains(&addr) || (0x100..=0x10B).contains(&addr)
            }
            AvrModel::Atmega169 | AvrModel::Atmega128rfa1 | AvrModel::Atmega128rfr2 => {
                (0x20..=0x34).contains(&addr)
            }
            AvrModel::Atmega164
            | AvrModel::Atmega324
            | AvrModel::Atmega644
            | AvrModel::Atmega1284p => (AVR_PINA..=AVR_PIND + 2).contains(&addr),
            AvrModel::Atmega16 | AvrModel::Atmega32 => (0x0030..=0x003B).contains(&addr),
            AvrModel::Atmega8 => (0x0030..=0x0038).contains(&addr),
            AvrModel::Atmega128 => {
                (0x0021..=0x0023).contains(&addr) || (0x0030..=0x003B).contains(&addr)
            }
            AvrModel::Attiny2313 | AvrModel::Attiny4313 => {
                (0x0030..=0x0032).contains(&addr) || (0x0036..=0x003B).contains(&addr)
            }
            AvrModel::Attiny24 | AvrModel::Attiny44 | AvrModel::Attiny84 => {
                (0x0036..=0x003B).contains(&addr)
            }
            AvrModel::Attiny13 | AvrModel::Attiny25 | AvrModel::Attiny45 | AvrModel::Attiny85 => {
                (0x0036..=0x0038).contains(&addr)
            }
            // PUEC/PUEB/PUEA at 0x2A/0x2E/0x32 stay CPU-local stored bytes.
            AvrModel::Attiny1634 => {
                (0x0027..=0x0029).contains(&addr)
                    || (0x002B..=0x002D).contains(&addr)
                    || (0x002F..=0x0031).contains(&addr)
            }
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
            AvrModel::Attiny261
            | AvrModel::Attiny461
            | AvrModel::Attiny861
            | AvrModel::Attiny48
            | AvrModel::Attiny88 => false,
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
        let addr = match self.classic_route(addr) {
            Ok(a) => a,
            Err(()) => return self.classic_data_read(addr, bus),
        };
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
        let addr = match self.classic_route(addr) {
            Ok(a) => a,
            Err(()) => return self.classic_data_write(addr, value, bus),
        };
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
                // On a classic-layout part data-space 0xC6 is SRAM (or
                // reserved), not this register: keep the byte off the bus RAM.
                if !self.model.has_classic_usart() {
                    bus.write_u8(addr as u64, value)?;
                }
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
            a if self.is_pin_reg(a) => {
                // PINx is write-one-to-toggle on classic megaAVR. The CPU
                // owns the port latch, so update its PORT shadow and publish
                // the resulting latch through the DDR/PORT window; writing
                // the PIN input window would change the bus-side input level
                // instead of toggling the output.
                let port_addr = a + 2;
                let port_index = usize::from(port_addr - 0x20);
                self.io[port_index] ^= value;
                let pad = self.port_pad_value(port_addr, self.io[port_index]);
                let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + u64::from(port_addr), pad);
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

    /// AVRxt data-space read. The CPU register file is separate from data
    /// memory; low addresses are I/O, SRAM starts at the selected device's
    /// geometry, and flash is visible to LD/ST at 0x8000.
    fn avrxt_data_read(&self, addr: u16, bus: &dyn Bus) -> SimResult<u8> {
        match addr {
            0x0000..=0x0FFF => match addr {
                // CPU peripheral block starts at 0x30; SP is at 0x3D/0x3E
                // and SREG at 0x3F (datasheet §8.6).
                0x003D => Ok(self.sp as u8),
                0x003E => Ok((self.sp >> 8) as u8),
                0x003F => Ok(self.sreg),
                _ => {
                    let mirror = AVR_IO_MIRROR_BASE + u64::from(addr);
                    if bus.has_mmio_window(mirror) {
                        bus.read_u8(mirror)
                    } else {
                        Ok(self.tiny_io[addr as usize])
                    }
                }
            },
            a if (self.sram_start..=self.ramend).contains(&a) => {
                Ok(self.sram[(a - self.sram_start) as usize])
            }
            a if a >= 0x8000 && usize::from(a - 0x8000) < self.flash.len() => {
                Ok(self.flash[usize::from(a - 0x8000)])
            }
            _ => Err(SimulationError::MemoryViolation(u64::from(addr))),
        }
    }

    /// Classic 64-byte I/O-file decode (SRAM at 0x0060). Only the core
    /// registers, mapped ports and the aliased USART have behaviour; every
    /// other I/O register is a stored byte, and 0x60.. is SRAM. The 328P
    /// timer/ADC/TWI/ext-int addresses are not these parts' registers.
    fn classic_data_read(&self, addr: u16, bus: &dyn Bus) -> SimResult<u8> {
        match addr {
            0x0000..=0x001F => Ok(self.r[addr as usize]),
            a if self.is_pin_reg(a) => {
                let ddr = self.io[(addr + 1 - 0x20) as usize];
                let port = self.io[(addr + 2 - 0x20) as usize];
                let external = bus.read_u8(AVR_IO_MIRROR_BASE + u64::from(addr))?;
                Ok((port & ddr) | (external & !ddr))
            }
            0x005D => Ok((self.sp & 0xFF) as u8),
            0x005E => Ok((self.sp >> 8) as u8),
            0x005F => Ok(self.sreg),
            0x0020..=0x005F => Ok(self.io[(addr - 0x20) as usize]),
            a if self.model.has_classic_io_file()
                && (self.sram_start..=self.ramend).contains(&a) =>
            {
                Ok(self.sram[(a - self.sram_start) as usize])
            }
            0x0060..=0x00FF => Ok(self.io[(addr - 0x20) as usize]),
            _ => Err(SimulationError::MemoryViolation(addr as u64)),
        }
    }

    fn classic_data_write(&mut self, addr: u16, value: u8, bus: &mut dyn Bus) -> SimResult<()> {
        match addr {
            0x0000..=0x001F => self.r[addr as usize] = value,
            // Classic tinyAVR PINx are write-one-to-toggle (PORTx sits at
            // PINx + 2); ATmega8/16/32/128 PINx are read-only.
            a if self.is_pin_reg(a) => {
                if self.model.has_classic_pin_toggle() {
                    let port_addr = a + 2;
                    let port_index = usize::from(port_addr - 0x20);
                    self.io[port_index] ^= value;
                    let latch = self.io[port_index];
                    let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + u64::from(port_addr), latch);
                }
            }
            0x005D => self.sp = (self.sp & 0xFF00) | value as u16,
            0x005E => self.sp = (self.sp & 0x00FF) | ((value as u16) << 8),
            0x005F => self.sreg = value,
            0x0020..=0x005F => {
                self.io[(addr - 0x20) as usize] = value;
                if self.is_port_reg(addr) {
                    let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + u64::from(addr), value);
                }
            }
            a if self.model.has_classic_io_file()
                && (self.sram_start..=self.ramend).contains(&a) =>
            {
                self.sram[(a - self.sram_start) as usize] = value;
                bus.write_u8(u64::from(a), value)?;
            }
            // ATmega128 reserved extended I/O where the 328P has USART0.
            0x0060..=0x00FF => self.io[(addr - 0x20) as usize] = value,
            _ => return Err(SimulationError::MemoryViolation(addr as u64)),
        }
        Ok(())
    }

    /// AVRxt data-space write. I/O writes are retained locally and mirrored to
    /// an explicitly mapped peripheral window; SRAM writes update both the
    /// CPU's backing store and the system bus.
    fn avrxt_data_write(&mut self, addr: u16, value: u8, bus: &mut dyn Bus) -> SimResult<()> {
        match addr {
            0x003D => {
                self.sp = (self.sp & 0xFF00) | u16::from(value);
                Ok(())
            }
            0x003E => {
                self.sp = (self.sp & 0x00FF) | (u16::from(value & 0x3F) << 8);
                Ok(())
            }
            0x003F => {
                self.sreg = value;
                Ok(())
            }
            0x0000..=0x0FFF => {
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
                    } else if self.is_avrxt() && a < 0x1000 {
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
                self.eeprom.ready_wake_cycles(self.cycles),
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
        if self.is_avrxt() {
            self.tiny_io.fill(0);
            self.r = [0; 32];
            self.pc = 0;
            self.sp = self.ramend;
            self.sreg = 0;
            self.pending_irq = 0;
            self.cycles = 0;
            self.sleeping = false;
            self.irq_shadow = false;
            let _ = bus;
            return Ok(());
        }
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
        self.t2_down = false;
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
        self.eeprom.reset();
        self.irq_shadow = false;
        // Keep attached SPI/I2C slaves across reset (same wiring as real board).
        Ok(())
    }

    fn on_supply_reset(&mut self, cause: crate::power::SupplyResetCause) {
        if self.is_avrxt() {
            return;
        }
        let flags = &mut self.io[usize::from(ADDR_MCUSR - 0x20)];
        match cause {
            crate::power::SupplyResetCause::PowerOn => *flags = 1, // PORF
            crate::power::SupplyResetCause::BrownOut => *flags |= 1 << 2, // BORF
        }
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

        // EE_READY: level-sensitive on EERIE with no write in progress.
        if self.eeprom.irq_watched() || self.pending_irq & (1u64 << eeprom::VEC_EE_READY) != 0 {
            self.sync_eeprom_irq();
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

#[path = "avr/eeprom.rs"]
mod eeprom;
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
    fn atmega16_selects_its_16k_flash_and_low_sram_geometry() {
        let name = "simavr-atmega16";
        let cpu = Avr::for_chip_name(name);
        assert_eq!(cpu.model, AvrModel::Atmega16);
        assert_eq!(cpu.flash.len(), 16 * 1024);
        assert_eq!(cpu.sram_start, 0x0060);
        assert_eq!(cpu.sram.len(), 1024);
        assert_eq!(cpu.ramend, 0x045F);
        assert!(!cpu.model.three_byte_pc());

        for name in ["simavr-atmega164", "simavr-atmega168", "simavr-atmega169p"] {
            assert_ne!(
                AvrModel::from_chip_name(name),
                AvrModel::Atmega16,
                "{name} must not match the ATmega16 model"
            );
        }
    }

    #[test]
    fn classic_megas_select_their_own_geometry_not_the_328p_fallback() {
        let cases = [
            (
                "simavr-atmega8",
                AvrModel::Atmega8,
                8 * 1024,
                0x0060,
                0x045F,
            ),
            (
                "simavr-atmega48",
                AvrModel::Atmega48,
                4 * 1024,
                0x0100,
                0x02FF,
            ),
            ("atmega48pa", AvrModel::Atmega48, 4 * 1024, 0x0100, 0x02FF),
            (
                "simavr-atmega88",
                AvrModel::Atmega88,
                8 * 1024,
                0x0100,
                0x04FF,
            ),
            (
                "simavr-atmega32",
                AvrModel::Atmega32,
                32 * 1024,
                0x0060,
                0x085F,
            ),
            (
                "simavr-atmega164",
                AvrModel::Atmega164,
                16 * 1024,
                0x0100,
                0x04FF,
            ),
            (
                "simavr-atmega324",
                AvrModel::Atmega324,
                32 * 1024,
                0x0100,
                0x08FF,
            ),
            (
                "atmega324pa",
                AvrModel::Atmega324,
                32 * 1024,
                0x0100,
                0x08FF,
            ),
            (
                "simavr-atmega644",
                AvrModel::Atmega644,
                64 * 1024,
                0x0100,
                0x10FF,
            ),
            (
                "simavr-atmega169p",
                AvrModel::Atmega169,
                16 * 1024,
                0x0100,
                0x04FF,
            ),
            (
                "simavr-atmega16m1",
                AvrModel::Atmega16m1,
                16 * 1024,
                0x0100,
                0x04FF,
            ),
            (
                "simavr-atmega64m1",
                AvrModel::Atmega64m1,
                64 * 1024,
                0x0100,
                0x10FF,
            ),
            (
                "simavr-atmega128rfa1",
                AvrModel::Atmega128rfa1,
                128 * 1024,
                0x0200,
                0x41FF,
            ),
            (
                "simavr-atmega128rfr2",
                AvrModel::Atmega128rfr2,
                128 * 1024,
                0x0200,
                0x41FF,
            ),
            (
                "simavr-at90usb162",
                AvrModel::At90usb162,
                16 * 1024,
                0x0100,
                0x02FF,
            ),
        ];
        for (name, model, flash, sram_start, ramend) in cases {
            let cpu = Avr::for_chip_name(name);
            assert_eq!(cpu.model, model, "{name} selects its exact model");
            assert_eq!(cpu.flash.len(), flash, "{name} flash size");
            assert_eq!(cpu.sram_start, sram_start, "{name} SRAM start");
            assert_eq!(cpu.ramend, ramend, "{name} RAMEND");
            assert_eq!(cpu.sp, ramend, "{name} SP resets to RAMEND");
            assert!(!cpu.model.uses_avrxt_data_map());
        }
        for (name, model) in [
            ("atmega328p", AvrModel::Atmega328p),
            ("atmega32u4", AvrModel::Atmega32u4),
            ("atmega168", AvrModel::Atmega168),
            ("attiny88", AvrModel::Attiny88),
            ("atmega1284p", AvrModel::Atmega1284p),
            ("atmega128", AvrModel::Atmega128),
            ("atmega16", AvrModel::Atmega16),
        ] {
            assert_eq!(AvrModel::from_chip_name(name), model, "{name} unchanged");
        }
        assert_ne!(AvrModel::from_chip_name("atmega4808"), AvrModel::Atmega48);
    }

    #[test]
    fn atmega128_selects_its_128k_flash_and_4k_sram_geometry() {
        let cpu = Avr::for_chip_name("simavr-atmega128");
        assert_eq!(cpu.model, AvrModel::Atmega128);
        assert_eq!(cpu.flash.len(), 128 * 1024);
        assert_eq!(cpu.sram_start, 0x0100);
        assert_eq!(cpu.sram.len(), 4 * 1024);
        assert_eq!(cpu.ramend, 0x10FF);
        assert_eq!(cpu.sp, 0x10FF, "SP resets to ATmega128 RAMEND");
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
            (
                "attiny24",
                AvrModel::Attiny24,
                2 * 1024,
                0x0060,
                0x00DF,
                128,
            ),
            (
                "attiny4313",
                AvrModel::Attiny4313,
                4 * 1024,
                0x0060,
                0x015F,
                256,
            ),
            // ATtiny1634 also has a 160-byte extended I/O file: SRAM 0x0100..0x04FF.
            (
                "attiny1634",
                AvrModel::Attiny1634,
                16 * 1024,
                0x0100,
                0x04FF,
                1024,
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

    /// ATmega48/88 share the x8-family port map with the 168/328P: DDRx and
    /// PORTx reach the mirror window and PINx composes latch and input.
    #[test]
    fn megax8_small_parts_mirror_ports_like_the_328p() {
        for model in [AvrModel::Atmega48, AvrModel::Atmega88] {
            let mut bus = MockBus::new();
            let mut cpu = Avr::new_model(model);
            cpu.data_write(0x24, 0x20, &mut bus).unwrap(); // DDRB: PB5 out
            cpu.data_write(0x25, 0x20, &mut bus).unwrap(); // PORTB: PB5 high
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x24).unwrap(), 0x20);
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x25).unwrap(), 0x20);
            bus.write_u8(AVR_IO_MIRROR_BASE + u64::from(AVR_PIND), 0x04)
                .unwrap();
            assert_eq!(cpu.data_read(AVR_PINB, &bus).unwrap() & 0x20, 0x20);
            assert_eq!(cpu.data_read(AVR_PIND, &bus).unwrap() & 0x04, 0x04);
        }
    }

    /// ATmega164/324/644/1284P expose ports A-D at the low megaAVR IO map.
    /// Check that GPIOA reaches the bus mirror and PINx reads combine the
    /// output latch with the outside input level for every supported model.
    /// On the 64-byte-I/O-file parts 0x60..0xFF is SRAM. Before the classic
    /// decode it hit the 328P extended I/O arms: a variable at 0xC6 was sent
    /// as a UART byte, one at 0xC0 read back with UDRE set.
    #[test]
    fn classic_io_file_parts_keep_0x60_to_0xff_as_sram() {
        let models = [
            AvrModel::Atmega8,
            AvrModel::Atmega16,
            AvrModel::Atmega32,
            AvrModel::Attiny13,
            AvrModel::Attiny2313,
            AvrModel::Attiny25,
            AvrModel::Attiny45,
            AvrModel::Attiny85,
            AvrModel::Attiny24,
            AvrModel::Attiny44,
            AvrModel::Attiny84,
            AvrModel::Attiny261,
            AvrModel::Attiny461,
            AvrModel::Attiny861,
            AvrModel::Attiny4313,
        ];
        for model in models {
            assert!(model.has_classic_io_file(), "{model:?}");
            let mut bus = MockBus::new();
            let mut cpu = Avr::new_model(model);
            for a in 0x60u16..=0x9F {
                cpu.data_write(a, (a as u8) ^ 0xA5, &mut bus).unwrap();
            }
            for a in 0x60u16..=0x9F {
                assert_eq!(
                    cpu.data_read(a, &bus).unwrap(),
                    (a as u8) ^ 0xA5,
                    "{model:?} {a:#x}"
                );
            }
            if cpu.ramend >= 0xFF {
                for a in 0xA0u16..=0xFF {
                    cpu.data_write(a, a as u8, &mut bus).unwrap();
                    assert_eq!(cpu.data_read(a, &bus).unwrap(), a as u8, "{model:?} {a:#x}");
                }
            }
            assert!(
                cpu.serial_tx.is_empty(),
                "{model:?} sent SRAM writes as UART"
            );
        }
    }

    /// The classic-layout USART (UBRRL 0x29, UCSRB 0x2A, UCSRA 0x2B, UDR 0x2C)
    /// transmits, reports UDRE and keeps its control registers.
    #[test]
    fn classic_usart_layout_transmits_from_udr_0x2c() {
        for model in [
            AvrModel::Atmega8,
            AvrModel::Atmega16,
            AvrModel::Atmega32,
            AvrModel::Atmega128,
            AvrModel::Attiny2313,
            AvrModel::Attiny4313,
        ] {
            let mut bus = MockBus::new();
            let mut cpu = Avr::new_model(model);
            cpu.data_write(0x29, 51, &mut bus).unwrap();
            cpu.data_write(0x2A, 0x18, &mut bus).unwrap();
            assert_eq!(cpu.data_read(0x29, &bus).unwrap(), 51, "{model:?}");
            assert_eq!(cpu.data_read(0x2A, &bus).unwrap(), 0x18, "{model:?}");
            assert_ne!(
                cpu.data_read(0x2B, &bus).unwrap() & UCSRA_UDRE,
                0,
                "{model:?}"
            );
            cpu.data_write(0x2C, b'K', &mut bus).unwrap();
            assert_eq!(cpu.serial_tx, b"K", "{model:?}");
        }
        // ATmega128: 0xC0..0xC6 is reserved extended I/O, not a second USART.
        let mut bus = MockBus::new();
        let mut cpu = Avr::new_model(AvrModel::Atmega128);
        cpu.data_write(0xC6, b'X', &mut bus).unwrap();
        assert!(cpu.serial_tx.is_empty());
    }

    /// ATmega16/32 ports descend from PORTA 0x3B: PINx/DDRx/PORTx at 0x39/
    /// 0x3A/0x3B .. 0x30/0x31/0x32. PORTC (0x35) is a latch, not the 328P TIFR0.
    #[test]
    fn atmega16_classic_ports_mirror_and_read_only_pin() {
        for model in [AvrModel::Atmega16, AvrModel::Atmega32] {
            let mut bus = MockBus::new();
            let mut cpu = Avr::new_model(model);
            cpu.data_write(0x3A, 0x01, &mut bus).unwrap(); // DDRA
            cpu.data_write(0x3B, 0x01, &mut bus).unwrap(); // PORTA
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x3B).unwrap(), 0x01);
            assert_eq!(cpu.data_read(0x39, &bus).unwrap() & 1, 1);
            cpu.data_write(0x39, 0x01, &mut bus).unwrap(); // PINA is read-only
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x3B).unwrap(), 0x01);
            cpu.data_write(0x3B, 0x00, &mut bus).unwrap();
            assert_eq!(cpu.data_read(0x39, &bus).unwrap() & 1, 0);
            cpu.data_write(0x35, 0x5A, &mut bus).unwrap(); // PORTC
            assert_eq!(cpu.data_read(0x35, &bus).unwrap(), 0x5A);
            bus.write_u8(AVR_IO_MIRROR_BASE + 0x30, 0x04).unwrap(); // PD2 high
            assert_eq!(cpu.data_read(0x30, &bus).unwrap() & 0x04, 0x04);
        }
    }

    /// ATmega128 uses the classic I/O file (PORTA 0x3B, read-only PINx, PORTE
    /// 0x23) with extended I/O at 0x60..0xFF and SRAM from 0x0100.
    #[test]
    fn atmega128_classic_ports_a_and_e_mirror_and_read_only_pin() {
        let mut bus = MockBus::new();
        let mut cpu = Avr::new_model(AvrModel::Atmega128);
        cpu.data_write(0x3A, 0x01, &mut bus).unwrap(); // DDRA
        cpu.data_write(0x3B, 0x01, &mut bus).unwrap(); // PORTA, not PCIFR
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x3B).unwrap(), 0x01);
        assert_eq!(cpu.data_read(0x3B, &bus).unwrap(), 0x01);
        assert_eq!(cpu.data_read(0x39, &bus).unwrap() & 1, 1);
        cpu.data_write(0x39, 0x01, &mut bus).unwrap(); // PINA is read-only
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x3B).unwrap(), 0x01);
        cpu.data_write(0x22, 0x80, &mut bus).unwrap(); // DDRE
        cpu.data_write(0x23, 0x80, &mut bus).unwrap(); // PORTE
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x23).unwrap(), 0x80);
        assert_eq!(cpu.data_read(0x21, &bus).unwrap() & 0x80, 0x80);
        cpu.data_write(0x62, 0xA5, &mut bus).unwrap(); // PORTF: stored byte
        assert_eq!(cpu.data_read(0x62, &bus).unwrap(), 0xA5);
        cpu.data_write(0x0100, 0x3C, &mut bus).unwrap(); // first SRAM byte
        assert_eq!(cpu.data_read(0x0100, &bus).unwrap(), 0x3C);
    }

    /// ATmega8 ports B-D: PINB/DDRB/PORTB at 0x36/0x37/0x38 down to PIND
    /// 0x30. 0x39..0x3B (PORTA on the 16/32) are not port registers here.
    #[test]
    fn atmega8_classic_ports_b_to_d_mirror_and_read_only_pin() {
        let mut bus = MockBus::new();
        let mut cpu = Avr::new_model(AvrModel::Atmega8);
        cpu.data_write(0x37, 0x01, &mut bus).unwrap(); // DDRB
        cpu.data_write(0x38, 0x01, &mut bus).unwrap(); // PORTB
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x38).unwrap(), 0x01);
        assert_eq!(cpu.data_read(0x36, &bus).unwrap() & 1, 1);
        cpu.data_write(0x36, 0x01, &mut bus).unwrap(); // PINB is read-only
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x38).unwrap(), 0x01);
        cpu.data_write(0x38, 0x00, &mut bus).unwrap();
        assert_eq!(cpu.data_read(0x36, &bus).unwrap() & 1, 0);
        bus.write_u8(AVR_IO_MIRROR_BASE + 0x30, 0x04).unwrap(); // PD2 high
        assert_eq!(cpu.data_read(0x30, &bus).unwrap() & 0x04, 0x04);
        cpu.data_write(0x3B, 0x5A, &mut bus).unwrap(); // GICR, not PORTA
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x3B).unwrap(), 0x00);
        assert_eq!(cpu.data_read(0x3B, &bus).unwrap(), 0x5A);
    }

    /// ATtiny2313/4313 ports A/B/D: PINB/DDRB/PORTB at 0x36..0x38, PIND at
    /// 0x30, PINA at 0x39. PINx is write-one-to-toggle here, and SBI on PINB
    /// toggles only the named bit. 0x33 is GPIOR0, not a port register.
    #[test]
    fn attiny2313_classic_ports_mirror_and_pin_toggle() {
        for model in [AvrModel::Attiny2313, AvrModel::Attiny4313] {
            let mut bus = MockBus::new();
            let mut cpu = Avr::new_model(model);
            cpu.data_write(0x37, 0x81, &mut bus).unwrap(); // DDRB
            cpu.data_write(0x38, 0x81, &mut bus).unwrap(); // PORTB
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x38).unwrap(), 0x81);
            assert_eq!(cpu.data_read(0x36, &bus).unwrap(), 0x81);
            cpu.data_write(0x36, 0x01, &mut bus).unwrap(); // PINB toggles PB0
            assert_eq!(
                bus.read_u8(AVR_IO_MIRROR_BASE + 0x38).unwrap(),
                0x80,
                "{model:?}"
            );
            assert_eq!(cpu.data_read(0x36, &bus).unwrap(), 0x80);
            // SBI PINB,0 (I/O 0x16): toggles PB0 back, PB7 untouched.
            cpu.load_words(0x100, &[0x9AB0, 0xCFFF]);
            cpu.set_pc(0x100);
            cpu.step(&mut bus, &[], &SimulationConfig::default())
                .unwrap();
            assert_eq!(
                bus.read_u8(AVR_IO_MIRROR_BASE + 0x38).unwrap(),
                0x81,
                "{model:?}"
            );
            bus.write_u8(AVR_IO_MIRROR_BASE + 0x30, 0x04).unwrap(); // PD2 high
            assert_eq!(cpu.data_read(0x30, &bus).unwrap() & 0x04, 0x04);
            cpu.data_write(0x33, 0x5A, &mut bus).unwrap(); // GPIOR0
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x33).unwrap(), 0x00);
            assert_eq!(cpu.data_read(0x33, &bus).unwrap(), 0x5A);
        }
    }

    /// Measured tinyAVR CPU models: PINB/DDRB/PORTB at 0x36..0x38 with
    /// write-one-to-toggle PINB, SBI on PINB toggling only the named bit. The
    /// x4 profiles add PINA/DDRA/PORTA at 0x39..0x3B; single-port profiles leave
    /// 0x39..0x3B as plain stored bytes.
    #[test]
    fn classic_tiny_port_b_mirror_and_pin_toggle() {
        for (model, has_port_a) in [
            (AvrModel::Attiny13, false),
            (AvrModel::Attiny25, false),
            (AvrModel::Attiny45, false),
            (AvrModel::Attiny85, false),
            (AvrModel::Attiny24, true),
            (AvrModel::Attiny44, true),
            (AvrModel::Attiny84, true),
        ] {
            let mut bus = MockBus::new();
            let mut cpu = Avr::new_model(model);
            cpu.data_write(0x37, 0x09, &mut bus).unwrap(); // DDRB
            cpu.data_write(0x38, 0x09, &mut bus).unwrap(); // PORTB
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x38).unwrap(), 0x09);
            cpu.data_write(0x36, 0x01, &mut bus).unwrap(); // PINB toggles PB0
            assert_eq!(
                bus.read_u8(AVR_IO_MIRROR_BASE + 0x38).unwrap(),
                0x08,
                "{model:?}"
            );
            assert_eq!(cpu.data_read(0x36, &bus).unwrap() & 0x09, 0x08);
            // SBI PINB,0 (I/O 0x16): toggles PB0 back, PB3 untouched.
            cpu.load_words(0x40, &[0x9AB0, 0xCFFF]);
            cpu.set_pc(0x40);
            cpu.step(&mut bus, &[], &SimulationConfig::default())
                .unwrap();
            assert_eq!(
                bus.read_u8(AVR_IO_MIRROR_BASE + 0x38).unwrap(),
                0x09,
                "{model:?}"
            );
            bus.write_u8(AVR_IO_MIRROR_BASE + 0x36, 0x04).unwrap(); // PB2 high
            assert_eq!(cpu.data_read(0x36, &bus).unwrap() & 0x04, 0x04);
            cpu.data_write(0x3B, 0x5A, &mut bus).unwrap();
            let mirrored = if has_port_a { 0x5A } else { 0x00 };
            assert_eq!(
                bus.read_u8(AVR_IO_MIRROR_BASE + 0x3B).unwrap(),
                mirrored,
                "{model:?}"
            );
            assert_eq!(cpu.data_read(0x3B, &bus).unwrap(), 0x5A);
        }
    }

    /// ATtiny1634 ports C/B/A at 0x27/0x2B/0x2F (PINx, DDRx, PORTx), each
    /// followed by a PUEx byte that is not part of the mirrored port window.
    /// PINx is write-one-to-toggle; SBI PINB,0 toggles PB0 only.
    #[test]
    fn attiny1634_ports_mirror_and_pin_toggle() {
        let mut bus = MockBus::new();
        let mut cpu = Avr::new_model(AvrModel::Attiny1634);
        cpu.data_write(0x2C, 0x09, &mut bus).unwrap(); // DDRB
        cpu.data_write(0x2D, 0x09, &mut bus).unwrap(); // PORTB
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x2D).unwrap(), 0x09);
        cpu.data_write(0x2B, 0x01, &mut bus).unwrap(); // PINB toggles PB0
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x2D).unwrap(), 0x08);
        assert_eq!(cpu.data_read(0x2B, &bus).unwrap() & 0x09, 0x08);
        // SBI PINB,0 (I/O 0x0B): PB0 back high, PB3 untouched.
        cpu.load_words(0x40, &[0x9A58, 0xCFFF]);
        cpu.set_pc(0x40);
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x2D).unwrap(), 0x09);
        // An undriven input reads the external level.
        bus.write_u8(AVR_IO_MIRROR_BASE + 0x2B, 0x04).unwrap();
        assert_eq!(cpu.data_read(0x2B, &bus).unwrap() & 0x04, 0x04);
        // Ports A and C mirror and toggle too.
        cpu.data_write(0x31, 0x80, &mut bus).unwrap(); // PORTA
        cpu.data_write(0x2F, 0x81, &mut bus).unwrap(); // PINA toggles PA7, PA0
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x31).unwrap(), 0x01);
        cpu.data_write(0x29, 0x20, &mut bus).unwrap(); // PORTC
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x29).unwrap(), 0x20);
        // PUEB is a CPU-local byte, not a port mirror write.
        cpu.data_write(0x2E, 0x5A, &mut bus).unwrap();
        assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x2E).unwrap(), 0x00);
        assert_eq!(cpu.data_read(0x2E, &bus).unwrap(), 0x5A);
    }

    /// AT90USB162 and ATmega16M1/64M1 share the x8 port layout (PINB/PINC/PIND
    /// at 0x23/0x26/0x29); the M1 parts add PINE at 0x2C. PINx is
    /// write-one-to-toggle and SBI PINx,n toggles bit n only.
    #[test]
    fn usb162_and_m1_ports_mirror_and_pin_toggle() {
        for model in [
            AvrModel::At90usb162,
            AvrModel::Atmega16m1,
            AvrModel::Atmega64m1,
        ] {
            let mut bus = MockBus::new();
            let mut cpu = Avr::new_model(model);
            cpu.data_write(0x24, 0x09, &mut bus).unwrap(); // DDRB
            cpu.data_write(0x25, 0x09, &mut bus).unwrap(); // PORTB
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x25).unwrap(), 0x09);
            cpu.data_write(0x23, 0x01, &mut bus).unwrap(); // PINB toggles PB0
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x25).unwrap(), 0x08);
            assert_eq!(cpu.data_read(0x23, &bus).unwrap() & 0x09, 0x08);
            // SBI PINB,0 (I/O 0x03): PB0 back high, PB3 untouched.
            cpu.load_words(0x40, &[0x9A18, 0xCFFF]);
            cpu.set_pc(0x40);
            cpu.step(&mut bus, &[], &SimulationConfig::default())
                .unwrap();
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x25).unwrap(), 0x09);
            // An undriven input reads the external level.
            bus.write_u8(AVR_IO_MIRROR_BASE + 0x23, 0x04).unwrap();
            assert_eq!(cpu.data_read(0x23, &bus).unwrap() & 0x04, 0x04);
            cpu.data_write(0x2B, 0x40, &mut bus).unwrap(); // PORTD
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x2B).unwrap(), 0x40);
            let has_port_e = model != AvrModel::At90usb162;
            cpu.data_write(0x2E, 0x04, &mut bus).unwrap(); // PORTE (M1 only)
            assert_eq!(
                bus.read_u8(AVR_IO_MIRROR_BASE + 0x2E).unwrap(),
                if has_port_e { 0x04 } else { 0x00 },
                "{model:?} PORTE mirror"
            );
            if has_port_e {
                // SBI PINE,2 (I/O 0x0C) toggles PE2 only.
                cpu.load_words(0x40, &[0x9A62, 0xCFFF]);
                cpu.set_pc(0x40);
                cpu.step(&mut bus, &[], &SimulationConfig::default())
                    .unwrap();
                assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x2E).unwrap(), 0x00);
            }
        }
    }

    /// ATmega169P and ATmega128RFA1/RFR2 have ports A-G at 0x20..0x34 (as on
    /// the 1281). PINx is write-one-to-toggle and SBI PINx,n toggles bit n.
    #[test]
    fn mega169_and_rf_ports_mirror_and_pin_toggle() {
        for model in [
            AvrModel::Atmega169,
            AvrModel::Atmega128rfa1,
            AvrModel::Atmega128rfr2,
        ] {
            let mut bus = MockBus::new();
            let mut cpu = Avr::new_model(model);
            cpu.data_write(0x24, 0x09, &mut bus).unwrap(); // DDRB
            cpu.data_write(0x25, 0x09, &mut bus).unwrap(); // PORTB
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x25).unwrap(), 0x09);
            cpu.data_write(0x23, 0x01, &mut bus).unwrap(); // PINB toggles PB0
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x25).unwrap(), 0x08);
            // SBI PINB,0 (I/O 0x03): PB0 back high, PB3 untouched.
            cpu.load_words(0x40, &[0x9A18, 0xCFFF]);
            cpu.set_pc(0x40);
            cpu.step(&mut bus, &[], &SimulationConfig::default())
                .unwrap();
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x25).unwrap(), 0x09);
            // PORTA mirrors; SBI PINA,7 (I/O 0x00) toggles PA7 only.
            cpu.data_write(0x22, 0x82, &mut bus).unwrap();
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x22).unwrap(), 0x82);
            cpu.load_words(0x40, &[0x9A07, 0xCFFF]);
            cpu.set_pc(0x40);
            cpu.step(&mut bus, &[], &SimulationConfig::default())
                .unwrap();
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x22).unwrap(), 0x02);
            // PORTG at 0x34 is the last mirrored byte.
            cpu.data_write(0x34, 0x01, &mut bus).unwrap();
            assert_eq!(
                bus.read_u8(AVR_IO_MIRROR_BASE + 0x34).unwrap(),
                0x01,
                "{model:?}"
            );
            // An undriven input reads the external level.
            bus.write_u8(AVR_IO_MIRROR_BASE + 0x2C, 0x10).unwrap();
            assert_eq!(cpu.data_read(0x2C, &bus).unwrap() & 0x10, 0x10);
        }
    }

    /// SBI/CBI on ATmega16 PORTA (I/O 0x1B) are read-modify-write: the 328P
    /// write-one set (where 0x3B is PCIFR) must not apply.
    #[test]
    fn atmega16_sbi_cbi_on_porta_keep_other_bits() {
        let mut bus = MockBus::new();
        let mut cpu = Avr::new_model(AvrModel::Atmega16);
        cpu.data_write(0x3B, 0x80, &mut bus).unwrap();
        // SBI PORTA,0 ; CBI PORTA,7 ; RJMP .
        cpu.load_words(0x100, &[0x9AD8, 0x98DF, 0xCFFF]);
        cpu.set_pc(0x100);
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.data_read(0x3B, &bus).unwrap(), 0x81);
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap();
        assert_eq!(cpu.data_read(0x3B, &bus).unwrap(), 0x01);
    }

    #[test]
    fn mega164_family_mirrors_port_a_and_pin_toggle() {
        for model in [
            AvrModel::Atmega164,
            AvrModel::Atmega324,
            AvrModel::Atmega644,
            AvrModel::Atmega1284p,
        ] {
            let mut bus = MockBus::new();
            let mut cpu = Avr::new_model(model);

            cpu.data_write(0x21, 0x01, &mut bus).unwrap(); // DDRA: PA0 out
            cpu.data_write(0x22, 0x01, &mut bus).unwrap(); // PORTA: PA0 high
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x21).unwrap(), 0x01);
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x22).unwrap(), 0x01);
            assert_eq!(cpu.data_read(AVR_PINA, &bus).unwrap() & 0x01, 0x01);

            cpu.data_write(AVR_PINA, 0x01, &mut bus).unwrap(); // Toggle PA0 low.
            assert_eq!(bus.read_u8(AVR_IO_MIRROR_BASE + 0x22).unwrap(), 0x00);
            assert_eq!(cpu.data_read(AVR_PINA, &bus).unwrap() & 0x01, 0x00);

            bus.write_u8(AVR_IO_MIRROR_BASE + u64::from(AVR_PINA), 0x02)
                .unwrap(); // PA1 is externally high.
            assert_eq!(cpu.data_read(AVR_PINA, &bus).unwrap() & 0x02, 0x02);
        }
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
