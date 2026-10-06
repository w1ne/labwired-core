// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Source-level debugging over any machine: pc <-> file:line, locals, and
//! "step one source line" (into, over or out of calls).
//!
//! The DWARF half ([`SourceDebug`]) is parsed once per firmware. The machine
//! half is [`SourceStepTarget`], which a plain `Machine` implements, so the
//! native debug adapter and any other host share one stepping algorithm. The
//! step drives the machine only through `advance_for_debug`: single
//! instructions while it looks for the next line, and the engine's own batched
//! run (idle fast-forward, breakpoints) for a call it steps over.
//!
//! On a dual-core chip the debugger follows the core the last breakpoint
//! stopped (an Arduino-ESP32 sketch's `loop()` runs on core 1).

use crate::source_map::{LineAddress, SourceMap};
use crate::{DwarfLocation, SymbolProvider};
use labwired_core::debug::SourceStepTarget;
use labwired_core::decoder::xtensa_length;
use labwired_core::snapshot::CpuSnapshot;
use labwired_core::system::arch_policy::MachineFamily;
use labwired_core::{AdvanceRequest, AdvanceStop};
use serde::Serialize;
use std::ops::Range;

/// Step-over runs a call through with at most this many cycles per
/// instruction a source step may single-step: a `delay(1000)` at 240 MHz is
/// 240M cycles, mostly skipped idle.
const RUN_THROUGH_CYCLES_PER_STEP: u64 = 256;

/// Step-out is step-over repeated until the line lands in another function;
/// this bounds how many lines it may cross on the way.
const STEP_OUT_MAX_LINES: u32 = 10_000;

/// The parsed DWARF of one firmware ELF.
pub struct SourceDebug {
    pub symbols: SymbolProvider,
    pub map: SourceMap,
}

/// A pc's place in the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceLocation {
    pub file: String,
    pub line: u32,
    pub column: Option<u32>,
    pub function: Option<String>,
}

/// One local variable or parameter in scope at a pc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Local {
    pub name: String,
    /// "register" | "frame_offset" | "address" | "other"
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub register: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
    /// The current raw 32-bit value; read for `kind == "register"` only.
    pub value: Option<u32>,
}

/// Which source step to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// To the next line, entering calls.
    Into,
    /// To the next line in this function or its caller, running calls through.
    Over,
    /// To the first line back in a different function (the caller).
    Out,
}

/// Why a source step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStop {
    LineChanged,
    Breakpoint,
    Cap,
    Halted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepOutcome {
    pub stop: StepStop,
    /// The debugged core's pc after the step.
    pub pc: u32,
    /// Single steps plus cycles run through calls.
    pub instructions: u32,
}

/// A call or exception being run through: where it returns to, and the
/// debugged core's stack pointer once it has.
struct Pending {
    targets: Vec<u32>,
    sp: u32,
}

/// How [`run_through`] ended.
enum Through {
    Returned,
    Breakpoint(u32),
    Cap,
    Halted,
}

impl SourceDebug {
    /// Parse an ELF's symbols and line table. An ELF without DWARF line
    /// information is an error saying so.
    pub fn from_elf(elf: Vec<u8>) -> Result<Self, String> {
        let symbols =
            SymbolProvider::from_bytes(elf).map_err(|e| format!("firmware debug info: {e:#}"))?;
        let map = SourceMap::from_provider(&symbols);
        if map.is_empty() {
            return Err(
                "the firmware ELF has no DWARF line information (built without \
                        debug info, or stripped)"
                    .into(),
            );
        }
        Ok(Self { symbols, map })
    }

    /// The function symbol whose range holds `pc`, as `start..end`.
    pub fn function_range(&self, pc: u32) -> Option<Range<u64>> {
        let pc = self.map.code_address(u64::from(pc));
        self.symbols
            .functions()
            .iter()
            .rev()
            .find(|(start, size, _)| *start <= pc && pc < start + size)
            .map(|(start, size, _)| *start..start + size)
    }

    /// Where `pc` is. At a function's first instruction the first statement's
    /// line is reported rather than the opening line (see
    /// [`SourceMap::location_at_entry`]).
    pub fn location(&self, pc: u32) -> Option<SourceLocation> {
        let at_entry = self
            .function_range(pc)
            .is_some_and(|r| r.start == self.map.code_address(u64::from(pc)));
        let pos = if at_entry {
            self.map.location_at_entry(u64::from(pc))?
        } else {
            self.map.location(u64::from(pc))?
        };
        let function = self
            .symbols
            .lookup(self.map.code_address(u64::from(pc)))
            .and_then(|l| l.function);
        Some(SourceLocation {
            file: pos.file,
            line: pos.line,
            column: pos.column,
            function,
        })
    }

    /// The code addresses of `file:line` (`file` matches by path suffix).
    pub fn line_to_pc(&self, file: &str, line: u32) -> Option<LineAddress> {
        self.map.line_to_pc(file, line)
    }

    /// The locals and parameters in scope at `pc`, with the current value of
    /// those held in a register of `target`'s debugged core.
    pub fn locals(
        &self,
        target: &dyn SourceStepTarget,
        family: MachineFamily,
        pc: u32,
    ) -> Vec<Local> {
        // DWARF numbers these files from 0 in register order: r0-r15,
        // x0-x31, the current window's a0-a15, AVR r0-r31.
        let reg_count: u16 = match family {
            MachineFamily::CortexM | MachineFamily::Xtensa => 16,
            MachineFamily::RiscV | MachineFamily::Avr => 32,
        };
        let cpu = target.debug_cpu();
        self.symbols
            .find_locals(self.map.code_address(u64::from(pc)))
            .into_iter()
            .map(|l| {
                let mut out = Local {
                    name: l.name,
                    kind: "other",
                    register: None,
                    offset: None,
                    address: None,
                    expr: None,
                    value: None,
                };
                match l.location {
                    DwarfLocation::Register(r) => {
                        out.kind = "register";
                        out.register = Some(r);
                        out.value = (r < reg_count).then(|| cpu.get_register(r as u8));
                    }
                    DwarfLocation::FrameRelative(o) => {
                        out.kind = "frame_offset";
                        out.offset = Some(o);
                    }
                    DwarfLocation::Address(a) => {
                        out.kind = "address";
                        out.address = Some(a as u32);
                    }
                    DwarfLocation::Other(e) => out.expr = Some(e),
                }
                out
            })
            .collect()
    }
}

/// The stack pointer's id for `get_register`.
fn stack_register(family: MachineFamily) -> u8 {
    match family {
        MachineFamily::CortexM => 13,
        MachineFamily::RiscV => 2,
        // a1 of the current register window.
        MachineFamily::Xtensa => 1,
        // SP, after r0..r31.
        MachineFamily::Avr => 32,
    }
}

fn pc_sp(target: &dyn SourceStepTarget, sp_id: u8) -> (u32, u32) {
    let cpu = target.debug_cpu();
    (cpu.get_pc(), cpu.get_register(sp_id))
}

/// The length of the Xtensa `CALLn`/`CALLXn` at `pc`, or `None` for any
/// other instruction. Both are three bytes: `CALLn` is `op0 == 5`,
/// `CALLXn` is `op0 == op1 == op2 == r == 0` with `m == 3`.
fn xtensa_call_at(target: &dyn SourceStepTarget, pc: u32) -> Option<u32> {
    let byte = |i: u32| target.read_code_u8(pc.wrapping_add(i));
    let insn = u32::from(byte(0)?) | u32::from(byte(1)?) << 8 | u32::from(byte(2)?) << 16;
    (insn & 0xF == 5 || insn & 0xFF_F0CF == 0xC0).then_some(3)
}

/// Xtensa `PS.EXCM` of the debugged core: set on taking any exception or
/// interrupt, so its rising edge is the entry to a handler.
fn xtensa_excm(target: &dyn SourceStepTarget) -> bool {
    match target.debug_cpu().snapshot() {
        CpuSnapshot::XtensaLx7(s) => s.ps & (1 << 4) != 0,
        _ => false,
    }
}

/// Runs the machine at batch speed until the debugged core is back at one of
/// `pending.targets` with its stack no deeper than `pending.sp` (a call or
/// exception handler has returned), a breakpoint stops it, `budget` cycles
/// are spent, or the firmware halts. Returns the outcome and the cycles run.
///
/// The targets are engine breakpoints for the duration, so the run is the
/// engine's own and keeps idle fast-forward and batching; single-stepping a
/// `delay(1000)` would take a billion steps.
fn run_through(
    target: &mut dyn SourceStepTarget,
    pending: &Pending,
    sp_id: u8,
    budget: u64,
    breakpoints: &[u32],
) -> Result<(Through, u64), String> {
    let focus = target.debug_core();
    let saved = target.engine_breakpoints();
    let mut user = saved.clone();
    user.extend(breakpoints.iter().map(|a| a & !1));
    user.sort_unstable();
    user.dedup();
    let mut temporary = user.clone();
    temporary.extend(pending.targets.iter().map(|a| a & !1));
    temporary.sort_unstable();
    temporary.dedup();
    target.set_engine_breakpoints(&temporary);

    let mut spent = 0u64;
    let outcome = loop {
        if spent >= budget {
            break Through::Cap;
        }
        let chunk = (budget - spent).min(1 << 22);
        let report = target.advance_for_debug(AdvanceRequest::run(Some(chunk)));
        let ran = match report {
            Ok(r) => r.elapsed_cycles,
            Err(e) => {
                target.set_engine_breakpoints(&saved);
                return Err(format!("run through a call: {e}"));
            }
        };
        spent += ran;
        let Some(at) = target.stopped_breakpoint() else {
            if ran == 0 {
                break Through::Halted;
            }
            continue;
        };
        let core = target.debug_core();
        let (pc, sp) = pc_sp(target, sp_id);
        if core == focus && pending.targets.iter().any(|t| t & !1 == at) && sp >= pending.sp {
            break Through::Returned;
        }
        if user.binary_search(&at).is_ok() {
            break Through::Breakpoint(pc);
        }
        // A return address reached deeper in a recursion, or by the other
        // core running the same code: not the frame being stepped.
        if core != focus {
            target.set_debug_core(focus);
        }
    };
    target.set_engine_breakpoints(&saved);
    Ok((outcome, spent))
}

/// Steps until the source line changes.
///
/// Steps the core a debugger follows; on a dual-core chip the other core runs
/// alongside. Stops at the first statement boundary on a different line than
/// the one it started on ([`StepStop::LineChanged`]); early on a pc in
/// `breakpoints` or in the engine's own breakpoint set
/// ([`StepStop::Breakpoint`]), when the firmware halts ([`StepStop::Halted`]),
/// or after `max_instructions` single steps or 256 cycles per such step run
/// through calls ([`StepStop::Cap`]).
///
/// Step-over spots a call as it happens and runs it at batch speed to the
/// instruction after it:
///
/// - Xtensa (windowed ABI): the instruction about to run is a `CALLn` or
///   `CALLXn`, so the call comes back to the next instruction with `a1`
///   (the caller's stack pointer) as it is now. `RETW` restores the caller's
///   window, so `a1` is the same value on return even when the window was
///   spilled to the stack and reloaded on the way.
/// - AVR: the stack pointer dropped by a pushed return address (2 or 3 bytes)
///   and the pc did not move on to the next instruction. The call returns to
///   the instruction after a 2- or 4-byte call (or, for an interrupt, to where
///   it was) with the stack pointer as it was.
/// - Cortex-M and RISC-V: the pc arrived at the first instruction of a
///   different function (`bl`/`blx`/`jal`/`jalr`, or a tail branch), and the
///   return-address register holds where it comes back to, with the stack no
///   deeper than at the callee's first instruction. Outside the function with
///   the stack deeper than the frame's (an exception handler on Cortex-M)
///   runs on one instruction at a time. Leaving the function with the stack
///   no deeper than the frame's is a return: the caller becomes the frame.
///   Recursion into the stepped function itself is not detected as a call.
///
/// On Xtensa an interrupt or exception taken during either kind of step
/// (`PS.EXCM` rising; a window overflow or underflow is one too) is run
/// through the same way, back to the instruction it interrupted.
///
/// The Cortex-M/RISC-V call's return is judged against the stack pointer at
/// the call, not the one recorded when the step started: a step that starts
/// on a function's first instruction records the stack before the prologue
/// pushes, and a return compared against it would never be seen.
///
/// Step-over also runs through code inlined into the stepped frame from
/// another source file (core's `write_volatile`, a range iterator, a `nop()`
/// delay loop): a statement in a different file, with the stack no shallower
/// than at the start, is still part of the line being stepped. The step ends
/// at the next statement back in the starting file (the starting line too,
/// once it has left it), or in another file once the frame has returned.
pub fn step_source_line(
    target: &mut dyn SourceStepTarget,
    debug: &SourceDebug,
    family: MachineFamily,
    over: bool,
    max_instructions: u32,
    breakpoints: &[u32],
) -> Result<StepOutcome, String> {
    let sp_id = stack_register(family);
    // Cortex-M and RISC-V find a call by its return-address register.
    let ra_id = match family {
        MachineFamily::CortexM => Some(14),
        MachineFamily::RiscV => Some(1),
        _ => None,
    };
    let run_budget = u64::from(max_instructions).saturating_mul(RUN_THROUGH_CYCLES_PER_STEP);
    let xtensa = family == MachineFamily::Xtensa;

    let (start_pc, start_sp) = pc_sp(target, sp_id);
    let start_line = debug.map.line_key(u64::from(start_pc));
    // Set once step-over has run into code inlined from another file.
    let mut left_file = false;
    let mut frame_sp = start_sp;
    let mut frame_fn = debug.function_range(start_pc);

    let mut stepped = 0u32;
    let mut ran_through = 0u64;
    let outcome = |stop, pc, stepped: u32, ran: u64| StepOutcome {
        stop,
        pc,
        instructions: (u64::from(stepped) + ran).min(u64::from(u32::MAX)) as u32,
    };
    while stepped < max_instructions {
        let (before_pc, before_sp) = pc_sp(target, sp_id);
        let call_return = if over && xtensa {
            xtensa_call_at(target, before_pc).map(|len| before_pc.wrapping_add(len))
        } else {
            None
        };
        let excm_before = xtensa && xtensa_excm(target);

        let report = target
            .advance_for_debug(AdvanceRequest::single())
            .map_err(|e| format!("step: {e}"))?;
        stepped += 1;
        let (pc, sp) = pc_sp(target, sp_id);
        if matches!(
            report.stop,
            AdvanceStop::NoProgress | AdvanceStop::FirmwareExit { .. }
        ) {
            return Ok(outcome(StepStop::Halted, pc, stepped, ran_through));
        }
        if breakpoints.contains(&pc) || target.engine_breakpoints().contains(&(pc & !1)) {
            return Ok(outcome(StepStop::Breakpoint, pc, stepped, ran_through));
        }

        let mut pending: Option<Pending> = None;
        if xtensa && !excm_before && xtensa_excm(target) {
            // Back to the interrupted instruction, or the one after it when
            // the exception came after it retired.
            let byte0 = target.read_code_u8(before_pc).unwrap_or(0);
            let next = before_pc.wrapping_add(xtensa_length::instruction_length(byte0));
            let mut targets = vec![before_pc, next];
            targets.extend(call_return);
            pending = Some(Pending {
                targets,
                sp: before_sp,
            });
        } else if let Some(ret) = call_return.filter(|r| *r != pc) {
            pending = Some(Pending {
                targets: vec![ret],
                sp: before_sp,
            });
        } else if over && family == MachineFamily::Avr {
            let pushed = before_sp.wrapping_sub(sp);
            let next = [before_pc.wrapping_add(2), before_pc.wrapping_add(4)];
            if (2..=3).contains(&pushed) && !next.contains(&pc) {
                pending = Some(Pending {
                    targets: vec![before_pc, next[0], next[1]],
                    sp: before_sp,
                });
            }
        } else if let (true, Some(ra_id)) = (over, ra_id) {
            let here = u64::from(pc & !1);
            if !frame_fn.as_ref().is_some_and(|r| r.contains(&here)) {
                let callee = debug.function_range(pc);
                // A Cortex-M exception entry also lands on a function start,
                // with EXC_RETURN (0xFFFF_FFxx) in LR rather than an address:
                // the stack rule below covers it.
                let ra = target.debug_cpu().get_register(ra_id) & !1;
                if callee.as_ref().is_some_and(|r| r.start == here) && ra < 0xF000_0000 {
                    pending = Some(Pending {
                        targets: vec![ra],
                        sp,
                    });
                } else if sp < frame_sp {
                    continue;
                } else {
                    // Equal counts as a return: a frame recorded at its
                    // function's entry, or one with no prologue, holds the
                    // caller's stack pointer.
                    frame_sp = sp;
                    frame_fn = callee;
                }
            }
        }

        if let Some(pending) = pending {
            let budget = run_budget.saturating_sub(ran_through);
            let (through, ran) = run_through(target, &pending, sp_id, budget, breakpoints)?;
            ran_through += ran;
            let pc = pc_sp(target, sp_id).0;
            match through {
                Through::Returned => {}
                Through::Breakpoint(at) => {
                    return Ok(outcome(StepStop::Breakpoint, at, stepped, ran_through))
                }
                Through::Cap => return Ok(outcome(StepStop::Cap, pc, stepped, ran_through)),
                Through::Halted => return Ok(outcome(StepStop::Halted, pc, stepped, ran_through)),
            }
        }

        let (pc, sp) = pc_sp(target, sp_id);
        if debug.map.is_statement(u64::from(pc)) {
            let key = debug.map.line_key(u64::from(pc));
            if key != start_line || left_file {
                let inlined_elsewhere = over
                    && key.is_some()
                    && start_line.is_some()
                    && key.map(|k| k.0) != start_line.map(|k| k.0)
                    && sp <= start_sp;
                if !inlined_elsewhere {
                    return Ok(outcome(StepStop::LineChanged, pc, stepped, ran_through));
                }
                left_file = true;
            }
        }
    }
    let pc = pc_sp(target, sp_id).0;
    Ok(outcome(StepStop::Cap, pc, stepped, ran_through))
}

/// One source step of `kind`. `Out` steps over line by line until a line in a
/// different function (the caller) is reached, or anything else stops it.
pub fn step(
    target: &mut dyn SourceStepTarget,
    debug: &SourceDebug,
    family: MachineFamily,
    kind: StepKind,
    max_instructions: u32,
    breakpoints: &[u32],
) -> Result<StepOutcome, String> {
    match kind {
        StepKind::Into => {
            step_source_line(target, debug, family, false, max_instructions, breakpoints)
        }
        StepKind::Over => {
            step_source_line(target, debug, family, true, max_instructions, breakpoints)
        }
        StepKind::Out => {
            let start = target.debug_cpu().get_pc();
            let function = debug.function_range(start);
            let mut total = 0u32;
            for _ in 0..STEP_OUT_MAX_LINES {
                let mut out =
                    step_source_line(target, debug, family, true, max_instructions, breakpoints)?;
                total = total.saturating_add(out.instructions);
                out.instructions = total;
                if out.stop != StepStop::LineChanged || debug.function_range(out.pc) != function {
                    return Ok(out);
                }
            }
            let pc = target.debug_cpu().get_pc();
            Ok(StepOutcome {
                stop: StepStop::Cap,
                pc,
                instructions: total,
            })
        }
    }
}
