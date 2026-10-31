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
//! The same type also runs the **enhanced mid-range** core (PIC16F1xxx,
//! 49 instructions; DS40001440E chapter 3 and Table 29-3) when the device
//! row says so:
//! - The PC is 15 bits and `PCLATH` is 7. `GOTO`/`CALL` take their page
//!   from `PCLATH<6:3>`.
//! - Data memory is 32 banks of 128 bytes, selected by `BSR`. Every bank
//!   starts with the twelve core registers (`INDF0`/`INDF1`, `PCL`,
//!   `STATUS`, `FSR0L/H`, `FSR1L/H`, `BSR`, `WREG`, `PCLATH`, `INTCON`) and
//!   ends with the 16-byte common RAM at 0x70-0x7F.
//! - The 16-bit `FSR0`/`FSR1` see three spaces: the banked map
//!   (0x0000-0x0FFF), linear GPR (0x2000-0x29AF, 80 bytes per bank), and
//!   program flash (0x8000-0xFFFF, low byte of each word, read-only).
//! - The return stack is 16 levels with `STKPTR`/`TOSL`/`TOSH` in bank 31.
//!   An overflow or underflow sets `PCON.STKOVF`/`STKUNF` and, with
//!   `CONFIG2.STVREN`, resets the core.
//! - Interrupt entry saves `W`, `STATUS`, `BSR`, `PCLATH` and both FSRs into
//!   the bank-31 shadow registers, and `RETFIE` restores them.
//!
//! Not modelled on the enhanced core: the extra cycle for a program-flash
//! read through an FSR, the 64-bank parts (PIC16F188xx, 6-bit `BSR`), and
//! every peripheral except the EUSART TX path, the ports, TMR0 and the
//! oscillator ready flags. An unknown opcode returns [`SimulationError::DecodeError`];
//! it is never guessed.

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

/// Enhanced mid-range: 15-bit program counter.
pub const PIC16E_PC_MASK: u32 = 0x7FFF;
/// Enhanced mid-range hardware return stack depth.
pub const PIC16E_STACK_DEPTH: usize = 16;
/// Enhanced mid-range user ID words (word addresses 0x8000-0x8003).
pub const PIC16E_IDLOC_BASE: u32 = 0x8000;
/// Enhanced mid-range CONFIG1 (word address); CONFIG2 follows.
pub const PIC16E_CONFIG_WORD: u32 = 0x8007;
/// Enhanced mid-range data EEPROM initialisers (word addresses 0xF000-0xF0FF).
pub const PIC16E_EEDATA_BASE: u32 = 0xF000;
/// Enhanced mid-range banks (BSR<4:0>).
pub const PIC16E_BANKS: usize = 32;
/// The enhanced mid-range "STKPTR = empty" value.
const STKPTR_EMPTY: u8 = 0x1F;
/// Return-stack storage: the larger of the two cores.
const STACK_SLOTS: usize = PIC16E_STACK_DEPTH;

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

// Enhanced mid-range core registers (offsets 0x00-0x0B of every bank).
pub const SFR_INDF0: u16 = 0x00;
pub const SFR_INDF1: u16 = 0x01;
pub const SFR_FSR0L: u16 = 0x04;
pub const SFR_FSR0H: u16 = 0x05;
pub const SFR_FSR1L: u16 = 0x06;
pub const SFR_FSR1H: u16 = 0x07;
pub const SFR_BSR: u16 = 0x08;
pub const SFR_WREG: u16 = 0x09;
// Enhanced mid-range bank 31: shadows (STATUS, WREG, BSR, PCLATH, FSR0L,
// FSR0H, FSR1L, FSR1H in that order) and the stack window.
pub const SFR_STATUS_SHAD: u16 = 0xFE4;
pub const SFR_STKPTR: u16 = 0xFED;
pub const SFR_TOSL: u16 = 0xFEE;
pub const SFR_TOSH: u16 = 0xFEF;

const PCON_STKOVF: u8 = 1 << 7;
const PCON_STKUNF: u8 = 1 << 6;
const PCON_RI: u8 = 1 << 2;
/// PCON after a power-on reset: RWDT = RMCLR = RI = 1 (DS40001440E Table 6-4).
const PCON_POR: u8 = 0x1C;
/// CONFIG2.STVREN: a stack overflow/underflow resets the core.
const CONFIG2_STVREN: u16 = 1 << 9;

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
/// Enhanced mid-range STATUS: bits 7-5 are unimplemented (bank select moved
/// to BSR).
const STATUS_E_IMPLEMENTED: u8 = 0x1F;
const STATUS_ALU: u8 = STATUS_C | STATUS_DC | STATUS_Z;

const INTCON_GIE: u8 = 1 << 7;
const INTCON_PEIE: u8 = 1 << 6;
const INTCON_T0IF: u8 = 1 << 2;

// OPTION_REG: T0CS (1 = T0CKI pin), PSA (1 = prescaler not on TMR0), PS<2:0>.
const OPTION_T0CS: u8 = 1 << 5;
const OPTION_PSA: u8 = 1 << 3;

const TXSTA_TXEN: u8 = 1 << 5;
const TXSTA_TRMT: u8 = 1 << 1;
const RCSTA_SPEN: u8 = 1 << 7;
const PIR1_TXIF: u8 = 1 << 4;

/// Where the registers this model implements live on one SFR layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pic16SfrMap {
    pub porta: u16,
    pub trisa: u16,
    /// LATA; the classic core has no LAT registers.
    pub lata: Option<u16>,
    /// PIR1.. and the matching PIE1.. (same length).
    pub pir: &'static [u16],
    pub pie: &'static [u16],
    pub tmr0: u16,
    pub option_reg: u16,
    pub rcsta: u16,
    pub txreg: u16,
    pub txsta: u16,
    pub spbrg: u16,
    /// PCON (stack overflow/underflow and RESET-instruction flags).
    pub pcon: Option<u16>,
    /// The oscillator status register and the ready bits it always reads
    /// set: the clocks are not modelled, so they are ready at once.
    pub osc_ready: Option<(u16, u8)>,
}

/// The classic mid-range layout (PIC16F877A, DS39582B Figure 2-3).
pub const PIC16_CLASSIC_MAP: Pic16SfrMap = Pic16SfrMap {
    porta: SFR_PORTA,
    trisa: SFR_TRISA,
    lata: None,
    pir: &[SFR_PIR1, SFR_PIR2],
    pie: &[SFR_PIE1, SFR_PIE2],
    tmr0: SFR_TMR0,
    option_reg: SFR_OPTION_REG,
    rcsta: SFR_RCSTA,
    txreg: SFR_TXREG,
    txsta: SFR_TXSTA,
    spbrg: SFR_SPBRG,
    pcon: None,
    osc_ready: None,
};

/// The PIC16F182x/F193x enhanced mid-range layout (DS40001440E Table 3-8):
/// ports at 0x00C, TRIS at 0x08C, LAT at 0x10C, PIR1-4 at 0x011, PIE1-4 at
/// 0x091, TMR0 0x015, OPTION_REG 0x095, PCON 0x096, OSCSTAT 0x09A (HFIOFR, HFIOFL,
/// MFIOFR, LFIOFR, HFIOFS) and the EUSART at 0x199-0x19F.
pub const PIC16E_F182X_MAP: Pic16SfrMap = Pic16SfrMap {
    porta: 0x00C,
    trisa: 0x08C,
    lata: Some(0x10C),
    pir: &[0x011, 0x012, 0x013, 0x014],
    pie: &[0x091, 0x092, 0x093, 0x094],
    tmr0: 0x015,
    option_reg: 0x095,
    rcsta: 0x19D,
    txreg: 0x19A,
    txsta: 0x19E,
    spbrg: 0x19B,
    pcon: Some(0x096),
    osc_ready: Some((0x09A, 0x1F)),
};

/// The PIC16F145x/150x/151x: the F182x layout without the MFINTOSC, so
/// OSCSTAT has no MFIOFR (bit 2) or HFIOFL (bit 3), which read 0
/// (per the parts' SDCC 4.4.0 pic14 headers).
pub const PIC16E_F151X_MAP: Pic16SfrMap = Pic16SfrMap {
    osc_ready: Some((0x09A, 0x13)),
    ..PIC16E_F182X_MAP
};

/// One mid-range part: core, program words, implemented port bits
/// (PORTA..PORTE), SFR layout and the GPR address ranges that alias
/// another range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pic16Device {
    pub name: &'static str,
    /// The enhanced mid-range (PIC16F1xxx) core instead of the classic one.
    pub enhanced: bool,
    pub flash_words: usize,
    pub port_masks: [u8; PIC16_PORT_COUNT as usize],
    pub map: Pic16SfrMap,
    /// `(start, end, target)`: banked addresses `start..=end` are the same
    /// bytes as `target..`. Taken from the gputils `SHAREBANK` groups (the
    /// 0x70-0x7F block on most parts; whole banks on the PIC16F873/874).
    /// The enhanced core's core registers and common RAM are built in.
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
        enhanced: false,
        flash_words,
        port_masks,
        map: PIC16_CLASSIC_MAP,
        aliases,
    }
}

/// An enhanced mid-range row.
const fn edev(
    name: &'static str,
    flash_words: usize,
    port_masks: [u8; PIC16_PORT_COUNT as usize],
    map: Pic16SfrMap,
) -> Pic16Device {
    Pic16Device {
        name,
        enhanced: true,
        flash_words,
        port_masks,
        map,
        aliases: &[],
    }
}

/// The parts this model knows. A row means every SFR address and bit the
/// model implements matches the part's SDCC 4.4.0 `pic14` header; flash
/// words come from the gputils `<part>_g.lkr` code pages, port masks from
/// the header's `R<port><bit>` fields and aliases from the lkr `SHAREBANK`
/// groups. `scripts/pic16/check_sfr_map.py --check-table` regenerates and
/// re-verifies every row. A row is geometry, not onboarding.
///
/// Enhanced mid-range rows (`edev`) must match `PIC16E_F182X_MAP`
/// (`--enhanced`): core registers, bank-31 shadows and stack, INTCON,
/// OPTION_REG, PCON, PIR1-4/PIE1-4, PORT/TRIS/LAT A-E and the EUSART at
/// 0x19A-0x19E, with any address the model touches either the same register
/// or free. The OSCSTAT ready mask picks the map.
///
/// Left out: classic parts without the USART at 0x18/0x19/0x98/0x99 or
/// without PORTC (PIC16F6xx/8x/81x/84A and friends), the OTP/EPROM PIC16C
/// parts; enhanced parts without an EUSART (PIC16F1503/1507/1613/1703/1707),
/// with PIR1 at 0x010 (PIC16F161x), with comparator SFRs at LATD/LATE
/// (PIC16F176x), the 64-bank PIC16F18xxx/19xxx, and for now the
/// PIC16F157x (their headers match, but gputils 1.4.0 has no processor
/// entry for them, so no lkr cross-check or SDCC firmware build).
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
    // Enhanced mid-range (32 banks). Every row passes
    // `check_sfr_map.py --enhanced` against PIC16E_F182X_MAP (OSCSTAT ready
    // mask aside); flash words from SDCC's pic14devices.txt.
    edev(
        "pic16f1454",
        8192,
        [0x3B, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16f1455",
        8192,
        [0x3B, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16f1459",
        8192,
        [0x3B, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16f1508",
        4096,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16f1509",
        8192,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16f1516",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16f1517",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16f1518",
        16384,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16f1519",
        16384,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F151X_MAP,
    ),
    // SDCC 4.4.0's pic14devices.txt lacks `enhanced 1` for 16(L)F1704/1708,
    // so SDCC emits classic code for them that gpasm rejects. Rebuilding
    // firmware for these parts needs a locally patched device DB.
    edev(
        "pic16f1704",
        4096,
        [0x3F, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1705",
        8192,
        [0x3F, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1708",
        4096,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1709",
        8192,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1713",
        4096,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1716",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1717",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1718",
        16384,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1719",
        16384,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1784",
        4096,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1786",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1787",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1788",
        16384,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1789",
        16384,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1823",
        2048,
        [0x3F, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1824",
        4096,
        [0x3F, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1825",
        8192,
        [0x3F, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1826",
        2048,
        [0xFF, 0xFF, 0x00, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1827",
        4096,
        [0xFF, 0xFF, 0x00, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1828",
        4096,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1829",
        8192,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1847",
        8192,
        [0xFF, 0xFF, 0x00, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1933",
        4096,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1934",
        4096,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1936",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1937",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1938",
        16384,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1939",
        16384,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1946",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16f1947",
        16384,
        [0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1454",
        8192,
        [0x3B, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16lf1455",
        8192,
        [0x3B, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16lf1459",
        8192,
        [0x3B, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16lf1508",
        4096,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16lf1509",
        8192,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16lf1516",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16lf1517",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16lf1518",
        16384,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16lf1519",
        16384,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F151X_MAP,
    ),
    edev(
        "pic16lf1704",
        4096,
        [0x3F, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1705",
        8192,
        [0x3F, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1708",
        4096,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1709",
        8192,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1713",
        4096,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1716",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1717",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1718",
        16384,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1719",
        16384,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1784",
        4096,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1786",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1787",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1788",
        16384,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1789",
        16384,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1823",
        2048,
        [0x3F, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1824",
        4096,
        [0x3F, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1825",
        8192,
        [0x3F, 0x00, 0x3F, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1826",
        2048,
        [0xFF, 0xFF, 0x00, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1827",
        4096,
        [0xFF, 0xFF, 0x00, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1828",
        4096,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1829",
        8192,
        [0x3F, 0xF0, 0xFF, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1847",
        8192,
        [0xFF, 0xFF, 0x00, 0x00, 0x00],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1933",
        4096,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1934",
        4096,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1936",
        8192,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1937",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1938",
        16384,
        [0xFF, 0xFF, 0xFF, 0x00, 0x08],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1939",
        16384,
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1946",
        8192,
        [0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        PIC16E_F182X_MAP,
    ),
    edev(
        "pic16lf1947",
        16384,
        [0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        PIC16E_F182X_MAP,
    ),
];

/// What a data-space access through an FSR (or a direct `INDFn`) lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// A banked data address (already canonical).
    Data(u16),
    /// A program-flash word (enhanced FSR 0x8000-0xFFFF): reads the low byte.
    Flash(u32),
    /// Unimplemented, or `INDFn` addressed through an FSR: reads 0, writes
    /// are ignored.
    Void,
}

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
    /// The enhanced mid-range core (see the module docs).
    enhanced: bool,
    map: Pic16SfrMap,
    flash: Vec<u16>,
    data: Vec<u8>,
    /// PORTA..PORTE output latches (LATx on the enhanced core). Writes to
    /// PORTx land here; reads see the pins.
    latch: [u8; PIC16_PORT_COUNT as usize],
    port_masks: [u8; PIC16_PORT_COUNT as usize],
    aliases: &'static [(u16, u16, u16)],
    stack: [u32; STACK_SLOTS],
    /// Classic: next free slot, modulo 8. Enhanced: `STKPTR`, the slot of
    /// the top of stack, with 0x1F meaning empty.
    stkptr: u8,
    /// A reset requested mid-instruction (enhanced `RESET`, or a stack
    /// overflow/underflow with STVREN): `(PCON bits to set, to clear)`.
    pending_reset: Option<(u8, u8)>,
    /// Instruction cycles counted toward the next TMR0 increment while the
    /// prescaler is assigned to TMR0.
    tmr0_prescale: u16,
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
        let data_bytes = if device.enhanced {
            PIC16E_BANKS * 0x80
        } else {
            PIC16_DATA_BYTES
        };
        let mut cpu = Self {
            wreg: 0,
            pc: 0,
            cycles: 0,
            enhanced: device.enhanced,
            map: device.map,
            flash: vec![PIC16_WORD_MASK; device.flash_words],
            data: vec![0; data_bytes],
            latch: [0; PIC16_PORT_COUNT as usize],
            port_masks: device.port_masks,
            aliases: device.aliases,
            stack: [0; STACK_SLOTS],
            stkptr: 0,
            pending_reset: None,
            tmr0_prescale: 0,
            exception_pending: None,
            asleep: false,
            idloc: [PIC16_WORD_MASK; 4],
            config: [PIC16_WORD_MASK; 2],
            eedata: [0xFF; PIC16_EEDATA_BYTES],
            serial_sink: None,
        };
        cpu.reset_core();
        if let Some(pcon) = cpu.map.pcon {
            cpu.data[usize::from(pcon)] = PCON_POR;
        }
        cpu
    }

    /// The chip's own geometry, or the default core for an unknown name.
    pub fn for_chip_name(name: &str) -> Self {
        match pic16_device(name) {
            Some(device) => Self::with_device(device),
            None => {
                tracing::warn!("PIC16: no device row for chip {name:?}; using PIC16F877A geometry");
                Self::new()
            }
        }
    }

    /// Route USART TX bytes to `sink` (the console / `uart_contains` buffer).
    pub fn set_serial_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>) {
        self.serial_sink = Some(sink);
    }

    /// Whether this is the enhanced mid-range core.
    pub fn is_enhanced(&self) -> bool {
        self.enhanced
    }

    /// The SFR layout this core decodes.
    pub fn sfr_map(&self) -> &Pic16SfrMap {
        &self.map
    }

    /// Program memory size in words.
    pub fn flash_words(&self) -> usize {
        self.flash.len()
    }

    /// The configuration word (CONFIG1 on the enhanced core) as loaded.
    pub fn config_word(&self) -> u16 {
        self.config[0]
    }

    /// The second configuration word (CONFIG2), erased (0x3FFF) if the image
    /// has none.
    pub fn config_word2(&self) -> u16 {
        self.config[1]
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

    /// `FSR0` (`n = 0`) or `FSR1` as a 16-bit address (enhanced core).
    pub fn fsr(&self, n: u16) -> u16 {
        let lo = usize::from(SFR_FSR0L + 2 * n);
        u16::from_le_bytes([self.data[lo], self.data[lo + 1]])
    }

    fn set_fsr(&mut self, n: u16, value: u16) {
        let lo = usize::from(SFR_FSR0L + 2 * n);
        let [l, h] = value.to_le_bytes();
        self.data[lo] = l;
        self.data[lo + 1] = h;
    }

    fn pc_mask(&self) -> u32 {
        if self.enhanced {
            PIC16E_PC_MASK
        } else {
            PIC16_PC_MASK
        }
    }

    fn pclath_mask(&self) -> u8 {
        if self.enhanced {
            0x7F
        } else {
            0x1F
        }
    }

    fn stack_depth(&self) -> usize {
        if self.enhanced {
            PIC16E_STACK_DEPTH
        } else {
            PIC16_STACK_DEPTH
        }
    }

    /// Power-on reset values for the registers this model implements
    /// (DS39582B Table 14-6; DS40001440E Table 3-8 for the enhanced core).
    /// General purpose RAM is left as it is.
    fn reset_core(&mut self) {
        self.wreg = 0;
        self.pc = 0;
        self.stack = [0; STACK_SLOTS];
        self.stkptr = if self.enhanced { STKPTR_EMPTY } else { 0 };
        self.pending_reset = None;
        self.tmr0_prescale = 0;
        self.asleep = false;
        self.latch = [0; PIC16_PORT_COUNT as usize];
        let map = self.map;
        // STATUS 0001 1xxx: TO = PD = 1, bank 0.
        self.data[usize::from(SFR_STATUS)] = STATUS_TO | STATUS_PD;
        self.data[usize::from(SFR_PCLATH)] = 0;
        self.data[usize::from(SFR_INTCON)] = 0;
        if self.enhanced {
            for r in [SFR_FSR0L, SFR_FSR0H, SFR_FSR1L, SFR_FSR1H, SFR_BSR] {
                self.data[usize::from(r)] = 0;
            }
        } else {
            self.data[usize::from(SFR_FSR)] = 0;
        }
        for (&pir, &pie) in map.pir.iter().zip(map.pie) {
            self.data[usize::from(pir)] = 0;
            self.data[usize::from(pie)] = 0;
        }
        self.data[usize::from(map.option_reg)] = 0xFF;
        for n in 0..PIC16_PORT_COUNT {
            self.data[usize::from(map.trisa + n)] = 0xFF;
        }
        if !self.enhanced {
            // TRISE 0000 -111: PSP mode off, RE0-RE2 inputs.
            self.data[usize::from(SFR_TRISA + 4)] = 0x07;
            self.data[usize::from(SFR_ADCON1)] = 0;
        }
        self.data[usize::from(map.txsta)] = TXSTA_TRMT;
        self.data[usize::from(map.rcsta)] = 0;
        self.data[usize::from(map.spbrg)] = 0;
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
    /// EEPROM initialisers (classic 0x2100-0x21FF, enhanced 0xF000-0xF0FF)
    /// in the EEPROM image; any other address past the part's flash is a
    /// fixture error and panics. [`Self::try_load_flash`] reports it instead.
    pub fn load_flash(&mut self, addr: u32, bytes: &[u8]) {
        if let Err(e) = self.try_load_flash(addr, bytes) {
            panic!("{e}");
        }
    }

    /// [`Self::load_flash`], returning an error for a byte this part has no
    /// home for (for example an enhanced mid-range image, CONFIG at word
    /// 0x8007, on a classic part). Bytes before the bad one are loaded.
    pub fn try_load_flash(&mut self, addr: u32, bytes: &[u8]) -> Result<(), String> {
        let (idloc, config, eedata) = if self.enhanced {
            (PIC16E_IDLOC_BASE, PIC16E_CONFIG_WORD, PIC16E_EEDATA_BASE)
        } else {
            (PIC16_IDLOC_BASE, PIC16_CONFIG_WORD, PIC16_EEDATA_BASE)
        };
        for (i, byte) in bytes.iter().enumerate() {
            let at = addr + i as u32;
            let word = at / 2;
            let high = at % 2 == 1;
            if (eedata..eedata + PIC16_EEDATA_BYTES as u32).contains(&word) {
                // One data byte per word; the high byte of the word is padding.
                if !high {
                    self.eedata[(word - eedata) as usize] = *byte;
                }
                continue;
            }
            let slot = match word {
                w if (w as usize) < self.flash.len() => &mut self.flash[w as usize],
                w if (idloc..idloc + 4).contains(&w) => &mut self.idloc[(w - idloc) as usize],
                w if (config..config + 2).contains(&w) => &mut self.config[(w - config) as usize],
                // Between the ID words and CONFIG1: reserved / revision and
                // device ID, not programmable.
                w if (idloc..=config).contains(&w) => continue,
                _ => {
                    return Err(format!(
                        "fixture exceeds PIC16 model flash (byte {at:#x}, word {word:#x}; \
                         {} has {} words)",
                        if self.enhanced {
                            "this enhanced mid-range part"
                        } else {
                            "this classic mid-range part"
                        },
                        self.flash.len()
                    ))
                }
            };
            *slot = if high {
                (*slot & 0x00FF) | (u16::from(*byte) << 8)
            } else {
                (*slot & 0xFF00) | u16::from(*byte)
            } & PIC16_WORD_MASK;
        }
        Ok(())
    }

    /// Load a program image (byte addresses). The PC starts at the reset
    /// vector. An image with bytes outside the part's program, ID, config
    /// and EEPROM spaces is an error, not a panic: it is user firmware.
    pub fn load_program_image(
        &mut self,
        image: &crate::memory::ProgramImage,
    ) -> Result<(), String> {
        for seg in &image.segments {
            self.try_load_flash(seg.start_addr as u32, &seg.data)?;
        }
        self.pc = 0;
        Ok(())
    }

    /// Map a banked address onto the register that backs it.
    fn canonical(&self, addr: u16) -> u16 {
        if self.enhanced {
            let addr = addr & (self.data.len() as u16 - 1);
            return match addr & 0x7F {
                // Core registers and common RAM appear in every bank.
                off @ (0x00..=0x0B | 0x70..=0x7F) => off,
                _ => addr,
            };
        }
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

    /// Direct file address for a 7-bit `f` in the bank `RP1:RP0` (classic)
    /// or `BSR` (enhanced) selects.
    pub fn file_address(&self, f: u8) -> u16 {
        let bank = if self.enhanced {
            u16::from(self.data[usize::from(SFR_BSR)])
        } else {
            u16::from((self.status() >> 5) & 0x03)
        };
        (bank << 7) | u16::from(f & 0x7F)
    }

    fn indirect_address(&self) -> u16 {
        let irp = u16::from(self.status() & STATUS_IRP != 0);
        (irp << 8) | u16::from(self.data[usize::from(SFR_FSR)])
    }

    /// What a 16-bit enhanced FSR value addresses.
    fn fsr_target(&self, fsr: u16) -> Target {
        match fsr {
            0x0000..=0x0FFF if usize::from(fsr) < self.data.len() => match self.canonical(fsr) {
                SFR_INDF0 | SFR_INDF1 => Target::Void,
                a => Target::Data(a),
            },
            // Linear GPR: 80 bytes per bank from offset 0x20; bank 31 has none.
            0x2000..=0x29AF => {
                let n = fsr - 0x2000;
                let bank = n / 80;
                Target::Data(self.canonical((bank << 7) | (0x20 + n % 80)))
            }
            0x8000..=0xFFFF => Target::Flash(u32::from(fsr & 0x7FFF)),
            _ => Target::Void,
        }
    }

    /// Resolve `f`, following `INDF` through `IRP:FSR` (classic) or `INDFn`
    /// through `FSRn` (enhanced).
    fn resolve(&self, f: u8) -> Target {
        let addr = self.canonical(self.file_address(f));
        if self.enhanced {
            match addr {
                SFR_INDF0 => self.fsr_target(self.fsr(0)),
                SFR_INDF1 => self.fsr_target(self.fsr(1)),
                _ => Target::Data(addr),
            }
        } else if addr == SFR_INDF {
            Target::Data(self.canonical(self.indirect_address()))
        } else {
            Target::Data(addr)
        }
    }

    fn read_target(&self, target: Target) -> u8 {
        match target {
            Target::Data(addr) => self.read_data(addr),
            Target::Flash(word) => self.fetch_word(word) as u8,
            Target::Void => 0,
        }
    }

    fn write_target(&mut self, target: Target, value: u8) {
        if let Target::Data(addr) = target {
            self.write_data(addr, value);
        }
    }

    /// The port index of a PORTx (`false`) or LATx (`true`) address.
    fn port_index(&self, addr: u16) -> Option<(usize, bool)> {
        if (self.map.porta..self.map.porta + PIC16_PORT_COUNT).contains(&addr) {
            return Some((usize::from(addr - self.map.porta), false));
        }
        match self.map.lata {
            Some(lata) if (lata..lata + PIC16_PORT_COUNT).contains(&addr) => {
                Some((usize::from(addr - lata), true))
            }
            _ => None,
        }
    }

    /// The top-of-stack slot (enhanced `TOSL`/`TOSH`), if the stack is not
    /// empty.
    fn tos_slot(&self) -> Option<usize> {
        let slot = usize::from(self.stkptr);
        (slot < PIC16E_STACK_DEPTH).then_some(slot)
    }

    /// Read a data-space register by banked address.
    pub fn read_data(&self, addr: u16) -> u8 {
        let addr = self.canonical(addr);
        let map = self.map;
        if self.enhanced {
            match addr {
                // INDFn addressed directly (not through an instruction) reads 0.
                SFR_INDF0 | SFR_INDF1 => return 0,
                SFR_STATUS => return self.data[usize::from(SFR_STATUS)] & STATUS_E_IMPLEMENTED,
                SFR_WREG => return self.wreg,
                SFR_STKPTR => return self.stkptr,
                SFR_TOSL => return self.tos_slot().map_or(0, |s| self.stack[s] as u8),
                SFR_TOSH => {
                    return self
                        .tos_slot()
                        .map_or(0, |s| (self.stack[s] >> 8) as u8 & 0x7F)
                }
                _ => {}
            }
            if let Some((osc, ready)) = map.osc_ready {
                if addr == osc {
                    return self.data[usize::from(addr)] | ready;
                }
            }
        }
        if let Some((n, lat)) = self.port_index(addr) {
            // LATx reads the latch. A PORTx pin set as output reads its
            // latch; an input reads 0 because nothing external drives it yet.
            let tris = self.data[usize::from(map.trisa) + n];
            return if lat {
                self.latch[n]
            } else {
                self.latch[n] & !tris & self.port_masks[n]
            };
        }
        match addr {
            SFR_INDF if !self.enhanced => 0,
            SFR_PCL => self.pc as u8,
            // TXIF is read-only and set while TXREG is empty; a byte leaves
            // the instant it is written here, so it follows TXEN.
            a if a == map.pir[0] => {
                let pir1 = self.data[usize::from(a)] & !PIR1_TXIF;
                if self.data[usize::from(map.txsta)] & TXSTA_TXEN != 0 {
                    pir1 | PIR1_TXIF
                } else {
                    pir1
                }
            }
            _ => self.data[usize::from(addr)],
        }
    }

    /// Write a data-space register by banked address.
    pub fn write_data(&mut self, addr: u16, value: u8) {
        let addr = self.canonical(addr);
        let map = self.map;
        if self.enhanced {
            match addr {
                SFR_INDF0 | SFR_INDF1 => return,
                SFR_STATUS => {
                    // Only C, DC and Z are writable; TO/PD are read-only.
                    let old = self.data[usize::from(SFR_STATUS)];
                    self.data[usize::from(SFR_STATUS)] = (old & !STATUS_ALU) | (value & STATUS_ALU);
                    return;
                }
                SFR_WREG => {
                    self.wreg = value;
                    return;
                }
                SFR_BSR => {
                    self.data[usize::from(SFR_BSR)] = value & (PIC16E_BANKS as u8 - 1);
                    return;
                }
                SFR_STKPTR => {
                    self.stkptr = value & 0x1F;
                    return;
                }
                SFR_TOSL | SFR_TOSH => {
                    if let Some(s) = self.tos_slot() {
                        let v = self.stack[s];
                        self.stack[s] = if addr == SFR_TOSL {
                            (v & 0x7F00) | u32::from(value)
                        } else {
                            (v & 0x00FF) | (u32::from(value & 0x7F) << 8)
                        };
                    }
                    return;
                }
                _ => {}
            }
        }
        if let Some((n, _)) = self.port_index(addr) {
            self.latch[n] = value & self.port_masks[n];
            return;
        }
        match addr {
            SFR_INDF if !self.enhanced => {}
            SFR_PCL => {
                let pclath = u32::from(self.data[usize::from(SFR_PCLATH)]);
                self.pc = ((pclath << 8) | u32::from(value)) & self.pc_mask();
            }
            SFR_STATUS => {
                let old = self.data[usize::from(SFR_STATUS)];
                self.data[usize::from(SFR_STATUS)] =
                    (old & !STATUS_WRITABLE) | (value & STATUS_WRITABLE);
            }
            SFR_PCLATH => self.data[usize::from(SFR_PCLATH)] = value & self.pclath_mask(),
            a if a == map.pir[0] => self.data[usize::from(a)] = value & !PIR1_TXIF,
            // A TMR0 write also clears the prescaler count.
            a if a == map.tmr0 => {
                self.data[usize::from(a)] = value;
                self.tmr0_prescale = 0;
            }
            // TRMT is read-only and stays set.
            a if a == map.txsta => self.data[usize::from(a)] = value | TXSTA_TRMT,
            a if a == map.txreg => {
                self.data[usize::from(a)] = value;
                let txsta = self.data[usize::from(map.txsta)];
                let rcsta = self.data[usize::from(map.rcsta)];
                if txsta & TXSTA_TXEN != 0 && rcsta & RCSTA_SPEN != 0 {
                    if let Some(sink) = &self.serial_sink {
                        if let Ok(mut buf) = sink.lock() {
                            buf.push(value);
                        }
                    }
                }
            }
            _ => self.data[usize::from(addr)] = value,
        }
    }

    /// Read `f` for an instruction (bank select + `INDF`).
    pub fn read_file(&self, f: u8) -> u8 {
        self.read_target(self.resolve(f))
    }

    /// Write `f` for an instruction (bank select + `INDF`).
    pub fn write_file(&mut self, f: u8, value: u8) {
        let target = self.resolve(f);
        self.write_target(target, value);
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
        Self::add_carry_flags(a, b, 0)
    }

    /// `a + b + carry_in` (ADDWFC).
    fn add_carry_flags(a: u8, b: u8, carry_in: u8) -> (u8, u8) {
        let result = a.wrapping_add(b).wrapping_add(carry_in);
        let mut set = Self::z(result);
        if u16::from(a) + u16::from(b) + u16::from(carry_in) > 0xFF {
            set |= STATUS_C;
        }
        if (a & 0x0F) + (b & 0x0F) + carry_in > 0x0F {
            set |= STATUS_DC;
        }
        (result, set)
    }

    /// `a - b`: C and DC are "no borrow".
    fn sub_flags(a: u8, b: u8) -> (u8, u8) {
        Self::sub_borrow_flags(a, b, 0)
    }

    /// `a - b - borrow_in` (SUBWFB, where `borrow_in = !C`).
    fn sub_borrow_flags(a: u8, b: u8, borrow_in: u8) -> (u8, u8) {
        let result = a.wrapping_sub(b).wrapping_sub(borrow_in);
        let mut set = Self::z(result);
        if u16::from(a) >= u16::from(b) + u16::from(borrow_in) {
            set |= STATUS_C;
        }
        if (a & 0x0F) >= (b & 0x0F) + borrow_in {
            set |= STATUS_DC;
        }
        (result, set)
    }

    /// Push a return address. Returns `false` if the push overflowed into
    /// a STVREN reset (the caller must not change the PC further).
    fn push(&mut self, value: u32) -> bool {
        let value = value & self.pc_mask();
        if !self.enhanced {
            self.stack[usize::from(self.stkptr)] = value;
            self.stkptr = (self.stkptr + 1) % PIC16_STACK_DEPTH as u8;
            return true;
        }
        if usize::from(self.stkptr) == PIC16E_STACK_DEPTH - 1 {
            // Overflow: the 17th push (DS40001440E section 3.5.2).
            if self.stack_fault(PCON_STKOVF) {
                return false;
            }
        }
        self.stkptr = self.stkptr.wrapping_add(1) % PIC16E_STACK_DEPTH as u8;
        self.stack[usize::from(self.stkptr)] = value;
        true
    }

    /// Pop a return address; `None` if the pop underflowed into a STVREN
    /// reset.
    fn pop(&mut self) -> Option<u32> {
        if !self.enhanced {
            self.stkptr = (self.stkptr + PIC16_STACK_DEPTH as u8 - 1) % PIC16_STACK_DEPTH as u8;
            return Some(self.stack[usize::from(self.stkptr)]);
        }
        if self.stkptr == STKPTR_EMPTY && self.stack_fault(PCON_STKUNF) {
            return None;
        }
        let slot = usize::from(self.stkptr) % PIC16E_STACK_DEPTH;
        let value = self.stack[slot];
        self.stkptr = if slot == 0 {
            STKPTR_EMPTY
        } else {
            slot as u8 - 1
        };
        Some(value)
    }

    /// Record a stack overflow/underflow in PCON; with STVREN, request a
    /// reset and return `true`.
    fn stack_fault(&mut self, flag: u8) -> bool {
        if let Some(pcon) = self.map.pcon {
            self.data[usize::from(pcon)] |= flag;
        }
        if self.config[1] & CONFIG2_STVREN != 0 {
            self.pending_reset = Some((flag, 0));
            return true;
        }
        false
    }

    /// Return through the stack: the popped address, or stay put if the pop
    /// requested a reset.
    fn ret(&mut self) {
        if let Some(pc) = self.pop() {
            self.pc = pc;
        }
    }

    fn fetch_word(&self, pc: u32) -> u16 {
        // Unimplemented program memory reads as erased (0x3FFF, ADDLW 0xFF).
        self.flash
            .get(pc as usize)
            .copied()
            .unwrap_or(PIC16_WORD_MASK)
    }

    /// `PCLATH<4:3>` (classic) or `PCLATH<6:3>` (enhanced) page for
    /// `GOTO`/`CALL`.
    fn page(&self) -> u32 {
        let mask = if self.enhanced { 0x78 } else { 0x18 };
        u32::from(self.data[usize::from(SFR_PCLATH)] & mask) << 8
    }

    fn interrupt_request(&self) -> bool {
        let intcon = self.read_data(SFR_INTCON);
        intcon & INTCON_GIE != 0 && self.source_pending(intcon)
    }

    /// An enabled source with its flag set, ignoring GIE (the `SLEEP` wake
    /// condition). INTCON T0IF/INTF/RBIF (IOCIF on the enhanced core) pair
    /// with T0IE/INTE/RBIE; the PIRx peripheral flags also need PEIE.
    fn source_pending(&self, intcon: u8) -> bool {
        let core = intcon & 0x07 & (intcon >> 3);
        let peripheral = intcon & INTCON_PEIE != 0
            && self
                .map
                .pir
                .iter()
                .zip(self.map.pie)
                .any(|(&pir, &pie)| self.read_data(pir) & self.read_data(pie) != 0);
        core != 0 || peripheral
    }

    /// Advance TMR0 by `cycles` instruction cycles in timer mode (T0CS = 0,
    /// Fosc/4), through the prescaler when PSA = 0 (1:2 .. 1:256). An
    /// overflow from 0xFF sets T0IF/TMR0IF. Counter mode (T0CKI) never
    /// counts: nothing drives the pin. The two-cycle increment inhibit after
    /// a TMR0 write is not modelled.
    fn tick_tmr0(&mut self, cycles: u32) {
        let option = self.data[usize::from(self.map.option_reg)];
        if option & OPTION_T0CS != 0 {
            return;
        }
        let tmr0 = usize::from(self.map.tmr0);
        for _ in 0..cycles {
            if option & OPTION_PSA == 0 {
                self.tmr0_prescale += 1;
                if self.tmr0_prescale < 2 << (option & 0x07) {
                    continue;
                }
                self.tmr0_prescale = 0;
            }
            self.data[tmr0] = self.data[tmr0].wrapping_add(1);
            if self.data[tmr0] == 0 {
                self.data[usize::from(SFR_INTCON)] |= INTCON_T0IF;
            }
        }
    }

    /// The core registers the enhanced core shadows, in shadow order.
    const SHADOWED: [u16; 8] = [
        SFR_STATUS, SFR_WREG, SFR_BSR, SFR_PCLATH, SFR_FSR0L, SFR_FSR0H, SFR_FSR1L, SFR_FSR1H,
    ];

    /// Vector to 0x0004: push the return address and clear GIE. The classic
    /// core has no hardware context save; the enhanced core copies its core
    /// registers into the bank-31 shadows (DS40001440E section 7.5).
    fn take_interrupt(&mut self) {
        if !self.push(self.pc) {
            return;
        }
        if self.enhanced {
            for (i, reg) in Self::SHADOWED.iter().enumerate() {
                let v = match *reg {
                    SFR_WREG => self.wreg,
                    // STATUS_SHAD holds only C, DC and Z: TO and PD are
                    // not shadowed (DS40001440E section 7.5).
                    SFR_STATUS => self.data[usize::from(SFR_STATUS)] & STATUS_ALU,
                    _ => self.data[usize::from(*reg)],
                };
                self.data[usize::from(SFR_STATUS_SHAD) + i] = v;
            }
        }
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
                if word & 0x0800 == 0 && !self.push(self.pc) {
                    return Ok(2);
                }
                self.pc = target;
                Ok(2)
            }
            _ if self.enhanced => self.execute_literal_enhanced(word, f, d),
            _ => {
                match (word >> 8) & 0x0F {
                    0x0..=0x3 => self.wreg = k8,
                    0x4..=0x7 => {
                        self.wreg = k8;
                        self.ret();
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

    /// Sign-extend the low `bits` of `v`.
    fn signed(v: u16, bits: u32) -> i32 {
        let shift = 32 - bits;
        ((u32::from(v) << shift) as i32) >> shift
    }

    /// The enhanced core's `11 xxxx` group (DS40001440E Table 29-3): the
    /// literal operations, BRA, ADDFSR/MOVLP, the shifts, ADDWFC/SUBWFB and
    /// indexed MOVIW/MOVWI.
    fn execute_literal_enhanced(&mut self, word: u16, f: u8, d: bool) -> SimResult<u32> {
        let k8 = word as u8;
        let extra = self.pcl_extra(f, d);
        match (word >> 8) & 0x0F {
            0x0 => self.wreg = k8,
            0x1 => {
                if word & 0x80 == 0 {
                    // ADDFSR FSRn, k (signed 6-bit).
                    let n = (word >> 6) & 1;
                    let k = Self::signed(word & 0x3F, 6);
                    self.set_fsr(n, self.fsr(n).wrapping_add(k as u16));
                } else {
                    // MOVLP k
                    self.data[usize::from(SFR_PCLATH)] = (word & 0x7F) as u8;
                }
            }
            0x2 | 0x3 => {
                // BRA k (signed 9-bit, relative to PC + 1).
                let k = Self::signed(word & 0x1FF, 9);
                self.pc = (self.pc as i32).wrapping_add(k) as u32 & PIC16E_PC_MASK;
                return Ok(2);
            }
            0x4 => {
                self.wreg = k8;
                self.ret();
                return Ok(2);
            }
            0x5 => {
                // LSLF
                let v = self.read_file(f);
                let r = v << 1;
                self.store(f, d, r, Some((STATUS_C | STATUS_Z, (v >> 7) | Self::z(r))));
                return Ok(1 + extra);
            }
            0x6 => {
                // LSRF
                let v = self.read_file(f);
                let r = v >> 1;
                self.store(f, d, r, Some((STATUS_C | STATUS_Z, (v & 1) | Self::z(r))));
                return Ok(1 + extra);
            }
            0x7 => {
                // ASRF: the sign bit is kept.
                let v = self.read_file(f);
                let r = (v >> 1) | (v & 0x80);
                self.store(f, d, r, Some((STATUS_C | STATUS_Z, (v & 1) | Self::z(r))));
                return Ok(1 + extra);
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
            0xB => {
                // SUBWFB: f - W - !C.
                let borrow = u8::from(self.status() & STATUS_C == 0);
                let (r, set) = Self::sub_borrow_flags(self.read_file(f), self.wreg, borrow);
                self.store(f, d, r, Some((STATUS_ALU, set)));
                return Ok(1 + extra);
            }
            0xC => {
                // SUBLW: k - W.
                let (r, set) = Self::sub_flags(k8, self.wreg);
                self.wreg = r;
                self.set_flags(STATUS_ALU, set);
            }
            0xD => {
                // ADDWFC: f + W + C.
                let carry = self.status() & STATUS_C;
                let (r, set) = Self::add_carry_flags(self.read_file(f), self.wreg, carry);
                self.store(f, d, r, Some((STATUS_ALU, set)));
                return Ok(1 + extra);
            }
            0xE => {
                let (r, set) = Self::add_flags(self.wreg, k8);
                self.wreg = r;
                self.set_flags(STATUS_ALU, set);
            }
            _ => {
                // MOVIW k[FSRn] (bit 7 clear) / MOVWI k[FSRn] (bit 7 set).
                let n = (word >> 6) & 1;
                let k = Self::signed(word & 0x3F, 6);
                let target = self.fsr_target(self.fsr(n).wrapping_add(k as u16));
                self.move_indirect(word & 0x80 != 0, target);
            }
        }
        Ok(1)
    }

    /// MOVIW (`to_memory = false`: W = target, Z affected) or MOVWI.
    fn move_indirect(&mut self, to_memory: bool, target: Target) {
        if to_memory {
            self.write_target(target, self.wreg);
        } else {
            self.wreg = self.read_target(target);
            let z = Self::z(self.wreg);
            self.set_flags(STATUS_Z, z);
        }
    }

    /// A write to PCL is a jump and costs a second cycle.
    fn pcl_extra(&self, f: u8, d: bool) -> u32 {
        u32::from(d && self.resolve(f) == Target::Data(SFR_PCL))
    }

    fn set_flags(&mut self, mask: u8, set: u8) {
        let status = &mut self.data[usize::from(SFR_STATUS)];
        *status = (*status & !mask) | (set & mask);
    }

    fn skip_if(&mut self, skip: bool) -> u32 {
        if skip {
            self.pc = (self.pc + 1) & self.pc_mask();
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
            return if self.enhanced {
                self.execute_control_enhanced(word, pc)
            } else {
                self.execute_control(word, pc)
            };
        }
        // The enhanced core assigns only 00 0001 0000 0011 to CLRW.
        if self.enhanced && op == 1 && !d && word != 0x0103 {
            return Err(SimulationError::DecodeError(u64::from(pc)));
        }
        let extra = self.pcl_extra(f, d);
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
                self.ret();
                Ok(2)
            }
            // RETFIE
            0x0009 => {
                self.ret();
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

    /// The enhanced core's `00 0000 0xxx xxxx` group (DS40001440E Table
    /// 29-3).
    fn execute_control_enhanced(&mut self, word: u16, pc: u32) -> SimResult<u32> {
        match word {
            0x0000 => Ok(1),
            // RESET: a software reset; PCON.RI reads 0 afterwards.
            0x0001 => {
                self.pending_reset = Some((0, PCON_RI));
                Ok(1)
            }
            0x0008 => {
                self.ret();
                Ok(2)
            }
            // RETFIE: return, restore the shadowed core registers, GIE = 1.
            0x0009 => {
                if self.pop().map(|pc| self.pc = pc).is_none() {
                    return Ok(2);
                }
                for (i, reg) in Self::SHADOWED.iter().enumerate() {
                    let v = self.data[usize::from(SFR_STATUS_SHAD) + i];
                    match *reg {
                        SFR_WREG => self.wreg = v,
                        // Keep the live TO/PD: RETFIE must not undo a
                        // CLRWDT/SLEEP executed inside the ISR.
                        SFR_STATUS => {
                            let st = &mut self.data[usize::from(SFR_STATUS)];
                            *st = (*st & !STATUS_ALU) | (v & STATUS_ALU);
                        }
                        _ => self.data[usize::from(*reg)] = v,
                    }
                }
                self.data[usize::from(SFR_INTCON)] |= INTCON_GIE;
                Ok(2)
            }
            // CALLW: call PCLATH<6:0>:W.
            0x000A => {
                let target =
                    (u32::from(self.data[usize::from(SFR_PCLATH)]) << 8) | u32::from(self.wreg);
                if self.push(self.pc) {
                    self.pc = target & PIC16E_PC_MASK;
                }
                Ok(2)
            }
            // BRW: branch to PC + 1 + W.
            0x000B => {
                self.pc = (self.pc + u32::from(self.wreg)) & PIC16E_PC_MASK;
                Ok(2)
            }
            // MOVIW (0x0010-0x0017) / MOVWI (0x0018-0x001F) with
            // ++FSRn, --FSRn, FSRn++, FSRn--.
            0x0010..=0x001F => {
                let n = (word >> 2) & 1;
                let mode = word & 0x03;
                let step: u16 = if mode & 1 == 0 { 1 } else { 0xFFFF };
                if mode < 2 {
                    self.set_fsr(n, self.fsr(n).wrapping_add(step));
                }
                let target = self.fsr_target(self.fsr(n));
                self.move_indirect(word & 0x08 != 0, target);
                if mode >= 2 {
                    self.set_fsr(n, self.fsr(n).wrapping_add(step));
                }
                Ok(1)
            }
            // MOVLB k (5-bit BSR).
            0x0020..=0x003F => {
                self.data[usize::from(SFR_BSR)] = (word & 0x1F) as u8;
                Ok(1)
            }
            0x0062 => {
                let option = self.map.option_reg;
                self.data[usize::from(option)] = self.wreg;
                Ok(1)
            }
            0x0063 => {
                self.set_flags(STATUS_TO | STATUS_PD, STATUS_TO);
                self.asleep = true;
                Ok(1)
            }
            0x0064 => {
                self.set_flags(STATUS_TO | STATUS_PD, STATUS_TO | STATUS_PD);
                Ok(1)
            }
            // TRIS PORTA/PORTB/PORTC.
            0x0065..=0x0067 => {
                let tris = self.map.trisa + (word - 0x0065);
                self.data[usize::from(tris)] = self.wreg;
                Ok(1)
            }
            _ => Err(SimulationError::DecodeError(u64::from(pc))),
        }
    }

    /// Apply a reset requested by the instruction that just ran.
    fn apply_pending_reset(&mut self) {
        if let Some((set, clear)) = self.pending_reset.take() {
            self.reset_core();
            if let Some(pcon) = self.map.pcon {
                let v = &mut self.data[usize::from(pcon)];
                *v = (*v | set) & !clear;
            }
        }
    }

    fn register_view(&self) -> [u32; 6] {
        let fsr = if self.enhanced {
            u32::from(self.fsr(0))
        } else {
            u32::from(self.data[usize::from(SFR_FSR)])
        };
        [
            u32::from(self.wreg),
            u32::from(self.status()),
            fsr,
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

        self.pc = (pc + 1) & self.pc_mask();
        let cycles = self.execute(word, pc)?;
        self.cycles += u64::from(cycles);
        crate::advance_batch_cycle(bus, u64::from(cycles));
        self.apply_pending_reset();
        self.tick_tmr0(cycles);

        // Interrupts are sampled between instructions.
        if self.interrupt_request() {
            self.take_interrupt();
            self.apply_pending_reset();
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
        self.pc = val & self.pc_mask();
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, val: u32) {
        self.stkptr = if self.enhanced {
            (val & 0x1F) as u8
        } else {
            (val % PIC16_STACK_DEPTH as u32) as u8
        };
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
            2 if self.enhanced => self.set_fsr(0, val as u16),
            2 => self.data[usize::from(SFR_FSR)] = val as u8,
            3 => self.data[usize::from(SFR_PCLATH)] = val as u8 & self.pclath_mask(),
            4 => self.set_sp(val),
            5 => self.set_pc(val),
            _ => {}
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let fsr = if self.enhanced { "FSR0" } else { "FSR" };
        ["W", "STATUS", fsr, "PCLATH", "SP", "PC"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        match name.trim().to_ascii_uppercase().as_str() {
            "W" | "WREG" => Some(0),
            "STATUS" => Some(1),
            "FSR" | "FSR0" => Some(2),
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
            stack: self.stack[..self.stack_depth()].to_vec(),
            asleep: self.asleep,
            bsr: if self.enhanced {
                self.data[usize::from(SFR_BSR)]
            } else {
                0
            },
            fsr0h: if self.enhanced {
                self.data[usize::from(SFR_FSR0H)]
            } else {
                0
            },
            fsr1: if self.enhanced { self.fsr(1) } else { 0 },
        }))
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Pic16(s) = snapshot {
            self.wreg = s.wreg;
            self.data[usize::from(SFR_STATUS)] = s.status;
            self.data[usize::from(SFR_FSR)] = s.fsr;
            self.data[usize::from(SFR_PCLATH)] = s.pclath & self.pclath_mask();
            self.pc = s.pc & self.pc_mask();
            self.cycles = s.cycles;
            let mask = self.pc_mask();
            let depth = self.stack_depth();
            for (slot, value) in self.stack[..depth].iter_mut().zip(s.stack.iter()) {
                *slot = *value & mask;
            }
            self.set_sp(u32::from(s.stkptr));
            self.asleep = s.asleep;
            if self.enhanced {
                self.data[usize::from(SFR_BSR)] = s.bsr & (PIC16E_BANKS as u8 - 1);
                self.data[usize::from(SFR_FSR0H)] = s.fsr0h;
                self.set_fsr(1, s.fsr1);
            }
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
        assert_eq!(cpu.pop(), Some(0x108));
        for i in (1..8).rev() {
            assert_eq!(cpu.pop(), Some(0x100 + i));
        }
        assert_eq!(cpu.pop(), Some(0x108));
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
            enhanced: false,
            flash_words: 16,
            port_masks: [0xFF; 5],
            map: PIC16_CLASSIC_MAP,
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
        assert!(pic16_device("pic16f1829").is_some_and(|d| d.enhanced));
        assert!(check_pic16_entry(0).is_ok());
        assert!(check_pic16_entry(1).is_err());
    }

    #[test]
    fn enhanced_rows_pick_the_core_flash_and_oscstat_map() {
        // Left out: no EUSART, PIR1 at 0x010, 64 banks.
        for name in ["pic16f1503", "pic16f1614", "pic16f18855"] {
            assert!(pic16_device(name).is_none(), "{name}");
        }
        let names: Vec<_> = PIC16_DEVICES.iter().map(|d| d.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate rows");

        let cpu = Pic16::for_chip_name("pic16lf1823");
        assert!(cpu.is_enhanced());
        assert_eq!(cpu.flash_words(), 2048);
        assert_eq!(cpu.read_data(0x09A) & 0x1F, 0x1F, "F182x OSCSTAT ready");

        // PIC16F1519: 16K words, 40-pin ports, no MFINTOSC.
        let cpu = Pic16::for_chip_name("pic16f1519");
        assert!(cpu.is_enhanced());
        assert_eq!(cpu.flash_words(), 16384);
        assert_eq!(cpu.read_data(0x09A) & 0x1F, 0x13, "no MFIOFR/HFIOFL");
        let d = pic16_device("pic16f1519").unwrap();
        assert_eq!(d.port_masks, [0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    }

    // ---- Enhanced mid-range (PIC16F1829; DS40001440E Table 29-3) ----

    fn ecpu_with(words: &[u16]) -> Pic16 {
        let mut cpu = Pic16::for_chip_name("pic16f1829");
        cpu.load_words(words);
        cpu
    }
    fn movlb(k: u8) -> u16 {
        0x0020 | u16::from(k & 0x1F)
    }
    fn movlp(k: u8) -> u16 {
        0x3180 | u16::from(k & 0x7F)
    }
    fn addfsr(n: u16, k: i8) -> u16 {
        0x3100 | (n << 6) | (k as u16 & 0x3F)
    }
    fn bra(k: i16) -> u16 {
        0x3200 | (k as u16 & 0x1FF)
    }
    /// MOVIW (`to_memory = false`) / MOVWI k[FSRn].
    fn indexed(to_memory: bool, n: u16, k: i8) -> u16 {
        0x3F00 | (u16::from(to_memory) << 7) | (n << 6) | (k as u16 & 0x3F)
    }
    /// Set FSRn to `v` with MOVLW/MOVWF.
    fn load_fsr(n: u8, v: u16) -> [u16; 4] {
        [
            movlw(v as u8),
            movwf(0x04 + 2 * n),
            movlw((v >> 8) as u8),
            movwf(0x05 + 2 * n),
        ]
    }

    #[test]
    fn enhanced_reset_state_matches_ds40001440_por_values() {
        let cpu = Pic16::for_chip_name("pic16f1829");
        assert!(cpu.is_enhanced());
        assert_eq!(cpu.flash_words(), 8192);
        assert_eq!(cpu.read_data(SFR_STATUS), STATUS_TO | STATUS_PD);
        assert_eq!(cpu.read_data(SFR_STKPTR), STKPTR_EMPTY);
        assert_eq!(cpu.read_data(SFR_BSR), 0);
        assert_eq!(cpu.read_data(0x08C), 0xFF, "TRISA");
        assert_eq!(cpu.read_data(0x095), 0xFF, "OPTION_REG");
        assert_eq!(cpu.read_data(0x096), PCON_POR, "PCON");
        assert_eq!(cpu.read_data(0x19E), TXSTA_TRMT, "TXSTA");
        assert_eq!(cpu.read_data(0x09A) & 0x1F, 0x1F, "OSCSTAT ready bits");
        // The classic core is unchanged.
        assert!(!Pic16::new().is_enhanced());
    }

    #[test]
    fn enhanced_core_registers_and_common_ram_appear_in_every_bank() {
        let mut cpu = ecpu_with(&[
            movlb(7),
            movlw(0x33),
            movwf(0x0A), // PCLATH, from bank 7
            movwf(0x70), // common RAM, from bank 7
            movwf(0x20), // bank 7 GPR = 0x3A0
            movlw(0x44),
            movwf(0x09), // WREG through its file address
        ]);
        run(&mut cpu, 7);
        assert_eq!(cpu.read_data(SFR_PCLATH), 0x33);
        assert_eq!(cpu.read_data(0x70), 0x33);
        assert_eq!(cpu.read_data(0x7F0), 0x33, "bank 15 sees the same byte");
        assert_eq!(cpu.read_data(0x3A0), 0x33);
        assert_eq!(cpu.read_data(0x020), 0, "bank 0 GPR untouched");
        assert_eq!(cpu.wreg(), 0x44);
        assert_eq!(cpu.read_data(0x389), 0x44, "WREG mirrored in bank 7");
    }

    #[test]
    fn enhanced_status_has_no_bank_bits_and_keeps_to_pd_read_only() {
        let mut cpu = ecpu_with(&[movlw(0xFF), movwf(0x03)]);
        run(&mut cpu, 2);
        assert_eq!(
            cpu.read_data(SFR_STATUS),
            STATUS_TO | STATUS_PD | STATUS_ALU
        );
        assert_eq!(
            cpu.file_address(0x20),
            0x020,
            "STATUS does not select a bank"
        );
    }

    #[test]
    fn enhanced_fsr_reaches_banked_linear_and_flash_spaces() {
        let mut words = Vec::new();
        words.extend(load_fsr(0, 0x2050)); // linear byte 80 = bank 1 offset 0x20
        words.push(movlw(0x5A));
        words.push(movwf(0x00)); // INDF0
        words.extend(load_fsr(1, 0x0120)); // banked address, bank 2
        words.push(movlw(0x5A));
        words.push(movwf(0x01)); // INDF1
        words.extend(load_fsr(0, 0x8000 + 20)); // flash word 20
        words.push(byte_op(0x8, 0x00, false)); // MOVF INDF0, W
        let steps = words.len();
        while words.len() < 20 {
            words.push(0x0000);
        }
        words.push(0x34A5); // word 20: RETLW 0xA5, low byte 0xA5
        let mut cpu = ecpu_with(&words);
        run(&mut cpu, steps);
        assert_eq!(cpu.read_data(0x0A0), 0x5A, "linear 0x2050 -> bank 1 0x20");
        assert_eq!(cpu.read_data(0x120), 0x5A);
        assert_eq!(cpu.wreg(), 0xA5, "flash read returns the low byte");
    }

    #[test]
    fn enhanced_flash_through_fsr_is_read_only_and_indf_via_fsr_reads_zero() {
        let mut words = Vec::new();
        words.extend(load_fsr(0, 0x8000));
        words.push(movlw(0x12));
        words.push(movwf(0x00));
        words.extend(load_fsr(1, 0x0000)); // FSR1 -> INDF0
        words.push(byte_op(0x8, 0x01, false));
        let mut cpu = ecpu_with(&words);
        run(&mut cpu, words.len());
        assert_eq!(cpu.fetch_word(0), movlw(0x00), "flash unchanged");
        assert_eq!(cpu.wreg(), 0);
        assert_ne!(cpu.status() & STATUS_Z, 0);
    }

    #[test]
    fn moviw_movwi_pre_and_post_increment_and_decrement() {
        let mut words = Vec::new();
        words.extend(load_fsr(0, 0x2000));
        words.push(movlw(1));
        words.push(0x001A); // MOVWI FSR0++  -> [0x2000] = 1, FSR0 = 0x2001
        words.push(movlw(2));
        words.push(0x0018); // MOVWI ++FSR0  -> FSR0 = 0x2002, [0x2002] = 2
        words.push(0x0011); // MOVIW --FSR0  -> FSR0 = 0x2001, W = 0, Z
        words.push(0x0013); // MOVIW FSR0--  -> W = [0x2001] = 0, FSR0 = 0x2000
        words.push(0x0012); // MOVIW FSR0++  -> W = 1, FSR0 = 0x2001
        let mut cpu = ecpu_with(&words);
        run(&mut cpu, words.len());
        assert_eq!(cpu.read_data(0x020), 1);
        assert_eq!(cpu.read_data(0x022), 2);
        assert_eq!(cpu.wreg(), 1);
        assert_eq!(cpu.fsr(0), 0x2001);
        assert_eq!(cpu.status() & STATUS_Z, 0);
    }

    #[test]
    fn indexed_moviw_movwi_and_addfsr_use_signed_offsets() {
        let mut words = Vec::new();
        words.extend(load_fsr(1, 0x2010));
        words.push(movlw(0x77));
        words.push(indexed(true, 1, -3)); // [0x200D] = 0x77
        words.push(addfsr(1, -16)); // FSR1 = 0x2000
        words.push(movlw(0));
        words.push(indexed(false, 1, 13)); // W = [0x200D]
        let mut cpu = ecpu_with(&words);
        run(&mut cpu, words.len());
        assert_eq!(cpu.read_data(0x02D), 0x77);
        assert_eq!(cpu.wreg(), 0x77);
        assert_eq!(cpu.fsr(1), 0x2000, "indexed forms leave FSR alone");
    }

    #[test]
    fn bra_brw_callw_and_movlp_paging() {
        // 0: BRA +2 -> 3; 3: MOVLW 2; 4: BRW -> 4+1+2 = 7; 7: MOVLP 0x10;
        // 8: MOVLW 0x05; 9: CALLW -> 0x1005; 0x1005: BRA -1 -> 0x1005.
        let mut words = vec![0u16; 0x1006];
        words[0] = bra(2);
        words[3] = movlw(2);
        words[4] = 0x000B;
        words[7] = movlp(0x10);
        words[8] = movlw(0x05);
        words[9] = 0x000A;
        words[0x1005] = bra(-1);
        let mut cpu = ecpu_with(&words);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 3);
        run(&mut cpu, 2);
        assert_eq!(cpu.get_pc(), 7);
        run(&mut cpu, 3);
        assert_eq!(cpu.get_pc(), 0x1005);
        assert_eq!(cpu.read_data(SFR_STKPTR), 0);
        assert_eq!(cpu.read_data(SFR_TOSL), 10, "return address = CALLW + 1");
        assert_eq!(cpu.read_data(SFR_TOSH), 0);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x1005, "BRA -1 loops on itself");
    }

    #[test]
    fn goto_takes_its_page_from_pclath_6_3() {
        let mut cpu = ecpu_with(&[movlp(0x38), 0x2805]); // GOTO 0x005, page 0x38
        run(&mut cpu, 2);
        assert_eq!(cpu.get_pc(), 0x3805);
    }

    #[test]
    fn shifts_and_carry_arithmetic_set_flags() {
        let mut cpu = ecpu_with(&[
            movlw(0x81),
            movwf(0x20),
            0x3500 | 0x20,        // LSLF 0x20, W: W = 0x02, C = 1
            0x3700 | 0x80 | 0x20, // ASRF 0x20, F: 0x81 -> 0xC0, C = 1
            0x3600 | 0x20,        // LSRF 0x20, W: 0xC0 -> 0x60, C = 0
        ]);
        run(&mut cpu, 3);
        assert_eq!(cpu.wreg(), 0x02);
        assert_ne!(cpu.status() & STATUS_C, 0);
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x20), 0xC0);
        assert_ne!(cpu.status() & STATUS_C, 0);
        run(&mut cpu, 1);
        assert_eq!(cpu.wreg(), 0x60);
        assert_eq!(cpu.status() & STATUS_C, 0);

        // 16-bit add 0x01FF + 0x0001 with ADDWF/ADDWFC, then 16-bit subtract
        // back with SUBWF/SUBWFB.
        let mut cpu = ecpu_with(&[
            movlw(0xFF),
            movwf(0x20),
            movlw(0x01),
            movwf(0x21),
            movlw(0x01),
            byte_op(0x7, 0x20, true), // ADDWF lo: 0x00, C = 1
            movlw(0x00),
            0x3D80 | 0x21, // ADDWFC hi: 0x02
            movlw(0x01),
            byte_op(0x2, 0x20, true), // SUBWF lo: 0x00 - 1 = 0xFF, C = 0 (borrow)
            movlw(0x00),
            0x3B80 | 0x21, // SUBWFB hi: 0x02 - 0 - 1 = 0x01, C = 1
        ]);
        run(&mut cpu, 8);
        assert_eq!(cpu.read_data(0x20), 0x00);
        assert_eq!(cpu.read_data(0x21), 0x02);
        run(&mut cpu, 4);
        assert_eq!(cpu.read_data(0x20), 0xFF);
        assert_eq!(cpu.read_data(0x21), 0x01);
        assert_ne!(cpu.status() & STATUS_C, 0);
        assert_eq!(cpu.status() & STATUS_Z, 0);
    }

    #[test]
    fn interrupt_entry_shadows_core_registers_and_retfie_restores_them() {
        // Main: BSR = 3, PCLATH = 0x02, FSR0 = 0x1234, W = 0x5A, C set,
        // then enable INTE + GIE with INTF set. The ISR at 4 trashes W,
        // BSR and PCLATH, clears INTF and returns.
        let mut words = vec![0u16; 0x20];
        words[0] = 0x2810; // GOTO 0x10
        words[4] = movlw(0xEE);
        words[5] = movlb(9);
        words[6] = movlp(0x05);
        words[7] = bcf(0x0B, 1); // INTF
        words[8] = 0x0009; // RETFIE
        let main = [
            movlb(3),
            movlp(0x02),
            movlw(0x34),
            movwf(0x04),
            movlw(0x12),
            movwf(0x05),
            bsf(0x03, 0), // C
            movlw(0x5A),
            movwf(0x7F),
            movlw(0x92), // GIE | INTE | INTF
            movwf(0x0B),
        ];
        words[0x10..0x10 + main.len()].copy_from_slice(&main);
        let mut cpu = ecpu_with(&words);
        run(&mut cpu, 1 + main.len());
        assert_eq!(cpu.get_pc(), 4, "vectored");
        assert_eq!(cpu.read_data(SFR_STATUS_SHAD + 2), 3, "BSR_SHAD");
        assert_eq!(cpu.read_data(SFR_STATUS_SHAD + 3), 0x02, "PCLATH_SHAD");
        // W was 0x92 at entry (the last MOVLW before MOVWF INTCON).
        assert_eq!(cpu.read_data(SFR_STATUS_SHAD + 1), 0x92, "WREG_SHAD");
        run(&mut cpu, 5);
        assert_eq!(cpu.get_pc(), 0x10 + main.len() as u32);
        assert_eq!(cpu.wreg(), 0x92);
        assert_eq!(cpu.read_data(SFR_BSR), 3);
        assert_eq!(cpu.read_data(SFR_PCLATH), 0x02);
        assert_eq!(cpu.fsr(0), 0x1234);
        assert_ne!(cpu.status() & STATUS_C, 0);
        assert_ne!(cpu.read_data(SFR_INTCON) & INTCON_GIE, 0);
    }

    #[test]
    fn retfie_keeps_live_to_pd() {
        // DS40001440E section 7.5: STATUS is shadowed except TO and PD. Main
        // sleeps (PD = 0), wakes on INTF with GIE = 0, then sets GIE and is
        // vectored; the ISR's CLRWDT sets PD = 1, which RETFIE must keep.
        let mut words = vec![0u16; 0x20];
        words[0] = 0x2810; // GOTO 0x10
        words[4] = 0x0064; // CLRWDT
        words[5] = bcf(0x0B, 1); // INTF
        words[6] = 0x0009; // RETFIE
        let main = [
            movlw(0x10),
            movwf(0x0B),
            0x0063,
            0x0000,
            bsf(0x0B, 7),
            0x0000,
        ];
        words[0x10..0x10 + main.len()].copy_from_slice(&main);
        let mut cpu = ecpu_with(&words);
        run(&mut cpu, 4);
        assert_eq!(cpu.status() & STATUS_PD, 0, "asleep, PD = 0");
        cpu.write_data(SFR_INTCON, 0x12); // INTE | INTF
        for _ in 0..8 {
            if cpu.get_pc() == 4 {
                break;
            }
            run(&mut cpu, 1);
        }
        assert_eq!(cpu.get_pc(), 4, "vectored");
        assert_eq!(cpu.status() & STATUS_PD, 0);
        assert_eq!(
            cpu.read_data(SFR_STATUS_SHAD) & (STATUS_TO | STATUS_PD),
            0,
            "TO/PD not shadowed"
        );
        run(&mut cpu, 3);
        assert_ne!(cpu.read_data(SFR_INTCON) & INTCON_GIE, 0, "returned");
        assert_ne!(cpu.status() & STATUS_PD, 0, "CLRWDT's PD survives RETFIE");
        assert_ne!(cpu.status() & STATUS_TO, 0);
    }

    #[test]
    fn stack_overflow_resets_with_stvren_and_wraps_without() {
        // CALL 0 forever: the 17th push overflows.
        let mut cpu = ecpu_with(&[0x2000]);
        run(&mut cpu, 16);
        assert_eq!(cpu.read_data(SFR_STKPTR), 15);
        cpu.write_data(0x096, 0);
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(SFR_STKPTR), STKPTR_EMPTY, "reset");
        assert_eq!(cpu.get_pc(), 0);
        assert_ne!(cpu.read_data(0x096) & PCON_STKOVF, 0);

        // CONFIG2 with STVREN = 0 (byte address of word 0x8008).
        let mut cpu = ecpu_with(&[0x2000]);
        cpu.load_flash(0x8008 * 2, &[0xFF, 0x3D]);
        assert_eq!(cpu.config_word2() & CONFIG2_STVREN, 0);
        run(&mut cpu, 17);
        assert_eq!(cpu.read_data(SFR_STKPTR), 0, "wrapped");
        assert_ne!(cpu.read_data(0x096) & PCON_STKOVF, 0);
    }

    #[test]
    fn stack_underflow_resets_with_stvren() {
        let mut cpu = ecpu_with(&[movlw(1), 0x0008]);
        run(&mut cpu, 2);
        assert_eq!(cpu.get_pc(), 0);
        assert_ne!(cpu.read_data(0x096) & PCON_STKUNF, 0);
        assert_eq!(cpu.wreg(), 0, "core reset");
    }

    #[test]
    fn reset_instruction_clears_ri_and_restarts() {
        let mut cpu = ecpu_with(&[movlw(9), 0x0001]);
        assert_ne!(cpu.read_data(0x096) & PCON_RI, 0);
        run(&mut cpu, 2);
        assert_eq!(cpu.get_pc(), 0);
        assert_eq!(cpu.wreg(), 0);
        assert_eq!(cpu.read_data(0x096) & PCON_RI, 0);
    }

    #[test]
    fn eusart_tx_and_lat_registers_on_the_f182x_map() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut cpu = ecpu_with(&[
            movlb(3),
            movlw(0x24),
            movwf(0x1E), // TXSTA (0x19E)
            movlw(0x80),
            movwf(0x1D), // RCSTA (0x19D)
            movlw(b'K'),
            movwf(0x1A), // TXREG (0x19A)
            movlb(1),
            bcf(0x0E, 0), // TRISC0 (0x08E)
            movlb(2),
            bsf(0x0E, 0), // LATC0 (0x10E)
        ]);
        cpu.set_serial_sink(sink.clone());
        run(&mut cpu, 11);
        assert_eq!(sink.lock().unwrap().as_slice(), b"K");
        assert_ne!(cpu.read_data(0x011) & PIR1_TXIF, 0, "TXIF follows TXEN");
        assert_eq!(cpu.read_data(0x10E), 0x01, "LATC");
        assert_eq!(cpu.read_data(0x00E), 0x01, "PORTC pin driven by LATC0");
        assert_eq!(cpu.port_latch(2), 0x01);
    }

    #[test]
    fn enhanced_image_config_and_eeprom_addresses() {
        let mut cpu = Pic16::for_chip_name("pic16f1829");
        cpu.load_flash(0x8007 * 2, &[0xC4, 0x3F, 0xFF, 0x1F]);
        cpu.load_flash(0x8000 * 2, &[0x01, 0x00]);
        cpu.load_flash(0xF000 * 2, &[0xAB, 0x00]);
        assert_eq!(cpu.config_word(), 0x3FC4);
        assert_eq!(cpu.config_word2(), 0x1FFF);
        assert_eq!(cpu.eedata(0), 0xAB);
    }

    #[test]
    fn enhanced_decode_differs_from_classic_where_the_isa_does() {
        // 0x3B00 is unassigned on the classic core and SUBWFB on the
        // enhanced one; 0x0100 is CLRW on the classic core only.
        let mut bus = crate::bus::SystemBus::new();
        let config = SimulationConfig::default();
        let mut classic = cpu_with(&[0x3B00]);
        assert!(classic.step(&mut bus, &[], &config).is_err());
        let mut enhanced = ecpu_with(&[0x3B00]);
        assert!(enhanced.step(&mut bus, &[], &config).is_ok());
        let mut enhanced = ecpu_with(&[0x0100]);
        assert!(enhanced.step(&mut bus, &[], &config).is_err());
        let mut enhanced = ecpu_with(&[0x0103]);
        assert!(enhanced.step(&mut bus, &[], &config).is_ok());
        for word in [0x0002u16, 0x0007, 0x000C, 0x0040, 0x0068] {
            let mut enhanced = ecpu_with(&[word]);
            assert!(
                enhanced.step(&mut bus, &[], &config).is_err(),
                "{word:#06x} must not decode"
            );
        }
    }

    #[test]
    fn enhanced_snapshot_round_trip_keeps_bsr_and_fsrs() {
        let mut words = vec![movlb(5)];
        words.extend(load_fsr(0, 0x2345));
        words.extend(load_fsr(1, 0x8ABC));
        words.push(0x2000 | 0x0F); // CALL 0x0F
        let mut cpu = ecpu_with(&words);
        run(&mut cpu, words.len());
        let snap = cpu.snapshot();
        let mut other = Pic16::for_chip_name("pic16f1829");
        other.apply_snapshot(&snap);
        assert_eq!(other.read_data(SFR_BSR), 5);
        assert_eq!(other.fsr(0), 0x2345);
        assert_eq!(other.fsr(1), 0x8ABC);
        assert_eq!(other.read_data(SFR_STKPTR), 0);
        assert_eq!(other.read_data(SFR_TOSL), words.len() as u8);
        assert_eq!(other.get_pc(), 0x0F);
    }

    #[test]
    fn tmr0_counts_instruction_cycles_through_the_prescaler() {
        // Classic: OPTION_REG resets to T0CS = 1, so TMR0 holds still.
        let mut cpu = cpu_with(&[0x0000; 8]);
        run(&mut cpu, 4);
        assert_eq!(cpu.read_data(SFR_TMR0), 0);

        // T0CS = 0, PSA = 1: one count per instruction cycle; 0xFE + 2
        // overflows and sets T0IF.
        let mut cpu = cpu_with(&[0x0000; 8]);
        cpu.write_data(SFR_OPTION_REG, OPTION_PSA);
        cpu.write_data(SFR_TMR0, 0xFE);
        run(&mut cpu, 2);
        assert_eq!(cpu.read_data(SFR_TMR0), 0x00);
        assert_ne!(cpu.read_data(SFR_INTCON) & INTCON_T0IF, 0);

        // Enhanced map (TMR0 at 0x015), PSA = 0, PS = 001: 1:4.
        let mut cpu = ecpu_with(&[0x0000; 16]);
        cpu.write_data(0x095, 0x01);
        run(&mut cpu, 7);
        assert_eq!(cpu.read_data(0x015), 1);
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x015), 2);
        // A TMR0 write clears the prescaler count.
        run(&mut cpu, 3);
        cpu.write_data(0x015, 0x10);
        run(&mut cpu, 3);
        assert_eq!(cpu.read_data(0x015), 0x10);
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x015), 0x11);
    }
}
