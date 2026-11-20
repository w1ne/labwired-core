// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Texas Instruments MSP430 CPU: the original 16-bit CPU (MSP430x1xx) and the
//! 20-bit CPUX (MSP430X: F2xx large-memory parts, F5xx).
//!
//! What is modelled, and where it comes from (MSP430x1xx Family User's Guide
//! SLAU049, chapter 3 "RISC 16-Bit CPU"; MSP430x2xx Family User's Guide
//! SLAU144, chapter 4 "CPUX"). Encodings in the unit tests come from the TI
//! `msp430-elf-as` 2.34 (msp430-gcc 9.3.1.11), and the core is compared
//! register-for-register against the GDB `msp430-elf` simulator on
//! compiler-built programs (`crates/core/tests/vendor/msp430_gdb_sim_lockstep.rs`):
//!
//! * **CPU**: the 27 core instructions (format I double-operand, format II
//!   single-operand, the eight conditional jumps), all seven addressing modes
//!   and both constant generators (R2/R3), byte and word forms. `DADD` adds
//!   decimal digits with carry. Status bits follow the per-instruction
//!   tables: C after a subtract is "no borrow"; `BIT`/`AND`/`XOR`/`SXT` set
//!   C = not Z; `RRC`/`RRA` clear V.
//! * **CPUX** (`cpux` set): 20-bit registers and PC, the extension word
//!   (`.A` operand size, 20-bit immediates/indexes/absolute addresses, the
//!   register-mode repeat count and `ZC`), the address instructions
//!   (`MOVA`/`CMPA`/`ADDA`/`SUBA`), `RRCM`/`RRAM`/`RLAM`/`RRUM`, `PUSHM`/
//!   `POPM`, `CALLA` and the 20-bit `RETI` frame. A non-extended instruction
//!   clears bits 19:16 of a register it writes, and an indexed operand whose
//!   base register points into the lower 64 KiB wraps at 64 KiB, as the user's
//!   guide specifies.
//! * **Interrupts**: one request line per vector, numbered by priority from
//!   the top of the vector table (line `n` vectors through `0xFFFE - 2n`:
//!   0 = reset, 1 = NMI). Maskable requests need `SR.GIE`; the highest
//!   priority (lowest `n`) wins. Entry pushes PC then SR (with PC bits 19:16
//!   in SR bits 15:12 on CPUX) and clears SR except SCG0. The instruction
//!   after one that sets GIE always executes before a request is accepted.
//!   Requests come from the shared [`Msp430Irq`] (the module IFG/IE bits)
//!   or from [`Cpu::set_exception_pending`].
//! * **Low-power modes**: `SR.CPUOFF` stops instruction execution until an
//!   interrupt is accepted. OSCOFF/SCG0/SCG1 are storage.
//!
//! Not modelled: instruction timing (every instruction is one clock cycle,
//! where silicon takes 1 to 6 MCLK cycles), the oscillator-fault and
//! access-violation NMI sources, and vacant-memory behaviour (an unmapped
//! access is a simulation error). An encoding the selected CPU does not
//! define returns [`SimulationError::DecodeError`]; silicon has no
//! illegal-instruction exception.

use crate::peripherals::msp430::{Msp430Irq, LINE_NMI};
use crate::{Bus, Cpu, SimResult, SimulationError, SimulationObserver};
use std::sync::Arc;

pub const SR_C: u32 = 1 << 0;
pub const SR_Z: u32 = 1 << 1;
pub const SR_N: u32 = 1 << 2;
pub const SR_GIE: u32 = 1 << 3;
pub const SR_CPUOFF: u32 = 1 << 4;
pub const SR_SCG0: u32 = 1 << 6;
pub const SR_V: u32 = 1 << 8;
/// Implemented SR bits; bits 15:9 are reserved and read as zero.
const SR_MASK: u32 = 0x01FF;

const PC: usize = 0;
const SP: usize = 1;
const SR: usize = 2;
const CG: usize = 3;

/// The reset vector; interrupt line `n` vectors through `RESET_VECTOR - 2n`.
pub const RESET_VECTOR: u32 = 0xFFFE;

/// Snapshot of every architectural register of an [`Msp430`] core.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Msp430CpuSnapshot {
    pub r: Vec<u32>,
    pub cpux: bool,
    /// Requests raised through `set_exception_pending`, one bit per line.
    pub requested: u64,
    pub cycles: u64,
}

/// One MSP430 CPU core.
#[derive(Debug)]
pub struct Msp430 {
    /// R0 = PC, R1 = SP, R2 = SR, R3 = constant generator (always 0), R4-R15.
    /// 20 bits wide on CPUX, 16 bits otherwise.
    pub r: [u32; 16],
    cpux: bool,
    irq: Option<Arc<Msp430Irq>>,
    requested: u64,
    /// The instruction just executed was `RETI`.
    returned: bool,
    pub cycles: u64,
}

impl Default for Msp430 {
    fn default() -> Self {
        Self::new()
    }
}

/// Operand size: byte, word, or the CPUX 20-bit address word.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sz {
    B,
    W,
    A,
}

impl Sz {
    fn mask(self) -> u32 {
        match self {
            Sz::B => 0xFF,
            Sz::W => 0xFFFF,
            Sz::A => 0xF_FFFF,
        }
    }
    fn msb(self) -> u32 {
        match self {
            Sz::B => 0x80,
            Sz::W => 0x8000,
            Sz::A => 0x8_0000,
        }
    }
    fn bits(self) -> u32 {
        match self {
            Sz::B => 8,
            Sz::W => 16,
            Sz::A => 20,
        }
    }
    /// Auto-increment step and stack slot size.
    fn step(self) -> u32 {
        match self {
            Sz::B => 1,
            Sz::W => 2,
            Sz::A => 4,
        }
    }
}

/// A resolved operand.
#[derive(Clone, Copy, Debug)]
enum Loc {
    Reg(usize),
    Mem(u32),
    /// An immediate or constant-generator value; a store to it is discarded.
    Imm(u32),
}

/// The fields of a CPUX extension word that apply to the next instruction.
#[derive(Clone, Copy, Debug)]
struct Ext {
    word: u32,
}

impl Ext {
    /// A/L: 1 selects the instruction's own B/W size, 0 the address word.
    fn al(self) -> bool {
        self.word & 0x40 != 0
    }
    fn src_hi(self) -> u32 {
        (self.word >> 7) & 0xF
    }
    fn dst_hi(self) -> u32 {
        self.word & 0xF
    }
    fn zc(self) -> bool {
        self.word & 0x100 != 0
    }
}

fn sext16(v: u32) -> u32 {
    v as u16 as i16 as i32 as u32
}

impl Msp430 {
    /// The original 16-bit CPU.
    pub fn new() -> Self {
        Self {
            r: [0; 16],
            cpux: false,
            irq: None,
            requested: 0,
            returned: false,
            cycles: 0,
        }
    }

    /// The 20-bit CPUX.
    pub fn new_cpux() -> Self {
        Self {
            cpux: true,
            ..Self::new()
        }
    }

    /// The CPU a chip descriptor names: `core: msp430x` selects CPUX.
    pub fn for_chip(chip: &labwired_config::ChipDescriptor) -> Self {
        match chip.core.as_deref() {
            Some(core) if core.eq_ignore_ascii_case("msp430x") => Self::new_cpux(),
            _ => Self::new(),
        }
    }

    /// Share the chip's interrupt request lines with the core.
    pub fn attach_irq(&mut self, irq: Arc<Msp430Irq>) {
        self.irq = Some(irq);
    }

    pub fn is_cpux(&self) -> bool {
        self.cpux
    }

    pub fn sr(&self) -> u32 {
        self.r[SR]
    }

    fn addr_mask(&self) -> u32 {
        if self.cpux {
            0xF_FFFF
        } else {
            0xFFFF
        }
    }

    fn flag(&self, f: u32) -> bool {
        self.r[SR] & f != 0
    }

    fn put_flag(&mut self, f: u32, on: bool) {
        if on {
            self.r[SR] |= f;
        } else {
            self.r[SR] &= !f;
        }
    }

    fn set_nz(&mut self, v: u32, sz: Sz) {
        self.put_flag(SR_N, v & sz.msb() != 0);
        self.put_flag(SR_Z, v & sz.mask() == 0);
    }

    /// Register store: PC and SP keep bit 0 clear, SR keeps its implemented
    /// bits, and the constant generator R3 holds nothing.
    fn set_reg(&mut self, r: usize, v: u32) {
        let v = v & self.addr_mask();
        match r {
            PC | SP => self.r[r] = v & !1,
            SR => self.r[SR] = v & SR_MASK,
            CG => {}
            _ => self.r[r] = v,
        }
    }

    // ---- memory ----------------------------------------------------------

    fn rd(&mut self, bus: &mut dyn Bus, addr: u32, sz: Sz) -> SimResult<u32> {
        let a = addr & self.addr_mask();
        Ok(match sz {
            Sz::B => u32::from(bus.read_u8(u64::from(a))?),
            Sz::W => u32::from(bus.read_u16(u64::from(a & !1))?),
            Sz::A => {
                let a = a & !1;
                let lo = u32::from(bus.read_u16(u64::from(a))?);
                let hi = u32::from(bus.read_u16(u64::from((a + 2) & self.addr_mask()))?);
                lo | (hi & 0xF) << 16
            }
        })
    }

    fn wr(&mut self, bus: &mut dyn Bus, addr: u32, sz: Sz, v: u32) -> SimResult<()> {
        let a = addr & self.addr_mask();
        match sz {
            Sz::B => bus.write_u8(u64::from(a), v as u8),
            Sz::W => bus.write_u16(u64::from(a & !1), v as u16),
            Sz::A => {
                let a = a & !1;
                bus.write_u16(u64::from(a), v as u16)?;
                bus.write_u16(
                    u64::from((a + 2) & self.addr_mask()),
                    ((v >> 16) & 0xF) as u16,
                )
            }
        }
    }

    /// The next instruction-stream word; PC moves past it.
    fn fetch(&mut self, bus: &mut dyn Bus) -> SimResult<u32> {
        let pc = self.r[PC];
        let w = bus.read_u16(u64::from(pc))?;
        self.r[PC] = pc.wrapping_add(2) & self.addr_mask();
        Ok(u32::from(w))
    }

    fn push(&mut self, bus: &mut dyn Bus, sz: Sz, v: u32) -> SimResult<()> {
        let slot = if sz == Sz::A { 4 } else { 2 };
        self.r[SP] = self.r[SP].wrapping_sub(slot) & self.addr_mask();
        let sp = self.r[SP];
        self.wr(bus, sp, sz, v)
    }

    fn pop(&mut self, bus: &mut dyn Bus, sz: Sz) -> SimResult<u32> {
        let sp = self.r[SP];
        let v = self.rd(bus, sp, sz)?;
        let slot = if sz == Sz::A { 4 } else { 2 };
        self.r[SP] = sp.wrapping_add(slot) & self.addr_mask();
        Ok(v)
    }

    // ---- operands --------------------------------------------------------

    /// `base + index`. A 20-bit index (extension word) adds across the whole
    /// address space; a 16-bit index wraps at 64 KiB when the base points into
    /// the lower 64 KiB and is sign-extended otherwise (SLAU144 4.4.2).
    fn indexed(&self, base: u32, index: u32, hi: Option<u32>) -> u32 {
        match hi {
            Some(hi) => base.wrapping_add(hi << 16 | index) & 0xF_FFFF,
            None if base <= 0xFFFF => base.wrapping_add(index) & 0xFFFF,
            None => base.wrapping_add(sext16(index)) & 0xF_FFFF,
        }
    }

    /// Resolve a source operand (`As` field). Fetches its extension word and
    /// applies the auto-increment.
    fn src_loc(
        &mut self,
        bus: &mut dyn Bus,
        mode: u32,
        reg: usize,
        sz: Sz,
        hi: Option<u32>,
    ) -> SimResult<Loc> {
        Ok(match (reg, mode) {
            (CG, 0) => Loc::Imm(0),
            (CG, 1) => Loc::Imm(1),
            (CG, 2) => Loc::Imm(2),
            (CG, _) => Loc::Imm(sz.mask()),
            (SR, 2) => Loc::Imm(4),
            (SR, 3) => Loc::Imm(8),
            (_, 0) => Loc::Reg(reg),
            (SR, 1) => {
                let index = self.fetch(bus)?;
                Loc::Mem(hi.unwrap_or(0) << 16 | index)
            }
            (_, 1) => {
                // Symbolic mode (PC) is relative to the index word itself.
                let base = self.r[reg];
                let index = self.fetch(bus)?;
                Loc::Mem(self.indexed(base, index, hi))
            }
            (PC, 3) => {
                let imm = self.fetch(bus)?;
                Loc::Imm(hi.unwrap_or(0) << 16 | imm)
            }
            (_, 2) => Loc::Mem(self.r[reg]),
            _ => {
                let addr = self.r[reg];
                // SP stays word-aligned even for a byte operand.
                let step = if reg == SP {
                    sz.step().max(2)
                } else {
                    sz.step()
                };
                self.r[reg] = addr.wrapping_add(step) & self.addr_mask();
                Loc::Mem(addr)
            }
        })
    }

    /// Resolve a destination operand (`Ad` field).
    fn dst_loc(
        &mut self,
        bus: &mut dyn Bus,
        ad: u32,
        reg: usize,
        hi: Option<u32>,
    ) -> SimResult<Loc> {
        if ad == 0 {
            return Ok(Loc::Reg(reg));
        }
        let base = self.r[reg];
        let index = self.fetch(bus)?;
        Ok(match reg {
            // Absolute mode; R3 has no indexed form and decodes the same way.
            SR | CG => Loc::Mem(hi.unwrap_or(0) << 16 | index),
            _ => Loc::Mem(self.indexed(base, index, hi)),
        })
    }

    fn get(&mut self, bus: &mut dyn Bus, loc: Loc, sz: Sz) -> SimResult<u32> {
        match loc {
            Loc::Reg(r) => Ok(self.r[r] & sz.mask()),
            Loc::Mem(a) => self.rd(bus, a, sz),
            Loc::Imm(v) => Ok(v & sz.mask()),
        }
    }

    fn put(&mut self, bus: &mut dyn Bus, loc: Loc, sz: Sz, v: u32) -> SimResult<()> {
        match loc {
            Loc::Reg(r) => {
                self.set_reg(r, v & sz.mask());
                Ok(())
            }
            Loc::Mem(a) => self.wr(bus, a, sz, v),
            Loc::Imm(_) => Ok(()),
        }
    }

    // ---- arithmetic ------------------------------------------------------

    fn add(&mut self, a: u32, b: u32, carry: u32, sz: Sz) -> u32 {
        let wide = a + b + carry;
        let r = wide & sz.mask();
        self.put_flag(SR_C, wide > sz.mask());
        self.put_flag(SR_V, !(a ^ b) & (a ^ r) & sz.msb() != 0);
        self.set_nz(r, sz);
        r
    }

    /// Decimal add with carry, one BCD digit per nibble.
    fn dadd(&mut self, a: u32, b: u32, sz: Sz) -> u32 {
        let mut carry = u32::from(self.flag(SR_C));
        let mut r = 0;
        for digit in 0..sz.bits() / 4 {
            let shift = digit * 4;
            let mut s = ((a >> shift) & 0xF) + ((b >> shift) & 0xF) + carry;
            carry = u32::from(s >= 10);
            if carry != 0 {
                s -= 10;
            }
            r |= (s & 0xF) << shift;
        }
        self.put_flag(SR_C, carry != 0);
        self.set_nz(r, sz);
        r
    }

    /// N and Z from the result, C = not Z, V as given (`BIT`/`AND`/`XOR`/`SXT`).
    fn logic_flags(&mut self, r: u32, sz: Sz, v: bool) {
        self.set_nz(r, sz);
        let z = self.flag(SR_Z);
        self.put_flag(SR_C, !z);
        self.put_flag(SR_V, v);
    }

    // ---- interrupts ------------------------------------------------------

    /// The line the core would accept now.
    fn acceptable_line(&self) -> Option<u32> {
        let pending = self.requested | self.irq.as_ref().map_or(0, |i| i.pending());
        if pending == 0 {
            return None;
        }
        if pending & (1 << LINE_NMI) != 0 {
            return Some(LINE_NMI);
        }
        let maskable = pending & !0b11;
        (maskable != 0 && self.flag(SR_GIE)).then(|| maskable.trailing_zeros())
    }

    fn take_interrupt(&mut self, bus: &mut dyn Bus) -> SimResult<bool> {
        let Some(line) = self.acceptable_line() else {
            return Ok(false);
        };
        self.requested &= !(1 << line);
        if let Some(irq) = &self.irq {
            irq.accept(line);
        }
        let pc = self.r[PC];
        let sr = self.r[SR];
        self.push(bus, Sz::W, pc & 0xFFFF)?;
        // CPUX keeps PC bits 19:16 in the reserved top nibble of the saved SR.
        self.push(bus, Sz::W, sr | (pc >> 16) << 12)?;
        self.r[SR] = sr & SR_SCG0;
        self.r[PC] = self.rd(bus, RESET_VECTOR - 2 * line, Sz::W)? & !1;
        Ok(true)
    }

    // ---- execution -------------------------------------------------------

    fn exec(&mut self, bus: &mut dyn Bus, at: u32) -> SimResult<()> {
        let undefined = || SimulationError::DecodeError(u64::from(at));
        let mut w = self.fetch(bus)?;
        let mut ext = None;
        if self.cpux && w & 0xF800 == 0x1800 {
            ext = Some(Ext { word: w });
            w = self.fetch(bus)?;
        }
        match w >> 12 {
            0 if self.cpux && ext.is_none() => self.exec_address(bus, w),
            1 if w & 0xFC00 == 0x1000 => self.exec_single(bus, w, ext, at),
            1 if self.cpux && ext.is_none() && w & 0xFC00 == 0x1400 => self.exec_pushm_popm(bus, w),
            2 | 3 if ext.is_none() => {
                let taken = match (w >> 10) & 7 {
                    0 => !self.flag(SR_Z),
                    1 => self.flag(SR_Z),
                    2 => !self.flag(SR_C),
                    3 => self.flag(SR_C),
                    4 => self.flag(SR_N),
                    5 => self.flag(SR_N) == self.flag(SR_V),
                    6 => self.flag(SR_N) != self.flag(SR_V),
                    _ => true,
                };
                if taken {
                    // Signed 10-bit word offset from the next instruction.
                    let offset = (((w & 0x3FF) << 22) as i32 >> 21) as u32;
                    self.r[PC] = self.r[PC].wrapping_add(offset) & self.addr_mask();
                }
                Ok(())
            }
            4..=15 => self.exec_double(bus, w, ext, at),
            _ => Err(undefined()),
        }
    }

    /// The operand size and register-mode repeat count an extension word
    /// gives the instruction it prefixes.
    fn extended_size(
        &self,
        ext: Option<Ext>,
        byte: bool,
        register_mode: bool,
        wordless: bool,
        at: u32,
    ) -> SimResult<(Sz, u32, bool)> {
        let Some(e) = ext else {
            return Ok((if byte { Sz::B } else { Sz::W }, 1, false));
        };
        let sz = match (e.al(), byte) {
            (true, false) => Sz::W,
            (true, true) => Sz::B,
            (false, true) => Sz::A,
            // SWPBX.A and SXTX.A: these two have no byte form, so A/L alone
            // selects the address word.
            (false, false) if wordless => Sz::A,
            (false, false) => return Err(SimulationError::DecodeError(u64::from(at))),
        };
        if !register_mode {
            return Ok((sz, 1, false));
        }
        // Bit 7 selects a count held in Rn[3:0]; otherwise bits 3:0 hold n-1.
        let n = if e.word & 0x80 != 0 {
            self.r[(e.word & 0xF) as usize] & 0xF
        } else {
            e.word & 0xF
        };
        Ok((sz, n + 1, e.zc()))
    }

    /// Format I: `MOV` … `AND` with a source and a destination operand.
    fn exec_double(
        &mut self,
        bus: &mut dyn Bus,
        w: u32,
        ext: Option<Ext>,
        at: u32,
    ) -> SimResult<()> {
        let op = w >> 12;
        let sreg = ((w >> 8) & 0xF) as usize;
        let ad = (w >> 7) & 1;
        let mode = (w >> 4) & 3;
        let dreg = (w & 0xF) as usize;
        let register_mode = mode == 0 && ad == 0;
        let (sz, count, zc) = self.extended_size(ext, w & 0x40 != 0, register_mode, false, at)?;
        let (src_hi, dst_hi) = match ext {
            Some(e) if !register_mode => (Some(e.src_hi()), Some(e.dst_hi())),
            Some(_) => (Some(0), Some(0)),
            None => (None, None),
        };
        let src = self.src_loc(bus, mode, sreg, sz, src_hi)?;
        let s = self.get(bus, src, sz)?;
        let dst = self.dst_loc(bus, ad, dreg, dst_hi)?;
        for _ in 0..count {
            // A register source can also be the destination of the repeat.
            let s = match src {
                Loc::Reg(_) => self.get(bus, src, sz)?,
                _ => s,
            };
            if zc {
                self.put_flag(SR_C, false);
            }
            let carry = u32::from(self.flag(SR_C));
            let d = if op == 4 { 0 } else { self.get(bus, dst, sz)? };
            let (r, store) = match op {
                4 => (s, true),
                5 => (self.add(d, s, 0, sz), true),
                6 => (self.add(d, s, carry, sz), true),
                7 => (self.add(d, !s & sz.mask(), carry, sz), true),
                8 => (self.add(d, !s & sz.mask(), 1, sz), true),
                9 => (self.add(d, !s & sz.mask(), 1, sz), false),
                10 => (self.dadd(d, s, sz), true),
                11 => {
                    self.logic_flags(d & s, sz, false);
                    (0, false)
                }
                12 => (d & !s, true),
                13 => (d | s, true),
                14 => {
                    let r = d ^ s;
                    self.logic_flags(r, sz, d & s & sz.msb() != 0);
                    (r, true)
                }
                _ => {
                    let r = d & s;
                    self.logic_flags(r, sz, false);
                    (r, true)
                }
            };
            if store {
                self.put(bus, dst, sz, r)?;
            }
        }
        Ok(())
    }

    /// Format II: `RRC`, `SWPB`, `RRA`, `SXT`, `PUSH`, `CALL`, `RETI` and the
    /// CPUX `CALLA` forms that share the `0x13xx` opcode space.
    fn exec_single(
        &mut self,
        bus: &mut dyn Bus,
        w: u32,
        ext: Option<Ext>,
        at: u32,
    ) -> SimResult<()> {
        let undefined = || SimulationError::DecodeError(u64::from(at));
        let op = (w >> 7) & 7;
        let mode = (w >> 4) & 3;
        let reg = (w & 0xF) as usize;
        if op >= 6 {
            if ext.is_some() {
                return Err(undefined());
            }
            if w == 0x1300 {
                let sr = self.pop(bus, Sz::W)?;
                let pc = self.pop(bus, Sz::W)?;
                self.r[SR] = sr & SR_MASK;
                let hi = if self.cpux { sr >> 12 } else { 0 };
                self.r[PC] = (hi << 16 | pc) & !1;
                self.returned = true;
                return Ok(());
            }
            if !self.cpux {
                return Err(undefined());
            }
            return self.exec_calla(bus, w, at);
        }
        let (sz, count, zc) =
            self.extended_size(ext, w & 0x40 != 0, mode == 0, op == 1 || op == 3, at)?;
        // A single operand takes its upper address bits from the source field.
        let hi = ext.map(|e| if mode == 0 { 0 } else { e.src_hi() });
        let loc = self.src_loc(bus, mode, reg, sz, hi)?;
        match op {
            // RRC, RRA
            0 | 2 => {
                for _ in 0..count {
                    let d = self.get(bus, loc, sz)?;
                    let top = if op == 2 {
                        d & sz.msb()
                    } else if zc || !self.flag(SR_C) {
                        0
                    } else {
                        sz.msb()
                    };
                    let r = d >> 1 | top;
                    self.put_flag(SR_C, d & 1 != 0);
                    self.put_flag(SR_V, false);
                    self.set_nz(r, sz);
                    self.put(bus, loc, sz, r)?;
                }
            }
            // SWPB: exchange the two low bytes; bits 19:16 survive only in
            // the address-word form.
            1 => {
                let d = self.get(bus, loc, sz)?;
                let r = (d & 0xF_0000) | (d & 0xFF) << 8 | (d >> 8) & 0xFF;
                self.put(bus, loc, sz, r)?;
            }
            // SXT: the sign of bit 7 fills a register up to bit 19 on CPUX
            // and a memory word up to bit 15.
            3 => {
                let d = self.get(bus, loc, sz)?;
                let wide = d as u8 as i8 as i32 as u32;
                let out = match loc {
                    Loc::Reg(_) if self.cpux => Sz::A,
                    _ => sz,
                };
                self.logic_flags(wide, out, false);
                self.put(bus, loc, out, wide)?;
            }
            4 => {
                let v = self.get(bus, loc, sz)?;
                self.push(bus, sz, v)?;
            }
            _ => {
                if ext.is_some() {
                    return Err(undefined());
                }
                let target = self.get(bus, loc, Sz::W)?;
                let ret = self.r[PC];
                self.push(bus, Sz::W, ret & 0xFFFF)?;
                self.r[PC] = target & !1;
            }
        }
        Ok(())
    }

    /// `CALLA`: push the 20-bit return address and jump.
    fn exec_calla(&mut self, bus: &mut dyn Bus, w: u32, at: u32) -> SimResult<()> {
        let low = w & 0xF;
        let target = match (w >> 4) & 0xF {
            // CALLA Rdst / x(Rdst) / @Rdst / @Rdst+
            sub @ 4..=7 => {
                let loc = self.src_loc(bus, sub & 3, low as usize, Sz::A, None)?;
                self.get(bus, loc, Sz::A)?
            }
            // CALLA &abs20
            8 => {
                let addr = low << 16 | self.fetch(bus)?;
                self.rd(bus, addr, Sz::A)?
            }
            // CALLA x(PC), 20-bit index
            9 => {
                let base = self.r[PC];
                let index = self.fetch(bus)?;
                let addr = self.indexed(base, index, Some(low));
                self.rd(bus, addr, Sz::A)?
            }
            // CALLA #imm20
            11 => low << 16 | self.fetch(bus)?,
            _ => return Err(SimulationError::DecodeError(u64::from(at))),
        };
        let ret = self.r[PC];
        self.push(bus, Sz::A, ret)?;
        self.r[PC] = target & !1;
        Ok(())
    }

    /// `PUSHM`/`POPM`: `n` registers, highest first on the way down.
    fn exec_pushm_popm(&mut self, bus: &mut dyn Bus, w: u32) -> SimResult<()> {
        let sz = if w & 0x0100 != 0 { Sz::W } else { Sz::A };
        let n = ((w >> 4) & 0xF) + 1;
        let reg = w & 0xF;
        for i in 0..n {
            if w & 0x0200 == 0 {
                let v = self.r[((reg.wrapping_sub(i)) & 0xF) as usize] & sz.mask();
                self.push(bus, sz, v)?;
            } else {
                let v = self.pop(bus, sz)?;
                self.set_reg(((reg + i) & 0xF) as usize, v);
            }
        }
        Ok(())
    }

    /// The CPUX address instructions and the `RRCM`-family register shifts.
    fn exec_address(&mut self, bus: &mut dyn Bus, w: u32) -> SimResult<()> {
        let hi = ((w >> 8) & 0xF) as usize;
        let lo = (w & 0xF) as usize;
        let s = match (w >> 4) & 0xF {
            0 => {
                let v = self.rd(bus, self.r[hi], Sz::A)?;
                self.set_reg(lo, v);
                return Ok(());
            }
            1 => {
                let addr = self.r[hi];
                self.r[hi] = addr.wrapping_add(4) & self.addr_mask();
                let v = self.rd(bus, addr, Sz::A)?;
                self.set_reg(lo, v);
                return Ok(());
            }
            2 => {
                let addr = (hi as u32) << 16 | self.fetch(bus)?;
                let v = self.rd(bus, addr, Sz::A)?;
                self.set_reg(lo, v);
                return Ok(());
            }
            3 => {
                let base = self.r[hi];
                let index = sext16(self.fetch(bus)?);
                let v = self.rd(bus, base.wrapping_add(index), Sz::A)?;
                self.set_reg(lo, v);
                return Ok(());
            }
            4 | 5 => {
                self.shift_multi(w);
                return Ok(());
            }
            6 => {
                let addr = (lo as u32) << 16 | self.fetch(bus)?;
                return self.wr(bus, addr, Sz::A, self.r[hi]);
            }
            7 => {
                let base = self.r[lo];
                let index = sext16(self.fetch(bus)?);
                return self.wr(bus, base.wrapping_add(index), Sz::A, self.r[hi]);
            }
            8..=11 => (hi as u32) << 16 | self.fetch(bus)?,
            _ => self.r[hi],
        };
        let d = self.r[lo];
        match (w >> 4) & 3 {
            0 => self.set_reg(lo, s),
            1 => {
                self.add(d, !s & 0xF_FFFF, 1, Sz::A);
            }
            2 => {
                let r = self.add(d, s, 0, Sz::A);
                self.set_reg(lo, r);
            }
            _ => {
                let r = self.add(d, !s & 0xF_FFFF, 1, Sz::A);
                self.set_reg(lo, r);
            }
        }
        Ok(())
    }

    /// `RRCM`/`RRAM`/`RLAM`/`RRUM`: shift a register by 1 to 4 bits.
    fn shift_multi(&mut self, w: u32) {
        let sz = if w & 0x10 != 0 { Sz::W } else { Sz::A };
        let reg = (w & 0xF) as usize;
        let mut v = self.r[reg] & sz.mask();
        for _ in 0..((w >> 10) & 3) + 1 {
            let carry_in = self.flag(SR_C);
            match (w >> 8) & 3 {
                0 => {
                    self.put_flag(SR_C, v & 1 != 0);
                    v = v >> 1 | if carry_in { sz.msb() } else { 0 };
                }
                1 => {
                    self.put_flag(SR_C, v & 1 != 0);
                    v = v >> 1 | v & sz.msb();
                }
                2 => {
                    self.put_flag(SR_C, v & sz.msb() != 0);
                    v = v << 1 & sz.mask();
                }
                _ => {
                    self.put_flag(SR_C, v & 1 != 0);
                    v >>= 1;
                }
            }
        }
        self.put_flag(SR_V, false);
        self.set_nz(v, sz);
        self.set_reg(reg, v);
    }

    fn observer_registers(&self) -> [u32; 18] {
        let mut regs = [0u32; 18];
        regs[..16].copy_from_slice(&self.r);
        regs[16] = self.r[SP];
        regs[17] = self.r[PC];
        regs
    }
}

impl Cpu for Msp430 {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        self.r = [0; 16];
        self.requested = 0;
        self.r[PC] = u32::from(bus.read_u16(u64::from(RESET_VECTOR))?) & !1;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &crate::SimulationConfig,
    ) -> SimResult<()> {
        if self.flag(SR_CPUOFF) {
            self.cycles = self.cycles.wrapping_add(1);
            self.take_interrupt(bus)?;
            return Ok(());
        }
        let pc = self.r[PC];
        let opcode = if observers.is_empty() {
            0
        } else {
            let op = u32::from(bus.read_u16(u64::from(pc))?);
            for o in observers {
                o.on_step_start(pc, op);
            }
            op
        };
        let gie_was_clear = !self.flag(SR_GIE);
        self.returned = false;
        if let Err(e) = self.exec(bus, pc) {
            self.r[PC] = pc;
            return Err(e);
        }
        self.cycles = self.cycles.wrapping_add(1);
        if !observers.is_empty() {
            crate::emit_trace_event(
                observers,
                labwired_hw_trace::TraceEvent::InstructionRetired { pc, opcode },
            );
        }
        // The instruction after one that sets GIE always executes first
        // (SLAU049 3.4.6, EINT). RETI and an entry into a low-power mode
        // accept a pending request at once.
        let eint_delay =
            gie_was_clear && self.flag(SR_GIE) && !self.flag(SR_CPUOFF) && !self.returned;
        if !eint_delay {
            self.take_interrupt(bus)?;
        }
        if !observers.is_empty() {
            let regs = self.observer_registers();
            for o in observers {
                o.on_step_end(1, &regs);
            }
        }
        Ok(())
    }

    /// The loader must not fall back to Cortex-M vector tables;
    /// [`crate::system::msp430::load_elf`] starts the core at the reset
    /// vector.
    fn uses_elf_entry_point(&self) -> bool {
        true
    }

    fn set_pc(&mut self, val: u32) {
        self.r[PC] = val & self.addr_mask() & !1;
    }
    fn get_pc(&self) -> u32 {
        self.r[PC]
    }
    fn set_sp(&mut self, val: u32) {
        self.r[SP] = val & self.addr_mask() & !1;
    }

    /// A peripheral requests interrupt line `n` (vector `0xFFFE - 2n`).
    fn set_exception_pending(&mut self, exception_num: u32) {
        if exception_num < 64 {
            self.requested |= 1 << exception_num;
        }
    }

    fn idle_fast_forward_budget(&self, _bus: &dyn Bus) -> Option<u64> {
        (self.flag(SR_CPUOFF) && self.acceptable_line().is_none()).then_some(u64::MAX)
    }

    fn fast_forward_idle_cycles(&mut self, cycles: u64) {
        self.cycles = self.cycles.wrapping_add(cycles);
    }

    fn get_register(&self, id: u8) -> u32 {
        self.r.get(id as usize).copied().unwrap_or(0)
    }
    fn set_register(&mut self, id: u8, val: u32) {
        if (id as usize) < 16 {
            self.set_reg(id as usize, val);
        }
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::Msp430(Box::new(Msp430CpuSnapshot {
            r: self.r.to_vec(),
            cpux: self.cpux,
            requested: self.requested,
            cycles: self.cycles,
        }))
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::Msp430(s) = snapshot {
            self.cpux = s.cpux;
            for (i, v) in s.r.iter().take(16).enumerate() {
                self.set_reg(i, *v);
            }
            self.requested = s.requested;
            self.cycles = s.cycles;
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names = vec!["pc".to_string(), "sp".to_string(), "sr".to_string()];
        names.extend((3..16).map(|i| format!("r{i}")));
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        let n = name.to_ascii_lowercase();
        match n.as_str() {
            "pc" => Some(0),
            "sp" => Some(1),
            "sr" => Some(2),
            _ => n
                .strip_prefix('r')
                .and_then(|d| d.parse().ok())
                .filter(|d: &u8| *d < 16),
        }
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "msp430_tests.rs"]
mod tests;
