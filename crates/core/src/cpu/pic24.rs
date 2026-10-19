// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! PIC24 / dsPIC33F CPU — **M0 + M1a subset** for the dsPIC33FJ32MC204 class.
//!
//! Scope:
//! * **M0** — reset/boot, the MOV/NOP/BRA subset needed for the UART smoke,
//!   CPU-internal UART1 transmit, deterministic stepping.
//! * **M1a** — Timers T1/T2/T3 (incl. the 32-bit T2/T3 pair), interrupt
//!   flags (IFS0), enables (IEC0), priorities (IPC0-IPC2), NSTDIS/DISI
//!   gating, exception entry with the datasheet's two-word stack frame, and
//!   RETFIE.
//!
//! Conventions (provisional until verified against an XC16 ELF):
//! * `pc` is a **word address**; each 24-bit instruction occupies one 4-byte
//!   little-endian slot in flash (`INSTR_SLOT_BYTES`). The mapping of real
//!   XC16 ELF code sections onto this layout is an open item (see
//!   `docs/pic24-m0-reference.md`).
//! * Data space: `0x0000–0x07FF` SFR (UART1 registers modelled, the rest is a
//!   shadow latch), `0x0800–0x0FFF` 2 KB SRAM. Anything else is a
//!   [`SimulationError::MemoryViolation`] — never silently dropped.
//! * Instruction encodings are transcribed from DS70000157G (16-Bit MCU and
//!   DSC Programmer's Reference Manual); see the reference doc for the table.
//!
//! ## Modelled instructions
//!
//! | Instruction | Encoding source |
//! | :--- | :--- |
//! | `NOP` | DS70000157G |
//! | `MOV #lit16, Wnd` | DS70000157G |
//! | `MOV Wns, f` | DS70000157G |
//! | `MOV f, Wnd` | DS70000157G |
//! | `BRA Expr` (slit16) | DS70000157G |
//! | `RETFIE` | DS70000157G |
//!
//! UART1 registers (DS70283H): `U1MODE 0x0220`, `U1STA 0x0222`, `U1TXREG 0x0224`.

use crate::snapshot::{CpuSnapshot, Pic24CpuSnapshot};
use crate::{Bus, Cpu, SimResult, SimulationConfig, SimulationError, SimulationObserver};
use std::sync::{Arc, Mutex};

/// Flash size of the dsPIC33FJ32MC204 (DS70283H): 32 KB.
pub const FLASH_BYTES: usize = 32 * 1024;
/// One instruction slot: 24-bit word in a 4-byte little-endian slot (provisional).
pub const INSTR_SLOT_BYTES: usize = 4;
/// Instruction slots available in flash.
pub const INSTR_SLOTS: usize = FLASH_BYTES / INSTR_SLOT_BYTES;
/// Data-space SRAM base (X RAM), DS70283H Figure 4-3.
pub const RAM_BASE: u32 = 0x0800;
/// 2 KB SRAM.
pub const RAM_BYTES: usize = 2 * 1024;
/// SFR space size (0x0000–0x07FF).
pub const SFR_BYTES: usize = 0x0800;

/// UART1 mode register.
pub const SFR_U1MODE: u16 = 0x0220;
/// UART1 status register.
pub const SFR_U1STA: u16 = 0x0222;
/// UART1 transmit register (write a byte here).
pub const SFR_U1TXREG: u16 = 0x0224;

/// U1STA.TRMT — transmit shift register empty (DS70283H U1STA bit 8).
pub const U1STA_TRMT: u16 = 0x0100;
/// U1STA.UTXBF — transmit buffer full (DS70283H U1STA bit 9).
pub const U1STA_UTXBF: u16 = 0x0200;

// ── DS70283H SFR map (M1a additions) ────────────────────────────────────────

/// Accumulator A low / high / upper words.
pub const SFR_ACCAL: u16 = 0x0022;
pub const SFR_ACCAH: u16 = 0x0024;
/// Accumulator A upper word (bits 39:32).
pub const SFR_ACCAU: u16 = 0x0026;
pub const SFR_ACCBL: u16 = 0x0028;
pub const SFR_ACCBH: u16 = 0x002A;
pub const SFR_ACCBU: u16 = 0x002C;
/// CPU status register.
pub const SFR_SR: u16 = 0x0042;
/// Core control register (IPL3 lives in bit 3).
pub const SFR_CORCON: u16 = 0x0044;
/// Interrupt control registers.
pub const SFR_INTCON1: u16 = 0x0080;
pub const SFR_INTCON2: u16 = 0x0082;
/// Interrupt flag status 0.
pub const SFR_IFS0: u16 = 0x0084;
/// Interrupt enable control 0.
pub const SFR_IEC0: u16 = 0x0094;
/// Interrupt priority control 0/1/2.
pub const SFR_IPC0: u16 = 0x00A4;
pub const SFR_IPC1: u16 = 0x00A6;
pub const SFR_IPC2: u16 = 0x00A8;
/// Timer registers (DS70283H Table 4-x).
pub const SFR_TMR1: u16 = 0x0100;
pub const SFR_PR1: u16 = 0x0102;
pub const SFR_T1CON: u16 = 0x0104;
pub const SFR_TMR2: u16 = 0x0106;
pub const SFR_TMR3HLD: u16 = 0x0108;
pub const SFR_TMR3: u16 = 0x010A;
pub const SFR_PR2: u16 = 0x010C;
pub const SFR_PR3: u16 = 0x010E;
pub const SFR_T2CON: u16 = 0x0110;
pub const SFR_T3CON: u16 = 0x0112;

/// TCON.TON — timer on.
pub const TCON_TON: u16 = 0x8000;
/// TCON.TCKPS mask / shift (`00`=1:1, `01`=1:8, `10`=1:64, `11`=1:256).
pub const TCON_TCKPS_MASK: u16 = 0x0030;
pub const TCON_TCKPS_SHIFT: u16 = 4;
/// T2CON.T32 — Timer2/3 32-bit pair mode.
pub const T2CON_T32: u16 = 0x0008;

/// IFS0 flag bit and vector number for each modelled source (DS70283H §7.3.1).
pub const IRQ_INT0: (u16, u32) = (0, 8);
pub const IRQ_IC1: (u16, u32) = (1, 9);
pub const IRQ_OC1: (u16, u32) = (2, 10);
pub const IRQ_T1: (u16, u32) = (3, 11);
pub const IRQ_IC2: (u16, u32) = (5, 12);
pub const IRQ_OC2: (u16, u32) = (6, 13);
pub const IRQ_T2: (u16, u32) = (7, 14);
pub const IRQ_T3: (u16, u32) = (8, 15);

/// INTCON1.NSTDIS — nesting disable (bit 15).
pub const INTCON1_NSTDIS: u16 = 0x8000;
/// INTCON2.DISI — disable interrupts 1-6 (bit 14).
pub const INTCON2_DISI: u16 = 0x4000;

/// Exception entry costs two extra instruction cycles (DS70283H §7).
pub const EXCEPTION_ENTRY_CYCLES: u32 = 2;
/// RETFIE takes three instruction cycles (DS70000157G).
pub const RETFIE_CYCLES: u32 = 3;

/// RETFIE — 0000 0110 0100 0000 0000 0000 (DS70000157G).
pub const INSN_RETFIE: u32 = 0x0064_0000;

/// dsPIC33F CPU core (M0 subset).
#[derive(Debug, Clone)]
pub struct Pic24 {
    /// W0–W15. W15 is the stack pointer.
    pub w: [u16; 16],
    /// Program counter — **word address** (see module docs).
    pub pc: u32,
    /// Status register (SR). Flags are not yet evaluated by the M0 subset.
    pub sr: u16,
    /// Retired instruction cycles (1 per single-cycle instruction).
    pub cycles: u64,
    flash: Vec<u8>,
    sfr: Vec<u8>,
    ram: Vec<u8>,
    u1mode: u16,
    u1sta: u16,
    // M1a: timers (with prescaler phase counters), interrupt latches, accumulators.
    t1con: u16,
    tmr1: u16,
    pr1: u16,
    t1_frac: u16,
    t2con: u16,
    tmr2: u16,
    pr2: u16,
    t2_frac: u16,
    t3con: u16,
    tmr3: u16,
    pr3: u16,
    t3_frac: u16,
    ifs0: u16,
    iec0: u16,
    ipc0: u16,
    ipc1: u16,
    ipc2: u16,
    intcon1: u16,
    intcon2: u16,
    corcon: u16,
    acc_a: i64,
    acc_b: i64,
    serial_sink: Option<Arc<Mutex<Vec<u8>>>>,
    exception_pending: Option<u32>,
}

impl Default for Pic24 {
    fn default() -> Self {
        Self::new()
    }
}

impl Pic24 {
    pub fn new() -> Self {
        Self {
            w: [0; 16],
            pc: 0,
            sr: 0,
            cycles: 0,
            flash: vec![0xFF; FLASH_BYTES],
            sfr: vec![0; SFR_BYTES],
            ram: vec![0; RAM_BYTES],
            u1mode: 0,
            u1sta: 0,
            t1con: 0,
            tmr1: 0,
            pr1: 0,
            t1_frac: 0,
            t2con: 0,
            tmr2: 0,
            pr2: 0,
            t2_frac: 0,
            t3con: 0,
            tmr3: 0,
            pr3: 0,
            t3_frac: 0,
            ifs0: 0,
            iec0: 0,
            ipc0: 0,
            ipc1: 0,
            ipc2: 0,
            intcon1: 0,
            intcon2: 0,
            corcon: 0,
            acc_a: 0,
            acc_b: 0,
            serial_sink: None,
            exception_pending: None,
        }
    }

    /// Load instruction words into flash from slot 0 (tests / fixtures).
    ///
    /// Words are 24-bit; the top byte of each slot is ignored.
    pub fn load_words(&mut self, words: &[u32]) {
        assert!(
            words.len() <= INSTR_SLOTS,
            "fixture exceeds flash instruction slots"
        );
        for (i, word) in words.iter().enumerate() {
            let off = i * INSTR_SLOT_BYTES;
            self.flash[off..off + 3].copy_from_slice(&word.to_le_bytes()[..3]);
        }
    }

    /// Load raw instruction-slot bytes at a word address (Harvard flash), the
    /// PIC24 counterpart of `Avr::load_flash` for fixture harnesses.
    pub fn load_flash(&mut self, addr_word: u32, bytes: &[u8]) {
        let off = addr_word as usize * INSTR_SLOT_BYTES;
        assert!(off + bytes.len() <= FLASH_BYTES, "fixture exceeds flash");
        self.flash[off..off + bytes.len()].copy_from_slice(bytes);
    }

    /// Load a program image (engine path). Segments are copied into flash at
    /// their start address; the entry point becomes the reset PC.
    pub fn load_program_image(&mut self, image: &crate::memory::ProgramImage) {
        for seg in &image.segments {
            let start = seg.start_addr as usize;
            let end = start.saturating_add(seg.data.len()).min(FLASH_BYTES);
            if start < FLASH_BYTES {
                self.flash[start..end].copy_from_slice(&seg.data[..end - start]);
            }
        }
        self.pc = image.entry_point as u32;
    }

    /// Attach the console sink that receives UART1 TX bytes (same type the AVR
    /// core and the session builders use).
    pub fn set_serial_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>) {
        self.serial_sink = Some(sink);
    }

    // ── M1a: timers, stack, interrupts ──────────────────────────────────────

    /// Timer prescaler from TCKPS (`00`=1:1, `01`=1:8, `10`=1:64, `11`=1:256).
    fn prescale(con: u16) -> u32 {
        match (con & TCON_TCKPS_MASK) >> TCON_TCKPS_SHIFT {
            0 => 1,
            1 => 8,
            2 => 64,
            _ => 256,
        }
    }

    /// Advance all running timers by `cycles` instruction cycles.
    ///
    /// Period match is `TMR > PR` (period `PR + 1` ticks): the timer wraps to
    /// zero and the IFS0 flag is set — the datasheet's "match → reset → flag"
    /// behaviour. In 32-bit mode Timer2/3 is controlled by Timer2's TON and
    /// prescaler with the interrupt reported as Timer3 (DS70283H §12).
    fn advance_timers(&mut self, cycles: u32) {
        if self.t1con & TCON_TON != 0 {
            let psc = Self::prescale(self.t1con);
            let acc = u32::from(self.t1_frac) + cycles;
            self.t1_frac = (acc % psc) as u16;
            for _ in 0..(acc / psc) {
                self.tmr1 = self.tmr1.wrapping_add(1);
                if self.tmr1 > self.pr1 {
                    self.tmr1 = 0;
                    self.ifs0 |= 1 << IRQ_T1.0;
                }
            }
        }
        if self.t2con & T2CON_T32 == 0 {
            if self.t2con & TCON_TON != 0 {
                let psc = Self::prescale(self.t2con);
                let acc = u32::from(self.t2_frac) + cycles;
                self.t2_frac = (acc % psc) as u16;
                for _ in 0..(acc / psc) {
                    self.tmr2 = self.tmr2.wrapping_add(1);
                    if self.tmr2 > self.pr2 {
                        self.tmr2 = 0;
                        self.ifs0 |= 1 << IRQ_T2.0;
                    }
                }
            }
            if self.t3con & TCON_TON != 0 {
                let psc = Self::prescale(self.t3con);
                let mut acc = u32::from(self.t3_frac) + cycles;
                self.t3_frac = (acc % psc) as u16;
                acc /= psc;
                for _ in 0..acc {
                    self.tmr3 = self.tmr3.wrapping_add(1);
                    if self.tmr3 > self.pr3 {
                        self.tmr3 = 0;
                        self.ifs0 |= 1 << IRQ_T3.0;
                    }
                }
            }
        } else if self.t2con & TCON_TON != 0 {
            let psc = Self::prescale(self.t2con);
            let mut acc = u32::from(self.t2_frac) + cycles;
            self.t2_frac = (acc % psc) as u16;
            acc /= psc;
            let period = (u32::from(self.pr3) << 16) | u32::from(self.pr2);
            let mut cnt = (u32::from(self.tmr3) << 16) | u32::from(self.tmr2);
            for _ in 0..acc {
                cnt = cnt.wrapping_add(1);
                if cnt > period {
                    cnt = 0;
                    self.ifs0 |= 1 << IRQ_T3.0;
                }
            }
            self.tmr2 = cnt as u16;
            self.tmr3 = (cnt >> 16) as u16;
        }
    }

    /// Push one 16-bit word on the W15 stack (SRAM only, as on silicon).
    fn push_word(&mut self, val: u16) -> SimResult<()> {
        self.w[15] = self.w[15].wrapping_sub(2);
        let a = self.w[15];
        let end = RAM_BASE as u16 + RAM_BYTES as u16;
        if a >= RAM_BASE as u16 && a + 2 <= end {
            let o = a as usize - RAM_BASE as usize;
            self.ram[o..o + 2].copy_from_slice(&val.to_le_bytes());
            Ok(())
        } else {
            Err(SimulationError::MemoryViolation(u64::from(a)))
        }
    }

    fn pop_word(&mut self) -> SimResult<u16> {
        let a = self.w[15];
        let end = RAM_BASE as u16 + RAM_BYTES as u16;
        if a < RAM_BASE as u16 || a + 2 > end {
            return Err(SimulationError::MemoryViolation(u64::from(a)));
        }
        let o = a as usize - RAM_BASE as usize;
        let val = u16::from_le_bytes([self.ram[o], self.ram[o + 1]]);
        self.w[15] = self.w[15].wrapping_add(2);
        Ok(val)
    }

    /// Current CPU priority level: IPL<2:0> (SR<7:5>) plus IPL<3> (CORCON<3>).
    fn current_ipl(&self) -> u16 {
        ((self.sr >> 5) & 0x7) | (((self.corcon >> 3) & 1) << 3)
    }

    /// The highest-priority pending, enabled interrupt: (vector, priority).
    ///
    /// Natural order = IFS0 bit order (DS70283H §7.3.1). Simplified gating:
    /// DISI blocks everything we model (levels 1-6), NSTDIS blocks nesting.
    fn pending_interrupt(&self) -> Option<(u32, u16)> {
        if self.intcon2 & INTCON2_DISI != 0 {
            return None;
        }
        let ipl = self.current_ipl();
        if self.intcon1 & INTCON1_NSTDIS != 0 && ipl > 0 {
            return None;
        }
        let sources: [(u16, u32, u8); 8] = [
            (IRQ_INT0.0, IRQ_INT0.1, 0),
            (IRQ_IC1.0, IRQ_IC1.1, 4),
            (IRQ_OC1.0, IRQ_OC1.1, 8),
            (IRQ_T1.0, IRQ_T1.1, 12),
            (IRQ_IC2.0, IRQ_IC2.1, 4),
            (IRQ_OC2.0, IRQ_OC2.1, 8),
            (IRQ_T2.0, IRQ_T2.1, 12),
            (IRQ_T3.0, IRQ_T3.1, 12),
        ];
        for (i, (bit, vector, shift)) in sources.iter().enumerate() {
            if self.ifs0 & (1 << bit) == 0 || self.iec0 & (1 << bit) == 0 {
                continue;
            }
            let ipc = match i {
                0..=3 => self.ipc0,
                4..=6 => self.ipc1,
                _ => self.ipc2,
            };
            let prio = (ipc >> shift) & 0x7;
            if prio != 0 && prio > ipl {
                return Some((*vector, prio));
            }
        }
        None
    }

    /// Take the pending interrupt, if any: push the DS70283H frame
    /// (PC<15:0>, then SR<7:0>|IPL3|PC<22:16>), raise IPL, jump the vector.
    fn take_interrupt(&mut self) -> SimResult<bool> {
        let Some((vector, prio)) = self.pending_interrupt() else {
            return Ok(false);
        };
        self.push_word(self.pc as u16)?;
        let packed = ((self.sr & 0x00FF) << 8)
            | (((self.corcon >> 3) & 1) << 7)
            | ((self.pc >> 16) as u16 & 0x007F);
        self.push_word(packed)?;
        self.sr = (self.sr & !0x00E0) | ((prio & 0x7) << 5);
        self.pc = 0x4 + 2 * vector;
        Ok(true)
    }

    fn fetch(&self) -> SimResult<u32> {
        let off = self.pc as usize * INSTR_SLOT_BYTES;
        if off + INSTR_SLOT_BYTES > FLASH_BYTES {
            return Err(SimulationError::MemoryViolation(off as u64));
        }
        Ok(u32::from_le_bytes([
            self.flash[off],
            self.flash[off + 1],
            self.flash[off + 2],
            0,
        ]))
    }

    /// Data-space read (word). DS70283H Figure 4-3 layout.
    pub fn data_read(&self, addr: u16) -> SimResult<u16> {
        match addr {
            a if a < SFR_BYTES as u16 => match a {
                SFR_U1MODE => Ok(self.u1mode),
                // TRMT reads set once the (instant) shift completes; UTXBF never sticks.
                SFR_U1STA => Ok((self.u1sta | U1STA_TRMT) & !U1STA_UTXBF),
                SFR_U1TXREG => Ok(0),
                SFR_ACCAL => Ok((self.acc_a & 0xFFFF) as u16),
                SFR_ACCAH => Ok(((self.acc_a >> 16) & 0xFFFF) as u16),
                SFR_ACCAU => Ok(((self.acc_a >> 32) & 0x00FF) as u16),
                SFR_ACCBL => Ok((self.acc_b & 0xFFFF) as u16),
                SFR_ACCBH => Ok(((self.acc_b >> 16) & 0xFFFF) as u16),
                SFR_ACCBU => Ok(((self.acc_b >> 32) & 0x00FF) as u16),
                SFR_SR => Ok(self.sr),
                SFR_CORCON => Ok(self.corcon),
                SFR_INTCON1 => Ok(self.intcon1),
                SFR_INTCON2 => Ok(self.intcon2),
                SFR_IFS0 => Ok(self.ifs0),
                SFR_IEC0 => Ok(self.iec0),
                SFR_IPC0 => Ok(self.ipc0),
                SFR_IPC1 => Ok(self.ipc1),
                SFR_IPC2 => Ok(self.ipc2),
                SFR_TMR1 => Ok(self.tmr1),
                SFR_PR1 => Ok(self.pr1),
                SFR_T1CON => Ok(self.t1con),
                SFR_TMR2 => Ok(self.tmr2),
                SFR_TMR3 => Ok(self.tmr3),
                SFR_PR2 => Ok(self.pr2),
                SFR_PR3 => Ok(self.pr3),
                SFR_T2CON => Ok(self.t2con),
                SFR_T3CON => Ok(self.t3con),
                _ => {
                    let o = a as usize;
                    Ok(u16::from_le_bytes([self.sfr[o], self.sfr[o + 1]]))
                }
            },
            a if (RAM_BASE as u16..(RAM_BASE as u16 + RAM_BYTES as u16)).contains(&a) => {
                let o = a as usize - RAM_BASE as usize;
                Ok(u16::from_le_bytes([self.ram[o], self.ram[o + 1]]))
            }
            a => Err(SimulationError::MemoryViolation(a as u64)),
        }
    }

    /// Data-space write (word). UART1 TX latches into the console sink.
    pub fn data_write(&mut self, addr: u16, val: u16) -> SimResult<()> {
        match addr {
            a if a < SFR_BYTES as u16 => {
                match a {
                    SFR_U1MODE => self.u1mode = val,
                    SFR_U1STA => self.u1sta = val,
                    SFR_U1TXREG => {
                        if let Some(sink) = &self.serial_sink {
                            if let Ok(mut buf) = sink.lock() {
                                buf.push((val & 0x00FF) as u8);
                            }
                        }
                    }
                    SFR_ACCAL => self.acc_a = (self.acc_a & !0xFFFF) | i64::from(val),
                    SFR_ACCAH => self.acc_a = (self.acc_a & !0xFFFF_0000) | (i64::from(val) << 16),
                    SFR_ACCAU => {
                        self.acc_a =
                            (self.acc_a & !0x00FF_0000_0000) | (i64::from(val & 0x00FF) << 32)
                    }
                    SFR_ACCBL => self.acc_b = (self.acc_b & !0xFFFF) | i64::from(val),
                    SFR_ACCBH => self.acc_b = (self.acc_b & !0xFFFF_0000) | (i64::from(val) << 16),
                    SFR_ACCBU => {
                        self.acc_b =
                            (self.acc_b & !0x00FF_0000_0000) | (i64::from(val & 0x00FF) << 32)
                    }
                    SFR_SR => self.sr = val,
                    SFR_CORCON => self.corcon = val,
                    SFR_INTCON1 => self.intcon1 = val,
                    SFR_INTCON2 => self.intcon2 = val,
                    SFR_IFS0 => self.ifs0 = val,
                    SFR_IEC0 => self.iec0 = val,
                    SFR_IPC0 => self.ipc0 = val,
                    SFR_IPC1 => self.ipc1 = val,
                    SFR_IPC2 => self.ipc2 = val,
                    SFR_TMR1 => self.tmr1 = val,
                    SFR_PR1 => self.pr1 = val,
                    SFR_T1CON => self.t1con = val,
                    SFR_TMR2 => self.tmr2 = val,
                    SFR_TMR3 => self.tmr3 = val,
                    SFR_PR2 => self.pr2 = val,
                    SFR_PR3 => self.pr3 = val,
                    SFR_T2CON => self.t2con = val,
                    SFR_T3CON => self.t3con = val,
                    _ => {
                        let o = a as usize;
                        self.sfr[o..o + 2].copy_from_slice(&val.to_le_bytes());
                    }
                }
                Ok(())
            }
            a if (RAM_BASE as u16..(RAM_BASE as u16 + RAM_BYTES as u16)).contains(&a) => {
                let o = a as usize - RAM_BASE as usize;
                self.ram[o..o + 2].copy_from_slice(&val.to_le_bytes());
                Ok(())
            }
            a => Err(SimulationError::MemoryViolation(a as u64)),
        }
    }

    /// Execute one decoded word. Returns the instruction cycles consumed.
    fn execute(&mut self, word: u32) -> SimResult<u32> {
        // NOP — DS70000157G: 0000 0000 xxxx xxxx xxxx xxxx.
        if word & 0x00FFFF == 0 && word >> 16 == 0 {
            self.pc = self.pc.wrapping_add(1);
            return Ok(1);
        }

        // MOV #lit16, Wnd — 0010 kkkk kkkk kkkk kkkk dddd.
        if word & 0x00F0_0000 == 0x0020_0000 {
            let k = ((word >> 4) & 0xFFFF) as u16;
            let d = (word & 0xF) as usize;
            self.w[d] = k;
            self.pc = self.pc.wrapping_add(1);
            return Ok(1);
        }

        // MOV Wns, f — 1000 1fff ffff ffff ffff ssss (f is a 16-bit even address:
        // 15 stored bits, LSB implied 0 — DS70000157G).
        if word >> 19 == 0b1_0001 {
            let f = (((word >> 4) & 0x7FFF) << 1) as u16;
            let s = (word & 0xF) as usize;
            self.data_write(f, self.w[s])?;
            self.pc = self.pc.wrapping_add(1);
            return Ok(1);
        }

        // MOV f, Wnd — 1000 0fff ffff ffff ffff dddd.
        if word >> 19 == 0b1_0000 {
            let f = (((word >> 4) & 0x7FFF) << 1) as u16;
            let d = (word & 0xF) as usize;
            self.w[d] = self.data_read(f)?;
            self.pc = self.pc.wrapping_add(1);
            return Ok(1);
        }

        // BRA Expr — 0011 0111 nnnn nnnn nnnn nnnn (slit16 words: PC = PC + 2 + 2*offset).
        if word >> 16 == 0x0037 {
            let off = (word & 0xFFFF) as u16 as i16;
            let next = self.pc as i64 + 2 + 2 * i64::from(off);
            if next < 0 {
                return Err(SimulationError::DecodeError(self.pc as u64));
            }
            self.pc = next as u32;
            return Ok(2);
        }

        // RETFIE — 0000 0110 0100 0000 0000 0000 (DS70000157G).
        if word == INSN_RETFIE {
            let low = self.pop_word()?;
            let packed = self.pop_word()?;
            self.sr = (self.sr & 0xFF00) | ((packed >> 8) & 0x00FF);
            self.corcon = (self.corcon & !0x0008) | (((packed >> 7) & 1) << 3);
            self.pc = (u32::from(packed & 0x007F) << 16) | u32::from(low);
            return Ok(RETFIE_CYCLES);
        }

        // Everything else is deliberately unimplemented (see module scope).
        Err(SimulationError::DecodeError(
            (self.pc as u64) * INSTR_SLOT_BYTES as u64,
        ))
    }
}

impl Cpu for Pic24 {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.w = [0; 16];
        self.pc = 0;
        self.sr = 0;
        self.cycles = 0;
        self.u1mode = 0;
        self.u1sta = 0;
        self.t1con = 0;
        self.tmr1 = 0;
        self.pr1 = 0;
        self.t1_frac = 0;
        self.t2con = 0;
        self.tmr2 = 0;
        self.pr2 = 0;
        self.t2_frac = 0;
        self.t3con = 0;
        self.tmr3 = 0;
        self.pr3 = 0;
        self.t3_frac = 0;
        self.ifs0 = 0;
        self.iec0 = 0;
        self.ipc0 = 0;
        self.ipc1 = 0;
        self.ipc2 = 0;
        self.intcon1 = 0;
        self.intcon2 = 0;
        self.corcon = 0;
        self.acc_a = 0;
        self.acc_b = 0;
        self.exception_pending = None;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
        // The per-instruction trace contract (see `SimulationObserver`): all
        // three notifications, in order, gated on anyone observing.
        let observed = !observers.is_empty();

        // Exception entry happens between instructions: vector first, then run
        // the handler's first instruction in this same retired step. The entry
        // itself retires nothing; it only costs cycles.
        let mut used = 0u32;
        if self.take_interrupt()? {
            used += EXCEPTION_ENTRY_CYCLES;
        }

        let pc = self.pc;
        let word = self.fetch()?;
        if observed {
            for obs in observers {
                obs.on_step_start(pc, word);
            }
        }

        used += self.execute(word)?;
        self.cycles += u64::from(used);
        crate::advance_batch_cycle(bus, u64::from(used));
        self.advance_timers(used);

        if observed {
            // W0-W15, then the standard trailer: SP (W15), then PC.
            let mut registers = [0u32; 18];
            for (slot, value) in registers.iter_mut().take(16).zip(self.w.iter()) {
                *slot = u32::from(*value);
            }
            registers[16] = u32::from(self.w[15]);
            registers[17] = self.pc;

            crate::emit_trace_event(
                observers,
                labwired_hw_trace::TraceEvent::InstructionRetired { pc, opcode: word },
            );
            for obs in observers {
                obs.on_step_end(used, &registers);
            }
        }
        Ok(())
    }

    fn clock_cycles(&self) -> u64 {
        self.cycles
    }

    /// Longest retired step: exception entry (2) plus RETFIE (3).
    fn max_step_cycles(&self) -> u32 {
        EXCEPTION_ENTRY_CYCLES + RETFIE_CYCLES
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, val: u32) {
        self.w[15] = val as u16;
    }

    fn set_exception_pending(&mut self, exception_num: u32) {
        self.exception_pending = Some(exception_num);
    }

    fn get_register(&self, id: u8) -> u32 {
        match id {
            0..=15 => u32::from(self.w[id as usize]),
            16 => self.pc,
            17 => u32::from(self.sr),
            _ => 0,
        }
    }

    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0..=15 => self.w[id as usize] = val as u16,
            16 => self.pc = val,
            17 => self.sr = val as u16,
            _ => {}
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names: Vec<String> = (0..16).map(|i| format!("W{i}")).collect();
        names.push("PC".to_string());
        names.push("SR".to_string());
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        let n = name.trim().to_ascii_uppercase();
        if let Some(rest) = n.strip_prefix('W') {
            if let Ok(i) = rest.parse::<u8>() {
                if i < 16 {
                    return Some(i);
                }
            }
        }
        match n.as_str() {
            "PC" => Some(16),
            "SR" => Some(17),
            _ => None,
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot::Pic24(Pic24CpuSnapshot {
            registers: self.w.to_vec(),
            pc: self.pc,
            sr: self.sr,
            cycles: self.cycles,
            u1mode: self.u1mode,
            u1sta: self.u1sta,
            t1con: self.t1con,
            tmr1: self.tmr1,
            pr1: self.pr1,
            t1_frac: self.t1_frac,
            t2con: self.t2con,
            tmr2: self.tmr2,
            pr2: self.pr2,
            t2_frac: self.t2_frac,
            t3con: self.t3con,
            tmr3: self.tmr3,
            pr3: self.pr3,
            t3_frac: self.t3_frac,
            ifs0: self.ifs0,
            iec0: self.iec0,
            ipc0: self.ipc0,
            ipc1: self.ipc1,
            ipc2: self.ipc2,
            intcon1: self.intcon1,
            intcon2: self.intcon2,
            corcon: self.corcon,
            acc_a: self.acc_a,
            acc_b: self.acc_b,
        })
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Pic24(s) = snapshot {
            if s.registers.len() == 16 {
                self.w.copy_from_slice(&s.registers);
            }
            self.pc = s.pc;
            self.sr = s.sr;
            self.cycles = s.cycles;
            self.u1mode = s.u1mode;
            self.u1sta = s.u1sta;
            self.t1con = s.t1con;
            self.tmr1 = s.tmr1;
            self.pr1 = s.pr1;
            self.t1_frac = s.t1_frac;
            self.t2con = s.t2con;
            self.tmr2 = s.tmr2;
            self.pr2 = s.pr2;
            self.t2_frac = s.t2_frac;
            self.t3con = s.t3con;
            self.tmr3 = s.tmr3;
            self.pr3 = s.pr3;
            self.t3_frac = s.t3_frac;
            self.ifs0 = s.ifs0;
            self.iec0 = s.iec0;
            self.ipc0 = s.ipc0;
            self.ipc1 = s.ipc1;
            self.ipc2 = s.ipc2;
            self.intcon1 = s.intcon1;
            self.intcon2 = s.intcon2;
            self.corcon = s.corcon;
            self.acc_a = s.acc_a;
            self.acc_b = s.acc_b;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;

    fn cpu_with(words: &[u32]) -> Pic24 {
        let mut cpu = Pic24::new();
        cpu.load_words(words);
        cpu
    }

    fn mov_lit_w0(val: u16) -> u32 {
        0x0020_0000 | (u32::from(val) << 4)
    }

    fn mov_w0_to_f(f: u16) -> u32 {
        debug_assert!(f % 2 == 0);
        0b1_0001 << 19 | (u32::from(f >> 1) << 4)
    }

    fn step(cpu: &mut Pic24, bus: &mut SystemBus) {
        cpu.step(bus, &[], &SimulationConfig::default()).unwrap();
    }

    #[test]
    fn reset_starts_at_word_zero() {
        let mut cpu = Pic24::new();
        let mut bus = SystemBus::new();
        cpu.reset(&mut bus).unwrap();
        assert_eq!(cpu.get_pc(), 0);
        assert_eq!(cpu.clock_cycles(), 0);
    }

    #[test]
    fn mov_literal_loads_w0() {
        let mut cpu = cpu_with(&[mov_lit_w0(0x48)]);
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.w[0], 0x48);
        assert_eq!(cpu.get_pc(), 1);
        assert_eq!(cpu.clock_cycles(), 1);
    }

    #[test]
    fn mov_w0_to_u1txreg_reaches_sink() {
        let sink: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let mut cpu = cpu_with(&[mov_lit_w0(0x48), mov_w0_to_f(SFR_U1TXREG)]);
        cpu.set_serial_sink(sink.clone());
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        step(&mut cpu, &mut bus);
        assert_eq!(*sink.lock().unwrap(), vec![b'H']);
        assert_eq!(cpu.get_pc(), 2);
    }

    #[test]
    fn nop_advances_pc() {
        let mut cpu = cpu_with(&[0x0000_0000]);
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.get_pc(), 1);
    }

    #[test]
    fn bra_minus_one_stays_put() {
        // BRA $-1 → PC = PC + 2 + 2*(-1) = PC.
        let mut cpu = cpu_with(&[0x0037_FFFF]);
        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus);
        assert_eq!(cpu.get_pc(), 0);
        assert_eq!(cpu.clock_cycles(), 2);
    }

    #[test]
    fn u1sta_read_reports_transmit_empty() {
        let cpu = Pic24::new();
        assert_eq!(cpu.data_read(SFR_U1STA).unwrap() & U1STA_TRMT, U1STA_TRMT);
        assert_eq!(cpu.data_read(SFR_U1STA).unwrap() & U1STA_UTXBF, 0);
    }

    #[test]
    fn ram_round_trip() {
        let mut cpu = Pic24::new();
        cpu.data_write(RAM_BASE as u16, 0xBEEF).unwrap();
        assert_eq!(cpu.data_read(RAM_BASE as u16).unwrap(), 0xBEEF);
    }

    #[test]
    fn unsupported_opcode_is_a_decode_error() {
        let mut cpu = cpu_with(&[0x00FF_FFFF]);
        let mut bus = SystemBus::new();
        let err = cpu.step(&mut bus, &[], &SimulationConfig::default());
        assert!(matches!(err, Err(SimulationError::DecodeError(_))));
    }

    // ── M1a: timers, interrupts, accumulators ───────────────────────────────

    fn enc(word: u32) -> [u8; INSTR_SLOT_BYTES] {
        word.to_le_bytes()
    }

    fn write_sfr(cpu: &mut Pic24, addr: u16, val: u16) {
        cpu.data_write(addr, val).expect("SFR write");
    }

    #[test]
    fn timer1_counts_to_period_and_raises_flag() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        write_sfr(&mut cpu, SFR_PR1, 2); // period = PR + 1 = 3 ticks
        write_sfr(&mut cpu, SFR_T1CON, TCON_TON);
        let mut bus = SystemBus::new();
        for _ in 0..3 {
            step(&mut cpu, &mut bus);
        }
        assert_eq!(cpu.data_read(SFR_TMR1).unwrap(), 0, "wraps on match");
        assert_ne!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_T1.0),
            0,
            "T1IF set on match"
        );
    }

    #[test]
    fn timer1_prescaler_1_to_8() {
        let mut cpu = cpu_with(&[0x0000_0000; 10]);
        write_sfr(&mut cpu, SFR_PR1, 0);
        write_sfr(&mut cpu, SFR_T1CON, TCON_TON | (1 << TCON_TCKPS_SHIFT)); // 1:8
        let mut bus = SystemBus::new();
        for _ in 0..7 {
            step(&mut cpu, &mut bus);
        }
        assert_eq!(
            cpu.data_read(SFR_IFS0).unwrap(),
            0,
            "no tick before the 8th instruction cycle"
        );
        step(&mut cpu, &mut bus);
        assert_ne!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_T1.0),
            0,
            "ticked and matched on the 8th"
        );
    }

    #[test]
    fn timer23_pair_counts_32_bit_and_flags_timer3() {
        let mut cpu = cpu_with(&[0x0000_0000; 5]);
        write_sfr(&mut cpu, SFR_PR2, 2); // 32-bit period = 3 ticks
        write_sfr(&mut cpu, SFR_PR3, 0);
        write_sfr(&mut cpu, SFR_T2CON, TCON_TON | T2CON_T32);
        let mut bus = SystemBus::new();
        for _ in 0..2 {
            step(&mut cpu, &mut bus);
        }
        assert_eq!(cpu.data_read(SFR_IFS0).unwrap(), 0, "not yet at period");
        step(&mut cpu, &mut bus);
        assert_ne!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_T3.0),
            0,
            "32-bit match reports as Timer3 (DS70283H §12)"
        );
        assert_eq!(cpu.data_read(SFR_TMR2).unwrap(), 0);
        assert_eq!(cpu.data_read(SFR_TMR3).unwrap(), 0);
    }

    #[test]
    fn timer1_interrupt_vectors_frames_and_returns() {
        let mut cpu = cpu_with(&[0x0037_FFFF]); // slot 0: BRA $-1
                                                // Handler at the T1 vector (number 11 -> address 0x1A):
                                                //   MOV #0, W0 ; MOV W0, T1CON ; MOV W0, IFS0 ; RETFIE
                                                // The timer is stopped first: with PR1 = 0 a tick re-sets the flag in
                                                // the very same step that clears it — correct silicon behaviour, but
                                                // not what this test wants to observe.
        let mut handler = Vec::new();
        handler.extend(enc(mov_lit_w0(0)));
        handler.extend(enc(mov_w0_to_f(SFR_T1CON)));
        handler.extend(enc(mov_w0_to_f(SFR_IFS0)));
        handler.extend(enc(INSN_RETFIE));
        cpu.load_flash(0x1A, &handler);

        cpu.set_sp(0x1000);
        write_sfr(&mut cpu, SFR_PR1, 0); // flag after every tick
        write_sfr(&mut cpu, SFR_T1CON, TCON_TON);
        write_sfr(&mut cpu, SFR_IEC0, 1 << IRQ_T1.0);
        write_sfr(&mut cpu, SFR_IPC0, 1 << 12); // T1IP = 1

        let mut bus = SystemBus::new();
        step(&mut cpu, &mut bus); // BRA; the timer ticks -> T1IF
        assert_ne!(cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_T1.0), 0);

        step(&mut cpu, &mut bus); // vector + first handler instruction
        assert_eq!(cpu.get_pc(), 0x1B, "handler began at the T1 vector");
        assert_eq!(
            (cpu.data_read(SFR_SR).unwrap() >> 5) & 0x7,
            1,
            "IPL raised to the source priority"
        );

        // DS70283H frame: [SP] = SR<7:0>|IPL3|PC<22:16>, [SP+2] = PC<15:0>.
        let sp = cpu.w[15];
        assert_eq!(cpu.data_read(sp).unwrap(), 0x0000, "packed frame word");
        assert_eq!(cpu.data_read(sp + 2).unwrap(), 0x0000, "saved PC (was 0)");

        step(&mut cpu, &mut bus); // MOV W0, T1CON stops the timer
        assert_eq!(
            cpu.data_read(SFR_T1CON).unwrap() & TCON_TON,
            0,
            "ISR stopped Timer1"
        );
        step(&mut cpu, &mut bus); // MOV W0, IFS0 clears the flag
        assert_eq!(
            cpu.data_read(SFR_IFS0).unwrap() & (1 << IRQ_T1.0),
            0,
            "ISR cleared T1IF"
        );
        step(&mut cpu, &mut bus); // RETFIE
        assert_eq!(cpu.get_pc(), 0, "returned to the interrupted loop");
        assert_eq!(cpu.w[15], 0x1000, "stack fully unwound");
        assert_eq!(
            (cpu.data_read(SFR_SR).unwrap() >> 5) & 0x7,
            0,
            "IPL restored"
        );
    }

    #[test]
    fn disabled_source_does_not_vector() {
        let mut cpu = cpu_with(&[0x0000_0000; 4]);
        write_sfr(&mut cpu, SFR_PR1, 0);
        write_sfr(&mut cpu, SFR_T1CON, TCON_TON);
        write_sfr(&mut cpu, SFR_IPC0, 1 << 12); // priority set, IEC0 clear
        cpu.set_sp(0x1000);
        let mut bus = SystemBus::new();
        for _ in 0..4 {
            step(&mut cpu, &mut bus);
        }
        assert_eq!(cpu.get_pc(), 4, "kept executing linearly");
        assert_eq!(cpu.w[15], 0x1000);
    }

    #[test]
    fn accumulator_sfr_words_round_trip() {
        let mut cpu = Pic24::new();
        write_sfr(&mut cpu, SFR_ACCAL, 0x1234);
        write_sfr(&mut cpu, SFR_ACCAH, 0x5678);
        write_sfr(&mut cpu, SFR_ACCAU, 0x9A);
        assert_eq!(cpu.data_read(SFR_ACCAL).unwrap(), 0x1234);
        assert_eq!(cpu.data_read(SFR_ACCAH).unwrap(), 0x5678);
        assert_eq!(cpu.data_read(SFR_ACCAU).unwrap(), 0x9A);
        write_sfr(&mut cpu, SFR_ACCBL, 0xBEEF);
        assert_eq!(cpu.data_read(SFR_ACCBL).unwrap(), 0xBEEF);
        assert_eq!(
            cpu.data_read(SFR_ACCAU).unwrap(),
            0x9A,
            "B write left A alone"
        );
    }
}
