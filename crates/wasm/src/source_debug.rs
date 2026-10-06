// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Source-level debugging for the browser: pc <-> file:line, the lines that
//! carry code, locals, and "step one source line" (into or over calls).
//!
//! The firmware ELF's DWARF is parsed once per simulator (cached in
//! `SimTools::source_debug`) by `labwired_loader`'s `SymbolProvider` and
//! `SourceMap`. Nothing here runs unless JS calls it: the normal run loop
//! (`step_batch`) is untouched, and `step_source_line` drives the machine with
//! the same single-instruction advance as `step_single`, journaling each step
//! as a `StepSingle`, and runs a call it steps over through `step_batch` with
//! engine breakpoints, journaled as those calls, so snapshots and replays
//! stay exact.
//!
//! On a dual-core chip the debugger follows the core the last breakpoint
//! stopped (`Machine::breakpoint_core`): an Arduino-ESP32 sketch's `loop()`
//! runs on core 1.

use crate::lab_tools::Op;
use crate::WasmSimulator;
use labwired_core::decoder::xtensa_length;
use labwired_core::snapshot::CpuSnapshot;
use labwired_core::system::arch_policy::MachineFamily;
use labwired_core::{AdvanceRequest, AdvanceStop, Bus, Cpu, Machine};
use labwired_loader::source_map::SourceMap;
use labwired_loader::{DwarfLocation, SymbolProvider};
use serde::Serialize;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

/// Step-over runs a call through with at most this many cycles per
/// instruction a source step may single-step: a `delay(1000)` at 240 MHz is
/// 240M cycles, mostly skipped idle.
const RUN_THROUGH_CYCLES_PER_STEP: u64 = 256;

/// The core a debugger shows and steps: core 0, or the second core of a
/// dual-core chip once a breakpoint stopped that one (`Machine::breakpoint_core`).
pub(crate) fn focused_cpu(machine: &Machine<Box<dyn Cpu>>) -> &dyn Cpu {
    match (machine.breakpoint_core, machine.cpu_secondary.as_ref()) {
        (1, Some(cpu)) => cpu.as_ref(),
        _ => machine.cpu.as_ref(),
    }
}

/// A call or exception being run through: where it returns to, and the
/// focused core's stack pointer once it has.
struct Pending {
    targets: Vec<u32>,
    sp: u32,
}

/// How [`WasmSimulator::run_through`] ended.
enum Through {
    Returned,
    Breakpoint(u32),
    Cap,
    Halted,
}

/// The parsed DWARF of the loaded firmware.
pub(crate) struct SourceDebug {
    symbols: SymbolProvider,
    map: SourceMap,
}

#[derive(Serialize)]
struct Location {
    file: String,
    line: u32,
    column: Option<u32>,
    function: Option<String>,
}

#[derive(Serialize)]
struct LineHit {
    pc: u32,
    pcs: Vec<u32>,
    file: String,
    line: u32,
}

#[derive(Serialize)]
struct Local {
    name: String,
    /// "register" | "frame_offset" | "address" | "other"
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    register: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expr: Option<String>,
    /// The current raw 32-bit value; read only for `kind == "register"`.
    value: Option<u32>,
}

#[derive(Serialize)]
struct StepOutcome {
    /// "line_changed" | "breakpoint" | "cap" | "halted"
    reason: &'static str,
    pc: u32,
    instructions: u32,
    location: Option<Location>,
}

fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| JsValue::from_str(&format!("source debug: {e}")))
}

fn js(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

impl WasmSimulator {
    fn source_debug_s(&self) -> Result<Rc<SourceDebug>, String> {
        if let Some(cached) = self.tools.borrow().source_debug.clone() {
            return Ok(cached);
        }
        let elf = self.firmware_elf().ok_or(
            "source debugging needs the firmware ELF; this machine was built from flash \
             images only",
        )?;
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
        let parsed = Rc::new(SourceDebug { symbols, map });
        self.tools.borrow_mut().source_debug = Some(parsed.clone());
        Ok(parsed)
    }

    /// The function symbol whose range holds `pc`, as `start..end`.
    fn function_range(debug: &SourceDebug, pc: u32) -> Option<std::ops::Range<u64>> {
        let pc = debug.map.code_address(u64::from(pc));
        debug
            .symbols
            .functions()
            .iter()
            .rev()
            .find(|(start, size, _)| *start <= pc && pc < start + size)
            .map(|(start, size, _)| *start..start + size)
    }

    /// At a function's first instruction the first statement's line is
    /// reported rather than the opening line (see
    /// [`SourceMap::location_at_entry`]).
    fn location_of(debug: &SourceDebug, pc: u32) -> Option<Location> {
        let at_entry = Self::function_range(debug, pc)
            .is_some_and(|r| r.start == debug.map.code_address(u64::from(pc)));
        let pos = if at_entry {
            debug.map.location_at_entry(u64::from(pc))?
        } else {
            debug.map.location(u64::from(pc))?
        };
        let function = debug
            .symbols
            .lookup(debug.map.code_address(u64::from(pc)))
            .and_then(|l| l.function);
        Some(Location {
            file: pos.file,
            line: pos.line,
            column: pos.column,
            function,
        })
    }

    fn locals_s(&self, pc: u32) -> Result<Vec<Local>, JsValue> {
        let debug = self.source_debug_s().map_err(js)?;
        let machine = self.machine_or_err()?;
        // DWARF numbers these files from 0 in register order: r0-r15,
        // x0-x31, the current window's a0-a15, AVR r0-r31.
        let reg_count: u16 = match self.arch {
            MachineFamily::CortexM | MachineFamily::Xtensa => 16,
            MachineFamily::RiscV | MachineFamily::Avr => 32,
        };
        let cpu = focused_cpu(machine);
        let locals: Vec<Local> = debug
            .symbols
            .find_locals(debug.map.code_address(u64::from(pc)))
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
            .collect();
        Ok(locals)
    }

    /// The stack pointer's id for `get_register`.
    fn stack_register(&self) -> u8 {
        match self.arch {
            MachineFamily::CortexM => 13,
            MachineFamily::RiscV => 2,
            // a1 of the current register window.
            MachineFamily::Xtensa => 1,
            // SP, after r0..r31.
            MachineFamily::Avr => 32,
        }
    }

    /// The pc and stack pointer of the core being debugged.
    fn focused_pc_sp(&self, sp_id: u8) -> Result<(u32, u32), JsValue> {
        let cpu = focused_cpu(self.machine_or_err()?);
        Ok((cpu.get_pc(), cpu.get_register(sp_id)))
    }

    /// The length of the Xtensa `CALLn`/`CALLXn` at `pc`, or `None` for any
    /// other instruction. Both are three bytes: `CALLn` is `op0 == 5`,
    /// `CALLXn` is `op0 == op1 == op2 == r == 0` with `m == 3`.
    fn xtensa_call_at(&self, pc: u32) -> Option<u32> {
        let bus = &self.machine.as_ref()?.bus;
        let byte = |i: u32| bus.read_u8(u64::from(pc.wrapping_add(i))).ok();
        let insn = u32::from(byte(0)?) | u32::from(byte(1)?) << 8 | u32::from(byte(2)?) << 16;
        (insn & 0xF == 5 || insn & 0xFF_F0CF == 0xC0).then_some(3)
    }

    /// Xtensa `PS.EXCM` of the focused core: set on taking any exception or
    /// interrupt, so its rising edge is the entry to a handler.
    fn xtensa_excm(&self) -> bool {
        let Some(machine) = self.machine.as_ref() else {
            return false;
        };
        match focused_cpu(machine).snapshot() {
            CpuSnapshot::XtensaLx7(s) => s.ps & (1 << 4) != 0,
            _ => false,
        }
    }

    /// Runs the machine at batch speed until the focused core is back at one
    /// of `pending.targets` with its stack no deeper than `pending.sp` (a call
    /// or exception handler has returned), a breakpoint stops it, `budget`
    /// cycles are spent, or the firmware halts. Returns the outcome and the
    /// cycles run.
    ///
    /// The targets are engine breakpoints for the duration, so the run is the
    /// engine's own (`step_batch`, journaled) and keeps idle fast-forward and
    /// batching; single-stepping a `delay(1000)` would take a billion steps.
    fn run_through(
        &mut self,
        pending: &Pending,
        sp_id: u8,
        budget: u64,
        breakpoints: &[u32],
    ) -> Result<(Through, u64), JsValue> {
        let machine = self.machine_or_err()?;
        let focus = machine.breakpoint_core;
        let mut saved: Vec<u32> = machine.breakpoints.iter().copied().collect();
        saved.sort_unstable();
        let mut user = saved.clone();
        user.extend(breakpoints.iter().map(|a| a & !1));
        user.sort_unstable();
        user.dedup();
        let mut temporary = user.clone();
        temporary.extend(pending.targets.iter().map(|a| a & !1));
        temporary.sort_unstable();
        temporary.dedup();
        self.set_breakpoints(temporary)?;

        let mut spent = 0u64;
        let outcome = loop {
            if spent >= budget {
                break Through::Cap;
            }
            let chunk = (budget - spent).min(1 << 22) as u32;
            let ran = self.step_batch(chunk)?;
            spent += u64::from(ran);
            let machine = self.machine_or_err()?;
            let hit = match machine.breakpoint_core {
                1 => machine.last_breakpoint_secondary,
                _ => machine.last_breakpoint,
            };
            let Some(at) = hit else {
                if ran == 0 {
                    break Through::Halted;
                }
                continue;
            };
            let core = machine.breakpoint_core;
            let (pc, sp) = self.focused_pc_sp(sp_id)?;
            if core == focus && pending.targets.iter().any(|t| t & !1 == at) && sp >= pending.sp {
                break Through::Returned;
            }
            if user.binary_search(&at).is_ok() {
                break Through::Breakpoint(pc);
            }
            // A return address reached deeper in a recursion, or by the other
            // core running the same code: not the frame being stepped.
            if core != focus {
                self.set_debug_core(focus)?;
            }
        };
        self.set_breakpoints(saved)?;
        Ok((outcome, spent))
    }

    /// Steps until the source line changes; see
    /// [`WasmSimulator::step_source_line`]. Returns `(reason, pc, instructions)`.
    ///
    /// Steps the core a debugger follows ([`focused_cpu`]); on a dual-core
    /// chip the other core runs alongside, one instruction for each.
    ///
    /// Step-over spots a call as it happens and runs it at batch speed
    /// ([`Self::run_through`]) to the instruction after it:
    ///
    /// - Xtensa (windowed ABI): the instruction about to run is a `CALLn` or
    ///   `CALLXn`, so the call comes back to the next instruction with `a1`
    ///   (the caller's stack pointer) as it is now. `RETW` restores the
    ///   caller's window, so `a1` is the same value on return even when the
    ///   window was spilled to the stack and reloaded on the way.
    /// - AVR: the stack pointer dropped by a pushed return address (2 or 3
    ///   bytes) and the pc did not move on to the next instruction. The call
    ///   returns to the instruction after a 2- or 4-byte call (or, for an
    ///   interrupt, to where it was) with the stack pointer as it was.
    /// - Cortex-M and RISC-V: the pc arrived at the first instruction of a
    ///   different function (`bl`/`blx`/`jal`/`jalr`, or a tail branch), and
    ///   the return-address register holds where it comes back to, with the
    ///   stack no deeper than at the callee's first instruction. Outside the
    ///   function with the stack deeper than the frame's (an exception
    ///   handler on Cortex-M) runs on one instruction at a time. Leaving the
    ///   function with the stack no deeper than the frame's is a return: the
    ///   caller becomes the frame. Recursion into the stepped function itself
    ///   is not detected as a call.
    ///
    /// On Xtensa an interrupt or exception taken during either kind of step
    /// (`PS.EXCM` rising; a window overflow or underflow is one too) is run
    /// through the same way, back to the instruction it interrupted.
    ///
    /// The Cortex-M/RISC-V call's return is judged against the stack pointer
    /// at the call, not the one recorded when the step started. A step that
    /// starts on a function's first instruction (where a breakpoint on its
    /// opening line stops) records the stack before the prologue pushes; the
    /// frame's own pushes then sit below that value for the whole body, and a
    /// return compared against it would never be seen. The stack pointer at a
    /// callee's first instruction is the caller's at the call, whatever the
    /// caller's prologue has done, and it is that value again on return.
    pub(crate) fn step_source_line_s(
        &mut self,
        over: bool,
        max_instructions: u32,
        breakpoints: &[u32],
    ) -> Result<(&'static str, u32, u32), JsValue> {
        let debug = self.source_debug_s().map_err(js)?;
        let arch = self.arch;
        let sp_id = self.stack_register();
        // Cortex-M and RISC-V find a call by its return-address register.
        let ra_id = match arch {
            MachineFamily::CortexM => Some(14),
            MachineFamily::RiscV => Some(1),
            _ => None,
        };
        let function_at = |pc: u32| Self::function_range(&debug, pc);
        let run_budget = u64::from(max_instructions).saturating_mul(RUN_THROUGH_CYCLES_PER_STEP);

        let (start_pc, start_sp) = self.focused_pc_sp(sp_id)?;
        let start_line = debug.map.line_key(u64::from(start_pc));
        let mut frame_sp = start_sp;
        let mut frame_fn = function_at(start_pc);

        let mut stepped = 0u32;
        let mut ran_through = 0u64;
        let total =
            |stepped: u32, ran: u64| (u64::from(stepped) + ran).min(u64::from(u32::MAX)) as u32;
        while stepped < max_instructions {
            let (before_pc, before_sp) = self.focused_pc_sp(sp_id)?;
            let xtensa = arch == MachineFamily::Xtensa;
            let call_return = if over && xtensa {
                self.xtensa_call_at(before_pc)
                    .map(|len| before_pc.wrapping_add(len))
            } else {
                None
            };
            let excm_before = xtensa && self.xtensa_excm();

            self.record(Op::StepSingle);
            let report = self
                .advance_machine(AdvanceRequest::single())
                .map_err(crate::cosim::AdvanceFailure::into_js)?;
            stepped += 1;
            let (pc, sp) = self.focused_pc_sp(sp_id)?;
            if matches!(
                report.stop,
                AdvanceStop::NoProgress | AdvanceStop::FirmwareExit { .. }
            ) {
                return Ok(("halted", pc, total(stepped, ran_through)));
            }
            if breakpoints.contains(&pc) || self.machine_or_err()?.breakpoints.contains(&(pc & !1))
            {
                return Ok(("breakpoint", pc, total(stepped, ran_through)));
            }

            let mut pending: Option<Pending> = None;
            if xtensa && !excm_before && self.xtensa_excm() {
                // Back to the interrupted instruction, or the one after it
                // when the exception came after it retired.
                let byte0 = self
                    .machine_or_err()?
                    .bus
                    .read_u8(u64::from(before_pc))
                    .unwrap_or(0);
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
            } else if over && arch == MachineFamily::Avr {
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
                    let callee = function_at(pc);
                    // A Cortex-M exception entry also lands on a function
                    // start, with EXC_RETURN (0xFFFF_FFxx) in LR rather than
                    // an address: the stack rule below covers it.
                    let ra = focused_cpu(self.machine_or_err()?).get_register(ra_id) & !1;
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
                let (outcome, ran) = self.run_through(&pending, sp_id, budget, breakpoints)?;
                ran_through += ran;
                let pc = self.focused_pc_sp(sp_id)?.0;
                match outcome {
                    Through::Returned => {}
                    Through::Breakpoint(at) => {
                        return Ok(("breakpoint", at, total(stepped, ran_through)))
                    }
                    Through::Cap => return Ok(("cap", pc, total(stepped, ran_through))),
                    Through::Halted => return Ok(("halted", pc, total(stepped, ran_through))),
                }
            }

            let pc = self.focused_pc_sp(sp_id)?.0;
            if debug.map.is_statement(u64::from(pc))
                && debug.map.line_key(u64::from(pc)) != start_line
            {
                return Ok(("line_changed", pc, total(stepped, ran_through)));
            }
        }
        let pc = self.focused_pc_sp(sp_id)?.0;
        Ok(("cap", pc, total(stepped, ran_through)))
    }

    /// Point the debugger at core `core` (0, or 1 on a dual-core chip).
    /// Journaled.
    pub(crate) fn set_debug_core(&mut self, core: u8) -> Result<(), JsValue> {
        self.record(Op::SetDebugCore(core));
        let machine = self.machine_mut_or_err()?;
        if core == 0 || machine.cpu_secondary.is_some() {
            machine.breakpoint_core = core;
        }
        Ok(())
    }
}

#[wasm_bindgen]
impl WasmSimulator {
    /// The source position of `pc`, from the firmware's DWARF:
    /// `{ file, line, column: number | null, function: string | null }`, or
    /// `null` when `pc` has no line information. `file` is the full path DWARF
    /// records. Throws when the firmware ELF is missing or has no line info.
    #[wasm_bindgen]
    pub fn source_location(&self, pc: u32) -> Result<JsValue, JsValue> {
        let debug = self.source_debug_s().map_err(js)?;
        to_js(&Self::location_of(&debug, pc))
    }

    /// Where `line` of `file` starts: `{ pc, pcs: number[], file, line }`, or
    /// `null` when no DWARF file matches. `file` is matched by trailing path
    /// components, so an editor path (`src/main.rs`,
    /// `my-project/src/main.rs`) finds the DWARF absolute path; the most
    /// components shared wins. A line without code snaps to the next line
    /// that has some (else the previous one); the returned `line` says which.
    /// `pc` is the lowest statement address on that line, `pcs` all of them
    /// (set breakpoints on all to catch inlined copies).
    #[wasm_bindgen]
    pub fn source_line_to_pc(&self, file: &str, line: u32) -> Result<JsValue, JsValue> {
        let debug = self.source_debug_s().map_err(js)?;
        let hit = debug.map.line_to_pc(file, line).map(|h| LineHit {
            pc: h.pc as u32,
            pcs: h.pcs.iter().map(|p| *p as u32).collect(),
            file: h.file,
            line: h.line,
        });
        to_js(&hit)
    }

    /// The lines of `file` (matched as in `source_line_to_pc`) that carry
    /// code, ascending: where a breakpoint can sit. Empty for an unknown file.
    #[wasm_bindgen]
    pub fn source_lines(&self, file: &str) -> Result<Vec<u32>, JsValue> {
        let debug = self.source_debug_s().map_err(js)?;
        Ok(debug.map.lines(file))
    }

    /// Variables and parameters of the function containing `pc`:
    /// `[{ name, kind, register?, offset?, address?, expr?, value }]`.
    ///
    /// Only single-expression DWARF locations are decoded. `kind: "register"`
    /// carries the current register value in `value`; `"frame_offset"` (an
    /// offset from the DWARF frame base) and the rest are reported without a
    /// value. Location lists — what optimised builds emit for most locals —
    /// are not decoded, so such variables are absent.
    #[wasm_bindgen]
    pub fn locals(&self, pc: u32) -> Result<JsValue, JsValue> {
        to_js(&self.locals_s(pc)?)
    }

    /// Make `step_batch` stop before executing any of `addresses` (Thumb bit
    /// ignored), replacing the previous set. The check runs inside the
    /// engine's own advance loop, so a run with breakpoints keeps its batch
    /// speed instead of being driven one instruction at a time from JS. A
    /// batch that stops early returns fewer cycles than asked, with the pc on
    /// the breakpoint; the next batch runs past it. `step_source_line`
    /// honours the same set. Journaled, so a snapshot replay stops at the
    /// same places.
    #[wasm_bindgen]
    pub fn set_breakpoints(&mut self, addresses: Vec<u32>) -> Result<(), JsValue> {
        self.record(Op::SetBreakpoints(addresses.clone()));
        let machine = self.machine_mut_or_err()?;
        machine.breakpoints = addresses.into_iter().map(|a| a & !1).collect();
        machine.last_breakpoint = None;
        machine.last_breakpoint_secondary = None;
        Ok(())
    }

    /// Run until the source line changes:
    /// `{ reason, pc, instructions, location }`.
    ///
    /// Steps single instructions and stops at the first statement boundary
    /// on a different line than the one it started on (`"line_changed"`).
    /// With `over`, calls (and exception handlers) entered on the way are run
    /// through to where they return, at batch speed (Cortex-M, RISC-V,
    /// Xtensa, AVR). Stops early on a pc in
    /// `breakpoints` or in the machine's own breakpoint set
    /// (`"breakpoint"`), when the firmware halts (`"halted"`), or after
    /// `max_instructions` single steps or 256 cycles per such step run
    /// through calls (`"cap"`). `instructions` counts both. Journaled.
    /// `location` is as `source_location` returns it.
    #[wasm_bindgen]
    pub fn step_source_line(
        &mut self,
        over: bool,
        max_instructions: u32,
        breakpoints: Vec<u32>,
    ) -> Result<JsValue, JsValue> {
        let (reason, pc, instructions) =
            self.step_source_line_s(over, max_instructions, &breakpoints)?;
        let debug = self.source_debug_s().map_err(js)?;
        to_js(&StepOutcome {
            reason,
            pc,
            instructions,
            location: Self::location_of(&debug, pc),
        })
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    //! Driven natively on committed fixtures with DWARF. Addresses come from
    //! `objdump -dl` of each fixture.

    use super::*;
    use crate::lab_tools::CtorInputs;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn build(system: &str, firmware: &str) -> WasmSimulator {
        let sys = root().join(system);
        let system_yaml = std::fs::read_to_string(&sys).unwrap();
        let manifest: labwired_config::SystemManifest = serde_yaml::from_str(&system_yaml).unwrap();
        let chip_yaml =
            std::fs::read_to_string(sys.parent().unwrap().join(&manifest.chip)).unwrap();
        WasmSimulator::new_from_config_parts(Rc::new(CtorInputs {
            system_yaml,
            chip_yaml,
            firmware: std::fs::read(root().join(firmware)).unwrap(),
            blobs: HashMap::new(),
        }))
        .unwrap_or_else(|_| panic!("build {system}"))
    }

    /// nRF54L15 smart-ring probe: Cortex-M33, gcc -Os -g3.
    fn ring() -> WasmSimulator {
        build(
            "examples/nrf54l15-smart-ring/system.yaml",
            "tests/fixtures/nrf54l15-smart-ring.elf",
        )
    }

    fn run_to(sim: &mut WasmSimulator, pc: u32) {
        for _ in 0..200_000 {
            if sim.get_pc().unwrap() == pc {
                return;
            }
            sim.step_single().unwrap();
        }
        panic!("never reached {pc:#x}");
    }

    fn line_of(sim: &WasmSimulator, pc: u32) -> u32 {
        let debug = sim.source_debug_s().unwrap();
        WasmSimulator::location_of(&debug, pc).unwrap().line
    }

    fn line_pc(sim: &WasmSimulator, file: &str, line: u32) -> u32 {
        let debug = sim.source_debug_s().unwrap();
        debug.map.line_to_pc(file, line).unwrap().pc as u32
    }

    #[test]
    fn location_and_line_lookup_round_trip() {
        let sim = ring();
        let debug = sim.source_debug_s().unwrap();
        let at = WasmSimulator::location_of(&debug, 0x2ca).unwrap();
        assert!(at.file.ends_with("nrf54l15-smart-ring/src/main.c"));
        assert_eq!(at.line, 223);
        assert_eq!(at.function.as_deref(), Some("main"));
        assert_eq!(line_pc(&sim, "ring/src/main.c", 223), 0x2ca);
        assert_eq!(line_of(&sim, line_pc(&sim, "src/main.c", 230)), 230);
        // Cached: the second call reuses the parse.
        assert!(Rc::ptr_eq(&debug, &sim.source_debug_s().unwrap()));
    }

    #[test]
    fn step_into_advances_exactly_one_line() {
        let mut sim = ring();
        let target = line_pc(&sim, "src/main.c", 220);
        run_to(&mut sim, target);
        let (reason, pc, n) = sim.step_source_line_s(false, 10_000, &[]).unwrap();
        // 220 is `ldr r0, =msg; bl uart_puts`: into lands in uart_puts.
        assert_eq!(reason, "line_changed");
        assert_eq!(n, 2, "ldr + bl");
        let debug = sim.source_debug_s().unwrap();
        let loc = WasmSimulator::location_of(&debug, pc).unwrap();
        // uart_puts' first instruction is str_len, inlined (main.c:45).
        assert_eq!(pc, 0x160, "uart_puts entry");
        assert_eq!(loc.function.as_deref(), Some("str_len"));
        assert_eq!(loc.line, 45);
    }

    #[test]
    fn step_over_runs_through_calls() {
        let mut sim = ring();
        let target = line_pc(&sim, "src/main.c", 220);
        run_to(&mut sim, target);
        let (reason, pc, n) = sim.step_source_line_s(true, 1_000_000, &[]).unwrap();
        assert_eq!(reason, "line_changed");
        assert_eq!((pc, line_of(&sim, pc)), (0x2ca, 223));
        assert!(n > 100, "uart_puts ran inside the step ({n} instructions)");
        // The step, its run through uart_puts included, is journaled: a
        // snapshot replay lands on the same instruction and cycle.
        let cycles = sim.machine.as_ref().unwrap().total_cycles;
        let saved = sim.snapshot_save(None).unwrap();
        let id = serde_json::from_str::<serde_json::Value>(&saved).unwrap()["id"]
            .as_u64()
            .unwrap() as u32;
        sim.step_batch(1000).unwrap();
        sim.snapshot_restore(id).unwrap();
        assert_eq!(sim.get_pc().unwrap(), 0x2ca);
        assert_eq!(sim.machine.as_ref().unwrap().total_cycles, cycles);

        // 223 calls probe8 (a full I2C transaction and a UART line): one step.
        let (reason, pc, _) = sim.step_source_line_s(true, 1_000_000, &[]).unwrap();
        assert_eq!(reason, "line_changed");
        assert_eq!((pc, line_of(&sim, pc)), (0x2d6, 224));
        assert!(String::from_utf8_lossy(&sim.drain_uart_output()).contains("BMI270"));
    }

    #[test]
    fn step_stops_at_breakpoint_and_cap() {
        let mut sim = ring();
        let target = line_pc(&sim, "src/main.c", 223);
        run_to(&mut sim, target);
        let probe8 = line_pc(&sim, "src/main.c", 177);
        let (reason, pc, _) = sim.step_source_line_s(true, 1_000_000, &[probe8]).unwrap();
        assert_eq!((reason, pc), ("breakpoint", probe8));

        let (reason, _, n) = sim.step_source_line_s(true, 3, &[]).unwrap();
        assert_eq!((reason, n), ("cap", 3));
    }

    /// Stopped on probe8's first instruction, as a breakpoint on its first
    /// line (177) stops: 0x1d8, before `stmdb sp!, {r0, r1, r4-r8, lr}`.
    fn ring_at_probe8_entry() -> WasmSimulator {
        let mut sim = ring();
        let caller = line_pc(&sim, "src/main.c", 223);
        run_to(&mut sim, caller);
        let entry = line_pc(&sim, "src/main.c", 177);
        assert_eq!(entry, 0x1d8, "probe8 entry");
        run_to(&mut sim, entry);
        let debug = sim.source_debug_s().unwrap();
        let at = WasmSimulator::location_of(&debug, entry).unwrap();
        // The entry address carries 176 (`{`), 177 and 176 again; the
        // statement wins.
        assert_eq!((at.line, at.function.as_deref()), (177, Some("probe8")));
        sim
    }

    #[test]
    fn step_over_from_function_entry_stays_in_the_function() {
        let mut sim = ring_at_probe8_entry();
        // 177 calls twim_read_reg; its return is seen although the step
        // started before the prologue moved the stack. The next statement
        // boundary is 0x1f2 (179/180, interleaved by -Os).
        let (reason, pc, n) = sim.step_source_line_s(true, 1_000_000, &[]).unwrap();
        assert_eq!(reason, "line_changed");
        let debug = sim.source_debug_s().unwrap();
        let at = WasmSimulator::location_of(&debug, pc).unwrap();
        assert_eq!(
            (pc, at.line, at.function.as_deref()),
            (0x1f2, 180, Some("probe8"))
        );
        assert!(
            n > 10,
            "twim_read_reg ran inside the step ({n} instructions)"
        );
    }

    #[test]
    fn step_out_from_function_entry_returns_to_caller() {
        let mut sim = ring_at_probe8_entry();
        // Step out as the playground does it: step over until the function
        // changes.
        let debug = sim.source_debug_s().unwrap();
        let mut total = 0;
        let (reason, pc) = loop {
            let (reason, pc, n) = sim.step_source_line_s(true, 1_000_000, &[]).unwrap();
            total += n;
            let function = WasmSimulator::location_of(&debug, pc).and_then(|l| l.function);
            if reason != "line_changed" || function.as_deref() != Some("probe8") {
                break (reason, pc);
            }
            assert!(total < 1_000_000, "step out ran away");
        };
        assert_eq!(reason, "line_changed");
        let at = WasmSimulator::location_of(&debug, pc).unwrap();
        assert_eq!(
            (pc, at.line, at.function.as_deref()),
            (0x2d6, 224, Some("main"))
        );
    }

    #[test]
    fn locals_of_probe8_report_frame_local() {
        let mut sim = ring();
        let target = line_pc(&sim, "src/main.c", 223);
        run_to(&mut sim, target);
        let entry = line_pc(&sim, "src/main.c", 177);
        run_to(&mut sim, entry);
        let locals = sim.locals_s(entry).unwrap();
        // `p` has its address taken (`app_str(&p, ...)`), so it lives in the
        // frame; the -Os parameters live in location lists and are absent.
        let p = locals.iter().find(|l| l.name == "p").expect("local p");
        assert_eq!(p.kind, "frame_offset");
        assert!(p.offset.is_some() && p.value.is_none());
    }

    #[test]
    fn step_batch_stops_at_engine_breakpoints() {
        let mut sim = ring();
        let target = line_pc(&sim, "src/main.c", 180);
        sim.set_breakpoints(vec![target]).unwrap();
        let mut ran = 0u64;
        while sim.get_pc().unwrap() != target {
            let n = sim.step_batch(100_000).unwrap();
            ran += u64::from(n);
            assert!(ran < 5_000_000, "never stopped at {target:#x}");
        }
        // Stopped before executing it; the next batch runs past it (probe8
        // runs again for the next sensor, so it may stop there once more).
        let n = sim.step_batch(100_000).unwrap();
        assert!(n > 1, "resumed past the breakpoint ({n} cycles)");

        // A snapshot replay reproduces the same stops.
        let saved = sim.snapshot_save(None).unwrap();
        let id = serde_json::from_str::<serde_json::Value>(&saved).unwrap()["id"]
            .as_u64()
            .unwrap() as u32;
        let pc = sim.get_pc().unwrap();
        sim.step_batch(1000).unwrap();
        sim.snapshot_restore(id).unwrap();
        assert_eq!(sim.get_pc().unwrap(), pc);

        sim.set_breakpoints(vec![]).unwrap();
        assert!(sim.step_batch(100_000).unwrap() >= 100_000);
    }

    /// RISC-V (riscv-rt, Rust release build with DWARF).
    #[test]
    fn riscv_step_over_and_lookup() {
        let mut sim = build(
            "configs/systems/ci-fixture-riscv-uart1.yaml",
            "tests/fixtures/riscv-ci-fixture.elf",
        );
        let main = line_pc(&sim, "riscv-ci-fixture/src/main.rs", 12);
        assert_eq!(main, 0x8000_02ec);
        run_to(&mut sim, main);
        // main.rs:12 -> the inlined write_volatile (core ptr/mod.rs).
        let (reason, pc, _) = sim.step_source_line_s(true, 10_000, &[]).unwrap();
        assert_eq!(reason, "line_changed");
        let debug = sim.source_debug_s().unwrap();
        assert!(WasmSimulator::location_of(&debug, pc)
            .unwrap()
            .file
            .ends_with("ptr/mod.rs"));
        // All six inlined writes share one line; the next step lands on `loop {}`.
        let (reason, pc, _) = sim.step_source_line_s(true, 10_000, &[]).unwrap();
        assert_eq!(reason, "line_changed");
        assert_eq!((pc, line_of(&sim, pc)), (0x8000_0320, 24));
        // `loop {}` never leaves its line.
        let (reason, _, _) = sim.step_source_line_s(true, 50, &[]).unwrap();
        assert_eq!(reason, "cap");
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod esp32_tests {
    //! The ESP32 family on what a hosted Build produces: an Arduino sketch
    //! (`tests/fixtures/source-debug/main.ino`) compiled by the LabWired
    //! compile service for esp32dev, esp32-s3-devkitc-1 and
    //! esp32-c3-supermini, debug sections zlib-compressed. Addresses come from
    //! `objdump -dl` of each ELF.

    use super::*;
    use crate::lab_tools::CtorInputs;
    use std::collections::HashMap;
    use std::path::PathBuf;

    const SKETCH: &str = "src/main.ino";

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn read(rel: &str) -> Vec<u8> {
        std::fs::read(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
    }

    fn text(rel: &str) -> String {
        String::from_utf8(read(rel)).unwrap()
    }

    /// Classic ESP32 as the playground boots it: the ELF fast-booted, the
    /// Arduino quirks installed from it, idle fast-forward on. `loop()` runs
    /// on core 1.
    fn esp32() -> WasmSimulator {
        let elf = read("tests/fixtures/source-debug/esp32-arduino.elf");
        let mut sim = WasmSimulator::new_from_config_parts(Rc::new(CtorInputs {
            system_yaml: text("configs/systems/esp32-wroom-32.yaml"),
            chip_yaml: text("configs/chips/esp32.yaml"),
            firmware: elf.clone(),
            blobs: HashMap::new(),
        }))
        .unwrap_or_else(|_| panic!("build esp32"));
        sim.install_arduino_esp32_quirks(&elf).unwrap();
        sim.set_idle_fast_forward_enabled(true);
        let tick = sim.recommended_tick_interval();
        sim.set_peripheral_tick_interval(tick);
        sim
    }

    fn line_pcs(sim: &WasmSimulator, line: u32) -> Vec<u32> {
        let debug = sim.source_debug_s().unwrap();
        let hit = debug.map.line_to_pc(SKETCH, line).unwrap();
        assert_eq!(hit.line, line);
        hit.pcs.iter().map(|p| *p as u32).collect()
    }

    fn at(sim: &WasmSimulator, pc: u32) -> (u32, String) {
        let debug = sim.source_debug_s().unwrap();
        let loc = WasmSimulator::location_of(&debug, pc).unwrap();
        assert!(loc.file.ends_with(SKETCH), "{pc:#x} is in {}", loc.file);
        (loc.line, loc.function.unwrap_or_default())
    }

    /// Run the way the playground runs a lab with breakpoints until one stops
    /// it; returns the stopped pc (`get_pc`, the core that stopped).
    fn run_to_breakpoint(sim: &mut WasmSimulator, max_cycles: u64) -> u32 {
        let mut ran = 0u64;
        loop {
            let n = sim.step_batch(200_000).unwrap();
            ran += u64::from(n);
            let m = sim.machine.as_ref().unwrap();
            let stopped = match m.breakpoint_core {
                1 => m.last_breakpoint_secondary.is_some(),
                _ => m.last_breakpoint.is_some(),
            };
            if stopped {
                return sim.get_pc().unwrap();
            }
            assert!(ran < max_cycles, "no breakpoint within {ran} cycles");
        }
    }

    fn step(sim: &mut WasmSimulator, over: bool) -> (&'static str, u32) {
        let (reason, pc, _) = sim.step_source_line_s(over, 2_000_000, &[]).unwrap();
        (reason, pc)
    }

    fn step_out(sim: &mut WasmSimulator, function: &str) -> (&'static str, u32) {
        loop {
            let (reason, pc) = step(sim, true);
            if reason != "line_changed" || at(sim, pc).1 != function {
                return (reason, pc);
            }
        }
    }

    #[test]
    fn esp32_breakpoint_in_loop_stops_core_1() {
        let mut sim = esp32();
        assert_eq!(line_pcs(&sim, 27), vec![0x400d_147a]);
        sim.set_breakpoints(line_pcs(&sim, 27)).unwrap();
        let pc = run_to_breakpoint(&mut sim, 200_000_000);
        assert_eq!(pc, 0x400d_147a);
        // Arduino-ESP32 pins loopTask to core 1: the second core stopped, and
        // the debugger follows it.
        assert_eq!(sim.machine.as_ref().unwrap().breakpoint_core, 1);
        assert_eq!(at(&sim, pc), (27, "loop()".into()));
        // Variables are read from that core too: a1 is its stack pointer.
        let sp = sim.get_register(1).unwrap();
        assert!((0x3ff0_0000..0x4000_0000).contains(&sp), "a1 {sp:#x}");

        // Continue: the next loop() iteration stops on the same line, having
        // bumped `ticks`.
        let ticks = |sim: &WasmSimulator| {
            u32::from_le_bytes(sim.read_memory(0x3ffc_1c00, 4).unwrap().try_into().unwrap())
        };
        let before = ticks(&sim);
        let pc = run_to_breakpoint(&mut sim, 200_000_000);
        assert_eq!(pc, 0x400d_147a);
        assert_ne!(ticks(&sim), before);
    }

    #[test]
    fn esp32_step_over_into_and_out() {
        let mut sim = esp32();
        sim.set_breakpoints(line_pcs(&sim, 27)).unwrap();
        run_to_breakpoint(&mut sim, 200_000_000);

        // 27 `int n = bump(ticks);` -> 28: bump and twice run inside the step.
        assert_eq!(step(&mut sim, true), ("line_changed", 0x400d_1485));
        assert_eq!(at(&sim, 0x400d_1485), (28, "loop()".into()));
        // 28 Serial.println, 29 digitalWrite, 30 delay(100): one step each.
        let (reason, pc) = step(&mut sim, true);
        assert_eq!((reason, at(&sim, pc).0), ("line_changed", 29));
        let (reason, pc) = step(&mut sim, true);
        assert_eq!((reason, at(&sim, pc).0), ("line_changed", 30));
        // 31 (`}`) carries no statement, so stepping over the delay ends in
        // the caller, Arduino's loopTask.
        let (reason, pc, _) = sim.step_source_line_s(true, 2_000_000, &[]).unwrap();
        assert_eq!(reason, "line_changed");
        let debug = sim.source_debug_s().unwrap();
        let loc = WasmSimulator::location_of(&debug, pc).unwrap();
        assert!(loc.file.ends_with("cores/esp32/main.cpp"), "{}", loc.file);
        assert_eq!(loc.function.as_deref(), Some("loopTask(void*)"));

        // Next iteration: into bump, into twice, then out twice.
        run_to_breakpoint(&mut sim, 200_000_000);
        let (reason, pc) = step(&mut sim, false);
        assert_eq!(reason, "line_changed");
        assert_eq!(pc, 0x400d_1428, "bump's entry");
        assert_eq!(at(&sim, pc).1, "bump(int)");
        let (reason, pc) = step(&mut sim, false);
        assert_eq!(
            (reason, at(&sim, pc)),
            ("line_changed", (15, "bump(int)".into()))
        );
        let (reason, pc) = step(&mut sim, false);
        assert_eq!((reason, pc), ("line_changed", 0x400d_1410), "twice's entry");

        // Step out as the playground does it: over until the function changes.
        let (reason, pc) = step_out(&mut sim, "twice(int)");
        assert_eq!(
            (reason, at(&sim, pc).1.as_str()),
            ("line_changed", "bump(int)")
        );
        let (reason, pc) = step_out(&mut sim, "bump(int)");
        assert_eq!(
            (reason, at(&sim, pc)),
            ("line_changed", (28, "loop()".into()))
        );
    }

    /// The flow a user runs, by line: stop in loop(), step over the call,
    /// step into bump and twice, step back out, continue to the next stop.
    fn breakpoint_step_flow(sim: &mut WasmSimulator, core: u8, max_cycles: u64) {
        sim.set_breakpoints(line_pcs(sim, 27)).unwrap();
        let pc = run_to_breakpoint(sim, max_cycles);
        assert_eq!(sim.machine.as_ref().unwrap().breakpoint_core, core);
        assert_eq!(at(sim, pc), (27, "loop()".into()));

        let (reason, pc) = step(sim, true);
        assert_eq!(
            (reason, at(sim, pc)),
            ("line_changed", (28, "loop()".into()))
        );

        let pc = run_to_breakpoint(sim, max_cycles);
        assert_eq!(at(sim, pc).0, 27);
        let (reason, pc) = step(sim, false);
        assert_eq!(
            (reason, at(sim, pc).1.as_str()),
            ("line_changed", "bump(int)")
        );
        let (reason, pc) = loop {
            let (reason, pc) = step(sim, false);
            if reason != "line_changed" || at(sim, pc).1 != "bump(int)" {
                break (reason, pc);
            }
        };
        assert_eq!(
            (reason, at(sim, pc).1.as_str()),
            ("line_changed", "twice(int)")
        );
        let (reason, pc) = step_out(sim, "twice(int)");
        assert_eq!(
            (reason, at(sim, pc).1.as_str()),
            ("line_changed", "bump(int)")
        );
        let (reason, pc) = step_out(sim, "bump(int)");
        assert_eq!(
            (reason, at(sim, pc)),
            ("line_changed", (28, "loop()".into()))
        );
        let (v_reg, v_value) = {
            let locals = sim.locals_s(pc).unwrap();
            let n = locals
                .iter()
                .find(|l| l.name == "n")
                .map(|l| (l.kind, l.value));
            (n.map(|n| n.0), n.and_then(|n| n.1))
        };
        // `n`, bump's result, is live in a register on line 28.
        assert_eq!(v_reg, Some("register"), "n on line 28");
        assert!(v_value.is_some());

        let pc = run_to_breakpoint(sim, max_cycles);
        assert_eq!(at(sim, pc).0, 27);
    }

    fn blobs(pairs: &[(&str, &str)]) -> HashMap<String, Vec<u8>> {
        pairs
            .iter()
            .map(|(k, rel)| (k.to_string(), read(rel)))
            .collect()
    }

    /// ESP32-S3 as the playground boots a hosted build: the merged flash
    /// image through the real mask ROM, the ELF alongside it for debugging.
    #[test]
    fn esp32s3_flash_boot_breakpoint_and_steps() {
        let mut sim = WasmSimulator::new_from_config_parts(Rc::new(CtorInputs {
            system_yaml: text("configs/systems/esp32s3-zero.yaml"),
            chip_yaml: text("configs/chips/esp32s3.yaml"),
            firmware: read("tests/fixtures/source-debug/esp32s3-arduino.elf"),
            blobs: blobs(&[
                ("esp32s3_irom", "crates/core/roms/esp32s3/esp32s3_rom.bin"),
                ("esp32s3_drom", "crates/core/roms/esp32s3/esp32s3_drom.bin"),
                (
                    "esp32s3_flash",
                    "tests/fixtures/source-debug/esp32s3-arduino-flash.bin",
                ),
            ]),
        }))
        .unwrap_or_else(|_| panic!("build esp32s3"));
        sim.set_idle_fast_forward_enabled(true);
        let tick = sim.recommended_tick_interval();
        sim.set_peripheral_tick_interval(tick);
        breakpoint_step_flow(&mut sim, 1, 2_000_000_000);
    }

    /// ESP32-C3 (RISC-V, one core) as the playground boots a hosted build.
    #[test]
    fn esp32c3_flash_boot_breakpoint_and_steps() {
        let mut sim = WasmSimulator::new_from_config_parts(Rc::new(CtorInputs {
            system_yaml: text("configs/systems/esp32c3-devkit.yaml"),
            chip_yaml: text("configs/chips/esp32c3.yaml"),
            firmware: read("tests/fixtures/source-debug/esp32c3-arduino.elf"),
            blobs: blobs(&[
                ("esp32c3_irom", "crates/core/roms/esp32c3/esp32c3_rom.bin"),
                ("esp32c3_drom", "crates/core/roms/esp32c3/esp32c3_drom.bin"),
                (
                    "esp32c3_flash",
                    "tests/fixtures/source-debug/esp32c3-arduino-flash.bin",
                ),
            ]),
        }))
        .unwrap_or_else(|_| panic!("build esp32c3"));
        sim.set_idle_fast_forward_enabled(true);
        let tick = sim.recommended_tick_interval();
        sim.set_peripheral_tick_interval(tick);
        breakpoint_step_flow(&mut sim, 0, 2_000_000_000);
    }

    #[test]
    fn esp32_flow_by_line() {
        let mut sim = esp32();
        breakpoint_step_flow(&mut sim, 1, 200_000_000);
    }

    /// Arduino Uno: the same sketch, built with `-g` kept through the LTO
    /// link (hosted builds drop it; see the fixture README). LTO inlines
    /// loop() into main(), so the calls left are the core's.
    #[test]
    fn avr_step_over_into_and_out() {
        let elf = read("tests/fixtures/source-debug/uno-arduino.elf");
        let mut sim = WasmSimulator::new_from_config_parts(Rc::new(CtorInputs {
            system_yaml: text("configs/systems/arduino-uno.yaml"),
            chip_yaml: text("configs/chips/atmega328p.yaml"),
            firmware: elf,
            blobs: HashMap::new(),
        }))
        .unwrap_or_else(|_| panic!("build uno"));
        let line = |sim: &WasmSimulator, pc: u32| {
            let debug = sim.source_debug_s().unwrap();
            let loc = WasmSimulator::location_of(&debug, pc).unwrap();
            (loc.file.rsplit('/').next().unwrap().to_string(), loc.line)
        };
        assert_eq!(line_pcs(&sim, 26), vec![0x692]);
        sim.set_breakpoints(vec![0x692]).unwrap();
        assert_eq!(run_to_breakpoint(&mut sim, 50_000_000), 0x692);

        // 26 `digitalWrite(LED, HIGH)`: a 4-byte CALL, run through.
        let (reason, pc, n) = sim.step_source_line_s(true, 2_000_000, &[]).unwrap();
        assert_eq!((reason, pc), ("line_changed", 0x698));
        assert!(n > 10, "digitalWrite ran inside the step ({n})");
        assert_eq!(line(&sim, pc), ("main.ino".into(), 27));

        // Into: the next iteration enters digitalWrite; out comes back to 27.
        assert_eq!(run_to_breakpoint(&mut sim, 50_000_000), 0x692);
        let (reason, pc) = step(&mut sim, false);
        assert_eq!(
            (reason, pc),
            ("line_changed", 0x10e),
            "digitalWrite's entry"
        );
        let (reason, pc) = loop {
            let (reason, pc) = step(&mut sim, true);
            if reason != "line_changed" || !(0x10e..0x19e).contains(&pc) {
                break (reason, pc);
            }
        };
        assert_eq!((reason, pc), ("line_changed", 0x698));

        // A breakpoint inside a call being stepped over stops the step there.
        let (reason, pc, _) = sim.step_source_line_s(true, 2_000_000, &[]).unwrap();
        assert_eq!(reason, "line_changed");
        assert_ne!(pc, 0x10e);
        let pc29 = line_pcs(&sim, 29)[0];
        sim.set_breakpoints(vec![0x692, pc29]).unwrap();
        assert_eq!(run_to_breakpoint(&mut sim, 50_000_000), pc29);
        let (reason, pc, _) = sim.step_source_line_s(true, 2_000_000, &[0x10e]).unwrap();
        assert_eq!((reason, pc), ("breakpoint", 0x10e));
    }

    #[test]
    fn esp32_locals_of_bump() {
        let mut sim = esp32();
        sim.set_breakpoints(line_pcs(&sim, 15)).unwrap();
        let pc = run_to_breakpoint(&mut sim, 200_000_000);
        // -Os puts both in location lists; at bump's first statement `v` is
        // still in a2 (the a10 the caller passed it in, after ENTRY's
        // rotation) and `t` has no value yet.
        let locals = sim.locals_s(pc).unwrap();
        let v = locals.iter().find(|l| l.name == "v").expect("parameter v");
        assert_eq!((v.kind, v.register), ("register", Some(2)));
        let ticks = sim.read_memory(0x3ffc_1c00, 4).unwrap();
        assert_eq!(v.value, Some(u32::from_le_bytes(ticks.try_into().unwrap())));
        let t = locals.iter().find(|l| l.name == "t").expect("local t");
        assert!(t.value.is_none());
    }
}
