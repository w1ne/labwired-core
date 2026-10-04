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
//! as a `StepSingle` so snapshots and replays stay exact.

use crate::lab_tools::Op;
use crate::WasmSimulator;
use labwired_core::system::arch_policy::MachineFamily;
use labwired_core::{AdvanceRequest, AdvanceStop};
use labwired_loader::source_map::SourceMap;
use labwired_loader::{DwarfLocation, SymbolProvider};
use serde::Serialize;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

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
        let pc = u64::from(pc & !1);
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
        let at_entry =
            Self::function_range(debug, pc).is_some_and(|r| r.start == u64::from(pc & !1));
        let pos = if at_entry {
            debug.map.location_at_entry(u64::from(pc))?
        } else {
            debug.map.location(u64::from(pc))?
        };
        let function = debug
            .symbols
            .lookup(u64::from(pc & !1))
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
        let reg_count: u16 = match self.arch {
            MachineFamily::CortexM => 16,
            MachineFamily::RiscV => 32,
            _ => 0,
        };
        let locals: Vec<Local> = debug
            .symbols
            .find_locals(u64::from(pc & !1))
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
                        out.value = (r < reg_count).then(|| machine.cpu.get_register(r as u8));
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

    /// `(stack pointer, return-address register)` ids for `get_register`,
    /// used by step-over.
    fn frame_register_ids(&self) -> Option<(u8, u8)> {
        match self.arch {
            MachineFamily::CortexM => Some((13, 14)),
            MachineFamily::RiscV => Some((2, 1)),
            _ => None,
        }
    }

    /// Steps until the source line changes; see
    /// [`WasmSimulator::step_source_line`]. Returns `(reason, pc, instructions)`.
    ///
    /// Step-over tracks the frame being stepped (its function's symbol range
    /// and its stack pointer). The pc arriving at the first instruction of a
    /// different function is a call (`bl`/`blx`/`jal`/`jalr`, or a tail
    /// branch): the return-address register then holds where the call comes
    /// back to, and stepping runs on until the pc is back there with the stack
    /// no deeper than it was at the callee's first instruction. Outside the
    /// function with the stack deeper than the frame's (an exception handler
    /// on Cortex-M) also runs on. Leaving the function with the stack no
    /// deeper than the frame's is a return: the caller becomes the frame.
    /// Recursion into the stepped function itself is not detected as a call.
    ///
    /// The call's return is judged against the stack pointer at the call, not
    /// the one recorded when the step started. A step that starts on a
    /// function's first instruction (where a breakpoint on its opening line
    /// stops) records the stack before the prologue pushes; the frame's own
    /// pushes then sit below that value for the whole body, and a return
    /// compared against it would never be seen. The stack pointer at a
    /// callee's first instruction is the caller's at the call, whatever the
    /// caller's prologue has done, and it is that value again on return.
    pub(crate) fn step_source_line_s(
        &mut self,
        over: bool,
        max_instructions: u32,
        breakpoints: &[u32],
    ) -> Result<(&'static str, u32, u32), JsValue> {
        let debug = self.source_debug_s().map_err(js)?;
        let regs = match (over, self.frame_register_ids()) {
            (true, None) => {
                return Err(js(
                    "step over a source line is supported on Cortex-M and RISC-V only; \
                     use step into",
                ))
            }
            (_, regs) => regs,
        };
        let function_at = |pc: u32| Self::function_range(&debug, pc);
        let reg = |sim: &Self, id: u8| sim.machine.as_ref().map_or(0, |m| m.cpu.get_register(id));

        let start_pc = self.machine_or_err()?.cpu.get_pc();
        let start_line = debug.map.line_key(u64::from(start_pc));
        let mut frame_sp = regs.map_or(0, |(sp, _)| reg(self, sp));
        let mut frame_fn = function_at(start_pc);
        // The call currently being stepped over: where it returns to, and the
        // stack pointer at its first instruction.
        let mut pending_return: Option<(u32, u32)> = None;

        let mut executed = 0u32;
        while executed < max_instructions {
            self.record(Op::StepSingle);
            let report = self
                .advance_machine(AdvanceRequest::single())
                .map_err(crate::cosim::AdvanceFailure::into_js)?;
            executed += 1;
            let pc = self.machine_or_err()?.cpu.get_pc();
            if matches!(
                report.stop,
                AdvanceStop::NoProgress | AdvanceStop::FirmwareExit { .. }
            ) {
                return Ok(("halted", pc, executed));
            }
            if breakpoints.contains(&pc) || self.machine_or_err()?.breakpoints.contains(&pc) {
                return Ok(("breakpoint", pc, executed));
            }
            if let (true, Some((sp_id, ra_id))) = (over, regs) {
                let sp = reg(self, sp_id);
                if let Some((ret, call_sp)) = pending_return {
                    if pc & !1 == ret && sp >= call_sp {
                        pending_return = None;
                    } else {
                        continue;
                    }
                }
                let here = u64::from(pc & !1);
                if !frame_fn.as_ref().is_some_and(|r| r.contains(&here)) {
                    let callee = function_at(pc);
                    // A Cortex-M exception entry also lands on a function
                    // start, with EXC_RETURN (0xFFFF_FFxx) in LR rather than
                    // an address: the stack rule below covers it.
                    let ra = reg(self, ra_id) & !1;
                    if callee.as_ref().is_some_and(|r| r.start == here) && ra < 0xF000_0000 {
                        pending_return = Some((ra, sp));
                        continue;
                    }
                    if sp < frame_sp {
                        continue;
                    }
                    // Equal counts as a return: a frame recorded at its
                    // function's entry, or one with no prologue, holds the
                    // caller's stack pointer.
                    if sp >= frame_sp {
                        frame_sp = sp;
                        frame_fn = callee;
                    }
                }
            }
            if debug.map.is_statement(u64::from(pc))
                && debug.map.line_key(u64::from(pc)) != start_line
            {
                return Ok(("line_changed", pc, executed));
            }
        }
        let pc = self.machine_or_err()?.cpu.get_pc();
        Ok(("cap", pc, executed))
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
        Ok(())
    }

    /// Run until the source line changes:
    /// `{ reason, pc, instructions, location }`.
    ///
    /// Steps single instructions and stops at the first statement boundary
    /// on a different line than the one it started on (`"line_changed"`).
    /// With `over`, calls (and exception handlers) entered on the way are run
    /// through: a pc outside the starting function with the stack pointer
    /// below its starting value is inside a callee. Stops early on a pc in
    /// `breakpoints` or in the machine's own breakpoint set
    /// (`"breakpoint"`), when the firmware halts (`"halted"`), or after
    /// `max_instructions` (`"cap"`). Each instruction is journaled like
    /// `step_single`. `location` is as `source_location` returns it.
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
        let journal_before = sim
            .tools
            .borrow()
            .journal
            .iter()
            .map(|(_, t)| *t)
            .sum::<u32>();

        let (reason, pc, n) = sim.step_source_line_s(true, 1_000_000, &[]).unwrap();
        assert_eq!(reason, "line_changed");
        assert_eq!((pc, line_of(&sim, pc)), (0x2ca, 223));
        assert!(n > 100, "uart_puts ran inside the step ({n} instructions)");
        let journal_after = sim
            .tools
            .borrow()
            .journal
            .iter()
            .map(|(_, t)| *t)
            .sum::<u32>();
        assert_eq!(
            journal_after - journal_before,
            n,
            "each instruction journaled"
        );

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
