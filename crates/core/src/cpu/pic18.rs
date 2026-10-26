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
//! * `a = 0` uses the Access Bank: `f < 0x60` is access RAM `0x000..0x05F`,
//!   `f >= 0x60` is the SFR window `0xF60..0xFFF`.
//!
//! Access-bank `INDFn`, `POSTINCn`, `POSTDECn`, `PREINCn` and `PLUSWn` are
//! virtual registers. Byte and bit file operations route through the
//! per-instruction resolver, which uses `FSRn` as a full 12-bit data address
//! and applies the alias's FSR update once per instruction access.
//! `MOVFF` keeps its 12-bit absolute-address path (see the progress doc).
//!
//! This milestone models the instruction subset listed in
//! `docs/PIC18-CORE-PROGRESS.md`. Anything else returns
//! [`SimulationError::DecodeError`]; it is never guessed.

use crate::snapshot::{CpuSnapshot, Pic18CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
use std::sync::Arc;

/// Modeled program-flash size. A per-device geometry can be supplied with
/// [`Pic18::with_geometry`]; this default is large enough for the hand
/// fixtures and does not claim any catalogue part.
pub const PIC18_FLASH_BYTES: usize = 64 * 1024;
/// Classic 16-bank data file (`0x000..0xFFF`).
pub const PIC18_DATA_BYTES: usize = 0x1000;
/// 12-bit data-space mask; classic PIC18 `FSRn` values are a full 12-bit
/// address, not a bank-relative offset.
pub const PIC18_DATA_MASK: u16 = (PIC18_DATA_BYTES - 1) as u16;
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
        Self {
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
        }
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

    /// Load raw bytes at a byte-addressed program address.
    pub fn load_flash(&mut self, addr: u32, bytes: &[u8]) {
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
        } else if f < 0x60 {
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
            _ => {
                if let Some(slot) = self.data.get_mut(addr as usize) {
                    *slot = value;
                }
            }
        }
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
            let src = word & 0x0FFF;
            let dst = second & 0x0FFF;
            let value = self.read_data(src);
            self.pc = Self::align_pc(pc.wrapping_add(4));
            self.write_data(dst, value);
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
        self.wreg = 0;
        self.status = 0;
        self.bsr = 0;
        self.pc = 0;
        self.cycles = 0;
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
        self.data.fill(0);
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
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
        let mut cpu = Pic18::new();
        cpu.load_words(&[0x0D00]); // MULLW is outside this milestone's subset.
        let err = run_err(&mut cpu, 1);
        assert!(matches!(err, SimulationError::DecodeError(0)));
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
}
