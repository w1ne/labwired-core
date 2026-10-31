// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! PIC16 classic mid-range core (14-bit instruction words, 35 instructions).
//!
//! The PC is a 13-bit **word** address. Every instruction is one word, so
//! the PC advances by 1. Program-image addresses are byte addresses (Intel
//! HEX / ELF, two bytes per word, little-endian), so word `w` lives at byte
//! `2 * w`.
//!
//! Data memory is four 128-byte banks selected by `STATUS.RP1:RP0` for
//! direct addressing and by `STATUS.IRP:FSR` for `INDF`. The core
//! registers (`INDF`, `PCL`, `STATUS`, `FSR`, `PCLATH`, `INTCON`) and the
//! device's shared GPR block appear in every bank. The PIC16F877A also
//! mirrors `TMR0`/`PORTB` into bank 2 and `OPTION_REG`/`TRISB` into bank 3
//! (DS39582B Figure 2-3).
//!
//! `GOTO`/`CALL` take an 11-bit literal and `PCLATH<4:3>` for the page; a
//! write to `PCL` loads `PCLATH<4:0>:value`. The return stack is eight
//! levels deep and circular: a ninth push overwrites the first, with no
//! overflow flag (DS39582B section 2.4).
//!
//! The enhanced mid-range (PIC16F1xxx) instruction set and SFR map are not
//! modelled here. An unknown opcode returns
//! [`SimulationError::DecodeError`]; it is never guessed.

use crate::snapshot::{CpuSnapshot, Pic16CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
use std::sync::{Arc, Mutex};

/// Banked data space: four 128-byte banks.
pub const PIC16_DATA_BYTES: usize = 0x200;
/// Hardware return stack depth.
pub const PIC16_STACK_DEPTH: usize = 8;
/// 13-bit program counter.
pub const PIC16_PC_MASK: u32 = 0x1FFF;
/// Instruction words are 14 bits wide.
pub const PIC16_WORD_MASK: u16 = 0x3FFF;
/// Default program memory (words) for an unnamed core: the largest classic
/// mid-range part, the 8K-word PIC16F877A.
pub const PIC16_FLASH_WORDS_DEFAULT: usize = 8192;
/// Interrupt vector (word address).
pub const PIC16_INT_VECTOR: u32 = 0x0004;
/// User ID locations (word addresses 0x2000-0x2003).
pub const PIC16_IDLOC_BASE: u32 = 0x2000;
/// Configuration word (word address). Parts with two words (PIC16F88x) put
/// CONFIG2 at the next address.
pub const PIC16_CONFIG_WORD: u32 = 0x2007;
/// Data EEPROM initialisers in the HEX image (word addresses 0x2100-0x21FF,
/// one data byte per word).
pub const PIC16_EEDATA_BASE: u32 = 0x2100;
/// Largest classic mid-range data EEPROM (256 B).
pub const PIC16_EEDATA_BYTES: usize = 256;

pub const SFR_INDF: u16 = 0x00;
pub const SFR_TMR0: u16 = 0x01;
pub const SFR_PCL: u16 = 0x02;
pub const SFR_STATUS: u16 = 0x03;
pub const SFR_FSR: u16 = 0x04;
pub const SFR_PORTA: u16 = 0x05;
pub const SFR_PCLATH: u16 = 0x0A;
pub const SFR_INTCON: u16 = 0x0B;
pub const SFR_PIR1: u16 = 0x0C;
pub const SFR_PIR2: u16 = 0x0D;
pub const SFR_RCSTA: u16 = 0x18;
pub const SFR_TXREG: u16 = 0x19;
pub const SFR_OPTION_REG: u16 = 0x81;
pub const SFR_TRISA: u16 = 0x85;
pub const SFR_PIE1: u16 = 0x8C;
pub const SFR_PIE2: u16 = 0x8D;
pub const SFR_TXSTA: u16 = 0x98;
pub const SFR_SPBRG: u16 = 0x99;
pub const SFR_ADCON1: u16 = 0x9F;

/// PORTA..PORTE.
pub const PIC16_PORT_COUNT: u16 = 5;

pub const STATUS_C: u8 = 1 << 0;
pub const STATUS_DC: u8 = 1 << 1;
pub const STATUS_Z: u8 = 1 << 2;
pub const STATUS_PD: u8 = 1 << 3;
pub const STATUS_TO: u8 = 1 << 4;
pub const STATUS_RP0: u8 = 1 << 5;
pub const STATUS_RP1: u8 = 1 << 6;
pub const STATUS_IRP: u8 = 1 << 7;
/// Bits an instruction result or a file write can change: TO and PD are
/// read-only (set by reset, `SLEEP` and `CLRWDT`).
const STATUS_WRITABLE: u8 = !(STATUS_TO | STATUS_PD);
const STATUS_ALU: u8 = STATUS_C | STATUS_DC | STATUS_Z;

const INTCON_GIE: u8 = 1 << 7;
const INTCON_PEIE: u8 = 1 << 6;

const TXSTA_TXEN: u8 = 1 << 5;
const TXSTA_TRMT: u8 = 1 << 1;
const RCSTA_SPEN: u8 = 1 << 7;
const PIR1_TXIF: u8 = 1 << 4;

/// One classic mid-range part: program words, implemented port bits
/// (PORTA..PORTE) and the GPR address ranges that alias another range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pic16Device {
    pub name: &'static str,
    pub flash_words: usize,
    pub port_masks: [u8; PIC16_PORT_COUNT as usize],
    /// `(start, end, target)`: banked addresses `start..=end` are the same
    /// bytes as `target..`. Taken from the gputils `SHAREBANK` groups (the
    /// 0x70-0x7F block on most parts; whole banks on the PIC16F873/874).
    pub aliases: &'static [(u16, u16, u16)],
}

const fn dev(
    name: &'static str,
    flash_words: usize,
    port_masks: [u8; PIC16_PORT_COUNT as usize],
    aliases: &'static [(u16, u16, u16)],
) -> Pic16Device {
    Pic16Device {
        name,
        flash_words,
        port_masks,
        aliases,
    }
}

/// The parts this model knows. A row means every SFR address and bit the
/// model implements matches the part's SDCC 4.4.0 `pic14` header; flash
/// words come from the gputils `<part>_g.lkr` code pages, port masks from
/// the header's `R<port><bit>` fields and aliases from the lkr `SHAREBANK`
/// groups. `scripts/pic16/check_sfr_map.py --check-table` regenerates and
/// re-verifies every row. A row is geometry, not onboarding.
///
/// Left out: the enhanced mid-range (PIC16F1xxx: BSR, linear memory, a
/// different SFR map), parts without the USART at 0x18/0x19/0x98/0x99 or
/// without PORTC (PIC16F6xx/8x/81x/84A and friends), and the OTP/EPROM
/// PIC16C parts.
const PIC16_DEVICES: &[Pic16Device] = &[
    dev(
        "pic16f687",
        2048,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f689",
        4096,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f690",
        4096,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f722a",
        2048,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f723a",
        4096,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f724",
        4096,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f726",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f727",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16lf722a",
        2048,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16lf723a",
        4096,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16lf724",
        4096,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16lf726",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16lf727",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f870",
        2048,
        [0x3F, 0xFF, 0xFF, 0x00, 0x00],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x120, 0x16F, 0x020),
            (0x170, 0x17F, 0x070),
            (0x1A0, 0x1BF, 0x0A0),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f871",
        2048,
        [0x3F, 0xFF, 0xFF, 0xFF, 0x07],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x120, 0x16F, 0x020),
            (0x170, 0x17F, 0x070),
            (0x1A0, 0x1BF, 0x0A0),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f873",
        4096,
        [0x3F, 0xFF, 0xFF, 0x00, 0x00],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x120, 0x17F, 0x020),
            (0x170, 0x17F, 0x070),
            (0x1A0, 0x1FF, 0x0A0),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f873a",
        4096,
        [0x3F, 0xFF, 0xFF, 0x00, 0x00],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x120, 0x17F, 0x020),
            (0x170, 0x17F, 0x070),
            (0x1A0, 0x1FF, 0x0A0),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f874",
        4096,
        [0x3F, 0xFF, 0xFF, 0xFF, 0x07],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x120, 0x17F, 0x020),
            (0x170, 0x17F, 0x070),
            (0x1A0, 0x1FF, 0x0A0),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f874a",
        4096,
        [0x3F, 0xFF, 0xFF, 0xFF, 0x07],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x120, 0x17F, 0x020),
            (0x170, 0x17F, 0x070),
            (0x1A0, 0x1FF, 0x0A0),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f876",
        8192,
        [0x3F, 0xFF, 0xFF, 0x00, 0x00],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f876a",
        8192,
        [0x3F, 0xFF, 0xFF, 0x00, 0x00],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f877",
        8192,
        [0x3F, 0xFF, 0xFF, 0xFF, 0x07],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f877a",
        8192,
        [0x3F, 0xFF, 0xFF, 0xFF, 0x07],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f882",
        2048,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f883",
        4096,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f884",
        4096,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f886",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f887",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f913",
        4096,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f914",
        4096,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f916",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f917",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f946",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16f707",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
    dev(
        "pic16lf707",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        &[
            (0x0F0, 0x0FF, 0x070),
            (0x170, 0x17F, 0x070),
            (0x1F0, 0x1FF, 0x070),
        ],
    ),
];

/// Geometry for a catalogue part, if this model knows it.
pub fn pic16_device(name: &str) -> Option<&'static Pic16Device> {
    let name = name.trim().to_ascii_lowercase();
    PIC16_DEVICES.iter().find(|d| d.name == name)
}

/// Reject an image whose entry point is not the reset vector's byte form.
/// PIC16 always resets to word 0; an odd byte entry cannot name a word.
pub fn check_pic16_entry(entry: u64) -> Result<(), String> {
    if entry % 2 != 0 {
        return Err(format!(
            "PIC16 ELF entry {entry:#x} is not word aligned (byte addresses, two per word)"
        ));
    }
    Ok(())
}

pub struct Pic16 {
    wreg: u8,
    pc: u32,
    cycles: u64,
    flash: Vec<u16>,
    data: Vec<u8>,
    /// PORTA..PORTE output latches. Writes to PORTx land here; reads see
    /// the pins.
    latch: [u8; PIC16_PORT_COUNT as usize],
    port_masks: [u8; PIC16_PORT_COUNT as usize],
    aliases: &'static [(u16, u16, u16)],
    stack: [u32; PIC16_STACK_DEPTH],
    /// Next free stack slot, modulo the depth.
    stkptr: u8,
    exception_pending: Option<u32>,
    asleep: bool,
    idloc: [u16; 4],
    config: [u16; 2],
    /// Data EEPROM contents as programmed by the image. The EEPROM itself
    /// (EECON1/EEDATA reads and writes) is not modelled yet.
    eedata: [u8; PIC16_EEDATA_BYTES],
    serial_sink: Option<Arc<Mutex<Vec<u8>>>>,
}

impl Default for Pic16 {
    fn default() -> Self {
        Self::new()
    }
}

impl Pic16 {
    /// A PIC16F877A-shaped core with no catalogue name.
    pub fn new() -> Self {
        Self::with_device(pic16_device("pic16f877a").expect("PIC16F877A row"))
    }

    pub fn with_device(device: &Pic16Device) -> Self {
        assert!(device.flash_words > 0, "PIC16 flash must hold a word");
        let mut cpu = Self {
            wreg: 0,
            pc: 0,
            cycles: 0,
            flash: vec![PIC16_WORD_MASK; device.flash_words],
            data: vec![0; PIC16_DATA_BYTES],
            latch: [0; PIC16_PORT_COUNT as usize],
            port_masks: device.port_masks,
            aliases: device.aliases,
            stack: [0; PIC16_STACK_DEPTH],
            stkptr: 0,
            exception_pending: None,
            asleep: false,
            idloc: [PIC16_WORD_MASK; 4],
            config: [PIC16_WORD_MASK; 2],
            eedata: [0xFF; PIC16_EEDATA_BYTES],
            serial_sink: None,
        };
        cpu.reset_core();
        cpu
    }

    /// The chip's own geometry, or the default core for an unknown name.
    pub fn for_chip_name(name: &str) -> Self {
        match pic16_device(name) {
            Some(device) => Self::with_device(device),
            None => Self::new(),
        }
    }

    /// Route USART TX bytes to `sink` (the console / `uart_contains` buffer).
    pub fn set_serial_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>) {
        self.serial_sink = Some(sink);
    }

    /// Program memory size in words.
    pub fn flash_words(&self) -> usize {
        self.flash.len()
    }

    /// The configuration word (word address 0x2007) as loaded.
    pub fn config_word(&self) -> u16 {
        self.config[0]
    }

    /// Data EEPROM byte `n` as programmed by the image (0xFF = erased).
    pub fn eedata(&self, n: usize) -> u8 {
        self.eedata[n]
    }

    /// Output latch `n` (0 = PORTA .. 4 = PORTE).
    pub fn port_latch(&self, n: usize) -> u8 {
        self.latch[n]
    }

    pub fn is_asleep(&self) -> bool {
        self.asleep
    }

    pub fn wreg(&self) -> u8 {
        self.wreg
    }

    pub fn status(&self) -> u8 {
        self.data[usize::from(SFR_STATUS)]
    }

    /// Power-on reset values for the registers this model implements
    /// (DS39582B Table 14-6). General purpose RAM is left as it is.
    fn reset_core(&mut self) {
        self.wreg = 0;
        self.pc = 0;
        self.stack = [0; PIC16_STACK_DEPTH];
        self.stkptr = 0;
        self.asleep = false;
        self.latch = [0; PIC16_PORT_COUNT as usize];
        // STATUS 0001 1xxx: TO = PD = 1, bank 0.
        self.data[usize::from(SFR_STATUS)] = STATUS_TO | STATUS_PD;
        self.data[usize::from(SFR_FSR)] = 0;
        self.data[usize::from(SFR_PCLATH)] = 0;
        self.data[usize::from(SFR_INTCON)] = 0;
        self.data[usize::from(SFR_PIR1)] = 0;
        self.data[usize::from(SFR_PIR2)] = 0;
        self.data[usize::from(SFR_PIE1)] = 0;
        self.data[usize::from(SFR_PIE2)] = 0;
        self.data[usize::from(SFR_OPTION_REG)] = 0xFF;
        for n in 0..PIC16_PORT_COUNT {
            self.data[usize::from(SFR_TRISA + n)] = 0xFF;
        }
        // TRISE 0000 -111: PSP mode off, RE0-RE2 inputs.
        self.data[usize::from(SFR_TRISA + 4)] = 0x07;
        self.data[usize::from(SFR_TXSTA)] = TXSTA_TRMT;
        self.data[usize::from(SFR_RCSTA)] = 0;
        self.data[usize::from(SFR_SPBRG)] = 0;
        self.data[usize::from(SFR_ADCON1)] = 0;
    }

    /// Load 14-bit instruction words from the reset vector (test fixtures).
    pub fn load_words(&mut self, words: &[u16]) {
        assert!(
            words.len() <= self.flash.len(),
            "fixture exceeds PIC16 model flash"
        );
        for (slot, word) in self.flash.iter_mut().zip(words) {
            *slot = word & PIC16_WORD_MASK;
        }
    }

    /// Load bytes at a **byte** address (two per word, little-endian). The
    /// ID and configuration words land in their own registers and the data
    /// EEPROM initialisers (0x2100-0x21FF) in the EEPROM image; any other
    /// address past the part's flash is a fixture error.
    pub fn load_flash(&mut self, addr: u32, bytes: &[u8]) {
        for (i, byte) in bytes.iter().enumerate() {
            let at = addr + i as u32;
            let word = at / 2;
            let high = at % 2 == 1;
            if (PIC16_EEDATA_BASE..PIC16_EEDATA_BASE + PIC16_EEDATA_BYTES as u32).contains(&word) {
                // One data byte per word; the high byte of the word is padding.
                if !high {
                    self.eedata[(word - PIC16_EEDATA_BASE) as usize] = *byte;
                }
                continue;
            }
            let slot = match word {
                w if (w as usize) < self.flash.len() => &mut self.flash[w as usize],
                w if (PIC16_IDLOC_BASE..PIC16_IDLOC_BASE + 4).contains(&w) => {
                    &mut self.idloc[(w - PIC16_IDLOC_BASE) as usize]
                }
                w if (PIC16_CONFIG_WORD..PIC16_CONFIG_WORD + 2).contains(&w) => {
                    &mut self.config[(w - PIC16_CONFIG_WORD) as usize]
                }
                // 0x2004-0x2006 (reserved / device ID) are not programmable.
                w if (PIC16_IDLOC_BASE..=PIC16_CONFIG_WORD).contains(&w) => continue,
                _ => panic!("fixture exceeds PIC16 model flash (byte {at:#x})"),
            };
            *slot = if high {
                (*slot & 0x00FF) | (u16::from(*byte) << 8)
            } else {
                (*slot & 0xFF00) | u16::from(*byte)
            } & PIC16_WORD_MASK;
        }
    }

    /// Load a program image (byte addresses). The PC starts at the reset
    /// vector.
    pub fn load_program_image(&mut self, image: &crate::memory::ProgramImage) {
        for seg in &image.segments {
            self.load_flash(seg.start_addr as u32, &seg.data);
        }
        self.pc = 0;
    }

    /// Map a 9-bit banked address onto the register that backs it.
    fn canonical(&self, addr: u16) -> u16 {
        let addr = addr & 0x1FF;
        let off = (addr & 0x7F) as u8;
        match off {
            0x00 | 0x02 | 0x03 | 0x04 | 0x0A | 0x0B => return u16::from(off),
            _ => {}
        }
        // Follow the alias ranges to the backing store. They can chain (on
        // the PIC16F873 0x1F0 aliases 0xF0, which aliases 0x70), so repeat
        // until no range matches (bounded: a chain is at most one hop per
        // bank).
        let mut addr = addr;
        for _ in 0..4 {
            match self
                .aliases
                .iter()
                .find(|(start, end, _)| (*start..=*end).contains(&addr))
            {
                Some(&(start, _, target)) => addr = target + (addr - start),
                None => break,
            }
        }
        match addr {
            0x101 => SFR_TMR0,
            0x106 => SFR_PORTA + 1,
            0x181 => SFR_OPTION_REG,
            0x186 => SFR_TRISA + 1,
            _ => addr,
        }
    }

    /// Direct file address for a 7-bit `f` in the bank `RP1:RP0` selects.
    pub fn file_address(&self, f: u8) -> u16 {
        let rp = u16::from((self.status() >> 5) & 0x03);
        (rp << 7) | u16::from(f & 0x7F)
    }

    fn indirect_address(&self) -> u16 {
        let irp = u16::from(self.status() & STATUS_IRP != 0);
        (irp << 8) | u16::from(self.data[usize::from(SFR_FSR)])
    }

    /// Resolve `f`, following `INDF` through `IRP:FSR`.
    fn resolve(&self, f: u8) -> u16 {
        let addr = self.canonical(self.file_address(f));
        if addr == SFR_INDF {
            self.canonical(self.indirect_address())
        } else {
            addr
        }
    }

    /// Read a data-space register by banked address.
    pub fn read_data(&self, addr: u16) -> u8 {
        let addr = self.canonical(addr);
        match addr {
            // INDF addressed through FSR = INDF reads 0.
            SFR_INDF => 0,
            SFR_PCL => self.pc as u8,
            // TXIF is read-only and set while TXREG is empty; a byte leaves
            // the instant it is written here, so it follows TXEN.
            SFR_PIR1 => {
                let pir1 = self.data[usize::from(SFR_PIR1)] & !PIR1_TXIF;
                if self.data[usize::from(SFR_TXSTA)] & TXSTA_TXEN != 0 {
                    pir1 | PIR1_TXIF
                } else {
                    pir1
                }
            }
            // A pin set as output reads its latch; an input reads 0 because
            // nothing external drives it yet.
            a if (SFR_PORTA..SFR_PORTA + PIC16_PORT_COUNT).contains(&a) => {
                let n = usize::from(a - SFR_PORTA);
                let tris = self.data[usize::from(SFR_TRISA) + n];
                self.latch[n] & !tris & self.port_masks[n]
            }
            _ => self.data[usize::from(addr)],
        }
    }

    /// Write a data-space register by banked address.
    pub fn write_data(&mut self, addr: u16, value: u8) {
        let addr = self.canonical(addr);
        match addr {
            SFR_INDF => {}
            SFR_PCL => {
                let pclath = u32::from(self.data[usize::from(SFR_PCLATH)] & 0x1F);
                self.pc = ((pclath << 8) | u32::from(value)) & PIC16_PC_MASK;
            }
            SFR_STATUS => {
                let old = self.data[usize::from(SFR_STATUS)];
                self.data[usize::from(SFR_STATUS)] =
                    (old & !STATUS_WRITABLE) | (value & STATUS_WRITABLE);
            }
            SFR_PCLATH => self.data[usize::from(SFR_PCLATH)] = value & 0x1F,
            SFR_PIR1 => self.data[usize::from(SFR_PIR1)] = value & !PIR1_TXIF,
            // TRMT is read-only and stays set.
            SFR_TXSTA => self.data[usize::from(SFR_TXSTA)] = value | TXSTA_TRMT,
            SFR_TXREG => {
                self.data[usize::from(SFR_TXREG)] = value;
                let txsta = self.data[usize::from(SFR_TXSTA)];
                let rcsta = self.data[usize::from(SFR_RCSTA)];
                if txsta & TXSTA_TXEN != 0 && rcsta & RCSTA_SPEN != 0 {
                    if let Some(sink) = &self.serial_sink {
                        if let Ok(mut buf) = sink.lock() {
                            buf.push(value);
                        }
                    }
                }
            }
            a if (SFR_PORTA..SFR_PORTA + PIC16_PORT_COUNT).contains(&a) => {
                let n = usize::from(a - SFR_PORTA);
                self.latch[n] = value & self.port_masks[n];
            }
            _ => self.data[usize::from(addr)] = value,
        }
    }

    /// Read `f` for an instruction (bank select + `INDF`).
    pub fn read_file(&self, f: u8) -> u8 {
        self.read_data(self.resolve(f))
    }

    /// Write `f` for an instruction (bank select + `INDF`).
    pub fn write_file(&mut self, f: u8, value: u8) {
        let addr = self.resolve(f);
        self.write_data(addr, value);
    }

    /// Store an ALU result to W (`d = 0`) or `f` (`d = 1`), then apply the
    /// instruction's flags. With STATUS as the destination of an instruction
    /// that affects Z, DC or C, those three bits take the flag values, not
    /// the written ones (DS39582B section 2.2.2.1).
    fn store(&mut self, f: u8, d: bool, value: u8, flags: Option<(u8, u8)>) {
        if d {
            self.write_file(f, value);
        } else {
            self.wreg = value;
        }
        if let Some((mask, set)) = flags {
            let status = &mut self.data[usize::from(SFR_STATUS)];
            *status = (*status & !mask) | (set & mask);
        }
    }

    fn z(value: u8) -> u8 {
        if value == 0 {
            STATUS_Z
        } else {
            0
        }
    }

    fn add_flags(a: u8, b: u8) -> (u8, u8) {
        let result = a.wrapping_add(b);
        let mut set = Self::z(result);
        if u16::from(a) + u16::from(b) > 0xFF {
            set |= STATUS_C;
        }
        if (a & 0x0F) + (b & 0x0F) > 0x0F {
            set |= STATUS_DC;
        }
        (result, set)
    }

    /// `a - b`: C and DC are "no borrow".
    fn sub_flags(a: u8, b: u8) -> (u8, u8) {
        let result = a.wrapping_sub(b);
        let mut set = Self::z(result);
        if a >= b {
            set |= STATUS_C;
        }
        if (a & 0x0F) >= (b & 0x0F) {
            set |= STATUS_DC;
        }
        (result, set)
    }

    fn push(&mut self, value: u32) {
        self.stack[usize::from(self.stkptr)] = value & PIC16_PC_MASK;
        self.stkptr = (self.stkptr + 1) % PIC16_STACK_DEPTH as u8;
    }

    fn pop(&mut self) -> u32 {
        self.stkptr = (self.stkptr + PIC16_STACK_DEPTH as u8 - 1) % PIC16_STACK_DEPTH as u8;
        self.stack[usize::from(self.stkptr)]
    }

    fn fetch_word(&self, pc: u32) -> u16 {
        // Unimplemented program memory reads as erased (0x3FFF, ADDLW 0xFF).
        self.flash
            .get(pc as usize)
            .copied()
            .unwrap_or(PIC16_WORD_MASK)
    }

    /// `PCLATH<4:3>` page for `GOTO`/`CALL`.
    fn page(&self) -> u32 {
        u32::from(self.data[usize::from(SFR_PCLATH)] & 0x18) << 8
    }

    fn interrupt_request(&self) -> bool {
        let intcon = self.read_data(SFR_INTCON);
        intcon & INTCON_GIE != 0 && self.source_pending(intcon)
    }

    /// An enabled source with its flag set, ignoring GIE (the `SLEEP` wake
    /// condition). INTCON T0IF/INTF/RBIF pair with T0IE/INTE/RBIE; the
    /// PIR1/PIR2 peripheral flags also need PEIE.
    fn source_pending(&self, intcon: u8) -> bool {
        let core = intcon & 0x07 & (intcon >> 3);
        let peripheral = intcon & INTCON_PEIE != 0
            && (self.read_data(SFR_PIR1) & self.read_data(SFR_PIE1) != 0
                || self.read_data(SFR_PIR2) & self.read_data(SFR_PIE2) != 0);
        core != 0 || peripheral
    }

    /// Vector to 0x0004: push the return address and clear GIE. There is no
    /// hardware context save on the classic mid-range.
    fn take_interrupt(&mut self) {
        self.push(self.pc);
        self.data[usize::from(SFR_INTCON)] &= !INTCON_GIE;
        self.pc = PIC16_INT_VECTOR;
    }

    /// Execute `word`; `self.pc` already points at the next instruction.
    /// Returns the instruction cycles taken.
    fn execute(&mut self, word: u16, pc: u32) -> SimResult<u32> {
        let f = (word & 0x7F) as u8;
        let d = word & 0x80 != 0;
        let k8 = word as u8;
        match word >> 12 {
            0b00 => self.execute_byte(word, pc, f, d),
            0b01 => {
                let mask = 1u8 << ((word >> 7) & 0x07);
                match (word >> 10) & 0x03 {
                    0b00 => {
                        let v = self.read_file(f) & !mask;
                        self.write_file(f, v);
                        Ok(1)
                    }
                    0b01 => {
                        let v = self.read_file(f) | mask;
                        self.write_file(f, v);
                        Ok(1)
                    }
                    0b10 => Ok(self.skip_if(self.read_file(f) & mask == 0)),
                    _ => Ok(self.skip_if(self.read_file(f) & mask != 0)),
                }
            }
            0b10 => {
                let target = self.page() | u32::from(word & 0x07FF);
                if word & 0x0800 == 0 {
                    self.push(self.pc);
                }
                self.pc = target;
                Ok(2)
            }
            _ => {
                match (word >> 8) & 0x0F {
                    0x0..=0x3 => self.wreg = k8,
                    0x4..=0x7 => {
                        self.wreg = k8;
                        self.pc = self.pop();
                        return Ok(2);
                    }
                    0x8 => {
                        self.wreg |= k8;
                        let z = Self::z(self.wreg);
                        self.set_flags(STATUS_Z, z);
                    }
                    0x9 => {
                        self.wreg &= k8;
                        let z = Self::z(self.wreg);
                        self.set_flags(STATUS_Z, z);
                    }
                    0xA => {
                        self.wreg ^= k8;
                        let z = Self::z(self.wreg);
                        self.set_flags(STATUS_Z, z);
                    }
                    0xB => return Err(SimulationError::DecodeError(u64::from(pc))),
                    0xC | 0xD => {
                        let (r, set) = Self::sub_flags(k8, self.wreg);
                        self.wreg = r;
                        self.set_flags(STATUS_ALU, set);
                    }
                    _ => {
                        let (r, set) = Self::add_flags(self.wreg, k8);
                        self.wreg = r;
                        self.set_flags(STATUS_ALU, set);
                    }
                }
                Ok(1)
            }
        }
    }

    fn set_flags(&mut self, mask: u8, set: u8) {
        let status = &mut self.data[usize::from(SFR_STATUS)];
        *status = (*status & !mask) | (set & mask);
    }

    fn skip_if(&mut self, skip: bool) -> u32 {
        if skip {
            self.pc = (self.pc + 1) & PIC16_PC_MASK;
            2
        } else {
            1
        }
    }

    /// Byte-oriented file operations and the `00 0000 0xxx xxxx` control
    /// group.
    fn execute_byte(&mut self, word: u16, pc: u32, f: u8, d: bool) -> SimResult<u32> {
        let op = (word >> 8) & 0x0F;
        if op == 0 && !d {
            return self.execute_control(word, pc);
        }
        // A write to PCL is a jump and costs a second cycle.
        let pcl_dest = d && self.resolve(f) == SFR_PCL;
        let extra = u32::from(pcl_dest);
        match op {
            0x0 => {
                self.write_file(f, self.wreg);
            }
            0x1 => {
                // CLRW (d = 0) / CLRF (d = 1): Z set.
                self.store(f, d, 0, Some((STATUS_Z, STATUS_Z)));
            }
            0x2 => {
                let (r, set) = Self::sub_flags(self.read_file(f), self.wreg);
                self.store(f, d, r, Some((STATUS_ALU, set)));
            }
            0x3 => {
                let r = self.read_file(f).wrapping_sub(1);
                self.store(f, d, r, Some((STATUS_Z, Self::z(r))));
            }
            0x4 => {
                let r = self.read_file(f) | self.wreg;
                self.store(f, d, r, Some((STATUS_Z, Self::z(r))));
            }
            0x5 => {
                let r = self.read_file(f) & self.wreg;
                self.store(f, d, r, Some((STATUS_Z, Self::z(r))));
            }
            0x6 => {
                let r = self.read_file(f) ^ self.wreg;
                self.store(f, d, r, Some((STATUS_Z, Self::z(r))));
            }
            0x7 => {
                let (r, set) = Self::add_flags(self.read_file(f), self.wreg);
                self.store(f, d, r, Some((STATUS_ALU, set)));
            }
            0x8 => {
                let r = self.read_file(f);
                self.store(f, d, r, Some((STATUS_Z, Self::z(r))));
            }
            0x9 => {
                let r = !self.read_file(f);
                self.store(f, d, r, Some((STATUS_Z, Self::z(r))));
            }
            0xA => {
                let r = self.read_file(f).wrapping_add(1);
                self.store(f, d, r, Some((STATUS_Z, Self::z(r))));
            }
            0xB => {
                let r = self.read_file(f).wrapping_sub(1);
                self.store(f, d, r, None);
                return Ok(self.skip_if(r == 0) + extra);
            }
            0xC => {
                let v = self.read_file(f);
                let c = self.status() & STATUS_C;
                let r = (v >> 1) | (c << 7);
                self.store(f, d, r, Some((STATUS_C, v & 1)));
            }
            0xD => {
                let v = self.read_file(f);
                let c = self.status() & STATUS_C;
                let r = (v << 1) | c;
                self.store(f, d, r, Some((STATUS_C, v >> 7)));
            }
            0xE => {
                let r = self.read_file(f).rotate_left(4);
                self.store(f, d, r, None);
            }
            _ => {
                let r = self.read_file(f).wrapping_add(1);
                self.store(f, d, r, None);
                return Ok(self.skip_if(r == 0) + extra);
            }
        }
        Ok(1 + extra)
    }

    fn execute_control(&mut self, word: u16, pc: u32) -> SimResult<u32> {
        match word {
            // NOP: 00 0000 0xx0 0000.
            0x0000 | 0x0020 | 0x0040 | 0x0060 => Ok(1),
            // RETURN
            0x0008 => {
                self.pc = self.pop();
                Ok(2)
            }
            // RETFIE
            0x0009 => {
                self.pc = self.pop();
                self.data[usize::from(SFR_INTCON)] |= INTCON_GIE;
                Ok(2)
            }
            // OPTION (not recommended on the PIC16F877A, still decoded).
            0x0062 => {
                self.data[usize::from(SFR_OPTION_REG)] = self.wreg;
                Ok(1)
            }
            // SLEEP: PD = 0, TO = 1.
            0x0063 => {
                self.set_flags(STATUS_TO | STATUS_PD, STATUS_TO);
                self.asleep = true;
                Ok(1)
            }
            // CLRWDT: TO = PD = 1. The WDT itself is not modelled.
            0x0064 => {
                self.set_flags(STATUS_TO | STATUS_PD, STATUS_TO | STATUS_PD);
                Ok(1)
            }
            // TRIS PORTA/PORTB/PORTC.
            0x0065..=0x0067 => {
                let n = usize::from(word - 0x0065);
                self.data[usize::from(SFR_TRISA) + n] = self.wreg;
                Ok(1)
            }
            _ => Err(SimulationError::DecodeError(u64::from(pc))),
        }
    }

    fn register_view(&self) -> [u32; 6] {
        [
            u32::from(self.wreg),
            u32::from(self.status()),
            u32::from(self.data[usize::from(SFR_FSR)]),
            u32::from(self.data[usize::from(SFR_PCLATH)]),
            u32::from(self.stkptr),
            self.pc,
        ]
    }
}

impl Cpu for Pic16 {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.reset_core();
        self.cycles = 0;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
        if self.asleep {
            // SLEEP idles until an enabled source raises its flag. Waking
            // does not need GIE. The instruction after SLEEP was prefetched,
            // so it runs first; with GIE set the vector is taken after it
            // (DS39582B section 14.14.1).
            self.cycles += 1;
            crate::advance_batch_cycle(bus, 1);
            if !self.source_pending(self.read_data(SFR_INTCON)) {
                return Ok(());
            }
            self.asleep = false;
            return Ok(());
        }

        let pc = self.pc;
        let word = self.fetch_word(pc);
        let observed = !observers.is_empty();
        if observed {
            for obs in observers {
                obs.on_step_start(pc, u32::from(word));
            }
        }

        self.pc = (pc + 1) & PIC16_PC_MASK;
        let cycles = self.execute(word, pc)?;
        self.cycles += u64::from(cycles);
        crate::advance_batch_cycle(bus, u64::from(cycles));

        // Interrupts are sampled between instructions.
        if self.interrupt_request() {
            self.take_interrupt();
        }

        if observed {
            let registers = self.register_view();
            crate::emit_trace_event(
                observers,
                labwired_hw_trace::TraceEvent::InstructionRetired {
                    pc,
                    opcode: u32::from(word),
                },
            );
            for obs in observers {
                obs.on_step_end(cycles, &registers);
            }
        }
        Ok(())
    }

    fn clock_cycles(&self) -> u64 {
        self.cycles
    }

    fn max_step_cycles(&self) -> u32 {
        3
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val & PIC16_PC_MASK;
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, val: u32) {
        self.stkptr = (val % PIC16_STACK_DEPTH as u32) as u8;
    }

    /// Recorded for the `Cpu` contract but not consumed: PIC16 interrupts
    /// come from the INTCON/PIRx flags, which `interrupt_request` samples.
    fn set_exception_pending(&mut self, exception_num: u32) {
        self.exception_pending = Some(exception_num);
    }

    fn get_register(&self, id: u8) -> u32 {
        self.register_view().get(id as usize).copied().unwrap_or(0)
    }

    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0 => self.wreg = val as u8,
            1 => self.data[usize::from(SFR_STATUS)] = val as u8,
            2 => self.data[usize::from(SFR_FSR)] = val as u8,
            3 => self.data[usize::from(SFR_PCLATH)] = val as u8 & 0x1F,
            4 => self.set_sp(val),
            5 => self.set_pc(val),
            _ => {}
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        ["W", "STATUS", "FSR", "PCLATH", "SP", "PC"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        match name.trim().to_ascii_uppercase().as_str() {
            "W" | "WREG" => Some(0),
            "STATUS" => Some(1),
            "FSR" => Some(2),
            "PCLATH" => Some(3),
            "SP" | "STKPTR" => Some(4),
            "PC" => Some(5),
            _ => None,
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::Pic16(Box::new(Pic16CpuSnapshot {
            wreg: self.wreg,
            status: self.status(),
            fsr: self.data[usize::from(SFR_FSR)],
            pclath: self.data[usize::from(SFR_PCLATH)],
            pc: self.pc,
            cycles: self.cycles,
            stkptr: self.stkptr,
            stack: self.stack.to_vec(),
            asleep: self.asleep,
        }))
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Pic16(s) = snapshot {
            self.wreg = s.wreg;
            self.data[usize::from(SFR_STATUS)] = s.status;
            self.data[usize::from(SFR_FSR)] = s.fsr;
            self.data[usize::from(SFR_PCLATH)] = s.pclath & 0x1F;
            self.pc = s.pc & PIC16_PC_MASK;
            self.cycles = s.cycles;
            for (slot, value) in self.stack.iter_mut().zip(s.stack.iter()) {
                *slot = *value & PIC16_PC_MASK;
            }
            self.stkptr = s.stkptr % PIC16_STACK_DEPTH as u8;
            self.asleep = s.asleep;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(cpu: &mut Pic16, count: usize) {
        let mut bus = crate::bus::SystemBus::new();
        let config = SimulationConfig::default();
        for _ in 0..count {
            cpu.step(&mut bus, &[], &config).expect("step");
        }
    }

    fn cpu_with(words: &[u16]) -> Pic16 {
        let mut cpu = Pic16::new();
        cpu.load_words(words);
        cpu
    }

    // Encoders for the hand fixtures (DS39582B Table 15-2).
    fn movlw(k: u8) -> u16 {
        0x3000 | u16::from(k)
    }
    fn movwf(f: u8) -> u16 {
        0x0080 | u16::from(f)
    }
    fn byte_op(op: u16, f: u8, d: bool) -> u16 {
        (op << 8) | (u16::from(d) << 7) | u16::from(f)
    }
    fn bsf(f: u8, b: u8) -> u16 {
        0x1400 | (u16::from(b) << 7) | u16::from(f)
    }
    fn bcf(f: u8, b: u8) -> u16 {
        0x1000 | (u16::from(b) << 7) | u16::from(f)
    }

    #[test]
    fn reset_state_matches_ds39582_por_values() {
        let cpu = Pic16::new();
        assert_eq!(cpu.status(), STATUS_TO | STATUS_PD);
        assert_eq!(cpu.read_data(SFR_OPTION_REG), 0xFF);
        assert_eq!(cpu.read_data(SFR_TRISA + 1), 0xFF);
        assert_eq!(cpu.read_data(SFR_TRISA + 4), 0x07);
        assert_eq!(cpu.read_data(SFR_TXSTA), TXSTA_TRMT);
        assert_eq!(cpu.get_pc(), 0);
        assert_eq!(cpu.flash_words(), 8192);
    }

    #[test]
    fn core_registers_and_shared_gpr_mirror_in_every_bank() {
        let mut cpu = Pic16::new();
        cpu.write_data(0x0A, 0x12);
        for bank in 0..4u16 {
            assert_eq!(
                cpu.read_data(bank * 0x80 + 0x0A),
                0x12,
                "PCLATH bank {bank}"
            );
        }
        cpu.write_data(0x1F5, 0xA5);
        assert_eq!(cpu.read_data(0x75), 0xA5);
        assert_eq!(cpu.read_data(0xF5), 0xA5);
        // Banked GPR is not mirrored.
        cpu.write_data(0x20, 1);
        cpu.write_data(0xA0, 2);
        assert_eq!(cpu.read_data(0x20), 1);
        assert_eq!(cpu.read_data(0xA0), 2);
        // TRISB is visible in bank 3, OPTION_REG too.
        assert_eq!(cpu.read_data(0x186), 0xFF);
        cpu.write_data(0x181, 0x07);
        assert_eq!(cpu.read_data(SFR_OPTION_REG), 0x07);
    }

    #[test]
    fn rp_bits_select_the_bank_for_direct_access() {
        // BSF STATUS,RP0 ; MOVLW 0x5A ; MOVWF 0x20 (-> 0xA0) ; BCF STATUS,RP0
        let mut cpu = cpu_with(&[bsf(3, 5), movlw(0x5A), movwf(0x20), bcf(3, 5)]);
        run(&mut cpu, 4);
        assert_eq!(cpu.read_data(0xA0), 0x5A);
        assert_eq!(cpu.read_data(0x20), 0);
    }

    #[test]
    fn indf_follows_irp_and_fsr() {
        // MOVLW 0x30 ; MOVWF FSR ; BSF STATUS,IRP ; MOVLW 0x77 ; MOVWF INDF
        let mut cpu = cpu_with(&[movlw(0x30), movwf(4), bsf(3, 7), movlw(0x77), movwf(0)]);
        run(&mut cpu, 5);
        assert_eq!(cpu.read_data(0x130), 0x77);
        assert_eq!(cpu.read_data(0x30), 0);
    }

    #[test]
    fn addwf_and_subwf_set_c_dc_z() {
        let mut cpu = Pic16::new();
        cpu.write_data(0x20, 0x0F);
        cpu.wreg = 0xF1;
        cpu.load_words(&[byte_op(0x7, 0x20, true)]);
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x20), 0x00);
        assert_eq!(cpu.status() & STATUS_ALU, STATUS_C | STATUS_DC | STATUS_Z);

        // SUBWF: 0x10 - 0x01 = 0x0F, no borrow (C = 1), nibble borrow (DC = 0).
        let mut cpu = Pic16::new();
        cpu.write_data(0x20, 0x10);
        cpu.wreg = 0x01;
        cpu.load_words(&[byte_op(0x2, 0x20, false)]);
        run(&mut cpu, 1);
        assert_eq!(cpu.wreg, 0x0F);
        assert_eq!(cpu.status() & STATUS_ALU, STATUS_C);

        // SUBLW 0x01 with W = 0x02: borrow, C = 0.
        let mut cpu = cpu_with(&[movlw(2), 0x3C01]);
        run(&mut cpu, 2);
        assert_eq!(cpu.wreg, 0xFF);
        assert_eq!(cpu.status() & STATUS_C, 0);
    }

    #[test]
    fn rotates_go_through_carry_and_swapf_leaves_flags() {
        let mut cpu = Pic16::new();
        cpu.write_data(0x20, 0x81);
        cpu.load_words(&[
            byte_op(0xD, 0x20, true),
            byte_op(0xC, 0x20, true),
            byte_op(0xE, 0x20, false),
        ]);
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x20), 0x02);
        assert_eq!(cpu.status() & STATUS_C, STATUS_C);
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x20), 0x81);
        assert_eq!(cpu.status() & STATUS_C, 0);
        run(&mut cpu, 1);
        assert_eq!(cpu.wreg, 0x18);
    }

    #[test]
    fn status_destination_keeps_alu_flags_from_the_result_and_to_pd_read_only() {
        // CLRF STATUS: the write of 0 is overridden by Z = 1; TO/PD stay.
        let mut cpu = cpu_with(&[byte_op(0x1, 0x03, true)]);
        run(&mut cpu, 1);
        assert_eq!(cpu.status(), STATUS_TO | STATUS_PD | STATUS_Z);
    }

    #[test]
    fn goto_and_call_use_pclath_page_and_return_pops() {
        // MOVLW 0x08 ; MOVWF PCLATH ; CALL 0x010 -> 0x810
        let mut cpu = cpu_with(&[movlw(0x08), movwf(0x0A), 0x2010]);
        cpu.flash[0x810] = 0x3442; // RETLW 0x42
        run(&mut cpu, 3);
        assert_eq!(cpu.get_pc(), 0x810);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 3);
        assert_eq!(cpu.wreg, 0x42);
        // GOTO 0x005 on page 1.
        let mut cpu = cpu_with(&[movlw(0x08), movwf(0x0A), 0x2805]);
        run(&mut cpu, 3);
        assert_eq!(cpu.get_pc(), 0x805);
    }

    #[test]
    fn addwf_pcl_is_a_computed_goto_from_the_next_instruction() {
        // MOVLW 2 ; ADDWF PCL,F ; RETLW 'a' ; RETLW 'b' ; RETLW 'c' (word 4)
        let mut cpu = cpu_with(&[movlw(2), byte_op(0x7, 0x02, true), 0x3461, 0x3462, 0x3463]);
        run(&mut cpu, 2);
        assert_eq!(cpu.get_pc(), 4);
    }

    #[test]
    fn skips_and_decfsz_take_two_cycles() {
        let mut cpu = Pic16::new();
        cpu.write_data(0x20, 1);
        cpu.load_words(&[byte_op(0xB, 0x20, true), 0x0000, 0x0000]);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 2);
        assert_eq!(cpu.clock_cycles(), 2);
        // BTFSC on a clear bit skips.
        let mut cpu = cpu_with(&[0x1820, 0x0000, 0x0000]);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 2);
    }

    #[test]
    fn stack_is_eight_deep_and_circular() {
        let mut cpu = Pic16::new();
        for i in 0..9 {
            cpu.push(0x100 + i);
        }
        // The ninth push overwrote the first slot.
        assert_eq!(cpu.pop(), 0x108);
        for i in (1..8).rev() {
            assert_eq!(cpu.pop(), 0x100 + i);
        }
        assert_eq!(cpu.pop(), 0x108);
    }

    #[test]
    fn port_reads_follow_tris_and_writes_land_in_the_latch() {
        let mut cpu = Pic16::new();
        cpu.write_data(0x06, 0xFF);
        assert_eq!(cpu.read_data(0x06), 0, "inputs read 0 with no driver");
        cpu.write_data(0x86, 0xF0);
        assert_eq!(cpu.read_data(0x06), 0x0F);
        // PORTE has three pins.
        cpu.write_data(0x89, 0x00);
        cpu.write_data(0x09, 0xFF);
        assert_eq!(cpu.read_data(0x09), 0x07);
    }

    #[test]
    fn usart_tx_reaches_the_sink_only_when_enabled() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut cpu = Pic16::new();
        cpu.set_serial_sink(sink.clone());
        cpu.write_data(SFR_TXREG, b'x');
        cpu.write_data(SFR_TXSTA, TXSTA_TXEN);
        cpu.write_data(SFR_TXREG, b'y');
        cpu.write_data(SFR_RCSTA, RCSTA_SPEN);
        cpu.write_data(SFR_TXREG, b'z');
        assert_eq!(&*sink.lock().unwrap(), b"z");
        assert_ne!(cpu.read_data(SFR_TXSTA) & TXSTA_TRMT, 0);
        assert_ne!(cpu.read_data(SFR_PIR1) & PIR1_TXIF, 0);
    }

    #[test]
    fn interrupt_vectors_to_4_and_retfie_sets_gie() {
        let mut cpu = Pic16::new();
        cpu.flash[4] = 0x0009; // RETFIE
                               // INTCON: GIE | INTE | INTF.
        cpu.write_data(SFR_INTCON, INTCON_GIE | 0x10 | 0x02);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 4);
        assert_eq!(cpu.read_data(SFR_INTCON) & INTCON_GIE, 0);
        // Clear the flag inside the "ISR", then return.
        cpu.write_data(SFR_INTCON, 0x10);
        run(&mut cpu, 1);
        // The vector was taken after the instruction at 0 retired.
        assert_eq!(cpu.get_pc(), 1);
        assert_ne!(cpu.read_data(SFR_INTCON) & INTCON_GIE, 0);
    }

    #[test]
    fn peripheral_interrupts_need_peie() {
        let mut cpu = Pic16::new();
        cpu.write_data(SFR_PIE1, PIR1_TXIF);
        cpu.write_data(SFR_TXSTA, TXSTA_TXEN);
        cpu.write_data(SFR_INTCON, INTCON_GIE);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 1);
        cpu.write_data(SFR_INTCON, INTCON_GIE | INTCON_PEIE);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 4);
    }

    #[test]
    fn sleep_clears_pd_and_wakes_on_an_enabled_flag_without_gie() {
        let mut cpu = cpu_with(&[0x0063, 0x0000]);
        run(&mut cpu, 1);
        assert!(cpu.is_asleep());
        assert_eq!(cpu.status() & (STATUS_TO | STATUS_PD), STATUS_TO);
        run(&mut cpu, 3);
        assert!(cpu.is_asleep());
        // RBIE | RBIF with GIE clear: wake and continue after SLEEP.
        cpu.write_data(SFR_INTCON, 0x08 | 0x01);
        run(&mut cpu, 1);
        assert!(!cpu.is_asleep());
        assert_eq!(cpu.get_pc(), 1);
    }

    #[test]
    fn wake_with_gie_runs_the_instruction_after_sleep_before_vectoring() {
        // SLEEP; BSF 0x20,0; ...; the vector at 4 is a NOP.
        let mut cpu = cpu_with(&[0x0063, 0x1420, 0x0000, 0x0000, 0x0000]);
        run(&mut cpu, 1);
        assert!(cpu.is_asleep());
        // GIE | RBIE | RBIF.
        cpu.write_data(SFR_INTCON, 0x80 | 0x08 | 0x01);
        run(&mut cpu, 1);
        assert!(!cpu.is_asleep());
        assert_eq!(cpu.get_pc(), 1, "waking does not vector yet");
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x20) & 1, 1, "post-SLEEP instruction ran");
        assert_eq!(cpu.get_pc(), 4, "then the vector was taken");
    }

    #[test]
    fn unknown_opcodes_are_decode_errors() {
        for word in [0x0001u16, 0x0007, 0x3B00] {
            let mut cpu = cpu_with(&[word]);
            let mut bus = crate::bus::SystemBus::new();
            let err = cpu
                .step(&mut bus, &[], &SimulationConfig::default())
                .unwrap_err();
            assert!(
                matches!(err, SimulationError::DecodeError(0)),
                "{word:#06x}"
            );
        }
    }

    #[test]
    fn program_image_bytes_load_as_words_with_config_and_idlocs() {
        let mut cpu = Pic16::new();
        cpu.load_flash(0, &[0x83, 0x12, 0x8A, 0x01]);
        assert_eq!(cpu.flash[0], 0x1283);
        assert_eq!(cpu.flash[1], 0x018A);
        cpu.load_flash(0x400E, &[0x72, 0x3F]);
        assert_eq!(cpu.config_word(), 0x3F72);
        cpu.load_flash(0x4000, &[0x01, 0x00]);
        assert_eq!(cpu.idloc[0], 0x0001);
        // CONFIG2 (PIC16F88x) and data EEPROM initialisers (__EEPROM_DATA)
        // load instead of being rejected as past-the-end flash.
        cpu.load_flash(0x4010, &[0xFF, 0x3E]);
        assert_eq!(cpu.config[1], 0x3EFF);
        cpu.load_flash(0x4200, &[0x12, 0x00, 0x34, 0x00]);
        assert_eq!(
            (cpu.eedata(0), cpu.eedata(1), cpu.eedata(2)),
            (0x12, 0x34, 0xFF)
        );
    }

    #[test]
    #[should_panic(expected = "fixture exceeds PIC16 model flash")]
    fn bytes_past_flash_are_rejected() {
        let mut small = Pic16::with_device(&Pic16Device {
            name: "t",
            flash_words: 16,
            port_masks: [0xFF; 5],
            aliases: &[],
        });
        small.load_flash(32, &[0]);
    }

    #[test]
    fn snapshot_round_trip_keeps_architectural_state() {
        let mut cpu = cpu_with(&[movlw(0x5A), 0x2005]);
        run(&mut cpu, 1);
        let snap = cpu.snapshot();
        let mut other = Pic16::new();
        other.apply_snapshot(&snap);
        assert_eq!(other.wreg(), 0x5A);
        assert_eq!(other.get_pc(), 1);
        assert_eq!(other.status(), cpu.status());
    }

    #[test]
    fn pic16f873_banks_2_and_3_alias_banks_0_and_1() {
        // DS30292 (PIC16F87X) Figure 2-4 / gputils 16f873_g.lkr: 192 B GPR,
        // bank 2 is bank 0 and bank 3 is bank 1.
        let mut cpu = Pic16::for_chip_name("pic16f873");
        assert_eq!(cpu.flash_words(), 4096);
        cpu.write_data(0x120, 0x11);
        assert_eq!(cpu.read_data(0x20), 0x11);
        cpu.write_data(0xA5, 0x22);
        assert_eq!(cpu.read_data(0x1A5), 0x22);
        // Chained: 0x1F0 -> 0xF0 -> 0x70.
        cpu.write_data(0x1F0, 0x33);
        assert_eq!(cpu.read_data(0x70), 0x33);
        assert_eq!(cpu.read_data(0xF0), 0x33);
        // The 877A keeps banks 2/3 separate.
        let mut big = Pic16::for_chip_name("pic16f877a");
        big.write_data(0x120, 0x11);
        assert_eq!(big.read_data(0x20), 0);
    }

    #[test]
    fn port_masks_follow_the_part() {
        // PIC16F690: PORTB is RB4-RB7 only.
        let mut cpu = Pic16::for_chip_name("pic16f690");
        cpu.write_data(0x86, 0x00);
        cpu.write_data(0x06, 0xFF);
        assert_eq!(cpu.read_data(0x06), 0xF0);
        // PIC16F887: PORTA is 8 bits, PORTE has RE0-RE3.
        let d = pic16_device("pic16f887").expect("887 row");
        assert_eq!(d.port_masks, [0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
        assert_eq!(d.flash_words, 8192);
    }

    #[test]
    fn device_table_knows_the_pic16f877a() {
        let d = pic16_device("PIC16F877A").expect("877A row");
        assert_eq!(d.flash_words, 8192);
        assert!(pic16_device("pic16f1847").is_none());
        assert!(check_pic16_entry(0).is_ok());
        assert!(check_pic16_entry(1).is_err());
    }
}
