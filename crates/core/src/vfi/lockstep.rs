// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Scheduled fault injection with a lockstep verdict.
//!
//! Two machines built from the same inputs are identical and deterministic, so
//! they stay identical until something makes them differ. This module makes
//! one thing differ on purpose: at a chosen cycle it flips a bit in a CPU
//! register or in memory (RAM or a peripheral register, through the bus), or
//! skips an instruction, on the *faulted* machine only. It then steps the
//! *golden* and the faulted machine one instruction at a time and compares
//! their architectural registers after every instruction, the same parity
//! check [`super::ShadowEngine`] performs.
//!
//! The result is a [`FaultReport`]: when the two first differed, whether they
//! became equal again (the fault was masked), whether control flow split,
//! whether the console output changed, and whether the faulted machine stopped
//! with an error. Every surface (CLI, Python, wasm, MCP) serialises this same
//! report, so a verdict means the same thing everywhere.
//!
//! What it compares: the core registers [`DebugControl::read_core_reg`]
//! exposes (for Cortex-M R0–R15, for RISC-V x0–x31 and pc; status flags are
//! not in that set), the console bytes each side printed, and for a memory
//! flip the flipped byte itself at the end of the run. It does not diff all of
//! RAM or peripheral state: a flip that is never read and never overwritten
//! shows as [`FaultVerdict::Latent`], not as masked.

use crate::DebugControl;
use serde::{Deserialize, Serialize};

/// Instruction set, used only to size an [`FaultAction::InstructionSkip`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Isa {
    /// ARMv6-M/ARMv7-M Thumb: 16- or 32-bit.
    Thumb,
    /// RV32 with the C extension: 16- or 32-bit.
    RiscV,
    /// Xtensa LX with the density option: 16- or 24-bit.
    Xtensa,
    /// AVR8: 16- or 32-bit.
    Avr,
}

impl Isa {
    /// The ISA for a firmware architecture, `None` when it is unknown.
    pub fn from_arch(arch: crate::Arch) -> Option<Isa> {
        match arch {
            crate::Arch::Arm => Some(Isa::Thumb),
            crate::Arch::RiscV => Some(Isa::RiscV),
            crate::Arch::XtensaLx7 => Some(Isa::Xtensa),
            crate::Arch::Avr => Some(Isa::Avr),
            crate::Arch::Unknown => None,
        }
    }

    /// The ISA a machine family executes.
    pub fn from_family(family: crate::system::arch_policy::MachineFamily) -> Isa {
        use crate::system::arch_policy::MachineFamily;
        match family {
            MachineFamily::CortexM => Isa::Thumb,
            MachineFamily::RiscV => Isa::RiscV,
            MachineFamily::Xtensa => Isa::Xtensa,
            MachineFamily::Avr => Isa::Avr,
        }
    }

    /// Length in bytes of the instruction whose first bytes are `bytes`
    /// (little-endian, at least 2 bytes).
    pub fn instruction_len(self, bytes: &[u8]) -> u32 {
        let b0 = bytes.first().copied().unwrap_or(0);
        let b1 = bytes.get(1).copied().unwrap_or(0);
        let half = u16::from_le_bytes([b0, b1]);
        match self {
            // Thumb-2: a first halfword whose top five bits are 0b11101,
            // 0b11110 or 0b11111 starts a 32-bit instruction.
            Isa::Thumb => {
                if matches!(half >> 11, 0b11101..=0b11111) {
                    4
                } else {
                    2
                }
            }
            // RISC-V: the low two bits are 0b11 for a 32-bit instruction.
            Isa::RiscV => {
                if b0 & 0b11 == 0b11 {
                    4
                } else {
                    2
                }
            }
            // Xtensa: op0 (low nibble) 8..=13 is a narrow 16-bit instruction.
            Isa::Xtensa => {
                if (8..=13).contains(&(b0 & 0x0F)) {
                    2
                } else {
                    3
                }
            }
            // AVR: JMP, CALL, LDS and STS are the 32-bit instructions.
            Isa::Avr => {
                let is_jmp_call = half & 0xFE0C == 0x940C;
                let is_lds_sts = half & 0xFC0F == 0x9000;
                if is_jmp_call || is_lds_sts {
                    4
                } else {
                    2
                }
            }
        }
    }
}

/// A CPU register named by index or by name (`"R3"`, `"sp"`, `"x10"`, `"pc"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RegisterRef {
    Index(u8),
    Name(String),
}

impl RegisterRef {
    /// Resolve against the machine's register names (case-insensitive).
    pub fn resolve(&self, names: &[String]) -> Result<u8, String> {
        match self {
            RegisterRef::Index(i) => {
                if (*i as usize) < names.len() {
                    Ok(*i)
                } else {
                    Err(format!(
                        "register index {i} is out of range; this CPU has {} registers ({})",
                        names.len(),
                        names.join(", ")
                    ))
                }
            }
            RegisterRef::Name(name) => names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(name))
                .map(|i| i as u8)
                .ok_or_else(|| {
                    format!(
                        "unknown register '{name}'; this CPU has {}",
                        names.join(", ")
                    )
                }),
        }
    }
}

/// What a scheduled fault does to the faulted machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FaultAction {
    /// XOR `1 << bit` into a core register.
    RegisterBitFlip { register: RegisterRef, bit: u8 },
    /// XOR `1 << bit` into the byte at `address`, read and written through the
    /// bus. On RAM this is a plain upset; on a peripheral register the read and
    /// the write have that register's side effects, as they would on silicon.
    MemoryBitFlip { address: u64, bit: u8 },
    /// Do not execute the next instruction: the faulted PC moves past it while
    /// the golden machine executes it (a classic glitch model).
    InstructionSkip,
}

/// A fault and the cycle it fires at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledFault {
    /// Absolute machine cycle (since power-on). The fault fires before the
    /// first instruction that starts at or after this cycle.
    pub at_cycle: u64,
    #[serde(flatten)]
    pub action: FaultAction,
}

/// A fault experiment: the faults, and how far to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultPlan {
    pub faults: Vec<ScheduledFault>,
    /// Stop when the golden machine reaches this absolute cycle.
    pub until_cycle: u64,
}

/// The lockstep verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultVerdict {
    /// No fault fired before the run ended.
    NotInjected,
    /// The fault changed state, and the machines became identical again (or it
    /// never changed a compared register and left nothing behind).
    Masked,
    /// Registers and console match at the end, but a flipped memory byte still
    /// differs: the upset is sitting in memory, unread.
    Latent,
    /// The machines ended in different register states; the console matches.
    Diverged,
    /// The console output differs.
    OutputChanged,
    /// The faulted machine stopped (an error, or it stopped making progress)
    /// while the golden machine did not.
    Crashed,
}

/// One register that differed at the first divergence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterDiff {
    pub register: String,
    pub golden: u32,
    pub faulted: u32,
}

/// Where the machines first differed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Divergence {
    /// Golden machine cycle at the comparison that found it.
    pub cycle: u64,
    /// Instructions both machines executed after the first fault fired.
    pub instructions_after_fault: u64,
    pub golden_pc: u32,
    pub faulted_pc: u32,
    /// `true` when the PCs differ (control flow split), not only data.
    pub control_flow: bool,
    pub registers: Vec<RegisterDiff>,
}

/// A fault that fired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectedFault {
    /// Index into [`FaultPlan::faults`].
    pub index: usize,
    pub at_cycle: u64,
    /// The faulted machine's cycle when it fired.
    pub applied_cycle: u64,
    /// The faulted machine's PC when it fired.
    pub pc: u32,
    /// What changed, e.g. `"R3 bit 4: 0x00000010 -> 0x00000000"`.
    pub detail: String,
}

/// One side's state at the end of the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideEnd {
    pub cycles: u64,
    pub pc: u32,
    /// Why this side stopped before the end, if it did.
    pub stopped: Option<String>,
    pub console_bytes: usize,
}

/// A memory byte a flip targeted, as it ended on both machines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDiff {
    pub address: u64,
    pub golden: Option<u8>,
    pub faulted: Option<u8>,
}

/// Everything a fault experiment found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultReport {
    pub verdict: FaultVerdict,
    /// One sentence a person can read.
    pub summary: String,
    pub start_cycle: u64,
    pub until_cycle: u64,
    pub injected: Vec<InjectedFault>,
    /// Indices of faults whose cycle the run never reached.
    pub not_injected: Vec<usize>,
    pub first_divergence: Option<Divergence>,
    /// Golden cycle at which a data divergence disappeared again.
    pub reconverged_cycle: Option<u64>,
    /// First byte offset where the console output differs.
    pub console_divergence_offset: Option<usize>,
    pub memory: Vec<MemoryDiff>,
    pub golden: SideEnd,
    pub faulted: SideEnd,
    /// Instructions compared one by one in lockstep.
    pub lockstep_instructions: u64,
}

/// A machine plus the console it prints to, as the lockstep engine drives it.
pub trait LockstepTarget {
    fn machine(&mut self) -> &mut dyn DebugControl;
    fn machine_ref(&self) -> &dyn DebugControl;
    /// Every console byte printed since the machine was built.
    fn console(&self) -> Vec<u8>;
}

/// A [`LockstepTarget`] over a machine and a shared console sink.
pub struct SinkTarget<'a> {
    pub machine: &'a mut dyn DebugControl,
    pub console: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
}

impl LockstepTarget for SinkTarget<'_> {
    fn machine(&mut self) -> &mut dyn DebugControl {
        self.machine
    }
    fn machine_ref(&self) -> &dyn DebugControl {
        self.machine
    }
    fn console(&self) -> Vec<u8> {
        self.console
            .lock()
            .map(|c| c.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }
}

/// Instructions per chunk once control flow has split and the sides run
/// independently.
const FREE_RUN_CHUNK: u32 = 10_000;

/// Registers as the parity check sees them.
fn registers(m: &dyn DebugControl, count: usize) -> Vec<u32> {
    (0..count).map(|i| m.read_core_reg(i as u8)).collect()
}

/// Step one instruction; `Err(reason)` when the side stopped.
fn step_side(m: &mut dyn DebugControl) -> Result<(), String> {
    let (pc, cycles) = (m.get_pc(), m.get_cycle_count());
    match m.step_single() {
        Err(e) => Err(format!("{e}")),
        Ok(crate::StopReason::FirmwareExit(code)) => Err(format!("firmware exited with {code}")),
        Ok(_) if m.get_pc() == pc && m.get_cycle_count() == cycles => {
            Err(format!("no forward progress at PC {pc:#010x}"))
        }
        Ok(_) => Ok(()),
    }
}

/// Run a side until `until` cycles, one chunk at a time, applying any faults
/// in `faults` that fall due (faulted side only).
fn free_run(
    m: &mut dyn DebugControl,
    until: u64,
    mut apply_due: impl FnMut(&mut dyn DebugControl) -> Result<(), String>,
    next_due: impl Fn() -> Option<u64>,
) -> Option<String> {
    while m.get_cycle_count() < until {
        if let Err(e) = apply_due(m) {
            return Some(e);
        }
        let now = m.get_cycle_count();
        // Step singly when a fault is due soon, so it fires on its cycle.
        let single = next_due().is_some_and(|c| c <= now.saturating_add(u64::from(FREE_RUN_CHUNK)));
        let r = if single {
            step_side(m)
        } else {
            let steps = (until - now).min(u64::from(FREE_RUN_CHUNK)) as u32;
            let before = (m.get_pc(), now);
            match m.run(Some(steps.max(1))) {
                Err(e) => Err(format!("{e}")),
                Ok(crate::StopReason::FirmwareExit(code)) => {
                    Err(format!("firmware exited with {code}"))
                }
                Ok(_) if (m.get_pc(), m.get_cycle_count()) == before => {
                    Err(format!("no forward progress at PC {:#010x}", before.0))
                }
                Ok(_) => Ok(()),
            }
        };
        if let Err(e) = r {
            return Some(e);
        }
    }
    None
}

/// Apply one fault to the faulted machine. Returns the human-readable detail.
fn apply_fault(
    m: &mut dyn DebugControl,
    action: &FaultAction,
    names: &[String],
    isa: Isa,
) -> Result<String, String> {
    match action {
        FaultAction::RegisterBitFlip { register, bit } => {
            if *bit > 31 {
                return Err(format!("bit {bit} is out of range for a 32-bit register"));
            }
            let id = register.resolve(names)?;
            let old = m.read_core_reg(id);
            let new = old ^ (1u32 << bit);
            // The PC is written through set_pc so the core's fetch state follows.
            if names
                .get(id as usize)
                .is_some_and(|n| n.eq_ignore_ascii_case("pc"))
            {
                m.set_pc(new);
            } else {
                m.write_core_reg(id, new);
            }
            Ok(format!(
                "{} bit {bit}: {old:#010x} -> {new:#010x}",
                names[id as usize]
            ))
        }
        FaultAction::MemoryBitFlip { address, bit } => {
            if *bit > 7 {
                return Err(format!("bit {bit} is out of range for a byte"));
            }
            let addr = u32::try_from(*address)
                .map_err(|_| format!("address {address:#x} is outside the 32-bit bus"))?;
            let old = m
                .read_memory(addr, 1)
                .map_err(|e| format!("reading {addr:#010x}: {e}"))?[0];
            let new = old ^ (1u8 << bit);
            m.write_memory(addr, &[new])
                .map_err(|e| format!("writing {addr:#010x}: {e}"))?;
            Ok(format!(
                "[{addr:#010x}] bit {bit}: {old:#04x} -> {new:#04x}"
            ))
        }
        FaultAction::InstructionSkip => {
            let pc = m.get_pc();
            let bytes = m
                .read_memory(pc, 4)
                .or_else(|_| m.read_memory(pc, 2))
                .map_err(|e| format!("reading the instruction at {pc:#010x}: {e}"))?;
            let len = isa.instruction_len(&bytes);
            m.set_pc(pc.wrapping_add(len));
            Ok(format!(
                "skipped the {}-byte instruction at {pc:#010x}",
                len
            ))
        }
    }
}

fn diff_registers(names: &[String], g: &[u32], f: &[u32]) -> Vec<RegisterDiff> {
    g.iter()
        .zip(f)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (a, b))| RegisterDiff {
            register: names.get(i).cloned().unwrap_or_else(|| format!("r{i}")),
            golden: *a,
            faulted: *b,
        })
        .collect()
}

/// Run a fault experiment in lockstep.
///
/// `golden` and `faulted` must be two builds of the same machine at the same
/// point (same cycle count, same registers); anything else is an error rather
/// than a verdict. Faults whose `at_cycle` is already in the past fire before
/// the first instruction.
pub fn run_lockstep(
    golden: &mut dyn LockstepTarget,
    faulted: &mut dyn LockstepTarget,
    plan: &FaultPlan,
    isa: Isa,
) -> Result<FaultReport, String> {
    let names = golden.machine_ref().get_register_names();
    let count = names.len();
    let start = golden.machine_ref().get_cycle_count();
    if faulted.machine_ref().get_cycle_count() != start
        || registers(golden.machine_ref(), count) != registers(faulted.machine_ref(), count)
    {
        return Err(
            "the golden and faulted machines are not at the same point; build both from the \
             same inputs before injecting"
                .into(),
        );
    }
    if plan.until_cycle <= start {
        return Err(format!(
            "until_cycle {} is not after the current cycle {start}",
            plan.until_cycle
        ));
    }
    for f in &plan.faults {
        if let FaultAction::RegisterBitFlip { register, .. } = &f.action {
            register.resolve(&names)?;
        }
    }

    let mut order: Vec<usize> = (0..plan.faults.len()).collect();
    order.sort_by_key(|&i| (plan.faults[i].at_cycle, i));
    let mut next = 0usize; // position in `order`
    let mut injected: Vec<InjectedFault> = Vec::new();

    #[derive(PartialEq)]
    enum Phase {
        Identical,
        DataDiverged,
        ControlDiverged,
    }
    let mut phase = Phase::Identical;
    let mut first: Option<Divergence> = None;
    let mut reconverged: Option<u64> = None;
    let mut steps_after_fault = 0u64;
    let mut lockstep = 0u64;
    let mut golden_stop: Option<String> = None;
    let mut faulted_stop: Option<String> = None;

    // Lockstep phase: both sides execute the same instruction sequence.
    while phase != Phase::ControlDiverged
        && golden.machine_ref().get_cycle_count() < plan.until_cycle
    {
        // Fire every fault now due on the faulted side.
        let mut skip_faulted = false;
        while next < order.len()
            && plan.faults[order[next]].at_cycle <= faulted.machine_ref().get_cycle_count()
        {
            let idx = order[next];
            let fm = faulted.machine();
            let (applied_cycle, pc) = (fm.get_cycle_count(), fm.get_pc());
            let fault = &plan.faults[idx];
            if fault.action == FaultAction::InstructionSkip {
                // The golden side executes the instruction below; the faulted
                // side is moved past it without executing it.
                skip_faulted = true;
            }
            let detail = apply_fault(fm, &fault.action, &names, isa)?;
            injected.push(InjectedFault {
                index: idx,
                at_cycle: fault.at_cycle,
                applied_cycle,
                pc,
                detail,
            });
            next += 1;
        }
        if let Err(e) = step_side(golden.machine()) {
            golden_stop = Some(e);
        }
        if !skip_faulted {
            if let Err(e) = step_side(faulted.machine()) {
                faulted_stop = Some(e);
            }
        }
        lockstep += 1;
        if !injected.is_empty() {
            steps_after_fault += 1;
        }
        if golden_stop.is_some() || faulted_stop.is_some() {
            if first.is_none() && golden_stop.is_none() {
                let (g, f) = (golden.machine_ref(), faulted.machine_ref());
                let (gr, fr) = (registers(g, count), registers(f, count));
                first = Some(Divergence {
                    cycle: g.get_cycle_count(),
                    instructions_after_fault: steps_after_fault,
                    golden_pc: g.get_pc(),
                    faulted_pc: f.get_pc(),
                    control_flow: true,
                    registers: diff_registers(&names, &gr, &fr),
                });
            }
            break;
        }
        let (g, f) = (golden.machine_ref(), faulted.machine_ref());
        let (gr, fr) = (registers(g, count), registers(f, count));
        let (gpc, fpc) = (g.get_pc(), f.get_pc());
        if gpc != fpc || gr != fr {
            let control = gpc != fpc;
            if first.is_none() {
                first = Some(Divergence {
                    cycle: g.get_cycle_count(),
                    instructions_after_fault: steps_after_fault,
                    golden_pc: gpc,
                    faulted_pc: fpc,
                    control_flow: control,
                    registers: diff_registers(&names, &gr, &fr),
                });
            }
            reconverged = None;
            phase = if control {
                Phase::ControlDiverged
            } else {
                Phase::DataDiverged
            };
        } else if phase == Phase::DataDiverged {
            reconverged = Some(g.get_cycle_count());
            phase = Phase::Identical;
        }
    }

    // Free-run phase: control flow split, each side runs to the end alone.
    if golden_stop.is_none() {
        golden_stop = free_run(golden.machine(), plan.until_cycle, |_| Ok(()), || None);
    }
    if faulted_stop.is_none() {
        let faults = &plan.faults;
        let order_ref = &order;
        let next_cell = std::cell::Cell::new(next);
        let injected_ref = &mut injected;
        let mut apply_err: Option<String> = None;
        faulted_stop = free_run(
            faulted.machine(),
            plan.until_cycle,
            |m| {
                let mut n = next_cell.get();
                while n < order_ref.len() && faults[order_ref[n]].at_cycle <= m.get_cycle_count() {
                    let idx = order_ref[n];
                    let (applied_cycle, pc) = (m.get_cycle_count(), m.get_pc());
                    let detail = apply_fault(m, &faults[idx].action, &names, isa)
                        .inspect_err(|e| apply_err = Some(e.clone()))?;
                    injected_ref.push(InjectedFault {
                        index: idx,
                        at_cycle: faults[idx].at_cycle,
                        applied_cycle,
                        pc,
                        detail,
                    });
                    n += 1;
                }
                next_cell.set(n);
                Ok(())
            },
            || order_ref.get(next_cell.get()).map(|&i| faults[i].at_cycle),
        );
        next = next_cell.get();
        if let Some(e) = apply_err {
            return Err(e);
        }
    }

    let not_injected: Vec<usize> = order[next..].to_vec();

    // Memory bytes the flips targeted, as they ended on each side.
    let mut memory = Vec::new();
    for inj in &injected {
        if let FaultAction::MemoryBitFlip { address, .. } = plan.faults[inj.index].action {
            if memory.iter().any(|m: &MemoryDiff| m.address == address) {
                continue;
            }
            let read = |m: &dyn DebugControl| {
                u32::try_from(address)
                    .ok()
                    .and_then(|a| m.read_memory(a, 1).ok())
                    .map(|b| b[0])
            };
            memory.push(MemoryDiff {
                address,
                golden: read(golden.machine_ref()),
                faulted: read(faulted.machine_ref()),
            });
        }
    }

    let (gc, fc) = (golden.console(), faulted.console());
    let console_divergence_offset = if gc == fc {
        None
    } else {
        Some(
            gc.iter()
                .zip(&fc)
                .position(|(a, b)| a != b)
                .unwrap_or(gc.len().min(fc.len())),
        )
    };
    let (g, f) = (golden.machine_ref(), faulted.machine_ref());
    let end_equal = g.get_pc() == f.get_pc() && registers(g, count) == registers(f, count);
    let memory_differs = memory.iter().any(|m| m.golden != m.faulted);

    let verdict = if injected.is_empty() {
        FaultVerdict::NotInjected
    } else if faulted_stop.is_some() && golden_stop.is_none() {
        FaultVerdict::Crashed
    } else if console_divergence_offset.is_some() {
        FaultVerdict::OutputChanged
    } else if !end_equal || phase != Phase::Identical {
        FaultVerdict::Diverged
    } else if memory_differs {
        FaultVerdict::Latent
    } else {
        FaultVerdict::Masked
    };

    let summary = match verdict {
        FaultVerdict::NotInjected => format!(
            "no fault fired: the run ended at cycle {} before the first scheduled cycle",
            g.get_cycle_count()
        ),
        FaultVerdict::Masked => match (&first, reconverged) {
            (Some(d), Some(c)) => format!(
                "masked: the machines differed at cycle {} and were identical again from cycle {c}",
                d.cycle
            ),
            _ => "masked: the fault changed no compared state".to_string(),
        },
        FaultVerdict::Latent => {
            "latent: registers and console match, but the flipped memory still differs".into()
        }
        FaultVerdict::Diverged => match &first {
            Some(d) if d.control_flow => format!(
                "diverged: control flow split at cycle {} (golden PC {:#010x}, faulted PC {:#010x}); \
                 the console still matches",
                d.cycle, d.golden_pc, d.faulted_pc
            ),
            Some(d) => format!(
                "diverged: registers differ from cycle {} to the end; the console still matches",
                d.cycle
            ),
            None => "diverged: the machines ended in different states".into(),
        },
        FaultVerdict::OutputChanged => format!(
            "output changed: the console differs from byte {}",
            console_divergence_offset.unwrap_or(0)
        ),
        FaultVerdict::Crashed => format!(
            "crashed: the faulted machine stopped ({}) while the golden one ran on",
            faulted_stop.as_deref().unwrap_or("stopped")
        ),
    };

    Ok(FaultReport {
        verdict,
        summary,
        start_cycle: start,
        until_cycle: plan.until_cycle,
        injected,
        not_injected,
        first_divergence: first,
        reconverged_cycle: reconverged,
        console_divergence_offset,
        memory,
        golden: SideEnd {
            cycles: g.get_cycle_count(),
            pc: g.get_pc(),
            stopped: golden_stop,
            console_bytes: gc.len(),
        },
        faulted: SideEnd {
            cycles: f.get_cycle_count(),
            pc: f.get_pc(),
            stopped: faulted_stop,
            console_bytes: fc.len(),
        },
        lockstep_instructions: lockstep,
    })
}
