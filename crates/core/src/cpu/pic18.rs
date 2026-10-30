// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! PIC18 (classic 16-bit instruction-word, byte-addressed PC) M0 core.
//!
//! The PC is a **byte** address. Instructions are 16-bit words fetched on a
//! two-byte boundary, so single-word instructions advance the PC by 2 and the
//! standard two-word forms (`CALL`, `GOTO`, `MOVFF`, `LFSR`) advance it by 4.
//! Branch literals are relative in *word* units; `CALL`/`GOTO` literals are
//! absolute **word** addresses (`k -> PC<20:1>`), hence the final `<< 1`.
//!
//! Data memory is the classic 16-bank file:
//!
//! * `a = 1` uses `BSR<3:0>:f`;
//! * `a = 0` uses the Access Bank: `f` below the device's split is access
//!   RAM, `f` at or above it is the SFR window `0xF00 | f`. The default
//!   split is `0x60` (`0x000..0x05F` / `0xF60..0xFFF`); devices whose SFRs
//!   start at `0xF80`, such as the PIC18F4520, split at `0x80`
//!   ([`Pic18::for_chip_name`]).
//!
//! Access-bank `INDFn`, `POSTINCn`, `POSTDECn`, `PREINCn` and `PLUSWn` are
//! virtual registers. Byte and bit file operations route through the
//! per-instruction resolver, which uses `FSRn` as a full 12-bit data address
//! and applies the alias's FSR update once per instruction access.
//! `MOVFF` takes 12-bit absolute operands and expands the same aliases on
//! either side (see the progress doc).
//!
//! This milestone models the instruction subset listed in
//! `docs/PIC18-CORE-PROGRESS.md`. Anything else returns
//! [`SimulationError::DecodeError`]; it is never guessed.

use crate::snapshot::{CpuSnapshot, Pic18CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
use std::sync::{Arc, Mutex};

/// Modeled program-flash size. A per-device geometry can be supplied with
/// [`Pic18::with_geometry`]; this default is large enough for the hand
/// fixtures and does not claim any catalogue part.
pub const PIC18_FLASH_BYTES: usize = 64 * 1024;
/// Classic 16-bank data file (`0x000..0xFFF`).
pub const PIC18_DATA_BYTES: usize = 0x1000;
/// 12-bit data-space mask; classic PIC18 `FSRn` values are a full 12-bit
/// address, not a bank-relative offset.
pub const PIC18_DATA_MASK: u16 = (PIC18_DATA_BYTES - 1) as u16;
/// Default Access Bank split (`0x000..0x05F` RAM, `0xF60..0xFFF` SFRs).
pub const PIC18_ACCESS_SPLIT_DEFAULT: u8 = 0x60;

/// Classic PIC18 hardware return-stack depth.
pub const PIC18_STACK_DEPTH: usize = 31;
/// 21-bit byte PC; bit 0 is always clear.
pub const PIC18_PC_MASK: u32 = 0x1F_FFFE;

pub const SFR_WREG: u16 = 0xFE8;
pub const SFR_STATUS: u16 = 0xFD8;
pub const SFR_BSR: u16 = 0xFE0;
pub const SFR_FSR0L: u16 = 0xFE9;
pub const SFR_FSR0H: u16 = 0xFEA;
pub const SFR_FSR1L: u16 = 0xFE1;
pub const SFR_FSR1H: u16 = 0xFE2;
pub const SFR_FSR2L: u16 = 0xFD9;
pub const SFR_FSR2H: u16 = 0xFDA;
pub const SFR_PLUSW2: u16 = 0xFDB;
pub const SFR_PREINC2: u16 = 0xFDC;
pub const SFR_POSTDEC2: u16 = 0xFDD;
pub const SFR_POSTINC2: u16 = 0xFDE;
pub const SFR_INDF2: u16 = 0xFDF;
pub const SFR_PLUSW1: u16 = 0xFE3;
pub const SFR_PREINC1: u16 = 0xFE4;
pub const SFR_POSTDEC1: u16 = 0xFE5;
pub const SFR_POSTINC1: u16 = 0xFE6;
pub const SFR_INDF1: u16 = 0xFE7;
pub const SFR_PLUSW0: u16 = 0xFEB;
pub const SFR_PREINC0: u16 = 0xFEC;
pub const SFR_POSTDEC0: u16 = 0xFED;
pub const SFR_POSTINC0: u16 = 0xFEE;
pub const SFR_INDF0: u16 = 0xFEF;
pub const SFR_PRODL: u16 = 0xFF3;
pub const SFR_PRODH: u16 = 0xFF4;
pub const SFR_TABLAT: u16 = 0xFF5;
pub const SFR_TBLPTRL: u16 = 0xFF6;
pub const SFR_TBLPTRH: u16 = 0xFF7;
pub const SFR_TBLPTRU: u16 = 0xFF8;
pub const SFR_PCL: u16 = 0xFF9;
pub const SFR_PCLATH: u16 = 0xFFA;
pub const SFR_PCLATU: u16 = 0xFFB;
pub const SFR_STKPTR: u16 = 0xFFC;
pub const SFR_TOSL: u16 = 0xFFD;
pub const SFR_TOSH: u16 = 0xFFE;
pub const SFR_TOSU: u16 = 0xFFF;

// Interrupt / reset control SFRs, classic PIC18F4520 map (DS39631 Table 5-1).
// The gate logic is generic; a per-device map can override the vector/aliases
// later without changing the CPU core.
pub const SFR_INTCON: u16 = 0xFF2;
pub const SFR_INTCON2: u16 = 0xFF1;
pub const SFR_INTCON3: u16 = 0xFF0;
pub const SFR_PIR1: u16 = 0xF9E;
pub const SFR_PIR2: u16 = 0xFA1;
pub const SFR_PIE1: u16 = 0xF9D;
pub const SFR_PIE2: u16 = 0xFA0;
pub const SFR_IPR1: u16 = 0xF9F;
pub const SFR_IPR2: u16 = 0xFA2;
pub const SFR_RCON: u16 = 0xFD0;

/// PIC18 high-priority interrupt vector (byte address).
pub const PIC18_HIGH_VECTOR: u32 = 0x0008;
/// PIC18 low-priority interrupt vector (byte address).
pub const PIC18_LOW_VECTOR: u32 = 0x0018;

// INTCON bits.
const INTCON_GIE_GIEH: u8 = 1 << 7;
const INTCON_GIEL: u8 = 1 << 6;
const INTCON_TMR0IE: u8 = 1 << 5;
const INTCON_INT0IE: u8 = 1 << 4;
const INTCON_RBIE: u8 = 1 << 3;
const INTCON_TMR0IF: u8 = 1 << 2;
const INTCON_INT0IF: u8 = 1 << 1;
const INTCON_RBIF: u8 = 1 << 0;
// INTCON2 priority bits.
const INTCON2_TMR0IP: u8 = 1 << 2;
const INTCON2_RBIP: u8 = 1 << 0;
// INTCON3 bits (PIC18F4520 DS39631 Register 9-3; bits 5 and 2 unimplemented).
const INTCON3_INT2IP: u8 = 1 << 7;
const INTCON3_INT1IP: u8 = 1 << 6;
const INTCON3_INT2IE: u8 = 1 << 4;
const INTCON3_INT1IE: u8 = 1 << 3;
const INTCON3_INT2IF: u8 = 1 << 1;
const INTCON3_INT1IF: u8 = 1 << 0;
// RCON.IPEN and the TO/PD status bits that SLEEP/CLRWDT update.
const RCON_IPEN: u8 = 1 << 7;
const RCON_TO: u8 = 1 << 3;
const RCON_PD: u8 = 1 << 2;

// EUSART and I/O port SFRs, PIC18F4520 map (DS39631 Table 5-1).
pub const SFR_TXREG: u16 = 0xFAD;
pub const SFR_TXSTA: u16 = 0xFAC;
pub const SFR_RCSTA: u16 = 0xFAB;
pub const SFR_SPBRG: u16 = 0xFAF;
pub const SFR_PORTA: u16 = 0xF80;
pub const SFR_LATA: u16 = 0xF89;
pub const SFR_TRISA: u16 = 0xF92;
/// Ports A..E; PORTx, LATx and TRISx are each five consecutive SFRs.
pub const PIC18_PORT_COUNT: u16 = 5;
const TXSTA_TXEN: u8 = 1 << 5;
const TXSTA_TRMT: u8 = 1 << 1;
const RCSTA_SPEN: u8 = 1 << 7;
const PIR1_TXIF: u8 = 1 << 4;

/// Table pointer width (TBLPTRU<5:0>:TBLPTRH:TBLPTRL).
pub const PIC18_TBLPTR_MASK: u32 = 0x3F_FFFF;
/// User ID locations (`IDLOC0..7`).
pub const PIC18_IDLOC_BASE: u32 = 0x20_0000;
/// Configuration words (`CONFIG1L..CONFIG7H`).
pub const PIC18_CONFIG_BASE: u32 = 0x30_0000;

pub const STATUS_C: u8 = 1 << 0;
pub const STATUS_DC: u8 = 1 << 1;
pub const STATUS_Z: u8 = 1 << 2;
pub const STATUS_OV: u8 = 1 << 3;
pub const STATUS_N: u8 = 1 << 4;
pub const STATUS_FLAG_MASK: u8 = STATUS_C | STATUS_DC | STATUS_Z | STATUS_OV | STATUS_N;

const STKPTR_STKFUL: u8 = 0x80;
const STKPTR_STKUNF: u8 = 0x40;

/// A PIC18 reset entry point must be a byte address on an instruction
/// boundary. The loader calls this before handing an image to a machine.
pub fn check_pic18_entry(entry: u64) -> Result<(), String> {
    if entry & 1 != 0 {
        return Err(format!(
            "PIC18 entry point {entry:#x} is not 2-byte aligned; PIC18 program memory is \
             byte-addressed with 16-bit instruction words"
        ));
    }
    if entry > u64::from(PIC18_PC_MASK) {
        return Err(format!(
            "PIC18 entry point {entry:#x} does not fit the 21-bit byte PC"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndirectMode {
    Indf,
    PostInc,
    PostDec,
    PreInc,
    PlusW,
}

/// One resolved file access. `post` is the FSR update to apply after the
/// instruction finishes reading/writing `addr`; RMW instructions resolve once
/// so `POSTINC`/`POSTDEC` move the pointer exactly one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileAccess {
    addr: u16,
    post: Option<(usize, i32)>,
}

#[derive(Debug, Clone)]
pub struct Pic18 {
    wreg: u8,
    status: u8,
    bsr: u8,
    pc: u32,
    cycles: u64,
    flash: Vec<u8>,
    data: Vec<u8>,
    fsr: [u16; 3],
    prod: u16,
    tblptr: u32,
    tablat: u8,
    pclath: u8,
    pclatu: u8,
    stack: Vec<u32>,
    stkptr: u8,
    stk_full: bool,
    stk_unf: bool,
    shadow: Option<(u8, u8, u8)>,
    exception_pending: Option<u32>,
    /// First Access Bank offset that maps to the SFR window.
    access_split: u8,
    /// Set by `SLEEP`; cleared by an enabled interrupt source.
    asleep: bool,
    idloc: [u8; 8],
    config: [u8; 16],
    /// EUSART TX bytes, when a console is attached.
    serial_sink: Option<Arc<Mutex<Vec<u8>>>>,
}

impl Default for Pic18 {
    fn default() -> Self {
        Self::new()
    }
}

impl Pic18 {
    pub fn new() -> Self {
        Self::with_geometry(PIC18_FLASH_BYTES, PIC18_STACK_DEPTH)
    }

    pub fn with_geometry(flash_bytes: usize, stack_depth: usize) -> Self {
        assert!(flash_bytes >= 2, "PIC18 flash must hold at least one word");
        assert!(stack_depth > 0, "PIC18 return stack must not be empty");
        let mut cpu = Self {
            wreg: 0,
            status: 0,
            bsr: 0,
            pc: 0,
            cycles: 0,
            flash: vec![0xFF; flash_bytes],
            data: vec![0; PIC18_DATA_BYTES],
            fsr: [0; 3],
            prod: 0,
            tblptr: 0,
            tablat: 0,
            pclath: 0,
            pclatu: 0,
            stack: vec![0; stack_depth],
            stkptr: 0,
            stk_full: false,
            stk_unf: false,
            shadow: None,
            exception_pending: None,
            access_split: PIC18_ACCESS_SPLIT_DEFAULT,
            asleep: false,
            idloc: [0xFF; 8],
            config: [0xFF; 16],
            serial_sink: None,
        };
        cpu.reset_core();
        cpu
    }

    /// Per-device geometry for the catalogue parts this model knows. Unknown
    /// names get the generic default core ([`Pic18::new`]).
    ///
    /// PIC18F4520 (DS39631E): 32 KB flash, 31-level stack, SFRs from
    /// `0xF80`, so the Access Bank splits at `0x80`.
    pub fn for_chip_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "pic18f4520" => {
                let mut cpu = Self::with_geometry(32 * 1024, PIC18_STACK_DEPTH);
                cpu.access_split = 0x80;
                cpu
            }
            _ => Self::new(),
        }
    }

    /// Route EUSART TX bytes to `sink` (the console / `uart_contains` buffer).
    pub fn set_serial_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>) {
        self.serial_sink = Some(sink);
    }

    /// Whether the core is idling in `SLEEP`.
    pub fn is_asleep(&self) -> bool {
        self.asleep
    }

    /// Power-on/`RESET`-instruction state of the core registers and the SFRs
    /// this model implements. Program memory, ID and configuration bytes are
    /// non-volatile and survive.
    fn reset_core(&mut self) {
        self.wreg = 0;
        self.status = 0;
        self.bsr = 0;
        self.pc = 0;
        self.fsr = [0; 3];
        self.prod = 0;
        self.tblptr = 0;
        self.tablat = 0;
        self.pclath = 0;
        self.pclatu = 0;
        self.stack.fill(0);
        self.stkptr = 0;
        self.stk_full = false;
        self.stk_unf = false;
        self.shadow = None;
        self.exception_pending = None;
        self.asleep = false;
        self.data.fill(0);
        // DS39631 power-on reset values: all port pins are inputs
        // (TRISE<3> is unimplemented), the EUSART shift register is empty,
        // and RCON reads TO = PD = 1 with the POR/BOR bits clear. Every
        // interrupt priority bit resets high (INTCON2 `1111 -1-1`, INTCON3
        // `11-0 0-00`, IPR1 `1111 1111`, IPR2 `11-1 1111`).
        for n in 0..PIC18_PORT_COUNT {
            self.data[usize::from(SFR_TRISA + n)] = 0xFF;
        }
        self.data[usize::from(SFR_TRISA + 4)] = 0x07;
        self.data[usize::from(SFR_TXSTA)] = TXSTA_TRMT;
        self.data[usize::from(SFR_RCON)] = 0x1C;
        self.data[usize::from(SFR_INTCON2)] = 0xF5;
        self.data[usize::from(SFR_INTCON3)] = INTCON3_INT2IP | INTCON3_INT1IP;
        self.data[usize::from(SFR_IPR1)] = 0xFF;
        self.data[usize::from(SFR_IPR2)] = 0xDF;
    }

    /// Load 16-bit instruction words starting at the reset vector.
    pub fn load_words(&mut self, words: &[u16]) {
        for (i, word) in words.iter().enumerate() {
            let off = i * 2;
            assert!(
                off + 2 <= self.flash.len(),
                "fixture exceeds PIC18 model flash"
            );
            self.flash[off..off + 2].copy_from_slice(&word.to_le_bytes());
        }
    }

    /// Load raw bytes at a byte-addressed program address. Bytes for the
    /// user ID and configuration windows land in those windows; anything
    /// else past the modelled flash is unimplemented memory and is dropped.
    pub fn load_flash(&mut self, addr: u32, bytes: &[u8]) {
        if addr >= PIC18_IDLOC_BASE {
            for (i, byte) in bytes.iter().enumerate() {
                let at = addr + i as u32;
                match at {
                    PIC18_IDLOC_BASE..=0x20_0007 => {
                        self.idloc[(at - PIC18_IDLOC_BASE) as usize] = *byte
                    }
                    PIC18_CONFIG_BASE..=0x30_000F => {
                        self.config[(at - PIC18_CONFIG_BASE) as usize] = *byte
                    }
                    _ => {}
                }
            }
            return;
        }
        let start = addr as usize & !1;
        let end = start.saturating_add(bytes.len()).min(self.flash.len());
        assert!(
            start <= self.flash.len(),
            "fixture exceeds PIC18 model flash"
        );
        self.flash[start..end].copy_from_slice(&bytes[..end - start]);
    }

    /// Load a program image. PIC18 image addresses are byte addresses; the
    /// reset vector is code at address 0, so the PC always starts there.
    pub fn load_program_image(&mut self, image: &crate::memory::ProgramImage) {
        for seg in &image.segments {
            self.load_flash(seg.start_addr as u32, &seg.data);
        }
        self.pc = 0;
    }

    pub fn wreg(&self) -> u8 {
        self.wreg
    }

    pub fn status(&self) -> u8 {
        self.status
    }

    pub fn bsr(&self) -> u8 {
        self.bsr
    }

    pub fn file_address(&self, f: u8, a: bool) -> u16 {
        if a {
            (u16::from(self.bsr) << 8) | u16::from(f)
        } else if f < self.access_split {
            u16::from(f)
        } else {
            0xF00 | u16::from(f)
        }
    }

    pub fn read_data(&self, addr: u16) -> u8 {
        match addr {
            SFR_WREG => self.wreg,
            SFR_STATUS => self.status & STATUS_FLAG_MASK,
            SFR_BSR => self.bsr,
            SFR_FSR0L => self.fsr[0] as u8,
            SFR_FSR0H => (self.fsr[0] >> 8) as u8,
            SFR_FSR1L => self.fsr[1] as u8,
            SFR_FSR1H => (self.fsr[1] >> 8) as u8,
            SFR_FSR2L => self.fsr[2] as u8,
            SFR_FSR2H => (self.fsr[2] >> 8) as u8,
            SFR_PRODL => self.prod as u8,
            SFR_PRODH => (self.prod >> 8) as u8,
            SFR_TABLAT => self.tablat,
            SFR_TBLPTRL => self.tblptr as u8,
            SFR_TBLPTRH => (self.tblptr >> 8) as u8,
            SFR_TBLPTRU => (self.tblptr >> 16) as u8,
            SFR_PCL => self.pc as u8,
            SFR_PCLATH => self.pclath,
            SFR_PCLATU => self.pclatu,
            SFR_STKPTR => self.stkptr | self.stack_flags(),
            SFR_TOSL => self.top_of_stack() as u8,
            SFR_TOSH => (self.top_of_stack() >> 8) as u8,
            SFR_TOSU => (self.top_of_stack() >> 16) as u8,
            // TXIF is read-only and set whenever TXREG is empty; this model
            // shifts a byte out the instant it is written, so it is set
            // whenever the transmitter is enabled.
            SFR_PIR1 => {
                let pir1 = self.data[usize::from(SFR_PIR1)] & !PIR1_TXIF;
                if self.data[usize::from(SFR_TXSTA)] & TXSTA_TXEN != 0 {
                    pir1 | PIR1_TXIF
                } else {
                    pir1
                }
            }
            // PORTx reads the pins: an output pin reads its latch; an input
            // pin reads 0 because no external driver is modelled yet.
            a if (SFR_PORTA..SFR_PORTA + PIC18_PORT_COUNT).contains(&a) => {
                let n = a - SFR_PORTA;
                let lat = self.data[usize::from(SFR_LATA + n)];
                let tris = self.data[usize::from(SFR_TRISA + n)];
                lat & !tris
            }
            _ => self.data.get(addr as usize).copied().unwrap_or(0),
        }
    }

    pub fn write_data(&mut self, addr: u16, value: u8) {
        match addr {
            SFR_WREG => self.wreg = value,
            SFR_STATUS => self.status = value & STATUS_FLAG_MASK,
            SFR_BSR => self.bsr = value & 0x0F,
            SFR_FSR0L => self.fsr[0] = (self.fsr[0] & 0xFF00) | u16::from(value),
            SFR_FSR0H => self.fsr[0] = (self.fsr[0] & 0x00FF) | (u16::from(value) << 8),
            SFR_FSR1L => self.fsr[1] = (self.fsr[1] & 0xFF00) | u16::from(value),
            SFR_FSR1H => self.fsr[1] = (self.fsr[1] & 0x00FF) | (u16::from(value) << 8),
            SFR_FSR2L => self.fsr[2] = (self.fsr[2] & 0xFF00) | u16::from(value),
            SFR_FSR2H => self.fsr[2] = (self.fsr[2] & 0x00FF) | (u16::from(value) << 8),
            SFR_PRODL => self.prod = (self.prod & 0xFF00) | u16::from(value),
            SFR_PRODH => self.prod = (self.prod & 0x00FF) | (u16::from(value) << 8),
            SFR_TABLAT => self.tablat = value,
            SFR_TBLPTRL => self.tblptr = (self.tblptr & 0xFFFF00) | u32::from(value),
            SFR_TBLPTRH => self.tblptr = (self.tblptr & 0xFF00FF) | (u32::from(value) << 8),
            SFR_TBLPTRU => self.tblptr = (self.tblptr & 0x00FFFF) | (u32::from(value) << 16),
            SFR_PCL => {
                let target = (u32::from(self.pclatu) << 16)
                    | (u32::from(self.pclath) << 8)
                    | u32::from(value);
                self.pc = Self::align_pc(target);
            }
            SFR_PCLATH => self.pclath = value,
            SFR_PCLATU => self.pclatu = value & 0x1F,
            SFR_STKPTR => {
                let depth = self.stack.len();
                self.stkptr = ((value & 0x1F) as usize).min(depth) as u8;
                self.stk_full = value & STKPTR_STKFUL != 0;
                self.stk_unf = value & STKPTR_STKUNF != 0;
            }
            SFR_TOSL => self.write_tos_byte(0, value),
            SFR_TOSH => self.write_tos_byte(1, value),
            SFR_TOSU => self.write_tos_byte(2, value),
            // TRMT is read-only and stays set (the shift register empties
            // instantly in this model).
            SFR_TXSTA => self.data[usize::from(SFR_TXSTA)] = value | TXSTA_TRMT,
            SFR_PIR1 => self.data[usize::from(SFR_PIR1)] = value & !PIR1_TXIF,
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
            // Writing PORTx writes the output latch (DS39631, I/O Ports).
            a if (SFR_PORTA..SFR_PORTA + PIC18_PORT_COUNT).contains(&a) => {
                self.data[usize::from(SFR_LATA + (a - SFR_PORTA))] = value;
            }
            _ => {
                if let Some(slot) = self.data.get_mut(addr as usize) {
                    *slot = value;
                }
            }
        }
    }

    /// Pick the highest-priority enabled pending interrupt.
    ///
    /// Sources modelled: the INTCON TMR0/INT0/RB flags, INTCON3 INT1/INT2, and
    /// the PIR1/PIR2 peripheral flags with their PIE/IPR enable and priority
    /// bits. Sticky flags are plain SFR bytes, so firmware clears them in its
    /// ISR exactly as on silicon. Returns `Some(true)` for a high-priority
    /// source and `Some(false)` for a low-priority one.
    ///
    /// With `RCON.IPEN = 0` every source vectors to `0x0008`: `INTCON.GIE`
    /// gates them all, and the PIR1/PIR2 peripheral sources additionally need
    /// `INTCON.PEIE` (INTCON<6>). With `IPEN = 1`, `GIEH` enables the
    /// high-priority vector and `GIEH && GIEL` the low-priority one — GIEH=0
    /// "disables all interrupts" (DS39631 Register 9-1) — so a high-priority
    /// source may nest inside a low-priority ISR but not the reverse.
    fn interrupt_request(&self) -> Option<bool> {
        let intcon = self.read_data(SFR_INTCON);
        let intcon2 = self.read_data(SFR_INTCON2);
        let intcon3 = self.read_data(SFR_INTCON3);
        let ipen = self.read_data(SFR_RCON) & RCON_IPEN != 0;

        let gie_gieh = intcon & INTCON_GIE_GIEH != 0;
        // INTCON<6> is GIEL with IPEN=1 and PEIE with IPEN=0.
        let giel_peie = intcon & INTCON_GIEL != 0;
        if !gie_gieh {
            return None;
        }
        // GIE/GIEH is set past this point, so a high-priority source is always
        // eligible; a low-priority one also needs GIEL when IPEN=1.
        let low_enabled = !ipen || giel_peie;
        let eligible = |high: bool| high || low_enabled;
        // PIR/PIE peripheral sources in compatibility mode also need PEIE.
        let peripheral_enabled = ipen || giel_peie;

        // INT0 is always a high-priority source with priority enabled; with
        // IPEN=0 it shares the single 0x0008 vector.
        if intcon & INTCON_INT0IF != 0 && intcon & INTCON_INT0IE != 0 && eligible(true) {
            return Some(true);
        }

        if intcon & INTCON_TMR0IF != 0 && intcon & INTCON_TMR0IE != 0 {
            let high = if ipen {
                intcon2 & INTCON2_TMR0IP != 0
            } else {
                true
            };
            if eligible(high) {
                return Some(high);
            }
        }

        if intcon & INTCON_RBIF != 0 && intcon & INTCON_RBIE != 0 {
            let high = if ipen {
                intcon2 & INTCON2_RBIP != 0
            } else {
                true
            };
            if eligible(high) {
                return Some(high);
            }
        }

        if intcon3 & INTCON3_INT1IF != 0 && intcon3 & INTCON3_INT1IE != 0 {
            let high = if ipen {
                intcon3 & INTCON3_INT1IP != 0
            } else {
                true
            };
            if eligible(high) {
                return Some(high);
            }
        }

        if intcon3 & INTCON3_INT2IF != 0 && intcon3 & INTCON3_INT2IE != 0 {
            let high = if ipen {
                intcon3 & INTCON3_INT2IP != 0
            } else {
                true
            };
            if eligible(high) {
                return Some(high);
            }
        }

        // Peripheral flags: PIR1/PIE1/IPR1 and PIR2/PIE2/IPR2, bit for bit.
        for (pir, pie, ipr) in [
            (SFR_PIR1, SFR_PIE1, SFR_IPR1),
            (SFR_PIR2, SFR_PIE2, SFR_IPR2),
        ] {
            let pending = self.read_data(pir);
            let enabled = self.read_data(pie);
            let priority = self.read_data(ipr);
            for bit in 0..8 {
                let mask = 1 << bit;
                if pending & mask != 0 && enabled & mask != 0 && peripheral_enabled {
                    let high = if ipen { priority & mask != 0 } else { true };
                    if eligible(high) {
                        return Some(high);
                    }
                }
            }
        }

        None
    }

    /// Whether any individually enabled interrupt source has its flag set,
    /// regardless of the global enables — the wake condition for `SLEEP`.
    fn wake_pending(&self) -> bool {
        let intcon = self.read_data(SFR_INTCON);
        let intcon3 = self.read_data(SFR_INTCON3);
        let intcon_src = (intcon & 0x07) & ((intcon >> 3) & 0x07);
        let int1 = intcon3 & INTCON3_INT1IF != 0 && intcon3 & INTCON3_INT1IE != 0;
        let int2 = intcon3 & INTCON3_INT2IF != 0 && intcon3 & INTCON3_INT2IE != 0;
        let peripheral = self.read_data(SFR_PIR1) & self.read_data(SFR_PIE1) != 0
            || self.read_data(SFR_PIR2) & self.read_data(SFR_PIE2) != 0;
        intcon_src != 0 || int1 || int2 || peripheral
    }

    /// Enter the interrupt vector for `high` priority, pushing the next
    /// instruction as the return address and clearing the matching global
    /// enable exactly as the hardware does.
    ///
    /// Entry also loads WREG, STATUS and BSR into the fast register stack
    /// (DS39631 "Fast Register Stack"), which `RETFIE FAST` restores. A
    /// high-priority entry nested in a low-priority ISR overwrites the
    /// low-priority save, as on silicon — the low ISR must then save its own
    /// context and return with a plain `RETFIE`.
    fn take_interrupt(&mut self, high: bool) {
        self.shadow = Some((self.wreg, self.status, self.bsr));
        self.push(self.pc);
        let intcon = self.read_data(SFR_INTCON);
        let ipen = self.read_data(SFR_RCON) & RCON_IPEN != 0;
        let next = if ipen {
            if high {
                intcon & !INTCON_GIE_GIEH
            } else {
                intcon & !INTCON_GIEL
            }
        } else {
            intcon & !INTCON_GIE_GIEH
        };
        self.write_data(SFR_INTCON, next);
        self.pc = if high {
            PIC18_HIGH_VECTOR
        } else {
            PIC18_LOW_VECTOR
        };
    }

    fn indirect_mode(addr: u16) -> Option<(usize, IndirectMode)> {
        match addr {
            SFR_INDF0 => Some((0, IndirectMode::Indf)),
            SFR_POSTINC0 => Some((0, IndirectMode::PostInc)),
            SFR_POSTDEC0 => Some((0, IndirectMode::PostDec)),
            SFR_PREINC0 => Some((0, IndirectMode::PreInc)),
            SFR_PLUSW0 => Some((0, IndirectMode::PlusW)),
            SFR_INDF1 => Some((1, IndirectMode::Indf)),
            SFR_POSTINC1 => Some((1, IndirectMode::PostInc)),
            SFR_POSTDEC1 => Some((1, IndirectMode::PostDec)),
            SFR_PREINC1 => Some((1, IndirectMode::PreInc)),
            SFR_PLUSW1 => Some((1, IndirectMode::PlusW)),
            SFR_INDF2 => Some((2, IndirectMode::Indf)),
            SFR_POSTINC2 => Some((2, IndirectMode::PostInc)),
            SFR_POSTDEC2 => Some((2, IndirectMode::PostDec)),
            SFR_PREINC2 => Some((2, IndirectMode::PreInc)),
            SFR_PLUSW2 => Some((2, IndirectMode::PlusW)),
            _ => None,
        }
    }

    /// Resolve an absolute data address, expanding the virtual `FSRn` aliases.
    ///
    /// Indirect addressing uses the whole 12-bit `FSRn` value; `BSR` and the
    /// instruction's `a` bit never contribute to this address. `PREINC`
    /// updates the pointer before the access. `POSTINC`/`POSTDEC` return a
    /// deferred update so a read-modify-write instruction moves the pointer
    /// only once, after it has written the original location. `PLUSW` adds a
    /// signed WREG offset and does not modify either register.
    fn resolve_data_access(&mut self, addr: u16) -> FileAccess {
        let Some((n, mode)) = Self::indirect_mode(addr) else {
            return FileAccess { addr, post: None };
        };
        match mode {
            IndirectMode::Indf => {
                let addr = self.fsr[n] & PIC18_DATA_MASK;
                FileAccess { addr, post: None }
            }
            IndirectMode::PostInc => {
                let addr = self.fsr[n] & PIC18_DATA_MASK;
                FileAccess {
                    addr,
                    post: Some((n, 1)),
                }
            }
            IndirectMode::PostDec => {
                let addr = self.fsr[n] & PIC18_DATA_MASK;
                FileAccess {
                    addr,
                    post: Some((n, -1)),
                }
            }
            IndirectMode::PreInc => {
                self.fsr[n] = self.fsr[n].wrapping_add(1) & PIC18_DATA_MASK;
                FileAccess {
                    addr: self.fsr[n],
                    post: None,
                }
            }
            IndirectMode::PlusW => {
                let offset = i16::from(self.wreg as i8);
                let addr = self.fsr[n].wrapping_add_signed(offset) & PIC18_DATA_MASK;
                FileAccess { addr, post: None }
            }
        }
    }

    fn resolve_file_access(&mut self, f: u8, a: bool) -> FileAccess {
        let addr = self.file_address(f, a);
        self.resolve_data_access(addr)
    }

    fn is_virtual_indirect(addr: u16) -> bool {
        Self::indirect_mode(addr).is_some()
    }

    fn is_fsr_pair(addr: u16) -> bool {
        matches!(
            addr,
            SFR_FSR0L | SFR_FSR0H | SFR_FSR1L | SFR_FSR1H | SFR_FSR2L | SFR_FSR2H
        )
    }

    /// Read through a resolved file target. Virtual indirect operands are not
    /// physically implemented, so an indirect read that lands on one returns
    /// zero (DS41303F §5.4.3.3).
    fn read_file_data(&self, addr: u16) -> u8 {
        if Self::is_virtual_indirect(addr) {
            0
        } else {
            self.read_data(addr)
        }
    }

    /// Write through a resolved file target. Returns false when the target is
    /// a virtual indirect operand, which makes the indirect write a NOP.
    fn write_file_data(&mut self, addr: u16, value: u8) -> bool {
        if Self::is_virtual_indirect(addr) {
            return false;
        }
        self.write_data(addr, value);
        true
    }

    fn apply_post_action(&mut self, access: FileAccess) {
        if let Some((n, delta)) = access.post {
            self.fsr[n] = self.fsr[n].wrapping_add_signed(delta as i16) & PIC18_DATA_MASK;
        }
    }

    fn complete_file_read(&mut self, access: FileAccess) {
        self.apply_post_action(access);
    }

    /// Complete a file write. A write that targets an FSR pair is documented
    /// to store the value without the selected operand's post-increment or
    /// post-decrement (DS41303F §5.4.3.3). A rejected write to a virtual
    /// operand is a NOP, including its pointer action.
    fn complete_file_write(&mut self, access: FileAccess, wrote: bool) {
        if !wrote || Self::is_fsr_pair(access.addr) {
            return;
        }
        self.apply_post_action(access);
    }

    fn read_file_access(&mut self, f: u8, a: bool) -> (u8, FileAccess) {
        let access = self.resolve_file_access(f, a);
        let value = self.read_file_data(access.addr);
        (value, access)
    }

    pub fn read_file(&mut self, f: u8, a: bool) -> u8 {
        let (value, access) = self.read_file_access(f, a);
        self.complete_file_read(access);
        value
    }

    pub fn write_file(&mut self, f: u8, a: bool, value: u8) {
        let access = self.resolve_file_access(f, a);
        let wrote = self.write_file_data(access.addr, value);
        self.complete_file_write(access, wrote);
    }

    pub fn stack_pointer(&self) -> u8 {
        self.stkptr
    }

    fn align_pc(pc: u32) -> u32 {
        pc & PIC18_PC_MASK
    }

    fn stack_flags(&self) -> u8 {
        (u8::from(self.stk_full) * STKPTR_STKFUL) | (u8::from(self.stk_unf) * STKPTR_STKUNF)
    }

    fn top_of_stack(&self) -> u32 {
        if self.stkptr == 0 {
            0
        } else {
            self.stack[self.stkptr as usize - 1]
        }
    }

    fn write_tos_byte(&mut self, byte: u32, value: u8) {
        if self.stkptr == 0 {
            return;
        }
        let idx = self.stkptr as usize - 1;
        let shift = byte * 8;
        let mask = !(0xFFu32 << shift);
        self.stack[idx] = (self.stack[idx] & mask) | (u32::from(value) << shift);
    }

    fn push(&mut self, value: u32) {
        let value = Self::align_pc(value);
        let depth = self.stack.len();
        let idx = self.stkptr as usize;
        if idx >= depth {
            self.stk_full = true;
            self.stack[depth - 1] = value;
        } else {
            self.stack[idx] = value;
            self.stkptr += 1;
        }
    }

    fn pop(&mut self) -> u32 {
        if self.stkptr == 0 {
            self.stk_unf = true;
            return 0;
        }
        self.stkptr -= 1;
        self.stack[self.stkptr as usize]
    }

    fn fetch_word(&self, addr: u32) -> u16 {
        let addr = (addr & !1) as usize;
        match (self.flash.get(addr), self.flash.get(addr + 1)) {
            (Some(lo), Some(hi)) => u16::from_le_bytes([*lo, *hi]),
            _ => 0xFFFF,
        }
    }

    fn is_two_word_first(word: u16) -> bool {
        word & 0xF000 == 0xC000
            || word & 0xFF00 == 0xEF00
            || word & 0xFE00 == 0xEC00
            || word & 0xFFC0 == 0xEE00
    }

    fn skip_next(&mut self, pc: u32) {
        let next = pc.wrapping_add(2);
        let second = self.fetch_word(next);
        let len = if Self::is_two_word_first(second) {
            4
        } else {
            2
        };
        self.pc = Self::align_pc(next.wrapping_add(len));
    }

    fn set_nz(&mut self, value: u8) {
        self.status &= !(STATUS_Z | STATUS_N);
        if value == 0 {
            self.status |= STATUS_Z;
        }
        if value & 0x80 != 0 {
            self.status |= STATUS_N;
        }
    }

    fn set_z_only(&mut self, value: u8) {
        if value == 0 {
            self.status |= STATUS_Z;
        } else {
            self.status &= !STATUS_Z;
        }
    }

    fn set_add_flags(&mut self, a: u8, b: u8, result: u8) {
        let mut flags = 0;
        if u16::from(a) + u16::from(b) > 0xFF {
            flags |= STATUS_C;
        }
        if (a & 0x0F) + (b & 0x0F) > 0x0F {
            flags |= STATUS_DC;
        }
        if (a ^ result) & (b ^ result) & 0x80 != 0 {
            flags |= STATUS_OV;
        }
        if result == 0 {
            flags |= STATUS_Z;
        }
        if result & 0x80 != 0 {
            flags |= STATUS_N;
        }
        self.status = (self.status & !STATUS_FLAG_MASK) | flags;
    }

    fn set_sub_flags(&mut self, a: u8, b: u8, result: u8) {
        let mut flags = 0;
        if a >= b {
            flags |= STATUS_C;
        }
        if (a & 0x0F) >= (b & 0x0F) {
            flags |= STATUS_DC;
        }
        if (a ^ b) & (a ^ result) & 0x80 != 0 {
            flags |= STATUS_OV;
        }
        if result == 0 {
            flags |= STATUS_Z;
        }
        if result & 0x80 != 0 {
            flags |= STATUS_N;
        }
        self.status = (self.status & !STATUS_FLAG_MASK) | flags;
    }

    fn write_result_at(&mut self, access: FileAccess, d: bool, value: u8) -> bool {
        if d {
            if access.addr == SFR_STATUS {
                return true;
            }
            self.write_file_data(access.addr, value)
        } else {
            self.wreg = value;
            false
        }
    }

    fn complete_result(&mut self, access: FileAccess, d: bool, wrote: bool) {
        if d {
            self.complete_file_write(access, wrote);
        } else {
            self.complete_file_read(access);
        }
    }

    fn execute(&mut self, word: u16, pc: u32) -> SimResult<u32> {
        let next = pc.wrapping_add(2);

        if word & 0xF000 == 0xC000 {
            let second = self.fetch_word(next);
            if second & 0xF000 != 0xF000 {
                return Err(SimulationError::DecodeError(u64::from(pc)));
            }
            // Both operands are 12-bit absolute addresses (no BSR), and each
            // may be a virtual indirect alias: the source is read in the
            // first cycle and its FSR updated, then the destination is
            // resolved and written. Compiled code depends on this — SDCC's
            // pic16 software stack is `MOVFF r, POSTDEC1` / `MOVFF PREINC1, r`
            // / `MOVFF PLUSW2, r`.
            let src = self.resolve_data_access(word & 0x0FFF);
            let value = self.read_file_data(src.addr);
            self.complete_file_read(src);
            self.pc = Self::align_pc(pc.wrapping_add(4));
            let dst = self.resolve_data_access(second & 0x0FFF);
            let wrote = self.write_file_data(dst.addr, value);
            self.complete_file_write(dst, wrote);
            return Ok(2);
        }

        if word & 0xFF00 == 0xEF00 {
            let second = self.fetch_word(next);
            if second & 0xF000 != 0xF000 {
                return Err(SimulationError::DecodeError(u64::from(pc)));
            }
            let k = (u32::from(second & 0x0FFF) << 8) | u32::from(word & 0x00FF);
            self.pc = Self::align_pc(k << 1);
            return Ok(2);
        }

        if word & 0xFE00 == 0xEC00 {
            let second = self.fetch_word(next);
            if second & 0xF000 != 0xF000 {
                return Err(SimulationError::DecodeError(u64::from(pc)));
            }
            let k = (u32::from(second & 0x0FFF) << 8) | u32::from(word & 0x00FF);
            if word & 0x0100 != 0 {
                self.shadow = Some((self.wreg, self.status, self.bsr));
            }
            self.push(pc.wrapping_add(4));
            self.pc = Self::align_pc(k << 1);
            return Ok(2);
        }

        if word & 0xFFC0 == 0xEE00 {
            let second = self.fetch_word(next);
            if second & 0xFF00 != 0xF000 {
                return Err(SimulationError::DecodeError(u64::from(pc)));
            }
            let f = usize::from((word >> 4) & 0x3);
            if f > 2 {
                return Err(SimulationError::DecodeError(u64::from(pc)));
            }
            let k = ((word & 0x000F) << 8) | (second & 0x00FF);
            self.fsr[f] = k;
            self.pc = Self::align_pc(pc.wrapping_add(4));
            return Ok(2);
        }

        if word & 0xF000 == 0xF000 || word == 0x0000 {
            return Ok(1);
        }

        if word & 0xFF00 == 0x0E00 {
            self.wreg = word as u8;
            return Ok(1);
        }

        if word & 0xFF00 == 0x0C00 {
            self.wreg = word as u8;
            self.pc = Self::align_pc(self.pop());
            return Ok(2);
        }

        if word & 0xFFF0 == 0x0100 {
            self.bsr = (word & 0x000F) as u8;
            return Ok(1);
        }

        if word & 0xFFFE == 0x0010 {
            let restore = word & 1 != 0;
            self.pc = Self::align_pc(self.pop());
            let intcon = self.read_data(SFR_INTCON);
            let ipen = self.read_data(SFR_RCON) & RCON_IPEN != 0;
            // With IPEN=1, RETFIE re-enables only the level it is returning
            // from: GIEH if it is clear (leaving a high ISR, possibly nested
            // in a low one, whose GIEL must stay clear), else GIEL.
            self.write_data(
                SFR_INTCON,
                if ipen && intcon & INTCON_GIE_GIEH != 0 {
                    intcon | INTCON_GIEL
                } else {
                    intcon | INTCON_GIE_GIEH
                },
            );
            if restore {
                if let Some((wreg, status, bsr)) = self.shadow {
                    self.wreg = wreg;
                    self.status = status & STATUS_FLAG_MASK;
                    self.bsr = bsr;
                }
            }
            return Ok(2);
        }

        if word & 0xFFFE == 0x0012 {
            let restore = word & 1 != 0;
            self.pc = Self::align_pc(self.pop());
            if restore {
                if let Some((wreg, status, bsr)) = self.shadow {
                    self.wreg = wreg;
                    self.status = status & STATUS_FLAG_MASK;
                    self.bsr = bsr;
                }
            }
            return Ok(2);
        }

        if let Some(cycles) = self.execute_control(word, pc)? {
            return Ok(cycles);
        }
        if let Some(cycles) = self.execute_literal(word) {
            return Ok(cycles);
        }
        if let Some(cycles) = self.execute_byte_extended(word, pc) {
            return Ok(cycles);
        }

        if word & 0xFE00 == 0x6E00 {
            let a = word & 0x0100 != 0;
            let f = word as u8;
            self.write_file(f, a, self.wreg);
            return Ok(1);
        }

        if word & 0xFE00 == 0x6A00 {
            let a = word & 0x0100 != 0;
            let f = word as u8;
            let access = self.resolve_file_access(f, a);
            let wrote = if access.addr == SFR_STATUS {
                true
            } else {
                self.write_file_data(access.addr, 0)
            };
            self.set_z_only(0);
            self.complete_file_write(access, wrote);
            return Ok(1);
        }

        if word & 0xFC00 == 0x5000 {
            let d = word & 0x0200 != 0;
            let a = word & 0x0100 != 0;
            let f = word as u8;
            let (value, access) = self.read_file_access(f, a);
            self.set_nz(value);
            let wrote = self.write_result_at(access, d, value);
            self.complete_result(access, d, wrote);
            return Ok(1);
        }

        if word & 0xFC00 == 0x2400 {
            let d = word & 0x0200 != 0;
            let a = word & 0x0100 != 0;
            let f = word as u8;
            let access = self.resolve_file_access(f, a);
            // Reading PCL exposes this instruction's own address and latches
            // the upper PC bytes; it does not expose the already-advanced PC.
            let b = if access.addr == SFR_PCL {
                self.pclath = (pc >> 8) as u8;
                self.pclatu = ((pc >> 16) & 0x1F) as u8;
                pc as u8
            } else {
                self.read_data(access.addr)
            };
            let result = self.wreg.wrapping_add(b);
            self.set_add_flags(self.wreg, b, result);
            let wrote = if d && access.addr == SFR_PCL {
                // Computed GOTO: the addition does not propagate into the
                // upper PC bytes (the table must not cross a 256-byte page).
                self.pc = Self::align_pc((pc & 0xFFFF00) | u32::from(result));
                true
            } else {
                self.write_result_at(access, d, result)
            };
            self.complete_result(access, d, wrote);
            return Ok(1);
        }

        if word & 0xFC00 == 0x1400 {
            let d = word & 0x0200 != 0;
            let a = word & 0x0100 != 0;
            let f = word as u8;
            let (value, access) = self.read_file_access(f, a);
            let result = self.wreg & value;
            self.set_nz(result);
            let wrote = self.write_result_at(access, d, result);
            self.complete_result(access, d, wrote);
            return Ok(1);
        }

        if word & 0xFC00 == 0x1000 {
            let d = word & 0x0200 != 0;
            let a = word & 0x0100 != 0;
            let f = word as u8;
            let (value, access) = self.read_file_access(f, a);
            let result = self.wreg | value;
            self.set_nz(result);
            let wrote = self.write_result_at(access, d, result);
            self.complete_result(access, d, wrote);
            return Ok(1);
        }

        if word & 0xFC00 == 0x1800 {
            let d = word & 0x0200 != 0;
            let a = word & 0x0100 != 0;
            let f = word as u8;
            let (value, access) = self.read_file_access(f, a);
            let result = self.wreg ^ value;
            self.set_nz(result);
            let wrote = self.write_result_at(access, d, result);
            self.complete_result(access, d, wrote);
            return Ok(1);
        }

        if word & 0xFC00 == 0x0400 {
            let d = word & 0x0200 != 0;
            let a = word & 0x0100 != 0;
            let f = word as u8;
            let (b, access) = self.read_file_access(f, a);
            let result = b.wrapping_sub(1);
            self.set_sub_flags(b, 1, result);
            let wrote = self.write_result_at(access, d, result);
            self.complete_result(access, d, wrote);
            return Ok(1);
        }

        if word & 0xFC00 == 0x2800 {
            let d = word & 0x0200 != 0;
            let a = word & 0x0100 != 0;
            let f = word as u8;
            let (b, access) = self.read_file_access(f, a);
            let result = b.wrapping_add(1);
            self.set_add_flags(b, 1, result);
            let wrote = self.write_result_at(access, d, result);
            self.complete_result(access, d, wrote);
            return Ok(1);
        }

        if word & 0xF000 == 0x9000 || word & 0xF000 == 0x8000 || word & 0xF000 == 0x7000 {
            let a = word & 0x0100 != 0;
            let b = ((word >> 9) & 0x7) as u8;
            let f = word as u8;
            let (value, access) = self.read_file_access(f, a);
            let mask = 1u8 << b;
            let result = match word & 0xF000 {
                0x9000 => value & !mask,
                0x8000 => value | mask,
                _ => value ^ mask,
            };
            let wrote = self.write_file_data(access.addr, result);
            self.complete_file_write(access, wrote);
            return Ok(1);
        }

        if word & 0xF000 == 0xB000 || word & 0xF000 == 0xA000 {
            let a = word & 0x0100 != 0;
            let b = ((word >> 9) & 0x7) as u8;
            let f = word as u8;
            let (value, access) = self.read_file_access(f, a);
            self.complete_file_read(access);
            let set = value & (1u8 << b) != 0;
            let skip = if word & 0xF000 == 0xB000 { !set } else { set };
            if skip {
                self.skip_next(pc);
                return Ok(2);
            }
            return Ok(1);
        }

        if word & 0xF800 == 0xD000 {
            let offset = Self::sign_extend(word & 0x07FF, 11);
            self.pc = Self::align_pc(next.wrapping_add_signed(offset * 2));
            return Ok(2);
        }

        if word & 0xF800 == 0xD800 {
            let offset = Self::sign_extend(word & 0x07FF, 11);
            self.push(next);
            self.pc = Self::align_pc(next.wrapping_add_signed(offset * 2));
            return Ok(2);
        }

        if word & 0xF800 == 0xE000 {
            let condition = ((word >> 8) & 0x7) as u8;
            let taken = match condition {
                0 => self.status & STATUS_Z != 0,
                1 => self.status & STATUS_Z == 0,
                2 => self.status & STATUS_C != 0,
                3 => self.status & STATUS_C == 0,
                4 => self.status & STATUS_OV != 0,
                5 => self.status & STATUS_OV == 0,
                6 => self.status & STATUS_N != 0,
                _ => self.status & STATUS_N == 0,
            };
            if taken {
                let offset = Self::sign_extend(word & 0x00FF, 8);
                self.pc = Self::align_pc(next.wrapping_add_signed(offset * 2));
                return Ok(2);
            }
            return Ok(1);
        }

        Err(SimulationError::DecodeError(u64::from(pc)))
    }

    /// Inherent control words in the `0x0003..=0x00FF` range (DS39631
    /// Table 26-2): `SLEEP`, `CLRWDT`, `PUSH`, `POP`, `DAW`, the eight table
    /// read/write forms, and `RESET`. `self.pc` already points past `word`.
    fn execute_control(&mut self, word: u16, pc: u32) -> SimResult<Option<u32>> {
        let cycles = match word {
            0x0003 => {
                // SLEEP: TO = 1, PD = 0; the core idles until a wake source.
                let rcon = self.read_data(SFR_RCON);
                self.write_data(SFR_RCON, (rcon | RCON_TO) & !RCON_PD);
                self.asleep = true;
                1
            }
            0x0004 => {
                // CLRWDT: no watchdog is modelled; only TO/PD are visible.
                let rcon = self.read_data(SFR_RCON);
                self.write_data(SFR_RCON, rcon | RCON_TO | RCON_PD);
                1
            }
            0x0005 => {
                self.push(pc.wrapping_add(2));
                1
            }
            0x0006 => {
                // POP discards the top of stack; underflow sets STKUNF.
                if self.stkptr == 0 {
                    self.stk_unf = true;
                } else {
                    self.stkptr -= 1;
                }
                1
            }
            0x0007 => {
                let mut w = u16::from(self.wreg);
                if w & 0x0F > 0x09 || self.status & STATUS_DC != 0 {
                    w += 0x06;
                }
                if w & 0x1F0 > 0x90 || self.status & STATUS_C != 0 {
                    w += 0x60;
                }
                if w > 0xFF {
                    self.status |= STATUS_C;
                }
                self.wreg = w as u8;
                1
            }
            0x0008..=0x000B => {
                // TBLRD *, *+, *-, +*
                if word == 0x000B {
                    self.tblptr = self.tblptr.wrapping_add(1) & PIC18_TBLPTR_MASK;
                }
                self.tablat = self.program_byte(self.tblptr);
                match word {
                    0x0009 => self.tblptr = self.tblptr.wrapping_add(1) & PIC18_TBLPTR_MASK,
                    0x000A => self.tblptr = self.tblptr.wrapping_sub(1) & PIC18_TBLPTR_MASK,
                    _ => {}
                }
                2
            }
            0x000C..=0x000F => {
                // TBLWT *, *+, *-, +*: only the pointer side effect is
                // modelled. The holding-register write and the EECON1
                // erase/write sequence that would commit it are not, so
                // program memory never changes.
                match word {
                    0x000D => self.tblptr = self.tblptr.wrapping_add(1) & PIC18_TBLPTR_MASK,
                    0x000E => self.tblptr = self.tblptr.wrapping_sub(1) & PIC18_TBLPTR_MASK,
                    0x000F => self.tblptr = self.tblptr.wrapping_add(1) & PIC18_TBLPTR_MASK,
                    _ => {}
                }
                2
            }
            0x00FF => {
                self.reset_core();
                1
            }
            _ => return Ok(None),
        };
        Ok(Some(cycles))
    }

    /// Literal ALU forms: `SUBLW`, `IORLW`, `XORLW`, `ANDLW`, `MULLW`,
    /// `ADDLW`. (`MOVLW`, `RETLW` and `MOVLB` are decoded earlier.)
    fn execute_literal(&mut self, word: u16) -> Option<u32> {
        let k = word as u8;
        match word & 0xFF00 {
            0x0800 => {
                let result = k.wrapping_sub(self.wreg);
                self.set_sub_flags(k, self.wreg, result);
                self.wreg = result;
            }
            0x0900 => {
                self.wreg |= k;
                self.set_nz(self.wreg);
            }
            0x0A00 => {
                self.wreg ^= k;
                self.set_nz(self.wreg);
            }
            0x0B00 => {
                self.wreg &= k;
                self.set_nz(self.wreg);
            }
            0x0D00 => self.prod = u16::from(self.wreg) * u16::from(k),
            0x0F00 => {
                let result = self.wreg.wrapping_add(k);
                self.set_add_flags(self.wreg, k, result);
                self.wreg = result;
            }
            _ => return None,
        }
        Some(1)
    }

    /// The remaining byte-oriented file operations: the `a`-only compare,
    /// test, multiply, set and negate forms, and the `d,a` carry/borrow
    /// arithmetic, complement, rotate, swap and skip-on-result forms.
    fn execute_byte_extended(&mut self, word: u16, pc: u32) -> Option<u32> {
        let a = word & 0x0100 != 0;
        let f = word as u8;

        match word & 0xFE00 {
            0x0200 => {
                // MULWF: PRODH:PRODL = W * f, no flags.
                let (value, access) = self.read_file_access(f, a);
                self.complete_file_read(access);
                self.prod = u16::from(self.wreg) * u16::from(value);
                return Some(1);
            }
            0x6000 | 0x6200 | 0x6400 | 0x6600 => {
                // CPFSLT / CPFSEQ / CPFSGT (unsigned compare with W) and
                // TSTFSZ; none of them touch STATUS.
                let (value, access) = self.read_file_access(f, a);
                self.complete_file_read(access);
                let skip = match word & 0xFE00 {
                    0x6000 => value < self.wreg,
                    0x6200 => value == self.wreg,
                    0x6400 => value > self.wreg,
                    _ => value == 0,
                };
                return Some(self.skip_if(skip, pc));
            }
            0x6800 => {
                // SETF: no flags.
                let access = self.resolve_file_access(f, a);
                let wrote = self.write_file_data(access.addr, 0xFF);
                self.complete_file_write(access, wrote);
                return Some(1);
            }
            0x6C00 => {
                // NEGF: f = -f, flags as for 0 - f.
                let (value, access) = self.read_file_access(f, a);
                let result = 0u8.wrapping_sub(value);
                self.set_sub_flags(0, value, result);
                let wrote = self.write_result_at(access, true, result);
                self.complete_result(access, true, wrote);
                return Some(1);
            }
            _ => {}
        }

        let d = word & 0x0200 != 0;
        let op = word & 0xFC00;
        if !matches!(
            op,
            0x2000
                | 0x5400
                | 0x5C00
                | 0x5800
                | 0x1C00
                | 0x3400
                | 0x4400
                | 0x3000
                | 0x4000
                | 0x3800
                | 0x2C00
                | 0x4C00
                | 0x3C00
                | 0x4800
        ) {
            return None;
        }

        let (value, access) = self.read_file_access(f, a);
        let carry = u8::from(self.status & STATUS_C != 0);
        let w = self.wreg;
        let (result, skip) = match op {
            0x2000 => {
                // ADDWFC: W + f + C.
                let result = w.wrapping_add(value).wrapping_add(carry);
                self.set_add_carry_flags(w, value, carry, result);
                (result, None)
            }
            0x5400 => {
                // SUBFWB: W - f - !C.
                let result = w.wrapping_sub(value).wrapping_sub(1 - carry);
                self.set_sub_borrow_flags(w, value, 1 - carry, result);
                (result, None)
            }
            0x5C00 => {
                // SUBWF: f - W.
                let result = value.wrapping_sub(w);
                self.set_sub_flags(value, w, result);
                (result, None)
            }
            0x5800 => {
                // SUBWFB: f - W - !C.
                let result = value.wrapping_sub(w).wrapping_sub(1 - carry);
                self.set_sub_borrow_flags(value, w, 1 - carry, result);
                (result, None)
            }
            0x1C00 => {
                let result = !value;
                self.set_nz(result);
                (result, None)
            }
            0x3400 => {
                // RLCF: C <- f<7>, f<0> <- C.
                let result = (value << 1) | carry;
                self.set_carry(value & 0x80 != 0);
                self.set_nz(result);
                (result, None)
            }
            0x4400 => {
                let result = value.rotate_left(1);
                self.set_nz(result);
                (result, None)
            }
            0x3000 => {
                // RRCF: C <- f<0>, f<7> <- C.
                let result = (value >> 1) | (carry << 7);
                self.set_carry(value & 0x01 != 0);
                self.set_nz(result);
                (result, None)
            }
            0x4000 => {
                let result = value.rotate_right(1);
                self.set_nz(result);
                (result, None)
            }
            0x3800 => (value.rotate_left(4), None),
            0x2C00 => {
                let result = value.wrapping_sub(1);
                (result, Some(result == 0))
            }
            0x4C00 => {
                let result = value.wrapping_sub(1);
                (result, Some(result != 0))
            }
            0x3C00 => {
                let result = value.wrapping_add(1);
                (result, Some(result == 0))
            }
            _ => {
                let result = value.wrapping_add(1);
                (result, Some(result != 0))
            }
        };

        // SWAPF and the skip forms leave STATUS alone, so a STATUS
        // destination takes the result; flag-setting forms let the flags win
        // (the same rule `write_result_at` applies to the existing ALU ops).
        let flagless = op == 0x3800 || skip.is_some();
        let wrote = if flagless && d {
            self.write_file_data(access.addr, result)
        } else {
            self.write_result_at(access, d, result)
        };
        self.complete_result(access, d, wrote);
        Some(match skip {
            Some(skip) => self.skip_if(skip, pc),
            None => 1,
        })
    }

    fn skip_if(&mut self, skip: bool, pc: u32) -> u32 {
        if skip {
            self.skip_next(pc);
            2
        } else {
            1
        }
    }

    fn set_carry(&mut self, carry: bool) {
        if carry {
            self.status |= STATUS_C;
        } else {
            self.status &= !STATUS_C;
        }
    }

    fn set_add_carry_flags(&mut self, a: u8, b: u8, carry: u8, result: u8) {
        let mut flags = 0;
        if u16::from(a) + u16::from(b) + u16::from(carry) > 0xFF {
            flags |= STATUS_C;
        }
        if (a & 0x0F) + (b & 0x0F) + carry > 0x0F {
            flags |= STATUS_DC;
        }
        if (a ^ result) & (b ^ result) & 0x80 != 0 {
            flags |= STATUS_OV;
        }
        if result == 0 {
            flags |= STATUS_Z;
        }
        if result & 0x80 != 0 {
            flags |= STATUS_N;
        }
        self.status = (self.status & !STATUS_FLAG_MASK) | flags;
    }

    fn set_sub_borrow_flags(&mut self, a: u8, b: u8, borrow: u8, result: u8) {
        let mut flags = 0;
        if u16::from(a) >= u16::from(b) + u16::from(borrow) {
            flags |= STATUS_C;
        }
        if (a & 0x0F) >= (b & 0x0F) + borrow {
            flags |= STATUS_DC;
        }
        if (a ^ b) & (a ^ result) & 0x80 != 0 {
            flags |= STATUS_OV;
        }
        if result == 0 {
            flags |= STATUS_Z;
        }
        if result & 0x80 != 0 {
            flags |= STATUS_N;
        }
        self.status = (self.status & !STATUS_FLAG_MASK) | flags;
    }

    /// Program-space byte for table reads. Flash below the modelled size
    /// reads back its contents; the user ID (`0x200000..0x200008`) and
    /// configuration (`0x300000..0x300010`) windows read back what the image
    /// programmed, or `0xFF` (erased) where it programmed nothing. Every other
    /// address — including the device ID words, which are not modelled —
    /// reads `0`, the documented value for unimplemented program memory.
    fn program_byte(&self, addr: u32) -> u8 {
        let addr = addr & PIC18_TBLPTR_MASK;
        if let Some(byte) = self.flash.get(addr as usize) {
            return *byte;
        }
        match addr {
            PIC18_IDLOC_BASE..=0x20_0007 => self.idloc[(addr - PIC18_IDLOC_BASE) as usize],
            PIC18_CONFIG_BASE..=0x30_000F => self.config[(addr - PIC18_CONFIG_BASE) as usize],
            _ => 0,
        }
    }

    fn sign_extend(value: u16, bits: u32) -> i32 {
        let shift = 32 - bits;
        (i32::from(value) << shift) >> shift
    }

    fn register_view(&self) -> [u32; 8] {
        [
            u32::from(self.wreg),
            u32::from(self.status),
            u32::from(self.bsr),
            u32::from(self.fsr[0]),
            u32::from(self.fsr[1]),
            u32::from(self.fsr[2]),
            u32::from(self.stkptr),
            self.pc,
        ]
    }
}

impl Cpu for Pic18 {
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
            // SLEEP: the core idles one instruction cycle at a time until an
            // enabled source raises its flag. Waking does not need GIE; with
            // GIE clear execution resumes after SLEEP, with it set the
            // vector is taken (DS39631, "Wake-up from Sleep and Idle").
            self.cycles += 1;
            crate::advance_batch_cycle(bus, 1);
            if !self.wake_pending() {
                return Ok(());
            }
            self.asleep = false;
            if let Some(high) = self.interrupt_request() {
                self.take_interrupt(high);
            }
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

        self.pc = pc.wrapping_add(2);
        let cycles = self.execute(word, pc)?;
        self.cycles += u64::from(cycles);
        crate::advance_batch_cycle(bus, u64::from(cycles));

        // Interrupts are sampled between instructions: the instruction that
        // just retired completes, then a pending enabled source vectors and
        // pushes the next instruction as the return address.
        if let Some(high) = self.interrupt_request() {
            self.take_interrupt(high);
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
        2
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = Self::align_pc(val);
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, val: u32) {
        let depth = self.stack.len();
        self.stkptr = (val as usize).min(depth) as u8;
    }

    /// Recorded for the `Cpu` contract but not consumed: PIC18 interrupts
    /// are raised by firmware-visible PIRx/INTCONx flags, which
    /// `interrupt_request` samples directly. No bus peripheral drives those
    /// flags yet, so nothing calls this on a PIC18 system.
    fn set_exception_pending(&mut self, exception_num: u32) {
        self.exception_pending = Some(exception_num);
    }

    fn get_register(&self, id: u8) -> u32 {
        self.register_view().get(id as usize).copied().unwrap_or(0)
    }

    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0 => self.wreg = val as u8,
            1 => self.status = val as u8 & STATUS_FLAG_MASK,
            2 => self.bsr = val as u8 & 0x0F,
            3 => self.fsr[0] = val as u16,
            4 => self.fsr[1] = val as u16,
            5 => self.fsr[2] = val as u16,
            6 => self.set_sp(val),
            7 => self.pc = Self::align_pc(val),
            _ => {}
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        vec![
            "WREG".into(),
            "STATUS".into(),
            "BSR".into(),
            "FSR0".into(),
            "FSR1".into(),
            "FSR2".into(),
            "SP".into(),
            "PC".into(),
        ]
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        match name.trim().to_ascii_uppercase().as_str() {
            "W" | "WREG" => Some(0),
            "STATUS" => Some(1),
            "BSR" => Some(2),
            "FSR0" => Some(3),
            "FSR1" => Some(4),
            "FSR2" => Some(5),
            "SP" | "STKPTR" => Some(6),
            "PC" | "PCL" => Some(7),
            _ => None,
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::Pic18(Box::new(Pic18CpuSnapshot {
            wreg: self.wreg,
            status: self.status,
            bsr: self.bsr,
            pc: self.pc,
            cycles: self.cycles,
            fsr: self.fsr,
            tblptr: self.tblptr,
            tablat: self.tablat,
            pclath: self.pclath,
            pclatu: self.pclatu,
            prod: self.prod,
            stkptr: self.stkptr,
            stack: self.stack.clone(),
            stk_full: self.stk_full,
            stk_unf: self.stk_unf,
            asleep: self.asleep,
        }))
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Pic18(s) = snapshot {
            self.wreg = s.wreg;
            self.status = s.status & STATUS_FLAG_MASK;
            self.bsr = s.bsr & 0x0F;
            self.pc = Self::align_pc(s.pc);
            self.cycles = s.cycles;
            self.fsr = s.fsr;
            self.tblptr = s.tblptr;
            self.tablat = s.tablat;
            self.pclath = s.pclath;
            self.pclatu = s.pclatu & 0x1F;
            self.prod = s.prod;
            for (slot, value) in self.stack.iter_mut().zip(s.stack.iter()) {
                *slot = *value & PIC18_PC_MASK;
            }
            let depth = self.stack.len();
            self.stkptr = (s.stkptr as usize).min(depth) as u8;
            self.stk_full = s.stk_full;
            self.stk_unf = s.stk_unf;
            self.asleep = s.asleep;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::ProgramImage;

    fn run(cpu: &mut Pic18, count: usize) {
        let mut bus = crate::bus::SystemBus::new();
        for _ in 0..count {
            cpu.step(&mut bus, &[], &SimulationConfig::default())
                .expect("fixture instruction must decode");
        }
    }

    fn run_err(cpu: &mut Pic18, count: usize) -> SimulationError {
        let mut bus = crate::bus::SystemBus::new();
        let mut last = None;
        for _ in 0..count {
            match cpu.step(&mut bus, &[], &SimulationConfig::default()) {
                Ok(()) => {}
                Err(e) => {
                    last = Some(e);
                    break;
                }
            }
        }
        last.expect("expected a decode error")
    }

    #[test]
    fn reset_state_and_pc_alignment() {
        let mut cpu = Pic18::new();
        let mut bus = crate::bus::SystemBus::new();
        cpu.reset(&mut bus).unwrap();
        assert_eq!(cpu.get_pc(), 0);
        assert_eq!(cpu.wreg(), 0);
        assert_eq!(cpu.status(), 0);
        assert_eq!(cpu.bsr(), 0);
        assert_eq!(cpu.stack_pointer(), 0);
        cpu.set_pc(0x1235);
        assert_eq!(cpu.get_pc(), 0x1234);
        cpu.set_register(7, 0x0003);
        assert_eq!(cpu.get_pc(), 0x0002);
    }

    #[test]
    fn access_bank_and_bsr_resolution() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[
            0x0E42, // MOVLW 0x42
            0x6E20, // MOVWF 0x20, access
            0x0101, // MOVLB 1
            0x0E99, // MOVLW 0x99
            0x6F20, // MOVWF 0x20, a=1 -> bank 1
            0x0E05, // MOVLW 0x05
            0x6ED8, // MOVWF STATUS through the access-bank SFR window
        ]);
        run(&mut cpu, 7);
        assert_eq!(cpu.data[0x020], 0x42);
        assert_eq!(cpu.data[0x120], 0x99);
        assert_eq!(cpu.wreg(), 0x05);
        assert_eq!(cpu.status(), 0x05);
    }

    #[test]
    fn indirect_aliases_use_their_fsr_not_bsr() {
        let mut cpu = Pic18::new();
        cpu.fsr = [0x020, 0x120, 0x220];
        cpu.bsr = 0x0F;
        cpu.load_words(&[
            0x0E11, // MOVLW 0x11
            0x6EEF, // MOVWF INDF0
            0x0E22, // MOVLW 0x22
            0x6EE7, // MOVWF INDF1
            0x0E33, // MOVLW 0x33
            0x6EDF, // MOVWF INDF2
            0x50EF, // MOVF INDF0,W
        ]);
        run(&mut cpu, 7);
        assert_eq!(cpu.read_data(0x020), 0x11);
        assert_eq!(cpu.read_data(0x120), 0x22);
        assert_eq!(cpu.read_data(0x220), 0x33);
        assert_eq!(cpu.read_data(0xF20), 0);
        assert_eq!(cpu.fsr, [0x020, 0x120, 0x220]);
        assert_eq!(cpu.bsr(), 0x0F);
        assert_eq!(cpu.wreg(), 0x11);
    }

    #[test]
    fn postinc_read_write_and_rmw_move_the_pointer_once() {
        let mut cpu = Pic18::new();
        cpu.fsr[0] = 0x020;
        cpu.write_data(0x020, 0x11);
        cpu.load_words(&[
            0x50EE, // MOVF POSTINC0,W -> W=0x11, FSR=0x21
            0x0E33, // MOVLW 0x33
            0x6EEE, // MOVWF POSTINC0 -> [0x21]=0x33, FSR=0x22
            0x2AEE, // INCF POSTINC0,F -> [0x22]=1, FSR=0x23 (one update)
        ]);
        run(&mut cpu, 4);
        assert_eq!(cpu.wreg(), 0x33);
        assert_eq!(cpu.read_data(0x021), 0x33);
        assert_eq!(cpu.read_data(0x022), 1);
        assert_eq!(cpu.read_data(0x023), 0);
        assert_eq!(cpu.fsr[0], 0x023);
    }

    #[test]
    fn postdec_and_preinc_move_before_or_after_the_access() {
        let mut cpu = Pic18::new();
        cpu.fsr[0] = 0x021;
        cpu.write_data(0x021, 0x44);
        cpu.load_words(&[
            0x50ED, // MOVF POSTDEC0,W -> W=0x44, FSR=0x20
            0x0E55, // MOVLW 0x55
            0x6EED, // MOVWF POSTDEC0 -> [0x20]=0x55, FSR=0x1F
        ]);
        run(&mut cpu, 3);
        assert_eq!(cpu.wreg(), 0x55);
        assert_eq!(cpu.read_data(0x021), 0x44);
        assert_eq!(cpu.read_data(0x020), 0x55);
        assert_eq!(cpu.fsr[0], 0x01F);

        let mut cpu = Pic18::new();
        cpu.fsr[0] = 0x01F;
        cpu.write_data(0x020, 0x66);
        cpu.load_words(&[
            0x50EC, // MOVF PREINC0,W -> FSR=0x20, W=0x66
            0x0E77, // MOVLW 0x77
            0x6EEC, // MOVWF PREINC0 -> FSR=0x21, [0x21]=0x77
        ]);
        run(&mut cpu, 3);
        assert_eq!(cpu.wreg(), 0x77);
        assert_eq!(cpu.read_data(0x020), 0x66);
        assert_eq!(cpu.read_data(0x021), 0x77);
        assert_eq!(cpu.fsr[0], 0x021);
    }

    #[test]
    fn plusw_uses_a_signed_wreg_offset_without_touching_fsr() {
        let mut cpu = Pic18::new();
        cpu.fsr[0] = 0x030;
        cpu.wreg = 0xF0;
        cpu.write_data(0x020, 0x01);
        cpu.load_words(&[0x26EB]); // ADDWF PLUSW0,F -> [0x20] = 0x01 + (-16)
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x020), 0xF1);
        assert_eq!(cpu.read_data(0x030), 0);
        assert_eq!(cpu.wreg(), 0xF0);
        assert_eq!(cpu.fsr[0], 0x030);

        let mut cpu = Pic18::new();
        cpu.fsr[0] = 0x030;
        cpu.wreg = 0x10;
        cpu.load_words(&[0x6EEB]); // MOVWF PLUSW0 -> [0x40] = 0x10
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x040), 0x10);
        assert_eq!(cpu.read_data(0x030), 0);
        assert_eq!(cpu.wreg(), 0x10);
        assert_eq!(cpu.fsr[0], 0x030);
    }

    #[test]
    fn postinc_aliases_for_fsr1_and_fsr2_update_their_own_pairs() {
        let mut cpu = Pic18::new();
        cpu.fsr = [0x020, 0x120, 0x220];
        cpu.write_data(0x120, 0x12);
        cpu.write_data(0x220, 0x22);
        cpu.load_words(&[
            0x50E6, // MOVF POSTINC1,W -> W=0x12, FSR1=0x121
            0x50DE, // MOVF POSTINC2,W -> W=0x22, FSR2=0x221
        ]);
        run(&mut cpu, 2);
        assert_eq!(cpu.wreg(), 0x22);
        assert_eq!(cpu.fsr, [0x020, 0x121, 0x221]);
    }

    #[test]
    fn bit_operations_resolve_the_same_indirect_aliases() {
        let mut cpu = Pic18::new();
        cpu.fsr[0] = 0x020;
        cpu.load_words(&[
            0x86EF, // BSF INDF0,3
            0x96EF, // BCF INDF0,3
            0x86EF, // BSF INDF0,3 again
        ]);
        run(&mut cpu, 3);
        assert_eq!(cpu.read_data(0x020), 0x08);
        assert_eq!(cpu.fsr[0], 0x020);
    }

    #[test]
    fn indirect_reads_and_writes_of_virtual_operands_are_zero_and_nop() {
        let aliases = [
            SFR_PLUSW2,
            SFR_PREINC2,
            SFR_POSTDEC2,
            SFR_POSTINC2,
            SFR_INDF2,
            SFR_PLUSW1,
            SFR_PREINC1,
            SFR_POSTDEC1,
            SFR_POSTINC1,
            SFR_INDF1,
            SFR_PLUSW0,
            SFR_PREINC0,
            SFR_POSTDEC0,
            SFR_POSTINC0,
            SFR_INDF0,
        ];
        for alias in aliases {
            let mut cpu = Pic18::new();
            cpu.write_data(alias, 0x5A);
            cpu.fsr[0] = alias;
            cpu.load_words(&[0x50EF]); // MOVF INDF0,W
            run(&mut cpu, 1);
            assert_eq!(cpu.wreg(), 0, "indirect read of {alias:#x} must be 0");
            assert_eq!(
                cpu.read_data(alias),
                0x5A,
                "raw slot for virtual alias {alias:#x} must not be read"
            );
            assert_eq!(cpu.fsr[0], alias);

            let mut cpu = Pic18::new();
            cpu.write_data(alias, 0x5A);
            cpu.fsr[0] = alias;
            cpu.wreg = 0x99;
            cpu.load_words(&[0x6EEF]); // MOVWF INDF0
            run(&mut cpu, 1);
            assert_eq!(
                cpu.read_data(alias),
                0x5A,
                "indirect write of {alias:#x} must be a NOP"
            );
            assert_eq!(cpu.fsr[0], alias);
        }

        // A rejected write also drops the selected operand's post-action.
        let mut cpu = Pic18::new();
        cpu.write_data(SFR_INDF1, 0x5A);
        cpu.fsr[0] = SFR_INDF1;
        cpu.wreg = 0x99;
        cpu.load_words(&[0x6EEE]); // MOVWF POSTINC0
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(SFR_INDF1), 0x5A);
        assert_eq!(cpu.fsr[0], SFR_INDF1);
    }

    #[test]
    fn indirect_write_to_an_fsr_pair_skips_the_post_action() {
        let mut cpu = Pic18::new();
        cpu.fsr[0] = SFR_FSR1L;
        cpu.wreg = 0x34;
        cpu.load_words(&[0x6EEE]); // MOVWF POSTINC0 -> FSR1L, no post-increment
        run(&mut cpu, 1);
        assert_eq!(cpu.fsr[0], SFR_FSR1L);
        assert_eq!(cpu.fsr[1], 0x0034);

        // Self-targeting POSTDEC2 writes FSR2L but does not decrement FSR2.
        let mut cpu = Pic18::new();
        cpu.fsr[2] = SFR_FSR2L;
        cpu.wreg = 0x56;
        cpu.load_words(&[0x6EDD]); // MOVWF POSTDEC2
        run(&mut cpu, 1);
        assert_eq!(cpu.fsr[2], 0x0F56);

        // INDF has no post-action, so writing an FSR pair is unchanged.
        let mut cpu = Pic18::new();
        cpu.fsr[0] = SFR_FSR1H;
        cpu.wreg = 0x77;
        cpu.load_words(&[0x6EEF]); // MOVWF INDF0
        run(&mut cpu, 1);
        assert_eq!(cpu.fsr[0], SFR_FSR1H);
        assert_eq!(cpu.fsr[1], 0x7700);
    }

    #[test]
    fn indirect_pointers_wrap_in_the_12_bit_data_space() {
        let mut cpu = Pic18::new();
        cpu.fsr[0] = 0x0FFF;
        cpu.load_words(&[0x50EE]); // MOVF POSTINC0,W
        run(&mut cpu, 1);
        assert_eq!(cpu.fsr[0], 0);
    }

    #[test]
    fn addwf_sets_exact_c_dc_z_ov_n() {
        let mut cpu = Pic18::new();
        cpu.write_data(0x020, 0x7F);
        cpu.load_words(&[
            0x0E01, // MOVLW 1
            0x2620, // ADDWF 0x20, d=1, a=0
        ]);
        run(&mut cpu, 2);
        assert_eq!(cpu.read_data(0x020), 0x80);
        assert_eq!(
            cpu.status() & STATUS_FLAG_MASK,
            STATUS_DC | STATUS_OV | STATUS_N
        );

        let mut cpu = Pic18::new();
        cpu.write_data(0x020, 0xFF);
        cpu.load_words(&[0x0E01, 0x2620]);
        run(&mut cpu, 2);
        assert_eq!(cpu.read_data(0x020), 0);
        assert_eq!(
            cpu.status() & STATUS_FLAG_MASK,
            STATUS_C | STATUS_DC | STATUS_Z
        );
    }

    #[test]
    fn incf_and_decf_set_borrow_and_overflow_flags() {
        let mut cpu = Pic18::new();
        cpu.write_data(0x020, 0x7F);
        cpu.load_words(&[0x2A20]); // INCF 0x20, d=1, a=0
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x020), 0x80);
        assert_eq!(
            cpu.status() & STATUS_FLAG_MASK,
            STATUS_DC | STATUS_OV | STATUS_N
        );

        let mut cpu = Pic18::new();
        cpu.write_data(0x020, 0x00);
        cpu.load_words(&[0x0620]); // DECF 0x20, d=1, a=0
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x020), 0xFF);
        assert_eq!(cpu.status() & STATUS_FLAG_MASK, STATUS_N);

        let mut cpu = Pic18::new();
        cpu.write_data(0x020, 0x80);
        cpu.load_words(&[0x0620]);
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(0x020), 0x7F);
        assert_eq!(cpu.status() & STATUS_FLAG_MASK, STATUS_C | STATUS_OV);
    }

    #[test]
    fn logic_ops_set_only_z_and_n() {
        let mut cpu = Pic18::new();
        cpu.write_data(0x020, 0xF0);
        cpu.load_words(&[
            0x0E0F, // MOVLW 0x0F
            0x1620, // ANDWF 0x20, d=1, a=0
        ]);
        run(&mut cpu, 2);
        assert_eq!(cpu.read_data(0x020), 0);
        assert_eq!(cpu.status() & STATUS_FLAG_MASK, STATUS_Z);
    }

    #[test]
    fn clrf_and_movf_destination_rules() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[
            0x6A20, // CLRF 0x20, a=0
            0x0E80, // MOVLW 0x80
            0x6E20, // MOVWF 0x20
            0x5020, // MOVF 0x20, d=0, a=0 -> W=0x80, N=1
        ]);
        run(&mut cpu, 4);
        assert_eq!(cpu.read_data(0x020), 0x80);
        assert_eq!(cpu.wreg(), 0x80);
        assert_eq!(cpu.status() & STATUS_FLAG_MASK, STATUS_N);

        // CLRF STATUS sets Z and leaves the other flags alone.
        cpu.status = STATUS_C | STATUS_DC | STATUS_N;
        cpu.load_words(&[0x6AD8]); // CLRF STATUS, a=0
        cpu.set_pc(0);
        run(&mut cpu, 1);
        assert_eq!(
            cpu.status() & STATUS_FLAG_MASK,
            STATUS_C | STATUS_DC | STATUS_Z | STATUS_N
        );
    }

    #[test]
    fn movff_uses_absolute_address_and_ignores_bsr() {
        let mut cpu = Pic18::new();
        cpu.write_data(0x123, 0x5A);
        cpu.load_words(&[
            0x0102, // MOVLB 2
            0xC123, // MOVFF 0x123 -> 0x456
            0xF456,
        ]);
        run(&mut cpu, 3);
        assert_eq!(cpu.read_data(0x456), 0x5A);
        assert_eq!(cpu.bsr(), 2);
    }

    #[test]
    fn two_word_goto_and_call_return() {
        let mut cpu = Pic18::new();
        // CALL 0x0010 (word address) = byte 0x20.
        cpu.load_flash(0x00, &[0x10, 0xEC, 0x00, 0xF0]);
        cpu.load_flash(0x04, &[0xAA, 0x0E]);
        cpu.load_flash(0x20, &[0x55, 0x0E, 0x12, 0x00]);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x20);
        assert_eq!(cpu.stack_pointer(), 1);
        run(&mut cpu, 1);
        assert_eq!(cpu.wreg(), 0x55);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x04);
        assert_eq!(cpu.stack_pointer(), 0);

        let mut cpu = Pic18::new();
        cpu.load_words(&[0xEF03, 0xF000]); // GOTO word 3 = byte 0x6
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x0006);
    }

    #[test]
    fn lfsr_loads_all_12_bits() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[0xEE12, 0xF03F]); // LFSR 1, 0x23F
        run(&mut cpu, 1);
        assert_eq!(cpu.fsr[1], 0x23F);
        assert_eq!(cpu.get_pc(), 4);
    }

    #[test]
    fn conditional_branch_and_skip_over_two_word_instruction() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[
            0x6A20, // CLRF 0x20 -> Z=1
            0xE001, // BZ +1 (skip MOVLW 0xEE)
            0x0EEE, 0x0E77, // W=0x77
        ]);
        run(&mut cpu, 4);
        assert_eq!(cpu.wreg(), 0x77);

        let mut cpu = Pic18::new();
        cpu.load_words(&[
            0x8000, // BSF 0x00, bit 0 (access address 0x000)
            0x9000, // BCF 0x00, bit 0
            0xB000, // BTFSC 0x00, bit 0 -> next instruction (GOTO) is 2 words
            0xEF06, // GOTO word 6 = byte 0xC
            0xF000, 0x0000, 0x0E77, // W=0x77
        ]);
        run(&mut cpu, 5);
        assert_eq!(cpu.wreg(), 0x77);
        assert_eq!(cpu.get_pc(), 0x0E);
    }

    #[test]
    fn btg_and_btfss_share_the_bit_file_resolver() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[
            0x0E00, // MOVLW 0
            0x6E20, // MOVWF 0x20
            0x7220, // BTG 0x20, bit 1 -> 0x02
            0xA220, // BTFSS 0x20, bit 1 -> skip because the bit is set
            0x0EAA, // skipped
            0x0E55, // MOVLW 0x55
        ]);
        run(&mut cpu, 5);
        assert_eq!(cpu.read_data(0x020), 0x02);
        assert_eq!(cpu.wreg(), 0x55);
    }

    #[test]
    fn rcall_pushes_the_following_address_and_retlw_pops_it() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[
            0xD801, // RCALL +1 word: next=2, target=4
            0x0E11, // return target: MOVLW 0x11
            0x0C42, // RETLW 0x42
        ]);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 4);
        assert_eq!(cpu.stack_pointer(), 1);
        assert_eq!(cpu.top_of_stack(), 2);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 2);
        assert_eq!(cpu.wreg(), 0x42);
        assert_eq!(cpu.stack_pointer(), 0);
        run(&mut cpu, 1);
        assert_eq!(cpu.wreg(), 0x11);
        assert_eq!(cpu.get_pc(), 4);
    }

    #[test]
    fn movff_to_pcl_combines_the_latches_and_overrides_the_advance() {
        let mut cpu = Pic18::new();
        cpu.write_data(0x010, 0x40);
        cpu.write_data(SFR_PCLATH, 0x01);
        cpu.load_words(&[
            0xC010, // MOVFF 0x010 -> PCL
            0xFFF9, // destination 0xFF9 (second word 1111:1111:1111:1001)
        ]);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x0140);
    }

    #[test]
    fn stack_overflow_overwrites_top_and_underflow_is_sticky() {
        let mut cpu = Pic18::with_geometry(64, 2);
        cpu.push(0x10);
        cpu.push(0x20);
        cpu.push(0x30);
        assert_eq!(cpu.stack_pointer(), 2);
        assert!(cpu.stk_full);
        assert_eq!(cpu.top_of_stack(), 0x30);
        assert_eq!(cpu.pop(), 0x30);
        assert_eq!(cpu.pop(), 0x10);
        assert_eq!(cpu.pop(), 0);
        assert!(cpu.stk_unf);
        assert_eq!(cpu.stack_pointer(), 0);
    }

    #[test]
    fn pcl_write_uses_latches_and_aligns() {
        let mut cpu = Pic18::new();
        cpu.write_data(SFR_PCLATH, 0x01);
        cpu.wreg = 0xFF;
        cpu.load_words(&[0x6EF9]); // MOVWF PCL, a=0 (resolves to 0xFF9)
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x01FE);
    }

    #[test]
    fn addwf_pcl_is_a_computed_goto_from_the_instruction_address() {
        let mut cpu = Pic18::new();
        cpu.wreg = 4;
        cpu.load_words(&[
            0x26F9, // ADDWF PCL, f, access: (PCL = 0) + 4 -> byte 4
            0x0000, // must be skipped
            0x0E11, // MOVLW 0x11 at byte 4
        ]);
        run(&mut cpu, 2);
        assert_eq!(cpu.get_pc(), 6);
        assert_eq!(cpu.wreg(), 0x11);
        // PCL low-byte addition must not carry into PCLATH.
        let mut cpu = Pic18::new();
        cpu.wreg = 4;
        cpu.set_pc(0x10FE);
        cpu.load_flash(0x10FE, &[0xF9, 0x26, 0x00, 0x00]);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x1002);
        assert_eq!(cpu.pclath, 0x10);
    }

    #[test]
    fn unknown_opcode_is_a_decode_error() {
        // 0x0001/0x0002 are unassigned in the standard set; the extended
        // (XINST) words such as ADDFSR (0xE8xx) are not decoded either.
        for word in [0x0001, 0x0002, 0xE800] {
            let mut cpu = Pic18::new();
            cpu.load_words(&[word]);
            let err = run_err(&mut cpu, 1);
            assert!(
                matches!(err, SimulationError::DecodeError(0)),
                "{word:#06x}"
            );
        }
    }

    /// Run one instruction from a fresh core with `w`, `c` and `[0x20]`
    /// seeded; returns the core for inspection.
    fn exec_one(word: u16, w: u8, c: bool, f: u8) -> Pic18 {
        let mut cpu = Pic18::new();
        cpu.wreg = w;
        cpu.status = if c { STATUS_C } else { 0 };
        cpu.data[0x20] = f;
        cpu.load_words(&[word]);
        run(&mut cpu, 1);
        cpu
    }

    #[test]
    fn literal_alu_ops_match_the_datasheet_examples() {
        // ADDLW 0x15, W = 0x10 -> 0x25.
        assert_eq!(exec_one(0x0F15, 0x10, false, 0).wreg(), 0x25);
        // SUBLW 0x02: W = 1 -> 1 (C); W = 2 -> 0 (C, Z); W = 3 -> 0xFF (N, !C).
        let cpu = exec_one(0x0802, 0x01, false, 0);
        assert_eq!(
            (cpu.wreg(), cpu.status() & (STATUS_C | STATUS_Z)),
            (0x01, STATUS_C)
        );
        let cpu = exec_one(0x0802, 0x02, false, 0);
        assert_eq!(cpu.wreg(), 0);
        assert_eq!(cpu.status() & (STATUS_C | STATUS_Z), STATUS_C | STATUS_Z);
        let cpu = exec_one(0x0802, 0x03, false, 0);
        assert_eq!(cpu.wreg(), 0xFF);
        assert_eq!(cpu.status() & (STATUS_C | STATUS_N), STATUS_N);
        // ANDLW 0x5F, W = 0xA3 -> 0x03; IORLW 0x35, W = 0x9A -> 0xBF;
        // XORLW 0xAF, W = 0xB5 -> 0x1A.
        assert_eq!(exec_one(0x0B5F, 0xA3, false, 0).wreg(), 0x03);
        let cpu = exec_one(0x0935, 0x9A, false, 0);
        assert_eq!((cpu.wreg(), cpu.status() & STATUS_N), (0xBF, STATUS_N));
        assert_eq!(exec_one(0x0AAF, 0xB5, false, 0).wreg(), 0x1A);
        // MULLW 0xC4, W = 0xE2 -> PRODH:PRODL = 0xAD08, W and STATUS untouched.
        let cpu = exec_one(0x0DC4, 0xE2, true, 0);
        assert_eq!(
            (cpu.prod, cpu.wreg(), cpu.status()),
            (0xAD08, 0xE2, STATUS_C)
        );
    }

    #[test]
    fn carry_and_borrow_arithmetic() {
        // SUBWF 0x20,F: 3 - 2 = 1, no borrow.
        let cpu = exec_one(0x5E20, 0x02, false, 0x03);
        assert_eq!(cpu.data[0x20], 0x01);
        assert_eq!(cpu.status() & STATUS_C, STATUS_C);
        // SUBWFB 0x20,F: 0x19 - 0x0D - !C(=0) = 0x0C.
        let cpu = exec_one(0x5A20, 0x0D, true, 0x19);
        assert_eq!(cpu.data[0x20], 0x0C);
        assert_eq!(cpu.status() & (STATUS_C | STATUS_Z), STATUS_C);
        // SUBWFB with C = 0 borrows one more: 0x19 - 0x0D - 1 = 0x0B.
        assert_eq!(exec_one(0x5A20, 0x0D, false, 0x19).data[0x20], 0x0B);
        // SUBFWB 0x20,W: W - f - !C = 2 - 3 - 0 = 0xFF, borrow, negative.
        let cpu = exec_one(0x5420, 0x02, true, 0x03);
        assert_eq!(cpu.wreg(), 0xFF);
        assert_eq!(cpu.status() & (STATUS_C | STATUS_N), STATUS_N);
        assert_eq!(cpu.data[0x20], 0x03);
        // ADDWFC 0x20,W: 0x02 + 0x4D + C = 0x50, half carry.
        let cpu = exec_one(0x2020, 0x4D, true, 0x02);
        assert_eq!(cpu.wreg(), 0x50);
        assert_eq!(cpu.status() & (STATUS_C | STATUS_DC), STATUS_DC);
        // ADDWFC carries out: 0xFF + 0x00 + 1 = 0x00 with C and Z.
        let cpu = exec_one(0x2020, 0x00, true, 0xFF);
        assert_eq!(cpu.wreg(), 0);
        assert_eq!(cpu.status() & (STATUS_C | STATUS_Z), STATUS_C | STATUS_Z);
        // NEGF 0x20: 0b0011_1010 -> 0b1100_0110.
        let cpu = exec_one(0x6C20, 0, false, 0x3A);
        assert_eq!(cpu.data[0x20], 0xC6);
        assert_eq!(cpu.status() & (STATUS_C | STATUS_N), STATUS_N);
        // MULWF 0x20: W = 0xC4, f = 0xB5 -> 0x8A94; f and W untouched.
        let cpu = exec_one(0x0220, 0xC4, false, 0xB5);
        assert_eq!((cpu.prod, cpu.wreg(), cpu.data[0x20]), (0x8A94, 0xC4, 0xB5));
    }

    #[test]
    fn rotate_swap_complement_and_set() {
        // RLCF 0x20,W: 0b1110_0110, C = 0 -> W = 0b1100_1100, C = 1.
        let cpu = exec_one(0x3420, 0, false, 0xE6);
        assert_eq!(cpu.wreg(), 0xCC);
        assert_eq!(cpu.status() & STATUS_C, STATUS_C);
        assert_eq!(cpu.data[0x20], 0xE6);
        // RRCF 0x20,W: 0b1110_0110, C = 0 -> 0b0111_0011, C = 0.
        let cpu = exec_one(0x3020, 0, false, 0xE6);
        assert_eq!((cpu.wreg(), cpu.status() & STATUS_C), (0x73, 0));
        // RRCF shifts C into bit 7.
        assert_eq!(exec_one(0x3020, 0, true, 0x02).wreg(), 0x81);
        // RLNCF 0b1010_1011 -> 0b0101_0111; RRNCF 0b1101_0111 -> 0b1110_1011;
        // neither touches C.
        let cpu = exec_one(0x4420, 0, true, 0xAB);
        assert_eq!((cpu.wreg(), cpu.status() & STATUS_C), (0x57, STATUS_C));
        assert_eq!(exec_one(0x4020, 0, false, 0xD7).wreg(), 0xEB);
        // SWAPF 0x20,F: 0x53 -> 0x35, no flags.
        let cpu = exec_one(0x3A20, 0, false, 0x53);
        assert_eq!((cpu.data[0x20], cpu.status()), (0x35, 0));
        // COMF 0x20,W: 0x13 -> 0xEC.
        let cpu = exec_one(0x1C20, 0, false, 0x13);
        assert_eq!((cpu.wreg(), cpu.status() & STATUS_N), (0xEC, STATUS_N));
        // SETF 0x20 -> 0xFF, no flags.
        let cpu = exec_one(0x6820, 0, false, 0x5A);
        assert_eq!((cpu.data[0x20], cpu.status()), (0xFF, 0));
    }

    #[test]
    fn skip_on_result_and_compare_forms() {
        // Each case: (opcode, W, f) -> skips?  The next word is NOP, then a
        // second NOP, so the PC tells whether the skip happened.
        let cases = [
            (0x2E20, 0, 0x01, true),  // DECFSZ: 1 -> 0, skip
            (0x2E20, 0, 0x02, false), // DECFSZ: 2 -> 1
            (0x4E20, 0, 0x02, true),  // DCFSNZ: 2 -> 1, skip
            (0x4E20, 0, 0x01, false), // DCFSNZ: 1 -> 0
            (0x3E20, 0, 0xFF, true),  // INCFSZ: 0xFF -> 0, skip
            (0x3E20, 0, 0x00, false), // INCFSZ: 0 -> 1
            (0x4A20, 0, 0x00, true),  // INFSNZ: 0 -> 1, skip
            (0x4A20, 0, 0xFF, false), // INFSNZ: 0xFF -> 0
            (0x6220, 7, 7, true),     // CPFSEQ
            (0x6220, 7, 8, false),
            (0x6420, 7, 8, true), // CPFSGT (unsigned)
            (0x6420, 0x80, 0x7F, false),
            (0x6020, 0x80, 0x7F, true), // CPFSLT (unsigned)
            (0x6020, 7, 7, false),
            (0x6620, 0, 0, true), // TSTFSZ
            (0x6620, 0, 1, false),
        ];
        for (word, w, f, skips) in cases {
            let mut cpu = Pic18::new();
            cpu.wreg = w;
            cpu.status = STATUS_C;
            cpu.data[0x20] = f;
            cpu.load_words(&[word, 0x0000, 0x0000]);
            run(&mut cpu, 1);
            let expected = if skips { 4 } else { 2 };
            assert_eq!(cpu.get_pc(), expected, "{word:#06x} W={w:#x} f={f:#x}");
            // None of these forms touch STATUS.
            assert_eq!(cpu.status(), STATUS_C, "{word:#06x}");
        }
        // The skip steps over a whole two-word instruction.
        let mut cpu = Pic18::new();
        cpu.data[0x20] = 1;
        cpu.load_words(&[0x2E20, 0xEF00, 0xF000, 0x0000]); // DECFSZ; GOTO 0
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 6);
        assert_eq!(cpu.data[0x20], 0);
    }

    #[test]
    fn daw_push_pop_and_reset() {
        // DAW: W = 0xA5 -> 0x05, C; W = 0xCE -> 0x34, C (DS39631 examples).
        let cpu = exec_one(0x0007, 0xA5, false, 0);
        assert_eq!((cpu.wreg(), cpu.status() & STATUS_C), (0x05, STATUS_C));
        let cpu = exec_one(0x0007, 0xCE, false, 0);
        assert_eq!((cpu.wreg(), cpu.status() & STATUS_C), (0x34, STATUS_C));
        // A valid BCD value is left alone.
        assert_eq!(exec_one(0x0007, 0x42, false, 0).wreg(), 0x42);

        // PUSH stacks the next instruction; POP discards the top of stack.
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0005, 0x0005, 0x0006]);
        run(&mut cpu, 2);
        assert_eq!(cpu.stack_pointer(), 2);
        assert_eq!(cpu.top_of_stack(), 4);
        run(&mut cpu, 1);
        assert_eq!(cpu.stack_pointer(), 1);
        assert_eq!(cpu.top_of_stack(), 2);
        // POP on an empty stack flags underflow.
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0006]);
        run(&mut cpu, 1);
        assert_ne!(cpu.read_data(SFR_STKPTR) & STKPTR_STKUNF, 0);

        // RESET returns to address 0 with reset SFR values but keeps flash.
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0E42, 0x6E20, 0x0E00, 0x6E93, 0x00FF]);
        run(&mut cpu, 5);
        assert_eq!(cpu.get_pc(), 0);
        assert_eq!(cpu.wreg(), 0);
        assert_eq!(cpu.read_data(0x20), 0);
        assert_eq!(cpu.read_data(SFR_TRISA + 1), 0xFF);
        assert_eq!(cpu.fetch_word(0), 0x0E42);
    }

    #[test]
    fn table_reads_walk_flash_and_the_configuration_window() {
        let mut cpu = Pic18::with_geometry(32 * 1024, PIC18_STACK_DEPTH);
        cpu.load_flash(0x0100, &[0x11, 0x22, 0x33]);
        cpu.load_flash(PIC18_CONFIG_BASE + 1, &[0x02]); // CONFIG1H
        cpu.tblptr = 0x0100;
        cpu.load_words(&[0x0009, 0x0009, 0x000A, 0x000B, 0x0008]);
        run(&mut cpu, 2); // TBLRD*+ twice
        assert_eq!((cpu.tablat, cpu.tblptr), (0x22, 0x0102));
        run(&mut cpu, 1); // TBLRD*-
        assert_eq!((cpu.tablat, cpu.tblptr), (0x33, 0x0101));
        run(&mut cpu, 1); // TBLRD+*
        assert_eq!((cpu.tablat, cpu.tblptr), (0x33, 0x0102));
        assert_eq!(cpu.clock_cycles(), 8);

        // Programmed config byte, erased config byte, unimplemented memory.
        for (addr, expected) in [
            (PIC18_CONFIG_BASE + 1, 0x02),
            (PIC18_CONFIG_BASE + 2, 0xFF),
            (0x8000, 0x00),
            (0x3F_FFFE, 0x00),
        ] {
            cpu.tblptr = addr;
            cpu.set_pc(8);
            run(&mut cpu, 1); // TBLRD*
            assert_eq!(cpu.tablat, expected, "{addr:#x}");
        }
        // TBLWT moves the pointer but never reprograms flash.
        let mut cpu = Pic18::new();
        cpu.tblptr = 0x0100;
        cpu.tablat = 0x55;
        cpu.load_words(&[0x000D]);
        run(&mut cpu, 1);
        assert_eq!(cpu.tblptr, 0x0101);
        assert_eq!(cpu.program_byte(0x0100), 0xFF);
    }

    #[test]
    fn sleep_idles_until_an_enabled_source_and_sets_pd() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0003, 0x0E42]); // SLEEP; MOVLW 0x42
        run(&mut cpu, 1);
        assert!(cpu.is_asleep());
        assert_eq!(cpu.read_data(SFR_RCON) & (RCON_TO | RCON_PD), RCON_TO);
        run(&mut cpu, 10);
        assert!(cpu.is_asleep());
        assert_eq!(cpu.wreg(), 0);
        // A flagged but disabled source does not wake the core.
        cpu.write_data(SFR_PIR1, 0x01);
        run(&mut cpu, 1);
        assert!(cpu.is_asleep());
        // Enabling it wakes the core even with GIE clear; execution resumes
        // after SLEEP without vectoring.
        cpu.write_data(SFR_PIE1, 0x01);
        run(&mut cpu, 2);
        assert!(!cpu.is_asleep());
        assert_eq!(cpu.wreg(), 0x42);
        assert_eq!(cpu.stack_pointer(), 0);
        // CLRWDT sets TO and PD again.
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0003]);
        run(&mut cpu, 1);
        cpu.load_words(&[0x0004]);
        cpu.asleep = false;
        cpu.set_pc(0);
        run(&mut cpu, 1);
        assert_eq!(
            cpu.read_data(SFR_RCON) & (RCON_TO | RCON_PD),
            RCON_TO | RCON_PD
        );
    }

    #[test]
    fn eusart_tx_reaches_the_sink_only_when_enabled() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut cpu = Pic18::new();
        cpu.set_serial_sink(sink.clone());
        // Power-on: TRMT set, TXIF clear (transmitter off).
        assert_eq!(cpu.read_data(SFR_TXSTA), TXSTA_TRMT);
        assert_eq!(cpu.read_data(SFR_PIR1) & PIR1_TXIF, 0);
        cpu.write_data(SFR_TXREG, b'x'); // disabled: dropped
        cpu.write_data(SFR_TXSTA, TXSTA_TXEN); // TRMT stays set
        assert_eq!(cpu.read_data(SFR_TXSTA), TXSTA_TXEN | TXSTA_TRMT);
        assert_ne!(cpu.read_data(SFR_PIR1) & PIR1_TXIF, 0);
        cpu.write_data(SFR_TXREG, b'y'); // SPEN still clear: dropped
        cpu.write_data(SFR_RCSTA, RCSTA_SPEN);
        cpu.write_data(SFR_SPBRG, 25);
        // Firmware writes TXREG through MOVWF like real code.
        cpu.load_words(&[0x0E4F, 0x6EAD, 0x0E4B, 0x6EAD]); // "OK"
        run(&mut cpu, 4);
        assert_eq!(sink.lock().unwrap().as_slice(), b"OK");
        // TXIF cannot be cleared by firmware while TXREG is empty.
        cpu.write_data(SFR_PIR1, 0);
        assert_ne!(cpu.read_data(SFR_PIR1) & PIR1_TXIF, 0);
    }

    #[test]
    fn port_reads_follow_tris_and_writes_land_in_the_latch() {
        let mut cpu = Pic18::new();
        let (portb, latb, trisb) = (SFR_PORTA + 1, SFR_LATA + 1, SFR_TRISA + 1);
        assert_eq!(cpu.read_data(trisb), 0xFF);
        assert_eq!(cpu.read_data(SFR_TRISA + 4), 0x07);
        cpu.load_words(&[
            0x0EF0, // MOVLW 0xF0
            0x6E93, // MOVWF TRISB: RB0..RB3 outputs
            0x0EFF, // MOVLW 0xFF
            0x6E81, // MOVWF PORTB -> LATB
            0x8081, // BSF PORTB,0 (RMW reads pins, writes the latch)
        ]);
        run(&mut cpu, 4);
        assert_eq!(cpu.read_data(latb), 0xFF);
        // Only the output pins read back high; inputs are undriven.
        assert_eq!(cpu.read_data(portb), 0x0F);
        // BSF on PORTB reads the pins and writes the latch, so the undriven
        // input bits come back as 0 in LATB: the classic PORT
        // read-modify-write hazard that LATx exists to avoid.
        run(&mut cpu, 1);
        assert_eq!(cpu.read_data(latb), 0x0F);
    }

    #[test]
    fn pic18f4520_access_bank_splits_at_0x80() {
        let mut cpu = Pic18::for_chip_name("PIC18F4520");
        cpu.load_words(&[
            0x0E5A, // MOVLW 0x5A
            0x6E70, // MOVWF 0x70, access -> GPR 0x070 (not SFR 0xF70)
            0x6E81, // MOVWF 0x81, access -> PORTB (0xF81) -> LATB
        ]);
        run(&mut cpu, 3);
        assert_eq!(cpu.data[0x070], 0x5A);
        assert_eq!(cpu.data[0xF70], 0);
        assert_eq!(cpu.read_data(SFR_LATA + 1), 0x5A);
        assert_eq!(cpu.flash.len(), 32 * 1024);
        // The generic core keeps the 0x60 split.
        assert_eq!(Pic18::new().file_address(0x70, false), 0xF70);
    }

    #[test]
    fn movff_expands_indirect_aliases_on_both_operands() {
        // SDCC's pic16 prologue/epilogue: push with POSTDEC1, frame reads
        // with PLUSW2, pop with PREINC1.
        let mut cpu = Pic18::new();
        cpu.fsr[1] = 0x0FF;
        cpu.data[0x010] = 0xAB;
        cpu.data[0x011] = 0xCD;
        cpu.load_words(&[
            0xC010, 0xFFE5, // MOVFF 0x010, POSTDEC1 -> [0xFF]=0xAB, FSR1=0xFE
            0xC011, 0xFFE5, // MOVFF 0x011, POSTDEC1 -> [0xFE]=0xCD, FSR1=0xFD
            0xCFE1, 0xFFD9, // MOVFF FSR1L, FSR2L -> FSR2=0x0FD (FSR2H is 0)
            0x0E02, //         MOVLW 2
            0xCFDB, 0xF020, // MOVFF PLUSW2, 0x020 -> [0x0FF]=0xAB
            0xCFE4, 0xF021, // MOVFF PREINC1, 0x021 -> FSR1=0xFE, 0xCD
            0xCFE4, 0xF022, // MOVFF PREINC1, 0x022 -> FSR1=0xFF, 0xAB
        ]);
        run(&mut cpu, 7);
        assert_eq!((cpu.data[0x0FF], cpu.data[0x0FE]), (0xAB, 0xCD));
        assert_eq!(cpu.fsr[2], 0x0FD);
        assert_eq!(
            (cpu.data[0x020], cpu.data[0x021], cpu.data[0x022]),
            (0xAB, 0xCD, 0xAB)
        );
        assert_eq!(cpu.fsr[1], 0x0FF);

        // Indirect-to-indirect: one update per side.
        let mut cpu = Pic18::new();
        cpu.fsr = [0x030, 0x040, 0];
        cpu.data[0x030] = 0x77;
        cpu.load_words(&[0xCFEE, 0xFFE6]); // MOVFF POSTINC0, POSTINC1
        run(&mut cpu, 1);
        assert_eq!(cpu.data[0x040], 0x77);
        assert_eq!((cpu.fsr[0], cpu.fsr[1]), (0x031, 0x041));
    }

    #[test]
    fn hand_assembled_program_totals_a_value() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[
            0x0E05, // MOVLW 5
            0x6E10, // MOVWF 0x10
            0x0E03, // MOVLW 3
            0x2610, // ADDWF 0x10, d=1, a=0 -> 8
            0x5010, // MOVF 0x10, d=0, a=0
            0x6E20, // MOVWF 0x20
            0xEF06, // GOTO byte 0x0C (word 6): halt here
            0xF000,
        ]);
        run(&mut cpu, 7);
        assert_eq!(cpu.read_data(0x010), 8);
        assert_eq!(cpu.read_data(0x020), 8);
        assert_eq!(cpu.wreg(), 8);
        assert_eq!(cpu.get_pc(), 0x0C);
        assert_eq!(cpu.status() & STATUS_FLAG_MASK, 0);
    }

    #[test]
    fn program_image_loads_at_byte_addresses() {
        let mut cpu = Pic18::new();
        let mut image = ProgramImage::new(0, crate::Arch::Pic18);
        image.add_segment(0x0000, vec![0x42, 0x0E]);
        cpu.load_program_image(&image);
        run(&mut cpu, 1);
        assert_eq!(cpu.wreg(), 0x42);
        assert_eq!(cpu.get_pc(), 2);
    }

    #[test]
    fn entry_point_must_be_word_aligned() {
        assert!(check_pic18_entry(0).is_ok());
        assert!(check_pic18_entry(0x1000).is_ok());
        let err = check_pic18_entry(0x1001).unwrap_err();
        assert!(err.contains("aligned"), "{err}");
    }

    #[test]
    fn snapshot_round_trip_keeps_architectural_state() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0E12, 0x6E20]);
        run(&mut cpu, 2);
        cpu.push(0x0004);
        let snap = cpu.snapshot();
        let mut restored = Pic18::new();
        restored.apply_snapshot(&snap);
        assert_eq!(restored.wreg(), 0x12);
        assert_eq!(restored.get_pc(), cpu.get_pc());
        assert_eq!(restored.stack_pointer(), 1);
        assert_eq!(restored.top_of_stack(), 0x0004);
    }

    #[test]
    fn interrupt_without_ipen_vectors_high_and_retfie_restores_gie() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0000]); // NOP at 0
        cpu.load_flash(PIC18_HIGH_VECTOR, &0x0010u16.to_le_bytes()); // RETFIE

        // IPEN=0: a PIR1 source needs GIE and PEIE (INTCON<6>).
        cpu.write_data(SFR_INTCON, INTCON_GIE_GIEH | INTCON_GIEL);
        cpu.write_data(SFR_PIE1, 0x01);
        cpu.write_data(SFR_PIR1, 0x01);

        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), PIC18_HIGH_VECTOR);
        assert_eq!(cpu.stack_pointer(), 1);
        assert_eq!(cpu.top_of_stack(), 0x0002);
        assert_eq!(
            cpu.read_data(SFR_INTCON) & INTCON_GIE_GIEH,
            0,
            "GIE clears on entry"
        );

        cpu.write_data(SFR_PIR1, 0); // ISR acknowledges the source
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x0002);
        assert_ne!(
            cpu.read_data(SFR_INTCON) & INTCON_GIE_GIEH,
            0,
            "RETFIE re-enables GIE"
        );
        assert_eq!(cpu.stack_pointer(), 0);
    }

    #[test]
    fn interrupt_priority_selects_low_and_high_vectors() {
        // Low-priority PIR1 bit 0 (IPR1 bit 0 clear): vector 0x0018, GIEL clears
        // and GIEH stays set.
        let mut low = Pic18::new();
        low.load_words(&[0x0000]);
        low.write_data(SFR_RCON, RCON_IPEN);
        low.write_data(SFR_INTCON, INTCON_GIE_GIEH | INTCON_GIEL);
        low.write_data(SFR_PIE1, 0x01);
        low.write_data(SFR_PIR1, 0x01);
        low.write_data(SFR_IPR1, 0x00);
        run(&mut low, 1);
        assert_eq!(low.get_pc(), PIC18_LOW_VECTOR);
        assert_eq!(
            low.read_data(SFR_INTCON) & INTCON_GIE_GIEH,
            INTCON_GIE_GIEH,
            "GIEH survives a low-priority entry"
        );
        assert_eq!(
            low.read_data(SFR_INTCON) & INTCON_GIEL,
            0,
            "GIEL clears on low-priority entry"
        );

        // High-priority PIR1 bit 0 (IPR1 bit 0 set): vector 0x0008, GIEH clears
        // and GIEL stays set.
        let mut high = Pic18::new();
        high.load_words(&[0x0000]);
        high.write_data(SFR_RCON, RCON_IPEN);
        high.write_data(SFR_INTCON, INTCON_GIE_GIEH | INTCON_GIEL);
        high.write_data(SFR_PIE1, 0x01);
        high.write_data(SFR_PIR1, 0x01);
        high.write_data(SFR_IPR1, 0x01);
        run(&mut high, 1);
        assert_eq!(high.get_pc(), PIC18_HIGH_VECTOR);
        assert_eq!(
            high.read_data(SFR_INTCON) & INTCON_GIE_GIEH,
            0,
            "GIEH clears on high-priority entry"
        );
        assert_eq!(
            high.read_data(SFR_INTCON) & INTCON_GIEL,
            INTCON_GIEL,
            "GIEL survives a high-priority entry"
        );
    }

    #[test]
    fn interrupt_priority_sfrs_match_ds39631_map_and_reset() {
        // DS39631 Table 5-1 puts IPR1 at 0xF9F, above PIR1/PIE1; 0xF9C is
        // unimplemented. Every priority bit powers up set (high priority).
        assert_eq!(SFR_IPR1, 0xF9F);
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0000]);
        assert_eq!(cpu.read_data(SFR_IPR1), 0xFF);
        assert_eq!(cpu.read_data(SFR_IPR2), 0xDF);
        assert_eq!(cpu.read_data(SFR_INTCON2), 0xF5);
        assert_eq!(cpu.read_data(SFR_INTCON3), INTCON3_INT2IP | INTCON3_INT1IP);
        // With IPR1 untouched, an enabled PIR1 source takes the high vector.
        cpu.write_data(SFR_RCON, RCON_IPEN);
        cpu.write_data(SFR_INTCON, INTCON_GIE_GIEH | INTCON_GIEL);
        cpu.write_data(SFR_PIE1, 0x01);
        cpu.write_data(SFR_PIR1, 0x01);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), PIC18_HIGH_VECTOR);
    }

    #[test]
    fn interrupt_high_priority_is_gated_by_gieh() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0000]);
        cpu.write_data(SFR_RCON, RCON_IPEN);
        // Only the low-priority enable is set.
        cpu.write_data(SFR_INTCON, INTCON_GIEL);
        cpu.write_data(SFR_PIE1, 0x01);
        cpu.write_data(SFR_PIR1, 0x01);
        cpu.write_data(SFR_IPR1, 0x01); // high-priority source
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x0002, "GIEH=0 masks a high source");
    }

    #[test]
    fn interrupt_intcon_and_intcon3_sources_are_recognised() {
        // TMR0: INTCON TMR0IF/TMR0IE, low priority on INTCON2.TMR0IP=0.
        let mut tmr0 = Pic18::new();
        tmr0.load_words(&[0x0000]);
        tmr0.write_data(SFR_RCON, RCON_IPEN);
        tmr0.write_data(SFR_INTCON, INTCON_GIE_GIEH | INTCON_GIEL);
        tmr0.write_data(SFR_INTCON2, 0);
        tmr0.write_data(
            SFR_INTCON,
            INTCON_GIE_GIEH | INTCON_GIEL | INTCON_TMR0IE | INTCON_TMR0IF,
        );
        run(&mut tmr0, 1);
        assert_eq!(tmr0.get_pc(), PIC18_LOW_VECTOR);

        // INT1: INTCON3 INT1IF/INT1IE, high priority on INTCON3.INT1IP=1.
        let mut int1 = Pic18::new();
        int1.load_words(&[0x0000]);
        int1.write_data(SFR_RCON, RCON_IPEN);
        int1.write_data(SFR_INTCON, INTCON_GIE_GIEH | INTCON_GIEL);
        int1.write_data(
            SFR_INTCON3,
            INTCON3_INT1IE | INTCON3_INT1IF | INTCON3_INT1IP,
        );
        run(&mut int1, 1);
        assert_eq!(int1.get_pc(), PIC18_HIGH_VECTOR);
    }

    #[test]
    fn interrupt_int0_is_always_high_priority() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0000]);
        cpu.write_data(SFR_RCON, RCON_IPEN);
        cpu.write_data(SFR_INTCON, INTCON_GIE_GIEH | INTCON_GIEL);
        cpu.write_data(
            SFR_INTCON,
            INTCON_GIE_GIEH | INTCON_GIEL | INTCON_INT0IE | INTCON_INT0IF,
        );
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), PIC18_HIGH_VECTOR);
    }

    #[test]
    fn interrupt_is_masked_when_global_enable_is_clear() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0000]);
        cpu.write_data(SFR_INTCON, 0); // GIE clear
        cpu.write_data(SFR_PIE1, 0xFF);
        cpu.write_data(SFR_PIR1, 0xFF);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x0002);
        assert_eq!(cpu.stack_pointer(), 0);
    }

    #[test]
    fn retfie_fast_restores_the_context_saved_on_interrupt_entry() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0000]);
        // ISR at 0x0008: MOVLW 0x99, MOVLB 5, RETFIE FAST.
        let isr: Vec<u8> = [0x0E99u16, 0x0105, 0x0011]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        cpu.load_flash(PIC18_HIGH_VECTOR, &isr);

        // Interrupted context — no CALL FAST ever runs, so only interrupt
        // entry can have filled the fast register stack.
        cpu.wreg = 0x34;
        cpu.status = STATUS_Z;
        cpu.bsr = 0x02;
        cpu.write_data(SFR_INTCON, INTCON_GIE_GIEH | INTCON_GIEL);
        cpu.write_data(SFR_PIE1, 0x01);
        cpu.write_data(SFR_PIR1, 0x01);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), PIC18_HIGH_VECTOR);

        cpu.write_data(SFR_PIR1, 0); // ISR acknowledges the source
        run(&mut cpu, 2); // MOVLW 0x99, MOVLB 5 clobber WREG/BSR
        assert_eq!(cpu.wreg(), 0x99);
        assert_eq!(cpu.bsr(), 0x05);
        cpu.status = STATUS_C; // and the ISR's arithmetic clobbers STATUS

        run(&mut cpu, 1); // RETFIE FAST
        assert_eq!(cpu.wreg(), 0x34);
        assert_eq!(cpu.status(), STATUS_Z);
        assert_eq!(cpu.bsr(), 0x02);
        assert_eq!(cpu.get_pc(), 0x0002);
    }

    #[test]
    fn retfie_from_a_high_isr_nested_in_a_low_isr_keeps_giel_clear() {
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0000, 0x0000]);
        cpu.load_flash(PIC18_HIGH_VECTOR, &0x0010u16.to_le_bytes()); // RETFIE
        cpu.load_flash(PIC18_LOW_VECTOR, &[0x00; 8]); // NOPs
        cpu.write_data(SFR_RCON, RCON_IPEN);
        cpu.write_data(SFR_INTCON, INTCON_GIE_GIEH | INTCON_GIEL);
        cpu.write_data(SFR_PIE1, 0x03);
        cpu.write_data(SFR_IPR1, 0x02); // bit 0 low, bit 1 high

        // A low source enters the low ISR and stays pending.
        cpu.write_data(SFR_PIR1, 0x01);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), PIC18_LOW_VECTOR);
        run(&mut cpu, 1);
        let in_low_isr = cpu.get_pc();

        // A high source nests after the next low-ISR NOP retires, then its
        // ISR acknowledges it and RETFIEs.
        cpu.write_data(SFR_PIR1, 0x03);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), PIC18_HIGH_VECTOR);
        assert_eq!(cpu.stack_pointer(), 2);
        cpu.write_data(SFR_PIR1, 0x01);
        run(&mut cpu, 1); // RETFIE
        let in_low_isr = in_low_isr + 2;
        assert_eq!(cpu.get_pc(), in_low_isr);
        let intcon = cpu.read_data(SFR_INTCON);
        assert_eq!(intcon & INTCON_GIE_GIEH, INTCON_GIE_GIEH, "GIEH restored");
        assert_eq!(intcon & INTCON_GIEL, 0, "GIEL stays clear in the low ISR");

        // The still-pending low source must not re-enter the low ISR.
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), in_low_isr + 2);
        assert_eq!(cpu.stack_pointer(), 1);
    }

    #[test]
    fn peie_gates_peripheral_sources_when_ipen_is_clear() {
        // GIE=1, PEIE=0: a PIR1/PIE1 pair must NOT vector.
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0000, 0x0000]);
        cpu.write_data(SFR_INTCON, INTCON_GIE_GIEH);
        cpu.write_data(SFR_PIE1, 0x01);
        cpu.write_data(SFR_PIR1, 0x01);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x0002, "PEIE=0 masks PIR1 with IPEN=0");

        // Setting PEIE (INTCON<6>, raw 0x40) lets it through.
        cpu.write_data(SFR_INTCON, 0x80 | 0x40);
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), PIC18_HIGH_VECTOR);

        // INTCON's own sources (TMR0 here) need only GIE, not PEIE.
        let mut tmr0 = Pic18::new();
        tmr0.load_words(&[0x0000]);
        tmr0.write_data(SFR_INTCON, INTCON_GIE_GIEH | INTCON_TMR0IE | INTCON_TMR0IF);
        run(&mut tmr0, 1);
        assert_eq!(tmr0.get_pc(), PIC18_HIGH_VECTOR);
    }

    #[test]
    fn low_priority_needs_gieh_as_well_as_giel() {
        // IPEN=1, GIEL=1 but GIEH=0: GIEH=0 disables all interrupts.
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0000]);
        cpu.write_data(SFR_RCON, RCON_IPEN);
        cpu.write_data(SFR_INTCON, INTCON_GIEL);
        cpu.write_data(SFR_PIE1, 0x01);
        cpu.write_data(SFR_PIR1, 0x01);
        cpu.write_data(SFR_IPR1, 0x00); // low-priority source
        run(&mut cpu, 1);
        assert_eq!(cpu.get_pc(), 0x0002, "GIEH=0 masks a low source too");
    }

    #[test]
    fn intcon3_matches_the_pic18f4520_datasheet_bit_layout() {
        // Raw bit values from DS39631 Register 9-3, deliberately NOT the
        // module's own constants: INT2IP=7, INT1IP=6, INT2IE=4, INT1IE=3,
        // INT2IF=1, INT1IF=0.
        let vector_for = |intcon3: u8| {
            let mut cpu = Pic18::new();
            cpu.load_words(&[0x0000]);
            cpu.write_data(SFR_RCON, RCON_IPEN);
            cpu.write_data(SFR_INTCON, 0x80 | 0x40); // GIEH | GIEL
            cpu.write_data(SFR_INTCON3, intcon3);
            run(&mut cpu, 1);
            cpu.get_pc()
        };
        assert_eq!(vector_for(0x08 | 0x01), PIC18_LOW_VECTOR, "INT1, IP=0");
        assert_eq!(
            vector_for(0x40 | 0x08 | 0x01),
            PIC18_HIGH_VECTOR,
            "INT1, IP=1"
        );
        assert_eq!(vector_for(0x10 | 0x02), PIC18_LOW_VECTOR, "INT2, IP=0");
        assert_eq!(
            vector_for(0x80 | 0x10 | 0x02),
            PIC18_HIGH_VECTOR,
            "INT2, IP=1"
        );
        // Flag without enable, and the unimplemented bits 5/2, never vector.
        assert_eq!(vector_for(0x01 | 0x02), 0x0002);
        assert_eq!(vector_for(0x20 | 0x04), 0x0002);
    }
}
