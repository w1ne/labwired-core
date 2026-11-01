// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RV64 machine-mode hart: RV64IMAC + Zicsr + Zifencei, with the SiFive-style
//! CLINT (`msip` / `mtimecmp` / `mtime`) folded into the core.
//!
//! This is a separate core from [`crate::cpu::RiscV`] rather than an XLEN
//! switch inside it: the RV32 core's registers, decode cache, spin windows
//! and wasm JIT are all 32-bit, and they are on the perf-gated hot path of
//! every ESP32-C3/C6 board. Widening them would tax those boards for a part
//! that does not share their memory map.
//!
//! What is modelled, and where it comes from (RISC-V Unprivileged ISA
//! 20191213 and Privileged Architecture 20211203):
//!
//! * **RV64I** with the `*W` 32-bit ops (sign-extended results), `LWU`/`LD`/
//!   `SD`, 6-bit shift amounts; **M** (`MUL[H[S]U]`, `DIV[U]`, `REM[U]` and the
//!   `*W` forms, spec division-by-zero / overflow results); **A** (`LR`/`SC`
//!   and every AMO, `.W` and `.D`); **C** for RV64 (`C.LD`/`C.SD`/`C.LDSP`/
//!   `C.SDSP`/`C.ADDIW`/`C.SUBW`/`C.ADDW`, 6-bit `C.SLLI`/`C.SRLI`/`C.SRAI`).
//! * **M-mode only.** `misa` reports exactly `I M A C` with MXL=2. `mstatus`
//!   keeps MIE/MPIE, MPP reads back M (the only privilege), and SXL/UXL/FS/MPRV
//!   are zero because S, U and F are not implemented.
//! * Traps: illegal instruction (cause 2, `mtval` = the instruction bits),
//!   breakpoint (3), misaligned load/store/AMO (4/6, `mtval` = address; the
//!   SiFive U54 does not do misaligned accesses in hardware), `ECALL` from M
//!   (11). Interrupts MEI > MSI > MTI, direct or vectored `mtvec`.
//! * CLINT at a chip-chosen base: `msip[hart]` at +0x0, `mtimecmp[hart]` at
//!   +0x4000 + 8·hart, `mtime` at +0xBFF8. `mtime` runs at the chip's RTC
//!   reference, a fixed number of CPU clocks per tick; one retired
//!   instruction is one CPU clock. No `time` CSR: the U54 does not implement
//!   it (rdtime traps and is emulated by M-mode firmware on silicon).
//! * `mcycle`/`minstret` (and their `cycle`/`instret` user aliases), 16 PMP
//!   entries as plain WARL storage (no enforcement), `mhartid` from the chip.
//!
//! Not modelled yet, and refused rather than guessed: the F/D extensions
//! (any FP opcode is an illegal instruction), S/U modes and Sv39, the PLIC
//! (`MEIP` is never set), and the other harts of a multi-hart complex.

use crate::{Bus, Cpu, SimResult, SimulationObserver};
use std::sync::Arc;

/// CPU clocks per CLINT `mtime` tick when the chip does not say otherwise.
pub const DEFAULT_MTIME_DIVIDER: u64 = 1;

const MSTATUS_MIE: u64 = 1 << 3;
const MSTATUS_MPIE: u64 = 1 << 7;
const MSTATUS_MPP: u64 = 0b11 << 11;

const MIP_MSIP: u64 = 1 << 3;
const MIP_MTIP: u64 = 1 << 7;
const MIP_MEIP: u64 = 1 << 11;
/// The machine-level interrupt bits that exist with no S-mode.
const MIE_MASK: u64 = MIP_MSIP | MIP_MTIP | MIP_MEIP;

/// `misa`: MXL=2 (64-bit), extensions A, C, I, M.
const MISA_RV64IMAC: u64 =
    (2 << 62) | (1 << 0) /* A */ | (1 << 2) /* C */ | (1 << 8) /* I */ | (1 << 12) /* M */;

const CAUSE_ILLEGAL: u64 = 2;
const CAUSE_BREAKPOINT: u64 = 3;
const CAUSE_LOAD_MISALIGNED: u64 = 4;
const CAUSE_STORE_MISALIGNED: u64 = 6;
const CAUSE_ECALL_M: u64 = 11;
const INTERRUPT: u64 = 1 << 63;

const PMP_ENTRIES: usize = 16;

const CLINT_SIZE: u64 = 0x1_0000;
const CLINT_MTIMECMP: u64 = 0x4000;
const CLINT_MTIME: u64 = 0xBFF8;

/// The core-local interruptor, as seen by one hart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Clint {
    /// Base address of the CLINT block, or `None` on a chip without one.
    pub base: Option<u64>,
    pub msip: bool,
    pub mtimecmp: u64,
    pub mtime: u64,
    /// CPU clocks per `mtime` tick (CPU clock / RTC reference clock).
    pub divider: u64,
    /// CPU clocks accumulated toward the next `mtime` tick.
    pub residue: u64,
}

impl Clint {
    fn new(base: Option<u64>, divider: u64) -> Self {
        Self {
            base,
            msip: false,
            // Reset value of mtimecmp is unspecified; all-ones keeps MTIP low
            // until firmware arms it, as every SiFive-derived CLINT driver
            // assumes.
            mtimecmp: u64::MAX,
            mtime: 0,
            divider: divider.max(1),
            residue: 0,
        }
    }

    fn advance(&mut self, cycles: u64) {
        let total = self.residue + cycles;
        self.mtime = self.mtime.wrapping_add(total / self.divider);
        self.residue = total % self.divider;
    }

    fn mtip(&self) -> bool {
        self.mtime >= self.mtimecmp
    }

    /// CPU clocks until `mtime` first reaches `mtimecmp`, `None` if it already has.
    fn cycles_until_mtip(&self) -> Option<u64> {
        if self.mtip() {
            return None;
        }
        let ticks = self.mtimecmp - self.mtime;
        Some(
            ticks
                .saturating_mul(self.divider)
                .saturating_sub(self.residue),
        )
    }
}

/// Snapshot of every architectural register of a [`RiscV64`] hart.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RiscV64CpuSnapshot {
    pub x: Vec<u64>,
    pub pc: u64,
    pub mstatus: u64,
    pub mie: u64,
    pub mtvec: u64,
    pub mscratch: u64,
    pub mepc: u64,
    pub mcause: u64,
    pub mtval: u64,
    pub mcycle: u64,
    pub minstret: u64,
    pub pmpcfg: Vec<u8>,
    pub pmpaddr: Vec<u64>,
    pub clint: Clint,
    pub reservation: Option<u64>,
}

#[derive(Debug)]
pub struct RiscV64 {
    pub x: [u64; 32],
    pub pc: u64,
    pub hart_id: u64,

    pub mstatus: u64,
    pub mie: u64,
    pub mtvec: u64,
    pub mscratch: u64,
    pub mepc: u64,
    pub mcause: u64,
    pub mtval: u64,
    pub mcycle: u64,
    pub minstret: u64,
    pub pmpcfg: [u8; PMP_ENTRIES],
    pub pmpaddr: [u64; PMP_ENTRIES],

    pub clint: Clint,

    /// Outstanding LR reservation (address). Any store by this hart to the
    /// reserved doubleword, and every SC, clears it.
    pub reservation: Option<u64>,
    waiting_for_interrupt: bool,
}

impl Default for RiscV64 {
    fn default() -> Self {
        Self::new(0, None, DEFAULT_MTIME_DIVIDER)
    }
}

/// Outcome of executing one instruction.
enum Flow {
    /// Fall through to `pc + len`.
    Next,
    /// Continue at this address.
    Jump(u64),
    /// A synchronous exception: (cause, mtval). PC stays on the instruction.
    Trap(u64, u64),
}

#[inline]
fn sext32(v: u64) -> u64 {
    v as u32 as i32 as i64 as u64
}

#[inline]
fn bits(v: u32, hi: u32, lo: u32) -> u32 {
    (v >> lo) & ((1u32 << (hi - lo + 1)) - 1)
}

/// Sign-extend the low `n` bits of `v`.
#[inline]
fn sext(v: u32, n: u32) -> u64 {
    (((v as i64) << (64 - n)) >> (64 - n)) as u64
}

impl RiscV64 {
    /// A hart with the given `mhartid`, CLINT base and CPU clocks per `mtime` tick.
    pub fn new(hart_id: u64, clint_base: Option<u64>, mtime_divider: u64) -> Self {
        Self {
            x: [0; 32],
            pc: 0,
            hart_id,
            mstatus: MSTATUS_MPP,
            mie: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mcycle: 0,
            minstret: 0,
            pmpcfg: [0; PMP_ENTRIES],
            pmpaddr: [0; PMP_ENTRIES],
            clint: Clint::new(clint_base, mtime_divider),
            reservation: None,
            waiting_for_interrupt: false,
        }
    }

    /// The hart a chip descriptor names: `mhartid`, CLINT base and `mtime`
    /// rate per [`Rv64Params::for_chip`].
    pub fn for_chip(chip: &labwired_config::ChipDescriptor) -> Self {
        let p = Rv64Params::for_chip(chip);
        Self::new(p.hart_id, p.clint_base, p.mtime_divider)
    }

    #[inline]
    fn reg(&self, n: u32) -> u64 {
        self.x[n as usize]
    }

    #[inline]
    fn set_reg(&mut self, n: u32, v: u64) {
        if n != 0 {
            self.x[n as usize] = v;
        }
    }

    fn mip(&self) -> u64 {
        let mut mip = 0;
        if self.clint.msip {
            mip |= MIP_MSIP;
        }
        if self.clint.mtip() {
            mip |= MIP_MTIP;
        }
        mip
    }

    fn clint_offset(&self, addr: u64) -> Option<u64> {
        let base = self.clint.base?;
        addr.checked_sub(base).filter(|off| *off < CLINT_SIZE)
    }

    /// CLINT register read, for an access of `size` bytes at `off`.
    fn clint_read(&self, off: u64, size: u64) -> u64 {
        let msip_off = 4 * self.hart_id;
        let cmp_off = CLINT_MTIMECMP + 8 * self.hart_id;
        let (reg_off, value) = if (msip_off..msip_off + 4).contains(&off) {
            (msip_off, u64::from(self.clint.msip))
        } else if (cmp_off..cmp_off + 8).contains(&off) {
            (cmp_off, self.clint.mtimecmp)
        } else if (CLINT_MTIME..CLINT_MTIME + 8).contains(&off) {
            (CLINT_MTIME, self.clint.mtime)
        } else {
            // Other harts' msip/mtimecmp: not modelled, read as zero.
            return 0;
        };
        let shift = (off - reg_off) * 8;
        let v = value >> shift;
        if size >= 8 {
            v
        } else {
            v & ((1u64 << (size * 8)) - 1)
        }
    }

    fn clint_write(&mut self, off: u64, size: u64, value: u64) {
        let merge = |old: u64, reg_off: u64| -> u64 {
            let shift = (off - reg_off) * 8;
            let mask = if size >= 8 {
                u64::MAX
            } else {
                ((1u64 << (size * 8)) - 1) << shift
            };
            (old & !mask) | ((value << shift) & mask)
        };
        let msip_off = 4 * self.hart_id;
        let cmp_off = CLINT_MTIMECMP + 8 * self.hart_id;
        if (msip_off..msip_off + 4).contains(&off) {
            if off == msip_off {
                self.clint.msip = value & 1 != 0;
            }
        } else if (cmp_off..cmp_off + 8).contains(&off) {
            self.clint.mtimecmp = merge(self.clint.mtimecmp, cmp_off);
        } else if (CLINT_MTIME..CLINT_MTIME + 8).contains(&off) {
            self.clint.mtime = merge(self.clint.mtime, CLINT_MTIME);
        }
    }

    fn load(&mut self, bus: &mut dyn Bus, addr: u64, size: u64) -> SimResult<u64> {
        if let Some(off) = self.clint_offset(addr) {
            return Ok(self.clint_read(off, size));
        }
        Ok(match size {
            1 => u64::from(bus.read_u8(addr)?),
            2 => u64::from(bus.read_u16(addr)?),
            4 => u64::from(bus.read_u32(addr)?),
            _ => bus.read_u64(addr)?,
        })
    }

    fn store(&mut self, bus: &mut dyn Bus, addr: u64, size: u64, value: u64) -> SimResult<()> {
        if let Some(r) = self.reservation {
            if addr < r + 8 && r < addr + size {
                self.reservation = None;
            }
        }
        if let Some(off) = self.clint_offset(addr) {
            self.clint_write(off, size, value);
            return Ok(());
        }
        match size {
            1 => bus.write_u8(addr, value as u8),
            2 => bus.write_u16(addr, value as u16),
            4 => bus.write_u32(addr, value as u32),
            _ => bus.write_u64(addr, value),
        }
    }

    fn read_csr(&self, csr: u32) -> Option<u64> {
        Some(match csr {
            0x300 => self.mstatus,
            0x301 => MISA_RV64IMAC,
            0x304 => self.mie,
            0x305 => self.mtvec,
            0x340 => self.mscratch,
            0x341 => self.mepc,
            0x342 => self.mcause,
            0x343 => self.mtval,
            0x344 => self.mip(),
            // RV64: only the even pmpcfg registers exist, 8 entries each.
            0x3A0 | 0x3A2 => {
                let first = ((csr - 0x3A0) * 4) as usize;
                let mut v = 0u64;
                for i in 0..8 {
                    v |= u64::from(self.pmpcfg[first + i]) << (8 * i);
                }
                v
            }
            0x3B0..=0x3BF => self.pmpaddr[(csr - 0x3B0) as usize],
            0x320 => 0,         // mcountinhibit: counters never inhibited
            0x323..=0x33F => 0, // mhpmevent3..31: no events selectable
            0xB00 | 0xC00 => self.mcycle,
            0xB02 | 0xC02 => self.minstret,
            0xB03..=0xB1F | 0xC03..=0xC1F => 0, // hpmcounters: hardwired zero
            0xF11..=0xF13 => 0,                 // mvendorid/marchid/mimpid: not modelled
            0xF14 => self.hart_id,
            0xF15 => 0, // mconfigptr
            _ => return None,
        })
    }

    /// Write a CSR. `false` means the CSR does not exist or is read-only.
    fn write_csr(&mut self, csr: u32, v: u64) -> bool {
        if csr >> 10 == 0b11 {
            return false; // read-only space (Privileged §2.1)
        }
        match csr {
            // MPP is WARL and M is the only legal value without U/S-mode.
            0x300 => self.mstatus = (v & (MSTATUS_MIE | MSTATUS_MPIE)) | MSTATUS_MPP,
            0x301 => {} // misa: WARL, extensions fixed
            0x304 => self.mie = v & MIE_MASK,
            // Mode 0 (direct) and 1 (vectored) only; base 4-byte aligned.
            0x305 => self.mtvec = v & !0b10,
            0x340 => self.mscratch = v,
            0x341 => self.mepc = v & !1,
            0x342 => self.mcause = v,
            0x343 => self.mtval = v,
            0x344 => {} // every M-level pending bit is driven by the CLINT/PLIC
            0x3A0 | 0x3A2 => {
                let first = ((csr - 0x3A0) * 4) as usize;
                for i in 0..8 {
                    // A locked entry ignores writes until reset.
                    if self.pmpcfg[first + i] & 0x80 == 0 {
                        // R=0,W=1 is reserved: WARL to no access.
                        let mut b = (v >> (8 * i)) as u8 & 0x9F;
                        if b & 0b11 == 0b10 {
                            b &= !0b11;
                        }
                        self.pmpcfg[first + i] = b;
                    }
                }
            }
            0x3B0..=0x3BF => {
                let i = (csr - 0x3B0) as usize;
                if self.pmpcfg[i] & 0x80 == 0 {
                    self.pmpaddr[i] = v & ((1 << 54) - 1);
                }
            }
            0x320 | 0x323..=0x33F => {}
            0xB00 => self.mcycle = v,
            0xB02 => self.minstret = v,
            0xB03..=0xB1F => {}
            _ => return false,
        }
        true
    }

    fn take_trap(&mut self, cause: u64, epc: u64, tval: u64) {
        self.mepc = epc;
        self.mcause = cause;
        self.mtval = tval;
        let mie = self.mstatus & MSTATUS_MIE;
        self.mstatus &= !(MSTATUS_MIE | MSTATUS_MPIE);
        if mie != 0 {
            self.mstatus |= MSTATUS_MPIE;
        }
        self.mstatus |= MSTATUS_MPP;
        let base = self.mtvec & !0b11;
        self.pc = if self.mtvec & 1 == 1 && cause & INTERRUPT != 0 {
            base.wrapping_add(4 * (cause & !INTERRUPT))
        } else {
            base
        };
        self.reservation = None;
    }

    fn pending_interrupt(&self) -> Option<u64> {
        if self.mstatus & MSTATUS_MIE == 0 {
            return None;
        }
        let pending = self.mip() & self.mie;
        [11u64, 3, 7]
            .into_iter()
            .find(|&irq| pending & (1 << irq) != 0)
    }

    fn fetch(&mut self, bus: &mut dyn Bus) -> SimResult<(u32, u64)> {
        let lo = u32::from(bus.read_u16(self.pc)?);
        if lo & 0b11 != 0b11 {
            return Ok((lo, 2));
        }
        let hi = u32::from(bus.read_u16(self.pc.wrapping_add(2))?);
        Ok((lo | (hi << 16), 4))
    }

    /// Execute one instruction already fetched.
    fn exec(&mut self, bus: &mut dyn Bus, inst: u32, len: u64) -> SimResult<Flow> {
        if len == 2 {
            return self.exec_compressed(bus, inst);
        }
        let opcode = inst & 0x7F;
        let rd = bits(inst, 11, 7);
        let rs1 = bits(inst, 19, 15);
        let rs2 = bits(inst, 24, 20);
        let f3 = bits(inst, 14, 12);
        let f7 = bits(inst, 31, 25);
        let imm_i = sext(inst >> 20, 12);
        let imm_s = sext((bits(inst, 31, 25) << 5) | rd, 12);
        let illegal = Flow::Trap(CAUSE_ILLEGAL, u64::from(inst));
        let a = self.reg(rs1);
        let b = self.reg(rs2);

        match opcode {
            0x37 => self.set_reg(rd, sext32(u64::from(inst & 0xFFFF_F000))),
            0x17 => self.set_reg(
                rd,
                self.pc.wrapping_add(sext32(u64::from(inst & 0xFFFF_F000))),
            ),
            0x6F => {
                let imm = (bits(inst, 31, 31) << 20)
                    | (bits(inst, 19, 12) << 12)
                    | (bits(inst, 20, 20) << 11)
                    | (bits(inst, 30, 21) << 1);
                let target = self.pc.wrapping_add(sext(imm, 21));
                self.set_reg(rd, self.pc.wrapping_add(4));
                return Ok(Flow::Jump(target));
            }
            0x67 if f3 == 0 => {
                let target = a.wrapping_add(imm_i) & !1;
                self.set_reg(rd, self.pc.wrapping_add(4));
                return Ok(Flow::Jump(target));
            }
            0x63 => {
                let take = match f3 {
                    0 => a == b,
                    1 => a != b,
                    4 => (a as i64) < (b as i64),
                    5 => (a as i64) >= (b as i64),
                    6 => a < b,
                    7 => a >= b,
                    _ => return Ok(illegal),
                };
                if take {
                    let imm = (bits(inst, 31, 31) << 12)
                        | (bits(inst, 7, 7) << 11)
                        | (bits(inst, 30, 25) << 5)
                        | (bits(inst, 11, 8) << 1);
                    return Ok(Flow::Jump(self.pc.wrapping_add(sext(imm, 13))));
                }
            }
            0x03 => {
                let addr = a.wrapping_add(imm_i);
                let (size, signed) = match f3 {
                    0 => (1, true),
                    1 => (2, true),
                    2 => (4, true),
                    3 => (8, false),
                    4 => (1, false),
                    5 => (2, false),
                    6 => (4, false),
                    _ => return Ok(illegal),
                };
                if addr % size != 0 {
                    return Ok(Flow::Trap(CAUSE_LOAD_MISALIGNED, addr));
                }
                let raw = self.load(bus, addr, size)?;
                let v = if signed && size < 8 {
                    let s = 64 - 8 * size as u32;
                    (((raw << s) as i64) >> s) as u64
                } else {
                    raw
                };
                self.set_reg(rd, v);
            }
            0x23 => {
                let addr = a.wrapping_add(imm_s);
                let size = match f3 {
                    0 => 1,
                    1 => 2,
                    2 => 4,
                    3 => 8,
                    _ => return Ok(illegal),
                };
                if addr % size != 0 {
                    return Ok(Flow::Trap(CAUSE_STORE_MISALIGNED, addr));
                }
                self.store(bus, addr, size, b)?;
            }
            0x13 => {
                let shamt = (inst >> 20) & 0x3F;
                let v = match f3 {
                    0 => a.wrapping_add(imm_i),
                    2 => u64::from((a as i64) < (imm_i as i64)),
                    3 => u64::from(a < imm_i),
                    4 => a ^ imm_i,
                    6 => a | imm_i,
                    7 => a & imm_i,
                    1 if inst >> 26 == 0 => a << shamt,
                    5 if inst >> 26 == 0 => a >> shamt,
                    5 if inst >> 26 == 0x10 => ((a as i64) >> shamt) as u64,
                    _ => return Ok(illegal),
                };
                self.set_reg(rd, v);
            }
            0x1B => {
                let shamt = rs2;
                let v = match (f3, f7) {
                    (0, _) => a.wrapping_add(imm_i),
                    (1, 0) => a << shamt,
                    (5, 0) => (a as u32 >> shamt) as u64,
                    (5, 0x20) => ((a as i32) >> shamt) as u64,
                    _ => return Ok(illegal),
                };
                self.set_reg(rd, sext32(v));
            }
            0x33 => {
                let v = match (f7, f3) {
                    (0, 0) => a.wrapping_add(b),
                    (0x20, 0) => a.wrapping_sub(b),
                    (0, 1) => a << (b & 0x3F),
                    (0, 2) => u64::from((a as i64) < (b as i64)),
                    (0, 3) => u64::from(a < b),
                    (0, 4) => a ^ b,
                    (0, 5) => a >> (b & 0x3F),
                    (0x20, 5) => ((a as i64) >> (b & 0x3F)) as u64,
                    (0, 6) => a | b,
                    (0, 7) => a & b,
                    (1, 0) => a.wrapping_mul(b),
                    (1, 1) => ((i128::from(a as i64) * i128::from(b as i64)) >> 64) as u64,
                    (1, 2) => ((i128::from(a as i64) * (b as i128)) >> 64) as u64,
                    (1, 3) => ((u128::from(a) * u128::from(b)) >> 64) as u64,
                    (1, 4) => {
                        let (a, b) = (a as i64, b as i64);
                        if b == 0 {
                            u64::MAX
                        } else {
                            a.wrapping_div(b) as u64
                        }
                    }
                    (1, 5) => a.checked_div(b).unwrap_or(u64::MAX),
                    (1, 6) => {
                        let (a, b) = (a as i64, b as i64);
                        if b == 0 {
                            a as u64
                        } else {
                            a.wrapping_rem(b) as u64
                        }
                    }
                    (1, 7) => {
                        if b == 0 {
                            a
                        } else {
                            a % b
                        }
                    }
                    _ => return Ok(illegal),
                };
                self.set_reg(rd, v);
            }
            0x3B => {
                let (a32, b32) = (a as u32, b as u32);
                let v = match (f7, f3) {
                    (0, 0) => a32.wrapping_add(b32),
                    (0x20, 0) => a32.wrapping_sub(b32),
                    (0, 1) => a32 << (b32 & 0x1F),
                    (0, 5) => a32 >> (b32 & 0x1F),
                    (0x20, 5) => ((a32 as i32) >> (b32 & 0x1F)) as u32,
                    (1, 0) => a32.wrapping_mul(b32),
                    (1, 4) => {
                        let (a, b) = (a32 as i32, b32 as i32);
                        if b == 0 {
                            u32::MAX
                        } else {
                            a.wrapping_div(b) as u32
                        }
                    }
                    (1, 5) => a32.checked_div(b32).unwrap_or(u32::MAX),
                    (1, 6) => {
                        let (a, b) = (a32 as i32, b32 as i32);
                        if b == 0 {
                            a as u32
                        } else {
                            a.wrapping_rem(b) as u32
                        }
                    }
                    (1, 7) => {
                        if b32 == 0 {
                            a32
                        } else {
                            a32 % b32
                        }
                    }
                    _ => return Ok(illegal),
                };
                self.set_reg(rd, sext32(u64::from(v)));
            }
            0x0F => match f3 {
                // FENCE / FENCE.I: one in-order hart, no caches modelled.
                0 | 1 => {}
                _ => return Ok(illegal),
            },
            0x2F => return self.exec_amo(bus, inst, rd, a, b, f3),
            0x73 => return self.exec_system(inst, rd, rs1, f3, a),
            _ => return Ok(illegal),
        }
        Ok(Flow::Next)
    }

    fn exec_amo(
        &mut self,
        bus: &mut dyn Bus,
        inst: u32,
        rd: u32,
        addr: u64,
        src: u64,
        f3: u32,
    ) -> SimResult<Flow> {
        let size = match f3 {
            2 => 4,
            3 => 8,
            _ => return Ok(Flow::Trap(CAUSE_ILLEGAL, u64::from(inst))),
        };
        let f5 = inst >> 27;
        if addr % size != 0 {
            // LR reports a load fault; SC and AMOs a store/AMO fault.
            let cause = if f5 == 0b00010 {
                CAUSE_LOAD_MISALIGNED
            } else {
                CAUSE_STORE_MISALIGNED
            };
            return Ok(Flow::Trap(cause, addr));
        }
        let widen = |v: u64| if size == 4 { sext32(v) } else { v };
        match f5 {
            0b00010 => {
                // LR
                if bits(inst, 24, 20) != 0 {
                    return Ok(Flow::Trap(CAUSE_ILLEGAL, u64::from(inst)));
                }
                let v = self.load(bus, addr, size)?;
                self.set_reg(rd, widen(v));
                self.reservation = Some(addr & !7);
            }
            0b00011 => {
                // SC
                if self.reservation == Some(addr & !7) {
                    self.store(bus, addr, size, src)?;
                    self.set_reg(rd, 0);
                } else {
                    self.set_reg(rd, 1);
                }
                self.reservation = None;
            }
            _ => {
                let old = widen(self.load(bus, addr, size)?);
                let s = widen(src);
                let new = match f5 {
                    0b00001 => s,
                    0b00000 => old.wrapping_add(s),
                    0b00100 => old ^ s,
                    0b01100 => old & s,
                    0b01000 => old | s,
                    0b10000 => (old as i64).min(s as i64) as u64,
                    0b10100 => (old as i64).max(s as i64) as u64,
                    0b11000 => {
                        if size == 4 {
                            u64::from((old as u32).min(s as u32))
                        } else {
                            old.min(s)
                        }
                    }
                    0b11100 => {
                        if size == 4 {
                            u64::from((old as u32).max(s as u32))
                        } else {
                            old.max(s)
                        }
                    }
                    _ => return Ok(Flow::Trap(CAUSE_ILLEGAL, u64::from(inst))),
                };
                self.store(bus, addr, size, new)?;
                self.set_reg(rd, old);
            }
        }
        Ok(Flow::Next)
    }

    fn exec_system(&mut self, inst: u32, rd: u32, rs1: u32, f3: u32, a: u64) -> SimResult<Flow> {
        let illegal = Flow::Trap(CAUSE_ILLEGAL, u64::from(inst));
        if f3 == 0 {
            return Ok(match inst {
                0x0000_0073 => Flow::Trap(CAUSE_ECALL_M, 0),
                0x0010_0073 => Flow::Trap(CAUSE_BREAKPOINT, self.pc),
                0x3020_0073 => {
                    // MRET: MIE <- MPIE, MPIE <- 1, privilege <- MPP (always M).
                    let mpie = self.mstatus & MSTATUS_MPIE != 0;
                    self.mstatus &= !MSTATUS_MIE;
                    if mpie {
                        self.mstatus |= MSTATUS_MIE;
                    }
                    self.mstatus |= MSTATUS_MPIE;
                    Flow::Jump(self.mepc)
                }
                0x1050_0073 => {
                    self.waiting_for_interrupt = true;
                    Flow::Next
                }
                _ => illegal,
            });
        }
        let csr = inst >> 20;
        let src = if f3 & 4 != 0 { u64::from(rs1) } else { a };
        // CSRRW[I] with rd=x0 does not read; CSRRS/C with a zero source does not write.
        // CSRRS/C with a zero source register (or immediate) do not write.
        let writes = f3 & 3 == 1 || rs1 != 0;
        // Reading a CSR here has no side effects, so the CSRRW rd=x0 "no read"
        // rule needs no special case; existence is checked either way.
        let Some(old) = self.read_csr(csr) else {
            return Ok(illegal);
        };
        if writes {
            let new = match f3 & 3 {
                1 => src,
                2 => old | src,
                3 => old & !src,
                _ => return Ok(illegal),
            };
            if !self.write_csr(csr, new) {
                return Ok(illegal);
            }
        }
        self.set_reg(rd, old);
        Ok(Flow::Next)
    }

    /// RVC for RV64 (Unprivileged ISA ch. 16, RV64C column).
    fn exec_compressed(&mut self, bus: &mut dyn Bus, c: u32) -> SimResult<Flow> {
        let illegal = Flow::Trap(CAUSE_ILLEGAL, u64::from(c));
        let op = c & 3;
        let f3 = bits(c, 15, 13);
        let rd = bits(c, 11, 7);
        let rs2 = bits(c, 6, 2);
        let rdp = bits(c, 4, 2) + 8;
        let rs1p = bits(c, 9, 7) + 8;
        let imm6 = (bits(c, 12, 12) << 5) | bits(c, 6, 2);
        // C.LD/C.SD offset: uimm[5:3|7:6]; C.LW/C.SW: uimm[5:3|2|6].
        let uimm_d = (bits(c, 12, 10) << 3) | (bits(c, 6, 5) << 6);
        let uimm_w = (bits(c, 12, 10) << 3) | (bits(c, 6, 6) << 2) | (bits(c, 5, 5) << 6);
        match (op, f3) {
            (0, 0) => {
                // C.ADDI4SPN
                let nz = (bits(c, 12, 11) << 4)
                    | (bits(c, 10, 7) << 6)
                    | (bits(c, 6, 6) << 2)
                    | (bits(c, 5, 5) << 3);
                if nz == 0 {
                    return Ok(illegal);
                }
                self.set_reg(rdp, self.reg(2).wrapping_add(u64::from(nz)));
            }
            (0, 2) => return self.c_load(bus, rdp, self.reg(rs1p), uimm_w, 4, c),
            (0, 3) => return self.c_load(bus, rdp, self.reg(rs1p), uimm_d, 8, c),
            (0, 6) => return self.c_store(bus, self.reg(rs1p), uimm_w, 4, self.reg(rdp)),
            (0, 7) => return self.c_store(bus, self.reg(rs1p), uimm_d, 8, self.reg(rdp)),
            (1, 0) => self.set_reg(rd, self.reg(rd).wrapping_add(sext(imm6, 6))), // C.ADDI / C.NOP
            (1, 1) => {
                // C.ADDIW (rd != 0)
                if rd == 0 {
                    return Ok(illegal);
                }
                self.set_reg(rd, sext32(self.reg(rd).wrapping_add(sext(imm6, 6))));
            }
            (1, 2) => self.set_reg(rd, sext(imm6, 6)), // C.LI
            (1, 3) => {
                if rd == 2 {
                    // C.ADDI16SP
                    let nz = (bits(c, 12, 12) << 9)
                        | (bits(c, 6, 6) << 4)
                        | (bits(c, 5, 5) << 6)
                        | (bits(c, 4, 3) << 7)
                        | (bits(c, 2, 2) << 5);
                    if nz == 0 {
                        return Ok(illegal);
                    }
                    self.set_reg(2, self.reg(2).wrapping_add(sext(nz, 10)));
                } else {
                    // C.LUI
                    if imm6 == 0 {
                        return Ok(illegal);
                    }
                    self.set_reg(rd, sext(imm6 << 12, 18));
                }
            }
            (1, 4) => {
                let r = rs1p;
                let v = self.reg(r);
                let shamt = imm6;
                match bits(c, 11, 10) {
                    0 => self.set_reg(r, v >> shamt),                   // C.SRLI
                    1 => self.set_reg(r, ((v as i64) >> shamt) as u64), // C.SRAI
                    2 => self.set_reg(r, v & sext(imm6, 6)),            // C.ANDI
                    _ => {
                        let s = self.reg(rdp);
                        let res = match (bits(c, 12, 12), bits(c, 6, 5)) {
                            (0, 0) => v.wrapping_sub(s),
                            (0, 1) => v ^ s,
                            (0, 2) => v | s,
                            (0, 3) => v & s,
                            (1, 0) => sext32(v.wrapping_sub(s)), // C.SUBW
                            (1, 1) => sext32(v.wrapping_add(s)), // C.ADDW
                            _ => return Ok(illegal),
                        };
                        self.set_reg(r, res);
                    }
                }
            }
            (1, 5) => {
                // C.J
                let imm = (bits(c, 12, 12) << 11)
                    | (bits(c, 11, 11) << 4)
                    | (bits(c, 10, 9) << 8)
                    | (bits(c, 8, 8) << 10)
                    | (bits(c, 7, 7) << 6)
                    | (bits(c, 6, 6) << 7)
                    | (bits(c, 5, 3) << 1)
                    | (bits(c, 2, 2) << 5);
                return Ok(Flow::Jump(self.pc.wrapping_add(sext(imm, 12))));
            }
            (1, 6) | (1, 7) => {
                // C.BEQZ / C.BNEZ
                let v = self.reg(rs1p);
                if (v == 0) == (f3 == 6) {
                    let imm = (bits(c, 12, 12) << 8)
                        | (bits(c, 11, 10) << 3)
                        | (bits(c, 6, 5) << 6)
                        | (bits(c, 4, 3) << 1)
                        | (bits(c, 2, 2) << 5);
                    return Ok(Flow::Jump(self.pc.wrapping_add(sext(imm, 9))));
                }
            }
            (2, 0) => self.set_reg(rd, self.reg(rd) << imm6), // C.SLLI
            (2, 2) => {
                // C.LWSP
                if rd == 0 {
                    return Ok(illegal);
                }
                let off = (bits(c, 12, 12) << 5) | (bits(c, 6, 4) << 2) | (bits(c, 3, 2) << 6);
                return self.c_load(bus, rd, self.reg(2), off, 4, c);
            }
            (2, 3) => {
                // C.LDSP
                if rd == 0 {
                    return Ok(illegal);
                }
                let off = (bits(c, 12, 12) << 5) | (bits(c, 6, 5) << 3) | (bits(c, 4, 2) << 6);
                return self.c_load(bus, rd, self.reg(2), off, 8, c);
            }
            (2, 4) => {
                let bit12 = bits(c, 12, 12);
                match (bit12, rd, rs2) {
                    (0, 0, 0) => return Ok(illegal),
                    (0, _, 0) => return Ok(Flow::Jump(self.reg(rd) & !1)), // C.JR
                    (0, _, _) => self.set_reg(rd, self.reg(rs2)),          // C.MV
                    (1, 0, 0) => return Ok(Flow::Trap(CAUSE_BREAKPOINT, self.pc)), // C.EBREAK
                    (1, _, 0) => {
                        // C.JALR
                        let target = self.reg(rd) & !1;
                        self.set_reg(1, self.pc.wrapping_add(2));
                        return Ok(Flow::Jump(target));
                    }
                    _ => self.set_reg(rd, self.reg(rd).wrapping_add(self.reg(rs2))), // C.ADD
                }
            }
            (2, 6) => {
                // C.SWSP
                let off = (bits(c, 12, 9) << 2) | (bits(c, 8, 7) << 6);
                return self.c_store(bus, self.reg(2), off, 4, self.reg(rs2));
            }
            (2, 7) => {
                // C.SDSP
                let off = (bits(c, 12, 10) << 3) | (bits(c, 9, 7) << 6);
                return self.c_store(bus, self.reg(2), off, 8, self.reg(rs2));
            }
            // C.FLD/C.FSD/C.FLDSP/C.FSDSP need D; reserved encodings.
            _ => return Ok(illegal),
        }
        Ok(Flow::Next)
    }

    fn c_load(
        &mut self,
        bus: &mut dyn Bus,
        rd: u32,
        base: u64,
        off: u32,
        size: u64,
        _c: u32,
    ) -> SimResult<Flow> {
        let addr = base.wrapping_add(u64::from(off));
        if addr % size != 0 {
            return Ok(Flow::Trap(CAUSE_LOAD_MISALIGNED, addr));
        }
        let v = self.load(bus, addr, size)?;
        self.set_reg(rd, if size == 4 { sext32(v) } else { v });
        Ok(Flow::Next)
    }

    fn c_store(
        &mut self,
        bus: &mut dyn Bus,
        base: u64,
        off: u32,
        size: u64,
        value: u64,
    ) -> SimResult<Flow> {
        let addr = base.wrapping_add(u64::from(off));
        if addr % size != 0 {
            return Ok(Flow::Trap(CAUSE_STORE_MISALIGNED, addr));
        }
        self.store(bus, addr, size, value)?;
        Ok(Flow::Next)
    }

    fn observer_registers(&self) -> [u32; 34] {
        let mut regs = [0u32; 34];
        for (dst, src) in regs.iter_mut().zip(self.x.iter()) {
            *dst = *src as u32;
        }
        regs[32] = self.x[2] as u32;
        regs[33] = self.pc as u32;
        regs
    }
}

/// Per-chip hart parameters.
///
/// A table keyed by chip name, as [`crate::cpu::Pic18::for_chip_name`] does,
/// rather than chip-YAML keys: the values are properties of the core complex,
/// not of a board, and there is one RV64 family so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rv64Params {
    pub hart_id: u64,
    pub clint_base: Option<u64>,
    pub mtime_divider: u64,
}

/// PolarFire SoC MSS (every MPFSxxxT): hart 0 is the E51 monitor core, harts
/// 1-4 the U54 application cores. This model runs the first U54, which is the
/// hart Zephyr's `mpfs_icicle/polarfire/u54` target boots on
/// (`CONFIG_RV_BOOT_HART=1`). CLINT at 0x0200_0000 (PolarFire SoC MSS TRM,
/// memory map; Zephyr `mpfs.dtsi` `clint@2000000`). `mtime` counts the 1 MHz
/// RTC reference (`CONFIG_SYS_CLOCK_HW_CYCLES_PER_SEC=1000000` on the board).
const MPFS_U54_HART: u64 = 1;
const MPFS_CLINT_BASE: u64 = 0x0200_0000;
const MPFS_MTIME_HZ: u64 = 1_000_000;

impl Rv64Params {
    pub fn for_chip(chip: &labwired_config::ChipDescriptor) -> Self {
        Self::for_chip_name(&chip.name, chip.cpu_hz)
    }

    pub fn for_chip_name(name: &str, cpu_hz: u64) -> Self {
        let name = name.to_ascii_lowercase();
        if name.starts_with("mpfs") {
            return Self {
                hart_id: MPFS_U54_HART,
                clint_base: Some(MPFS_CLINT_BASE),
                mtime_divider: (cpu_hz / MPFS_MTIME_HZ).max(1),
            };
        }
        Self {
            hart_id: 0,
            clint_base: None,
            mtime_divider: DEFAULT_MTIME_DIVIDER,
        }
    }
}

/// True when a chip descriptor names a 64-bit RISC-V core (`core: rv64…`).
pub fn is_rv64_chip(chip: &labwired_config::ChipDescriptor) -> bool {
    chip.arch == labwired_config::Arch::RiscV
        && chip
            .core
            .as_deref()
            .is_some_and(|c| c.trim().to_ascii_lowercase().starts_with("rv64"))
}

impl Cpu for RiscV64 {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        let (hart, base, div) = (self.hart_id, self.clint.base, self.clint.divider);
        let pc = self.pc;
        *self = Self::new(hart, base, div);
        self.pc = pc;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &crate::SimulationConfig,
    ) -> SimResult<()> {
        self.waiting_for_interrupt = false;
        let pc = self.pc;
        if pc & 1 != 0 {
            return Err(crate::SimulationError::DecodeError(pc));
        }
        let (inst, len) = self.fetch(bus)?;
        for o in observers {
            o.on_step_start(pc as u32, inst);
        }
        let flow = self.exec(bus, inst, len)?;
        match flow {
            Flow::Next => self.pc = pc.wrapping_add(len),
            Flow::Jump(t) => self.pc = t,
            Flow::Trap(cause, tval) => {
                if cause == CAUSE_ILLEGAL {
                    // The guest sees an architectural trap; the host log line
                    // is what `scripts/unsupported_instruction_audit.sh` counts.
                    tracing::warn!("Unknown instruction {:#x} at {:#x}", inst, pc);
                }
                self.take_trap(cause, pc, tval)
            }
        }
        self.mcycle = self.mcycle.wrapping_add(1);
        if !matches!(flow, Flow::Trap(..)) {
            self.minstret = self.minstret.wrapping_add(1);
        }
        self.clint.advance(1);
        if !observers.is_empty() {
            crate::emit_trace_event(
                observers,
                labwired_hw_trace::TraceEvent::InstructionRetired {
                    pc: pc as u32,
                    opcode: inst,
                },
            );
        }
        if let Some(irq) = self.pending_interrupt() {
            self.waiting_for_interrupt = false;
            self.take_trap(INTERRUPT | irq, self.pc, 0);
        }
        if !observers.is_empty() {
            let regs = self.observer_registers();
            for o in observers {
                o.on_step_end(1, &regs);
            }
        }
        Ok(())
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = u64::from(val);
    }
    fn get_pc(&self) -> u32 {
        self.pc as u32
    }
    fn set_sp(&mut self, val: u32) {
        self.x[2] = u64::from(val);
    }
    fn set_exception_pending(&mut self, _exception_num: u32) {
        // External interrupts arrive through a PLIC, which this core does not
        // model yet; nothing to pend.
    }

    fn idle_fast_forward_budget(&self, _bus: &dyn Bus) -> Option<u64> {
        if !self.waiting_for_interrupt || self.mip() & self.mie != 0 {
            return None;
        }
        if self.mie & MIP_MTIP == 0 {
            return Some(u64::MAX);
        }
        self.clint
            .cycles_until_mtip()
            .map(|c| c.saturating_sub(1))
            .filter(|c| *c > 0)
    }

    fn fast_forward_idle_cycles(&mut self, cycles: u64) {
        self.mcycle = self.mcycle.wrapping_add(cycles);
        self.clint.advance(cycles);
    }

    fn get_register(&self, id: u8) -> u32 {
        match id {
            0..=31 => self.x[id as usize] as u32,
            32 => self.pc as u32,
            _ => 0,
        }
    }
    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            1..=31 => self.x[id as usize] = u64::from(val),
            32 => self.pc = u64::from(val),
            _ => {}
        }
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::RiscV64(Box::new(RiscV64CpuSnapshot {
            x: self.x.to_vec(),
            pc: self.pc,
            mstatus: self.mstatus,
            mie: self.mie,
            mtvec: self.mtvec,
            mscratch: self.mscratch,
            mepc: self.mepc,
            mcause: self.mcause,
            mtval: self.mtval,
            mcycle: self.mcycle,
            minstret: self.minstret,
            pmpcfg: self.pmpcfg.to_vec(),
            pmpaddr: self.pmpaddr.to_vec(),
            clint: self.clint,
            reservation: self.reservation,
        }))
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::RiscV64(s) = snapshot {
            for (dst, src) in self.x.iter_mut().zip(s.x.iter()) {
                *dst = *src;
            }
            self.x[0] = 0;
            self.pc = s.pc;
            self.mstatus = (s.mstatus & (MSTATUS_MIE | MSTATUS_MPIE)) | MSTATUS_MPP;
            self.mie = s.mie & MIE_MASK;
            self.mtvec = s.mtvec;
            self.mscratch = s.mscratch;
            self.mepc = s.mepc;
            self.mcause = s.mcause;
            self.mtval = s.mtval;
            self.mcycle = s.mcycle;
            self.minstret = s.minstret;
            for (dst, src) in self.pmpcfg.iter_mut().zip(s.pmpcfg.iter()) {
                *dst = *src;
            }
            for (dst, src) in self.pmpaddr.iter_mut().zip(s.pmpaddr.iter()) {
                *dst = *src;
            }
            self.clint = s.clint;
            self.reservation = s.reservation;
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names: Vec<String> = (0..32).map(|i| format!("x{i}")).collect();
        names.push("pc".to_string());
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        if let Some(n) = name.strip_prefix('x') {
            n.parse().ok().filter(|n: &u8| *n < 32)
        } else if name.eq_ignore_ascii_case("pc") {
            Some(32)
        } else {
            None
        }
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "riscv64_tests.rs"]
mod tests;
