// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Cortex-M **fault verdict**: why the firmware faulted and where, in one
//! sentence.
//!
//! The CPU model already escalates faults the way ARMv7-M B1.5.14 says it must
//! and leaves the evidence in the SCB fault registers (HFSR, CFSR, MMFAR, BFAR).
//! Those registers are bit fields. A person who asks "why did my firmware
//! crash" wants an answer like:
//!
//! > HardFault escalated from a precise BusFault: data access at 0x9000_0000
//! > (BFAR valid), at PC 0x0800_1234 in `sensor_read` (main.c:42), called from
//! > `main`.
//!
//! [`decode_fault`] makes that answer. It is a **pure** function: SCB state,
//! the stacked exception frame and an optional symbolizer go in, a
//! [`FaultVerdict`] comes out. It touches no CPU and no bus, so every surface
//! (CLI `result.json`, MCP, wasm/playground, Python) calls the same code and
//! gets the same sentence.
//!
//! [`FaultCapture`] is what the CPU records when it takes a fault exception
//! (the frame it stacked, the EXC_RETURN it built) or when it cannot take one
//! at all (LOCKUP). `Cpu::fault_capture` returns it; `None` means nothing
//! faulted.
//!
//! Honesty rules the decode keeps:
//! * A fault address is reported **only** when its VALID bit is set
//!   (`BFARVALID` / `MMARVALID`). An imprecise BusFault never gets one, and
//!   its stacked PC is named as "after" the faulting access, not "at" it.
//! * The caller hint comes from the stacked LR. It is omitted when LR is an
//!   EXC_RETURN value or points into the same function as the PC, because then
//!   it does not name a caller.

use serde::{Deserialize, Serialize};

// ---- Register bit definitions (ARMv7-M ARM B3.2.15 / B3.2.16) --------------

/// One named status bit in a fault status register.
struct FlagDef {
    /// The architectural register that holds the bit.
    register: &'static str,
    /// The architectural bit name.
    flag: &'static str,
    /// Bit position inside the 32-bit register it is read from (CFSR or HFSR).
    bit: u32,
    /// Plain-language meaning.
    meaning: &'static str,
}

/// CFSR bits. MMFSR = CFSR[7:0], BFSR = CFSR[15:8], UFSR = CFSR[31:16].
const CFSR_FLAGS: &[FlagDef] = &[
    FlagDef {
        register: "MMFSR",
        flag: "IACCVIOL",
        bit: 0,
        meaning: "instruction fetch from a no-execute or protected region",
    },
    FlagDef {
        register: "MMFSR",
        flag: "DACCVIOL",
        bit: 1,
        meaning: "data access violation (MPU)",
    },
    FlagDef {
        register: "MMFSR",
        flag: "MUNSTKERR",
        bit: 3,
        meaning: "MPU fault while unstacking on exception return",
    },
    FlagDef {
        register: "MMFSR",
        flag: "MSTKERR",
        bit: 4,
        meaning: "MPU fault while stacking on exception entry",
    },
    FlagDef {
        register: "MMFSR",
        flag: "MLSPERR",
        bit: 5,
        meaning: "MPU fault during lazy FPU state preservation",
    },
    FlagDef {
        register: "BFSR",
        flag: "IBUSERR",
        bit: 8,
        meaning: "bus error on instruction fetch",
    },
    FlagDef {
        register: "BFSR",
        flag: "PRECISERR",
        bit: 9,
        meaning: "data access",
    },
    FlagDef {
        register: "BFSR",
        flag: "IMPRECISERR",
        bit: 10,
        meaning: "a buffered write failed after the instruction retired",
    },
    FlagDef {
        register: "BFSR",
        flag: "UNSTKERR",
        bit: 11,
        meaning: "bus error while unstacking on exception return",
    },
    FlagDef {
        register: "BFSR",
        flag: "STKERR",
        bit: 12,
        meaning: "bus error while stacking on exception entry (stack overflow or corrupt SP)",
    },
    FlagDef {
        register: "BFSR",
        flag: "LSPERR",
        bit: 13,
        meaning: "bus error during lazy FPU state preservation",
    },
    FlagDef {
        register: "UFSR",
        flag: "UNDEFINSTR",
        bit: 16,
        meaning: "undefined instruction",
    },
    FlagDef {
        register: "UFSR",
        flag: "INVSTATE",
        bit: 17,
        meaning: "invalid execution state (Thumb bit clear, e.g. a call through an even address)",
    },
    FlagDef {
        register: "UFSR",
        flag: "INVPC",
        bit: 18,
        meaning: "invalid EXC_RETURN or PC on exception return",
    },
    FlagDef {
        register: "UFSR",
        flag: "NOCP",
        bit: 19,
        meaning: "coprocessor (FPU) instruction while the coprocessor is disabled",
    },
    FlagDef {
        register: "UFSR",
        flag: "UNALIGNED",
        bit: 24,
        meaning: "unaligned memory access",
    },
    FlagDef {
        register: "UFSR",
        flag: "DIVBYZERO",
        bit: 25,
        meaning: "divide by zero (CCR.DIV_0_TRP is set)",
    },
];

/// `MMFSR.MMARVALID`, CFSR bit 7.
const MMARVALID: u32 = 1 << 7;
/// `BFSR.BFARVALID`, CFSR bit 15.
const BFARVALID: u32 = 1 << 15;
const PRECISERR: u32 = 1 << 9;
const IMPRECISERR: u32 = 1 << 10;

/// HFSR bits.
const HFSR_FLAGS: &[FlagDef] = &[
    FlagDef {
        register: "HFSR",
        flag: "VECTTBL",
        bit: 1,
        meaning: "bus error reading the vector table",
    },
    FlagDef {
        register: "HFSR",
        flag: "FORCED",
        bit: 30,
        meaning: "a configurable fault escalated to HardFault",
    },
    FlagDef {
        register: "HFSR",
        flag: "DEBUGEVT",
        bit: 31,
        meaning: "debug event (e.g. BKPT) with no debugger attached",
    },
];
const HFSR_VECTTBL: u32 = 1 << 1;
const HFSR_FORCED: u32 = 1 << 30;
const HFSR_DEBUGEVT: u32 = 1 << 31;

// ---- Inputs ----------------------------------------------------------------

/// The SCB fault status registers, read as firmware reads them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultRegs {
    /// HardFault Status Register (0xE000ED2C).
    pub hfsr: u32,
    /// Configurable Fault Status Register (0xE000ED28): MMFSR:BFSR:UFSR.
    pub cfsr: u32,
    /// MemManage Fault Address Register (0xE000ED34).
    pub mmfar: u32,
    /// BusFault Address Register (0xE000ED38).
    pub bfar: u32,
}

impl FaultRegs {
    /// MMFSR, CFSR[7:0].
    pub fn mmfsr(&self) -> u8 {
        (self.cfsr & 0xFF) as u8
    }
    /// BFSR, CFSR[15:8].
    pub fn bfsr(&self) -> u8 {
        ((self.cfsr >> 8) & 0xFF) as u8
    }
    /// UFSR, CFSR[31:16].
    pub fn ufsr(&self) -> u16 {
        (self.cfsr >> 16) as u16
    }
    /// True when no fault status bit is set.
    pub fn is_clear(&self) -> bool {
        self.hfsr == 0 && self.cfsr == 0
    }
}

/// The eight words the core pushes on exception entry (ARMv7-M B1.5.6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackedFrame {
    pub r0: u32,
    pub r1: u32,
    pub r2: u32,
    pub r3: u32,
    pub r12: u32,
    pub lr: u32,
    /// The return address. For a precise fault this is the faulting
    /// instruction; for an imprecise BusFault it is somewhere after it.
    pub pc: u32,
    pub xpsr: u32,
}

impl StackedFrame {
    /// Read a stacked frame from the stack that `exc_return` names.
    ///
    /// EXC_RETURN bit 2 selects the stack the frame was pushed on: set means
    /// PSP (Thread mode on the process stack), clear means MSP. Returns `None`
    /// when `exc_return` is not an EXC_RETURN value or a word cannot be read.
    pub fn read_by_exc_return(
        exc_return: u32,
        msp: u32,
        psp: u32,
        mut read_u32: impl FnMut(u32) -> Option<u32>,
    ) -> Option<(u32, StackedFrame)> {
        let info = ExcReturnInfo::decode(exc_return)?;
        let sp = if info.stack == "psp" { psp } else { msp };
        let mut w = [0u32; 8];
        for (i, slot) in w.iter_mut().enumerate() {
            *slot = read_u32(sp.wrapping_add(4 * i as u32))?;
        }
        Some((
            sp,
            StackedFrame {
                r0: w[0],
                r1: w[1],
                r2: w[2],
                r3: w[3],
                r12: w[4],
                lr: w[5],
                pc: w[6],
                xpsr: w[7],
            },
        ))
    }
}

/// What the CPU recorded when it entered a fault handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultEntry {
    /// Exception number taken: 3 HardFault, 4 MemManage, 5 BusFault, 6 UsageFault.
    pub exception: u32,
    /// The EXC_RETURN value the core put in LR.
    pub exc_return: u32,
    /// Address of the stacked frame (the SP EXC_RETURN names, after stacking).
    pub frame_sp: u32,
    /// The stacked frame itself.
    pub frame: StackedFrame,
}

/// The core could not take any fault handler: LOCKUP (ARMv7-M B1.5.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockupEntry {
    /// The PC of the instruction that faulted.
    pub pc: u32,
    /// The exception that was active when it faulted (3 = in the HardFault
    /// handler, i.e. a double fault; 2 = in NMI).
    pub active_exception: u32,
}

/// Everything the CPU knows about a fault. Returned by `Cpu::fault_capture`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultCapture {
    pub regs: FaultRegs,
    /// The first fault-handler entry since reset.
    pub entry: Option<FaultEntry>,
    /// Set when the core locked up instead of taking a handler.
    pub lockup: Option<LockupEntry>,
}

impl FaultCapture {
    /// True when there is anything to report.
    pub fn is_fault(&self) -> bool {
        !self.regs.is_clear() || self.entry.is_some() || self.lockup.is_some()
    }
}

/// A source position for an address.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeLocation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

impl CodeLocation {
    fn is_empty(&self) -> bool {
        self.function.is_none() && self.file.is_none()
    }
}

/// Maps a code address to a function and a source line. The loader's
/// `SymbolProvider` implements it (ELF symbols + DWARF line info).
pub trait FaultSymbolizer {
    fn symbolize(&self, addr: u32) -> Option<CodeLocation>;
}

// ---- Output ----------------------------------------------------------------

/// Decoded EXC_RETURN (ARMv7-M B1.5.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcReturnInfo {
    pub value: String,
    /// "thread" or "handler": the mode the fault interrupted.
    pub return_to: String,
    /// "msp" or "psp": the stack the frame is on.
    pub stack: String,
    /// True when the frame is the extended (FPU) frame.
    pub fp_frame: bool,
}

impl ExcReturnInfo {
    /// Decode an EXC_RETURN. `None` if the value is not one.
    pub fn decode(v: u32) -> Option<Self> {
        if v & 0xFFFF_FF00 != 0xFFFF_FF00 {
            return None;
        }
        let handler = v & 0x8 == 0;
        Some(Self {
            value: hex(v),
            return_to: if handler { "handler" } else { "thread" }.to_string(),
            stack: if !handler && v & 0x4 != 0 {
                "psp"
            } else {
                "msp"
            }
            .to_string(),
            fp_frame: v & 0x10 == 0,
        })
    }
}

/// One set status bit, with its meaning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultCause {
    pub register: String,
    pub flag: String,
    pub meaning: String,
}

/// The raw registers, in hex, plus the split CFSR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultRegsHex {
    pub hfsr: String,
    pub cfsr: String,
    pub mmfsr: String,
    pub bfsr: String,
    pub ufsr: String,
    pub mmfar: String,
    pub bfar: String,
}

/// The stacked frame, in hex, and where it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameHex {
    pub sp: String,
    pub r0: String,
    pub r1: String,
    pub r2: String,
    pub r3: String,
    pub r12: String,
    pub lr: String,
    pub pc: String,
    pub xpsr: String,
}

/// Why a data fault happened, when the engine knows more than the registers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataFaultReason {
    /// The store targeted a region with `cpu_writes: fault`.
    ReadOnlyRegion { region: &'static str },
}

/// The verdict: why and where the firmware faulted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultVerdict {
    /// One plain sentence. The headline every surface shows.
    pub summary: String,
    /// "hard_fault", "mem_manage", "bus_fault", "usage_fault" or "lockup".
    pub kind: String,
    /// True when HFSR.FORCED says a configurable fault escalated to HardFault.
    pub escalated: bool,
    /// The configurable fault class the CFSR bits name ("bus_fault",
    /// "mem_manage", "usage_fault"), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Every set status bit, HFSR first, then MMFSR, BFSR, UFSR.
    pub causes: Vec<FaultCause>,
    /// For a BusFault: true when precise, false when imprecise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precise: Option<bool>,
    /// The faulting data address. Present ONLY when BFARVALID/MMARVALID is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault_address: Option<String>,
    /// "BFAR" or "MMFAR": which register `fault_address` came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault_address_source: Option<String>,
    /// The stacked PC (the faulting instruction for a precise fault).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pc: Option<String>,
    /// The stacked LR.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exc_return: Option<ExcReturnInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<FrameHex>,
    /// Where the PC is in the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<CodeLocation>,
    /// The caller, from the stacked LR (a one-frame hint, not a full unwind).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<CodeLocation>,
    pub registers: FaultRegsHex,
}

// ---- Formatting helpers ------------------------------------------------------

fn hex(v: u32) -> String {
    format!("0x{v:08X}")
}

/// `0x2002_0004` — the grouped form used in sentences.
fn hex_grouped(v: u32) -> String {
    format!("0x{:04X}_{:04X}", v >> 16, v & 0xFFFF)
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn set_flags(value: u32, defs: &'static [FlagDef]) -> impl Iterator<Item = &'static FlagDef> {
    defs.iter().filter(move |d| value & (1 << d.bit) != 0)
}

fn exception_name(n: u32) -> &'static str {
    match n {
        2 => "NMI",
        3 => "HardFault",
        4 => "MemManage",
        5 => "BusFault",
        6 => "UsageFault",
        _ => "an exception",
    }
}

fn exception_kind(n: u32) -> &'static str {
    match n {
        4 => "mem_manage",
        5 => "bus_fault",
        6 => "usage_fault",
        _ => "hard_fault",
    }
}

/// The configurable fault class the CFSR names. When bits from more than one
/// class are set, the first in MMFSR, BFSR, UFSR order wins.
fn origin_of(regs: &FaultRegs) -> Option<&'static str> {
    if regs.mmfsr() & !(MMARVALID as u8) != 0 {
        Some("mem_manage")
    } else if regs.bfsr() & !((BFARVALID >> 8) as u8) != 0 {
        Some("bus_fault")
    } else if regs.ufsr() != 0 {
        Some("usage_fault")
    } else {
        None
    }
}

fn origin_name(origin: &str, regs: &FaultRegs) -> &'static str {
    match origin {
        "mem_manage" => "a MemManage fault",
        "bus_fault" if regs.cfsr & PRECISERR != 0 => "a precise BusFault",
        "bus_fault" if regs.cfsr & IMPRECISERR != 0 => "an imprecise BusFault",
        "bus_fault" => "a BusFault",
        _ => "a UsageFault",
    }
}

/// The cause clause: what went wrong, with the address when it is valid.
fn cause_clause(
    regs: &FaultRegs,
    origin: Option<&str>,
    reason: Option<&DataFaultReason>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    for d in set_flags(regs.cfsr, CFSR_FLAGS) {
        let part = match d.flag {
            "PRECISERR" if regs.cfsr & BFARVALID != 0 => match reason {
                Some(DataFaultReason::ReadOnlyRegion { region }) => format!(
                    "data access at {} (BFAR valid; store to read-only {region})",
                    hex_grouped(regs.bfar)
                ),
                None => format!("data access at {} (BFAR valid)", hex_grouped(regs.bfar)),
            },
            "PRECISERR" => "data access (address not captured)".to_string(),
            "IMPRECISERR" => "a buffered write failed; the fault address is not known".to_string(),
            "DACCVIOL" if regs.cfsr & MMARVALID != 0 => format!(
                "data access violation at {} (MMFAR valid)",
                hex_grouped(regs.mmfar)
            ),
            _ => d.meaning.to_string(),
        };
        parts.push(part);
    }
    if parts.is_empty() {
        if regs.hfsr & HFSR_VECTTBL != 0 {
            parts.push("bus error reading the vector table".into());
        } else if regs.hfsr & HFSR_DEBUGEVT != 0 {
            parts.push("debug event (e.g. BKPT) with no debugger attached".into());
        } else if origin.is_none() && regs.hfsr & HFSR_FORCED != 0 {
            parts.push("fault status bits already cleared by the handler".into());
        }
    }
    parts.join("; ")
}

/// The "where" clause: `at PC 0x… in `f` (file:line), called from `g``.
fn where_clause(
    pc: u32,
    imprecise: bool,
    location: Option<&CodeLocation>,
    caller: Option<&CodeLocation>,
) -> String {
    let mut s = if imprecise {
        format!("stacked PC {} (after the faulting access)", hex_grouped(pc))
    } else {
        format!("at PC {}", hex_grouped(pc))
    };
    if let Some(loc) = location {
        let near = if imprecise { " near " } else { " in " };
        match (&loc.function, &loc.file, loc.line) {
            (Some(f), Some(file), Some(line)) => {
                s.push_str(&format!("{near}`{f}` ({}:{line})", basename(file)))
            }
            (Some(f), Some(file), None) => s.push_str(&format!("{near}`{f}` ({})", basename(file))),
            (Some(f), None, _) => s.push_str(&format!("{near}`{f}`")),
            (None, Some(file), Some(line)) => s.push_str(&format!(" ({}:{line})", basename(file))),
            _ => {}
        }
    }
    if let Some(f) = caller.and_then(|c| c.function.as_ref()) {
        s.push_str(&format!(", called from `{f}`"));
    }
    s
}

/// Decode a fault into a verdict. `None` when `capture` records no fault.
///
/// `reason` carries what the engine saw that the registers cannot name.
///
/// Pure: the same inputs always give the same verdict and sentence.
pub fn decode_fault(
    capture: &FaultCapture,
    symbolizer: Option<&dyn FaultSymbolizer>,
    reason: Option<&DataFaultReason>,
) -> Option<FaultVerdict> {
    if !capture.is_fault() {
        return None;
    }
    let regs = &capture.regs;
    let escalated = regs.hfsr & HFSR_FORCED != 0;
    let origin = origin_of(regs);
    let entered = capture.entry.map(|e| e.exception);

    let kind: &'static str = if capture.lockup.is_some() {
        "lockup"
    } else if let Some(n) = entered {
        exception_kind(n)
    } else if escalated || regs.hfsr != 0 {
        "hard_fault"
    } else {
        origin.unwrap_or("hard_fault")
    };

    let busfault = regs.bfsr() & !((BFARVALID >> 8) as u8) != 0;
    let precise = busfault.then_some(regs.cfsr & PRECISERR != 0);
    let imprecise = precise == Some(false) && regs.cfsr & IMPRECISERR != 0;

    let (fault_address, fault_address_source) = if regs.cfsr & BFARVALID != 0 {
        (Some(regs.bfar), Some("BFAR"))
    } else if regs.cfsr & MMARVALID != 0 {
        (Some(regs.mmfar), Some("MMFAR"))
    } else {
        (None, None)
    };

    // The PC: the stacked return address, or the lockup PC.
    let pc = match (capture.lockup, capture.entry) {
        (Some(l), _) => Some(l.pc),
        (None, Some(e)) => Some(e.frame.pc),
        (None, None) => None,
    };
    let stacked_lr = if capture.lockup.is_none() {
        capture.entry.map(|e| e.frame.lr)
    } else {
        None
    };

    let location = pc
        .and_then(|pc| symbolizer.and_then(|s| s.symbolize(pc & !1)))
        .filter(|l| !l.is_empty());
    let caller = stacked_lr
        .filter(|lr| ExcReturnInfo::decode(*lr).is_none() && *lr > 1)
        .and_then(|lr| symbolizer.and_then(|s| s.symbolize((lr & !1).wrapping_sub(2))))
        .filter(|c| c.function.is_some())
        .filter(|c| location.as_ref().and_then(|l| l.function.as_ref()) != c.function.as_ref());

    // ---- The sentence ----
    let cause = cause_clause(regs, origin, reason);
    let head = match (kind, entered) {
        ("lockup", _) => {
            let during = capture
                .lockup
                .map(|l| match l.active_exception {
                    3 => " while the HardFault handler was running (double fault)".to_string(),
                    2 => " while the NMI handler was running".to_string(),
                    0 => String::new(),
                    n => format!(" while {} was running", exception_name(n)),
                })
                .unwrap_or_default();
            match origin {
                Some(o) => format!(
                    "Core LOCKUP: {} could not be taken{during}",
                    origin_name(o, regs)
                ),
                None => format!("Core LOCKUP: a fault could not be taken{during}"),
            }
        }
        ("hard_fault", _) if escalated => match origin {
            Some(o) => format!("HardFault escalated from {}", origin_name(o, regs)),
            None => "HardFault escalated from a configurable fault".to_string(),
        },
        ("hard_fault", _) => "HardFault".to_string(),
        (_, Some(n)) => match (n, precise) {
            (5, Some(true)) => "Precise BusFault".to_string(),
            (5, Some(false)) => "Imprecise BusFault".to_string(),
            _ => exception_name(n).to_string(),
        },
        (k, None) => match k {
            "bus_fault" => "BusFault (handler not yet entered)".to_string(),
            "mem_manage" => "MemManage fault (handler not yet entered)".to_string(),
            _ => "UsageFault (handler not yet entered)".to_string(),
        },
    };
    let mut summary = head;
    if !cause.is_empty() {
        summary.push_str(": ");
        summary.push_str(&cause);
    }
    if let Some(pc) = pc {
        summary.push_str(", ");
        summary.push_str(&where_clause(
            pc,
            imprecise,
            location.as_ref(),
            caller.as_ref(),
        ));
    }
    if kind == "lockup" {
        summary.push_str(". The core stopped");
        // Name the fault that put the core in the handler in the first place:
        // that is usually the bug, the lockup is its consequence.
        if let Some(e) = capture.entry {
            let first = symbolizer
                .and_then(|s| s.symbolize(e.frame.pc & !1))
                .filter(|l| !l.is_empty());
            summary.push_str(&format!(
                "; the first fault ({}) was {}",
                exception_name(e.exception),
                where_clause(e.frame.pc, false, first.as_ref(), None)
            ));
        }
    }
    summary.push('.');

    let mut causes: Vec<FaultCause> = set_flags(regs.hfsr, HFSR_FLAGS)
        .map(|d| FaultCause {
            register: d.register.to_string(),
            flag: d.flag.to_string(),
            meaning: d.meaning.to_string(),
        })
        .collect();
    causes.extend(set_flags(regs.cfsr, CFSR_FLAGS).map(|d| FaultCause {
        register: d.register.to_string(),
        flag: d.flag.to_string(),
        meaning: d.meaning.to_string(),
    }));

    let entry = capture.entry.filter(|_| capture.lockup.is_none());
    Some(FaultVerdict {
        summary,
        kind: kind.to_string(),
        escalated,
        origin: origin.map(str::to_string),
        causes,
        precise,
        fault_address: fault_address.map(hex),
        fault_address_source: fault_address_source.map(str::to_string),
        pc: pc.map(hex),
        lr: stacked_lr.map(hex),
        exc_return: entry.and_then(|e| ExcReturnInfo::decode(e.exc_return)),
        frame: entry.map(|e| FrameHex {
            sp: hex(e.frame_sp),
            r0: hex(e.frame.r0),
            r1: hex(e.frame.r1),
            r2: hex(e.frame.r2),
            r3: hex(e.frame.r3),
            r12: hex(e.frame.r12),
            lr: hex(e.frame.lr),
            pc: hex(e.frame.pc),
            xpsr: hex(e.frame.xpsr),
        }),
        location,
        caller,
        registers: FaultRegsHex {
            hfsr: hex(regs.hfsr),
            cfsr: hex(regs.cfsr),
            mmfsr: format!("0x{:02X}", regs.mmfsr()),
            bfsr: format!("0x{:02X}", regs.bfsr()),
            ufsr: format!("0x{:04X}", regs.ufsr()),
            mmfar: hex(regs.mmfar),
            bfar: hex(regs.bfar),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Syms;
    impl FaultSymbolizer for Syms {
        fn symbolize(&self, addr: u32) -> Option<CodeLocation> {
            let (f, line) = match addr {
                0x0800_1200..=0x0800_12FF => ("sensor_read", 42),
                0x0800_1300..=0x0800_13FF => ("main", 17),
                _ => return None,
            };
            Some(CodeLocation {
                function: Some(f.into()),
                file: Some("/src/app/main.c".into()),
                line: Some(line),
            })
        }
    }

    fn entry(exception: u32, pc: u32, lr: u32) -> FaultEntry {
        FaultEntry {
            exception,
            exc_return: 0xFFFF_FFF9,
            frame_sp: 0x2000_7FE0,
            frame: StackedFrame {
                pc,
                lr,
                xpsr: 0x0100_0000,
                ..Default::default()
            },
        }
    }

    #[test]
    fn nothing_faulted_is_no_verdict() {
        assert_eq!(
            decode_fault(&FaultCapture::default(), Some(&Syms), None),
            None
        );
    }

    #[test]
    fn escalated_precise_busfault_sentence() {
        let cap = FaultCapture {
            regs: FaultRegs {
                hfsr: HFSR_FORCED,
                cfsr: PRECISERR | BFARVALID,
                bfar: 0x2002_0004,
                mmfar: 0,
            },
            entry: Some(entry(3, 0x0800_1234, 0x0800_1305)),
            lockup: None,
        };
        let v = decode_fault(&cap, Some(&Syms), None).unwrap();
        assert_eq!(
            v.summary,
            "HardFault escalated from a precise BusFault: data access at 0x2002_0004 (BFAR valid), \
             at PC 0x0800_1234 in `sensor_read` (main.c:42), called from `main`."
        );
        assert_eq!(v.kind, "hard_fault");
        assert!(v.escalated);
        assert_eq!(v.origin.as_deref(), Some("bus_fault"));
        assert_eq!(v.precise, Some(true));
        assert_eq!(v.fault_address.as_deref(), Some("0x20020004"));
        assert_eq!(v.fault_address_source.as_deref(), Some("BFAR"));
        let ex = v.exc_return.unwrap();
        assert_eq!(
            (ex.return_to.as_str(), ex.stack.as_str(), ex.fp_frame),
            ("thread", "msp", false)
        );
    }

    #[test]
    fn read_only_region_store_names_the_region() {
        // The same capture as `escalated_precise_busfault_sentence`: the
        // registers alone cannot say why the store failed.
        let cap = FaultCapture {
            regs: FaultRegs {
                hfsr: HFSR_FORCED,
                cfsr: PRECISERR | BFARVALID,
                bfar: 0x2002_0004,
                mmfar: 0,
            },
            entry: Some(entry(3, 0x0800_1234, 0x0800_1305)),
            lockup: None,
        };
        let v = decode_fault(
            &cap,
            Some(&Syms),
            Some(&DataFaultReason::ReadOnlyRegion { region: "flash" }),
        )
        .unwrap();
        assert!(
            v.summary.contains("(BFAR valid; store to read-only flash)"),
            "{}",
            v.summary
        );

        let plain = decode_fault(&cap, Some(&Syms), None).unwrap();
        assert!(plain.summary.contains("(BFAR valid)"), "{}", plain.summary);
        assert!(!plain.summary.contains("read-only"), "{}", plain.summary);
    }

    #[test]
    fn imprecise_busfault_never_claims_an_address() {
        let cap = FaultCapture {
            // BFAR holds junk but BFARVALID is clear: it must not be reported.
            regs: FaultRegs {
                hfsr: HFSR_FORCED,
                cfsr: IMPRECISERR,
                bfar: 0xDEAD_BEEF,
                mmfar: 0,
            },
            entry: Some(entry(3, 0x0800_1240, 0x0800_1305)),
            lockup: None,
        };
        let v = decode_fault(&cap, Some(&Syms), None).unwrap();
        assert_eq!(
            v.summary,
            "HardFault escalated from an imprecise BusFault: a buffered write failed; the fault \
             address is not known, stacked PC 0x0800_1240 (after the faulting access) near \
             `sensor_read` (main.c:42), called from `main`."
        );
        assert_eq!(v.fault_address, None);
        assert_eq!(v.precise, Some(false));
    }

    #[test]
    fn direct_usagefault_divbyzero() {
        let cap = FaultCapture {
            regs: FaultRegs {
                cfsr: 1 << 25,
                ..Default::default()
            },
            entry: Some(entry(6, 0x0800_1250, 0xFFFF_FFF9)),
            lockup: None,
        };
        let v = decode_fault(&cap, Some(&Syms), None).unwrap();
        // LR is an EXC_RETURN, so there is no caller hint.
        assert_eq!(
            v.summary,
            "UsageFault: divide by zero (CCR.DIV_0_TRP is set), at PC 0x0800_1250 in \
             `sensor_read` (main.c:42)."
        );
        assert_eq!(v.kind, "usage_fault");
        assert!(!v.escalated);
    }

    #[test]
    fn lockup_double_fault() {
        let cap = FaultCapture {
            regs: FaultRegs {
                hfsr: HFSR_FORCED,
                cfsr: PRECISERR | BFARVALID,
                bfar: 0x9000_0000,
                mmfar: 0,
            },
            entry: Some(entry(3, 0x0800_1234, 0x0800_1305)),
            lockup: Some(LockupEntry {
                pc: 0x0800_1300,
                active_exception: 3,
            }),
        };
        let v = decode_fault(&cap, Some(&Syms), None).unwrap();
        assert_eq!(v.kind, "lockup");
        assert_eq!(
            v.summary,
            "Core LOCKUP: a precise BusFault could not be taken while the HardFault handler was \
             running (double fault): data access at 0x9000_0000 (BFAR valid), at PC 0x0800_1300 \
             in `main` (main.c:17). The core stopped; the first fault (HardFault) was at PC \
             0x0800_1234 in `sensor_read` (main.c:42)."
        );
    }

    #[test]
    fn same_function_lr_is_not_a_caller() {
        let cap = FaultCapture {
            regs: FaultRegs {
                hfsr: HFSR_FORCED,
                cfsr: 1 << 16,
                ..Default::default()
            },
            entry: Some(entry(3, 0x0800_1234, 0x0800_1211)),
            lockup: None,
        };
        let v = decode_fault(&cap, Some(&Syms), None).unwrap();
        assert!(v.caller.is_none());
        assert!(
            v.summary.ends_with("in `sensor_read` (main.c:42)."),
            "{}",
            v.summary
        );
    }

    #[test]
    fn frame_read_follows_exc_return_bit_2() {
        let mem = |a: u32| Some(a); // each word reads back as its own address
        let (sp, f) =
            StackedFrame::read_by_exc_return(0xFFFF_FFFD, 0x2000_1000, 0x2000_2000, mem).unwrap();
        assert_eq!((sp, f.r0, f.pc), (0x2000_2000, 0x2000_2000, 0x2000_2018));
        let (sp, _) =
            StackedFrame::read_by_exc_return(0xFFFF_FFF9, 0x2000_1000, 0x2000_2000, mem).unwrap();
        assert_eq!(sp, 0x2000_1000);
        // Handler-mode return always uses MSP even with bit 2 set in a bogus value.
        let (sp, _) =
            StackedFrame::read_by_exc_return(0xFFFF_FFF1, 0x2000_1000, 0x2000_2000, mem).unwrap();
        assert_eq!(sp, 0x2000_1000);
        assert!(StackedFrame::read_by_exc_return(0x0800_0001, 0, 0, mem).is_none());
    }
}
