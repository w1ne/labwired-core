// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Renesas RX CPU (RXv3 instruction set, RX261 class) with the RX interrupt
//! controller unit (ICU) folded into the core.
//!
//! What is modelled, and where it comes from (RX Family RXv3 Instruction Set
//! Architecture User's Manual: Software, R01US0316; RX261 Group User's
//! Manual: Hardware, R01UH1045, ICUb chapter). Encodings were checked
//! against the Zephyr SDK 1.0.1 `rx-zephyr-elf` assembler (`--misa=v3`):
//!
//! * **Integer ISA**: every RXv1 instruction (all operand forms, including
//!   the `06` memory-extension prefix with `.B`/`.W`/`.L`/`.UW` sources and
//!   size-scaled displacements), the RXv2 additions (three-operand `XOR`,
//!   `STZ`/`STNZ Rs`, `MOVLI`/`MOVCO`, `UTOF`/`FTOU`/`FSQRT`, three-operand
//!   `FADD`/`FSUB`/`FMUL`) and the RXv3 `BFMOV`/`BFMOVZ`. Flags follow the
//!   manual's per-instruction O/S/Z/C tables; `C` after a subtract is "no
//!   borrow".
//! * **DSP**: the two 72-bit accumulators A0/A1 (`MULHI`/`MULLO`/`MULLH`,
//!   `MACHI`/`MACLO`/`MACLH`, `MSBHI`/`MSBLO`/`MSBLH`, `EMULA`/`EMACA`/`EMSBA`,
//!   `MVTACHI`/`MVTACLO`/`MVTACGU`, `MVFACHI`/`MVFACMI`/`MVFACLO`/`MVFACGU`
//!   with their `#imm` shift, `RACW`/`RACL`/`RDACW`/`RDACL`), `SAT`/`SATR`,
//!   `RMPA`.
//! * **Single-precision FPU** (the RX261 FPU): IEEE-754 binary32 results,
//!   `FTOI` truncating, `ROUND`/`ITOF`/`UTOF` honouring `FPSW.RM` only for
//!   round-to-nearest (the other modes are not modelled), `FCMP` setting
//!   Z (equal) / S (less) / O (unordered). FPSW is storage: no FP exception
//!   is ever raised and denormals are not flushed.
//! * **Control registers** PSW, PC, USP, ISP, INTB, EXTB, BPSW, BPC, FINTV,
//!   FPSW. R0 is the stack pointer selected by `PSW.U`; switching U swaps
//!   USP/ISP exactly as the manual describes.
//! * **Exceptions**: `INT #n` and `BRK` (INTB vectors, IPL unchanged),
//!   undefined instruction (EXTB + 0x5C, saved PC = the instruction), reset
//!   (the vector at 0xFFFFFFFC). Interrupt entry pushes PSW then PC on the
//!   ISP and clears I/U/PM; `RTE` pops them. The fast interrupt (`FIR`)
//!   saves to BPC/BPSW and returns with `RTFI`.
//! * **ICU** (RX261 base 0x00087000): `IRn` (+0x000), `DTCERn` (+0x100,
//!   storage), `IERm` (+0x200), `SWINTR`/`SWINT2R` (+0x2E0/+0x2E1, raising
//!   vectors 27/26), `FIR` (+0x2F0), `IPRn` (+0x300, the RX261 low-vector
//!   sharing: 16/18 → IPR000 … 31 → IPR007, ≥ 34 one IPR per vector). A
//!   peripheral requests vector `n` through [`Cpu::set_exception_pending`];
//!   that sets IRn, and the highest IPR among enabled requests above
//!   `PSW.IPL` is taken (lowest vector on a tie). Accepting a request clears
//!   its IR flag (edge detection); firmware can clear IR by writing 0.
//!
//! Not modelled yet, and refused rather than guessed: user-mode privilege
//! checks (Zephyr runs every thread in supervisor mode, `PSW.PM` = 0), the
//! RXv3 register-bank `SAVE`/`RSTR` and the double-precision FPU (absent
//! on the RX261; their opcodes are undefined instructions), DTC transfers,
//! group interrupts, and IRQ pin edge detection (`IRQCRn` is storage).

use crate::{Bus, Cpu, SimResult, SimulationObserver};
use std::sync::Arc;

const PSW_C: u32 = 1 << 0;
const PSW_Z: u32 = 1 << 1;
const PSW_S: u32 = 1 << 2;
const PSW_O: u32 = 1 << 3;
const PSW_I: u32 = 1 << 16;
const PSW_U: u32 = 1 << 17;
const PSW_PM: u32 = 1 << 20;
const PSW_IPL_SHIFT: u32 = 24;
const PSW_IPL_MASK: u32 = 0xF << PSW_IPL_SHIFT;
const PSW_WRITABLE: u32 = PSW_C | PSW_Z | PSW_S | PSW_O | PSW_I | PSW_U | PSW_PM | PSW_IPL_MASK;

/// Reset value of EXTB, the exception (fixed) vector table base.
const EXTB_RESET: u32 = 0xFFFF_FF80;
const RESET_VECTOR: u32 = 0xFFFF_FFFC;
/// Offsets into the exception vector table (RXv3 manual, Table 11.2).
const EXC_UNDEFINED: u32 = 0x5C;

/// ICU base on the RX100/200/600 parts with the ICUb/ICUe layout (RX261:
/// 0x00087000, Zephyr `rx261-common.dtsi` `icu`).
pub const RX_ICU_BASE: u32 = 0x0008_7000;
/// Covers IR/DTCER/IER/SWINTR/FIR/IPR, the IRQ pin and NMI registers and
/// the group interrupt block that follows them.
const ICU_SIZE: u32 = 0x700;
const ICU_IR: u32 = 0x000;
const ICU_IER: u32 = 0x200;
const ICU_SWINTR: u32 = 0x2E0;
const ICU_SWINT2R: u32 = 0x2E1;
const ICU_FIR: u32 = 0x2F0;
const ICU_IPR: u32 = 0x300;
const VEC_SWINT: u32 = 27;
const VEC_SWINT2: u32 = 26;

/// The interrupt controller registers the core consults on every step.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RxIcu {
    /// Raw register file (IR bytes, IER bits, IPR nibbles, FIR, and the
    /// registers held as plain storage).
    pub regs: Vec<u8>,
    /// IR flags as a bitset, mirrored from `regs` for a cheap "anything
    /// pending?" test.
    pub ir: [u64; 4],
}

impl Default for RxIcu {
    fn default() -> Self {
        Self {
            regs: vec![0; ICU_SIZE as usize],
            ir: [0; 4],
        }
    }
}

/// IPR register index for an interrupt vector (RX261 hardware manual,
/// Table 15.3; mirrored by Zephyr `arch/rx/core/irq_manage.c`). `None`
/// for vectors with no priority register: those never interrupt.
fn ipr_index(vec: u32) -> Option<u32> {
    Some(match vec {
        16 | 18 => 0,
        21 => 1,
        23 => 2,
        26 | 27 => 3,
        28..=31 => vec - 24,
        34..=255 => vec,
        _ => return None,
    })
}

impl RxIcu {
    fn set_ir(&mut self, vec: u32) {
        let v = (vec & 0xFF) as usize;
        self.regs[ICU_IR as usize + v] = 1;
        self.ir[v / 64] |= 1 << (v % 64);
    }

    fn clear_ir(&mut self, vec: u32) {
        let v = (vec & 0xFF) as usize;
        self.regs[ICU_IR as usize + v] = 0;
        self.ir[v / 64] &= !(1 << (v % 64));
    }

    fn enabled(&self, vec: u32) -> bool {
        self.regs[(ICU_IER + vec / 8) as usize] & (1 << (vec % 8)) != 0
    }

    fn priority(&self, vec: u32) -> u32 {
        ipr_index(vec).map_or(0, |i| u32::from(self.regs[(ICU_IPR + i) as usize] & 0xF))
    }

    fn fir(&self) -> u16 {
        u16::from_le_bytes([self.regs[ICU_FIR as usize], self.regs[ICU_FIR as usize + 1]])
    }

    /// The fast-interrupt vector, when FIR.FIE is set.
    fn fast_vector(&self) -> Option<u32> {
        let fir = self.fir();
        (fir & 0x8000 != 0).then_some(u32::from(fir & 0xFF))
    }

    /// The request the CPU would take next: `(vector, priority, fast)`.
    fn highest(&self) -> Option<(u32, u32, bool)> {
        let fast = self.fast_vector();
        let mut best: Option<(u32, u32, bool)> = None;
        for (w, &word) in self.ir.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let vec = (w * 64) as u32 + bits.trailing_zeros();
                bits &= bits - 1;
                if !self.enabled(vec) {
                    continue;
                }
                if fast == Some(vec) {
                    // The fast interrupt outranks every IPR level.
                    return Some((vec, 15, true));
                }
                let prio = self.priority(vec);
                if prio == 0 {
                    continue;
                }
                if best.is_none_or(|(_, p, _)| prio > p) {
                    best = Some((vec, prio, false));
                }
            }
        }
        best
    }

    fn read(&self, off: u32) -> u8 {
        match off {
            ICU_SWINTR | ICU_SWINT2R => 0,
            _ => self.regs[off as usize],
        }
    }

    fn write(&mut self, off: u32, value: u8) {
        match off {
            o if o < 0x100 => {
                // IRn: only 0 can be written (clears a held request).
                if value & 1 == 0 {
                    self.clear_ir(o);
                }
            }
            ICU_SWINTR => {
                if value & 1 != 0 {
                    self.set_ir(VEC_SWINT);
                }
            }
            ICU_SWINT2R => {
                if value & 1 != 0 {
                    self.set_ir(VEC_SWINT2);
                }
            }
            o if (ICU_IPR..ICU_IPR + 0x100).contains(&o) => {
                self.regs[o as usize] = value & 0xF;
            }
            o => self.regs[o as usize] = value,
        }
    }
}

/// Snapshot of every architectural register of an [`Rx`] core.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RxCpuSnapshot {
    pub r: Vec<u32>,
    pub pc: u32,
    pub psw: u32,
    pub usp: u32,
    pub isp: u32,
    pub intb: u32,
    pub extb: u32,
    pub bpsw: u32,
    pub bpc: u32,
    pub fintv: u32,
    pub fpsw: u32,
    /// A0/A1 as 72-bit two's complement, sign-extended to 128 bits.
    pub acc: Vec<i128>,
    pub icu: RxIcu,
    pub waiting: bool,
    pub cycles: u64,
}

/// One RX CPU core.
#[derive(Debug)]
pub struct Rx {
    /// General registers. `r[0]` is the active stack pointer (ISP when
    /// `PSW.U` = 0, USP when 1); the inactive one is held in `usp`/`isp`.
    pub r: [u32; 16],
    pub pc: u32,
    psw: u32,
    usp: u32,
    isp: u32,
    pub intb: u32,
    pub extb: u32,
    pub bpsw: u32,
    pub bpc: u32,
    pub fintv: u32,
    pub fpsw: u32,
    acc: [i128; 2],
    pub icu: RxIcu,
    icu_base: u32,
    /// `MOVLI` reservation (the LI flag): cleared by MOVCO, RTE/RTFI and
    /// exception or interrupt acceptance, not by ordinary stores.
    li: bool,
    waiting: bool,
    pub cycles: u64,
    /// Start of the instruction being decoded and the bytes fetched so far.
    ipc: u32,
    ilen: u32,
}

impl Default for Rx {
    fn default() -> Self {
        Self::new()
    }
}

/// Control flow out of one executed instruction.
enum Flow {
    Next,
    Jump(u32),
    /// Unconditional trap through INTB vector `n` (`INT #n`, `BRK`).
    Trap(u32),
    Undefined,
}

/// Operand location resolved from an `ld`/`ss` field.
#[derive(Clone, Copy)]
enum Loc {
    Reg(usize),
    Mem(u32),
}

/// Memory operand size, with the `06` prefix's unsigned-word variant.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sz {
    B,
    W,
    L,
    UW,
    UB,
}

impl Sz {
    fn bytes(self) -> u32 {
        match self {
            Sz::B | Sz::UB => 1,
            Sz::W | Sz::UW => 2,
            Sz::L => 4,
        }
    }
    /// `.B`/`.W`/`.L` from a two-bit `sz` field.
    fn bwl(sz: u32) -> Self {
        match sz & 3 {
            0 => Sz::B,
            1 => Sz::W,
            _ => Sz::L,
        }
    }
    /// The `06` prefix `mi` field.
    fn mi(mi: u32) -> Self {
        match mi & 3 {
            0 => Sz::B,
            1 => Sz::W,
            2 => Sz::L,
            _ => Sz::UW,
        }
    }
}

fn sext(v: u32, sz: Sz) -> u32 {
    match sz {
        Sz::B => v as u8 as i8 as i32 as u32,
        Sz::W => v as u16 as i16 as i32 as u32,
        Sz::UB => v & 0xFF,
        Sz::UW => v & 0xFFFF,
        Sz::L => v,
    }
}

/// Wrap a value to the 72-bit accumulator width.
fn acc72(v: i128) -> i128 {
    (v << 56) >> 56
}

/// Branch displacement of the 3-bit `BRA.S`/`BCnd.S` field (3..10).
fn dsp3(v: u32) -> u32 {
    let v = v & 7;
    if v < 3 {
        v + 8
    } else {
        v
    }
}

impl Rx {
    pub fn new() -> Self {
        Self {
            r: [0; 16],
            pc: 0,
            psw: 0,
            usp: 0,
            isp: 0,
            intb: 0,
            extb: EXTB_RESET,
            bpsw: 0,
            bpc: 0,
            fintv: 0,
            fpsw: 0,
            acc: [0; 2],
            icu: RxIcu::default(),
            icu_base: RX_ICU_BASE,
            li: false,
            waiting: false,
            cycles: 0,
            ipc: 0,
            ilen: 0,
        }
    }

    pub fn for_chip(_chip: &labwired_config::ChipDescriptor) -> Self {
        Self::new()
    }

    pub fn psw(&self) -> u32 {
        self.psw
    }

    /// Write PSW, swapping R0 between ISP and USP when U changes.
    fn set_psw(&mut self, v: u32) {
        let v = v & PSW_WRITABLE;
        if (v ^ self.psw) & PSW_U != 0 {
            if self.psw & PSW_U != 0 {
                self.usp = self.r[0];
                self.r[0] = self.isp;
            } else {
                self.isp = self.r[0];
                self.r[0] = self.usp;
            }
        }
        self.psw = v;
    }

    fn flag(&self, f: u32) -> bool {
        self.psw & f != 0
    }

    fn put_flag(&mut self, f: u32, on: bool) {
        if on {
            self.psw |= f;
        } else {
            self.psw &= !f;
        }
    }

    fn set_sz(&mut self, r: u32) {
        self.put_flag(PSW_Z, r == 0);
        self.put_flag(PSW_S, r & 0x8000_0000 != 0);
    }

    fn ipl(&self) -> u32 {
        (self.psw & PSW_IPL_MASK) >> PSW_IPL_SHIFT
    }

    /// Condition code `cd` (RXv3 manual, Table 1.4).
    fn cond(&self, cd: u32) -> bool {
        let c = self.flag(PSW_C);
        let z = self.flag(PSW_Z);
        let s = self.flag(PSW_S);
        let o = self.flag(PSW_O);
        match cd & 0xF {
            0 => z,
            1 => !z,
            2 => c,
            3 => !c,
            4 => c && !z,
            5 => !c || z,
            6 => !s,
            7 => s,
            8 => s == o,
            9 => s != o,
            10 => !((s != o) || z),
            11 => (s != o) || z,
            12 => o,
            13 => !o,
            14 => true,
            _ => false,
        }
    }

    // ---- arithmetic with flags -------------------------------------------

    fn add_flags(&mut self, a: u32, b: u32, cin: bool) -> u32 {
        let wide = u64::from(a) + u64::from(b) + u64::from(cin);
        let r = wide as u32;
        self.put_flag(PSW_C, wide > 0xFFFF_FFFF);
        self.put_flag(PSW_O, ((a ^ r) & (b ^ r)) & 0x8000_0000 != 0);
        self.set_sz(r);
        r
    }

    /// `a - b - !cin` with C = no borrow.
    fn sub_flags(&mut self, a: u32, b: u32, cin: bool) -> u32 {
        self.add_flags(a, !b, cin)
    }

    // ---- control registers -----------------------------------------------

    fn read_cr(&self, cr: u32) -> Option<u32> {
        Some(match cr {
            0 => self.psw,
            1 => self.ipc,
            2 => {
                if self.psw & PSW_U != 0 {
                    self.r[0]
                } else {
                    self.usp
                }
            }
            3 => self.fpsw,
            8 => self.bpsw,
            9 => self.bpc,
            10 => {
                if self.psw & PSW_U == 0 {
                    self.r[0]
                } else {
                    self.isp
                }
            }
            11 => self.fintv,
            12 => self.intb,
            13 => self.extb,
            _ => return None,
        })
    }

    fn write_cr(&mut self, cr: u32, v: u32) -> bool {
        match cr {
            0 => self.set_psw(v),
            // PC is not writable through MVTC/POPC.
            1 => {}
            2 => {
                if self.psw & PSW_U != 0 {
                    self.r[0] = v;
                } else {
                    self.usp = v;
                }
            }
            3 => self.fpsw = v,
            8 => self.bpsw = v & PSW_WRITABLE,
            9 => self.bpc = v,
            10 => {
                if self.psw & PSW_U == 0 {
                    self.r[0] = v;
                } else {
                    self.isp = v;
                }
            }
            11 => self.fintv = v,
            12 => self.intb = v,
            13 => self.extb = v,
            _ => return false,
        }
        true
    }

    // ---- memory ----------------------------------------------------------

    fn icu_off(&self, addr: u32) -> Option<u32> {
        let off = addr.wrapping_sub(self.icu_base);
        (off < ICU_SIZE).then_some(off)
    }

    fn read(&mut self, bus: &mut dyn Bus, addr: u32, n: u32) -> SimResult<u32> {
        if self.icu_off(addr).is_some() || self.icu_off(addr.wrapping_add(n - 1)).is_some() {
            let mut v = 0u32;
            for i in 0..n {
                let a = addr.wrapping_add(i);
                let b = match self.icu_off(a) {
                    Some(off) => self.icu.read(off),
                    None => bus.read_u8(u64::from(a))?,
                };
                v |= u32::from(b) << (8 * i);
            }
            return Ok(v);
        }
        let a = u64::from(addr);
        Ok(match n {
            1 => u32::from(bus.read_u8(a)?),
            2 => u32::from(bus.read_u16(a)?),
            _ => bus.read_u32(a)?,
        })
    }

    fn write(&mut self, bus: &mut dyn Bus, addr: u32, n: u32, v: u32) -> SimResult<()> {
        if self.icu_off(addr).is_some() || self.icu_off(addr.wrapping_add(n - 1)).is_some() {
            for i in 0..n {
                let a = addr.wrapping_add(i);
                let b = (v >> (8 * i)) as u8;
                match self.icu_off(a) {
                    Some(off) => self.icu.write(off, b),
                    None => bus.write_u8(u64::from(a), b)?,
                }
            }
            return Ok(());
        }
        let a = u64::from(addr);
        match n {
            1 => bus.write_u8(a, v as u8),
            2 => bus.write_u16(a, v as u16),
            _ => bus.write_u32(a, v),
        }
    }

    fn load(&mut self, bus: &mut dyn Bus, addr: u32, sz: Sz) -> SimResult<u32> {
        let v = self.read(bus, addr, sz.bytes())?;
        Ok(sext(v, sz))
    }

    fn store(&mut self, bus: &mut dyn Bus, addr: u32, sz: Sz, v: u32) -> SimResult<()> {
        self.write(bus, addr, sz.bytes(), v)
    }

    fn push(&mut self, bus: &mut dyn Bus, v: u32) -> SimResult<()> {
        self.r[0] = self.r[0].wrapping_sub(4);
        let sp = self.r[0];
        self.write(bus, sp, 4, v)
    }

    fn pop(&mut self, bus: &mut dyn Bus) -> SimResult<u32> {
        let sp = self.r[0];
        let v = self.read(bus, sp, 4)?;
        self.r[0] = sp.wrapping_add(4);
        Ok(v)
    }

    // ---- instruction stream ----------------------------------------------

    fn fetch8(&mut self, bus: &mut dyn Bus) -> SimResult<u32> {
        let b = bus.read_u8(u64::from(self.ipc.wrapping_add(self.ilen)))?;
        self.ilen += 1;
        Ok(u32::from(b))
    }

    /// Little-endian unsigned field of `n` bytes.
    fn fetch_u(&mut self, bus: &mut dyn Bus, n: u32) -> SimResult<u32> {
        let mut v = 0;
        for i in 0..n {
            v |= self.fetch8(bus)? << (8 * i);
        }
        Ok(v)
    }

    /// Immediate selected by an `li` field: 00 = 32-bit, 01 = simm8,
    /// 10 = simm16, 11 = simm24.
    fn fetch_li(&mut self, bus: &mut dyn Bus, li: u32) -> SimResult<u32> {
        Ok(match li & 3 {
            0 => self.fetch_u(bus, 4)?,
            1 => self.fetch_u(bus, 1)? as u8 as i8 as i32 as u32,
            2 => self.fetch_u(bus, 2)? as u16 as i16 as i32 as u32,
            _ => (((self.fetch_u(bus, 3)? << 8) as i32) >> 8) as u32,
        })
    }

    /// Resolve an `ld` (`ss`/`sd`) field: 0 `[Rn]`, 1 `dsp:8[Rn]`,
    /// 2 `dsp:16[Rn]` (unsigned, scaled by the operand size), 3 `Rn`.
    fn ea(&mut self, bus: &mut dyn Bus, ld: u32, reg: u32, sz: Sz) -> SimResult<Loc> {
        let base = self.r[reg as usize & 15];
        Ok(match ld & 3 {
            0 => Loc::Mem(base),
            1 => Loc::Mem(base.wrapping_add(self.fetch_u(bus, 1)? * sz.bytes())),
            2 => Loc::Mem(base.wrapping_add(self.fetch_u(bus, 2)? * sz.bytes())),
            _ => Loc::Reg(reg as usize & 15),
        })
    }

    /// Source operand value: a register as is, memory loaded and extended.
    fn src(&mut self, bus: &mut dyn Bus, loc: Loc, sz: Sz) -> SimResult<u32> {
        match loc {
            Loc::Reg(r) => Ok(self.r[r]),
            Loc::Mem(a) => self.load(bus, a, sz),
        }
    }

    fn put(&mut self, bus: &mut dyn Bus, loc: Loc, sz: Sz, v: u32) -> SimResult<()> {
        match loc {
            Loc::Reg(r) => {
                self.r[r] = v;
                Ok(())
            }
            Loc::Mem(a) => self.store(bus, a, sz, v),
        }
    }

    // ---- exceptions and interrupts ---------------------------------------

    /// Common entry: save PSW and `ret` on the ISP and vector through `vaddr`.
    fn enter(
        &mut self,
        bus: &mut dyn Bus,
        vaddr: u32,
        ret: u32,
        ipl: Option<u32>,
    ) -> SimResult<()> {
        let old = self.psw;
        let mut new = old & !(PSW_I | PSW_U | PSW_PM);
        if let Some(l) = ipl {
            new = (new & !PSW_IPL_MASK) | (l << PSW_IPL_SHIFT);
        }
        self.set_psw(new);
        self.push(bus, old)?;
        self.push(bus, ret)?;
        self.li = false;
        self.pc = self.read(bus, vaddr, 4)?;
        Ok(())
    }

    /// The request the core would take now: `(vector, priority, fast)`.
    /// IR flags whose IER bit is clear, or whose priority does not exceed
    /// PSW.IPL, are held, not taken.
    fn acceptable_interrupt(&self) -> Option<(u32, u32, bool)> {
        if self.icu.ir == [0; 4] || !self.flag(PSW_I) {
            return None;
        }
        let (vec, prio, fast) = self.icu.highest()?;
        (fast || prio > self.ipl()).then_some((vec, prio, fast))
    }

    fn take_interrupt(&mut self, bus: &mut dyn Bus) -> SimResult<bool> {
        let Some((vec, prio, fast)) = self.acceptable_interrupt() else {
            return Ok(false);
        };
        self.waiting = false;
        self.icu.clear_ir(vec);
        if fast {
            self.bpsw = self.psw;
            self.bpc = self.pc;
            let new = (self.psw & !(PSW_I | PSW_U | PSW_PM | PSW_IPL_MASK)) | (15 << PSW_IPL_SHIFT);
            self.set_psw(new);
            self.li = false;
            self.pc = self.fintv;
        } else {
            let ret = self.pc;
            self.enter(bus, self.intb.wrapping_add(vec * 4), ret, Some(prio))?;
        }
        Ok(true)
    }

    fn observer_registers(&self) -> [u32; 20] {
        let mut regs = [0u32; 20];
        regs[..16].copy_from_slice(&self.r);
        regs[16] = self.psw;
        regs[17] = self.pc;
        regs[18] = self.r[0];
        regs[19] = self.pc;
        regs
    }

    // ---- execution -------------------------------------------------------

    fn exec(&mut self, bus: &mut dyn Bus) -> SimResult<Flow> {
        let b0 = self.fetch8(bus)?;
        match b0 {
            0x00 => Ok(Flow::Trap(0)),
            // DBT: debug trap; no debugger is attached.
            0x01 => Ok(Flow::Undefined),
            0x02 => Ok(Flow::Jump(self.pop(bus)?)),
            0x03 => Ok(Flow::Next),
            0x04 | 0x05 => {
                let d = (((self.fetch_u(bus, 3)? << 8) as i32) >> 8) as u32;
                let target = self.ipc.wrapping_add(d);
                if b0 == 0x05 {
                    let ret = self.ipc.wrapping_add(self.ilen);
                    self.push(bus, ret)?;
                }
                Ok(Flow::Jump(target))
            }
            0x06 => self.exec_memex(bus),
            0x08..=0x0F => Ok(Flow::Jump(self.ipc.wrapping_add(dsp3(b0)))),
            0x10..=0x1F => {
                let cd = (b0 >> 3) & 1;
                if self.cond(cd) {
                    Ok(Flow::Jump(self.ipc.wrapping_add(dsp3(b0))))
                } else {
                    Ok(Flow::Next)
                }
            }
            0x20..=0x2F => {
                let d = self.fetch_u(bus, 1)? as u8 as i8 as i32 as u32;
                if self.cond(b0 & 0xF) {
                    Ok(Flow::Jump(self.ipc.wrapping_add(d)))
                } else {
                    Ok(Flow::Next)
                }
            }
            0x38..=0x3B => {
                let d = self.fetch_u(bus, 2)? as u16 as i16 as i32 as u32;
                let target = self.ipc.wrapping_add(d);
                match b0 {
                    0x38 => Ok(Flow::Jump(target)),
                    0x39 => {
                        let ret = self.ipc.wrapping_add(self.ilen);
                        self.push(bus, ret)?;
                        Ok(Flow::Jump(target))
                    }
                    _ => {
                        if self.cond(b0 & 1) {
                            Ok(Flow::Jump(target))
                        } else {
                            Ok(Flow::Next)
                        }
                    }
                }
            }
            0x3C..=0x3E => {
                // MOV.size #uimm8, dsp:5[Rd]
                let sz = Sz::bwl(b0 & 3);
                let b1 = self.fetch8(bus)?;
                let imm = self.fetch8(bus)?;
                let rd = ((b1 >> 4) & 7) as usize;
                let dsp = (((b1 >> 7) << 4) | (b1 & 0xF)) * sz.bytes();
                let addr = self.r[rd].wrapping_add(dsp);
                self.store(bus, addr, sz, imm)?;
                Ok(Flow::Next)
            }
            0x3F => {
                // RTSD #uimm8, Rd-Rd2
                let b1 = self.fetch8(bus)?;
                let imm = self.fetch8(bus)?;
                let (rd, rd2) = (b1 >> 4, b1 & 0xF);
                let n = rd2.saturating_sub(rd) + 1;
                self.r[0] = self.r[0].wrapping_add(imm.wrapping_sub(n).wrapping_mul(4));
                for reg in rd..=rd2 {
                    let v = self.pop(bus)?;
                    if reg != 0 {
                        self.r[reg as usize] = v;
                    }
                }
                Ok(Flow::Jump(self.pop(bus)?))
            }
            0x40..=0x57 => {
                // SUB/CMP/ADD/MUL/AND/OR  src(.UB or Rs), Rd
                let b1 = self.fetch8(bus)?;
                let ld = b0 & 3;
                let (rs, rd) = (b1 >> 4, (b1 & 0xF) as usize);
                let loc = self.ea(bus, ld, rs, Sz::UB)?;
                let v = self.src(bus, loc, Sz::UB)?;
                self.alu2((b0 >> 2) & 7, rd, v);
                Ok(Flow::Next)
            }
            0x58..=0x5F => {
                // MOVU.size src, Rd
                let b1 = self.fetch8(bus)?;
                let sz = if b0 & 4 != 0 { Sz::UW } else { Sz::UB };
                let (rs, rd) = (b1 >> 4, (b1 & 0xF) as usize);
                let loc = self.ea(bus, b0 & 3, rs, sz)?;
                let v = self.src(bus, loc, sz)?;
                self.r[rd] = sext(v, sz);
                Ok(Flow::Next)
            }
            0x60..=0x65 => {
                let b1 = self.fetch8(bus)?;
                let (imm, rd) = (b1 >> 4, (b1 & 0xF) as usize);
                let op = match b0 {
                    0x60 => 0, // SUB
                    0x61 => 1, // CMP
                    0x62 => 2, // ADD
                    0x63 => 3, // MUL
                    0x64 => 4, // AND
                    _ => 5,    // OR
                };
                self.alu2(op, rd, imm);
                Ok(Flow::Next)
            }
            0x66 => {
                let b1 = self.fetch8(bus)?;
                self.r[(b1 & 0xF) as usize] = b1 >> 4;
                Ok(Flow::Next)
            }
            0x67 => {
                let imm = self.fetch8(bus)?;
                self.r[0] = self.r[0].wrapping_add(imm * 4);
                Ok(Flow::Jump(self.pop(bus)?))
            }
            0x68..=0x6D => {
                let b1 = self.fetch8(bus)?;
                let imm = ((b0 & 1) << 4) | (b1 >> 4);
                let rd = (b1 & 0xF) as usize;
                let v = self.r[rd];
                self.r[rd] = match b0 & 0xE {
                    0x8 => self.shlr(v, imm),
                    0xA => self.shar(v, imm),
                    _ => self.shll(v, imm),
                };
                Ok(Flow::Next)
            }
            0x6E => {
                let b1 = self.fetch8(bus)?;
                let (rs, rs2) = (b1 >> 4, b1 & 0xF);
                for reg in (rs..=rs2).rev() {
                    let v = self.r[reg as usize];
                    self.push(bus, v)?;
                }
                Ok(Flow::Next)
            }
            0x6F => {
                let b1 = self.fetch8(bus)?;
                let (rd, rd2) = (b1 >> 4, b1 & 0xF);
                for reg in rd..=rd2 {
                    let v = self.pop(bus)?;
                    if reg != 0 {
                        self.r[reg as usize] = v;
                    }
                }
                Ok(Flow::Next)
            }
            0x70..=0x73 => {
                // ADD #imm, Rs, Rd
                let b1 = self.fetch8(bus)?;
                let imm = self.fetch_li(bus, b0 & 3)?;
                let (rs, rd) = ((b1 >> 4) as usize, (b1 & 0xF) as usize);
                let r = self.add_flags(self.r[rs], imm, false);
                self.r[rd] = r;
                Ok(Flow::Next)
            }
            0x74..=0x77 => self.exec_74(bus, b0),
            0x78..=0x7D => {
                let b1 = self.fetch8(bus)?;
                let bit = ((b0 & 1) << 4) | (b1 >> 4);
                let rd = (b1 & 0xF) as usize;
                let m = 1u32 << bit;
                match b0 & 0xE {
                    0x8 => self.r[rd] |= m,
                    0xA => self.r[rd] &= !m,
                    _ => {
                        let set = self.r[rd] & m != 0;
                        self.put_flag(PSW_C, set);
                        self.put_flag(PSW_Z, !set);
                    }
                }
                Ok(Flow::Next)
            }
            0x7E => self.exec_7e(bus),
            0x7F => self.exec_7f(bus),
            0x80..=0xBF => {
                // MOV/MOVU with a 5-bit displacement, R0-R7 only.
                let b1 = self.fetch8(bus)?;
                let dsp5 = ((b0 & 7) << 2) | ((b1 >> 7) << 1) | ((b1 >> 3) & 1);
                let ra = ((b1 >> 4) & 7) as usize;
                let rb = (b1 & 7) as usize;
                if b0 >= 0xB0 {
                    let sz = if b0 & 8 != 0 { Sz::UW } else { Sz::UB };
                    let addr = self.r[ra].wrapping_add(dsp5 * sz.bytes());
                    self.r[rb] = self.load(bus, addr, sz)?;
                } else {
                    let sz = Sz::bwl((b0 >> 4) & 3);
                    let addr = self.r[ra].wrapping_add(dsp5 * sz.bytes());
                    if b0 & 8 != 0 {
                        self.r[rb] = self.load(bus, addr, sz)?;
                    } else {
                        self.store(bus, addr, sz, self.r[rb])?;
                    }
                }
                Ok(Flow::Next)
            }
            0xC0..=0xEF => {
                // MOV.size src, dest (ldd/lds)
                let b1 = self.fetch8(bus)?;
                let sz = Sz::bwl((b0 >> 4) & 3);
                let (ldd, lds) = ((b0 >> 2) & 3, b0 & 3);
                let (hi, lo) = (b1 >> 4, b1 & 0xF);
                if lds == 3 && ldd == 3 {
                    self.r[lo as usize] = sext(self.r[hi as usize], sz);
                } else if lds == 3 {
                    // Store: Rs (low nibble) to dsp[Rd] (high nibble).
                    let loc = self.ea(bus, ldd, hi, sz)?;
                    let v = self.r[lo as usize];
                    self.put(bus, loc, sz, v)?;
                } else if ldd == 3 {
                    let loc = self.ea(bus, lds, hi, sz)?;
                    self.r[lo as usize] = self.src(bus, loc, sz)?;
                } else {
                    let sloc = self.ea(bus, lds, hi, sz)?;
                    let v = self.src(bus, sloc, sz)?;
                    let dloc = self.ea(bus, ldd, lo, sz)?;
                    self.put(bus, dloc, sz, v)?;
                }
                Ok(Flow::Next)
            }
            0xF0..=0xF3 => {
                // BSET/BCLR #imm3, [Rd].B
                let b1 = self.fetch8(bus)?;
                let loc = self.ea(bus, b0 & 3, b1 >> 4, Sz::UB)?;
                let m = 1u32 << (b1 & 7);
                let v = self.src(bus, loc, Sz::UB)?;
                let v = if b1 & 8 != 0 { v & !m } else { v | m };
                self.put(bus, loc, Sz::UB, v)?;
                Ok(Flow::Next)
            }
            0xF4..=0xF7 => {
                let b1 = self.fetch8(bus)?;
                if b1 & 8 == 0 {
                    // BTST #imm3, [Rs].B
                    let loc = self.ea(bus, b0 & 3, b1 >> 4, Sz::UB)?;
                    let v = self.src(bus, loc, Sz::UB)?;
                    let set = v & (1 << (b1 & 7)) != 0;
                    self.put_flag(PSW_C, set);
                    self.put_flag(PSW_Z, !set);
                } else if b1 & 4 == 0 {
                    // PUSH.size src
                    let sz = Sz::bwl(b1 & 3);
                    let loc = self.ea(bus, b0 & 3, b1 >> 4, sz)?;
                    let v = self.src(bus, loc, sz)?;
                    self.push(bus, v)?;
                } else {
                    return Ok(Flow::Undefined);
                }
                Ok(Flow::Next)
            }
            0xF8..=0xFB => {
                // MOV.size #imm, dest
                let b1 = self.fetch8(bus)?;
                let sz = Sz::bwl(b1 & 3);
                let loc = self.ea(bus, b0 & 3, b1 >> 4, sz)?;
                let imm = self.fetch_li(bus, (b1 >> 2) & 3)?;
                self.put(bus, loc, sz, imm)?;
                Ok(Flow::Next)
            }
            0xFC => self.exec_fc(bus),
            0xFD => self.exec_fd(bus),
            0xFE => {
                // MOV/MOVU [Ri, Rb]
                let b1 = self.fetch8(bus)?;
                let b2 = self.fetch8(bus)?;
                let ri = (b1 & 0xF) as usize;
                let (rb, rx) = ((b2 >> 4) as usize, (b2 & 0xF) as usize);
                let kind = b1 >> 6;
                let sz = match kind {
                    3 => {
                        if b1 & 0x10 != 0 {
                            Sz::UW
                        } else {
                            Sz::UB
                        }
                    }
                    _ => Sz::bwl((b1 >> 4) & 3),
                };
                let addr = self.r[rb].wrapping_add(self.r[ri].wrapping_mul(sz.bytes()));
                match kind {
                    0 => self.store(bus, addr, sz, self.r[rx])?,
                    1 | 3 => self.r[rx] = self.load(bus, addr, sz)?,
                    _ => return Ok(Flow::Undefined),
                }
                Ok(Flow::Next)
            }
            0xFF => {
                // Three-operand: Rd = Rs2 op Rs (Rs high nibble, Rs2 low).
                let b1 = self.fetch8(bus)?;
                let b2 = self.fetch8(bus)?;
                let rd = (b1 & 0xF) as usize;
                let (a, b) = (self.r[(b2 & 0xF) as usize], self.r[(b2 >> 4) as usize]);
                let r = match b1 >> 4 {
                    0x0 => self.sub_flags(a, b, true),
                    0x2 => self.add_flags(a, b, false),
                    0x3 => a.wrapping_mul(b),
                    0x4 => {
                        let r = a & b;
                        self.set_sz(r);
                        r
                    }
                    0x5 => {
                        let r = a | b;
                        self.set_sz(r);
                        r
                    }
                    0x6 => {
                        let r = a ^ b;
                        self.set_sz(r);
                        r
                    }
                    0x8 => self.fop(1, f32::from_bits(a), f32::from_bits(b)),
                    0xA => self.fop(2, f32::from_bits(a), f32::from_bits(b)),
                    0xB => self.fop(3, f32::from_bits(a), f32::from_bits(b)),
                    _ => return Ok(Flow::Undefined),
                };
                self.r[rd] = r;
                Ok(Flow::Next)
            }
            _ => Ok(Flow::Undefined),
        }
    }

    /// Two-operand ALU ops shared by the short forms: 0 SUB, 1 CMP, 2 ADD,
    /// 3 MUL, 4 AND, 5 OR (`Rd = Rd op v`).
    fn alu2(&mut self, op: u32, rd: usize, v: u32) {
        let d = self.r[rd];
        match op {
            0 => self.r[rd] = self.sub_flags(d, v, true),
            1 => {
                self.sub_flags(d, v, true);
            }
            2 => self.r[rd] = self.add_flags(d, v, false),
            3 => self.r[rd] = d.wrapping_mul(v),
            4 => {
                let r = d & v;
                self.set_sz(r);
                self.r[rd] = r;
            }
            _ => {
                let r = d | v;
                self.set_sz(r);
                self.r[rd] = r;
            }
        }
    }

    /// Ops of the `06`/`FC`/`FD 7x` "extended" group, keyed by the opcode
    /// nibble they share: 0 SBB, 2 ADC, 4 MAX, 5 MIN, 6 EMUL, 7 EMULU,
    /// 8 DIV, 9 DIVU, C TST, D XOR, E STZ, F STNZ.
    fn alu_ext(&mut self, op: u32, rd: usize, v: u32) -> bool {
        let d = self.r[rd];
        match op {
            0x0 => {
                let c = self.flag(PSW_C);
                self.r[rd] = self.sub_flags(d, v, c);
            }
            0x2 => {
                let c = self.flag(PSW_C);
                self.r[rd] = self.add_flags(d, v, c);
            }
            0x4 => self.r[rd] = (d as i32).max(v as i32) as u32,
            0x5 => self.r[rd] = (d as i32).min(v as i32) as u32,
            0x6 | 0x7 => {
                let p = if op == 6 {
                    (i64::from(d as i32) * i64::from(v as i32)) as u64
                } else {
                    u64::from(d) * u64::from(v)
                };
                self.r[rd] = p as u32;
                self.r[(rd + 1) & 15] = (p >> 32) as u32;
            }
            0x8 => {
                if v == 0 || (d == 0x8000_0000 && v == 0xFFFF_FFFF) {
                    self.put_flag(PSW_O, true);
                } else {
                    self.put_flag(PSW_O, false);
                    self.r[rd] = (d as i32).wrapping_div(v as i32) as u32;
                }
            }
            0x9 => match d.checked_div(v) {
                Some(q) => {
                    self.put_flag(PSW_O, false);
                    self.r[rd] = q;
                }
                None => self.put_flag(PSW_O, true),
            },
            0xC => self.set_sz(d & v),
            0xD => {
                let r = d ^ v;
                self.set_sz(r);
                self.r[rd] = r;
            }
            0xE => {
                if self.flag(PSW_Z) {
                    self.r[rd] = v;
                }
            }
            0xF => {
                if !self.flag(PSW_Z) {
                    self.r[rd] = v;
                }
            }
            _ => return false,
        }
        true
    }

    fn exec_memex(&mut self, bus: &mut dyn Bus) -> SimResult<Flow> {
        let b1 = self.fetch8(bus)?;
        let sz = Sz::mi(b1 >> 6);
        let op = (b1 >> 2) & 0xF;
        let ld = b1 & 3;
        if op == 0x8 {
            let ext = self.fetch8(bus)?;
            let b3 = self.fetch8(bus)?;
            let (rs, rd) = (b3 >> 4, (b3 & 0xF) as usize);
            let loc = self.ea(bus, ld, rs, sz)?;
            match ext {
                0x10 => {
                    // XCHG src, Rd
                    let v = self.src(bus, loc, sz)?;
                    let old = self.r[rd];
                    self.put(bus, loc, sz, old)?;
                    self.r[rd] = v;
                }
                0x11 | 0x15 => {
                    let v = self.src(bus, loc, sz)?;
                    let f = if ext == 0x11 {
                        v as i32 as f32
                    } else {
                        v as f32
                    };
                    self.r[rd] = f.to_bits();
                    self.set_sz_f(f);
                }
                _ => {
                    let v = self.src(bus, loc, sz)?;
                    if !self.alu_ext(ext, rd, v) {
                        return Ok(Flow::Undefined);
                    }
                }
            }
            return Ok(Flow::Next);
        }
        let b2 = self.fetch8(bus)?;
        let (rs, rd) = (b2 >> 4, (b2 & 0xF) as usize);
        if op > 5 {
            return Ok(Flow::Undefined);
        }
        let loc = self.ea(bus, ld, rs, sz)?;
        let v = self.src(bus, loc, sz)?;
        self.alu2(op, rd, v);
        Ok(Flow::Next)
    }

    fn exec_74(&mut self, bus: &mut dyn Bus, b0: u32) -> SimResult<Flow> {
        let b1 = self.fetch8(bus)?;
        let li = b0 & 3;
        let rd = (b1 & 0xF) as usize;
        match (li, b1 >> 4) {
            (_, 0..=3) => {
                let imm = self.fetch_li(bus, li)?;
                let op = match b1 >> 4 {
                    0 => 1, // CMP
                    1 => 3, // MUL
                    2 => 4, // AND
                    _ => 5, // OR
                };
                self.alu2(op, rd, imm);
            }
            (1, 4) => self.r[rd] = self.fetch8(bus)?,
            (1, 5) => {
                let imm = self.fetch8(bus)?;
                self.sub_flags(self.r[rd], imm, true);
            }
            (1, 6) if b1 == 0x60 => {
                let imm = self.fetch8(bus)?;
                return Ok(Flow::Trap(imm));
            }
            (1, 7) if b1 == 0x70 => {
                let imm = self.fetch8(bus)?;
                if imm & 0xF0 != 0 {
                    return Ok(Flow::Undefined);
                }
                self.psw = (self.psw & !PSW_IPL_MASK) | ((imm & 0xF) << PSW_IPL_SHIFT);
            }
            _ => return Ok(Flow::Undefined),
        }
        Ok(Flow::Next)
    }

    fn exec_7e(&mut self, bus: &mut dyn Bus) -> SimResult<Flow> {
        let b1 = self.fetch8(bus)?;
        let rd = (b1 & 0xF) as usize;
        let v = self.r[rd];
        match b1 >> 4 {
            0x0 => {
                let r = !v;
                self.set_sz(r);
                self.r[rd] = r;
            }
            0x1 => self.r[rd] = self.neg(v),
            0x2 => self.r[rd] = self.abs(v),
            0x3 => {
                if self.flag(PSW_O) {
                    self.r[rd] = if self.flag(PSW_S) {
                        0x7FFF_FFFF
                    } else {
                        0x8000_0000
                    };
                }
            }
            0x4 => {
                // RORC
                let c = u32::from(self.flag(PSW_C));
                let r = (v >> 1) | (c << 31);
                self.put_flag(PSW_C, v & 1 != 0);
                self.set_sz(r);
                self.r[rd] = r;
            }
            0x5 => {
                // ROLC
                let c = u32::from(self.flag(PSW_C));
                let r = (v << 1) | c;
                self.put_flag(PSW_C, v & 0x8000_0000 != 0);
                self.set_sz(r);
                self.r[rd] = r;
            }
            0x8..=0xA => {
                // PUSH.size Rs: the stack moves by 4 whatever the size.
                let sz = Sz::bwl((b1 >> 4) & 3);
                self.r[0] = self.r[0].wrapping_sub(4);
                let sp = self.r[0];
                self.store(bus, sp, sz, v)?;
            }
            0xB => {
                let val = self.pop(bus)?;
                self.r[rd] = val;
            }
            0xC | 0xD => {
                let cr = b1 & 0x1F;
                let Some(val) = self.read_cr(cr) else {
                    return Ok(Flow::Undefined);
                };
                self.push(bus, val)?;
            }
            0xE | 0xF => {
                let cr = b1 & 0x1F;
                let val = self.pop(bus)?;
                if cr == 1 || !self.write_cr(cr, val) {
                    return Ok(Flow::Undefined);
                }
            }
            _ => return Ok(Flow::Undefined),
        }
        Ok(Flow::Next)
    }

    fn exec_7f(&mut self, bus: &mut dyn Bus) -> SimResult<Flow> {
        let b1 = self.fetch8(bus)?;
        let rs = (b1 & 0xF) as usize;
        let next = self.ipc.wrapping_add(self.ilen);
        match b1 {
            0x00..=0x0F => Ok(Flow::Jump(self.r[rs])),
            0x10..=0x1F => {
                self.push(bus, next)?;
                Ok(Flow::Jump(self.r[rs]))
            }
            0x40..=0x4F => Ok(Flow::Jump(self.ipc.wrapping_add(self.r[rs]))),
            0x50..=0x5F => {
                self.push(bus, next)?;
                Ok(Flow::Jump(self.ipc.wrapping_add(self.r[rs])))
            }
            0x80..=0x8F => {
                self.exec_string(bus, b1 & 0xF)?;
                Ok(Flow::Next)
            }
            0x93 => {
                // SATR
                if self.flag(PSW_O) {
                    if self.flag(PSW_S) {
                        self.r[6] = 0;
                        self.r[5] = 0x7FFF_FFFF;
                        self.r[4] = 0xFFFF_FFFF;
                    } else {
                        self.r[6] = 0xFFFF_FFFF;
                        self.r[5] = 0x8000_0000;
                        self.r[4] = 0;
                    }
                }
                Ok(Flow::Next)
            }
            0x94 => {
                // RTFI
                let psw = self.bpsw;
                self.set_psw(psw);
                self.li = false;
                Ok(Flow::Jump(self.bpc))
            }
            0x95 => {
                // RTE
                let pc = self.pop(bus)?;
                let psw = self.pop(bus)?;
                self.set_psw(psw);
                self.li = false;
                Ok(Flow::Jump(pc))
            }
            0x96 => {
                // WAIT: enable interrupts and sleep until one is taken.
                self.psw |= PSW_I;
                self.waiting = true;
                Ok(Flow::Next)
            }
            0xA0..=0xBF => {
                let m = match b1 & 0xF {
                    0 => PSW_C,
                    1 => PSW_Z,
                    2 => PSW_S,
                    3 => PSW_O,
                    8 => PSW_I,
                    9 => PSW_U,
                    _ => return Ok(Flow::Undefined),
                };
                let psw = if b1 & 0x10 == 0 {
                    self.psw | m
                } else {
                    self.psw & !m
                };
                self.set_psw(psw);
                Ok(Flow::Next)
            }
            _ => Ok(Flow::Undefined),
        }
    }

    /// The string group, `7F 8x` (R1 destination, R2 source/value, R3
    /// count). Executed to completion in one step.
    fn exec_string(&mut self, bus: &mut dyn Bus, op: u32) -> SimResult<()> {
        match op {
            0x3 => {
                // SCMPU
                while self.r[3] != 0 {
                    let a = self.load(bus, self.r[1], Sz::UB)?;
                    let b = self.load(bus, self.r[2], Sz::UB)?;
                    self.r[1] = self.r[1].wrapping_add(1);
                    self.r[2] = self.r[2].wrapping_add(1);
                    self.r[3] -= 1;
                    self.put_flag(PSW_Z, a == b);
                    self.put_flag(PSW_C, a >= b);
                    if a != b || a == 0 {
                        break;
                    }
                }
            }
            0x7 => {
                // SMOVU
                while self.r[3] != 0 {
                    let v = self.load(bus, self.r[2], Sz::UB)?;
                    self.store(bus, self.r[1], Sz::UB, v)?;
                    self.r[1] = self.r[1].wrapping_add(1);
                    self.r[2] = self.r[2].wrapping_add(1);
                    self.r[3] -= 1;
                    if v == 0 {
                        break;
                    }
                }
            }
            0xB | 0xF => {
                // SMOVB (decrementing) / SMOVF (incrementing)
                let step = if op == 0xB { u32::MAX } else { 1 };
                while self.r[3] != 0 {
                    let v = self.load(bus, self.r[2], Sz::UB)?;
                    self.store(bus, self.r[1], Sz::UB, v)?;
                    self.r[1] = self.r[1].wrapping_add(step);
                    self.r[2] = self.r[2].wrapping_add(step);
                    self.r[3] -= 1;
                }
            }
            0x0..=0x2 | 0x4..=0x6 => {
                // SUNTIL / SWHILE
                let sz = Sz::bwl(op & 3);
                let until = op < 4;
                while self.r[3] != 0 {
                    let v = self.read(bus, self.r[1], sz.bytes())?;
                    self.r[1] = self.r[1].wrapping_add(sz.bytes());
                    self.r[3] -= 1;
                    let key = self.r[2] & ((1u64 << (8 * sz.bytes())) - 1) as u32;
                    self.put_flag(PSW_Z, v == key);
                    self.put_flag(PSW_C, v >= key);
                    if (v == key) == until {
                        break;
                    }
                }
            }
            0x8..=0xA => {
                // SSTR.size
                let sz = Sz::bwl(op & 3);
                while self.r[3] != 0 {
                    self.store(bus, self.r[1], sz, self.r[2])?;
                    self.r[1] = self.r[1].wrapping_add(sz.bytes());
                    self.r[3] -= 1;
                }
            }
            _ => {
                // RMPA.size: R6:R5:R4 += sum(*R1++ * *R2++), R3 times.
                let sz = Sz::bwl(op & 3);
                let mut acc = (i128::from(self.r[6] as i32) << 64)
                    | (i128::from(self.r[5]) << 32)
                    | i128::from(self.r[4]);
                while self.r[3] != 0 {
                    let a = self.load(bus, self.r[1], sz)? as i32;
                    let b = self.load(bus, self.r[2], sz)? as i32;
                    self.r[1] = self.r[1].wrapping_add(sz.bytes());
                    self.r[2] = self.r[2].wrapping_add(sz.bytes());
                    self.r[3] -= 1;
                    acc += i128::from(a) * i128::from(b);
                }
                let wrapped = (acc << 48) >> 48;
                self.put_flag(PSW_O, wrapped != acc);
                self.put_flag(PSW_S, wrapped < 0);
                self.r[4] = wrapped as u32;
                self.r[5] = (wrapped >> 32) as u32;
                self.r[6] = (wrapped >> 64) as u32;
            }
        }
        Ok(())
    }

    fn exec_fc(&mut self, bus: &mut dyn Bus) -> SimResult<Flow> {
        let b1 = self.fetch8(bus)?;
        let b2 = self.fetch8(bus)?;
        let (hi, lo) = (b2 >> 4, b2 & 0xF);
        let ld = b1 & 3;
        match b1 {
            0x03 | 0x07 | 0x0B | 0x0F | 0x3B | 0x43 => {
                let (rs, rd) = (hi as usize, lo as usize);
                let v = self.r[rs];
                match b1 {
                    0x03 => {
                        let c = self.flag(PSW_C);
                        self.r[rd] = self.sub_flags(self.r[rd], v, c);
                    }
                    0x07 => self.r[rd] = self.neg(v),
                    0x0B => {
                        let c = self.flag(PSW_C);
                        self.r[rd] = self.add_flags(self.r[rd], v, c);
                    }
                    0x0F => self.r[rd] = self.abs(v),
                    0x3B => {
                        let r = !v;
                        self.set_sz(r);
                        self.r[rd] = r;
                    }
                    _ => {
                        self.r[rs] = self.r[rd];
                        self.r[rd] = v;
                    }
                }
            }
            0x10..=0x37 => {
                let op = match b1 >> 2 {
                    0x4 => 0x4,
                    0x5 => 0x5,
                    0x6 => 0x6,
                    0x7 => 0x7,
                    0x8 => 0x8,
                    0x9 => 0x9,
                    0xC => 0xC,
                    0xD => 0xD,
                    _ => return Ok(Flow::Undefined),
                };
                let loc = self.ea(bus, ld, hi, Sz::UB)?;
                let v = self.src(bus, loc, Sz::UB)?;
                self.alu_ext(op, lo as usize, v);
            }
            0x40..=0x42 => {
                // XCHG [Rs].UB, Rd
                let loc = self.ea(bus, ld, hi, Sz::UB)?;
                let v = self.src(bus, loc, Sz::UB)?;
                let old = self.r[lo as usize];
                self.put(bus, loc, Sz::UB, old)?;
                self.r[lo as usize] = v;
            }
            0x44..=0x47 | 0x54..=0x57 => {
                // ITOF / UTOF
                let loc = self.ea(bus, ld, hi, Sz::UB)?;
                let v = self.src(bus, loc, Sz::UB)?;
                let f = if b1 < 0x50 { v as i32 as f32 } else { v as f32 };
                self.r[lo as usize] = f.to_bits();
                self.set_sz_f(f);
            }
            0x4B | 0x4F => {
                // STZ/STNZ Rs, Rd
                let take = self.flag(PSW_Z) == (b1 == 0x4B);
                if take {
                    self.r[lo as usize] = self.r[hi as usize];
                }
            }
            0x5A | 0x5E => {
                // BFMOVZ / BFMOV #slsb, #dlsb, #width, Rs, Rd
                let imm = self.fetch_u(bus, 2)?;
                let dlsb = (imm >> 5) & 0x1F;
                let slsb = dlsb.wrapping_sub(imm & 0x1F) & 0x1F;
                // The end bit (dlsb + width) is held modulo 32: a field
                // ending at bit 31 encodes 0.
                let width = ((imm >> 10).wrapping_sub(dlsb) & 0x1F) as u64;
                let width = if width == 0 { 32 } else { width };
                let mask = ((1u64 << width) - 1) as u32;
                let field = (self.r[hi as usize] >> slsb) & mask;
                let rd = lo as usize;
                let keep = if b1 == 0x5E {
                    self.r[rd] & !(mask << dlsb)
                } else {
                    0
                };
                self.r[rd] = keep | (field << dlsb);
            }
            0x60..=0x6F => {
                // BSET/BCLR/BTST/BNOT Rs, dest (dest high nibble, bit low)
                let op = (b1 >> 2) & 3;
                let bitreg = self.r[lo as usize];
                let (loc, bit, sz) = if ld == 3 {
                    (Loc::Reg(hi as usize), bitreg & 31, Sz::L)
                } else {
                    (self.ea(bus, ld, hi, Sz::UB)?, bitreg & 7, Sz::UB)
                };
                self.bitop(bus, op, loc, bit, sz)?;
            }
            0x80..=0x9B | 0xA0..=0xA7 => {
                let loc = self.ea(bus, ld, hi, Sz::L)?;
                let v = self.src(bus, loc, Sz::L)?;
                let rd = lo as usize;
                let fv = f32::from_bits(v);
                let fd = f32::from_bits(self.r[rd]);
                match b1 >> 2 {
                    0x20 => self.r[rd] = self.fop(1, fd, fv),
                    0x21 => self.fcmp(fd, fv),
                    0x22 => self.r[rd] = self.fop(2, fd, fv),
                    0x23 => self.r[rd] = self.fop(3, fd, fv),
                    0x24 => self.r[rd] = self.fop(4, fd, fv),
                    0x25 => {
                        let r = fv.trunc() as i32 as u32;
                        self.set_sz(r);
                        self.r[rd] = r;
                    }
                    0x26 => {
                        let r = fv.round_ties_even() as i32 as u32;
                        self.set_sz(r);
                        self.r[rd] = r;
                    }
                    0x28 => {
                        let r = fv.sqrt();
                        self.set_sz_f(r);
                        self.r[rd] = r.to_bits();
                    }
                    0x29 => {
                        let r = fv.trunc() as u32;
                        self.set_sz(r);
                        self.r[rd] = r;
                    }
                    _ => return Ok(Flow::Undefined),
                }
            }
            0xD0..=0xDF => {
                // SCCnd.size dest
                let sz = Sz::bwl((b1 >> 2) & 3);
                let v = u32::from(self.cond(lo));
                let loc = self.ea(bus, ld, hi, sz)?;
                self.put(bus, loc, sz, v)?;
            }
            0xE0..=0xFF => {
                // BMCnd / BNOT #imm3, [Rd].B
                let bit = (b1 >> 2) & 7;
                let loc = self.ea(bus, ld, hi, Sz::UB)?;
                let op = if lo == 0xF { 3 } else { 4 + lo };
                self.bitop(bus, op, loc, bit, Sz::UB)?;
            }
            _ => return Ok(Flow::Undefined),
        }
        Ok(Flow::Next)
    }

    /// 0 BSET, 1 BCLR, 2 BTST, 3 BNOT, 4+cd BMCnd.
    fn bitop(&mut self, bus: &mut dyn Bus, op: u32, loc: Loc, bit: u32, sz: Sz) -> SimResult<()> {
        let v = self.src(bus, loc, sz)?;
        let m = 1u32 << bit;
        let r = match op {
            0 => v | m,
            1 => v & !m,
            2 => {
                let set = v & m != 0;
                self.put_flag(PSW_C, set);
                self.put_flag(PSW_Z, !set);
                return Ok(());
            }
            3 => v ^ m,
            _ => {
                if self.cond(op - 4) {
                    v | m
                } else {
                    v & !m
                }
            }
        };
        self.put(bus, loc, sz, r)
    }

    fn exec_fd(&mut self, bus: &mut dyn Bus) -> SimResult<Flow> {
        let b1 = self.fetch8(bus)?;
        let b2 = self.fetch8(bus)?;
        let (hi, lo) = (b2 >> 4, b2 & 0xF);
        match b1 {
            0x00..=0x0F | 0x44..=0x47 | 0x4C..=0x4F => {
                let a = ((b1 >> 3) & 1) as usize;
                let (x, y) = (self.r[hi as usize], self.r[lo as usize]);
                let hi16 = |v: u32| i128::from((v >> 16) as u16 as i16);
                let lo16 = |v: u32| i128::from(v as u16 as i16);
                let op = (b1 & 7) | if b1 >= 0x40 { 8 } else { 0 };
                let (prod, mode) = match op {
                    0 => ((hi16(x) * hi16(y)) << 16, 0),                    // MULHI
                    1 => ((lo16(x) * lo16(y)) << 16, 0),                    // MULLO
                    2 => ((lo16(x) * hi16(y)) << 16, 0),                    // MULLH
                    3 => (i128::from(x as i32) * i128::from(y as i32), 0),  // EMULA
                    4 => ((hi16(x) * hi16(y)) << 16, 1),                    // MACHI
                    5 => ((lo16(x) * lo16(y)) << 16, 1),                    // MACLO
                    6 => ((lo16(x) * hi16(y)) << 16, 1),                    // MACLH
                    7 => (i128::from(x as i32) * i128::from(y as i32), 1),  // EMACA
                    12 => ((hi16(x) * hi16(y)) << 16, 2),                   // MSBHI
                    13 => ((lo16(x) * lo16(y)) << 16, 2),                   // MSBLO
                    14 => ((lo16(x) * hi16(y)) << 16, 2),                   // MSBLH
                    15 => (i128::from(x as i32) * i128::from(y as i32), 2), // EMSBA
                    _ => return Ok(Flow::Undefined),
                };
                self.acc[a] = acc72(match mode {
                    0 => prod,
                    1 => self.acc[a] + prod,
                    _ => self.acc[a] - prod,
                });
            }
            0x17 => {
                let a = (b2 >> 7) as usize;
                let v = self.r[lo as usize];
                let acc = self.acc[a];
                self.acc[a] = match (b2 >> 4) & 7 {
                    0 => (acc & 0xFFFF_FFFF) | (i128::from(v as i32) << 32),
                    1 => (acc & !0xFFFF_FFFF) | i128::from(v),
                    3 => (acc & 0xFFFF_FFFF_FFFF_FFFF) | (i128::from(v as u8 as i8) << 64),
                    _ => return Ok(Flow::Undefined),
                };
            }
            0x18 | 0x19 => {
                // RACW/RDACW (18) and RACL/RDACL (19)
                if lo != 0 {
                    return Ok(Flow::Undefined);
                }
                let a = (b2 >> 7) as usize;
                let shift = ((b2 >> 4) & 1) + 1;
                let round = (b2 >> 6) & 1 == 0;
                let mut t = self.acc[a] << shift;
                if round {
                    t += 0x8000_0000;
                }
                let (max, min) = if b1 == 0x18 {
                    (0x0000_7FFF_0000_0000i128, -0x0000_8000_0000_0000i128)
                } else {
                    (0x7FFF_FFFF_0000_0000i128, -0x8000_0000_0000_0000i128)
                };
                self.acc[a] = if t > max {
                    max
                } else if t < min {
                    min
                } else {
                    t & !0xFFFF_FFFF
                };
            }
            0x1E | 0x1F => {
                // MVFACHI/MVFACLO/MVFACMI/MVFACGU #imm, A, Rd
                let a = (b2 >> 7) as usize;
                let imm = ((((b1 & 1) << 1) | ((b2 >> 6) & 1)) ^ 2) & 3;
                let t = self.acc[a] << imm;
                self.r[lo as usize] = match (b2 >> 4) & 3 {
                    0 => (t >> 32) as u32,
                    1 => t as u32,
                    2 => (t >> 16) as u32,
                    _ => ((t >> 64) as i8) as i32 as u32,
                };
            }
            0x20..=0x2F | 0x38..=0x3F => {
                let sz_bits = b1 & 3;
                let pre = b1 & 4 != 0;
                if b1 == 0x27 {
                    // MOVCO Rs, [Rd]  (Rd high nibble)
                    let (rd, rs) = (hi as usize, lo as usize);
                    if self.li {
                        let addr = self.r[rd];
                        let v = self.r[rs];
                        self.write(bus, addr, 4, v)?;
                        self.r[rs] = 0;
                    } else {
                        self.r[rs] = 1;
                    }
                    self.li = false;
                    return Ok(Flow::Next);
                }
                if b1 == 0x2F {
                    // MOVLI [Rs], Rd
                    let v = self.read(bus, self.r[hi as usize], 4)?;
                    self.r[lo as usize] = v;
                    self.li = true;
                    return Ok(Flow::Next);
                }
                if sz_bits == 3 || (b1 >= 0x38 && sz_bits > 1) {
                    return Ok(Flow::Undefined);
                }
                let load = b1 >= 0x28;
                let sz = if b1 >= 0x38 {
                    if sz_bits & 1 != 0 {
                        Sz::UW
                    } else {
                        Sz::UB
                    }
                } else {
                    Sz::bwl(sz_bits)
                };
                let n = sz.bytes();
                // Store: base high, value low. Load: base high, dest low.
                let base = hi as usize;
                if pre {
                    self.r[base] = self.r[base].wrapping_sub(n);
                }
                let addr = self.r[base];
                if load {
                    let v = self.load(bus, addr, sz)?;
                    if !pre {
                        self.r[base] = self.r[base].wrapping_add(n);
                    }
                    self.r[lo as usize] = v;
                } else {
                    let v = self.r[lo as usize];
                    self.store(bus, addr, sz, v)?;
                    if !pre {
                        self.r[base] = self.r[base].wrapping_add(n);
                    }
                }
            }
            0x60..=0x67 => {
                let (rs, rd) = (hi as usize, lo as usize);
                let n = self.r[rs];
                let v = self.r[rd];
                self.r[rd] = match b1 {
                    0x60 => self.shlr(v, n & 31),
                    0x61 => self.shar(v, n & 31),
                    0x62 => self.shll(v, n & 31),
                    0x64 => self.rotr(v, n & 31),
                    0x65 => {
                        let v = self.r[rs];
                        ((v & 0x00FF_00FF) << 8) | ((v >> 8) & 0x00FF_00FF)
                    }
                    0x66 => self.rotl(v, n & 31),
                    0x67 => self.r[rs].swap_bytes(),
                    _ => return Ok(Flow::Undefined),
                };
            }
            0x68 | 0x69 => {
                // MVTC Rs, CR
                let cr = ((b1 & 1) << 4) | lo;
                if !self.write_cr(cr, self.r[hi as usize]) {
                    return Ok(Flow::Undefined);
                }
            }
            0x6A | 0x6B => {
                // MVFC CR, Rd
                let cr = ((b1 & 1) << 4) | hi;
                let Some(v) = self.read_cr(cr) else {
                    return Ok(Flow::Undefined);
                };
                self.r[lo as usize] = v;
            }
            0x6C..=0x6F => {
                let imm = ((b1 & 1) << 4) | hi;
                let rd = lo as usize;
                let v = self.r[rd];
                self.r[rd] = if b1 >= 0x6E {
                    self.rotl(v, imm)
                } else {
                    self.rotr(v, imm)
                };
            }
            0x70..=0x7F => {
                let li = (b1 >> 2) & 3;
                match b1 & 3 {
                    0 => {
                        let imm = self.fetch_li(bus, li)?;
                        if !self.alu_ext(hi, lo as usize, imm) {
                            return Ok(Flow::Undefined);
                        }
                    }
                    2 if li == 0 => {
                        // FSUB/FCMP/FADD/FMUL/FDIV #imm32, Rd
                        let imm = f32::from_bits(self.fetch_u(bus, 4)?);
                        let rd = lo as usize;
                        let fd = f32::from_bits(self.r[rd]);
                        match hi {
                            0 => self.r[rd] = self.fop(1, fd, imm),
                            1 => self.fcmp(fd, imm),
                            2 => self.r[rd] = self.fop(2, fd, imm),
                            3 => self.r[rd] = self.fop(3, fd, imm),
                            4 => self.r[rd] = self.fop(4, fd, imm),
                            _ => return Ok(Flow::Undefined),
                        }
                    }
                    3 if hi & 0xE == 0 => {
                        // MVTC #imm, CR
                        let imm = self.fetch_li(bus, li)?;
                        let cr = ((hi & 1) << 4) | lo;
                        if !self.write_cr(cr, imm) {
                            return Ok(Flow::Undefined);
                        }
                    }
                    _ => return Ok(Flow::Undefined),
                }
            }
            0x80..=0xDF => {
                // SHLR/SHAR/SHLL #imm5, Rs, Rd
                let imm = b1 & 0x1F;
                let v = self.r[hi as usize];
                let r = match b1 >> 5 {
                    4 => self.shlr(v, imm),
                    5 => self.shar(v, imm),
                    _ => self.shll(v, imm),
                };
                self.r[lo as usize] = r;
            }
            0xE0..=0xFF => {
                // BMCnd / BNOT #imm5, Rd (cond high nibble, Rd low)
                let bit = b1 & 0x1F;
                let op = if hi == 0xF { 3 } else { 4 + hi };
                self.bitop(bus, op, Loc::Reg(lo as usize), bit, Sz::L)?;
            }
            _ => return Ok(Flow::Undefined),
        }
        Ok(Flow::Next)
    }

    // ---- shifts and unary ops --------------------------------------------

    fn shll(&mut self, v: u32, n: u32) -> u32 {
        let (r, c, o) = if n == 0 {
            (v, false, false)
        } else {
            let r = v.checked_shl(n).unwrap_or(0);
            let c = (v >> (32 - n)) & 1 != 0;
            // O: the bits shifted out and the new MSB (the top n + 1 bits
            // of the operand) are not all equal.
            let top = u64::from(v >> (31 - n));
            let all = (1u64 << (n + 1)) - 1;
            let o = top != 0 && top != all;
            (r, c, o)
        };
        self.put_flag(PSW_C, c);
        self.put_flag(PSW_O, o);
        self.set_sz(r);
        r
    }

    fn shlr(&mut self, v: u32, n: u32) -> u32 {
        let (r, c) = if n == 0 {
            (v, false)
        } else {
            (v.checked_shr(n).unwrap_or(0), (v >> (n - 1)) & 1 != 0)
        };
        self.put_flag(PSW_C, c);
        self.put_flag(PSW_O, false);
        self.set_sz(r);
        r
    }

    fn shar(&mut self, v: u32, n: u32) -> u32 {
        let (r, c) = if n == 0 {
            (v, false)
        } else {
            (
                ((v as i32) >> n.min(31)) as u32,
                ((v as i32) >> (n - 1)) & 1 != 0,
            )
        };
        self.put_flag(PSW_C, c);
        self.put_flag(PSW_O, false);
        self.set_sz(r);
        r
    }

    fn rotl(&mut self, v: u32, n: u32) -> u32 {
        let r = v.rotate_left(n);
        self.put_flag(PSW_C, r & 1 != 0);
        self.set_sz(r);
        r
    }

    fn rotr(&mut self, v: u32, n: u32) -> u32 {
        let r = v.rotate_right(n);
        self.put_flag(PSW_C, r & 0x8000_0000 != 0);
        self.set_sz(r);
        r
    }

    fn neg(&mut self, v: u32) -> u32 {
        let r = v.wrapping_neg();
        self.put_flag(PSW_O, v == 0x8000_0000);
        self.put_flag(PSW_C, r == 0);
        self.set_sz(r);
        r
    }

    fn abs(&mut self, v: u32) -> u32 {
        let r = (v as i32).wrapping_abs() as u32;
        self.put_flag(PSW_O, v == 0x8000_0000);
        self.set_sz(r);
        r
    }

    // ---- FPU -------------------------------------------------------------

    fn set_sz_f(&mut self, f: f32) {
        self.put_flag(PSW_Z, f == 0.0);
        self.put_flag(PSW_S, f.is_sign_negative() && !f.is_nan());
    }

    /// 1 SUB, 2 ADD, 3 MUL, 4 DIV: `a op b`.
    fn fop(&mut self, op: u32, a: f32, b: f32) -> u32 {
        let r = match op {
            1 => a - b,
            2 => a + b,
            3 => a * b,
            _ => a / b,
        };
        self.set_sz_f(r);
        r.to_bits()
    }

    fn fcmp(&mut self, a: f32, b: f32) {
        let unordered = a.is_nan() || b.is_nan();
        self.put_flag(PSW_O, unordered);
        self.put_flag(PSW_Z, !unordered && a == b);
        self.put_flag(PSW_S, !unordered && a < b);
    }
}

impl Cpu for Rx {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        let icu_base = self.icu_base;
        *self = Self::new();
        self.icu_base = icu_base;
        self.pc = bus.read_u32(u64::from(RESET_VECTOR))?;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &crate::SimulationConfig,
    ) -> SimResult<()> {
        if self.waiting {
            self.cycles = self.cycles.wrapping_add(1);
            self.take_interrupt(bus)?;
            return Ok(());
        }
        let pc = self.pc;
        self.ipc = pc;
        self.ilen = 0;
        let opcode = if observers.is_empty() {
            0
        } else {
            let mut op = 0u32;
            for i in 0..4 {
                op |= u32::from(bus.read_u8(u64::from(pc.wrapping_add(i)))?) << (8 * i);
            }
            for o in observers {
                o.on_step_start(pc, op);
            }
            op
        };
        let flow = self.exec(bus)?;
        let next = pc.wrapping_add(self.ilen);
        match flow {
            Flow::Next => self.pc = next,
            Flow::Jump(t) => self.pc = t,
            Flow::Trap(n) => {
                let vaddr = self.intb.wrapping_add(n * 4);
                self.enter(bus, vaddr, next, None)?;
            }
            Flow::Undefined => {
                // The guest sees an architectural exception; the host log
                // line is what `scripts/unsupported_instruction_audit.sh`
                // counts.
                let mut bytes = 0u32;
                for i in 0..4 {
                    let b = bus.read_u8(u64::from(pc.wrapping_add(i)))?;
                    bytes |= u32::from(b) << (8 * i);
                }
                tracing::warn!("Unknown instruction {:#x} at {:#x}", bytes, pc);
                let vaddr = self.extb.wrapping_add(EXC_UNDEFINED);
                self.enter(bus, vaddr, pc, None)?;
            }
        }
        self.cycles = self.cycles.wrapping_add(1);
        if !observers.is_empty() {
            crate::emit_trace_event(
                observers,
                labwired_hw_trace::TraceEvent::InstructionRetired { pc, opcode },
            );
        }
        self.take_interrupt(bus)?;
        if !observers.is_empty() {
            let regs = self.observer_registers();
            for o in observers {
                o.on_step_end(1, &regs);
            }
        }
        Ok(())
    }

    /// The loader must not fall back to Cortex-M vector tables;
    /// [`crate::system::rx::load_elf`] then starts the core at the reset
    /// vector.
    fn uses_elf_entry_point(&self) -> bool {
        true
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }
    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_sp(&mut self, val: u32) {
        self.r[0] = val;
    }

    /// A peripheral requests interrupt vector `n`: set its IR flag.
    fn set_exception_pending(&mut self, exception_num: u32) {
        if exception_num < 256 {
            self.icu.set_ir(exception_num);
        }
    }

    fn idle_fast_forward_budget(&self, _bus: &dyn Bus) -> Option<u64> {
        (self.waiting && self.acceptable_interrupt().is_none()).then_some(u64::MAX)
    }

    fn fast_forward_idle_cycles(&mut self, cycles: u64) {
        self.cycles = self.cycles.wrapping_add(cycles);
    }

    fn get_register(&self, id: u8) -> u32 {
        match id {
            0..=15 => self.r[id as usize],
            16 => self.psw,
            17 => self.pc,
            _ => 0,
        }
    }
    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0..=15 => self.r[id as usize] = val,
            16 => self.set_psw(val),
            17 => self.pc = val,
            _ => {}
        }
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::Rx(Box::new(RxCpuSnapshot {
            r: self.r.to_vec(),
            pc: self.pc,
            psw: self.psw,
            usp: self.usp,
            isp: self.isp,
            intb: self.intb,
            extb: self.extb,
            bpsw: self.bpsw,
            bpc: self.bpc,
            fintv: self.fintv,
            fpsw: self.fpsw,
            acc: self.acc.to_vec(),
            icu: self.icu.clone(),
            waiting: self.waiting,
            cycles: self.cycles,
        }))
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::Rx(s) = snapshot {
            for (dst, src) in self.r.iter_mut().zip(s.r.iter()) {
                *dst = *src;
            }
            self.pc = s.pc;
            self.psw = s.psw & PSW_WRITABLE;
            self.usp = s.usp;
            self.isp = s.isp;
            self.intb = s.intb;
            self.extb = s.extb;
            self.bpsw = s.bpsw;
            self.bpc = s.bpc;
            self.fintv = s.fintv;
            self.fpsw = s.fpsw;
            for (dst, src) in self.acc.iter_mut().zip(s.acc.iter()) {
                *dst = acc72(*src);
            }
            if s.icu.regs.len() == ICU_SIZE as usize {
                self.icu = s.icu.clone();
            }
            self.waiting = s.waiting;
            self.cycles = s.cycles;
            self.li = false;
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names: Vec<String> = (0..16).map(|i| format!("r{i}")).collect();
        names.push("psw".to_string());
        names.push("pc".to_string());
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        let n = name.to_ascii_lowercase();
        match n.as_str() {
            "sp" => Some(0),
            "psw" => Some(16),
            "pc" => Some(17),
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
#[path = "rx_tests.rs"]
mod tests;
