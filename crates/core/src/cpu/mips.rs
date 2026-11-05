// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! MIPS32 release 2 CPU (the MIPS M5150 / microAptiv class core in the
//! Microchip PIC32MZ EF) with the PIC32MZ interrupt controller (EVIC)
//! folded into the core.
//!
//! What is modelled, and where it comes from (MIPS32 Architecture for
//! Programmers Vol. II: The MIPS32 Instruction Set, MD00086; Vol. III: The
//! MIPS32 Privileged Resource Architecture, MD00090; PIC32MZ EF data sheet
//! DS60001320, "CPU" and "Interrupt Controller" chapters; Apache NuttX 12.11
//! `arch/mips/src/pic32mz` for the register offsets). Encodings were checked
//! against the Zephyr SDK 0.17.0 `mips-zephyr-elf` assembler
//! (`-march=mips32r2 -EL`):
//!
//! * **Integer ISA**: every MIPS32r2 base instruction, little-endian:
//!   ALU/shift/rotate, `MULT`/`DIV` and the `SPECIAL2` `MUL`/`MADD`/`MSUB`/
//!   `CLZ`/`CLO`, `EXT`/`INS`/`WSBH`/`SEB`/`SEH`, `RDHWR`, every branch and
//!   jump with its delay slot (branch-likely nullifies a not-taken slot),
//!   the trap instructions, `LWL`/`LWR`/`SWL`/`SWR`, `LL`/`SC`, `SYSCALL`,
//!   `BREAK`, `SYNC`/`SYNCI`/`PREF`/`CACHE` (no-ops: caches are not
//!   modelled), `ADD`/`ADDI`/`SUB` overflow traps.
//! * **CP0**: Status, Cause, EPC, ErrorEPC, BadVAddr, Count/Compare (Count
//!   advances every second core cycle, as on the PIC32MZ), EBase, IntCtl,
//!   SRSCtl/SRSMap, HWREna, UserLocal, LLAddr, PRId, Config0-3/7, DESAVE;
//!   the remaining CP0 registers are plain storage. `DI`/`EI`, `ERET`,
//!   `WAIT`, `EHB`, `RDPGPR`/`WRPGPR`.
//! * **Shadow register sets**: eight GPR sets (SRSCtl.HSS = 7). Exception
//!   entry selects SRSCtl.ESS, interrupt entry the EVIC's shadow set
//!   (PRISS), and `ERET` returns to SRSCtl.PSS.
//! * **Exceptions**: interrupt, address error (AdEL/AdES, including user
//!   access above 0x80000000), TLB refill for every mapped address (no TLB
//!   entry is ever valid), `SYSCALL`, `BREAK`, reserved instruction,
//!   coprocessor unusable, overflow, trap. Vectors follow Status.BEV, EBase
//!   and Cause.IV; EPC and Cause.BD account for branch delay slots.
//! * **EVIC** (PIC32MZ, physical 0x1F810000): INTCON, PRISS, INTSTAT,
//!   IPTMR, IFS0-7, IEC0-7, IPC0-63 (each with its CLR/SET/INV aliases at
//!   +4/+8/+C) and OFF000-OFF213. The core runs in external interrupt
//!   controller (EIC) mode: the request level RIPL is the highest IPC
//!   priority among the flagged and enabled sources, taken when it exceeds
//!   Status.IPL with IE set and EXL/ERL clear. Single-vector mode
//!   (INTCON.MVEC = 0) vectors to EBase + 0x200; multi-vector mode to
//!   EBase + OFFn. INTSTAT reports the request the core would take next. A
//!   peripheral requests interrupt `n` through [`Cpu::set_exception_pending`]
//!   (that sets IFSn); the core timer is IRQ 0 and the core software
//!   interrupts are IRQs 1 and 2, flagged when Cause.IP0/IP1 is written
//!   from 0 to 1.
//!
//! Not modelled yet, and refused rather than guessed: the TLB itself
//! (`TLBR`/`TLBWI`/`TLBWR`/`TLBP` are reserved instructions), microMIPS
//! (an image whose DEVCFG0.BOOTISA selects microMIPS is refused at reset),
//! the FPU (CP1 instructions are reserved once Status.CU1 is set, and
//! coprocessor-unusable before), the DSP ASE, EJTAG (`SDBBP`, `DERET`),
//! caches and the prefetch module's wait states (one cycle per
//! instruction), the interrupt proximity timer, and bus error exceptions
//! (an access to unmapped space stops the simulation instead).

use crate::{Bus, Cpu, SimResult, SimulationObserver};
use std::sync::Arc;

/// Status register bits (PRA, Table 9.32).
const ST_IE: u32 = 1 << 0;
const ST_EXL: u32 = 1 << 1;
const ST_ERL: u32 = 1 << 2;
const ST_UM: u32 = 1 << 4;
const ST_IPL_SHIFT: u32 = 10;
const ST_IPL_MASK: u32 = 0x3F << ST_IPL_SHIFT;
const ST_NMI: u32 = 1 << 19;
const ST_SR: u32 = 1 << 20;
const ST_BEV: u32 = 1 << 22;
const ST_CU0: u32 = 1 << 28;
const ST_CU1: u32 = 1 << 29;
/// CU1/CU0, RP, FR, BEV, SR, NMI, IM/IPL, UM, ERL, EXL, IE.
const ST_WRITABLE: u32 = 0x3C58_FF17;

/// Cause register bits.
const CA_BD: u32 = 1 << 31;
const CA_TI: u32 = 1 << 30;
const CA_CE_SHIFT: u32 = 28;
const CA_DC: u32 = 1 << 27;
const CA_IV: u32 = 1 << 23;
const CA_WP: u32 = 1 << 22;
const CA_RIPL_SHIFT: u32 = 10;
const CA_IP_SW: u32 = 0x300;
const CA_WRITABLE: u32 = CA_DC | CA_IV | CA_WP | CA_IP_SW;

/// Exception codes (PRA, Table 9.37).
const EXC_INT: u32 = 0;
const EXC_TLBL: u32 = 2;
const EXC_TLBS: u32 = 3;
const EXC_ADEL: u32 = 4;
const EXC_ADES: u32 = 5;
const EXC_SYS: u32 = 8;
const EXC_BP: u32 = 9;
const EXC_RI: u32 = 10;
const EXC_CPU: u32 = 11;
const EXC_OV: u32 = 12;
const EXC_TR: u32 = 13;

/// The reset, soft reset and NMI vector.
pub const RESET_VECTOR: u32 = 0xBFC0_0000;
/// Exception vector base while Status.BEV is set.
const BEV_BASE: u32 = 0xBFC0_0200;
/// EBase at reset. The PIC32MZ boot flash holds the exception vectors, and
/// NuttX's PIC32MZ port (which never writes EBase in single-vector mode)
/// places its interrupt vector at 0xBFC00200: EBase resets into boot flash.
const EBASE_RESET: u32 = 0xBFC0_0000;
/// PRId of the PIC32MZ EF's M5150 core: company 0x01 (MIPS), processor
/// 0xA7. The revision field is not modelled (0).
const PRID: u32 = 0x0001_A700;
/// Config0: M (Config1 follows), AR = 1 (release 2), MT = 1 (TLB), K0 = 2
/// (uncached) at reset.
const CONFIG0_RESET: u32 = 0x8000_0482;
/// Config1: M, 16 TLB entries, 16 KiB 4-way 16-byte-line I-cache, 4 KiB
/// 4-way 16-byte-line D-cache, EJTAG present, FPU present (PIC32MZ EF).
const CONFIG1: u32 = (1 << 31)
    | (15 << 25)
    | (2 << 22)
    | (3 << 19)
    | (3 << 16)
    | (3 << 10)
    | (3 << 7)
    | (1 << 1)
    | 1;
/// Config2: M (Config3 follows).
const CONFIG2: u32 = 1 << 31;
/// Config3: ISA = 2 (MIPS32 and microMIPS, MIPS32 at reset), ULRI (UserLocal),
/// DSPP, VEIC (external interrupt controller), VInt.
const CONFIG3_RESET: u32 = (2 << 14) | (1 << 13) | (1 << 10) | (1 << 6) | (1 << 5);
const CONFIG3_ISAONEXC: u32 = 1 << 16;
/// Highest shadow register set (eight sets on the PIC32MZ).
const HSS: u32 = 7;

/// EVIC physical base and window (PIC32MZ EF, SFR base + 0x10000).
pub const EVIC_BASE: u32 = 0x1F81_0000;
const EVIC_SIZE: u32 = 0x1000;
const EV_INTCON: u32 = 0x000;
const EV_PRISS: u32 = 0x010;
const EV_INTSTAT: u32 = 0x020;
const EV_IPTMR: u32 = 0x030;
const EV_IFS: u32 = 0x040;
const EV_IEC: u32 = 0x0C0;
const EV_IPC: u32 = 0x140;
const EV_OFF: u32 = 0x540;
const INTCON_MVEC: u32 = 1 << 12;
/// Interrupt sources the EVIC has (PIC32MZ EF: vectors 0-213).
pub const EVIC_IRQS: u32 = 214;
const IRQ_CORE_TIMER: u32 = 0;
const IRQ_CORE_SW0: u32 = 1;
const IRQ_CORE_SW1: u32 = 2;

/// The PIC32MZ interrupt controller registers the core consults on every
/// step.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pic32Evic {
    pub intcon: u32,
    pub priss: u32,
    pub iptmr: u32,
    pub ifs: [u32; 8],
    pub iec: [u32; 8],
    pub ipc: Vec<u32>,
    pub off: Vec<u32>,
}

impl Default for Pic32Evic {
    fn default() -> Self {
        Self {
            intcon: 0,
            priss: 0,
            iptmr: 0,
            ifs: [0; 8],
            iec: [0; 8],
            ipc: vec![0; 64],
            off: vec![0; EVIC_IRQS as usize],
        }
    }
}

impl Pic32Evic {
    fn flag(&mut self, irq: u32) {
        if irq < EVIC_IRQS {
            self.ifs[(irq / 32) as usize] |= 1 << (irq % 32);
        }
    }

    /// IPC priority (bits 4:2 of the source's byte) and subpriority (1:0).
    fn prio(&self, irq: u32) -> (u32, u32) {
        let b = (self.ipc[(irq / 4) as usize] >> ((irq % 4) * 8)) & 0x1F;
        (b >> 2, b & 3)
    }

    /// The request the EVIC presents to the core: `(irq, priority)` of the
    /// highest-priority flagged and enabled source with a non-zero
    /// priority; ties go to the higher subpriority, then the lower IRQ
    /// number (the natural order).
    fn highest(&self) -> Option<(u32, u32)> {
        if self.ifs.iter().zip(&self.iec).all(|(f, e)| f & e == 0) {
            return None;
        }
        let mut best: Option<(u32, u32, u32)> = None;
        for w in 0..8 {
            let mut bits = self.ifs[w] & self.iec[w];
            while bits != 0 {
                let irq = (w as u32) * 32 + bits.trailing_zeros();
                bits &= bits - 1;
                let (p, s) = self.prio(irq);
                if p == 0 {
                    continue;
                }
                if best.is_none_or(|(_, bp, bs)| (p, s) > (bp, bs)) {
                    best = Some((irq, p, s));
                }
            }
        }
        best.map(|(irq, p, _)| (irq, p))
    }

    /// The shadow register set an interrupt at `level` uses (PRISS: SS0 in
    /// single-vector mode, PRIxSS per level in multi-vector mode).
    fn shadow_set(&self, level: u32) -> u32 {
        if self.intcon & INTCON_MVEC == 0 {
            self.priss & 1
        } else {
            (self.priss >> (4 * level)) & 0xF
        }
    }

    fn intstat(&self) -> u32 {
        self.highest().map_or(0, |(irq, p)| (p << 8) | irq)
    }

    /// The register a word offset names, without its CLR/SET/INV alias.
    fn reg_mut(&mut self, base: u32) -> Option<&mut u32> {
        Some(match base {
            EV_INTCON => &mut self.intcon,
            EV_PRISS => &mut self.priss,
            EV_IPTMR => &mut self.iptmr,
            o if (EV_IFS..EV_IEC).contains(&o) => &mut self.ifs[((o - EV_IFS) / 16) as usize],
            o if (EV_IEC..EV_IPC).contains(&o) => &mut self.iec[((o - EV_IEC) / 16) as usize],
            o if (EV_IPC..EV_OFF).contains(&o) => &mut self.ipc[((o - EV_IPC) / 16) as usize],
            _ => return None,
        })
    }

    fn read(&self, off: u32) -> u32 {
        let w = off & !3;
        if w >= EV_OFF {
            let i = (w - EV_OFF) / 4;
            return self.off.get(i as usize).copied().unwrap_or(0);
        }
        let base = w & !0xF;
        if w != base {
            // The CLR/SET/INV aliases read as zero.
            return 0;
        }
        match base {
            EV_INTSTAT => self.intstat(),
            EV_INTCON => self.intcon,
            EV_PRISS => self.priss,
            EV_IPTMR => self.iptmr,
            o if (EV_IFS..EV_IEC).contains(&o) => self.ifs[((o - EV_IFS) / 16) as usize],
            o if (EV_IEC..EV_IPC).contains(&o) => self.iec[((o - EV_IEC) / 16) as usize],
            o if (EV_IPC..EV_OFF).contains(&o) => self.ipc[((o - EV_IPC) / 16) as usize],
            _ => 0,
        }
    }

    fn write(&mut self, off: u32, v: u32) {
        let w = off & !3;
        if w >= EV_OFF {
            let i = ((w - EV_OFF) / 4) as usize;
            if let Some(slot) = self.off.get_mut(i) {
                // OFFn<17:1>: the vector offset is halfword aligned.
                *slot = v & 0x0003_FFFE;
            }
            return;
        }
        let base = w & !0xF;
        let Some(r) = self.reg_mut(base) else {
            // INTSTAT is read-only.
            return;
        };
        match w & 0xF {
            0x0 => *r = v,
            0x4 => *r &= !v,
            0x8 => *r |= v,
            _ => *r ^= v,
        }
    }
}

/// Serialisable MIPS core state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MipsCpuSnapshot {
    pub gpr: Vec<Vec<u32>>,
    pub hi: u32,
    pub lo: u32,
    pub pc: u32,
    pub npc: u32,
    pub in_delay: bool,
    pub cp0: Vec<u32>,
    pub evic: Pic32Evic,
    pub llbit: bool,
    pub waiting: bool,
    pub cycles: u64,
}

/// How an instruction left the PC.
enum Flow {
    Next,
    /// A taken branch or jump: the delay slot runs, then the target.
    Branch(u32),
    /// A branch-likely that was not taken: the delay slot is skipped.
    Nullify,
    /// `ERET`: no delay slot.
    Jump(u32),
    Exception {
        code: u32,
        badvaddr: Option<u32>,
        ce: u32,
    },
    /// An encoding the model does not implement: reserved-instruction
    /// exception plus the host log line the unsupported-instruction audit
    /// counts.
    Unsupported,
}

fn exc(code: u32) -> Flow {
    Flow::Exception {
        code,
        badvaddr: None,
        ce: 0,
    }
}

/// CP0 storage index for (register, select).
const fn c0(reg: u32, sel: u32) -> usize {
    (reg * 8 + sel) as usize
}
const C0_USERLOCAL: usize = c0(4, 2);
const C0_HWRENA: usize = c0(7, 0);
const C0_BADVADDR: usize = c0(8, 0);
const C0_COUNT: usize = c0(9, 0);
const C0_COMPARE: usize = c0(11, 0);
const C0_STATUS: usize = c0(12, 0);
const C0_INTCTL: usize = c0(12, 1);
const C0_SRSCTL: usize = c0(12, 2);
const C0_SRSMAP: usize = c0(12, 3);
const C0_CAUSE: usize = c0(13, 0);
const C0_EPC: usize = c0(14, 0);
const C0_PRID: usize = c0(15, 0);
const C0_EBASE: usize = c0(15, 1);
const C0_CONFIG0: usize = c0(16, 0);
const C0_CONFIG1: usize = c0(16, 1);
const C0_CONFIG2: usize = c0(16, 2);
const C0_CONFIG3: usize = c0(16, 3);
const C0_LLADDR: usize = c0(17, 0);
const C0_ERROREPC: usize = c0(30, 0);

/// MIPS32r2 core with the PIC32MZ EVIC.
#[derive(Debug, Clone)]
pub struct Mips32 {
    /// Eight shadow GPR sets; `gpr[css]` is the current one.
    pub gpr: [[u32; 32]; 8],
    pub hi: u32,
    pub lo: u32,
    pub pc: u32,
    /// The address after `pc` (the delay-slot / branch target pipeline).
    pub npc: u32,
    /// `pc` holds a branch delay slot.
    pub in_delay: bool,
    pub cp0: [u32; 256],
    pub evic: Pic32Evic,
    pub llbit: bool,
    pub waiting: bool,
    pub cycles: u64,
    evic_base: u32,
    /// Instruction words fetched from program/boot flash, direct-mapped by
    /// physical address (see [`Self::fetch`]).
    fetch_cache: Vec<(u32, u32)>,
    /// Idle-loop proof (see [`Self::note_backward_branch`]): the register
    /// state at the last few backward branches.
    loop_anchors: [Option<LoopAnchor>; 4],
    loop_next: usize,
    /// Bumped by every load, store, CP0/hardware-register access and
    /// exception.
    side_effect: bool,
    epoch: u64,
    /// The core is in a proven fixed-point loop: nothing changes until an
    /// interrupt, so idle time may be skipped as for `WAIT`.
    idle_loop: bool,
}

/// Direct-mapped fetch cache size (words).
const FETCH_CACHE_LINES: usize = 1 << 14;

/// Physical windows the CPU only reads: program flash and boot flash (the
/// PIC32MZ NVM controller that programs them is not modelled).
fn fetch_cacheable(pa: u32) -> bool {
    (0x1D00_0000..0x1E00_0000).contains(&pa) || (0x1FC0_0000..0x1FC8_0000).contains(&pa)
}

/// Register state at a backward branch, to prove a loop is a fixed point.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LoopAnchor {
    epoch: u64,
    pc: u32,
    css: usize,
    gpr: [u32; 32],
    hi: u32,
    lo: u32,
}

impl Default for Mips32 {
    fn default() -> Self {
        Self::new()
    }
}

fn sext16(v: u32) -> u32 {
    v as u16 as i16 as i32 as u32
}

impl Mips32 {
    pub fn new() -> Self {
        let mut cp0 = [0u32; 256];
        cp0[C0_STATUS] = ST_BEV | ST_ERL;
        cp0[C0_PRID] = PRID;
        cp0[C0_EBASE] = EBASE_RESET;
        cp0[C0_CONFIG0] = CONFIG0_RESET;
        cp0[C0_CONFIG1] = CONFIG1;
        cp0[C0_CONFIG2] = CONFIG2;
        cp0[C0_CONFIG3] = CONFIG3_RESET;
        cp0[C0_SRSCTL] = HSS << 26;
        Self {
            gpr: [[0; 32]; 8],
            hi: 0,
            lo: 0,
            pc: RESET_VECTOR,
            npc: RESET_VECTOR.wrapping_add(4),
            in_delay: false,
            cp0,
            evic: Pic32Evic::default(),
            llbit: false,
            waiting: false,
            cycles: 0,
            evic_base: EVIC_BASE,
            fetch_cache: vec![(u32::MAX, 0); FETCH_CACHE_LINES],
            loop_anchors: [None, None, None, None],
            loop_next: 0,
            side_effect: false,
            epoch: 0,
            idle_loop: false,
        }
    }

    pub fn for_chip(_chip: &labwired_config::ChipDescriptor) -> Self {
        Self::new()
    }

    pub fn status(&self) -> u32 {
        self.cp0[C0_STATUS]
    }

    pub fn cause(&self) -> u32 {
        self.cp0[C0_CAUSE]
    }

    fn css(&self) -> usize {
        (self.cp0[C0_SRSCTL] & 0xF) as usize
    }

    fn pss(&self) -> usize {
        ((self.cp0[C0_SRSCTL] >> 6) & 0xF) as usize
    }

    /// GPR `r` of the current shadow set.
    pub fn reg(&self, r: u32) -> u32 {
        self.gpr[self.css()][r as usize]
    }

    fn set_reg(&mut self, r: u32, v: u32) {
        if r != 0 {
            let css = self.css();
            self.gpr[css][r as usize] = v;
        }
    }

    /// Switch the current shadow set to `new`, saving the old one in PSS.
    fn enter_shadow_set(&mut self, new: u32) {
        let s = self.cp0[C0_SRSCTL];
        let css = s & 0xF;
        let new = new.min(HSS);
        self.cp0[C0_SRSCTL] = (s & !0x3CF) | (css << 6) | new;
    }

    fn kernel_mode(&self) -> bool {
        let st = self.status();
        st & ST_UM == 0 || st & (ST_EXL | ST_ERL) != 0
    }

    // ---- memory ----------------------------------------------------------

    /// Translate a virtual address: KSEG0/KSEG1 are the fixed unmapped
    /// windows; KUSEG is unmapped only while Status.ERL is set; every other
    /// address needs a TLB entry and none is ever valid.
    fn translate(&self, va: u32, store: bool) -> Result<u32, Flow> {
        let ae = |code| Flow::Exception {
            code,
            badvaddr: Some(va),
            ce: 0,
        };
        if va >= 0x8000_0000 && !self.kernel_mode() {
            return Err(ae(if store { EXC_ADES } else { EXC_ADEL }));
        }
        match va {
            0x8000_0000..=0xBFFF_FFFF => Ok(va & 0x1FFF_FFFF),
            0..=0x7FFF_FFFF if self.status() & ST_ERL != 0 => Ok(va),
            _ => Err(ae(if store { EXC_TLBS } else { EXC_TLBL })),
        }
    }

    /// Fetch the instruction word at physical `pa`. Flash words are cached;
    /// [`Cpu::invalidate_code_caches`] drops the cache when something else
    /// rewrites code (a debugger, a reload).
    fn fetch(&mut self, bus: &mut dyn Bus, pa: u32) -> SimResult<u32> {
        if !fetch_cacheable(pa) {
            return bus.read_u32(u64::from(pa));
        }
        let line = ((pa >> 2) as usize) & (FETCH_CACHE_LINES - 1);
        let (tag, word) = self.fetch_cache[line];
        if tag == pa {
            return Ok(word);
        }
        let word = bus.read_u32(u64::from(pa))?;
        self.fetch_cache[line] = (pa, word);
        Ok(word)
    }

    fn evic_off(&self, pa: u32) -> Option<u32> {
        let off = pa.wrapping_sub(self.evic_base);
        (off < EVIC_SIZE).then_some(off)
    }

    fn load(&mut self, bus: &mut dyn Bus, va: u32, n: u32) -> SimResult<Result<u32, Flow>> {
        self.side_effect = true;
        if va & (n - 1) != 0 {
            return Ok(Err(Flow::Exception {
                code: EXC_ADEL,
                badvaddr: Some(va),
                ce: 0,
            }));
        }
        let pa = match self.translate(va, false) {
            Ok(pa) => pa,
            Err(f) => return Ok(Err(f)),
        };
        if let Some(off) = self.evic_off(pa) {
            let w = self.evic.read(off);
            let sh = (off & 3) * 8;
            let mask = if n == 4 {
                u32::MAX
            } else {
                (1u32 << (8 * n)) - 1
            };
            return Ok(Ok((w >> sh) & mask));
        }
        let a = u64::from(pa);
        Ok(Ok(match n {
            1 => u32::from(bus.read_u8(a)?),
            2 => u32::from(bus.read_u16(a)?),
            _ => bus.read_u32(a)?,
        }))
    }

    fn store(&mut self, bus: &mut dyn Bus, va: u32, n: u32, v: u32) -> SimResult<Result<(), Flow>> {
        self.side_effect = true;
        if va & (n - 1) != 0 {
            return Ok(Err(Flow::Exception {
                code: EXC_ADES,
                badvaddr: Some(va),
                ce: 0,
            }));
        }
        let pa = match self.translate(va, true) {
            Ok(pa) => pa,
            Err(f) => return Ok(Err(f)),
        };
        if let Some(off) = self.evic_off(pa) {
            let v = if n == 4 {
                v
            } else {
                // Sub-word stores merge into the addressed register.
                let sh = (off & 3) * 8;
                let mask = ((1u32 << (8 * n)) - 1) << sh;
                (self.evic.read(off & !3) & !mask) | ((v << sh) & mask)
            };
            self.evic.write(off & !3, v);
            return Ok(Ok(()));
        }
        let a = u64::from(pa);
        match n {
            1 => bus.write_u8(a, v as u8)?,
            2 => bus.write_u16(a, v as u16)?,
            _ => bus.write_u32(a, v)?,
        }
        Ok(Ok(()))
    }

    // ---- CP0 -------------------------------------------------------------

    fn read_cp0(&self, reg: u32, sel: u32) -> u32 {
        let i = c0(reg, sel);
        match i {
            C0_CAUSE => {
                // Cause.RIPL mirrors the EVIC request while one is pending.
                let ripl = self.evic.highest().map_or(0, |(_, p)| p);
                (self.cp0[i] & !(0x3F << CA_RIPL_SHIFT)) | (ripl << CA_RIPL_SHIFT)
            }
            _ => self.cp0[i],
        }
    }

    fn write_cp0(&mut self, reg: u32, sel: u32, v: u32) {
        let i = c0(reg, sel);
        let old = self.cp0[i];
        self.cp0[i] = match i {
            C0_STATUS => {
                // SR and NMI can only be cleared by software.
                let keep = old & (ST_SR | ST_NMI) & v;
                (old & !ST_WRITABLE) | (v & ST_WRITABLE & !(ST_SR | ST_NMI)) | keep
            }
            C0_CAUSE => (old & !CA_WRITABLE) | (v & CA_WRITABLE),
            C0_COMPARE => {
                self.cp0[C0_CAUSE] &= !CA_TI;
                v
            }
            C0_INTCTL => (old & !0x3E0) | (v & 0x3E0),
            C0_SRSCTL => (old & !0xF3C0) | (v & 0xF3C0),
            C0_SRSMAP => v,
            C0_EBASE => 0x8000_0000 | (v & 0x3FFF_F000),
            C0_CONFIG0 => (old & !7) | (v & 7),
            C0_CONFIG3 => (old & !CONFIG3_ISAONEXC) | (v & CONFIG3_ISAONEXC),
            C0_HWRENA => v & 0x2000_000F,
            C0_PRID | C0_CONFIG1 | C0_CONFIG2 | C0_BADVADDR | C0_LLADDR => old,
            _ => v,
        };
        if i == C0_CAUSE {
            // Cause.IP0/IP1 are the EVIC's core software interrupt sources,
            // latched in IFS0 on the 0 -> 1 edge: NuttX's context switch
            // clears CS0IF before it clears IP0, which only works on
            // silicon if the flag is an edge latch.
            let rose = !old & self.cp0[i];
            if rose & (1 << 8) != 0 {
                self.evic.flag(IRQ_CORE_SW0);
            }
            if rose & (1 << 9) != 0 {
                self.evic.flag(IRQ_CORE_SW1);
            }
        }
    }

    /// Advance the core clock by `n` cycles; Count runs at half rate and a
    /// Count == Compare match raises Cause.TI and the core timer IRQ.
    fn advance(&mut self, n: u64) {
        let before = self.cycles;
        self.cycles = self.cycles.wrapping_add(n);
        let ticks = (self.cycles / 2).wrapping_sub(before / 2);
        if ticks == 0 {
            return;
        }
        let count = self.cp0[C0_COUNT];
        let to_match = self.cp0[C0_COMPARE].wrapping_sub(count).wrapping_sub(1) as u64 + 1;
        self.cp0[C0_COUNT] = count.wrapping_add(ticks as u32);
        if ticks >= to_match {
            self.cp0[C0_CAUSE] |= CA_TI;
            self.evic.flag(IRQ_CORE_TIMER);
        }
    }

    /// Core cycles until Count next equals Compare.
    fn cycles_to_compare(&self) -> u64 {
        let n = self.cp0[C0_COMPARE]
            .wrapping_sub(self.cp0[C0_COUNT])
            .wrapping_sub(1) as u64
            + 1;
        n * 2 - (self.cycles & 1)
    }

    // ---- exceptions and interrupts ---------------------------------------

    fn vector_base(&self) -> u32 {
        if self.status() & ST_BEV != 0 {
            BEV_BASE
        } else {
            self.cp0[C0_EBASE] & 0xFFFF_F000
        }
    }

    /// Synchronous exception entry. `pc` is the faulting instruction.
    fn raise(&mut self, code: u32, badvaddr: Option<u32>, ce: u32) {
        let st = self.status();
        let mut cause = self.cp0[C0_CAUSE] & !(CA_BD | (3 << CA_CE_SHIFT) | 0x7C);
        let exl_was = st & ST_EXL != 0;
        if !exl_was {
            if self.in_delay {
                self.cp0[C0_EPC] = self.pc.wrapping_sub(4);
                cause |= CA_BD;
            } else {
                self.cp0[C0_EPC] = self.pc;
            }
        }
        cause |= (code << 2) | (ce << CA_CE_SHIFT);
        self.cp0[C0_CAUSE] = cause;
        if let Some(va) = badvaddr {
            self.cp0[C0_BADVADDR] = va;
        }
        if !exl_was && st & ST_BEV == 0 && HSS > 0 {
            let ess = (self.cp0[C0_SRSCTL] >> 12) & 0xF;
            self.enter_shadow_set(ess);
        }
        self.cp0[C0_STATUS] = st | ST_EXL;
        let refill = matches!(code, EXC_TLBL | EXC_TLBS) && !exl_was;
        let off = if refill { 0x000 } else { 0x180 };
        let base = self.vector_base();
        // With BEV set, the TLB refill vector is 0xBFC00200 and the general
        // vector 0xBFC00380.
        self.pc = base.wrapping_add(off);
        self.npc = self.pc.wrapping_add(4);
        self.in_delay = false;
        self.waiting = false;
    }

    /// The request the core would take now: `(irq, level)`.
    fn acceptable_interrupt(&self) -> Option<(u32, u32)> {
        let st = self.status();
        if st & ST_IE == 0 || st & (ST_EXL | ST_ERL) != 0 || st & ST_IPL_MASK == ST_IPL_MASK {
            return None;
        }
        let (irq, level) = self.evic.highest()?;
        let ipl = (st & ST_IPL_MASK) >> ST_IPL_SHIFT;
        (level > ipl).then_some((irq, level))
    }

    fn take_interrupt(&mut self) -> bool {
        // An interrupt is taken between instructions, never with a delay
        // slot pending: entry waits for the slot to retire.
        if self.in_delay {
            return false;
        }
        let Some((irq, level)) = self.acceptable_interrupt() else {
            return false;
        };
        self.waiting = false;
        self.forget_loop();
        let st = self.status();
        self.cp0[C0_EPC] = self.pc;
        let cause = self.cp0[C0_CAUSE] & !(CA_BD | 0x7C | (0x3F << CA_RIPL_SHIFT));
        self.cp0[C0_CAUSE] = cause | (EXC_INT << 2) | (level << CA_RIPL_SHIFT);
        let eicss = self.evic.shadow_set(level).min(HSS);
        self.cp0[C0_SRSCTL] = (self.cp0[C0_SRSCTL] & !(0xF << 18)) | (eicss << 18);
        if st & ST_BEV == 0 {
            self.enter_shadow_set(eicss);
        }
        self.cp0[C0_STATUS] = st | ST_EXL;
        let base = self.vector_base();
        self.pc = if self.cp0[C0_CAUSE] & CA_IV == 0 {
            base.wrapping_add(0x180)
        } else if st & ST_BEV != 0 {
            base.wrapping_add(0x200)
        } else if self.evic.intcon & INTCON_MVEC != 0 {
            base.wrapping_add(self.evic.off[irq as usize])
        } else {
            base.wrapping_add(0x200)
        };
        self.npc = self.pc.wrapping_add(4);
        true
    }

    fn eret(&mut self) -> Flow {
        let st = self.status();
        let target = if st & ST_ERL != 0 {
            self.cp0[C0_STATUS] = st & !ST_ERL;
            self.cp0[C0_ERROREPC]
        } else {
            self.cp0[C0_STATUS] = st & !ST_EXL;
            if st & ST_BEV == 0 {
                let s = self.cp0[C0_SRSCTL];
                let pss = (s >> 6) & 0xF;
                self.cp0[C0_SRSCTL] = (s & !0xF) | pss;
            }
            self.cp0[C0_EPC]
        };
        self.llbit = false;
        Flow::Jump(target)
    }

    /// `$0-$31`, HI, LO, then the standard `[SP, PC]` trailer.
    fn observer_registers(&self) -> [u32; 36] {
        let mut regs = [0u32; 36];
        regs[..32].copy_from_slice(&self.gpr[self.css()]);
        regs[32] = self.hi;
        regs[33] = self.lo;
        regs[34] = self.reg(29);
        regs[35] = self.pc;
        regs
    }

    // ---- execution -------------------------------------------------------

    fn exec(&mut self, bus: &mut dyn Bus, ins: u32) -> SimResult<Flow> {
        let op = ins >> 26;
        let rs = (ins >> 21) & 31;
        let rt = (ins >> 16) & 31;
        let rd = (ins >> 11) & 31;
        let sa = (ins >> 6) & 31;
        let imm = ins & 0xFFFF;
        let simm = sext16(imm);
        let vs = self.reg(rs);
        let vt = self.reg(rt);
        let btarget = self.npc.wrapping_add(simm << 2);
        let branch = |taken: bool| {
            if taken {
                Flow::Branch(btarget)
            } else {
                Flow::Next
            }
        };
        let likely = |taken: bool| {
            if taken {
                Flow::Branch(btarget)
            } else {
                Flow::Nullify
            }
        };
        macro_rules! ld {
            ($n:expr) => {
                match self.load(bus, vs.wrapping_add(simm), $n)? {
                    Ok(v) => v,
                    Err(f) => return Ok(f),
                }
            };
        }
        macro_rules! st {
            ($n:expr, $v:expr) => {
                if let Err(f) = self.store(bus, vs.wrapping_add(simm), $n, $v)? {
                    return Ok(f);
                }
            };
        }
        Ok(match op {
            0x00 => return self.exec_special(ins, rs, rt, rd, sa, vs, vt),
            0x01 => {
                let lt = (vs as i32) < 0;
                match rt {
                    0x00 => branch(lt),
                    0x01 => branch(!lt),
                    0x02 => likely(lt),
                    0x03 => likely(!lt),
                    0x08 => self.trap((vs as i32) >= (simm as i32)),
                    0x09 => self.trap(vs >= simm),
                    0x0A => self.trap((vs as i32) < (simm as i32)),
                    0x0B => self.trap(vs < simm),
                    0x0C => self.trap(vs == simm),
                    0x0E => self.trap(vs != simm),
                    0x10..=0x13 => {
                        let taken = if rt & 1 == 0 { lt } else { !lt };
                        self.set_reg(31, self.npc.wrapping_add(4));
                        if rt & 2 == 0 {
                            branch(taken)
                        } else {
                            likely(taken)
                        }
                    }
                    // SYNCI: caches are not modelled.
                    0x1F => Flow::Next,
                    _ => Flow::Unsupported,
                }
            }
            0x02 | 0x03 => {
                let target = (self.npc & 0xF000_0000) | ((ins & 0x03FF_FFFF) << 2);
                if op == 0x03 {
                    self.set_reg(31, self.npc.wrapping_add(4));
                }
                Flow::Branch(target)
            }
            0x04 => branch(vs == vt),
            0x05 => branch(vs != vt),
            0x06 => branch((vs as i32) <= 0),
            0x07 => branch((vs as i32) > 0),
            0x08 => match (vs as i32).checked_add(simm as i32) {
                Some(r) => {
                    self.set_reg(rt, r as u32);
                    Flow::Next
                }
                None => exc(EXC_OV),
            },
            0x09 => {
                self.set_reg(rt, vs.wrapping_add(simm));
                Flow::Next
            }
            0x0A => {
                self.set_reg(rt, u32::from((vs as i32) < (simm as i32)));
                Flow::Next
            }
            0x0B => {
                self.set_reg(rt, u32::from(vs < simm));
                Flow::Next
            }
            0x0C => {
                self.set_reg(rt, vs & imm);
                Flow::Next
            }
            0x0D => {
                self.set_reg(rt, vs | imm);
                Flow::Next
            }
            0x0E => {
                self.set_reg(rt, vs ^ imm);
                Flow::Next
            }
            0x0F => {
                self.set_reg(rt, imm << 16);
                Flow::Next
            }
            0x10 => self.exec_cop0(ins, rs, rt, rd),
            // COP1, COP1X: the FPU is not modelled.
            0x11 | 0x13 | 0x31 | 0x35 | 0x39 | 0x3D => self.cop1_unusable_or_unsupported(),
            // COP2 and its loads/stores: the PIC32MZ has no coprocessor 2.
            0x12 | 0x32 | 0x36 | 0x3A | 0x3E => Flow::Exception {
                code: EXC_CPU,
                badvaddr: None,
                ce: 2,
            },
            0x14 => likely(vs == vt),
            0x15 => likely(vs != vt),
            0x16 => likely((vs as i32) <= 0),
            0x17 => likely((vs as i32) > 0),
            0x1C => self.exec_special2(ins, rs, rt, rd, vs, vt),
            0x1F => self.exec_special3(ins, rt, rd, sa, vs, vt),
            0x20 => {
                let v = ld!(1);
                self.set_reg(rt, v as u8 as i8 as i32 as u32);
                Flow::Next
            }
            0x21 => {
                let v = ld!(2);
                self.set_reg(rt, v as u16 as i16 as i32 as u32);
                Flow::Next
            }
            0x22 | 0x26 => {
                let va = vs.wrapping_add(simm);
                let w = match self.load(bus, va & !3, 4)? {
                    Ok(v) => v,
                    Err(f) => return Ok(f),
                };
                let b = va & 3;
                let r = if op == 0x22 {
                    // LWL (little-endian): the high-order bytes of rt.
                    let sh = 8 * (3 - b);
                    let keep = if sh == 0 { 0 } else { (1u32 << sh) - 1 };
                    (w << sh) | (vt & keep)
                } else {
                    // LWR: the low-order bytes of rt.
                    let sh = 8 * b;
                    (w >> sh) | (vt & !(u32::MAX >> sh))
                };
                self.set_reg(rt, r);
                Flow::Next
            }
            0x23 => {
                let v = ld!(4);
                self.set_reg(rt, v);
                Flow::Next
            }
            0x24 => {
                let v = ld!(1);
                self.set_reg(rt, v);
                Flow::Next
            }
            0x25 => {
                let v = ld!(2);
                self.set_reg(rt, v);
                Flow::Next
            }
            0x28 => {
                st!(1, vt);
                Flow::Next
            }
            0x29 => {
                st!(2, vt);
                Flow::Next
            }
            0x2A | 0x2E => {
                let va = vs.wrapping_add(simm);
                let w = match self.load(bus, va & !3, 4)? {
                    Ok(v) => v,
                    Err(f) => return Ok(f),
                };
                let b = va & 3;
                let r = if op == 0x2A {
                    // SWL: rt's high-order bytes to the bytes at and below va.
                    let sh = 8 * (3 - b);
                    (w & !(u32::MAX >> sh)) | (vt >> sh)
                } else {
                    // SWR: rt's low-order bytes to the bytes at and above va.
                    let sh = 8 * b;
                    let keep = if sh == 0 { 0 } else { (1u32 << sh) - 1 };
                    (vt << sh) | (w & keep)
                };
                if let Err(f) = self.store(bus, va & !3, 4, r)? {
                    return Ok(f);
                }
                Flow::Next
            }
            0x2B => {
                st!(4, vt);
                Flow::Next
            }
            // CACHE / PREF: caches and the prefetch buffer are not modelled.
            0x2F | 0x33 => Flow::Next,
            0x30 => {
                let va = vs.wrapping_add(simm);
                let v = ld!(4);
                self.llbit = true;
                self.cp0[C0_LLADDR] = self.translate(va, false).unwrap_or(0) >> 4;
                self.set_reg(rt, v);
                Flow::Next
            }
            0x38 => {
                if self.llbit {
                    st!(4, vt);
                    self.set_reg(rt, 1);
                } else {
                    self.set_reg(rt, 0);
                }
                self.llbit = false;
                Flow::Next
            }
            _ => Flow::Unsupported,
        })
    }

    fn trap(&self, cond: bool) -> Flow {
        if cond {
            exc(EXC_TR)
        } else {
            Flow::Next
        }
    }

    fn cop1_unusable_or_unsupported(&self) -> Flow {
        if self.status() & ST_CU1 == 0 {
            Flow::Exception {
                code: EXC_CPU,
                badvaddr: None,
                ce: 1,
            }
        } else {
            Flow::Unsupported
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_special(
        &mut self,
        ins: u32,
        rs: u32,
        _rt: u32,
        rd: u32,
        sa: u32,
        vs: u32,
        vt: u32,
    ) -> SimResult<Flow> {
        let funct = ins & 0x3F;
        let w = |cpu: &mut Self, v: u32| {
            cpu.set_reg(rd, v);
            Flow::Next
        };
        Ok(match funct {
            // SLL (NOP, SSNOP, EHB, PAUSE are SLL $0).
            0x00 => w(self, vt << sa),
            0x01 => self.cop1_unusable_or_unsupported(),
            0x02 => match rs {
                0 => w(self, vt >> sa),
                1 => w(self, vt.rotate_right(sa)),
                _ => Flow::Unsupported,
            },
            0x03 => w(self, ((vt as i32) >> sa) as u32),
            0x04 => w(self, vt << (vs & 31)),
            0x06 => match sa {
                0 => w(self, vt >> (vs & 31)),
                1 => w(self, vt.rotate_right(vs & 31)),
                _ => Flow::Unsupported,
            },
            0x07 => w(self, ((vt as i32) >> (vs & 31)) as u32),
            // JR / JR.HB
            0x08 => Flow::Branch(vs),
            // JALR / JALR.HB
            0x09 => {
                self.set_reg(rd, self.npc.wrapping_add(4));
                Flow::Branch(vs)
            }
            0x0A => {
                if vt == 0 {
                    self.set_reg(rd, vs);
                }
                Flow::Next
            }
            0x0B => {
                if vt != 0 {
                    self.set_reg(rd, vs);
                }
                Flow::Next
            }
            0x0C => exc(EXC_SYS),
            0x0D => exc(EXC_BP),
            // SYNC: one in-order core, nothing to order.
            0x0F => Flow::Next,
            0x10 => w(self, self.hi),
            0x11 => {
                self.hi = vs;
                Flow::Next
            }
            0x12 => w(self, self.lo),
            0x13 => {
                self.lo = vs;
                Flow::Next
            }
            0x18 => {
                let p = i64::from(vs as i32) * i64::from(vt as i32);
                self.hi = (p >> 32) as u32;
                self.lo = p as u32;
                Flow::Next
            }
            0x19 => {
                let p = u64::from(vs) * u64::from(vt);
                self.hi = (p >> 32) as u32;
                self.lo = p as u32;
                Flow::Next
            }
            0x1A => {
                // Division by zero leaves HI/LO unchanged (UNPREDICTABLE in
                // the ISA; GCC guards it with TEQ).
                if vt != 0 {
                    let (a, b) = (vs as i32, vt as i32);
                    self.lo = a.wrapping_div(b) as u32;
                    self.hi = a.wrapping_rem(b) as u32;
                }
                Flow::Next
            }
            0x1B => {
                if let (Some(q), Some(r)) = (vs.checked_div(vt), vs.checked_rem(vt)) {
                    self.lo = q;
                    self.hi = r;
                }
                Flow::Next
            }
            0x20 => match (vs as i32).checked_add(vt as i32) {
                Some(r) => w(self, r as u32),
                None => exc(EXC_OV),
            },
            0x21 => w(self, vs.wrapping_add(vt)),
            0x22 => match (vs as i32).checked_sub(vt as i32) {
                Some(r) => w(self, r as u32),
                None => exc(EXC_OV),
            },
            0x23 => w(self, vs.wrapping_sub(vt)),
            0x24 => w(self, vs & vt),
            0x25 => w(self, vs | vt),
            0x26 => w(self, vs ^ vt),
            0x27 => w(self, !(vs | vt)),
            0x2A => w(self, u32::from((vs as i32) < (vt as i32))),
            0x2B => w(self, u32::from(vs < vt)),
            0x30 => self.trap((vs as i32) >= (vt as i32)),
            0x31 => self.trap(vs >= vt),
            0x32 => self.trap((vs as i32) < (vt as i32)),
            0x33 => self.trap(vs < vt),
            0x34 => self.trap(vs == vt),
            0x36 => self.trap(vs != vt),
            _ => Flow::Unsupported,
        })
    }

    fn exec_special2(&mut self, ins: u32, _rs: u32, _rt: u32, rd: u32, vs: u32, vt: u32) -> Flow {
        let acc = (u64::from(self.hi) << 32) | u64::from(self.lo);
        let put = |cpu: &mut Self, v: u64| {
            cpu.hi = (v >> 32) as u32;
            cpu.lo = v as u32;
            Flow::Next
        };
        match ins & 0x3F {
            0x00 => {
                let p = i64::from(vs as i32) * i64::from(vt as i32);
                put(self, (acc as i64).wrapping_add(p) as u64)
            }
            0x01 => put(self, acc.wrapping_add(u64::from(vs) * u64::from(vt))),
            0x02 => {
                self.set_reg(rd, vs.wrapping_mul(vt));
                Flow::Next
            }
            0x04 => {
                let p = i64::from(vs as i32) * i64::from(vt as i32);
                put(self, (acc as i64).wrapping_sub(p) as u64)
            }
            0x05 => put(self, acc.wrapping_sub(u64::from(vs) * u64::from(vt))),
            0x20 => {
                self.set_reg(rd, vs.leading_zeros());
                Flow::Next
            }
            0x21 => {
                self.set_reg(rd, vs.leading_ones());
                Flow::Next
            }
            // SDBBP: EJTAG is not modelled.
            _ => Flow::Unsupported,
        }
    }

    fn exec_special3(&mut self, ins: u32, rt: u32, rd: u32, sa: u32, vs: u32, vt: u32) -> Flow {
        match ins & 0x3F {
            // EXT rt, rs, pos=sa, size=rd+1
            0x00 => {
                let size = rd + 1;
                if sa + size > 32 {
                    return Flow::Unsupported;
                }
                let mask = if size == 32 {
                    u32::MAX
                } else {
                    (1 << size) - 1
                };
                self.set_reg(rt, (vs >> sa) & mask);
                Flow::Next
            }
            // INS rt, rs, pos=sa, size=rd-sa+1
            0x04 => {
                if rd < sa {
                    return Flow::Unsupported;
                }
                let size = rd - sa + 1;
                let mask = if size == 32 {
                    u32::MAX
                } else {
                    (1 << size) - 1
                };
                self.set_reg(rt, (vt & !(mask << sa)) | ((vs & mask) << sa));
                Flow::Next
            }
            0x20 => {
                let v = match sa {
                    0x02 => ((vt & 0x00FF_00FF) << 8) | ((vt >> 8) & 0x00FF_00FF),
                    0x10 => vt as u8 as i8 as i32 as u32,
                    0x18 => vt as u16 as i16 as i32 as u32,
                    _ => return Flow::Unsupported,
                };
                self.set_reg(rd, v);
                Flow::Next
            }
            // RDHWR rt, rd
            0x3B => {
                self.side_effect = true;
                if !self.kernel_mode()
                    && self.status() & ST_CU0 == 0
                    && self.cp0[C0_HWRENA] & (1 << rd) == 0
                {
                    return exc(EXC_RI);
                }
                let v = match rd {
                    0 => self.cp0[C0_EBASE] & 0x3FF,
                    // SYNCI step: the 16-byte cache line.
                    1 => 16,
                    2 => self.cp0[C0_COUNT],
                    // Count runs at half the core clock.
                    3 => 2,
                    29 => self.cp0[C0_USERLOCAL],
                    _ => return exc(EXC_RI),
                };
                self.set_reg(rt, v);
                Flow::Next
            }
            _ => Flow::Unsupported,
        }
    }

    fn exec_cop0(&mut self, ins: u32, rs: u32, rt: u32, rd: u32) -> Flow {
        self.side_effect = true;
        if !self.kernel_mode() && self.status() & ST_CU0 == 0 {
            return Flow::Exception {
                code: EXC_CPU,
                badvaddr: None,
                ce: 0,
            };
        }
        let sel = ins & 7;
        match rs {
            0x00 => {
                let v = self.read_cp0(rd, sel);
                self.set_reg(rt, v);
                Flow::Next
            }
            0x04 => {
                let v = self.reg(rt);
                self.write_cp0(rd, sel, v);
                Flow::Next
            }
            // RDPGPR rd, rt: read rt of the previous shadow set.
            0x0A => {
                let v = self.gpr[self.pss()][rt as usize];
                self.set_reg(rd, v);
                Flow::Next
            }
            // MFMC0: DI (sc = 0) / EI (sc = 1).
            0x0B if rd == 12 && ins & 0x7DF == 0 => {
                let st = self.status();
                self.set_reg(rt, st);
                self.cp0[C0_STATUS] = if ins & 0x20 != 0 {
                    st | ST_IE
                } else {
                    st & !ST_IE
                };
                Flow::Next
            }
            // WRPGPR rd, rt: write rd of the previous shadow set.
            0x0E => {
                let v = self.reg(rt);
                let pss = self.pss();
                if rd != 0 {
                    self.gpr[pss][rd as usize] = v;
                }
                Flow::Next
            }
            0x10..=0x1F => match ins & 0x3F {
                0x18 => self.eret(),
                0x20 => {
                    self.waiting = true;
                    Flow::Next
                }
                // TLBR/TLBWI/TLBWR/TLBP: the TLB is not modelled. DERET:
                // EJTAG is not modelled.
                _ => Flow::Unsupported,
            },
            _ => Flow::Unsupported,
        }
    }

    /// A taken backward branch at `pc`. When the core comes back to a
    /// backward branch with the same PC, register file and shadow set as one
    /// of the last few, and executed no load, store, CP0/hardware-register
    /// access or exception in between, every further iteration is identical:
    /// the loop is a fixed point that only an interrupt can leave (NuttX's
    /// idle loop, `jal up_idle` twice then `b`). Skipping its cycles is then
    /// exact up to the phase within one iteration.
    fn note_backward_branch(&mut self, pc: u32) {
        if self.side_effect {
            self.side_effect = false;
            self.epoch = self.epoch.wrapping_add(1);
        }
        let css = self.css();
        let here = LoopAnchor {
            epoch: self.epoch,
            pc,
            css,
            gpr: self.gpr[css],
            hi: self.hi,
            lo: self.lo,
        };
        if self.loop_anchors.iter().flatten().any(|a| *a == here) {
            self.idle_loop = true;
            return;
        }
        self.idle_loop = false;
        self.loop_anchors[self.loop_next] = Some(here);
        self.loop_next = (self.loop_next + 1) % self.loop_anchors.len();
    }

    fn forget_loop(&mut self) {
        self.loop_anchors = [None, None, None, None];
        self.idle_loop = false;
        self.side_effect = false;
    }

    /// One instruction (or one idle cycle while `WAIT`ing).
    fn step_one(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
    ) -> SimResult<()> {
        if self.waiting {
            self.advance(1);
            if self.evic.highest().is_some() {
                // A pending request ends WAIT whether or not it is taken.
                self.waiting = false;
                self.take_interrupt();
            }
            return Ok(());
        }
        let pc = self.pc;
        let ins = if pc & 3 != 0 {
            None
        } else {
            match self.translate(pc, false) {
                Ok(pa) => Some(self.fetch(bus, pa)?),
                Err(_) => None,
            }
        };
        let flow = match ins {
            Some(ins) => {
                for o in observers {
                    o.on_step_start(pc, ins);
                }
                self.exec(bus, ins)?
            }
            None => {
                let code = if pc & 3 != 0 || pc >= 0x8000_0000 {
                    EXC_ADEL
                } else {
                    EXC_TLBL
                };
                Flow::Exception {
                    code,
                    badvaddr: Some(pc),
                    ce: 0,
                }
            }
        };
        match flow {
            Flow::Next => {
                self.pc = self.npc;
                self.npc = self.npc.wrapping_add(4);
                self.in_delay = false;
            }
            Flow::Branch(t) => {
                if t <= pc {
                    self.note_backward_branch(pc);
                }
                self.pc = self.npc;
                self.npc = t;
                self.in_delay = true;
            }
            Flow::Nullify => {
                self.pc = self.npc.wrapping_add(4);
                self.npc = self.pc.wrapping_add(4);
                self.in_delay = false;
            }
            Flow::Jump(t) => {
                self.pc = t;
                self.npc = t.wrapping_add(4);
                self.in_delay = false;
            }
            Flow::Exception { code, badvaddr, ce } => {
                self.forget_loop();
                self.raise(code, badvaddr, ce);
            }
            Flow::Unsupported => {
                // The guest sees an architectural exception; the host log
                // line is what `scripts/unsupported_instruction_audit.sh`
                // counts.
                tracing::warn!("Unknown instruction {:#x} at {:#x}", ins.unwrap_or(0), pc);
                self.forget_loop();
                self.raise(EXC_RI, None, 0);
            }
        }
        self.advance(1);
        if !observers.is_empty() {
            crate::emit_trace_event(
                observers,
                labwired_hw_trace::TraceEvent::InstructionRetired {
                    pc,
                    opcode: ins.unwrap_or(0),
                },
            );
        }
        self.take_interrupt();
        if !observers.is_empty() {
            let regs = self.observer_registers();
            for o in observers {
                o.on_step_end(1, &regs);
            }
        }
        Ok(())
    }

    /// Refuse an image the model cannot boot: DEVCFG0.BOOTISA (bit 6) = 0
    /// starts the core in microMIPS, which is not modelled.
    fn check_boot_isa(bus: &mut dyn Bus) -> SimResult<()> {
        // DEVCFG0 in the lower boot alias configuration space.
        let devcfg0 = bus.read_u32(0x1FC0_FFCC)?;
        if devcfg0 & (1 << 6) == 0 {
            return Err(crate::SimulationError::NotImplemented(
                "DEVCFG0.BOOTISA selects microMIPS at reset; the MIPS model runs MIPS32 only"
                    .into(),
            ));
        }
        Ok(())
    }
}

impl Cpu for Mips32 {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        let evic_base = self.evic_base;
        *self = Self::new();
        self.evic_base = evic_base;
        Self::check_boot_isa(bus)?;
        Ok(())
    }

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        _config: &crate::SimulationConfig,
    ) -> SimResult<()> {
        self.step_one(bus, observers)
    }

    /// The loader must not fall back to Cortex-M vector tables;
    /// [`crate::system::mips::load_elf`] starts the core at the reset
    /// vector.
    fn uses_elf_entry_point(&self) -> bool {
        true
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val;
        self.npc = val.wrapping_add(4);
        self.in_delay = false;
        self.forget_loop();
    }
    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_sp(&mut self, val: u32) {
        self.set_reg(29, val);
    }

    /// A peripheral requests EVIC interrupt `n`: set its IFS flag.
    fn set_exception_pending(&mut self, exception_num: u32) {
        self.evic.flag(exception_num);
    }

    /// While `WAIT`ing, or spinning in a proven fixed-point loop, with no
    /// EVIC request pending: the cycles until Count next meets Compare (the
    /// machine also clamps to the next peripheral event).
    fn idle_fast_forward_budget(&self, _bus: &dyn Bus) -> Option<u64> {
        if !(self.waiting || self.idle_loop) || self.evic.highest().is_some() {
            return None;
        }
        Some(self.cycles_to_compare().saturating_sub(1))
    }

    fn fast_forward_idle_cycles(&mut self, cycles: u64) {
        self.advance(cycles);
    }

    fn get_register(&self, id: u8) -> u32 {
        match id {
            0..=31 => self.reg(u32::from(id)),
            32 => self.hi,
            33 => self.lo,
            34 => self.pc,
            35 => self.status(),
            36 => self.read_cp0(13, 0),
            37 => self.cp0[C0_EPC],
            _ => 0,
        }
    }
    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0..=31 => self.set_reg(u32::from(id), val),
            32 => self.hi = val,
            33 => self.lo = val,
            34 => self.set_pc(val),
            35 => self.write_cp0(12, 0, val),
            36 => self.write_cp0(13, 0, val),
            37 => self.cp0[C0_EPC] = val,
            _ => {}
        }
    }

    fn invalidate_code_caches(&mut self) {
        self.fetch_cache.fill((u32::MAX, 0));
    }

    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::Mips(Box::new(MipsCpuSnapshot {
            gpr: self.gpr.iter().map(|s| s.to_vec()).collect(),
            hi: self.hi,
            lo: self.lo,
            pc: self.pc,
            npc: self.npc,
            in_delay: self.in_delay,
            cp0: self.cp0.to_vec(),
            evic: self.evic.clone(),
            llbit: self.llbit,
            waiting: self.waiting,
            cycles: self.cycles,
        }))
    }

    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::Mips(s) = snapshot {
            for (dst, src) in self.gpr.iter_mut().zip(s.gpr.iter()) {
                for (d, v) in dst.iter_mut().zip(src.iter()) {
                    *d = *v;
                }
                dst[0] = 0;
            }
            self.hi = s.hi;
            self.lo = s.lo;
            self.pc = s.pc;
            self.npc = s.npc;
            self.in_delay = s.in_delay;
            for (d, v) in self.cp0.iter_mut().zip(s.cp0.iter()) {
                *d = *v;
            }
            if s.evic.ipc.len() == 64 && s.evic.off.len() == EVIC_IRQS as usize {
                self.evic = s.evic.clone();
            }
            self.llbit = s.llbit;
            self.waiting = s.waiting;
            self.cycles = s.cycles;
            self.invalidate_code_caches();
            self.forget_loop();
        }
    }

    fn get_register_names(&self) -> Vec<String> {
        let mut names: Vec<String> = (0..32).map(|i| format!("r{i}")).collect();
        for n in ["hi", "lo", "pc", "status", "cause", "epc"] {
            names.push(n.to_string());
        }
        names
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        const ABI: [&str; 32] = [
            "zero", "at", "v0", "v1", "a0", "a1", "a2", "a3", "t0", "t1", "t2", "t3", "t4", "t5",
            "t6", "t7", "s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "t8", "t9", "k0", "k1",
            "gp", "sp", "fp", "ra",
        ];
        let n = name.to_ascii_lowercase();
        let n = n.trim_start_matches('$');
        if let Some(i) = ABI.iter().position(|a| *a == n) {
            return Some(i as u8);
        }
        match n {
            "s8" => Some(30),
            "hi" => Some(32),
            "lo" => Some(33),
            "pc" => Some(34),
            "status" => Some(35),
            "cause" => Some(36),
            "epc" => Some(37),
            _ => n
                .strip_prefix('r')
                .unwrap_or(n)
                .parse()
                .ok()
                .filter(|d: &u8| *d < 32),
        }
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "mips_tests.rs"]
mod tests;
