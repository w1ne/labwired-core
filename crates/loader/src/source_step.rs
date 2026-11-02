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
use crate::{
    Abi, DwarfLocation, FrameContext, InlineFrame, PhysicalFrame, RegisterSet, SymbolProvider,
    TargetMemory, VariableValue,
};
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

/// The most frames a call stack shows.
pub const MAX_STACK_FRAMES: usize = 32;

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
    /// How many frames of the inline chain at `pc` the stop shows (see
    /// [`SourceDebug::location_at`]); 0 when `pc` has no debug information.
    pub depth: usize,
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

/// Where a source step starts in the inline chain at its pc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepFrame {
    /// Frames shown (1 = the physical function); 0 without debug info.
    pub depth: usize,
    /// Frames in the chain.
    pub len: usize,
    /// The shown frame's file and line.
    pub pos: Option<(String, u32)>,
    /// The physical function.
    pub function: Option<Range<u64>>,
}

/// See [`SourceDebug::over_inline`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineVerdict {
    /// Not inline-related: the plain line comparison decides.
    Defer,
    /// Still the line being stepped.
    Continue,
    /// Another line's code, but not where it starts: keep stepping.
    Pass,
    /// A new line.
    Stop,
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

    /// The inline chain at `pc`, innermost first; the last frame is the
    /// physical function (see [`SymbolProvider::frames`]).
    pub fn frames(&self, pc: u32) -> Vec<InlineFrame> {
        self.symbols.frames(self.map.code_address(u64::from(pc)))
    }

    /// How many frames of the chain at `pc` a stop that no step chose (a
    /// breakpoint, a pause) shows: all of them, less the inlined calls whose
    /// body starts exactly at `pc`. There the program is about to run the
    /// call, so the line that makes it is shown, as a breakpoint set on that
    /// line expects.
    pub fn default_depth(&self, pc: u32) -> usize {
        let frames = self.frames(pc);
        let calls: Vec<(&str, u32)> = self.map.inline_calls_at(u64::from(pc)).collect();
        let mut depth = frames.len();
        for caller in frames.iter().skip(1) {
            let site = caller.file.as_deref().zip(caller.line);
            if depth > 1 && site.is_some_and(|site| calls.contains(&site)) {
                depth -= 1;
            } else {
                break;
            }
        }
        depth
    }

    /// Where `pc` is, shown `depth` frames deep into its inline chain: 1 is
    /// the physical function at the line of the outermost inlined call, the
    /// chain's length (or more) is [`Self::location`].
    pub fn location_at(&self, pc: u32, depth: usize) -> Option<SourceLocation> {
        let frames = self.frames(pc);
        if depth == 0 || depth >= frames.len() {
            return self.location(pc);
        }
        let frame = &frames[frames.len() - depth];
        match (frame.file.clone(), frame.line.filter(|l| *l != 0)) {
            (Some(file), Some(line)) => Some(SourceLocation {
                file,
                line,
                column: None,
                function: frame.function.clone(),
            }),
            // A call site the compiler gave no line: show the code itself.
            _ => self.location(pc),
        }
    }

    /// Where a source step starts: the frame shown at `pc` (`view`, else
    /// [`Self::default_depth`]).
    pub fn step_frame(&self, pc: u32, view: Option<usize>) -> StepFrame {
        let frames = self.frames(pc);
        let len = frames.len();
        let depth = view.unwrap_or_else(|| self.default_depth(pc)).min(len);
        let pos = depth
            .checked_sub(1)
            .and_then(|d| frames.get(len - 1 - d))
            .and_then(|f| f.file.clone().zip(f.line));
        StepFrame {
            depth,
            len,
            pos,
            function: self.function_range(pc),
        }
    }

    /// Step-over's verdict on the instruction at `pc`, in the physical
    /// function the step started in.
    ///
    /// Deeper than the frame being stepped is code inlined into it. Code
    /// inlined from another file (core's `write_volatile`, a range iterator,
    /// a `nop()` delay loop) is part of the line being stepped, as it always
    /// was. A function of the stepped frame's own file inlined into it
    /// (`lcd_frame(&FB);`) is a line of its own: the step stops where the
    /// next line's inlined call starts. Shallower than the stepped frame is
    /// its return to the line that inlined it.
    pub fn over_inline(&self, start: &StepFrame, pc: u32) -> InlineVerdict {
        if start.depth == 0 || self.function_range(pc) != start.function {
            return InlineVerdict::Defer;
        }
        let frames = self.frames(pc);
        let n = frames.len();
        let statement = self.map.is_statement(u64::from(pc));
        if n < start.depth {
            return if statement {
                InlineVerdict::Stop
            } else {
                InlineVerdict::Pass
            };
        }
        if n == start.depth {
            return InlineVerdict::Defer;
        }
        let at = &frames[n - start.depth];
        let callee = &frames[n - start.depth - 1];
        let own_file = start.pos.as_ref().map(|p| p.0.as_str());
        if callee.file.as_deref() != own_file {
            return InlineVerdict::Continue;
        }
        let line = at.line.filter(|l| *l != 0);
        if line.is_none() || at.file.clone().zip(line) == start.pos {
            // Still the line being stepped (or code the compiler gave no line).
            InlineVerdict::Continue
        } else if statement || self.map.inline_calls_at(u64::from(pc)).next().is_some() {
            // The next line's inlined call starts here; its first
            // instruction need not start a line-table statement.
            InlineVerdict::Stop
        } else {
            InlineVerdict::Pass
        }
    }

    /// The depth a step that stopped at `pc` shows: the stepped frame (or
    /// the shallower one it returned to) for a step over that stayed in its
    /// physical function, the whole chain for a step into, else as a
    /// breakpoint would.
    pub fn stop_depth(&self, start: &StepFrame, pc: u32, over: bool) -> usize {
        let n = self.frames(pc).len();
        if !over {
            n
        } else if start.depth > 0 && self.function_range(pc) == start.function {
            n.min(start.depth)
        } else {
            self.default_depth(pc)
        }
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
            MachineFamily::CortexM
            | MachineFamily::CortexA
            | MachineFamily::CortexR
            | MachineFamily::Xtensa => 16,
            MachineFamily::RiscV | MachineFamily::Avr => 32,
            // W0-W15, PC (16), SR (17) — provisional until XC16 DWARF numbers
            // are verified.
            MachineFamily::Pic24 => 18,
            // WREG, STATUS, BSR, FSR0-2, SP, PC — provisional until XC8 DWARF
            // numbering is verified.
            MachineFamily::Pic18 => 8,
            // W, STATUS, FSR, PCLATH, SP, PC — provisional, same caveat.
            MachineFamily::Pic16 => 6,
            // R0-R15 (GCC's RX DWARF numbering).
            MachineFamily::Rx => 16,
            // $0-$31 (GCC's MIPS DWARF numbering).
            MachineFamily::Mips => 32,
            // X0-X30, SP (31).
            MachineFamily::Aarch64 => 32,
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

/// One frame of the call stack, innermost first. A physical frame (one
/// activation of a real function) shows as its inline chain: the inlined
/// calls first, then the function they were inlined into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StackFrame {
    pub index: usize,
    /// The frame's pc: where frame 0 stopped, or for a caller the return
    /// address (the instruction after the call).
    pub pc: u32,
    pub function: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// True for a call the compiler inlined into the frame below it.
    pub inlined: bool,
}

/// A logical frame and where its variables come from.
struct FrameSlot {
    frame: StackFrame,
    /// Index into the physical frames.
    physical: usize,
    /// The inline level whose variables the frame shows (0 is the physical
    /// function); `None` without debug information.
    level: Option<usize>,
}

/// The debugged core's memory, read through the bus.
struct BusMemory<'a>(&'a dyn SourceStepTarget);

impl TargetMemory for BusMemory<'_> {
    fn read_memory(&self, addr: u64, buf: &mut [u8]) -> bool {
        let Ok(base) = u32::try_from(addr) else {
            return false;
        };
        for (i, b) in buf.iter_mut().enumerate() {
            match self.0.read_code_u8(base.wrapping_add(i as u32)) {
                Some(v) => *b = v,
                None => return false,
            }
        }
        true
    }
}

/// How to unwind and read registers on `family`. Cortex-M/A and RISC-V
/// unwind with CFI; Xtensa (register windows) and AVR show frame 0 only.
fn abi(family: MachineFamily) -> Abi {
    match family {
        MachineFamily::CortexM | MachineFamily::CortexA | MachineFamily::CortexR => Abi {
            sp: 13,
            callee_saved: (4..=11).collect(),
            code_mask: !1,
            register_size: 4,
            unwind: true,
            exc_return: family == MachineFamily::CortexM,
        },
        MachineFamily::RiscV => Abi {
            sp: 2,
            callee_saved: [3, 4, 8, 9].into_iter().chain(18..=27).collect(),
            code_mask: !0,
            register_size: 4,
            unwind: true,
            exc_return: false,
        },
        MachineFamily::Xtensa => Abi {
            sp: 1,
            callee_saved: Vec::new(),
            code_mask: !0,
            register_size: 4,
            unwind: false,
            exc_return: false,
        },
        MachineFamily::Avr => Abi {
            sp: 32,
            callee_saved: Vec::new(),
            code_mask: !0,
            register_size: 1,
            unwind: false,
            exc_return: false,
        },
        MachineFamily::Pic24 => Abi {
            // W15 is the stack pointer.
            sp: 15,
            callee_saved: Vec::new(),
            code_mask: !0,
            register_size: 2,
            unwind: false,
            exc_return: false,
        },
        MachineFamily::Pic18 => Abi {
            // SP (register 6) is the hardware return-stack pointer.
            sp: 6,
            callee_saved: Vec::new(),
            code_mask: !1,
            register_size: 1,
            unwind: false,
            exc_return: false,
        },
        MachineFamily::Pic16 => Abi {
            // SP (register 4) is the hardware return-stack pointer; the PC
            // is a word address, so no bit is masked off.
            sp: 4,
            callee_saved: Vec::new(),
            code_mask: !0,
            register_size: 1,
            unwind: false,
            exc_return: false,
        },
        MachineFamily::Rx => Abi {
            // R0 is the stack pointer. Frame 0 only until the RX CFI unwind
            // has been checked against GCC output.
            sp: 0,
            callee_saved: (6..=13).collect(),
            code_mask: !0,
            register_size: 4,
            unwind: false,
            exc_return: false,
        },
        MachineFamily::Mips => Abi {
            // $29 is the stack pointer; s0-s7 and s8/fp are callee-saved
            // (o32). Frame 0 only until the MIPS CFI unwind has been checked
            // against GCC output.
            sp: 29,
            callee_saved: (16..=23).chain(30..=30).collect(),
            code_mask: !0,
            register_size: 4,
            unwind: false,
            exc_return: false,
        },
        MachineFamily::Aarch64 => Abi {
            // DWARF 31 is SP. Frame 0 only until the AArch64 CFI unwind has
            // been checked against GCC output (registers read as 32 bits).
            sp: 31,
            callee_saved: (19..=29).collect(),
            code_mask: !0,
            register_size: 8,
            unwind: false,
            exc_return: false,
        },
    }
}

/// The debugged core's registers by DWARF number: r0-r15, x0-x31, the
/// current window's a0-a15, or AVR r0-r31 and SP (32).
fn live_registers(target: &dyn SourceStepTarget, family: MachineFamily) -> RegisterSet {
    let cpu = target.debug_cpu();
    let mut regs = RegisterSet::default();
    let count: u8 = match family {
        MachineFamily::CortexM
        | MachineFamily::CortexA
        | MachineFamily::CortexR
        | MachineFamily::Xtensa => 16,
        MachineFamily::RiscV => 32,
        MachineFamily::Avr => 33,
        // W0-W15, PC (16), SR (17).
        MachineFamily::Pic24 => 18,
        MachineFamily::Pic18 => 8,
        MachineFamily::Pic16 => 6,
        MachineFamily::Rx => 16,
        MachineFamily::Mips => 32,
        // X0-X30, SP.
        MachineFamily::Aarch64 => 32,
    };
    for r in 0..count {
        regs.set(u16::from(r), Some(u64::from(cpu.get_register(r))));
    }
    match family {
        MachineFamily::CortexM | MachineFamily::CortexA | MachineFamily::CortexR => {
            regs.set(15, Some(u64::from(cpu.get_pc())));
        }
        MachineFamily::RiscV => regs.set(0, Some(0)),
        _ => {}
    }
    regs
}

impl SourceDebug {
    /// The call stack of the debugged core, innermost frame first, at most
    /// `max_frames` frames. Frame 0 is the frame the stop shows (`view`, the
    /// inline depth the last source step left, else
    /// [`Self::default_depth`]), as [`Self::location_at`] reports it.
    pub fn call_stack(
        &self,
        target: &dyn SourceStepTarget,
        family: MachineFamily,
        view: Option<usize>,
        max_frames: usize,
    ) -> Vec<StackFrame> {
        let (_, slots) = self.stack(target, family, view, max_frames);
        slots.into_iter().map(|s| s.frame).collect()
    }

    /// The variables and parameters of call-stack frame `index` (see
    /// [`Self::call_stack`]) with their values in that frame. `None` when
    /// there is no such frame.
    pub fn frame_variables(
        &self,
        target: &dyn SourceStepTarget,
        family: MachineFamily,
        view: Option<usize>,
        index: usize,
    ) -> Option<Vec<VariableValue>> {
        let (physical, slots) = self.stack(target, family, view, index + 1);
        let slot = slots.get(index)?;
        let frame = &physical[slot.physical];
        let mem = BusMemory(target);
        let ctx = FrameContext {
            pc: frame.lookup_pc,
            regs: &frame.regs,
            cfa: frame.cfa,
            mem: &mem,
            register_size: abi(family).register_size,
        };
        Some(self.symbols.variables(&ctx, slot.level))
    }

    /// The physical frames and, for at least `max_frames` logical frames,
    /// what each shows.
    fn stack(
        &self,
        target: &dyn SourceStepTarget,
        family: MachineFamily,
        view: Option<usize>,
        max_frames: usize,
    ) -> (Vec<PhysicalFrame>, Vec<FrameSlot>) {
        let abi = abi(family);
        let pc = target.debug_cpu().get_pc();
        let regs = live_registers(target, family);
        let mem = BusMemory(target);
        let max_frames = max_frames.clamp(1, MAX_STACK_FRAMES);
        let physical = self.symbols.unwind(
            self.map.code_address(u64::from(pc)),
            regs,
            &mem,
            &abi,
            max_frames,
        );
        let mut slots: Vec<FrameSlot> = Vec::new();
        for (i, frame) in physical.iter().enumerate() {
            if slots.len() >= max_frames {
                break;
            }
            let lookup = frame.lookup_pc as u32;
            let chain = self.frames(lookup);
            let n = chain.len();
            let push = |slots: &mut Vec<FrameSlot>, mut f: StackFrame, level| {
                f.index = slots.len();
                slots.push(FrameSlot {
                    frame: f,
                    physical: i,
                    level,
                });
            };
            if n == 0 {
                // No debug information: the symbol table names the function.
                let function = self
                    .symbols
                    .functions()
                    .iter()
                    .rev()
                    .find(|(start, size, _)| {
                        *start <= frame.lookup_pc && frame.lookup_pc < start + size
                    })
                    .map(|(_, _, name)| name.clone());
                push(
                    &mut slots,
                    StackFrame {
                        index: 0,
                        pc: frame.pc as u32,
                        function,
                        file: None,
                        line: None,
                        column: None,
                        inlined: false,
                    },
                    None,
                );
                continue;
            }
            // Frame 0 shows `depth` frames of its chain; callers show all.
            let depth = if i == 0 {
                view.unwrap_or_else(|| self.default_depth(pc)).clamp(1, n)
            } else {
                n
            };
            for (k, inline) in chain.iter().enumerate().skip(n - depth) {
                let level = n - 1 - k;
                let shown = if i == 0 && k == n - depth {
                    self.location_at(pc, depth)
                        .map(|l| (Some(l.file), Some(l.line), l.column, l.function))
                } else {
                    None
                };
                let (file, line, column, function) = shown.unwrap_or_else(|| {
                    (
                        inline.file.clone(),
                        inline.line.filter(|l| *l != 0),
                        None,
                        inline.function.clone(),
                    )
                });
                push(
                    &mut slots,
                    StackFrame {
                        index: 0,
                        pc: frame.pc as u32,
                        function,
                        file,
                        line,
                        column,
                        inlined: k + 1 < n,
                    },
                    Some(level),
                );
            }
        }
        slots.truncate(max_frames);
        (physical, slots)
    }
}

/// The stack pointer's id for `get_register`.
fn stack_register(family: MachineFamily) -> u8 {
    match family {
        MachineFamily::CortexM | MachineFamily::CortexA | MachineFamily::CortexR => 13,
        MachineFamily::RiscV => 2,
        // a1 of the current register window.
        MachineFamily::Xtensa => 1,
        // SP, after r0..r31.
        MachineFamily::Avr => 32,
        // W15 is the stack pointer.
        MachineFamily::Pic24 => 15,
        MachineFamily::Pic18 => 6,
        MachineFamily::Pic16 => 4,
        MachineFamily::Rx => 0,
        MachineFamily::Mips => 29,
        MachineFamily::Aarch64 => 31,
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
    view: Option<usize>,
) -> Result<StepOutcome, String> {
    let sp_id = stack_register(family);
    // Cortex-M and RISC-V find a call by its return-address register.
    let ra_id = match family {
        MachineFamily::CortexM | MachineFamily::CortexA | MachineFamily::CortexR => Some(14),
        MachineFamily::RiscV => Some(1),
        _ => None,
    };
    let run_budget = u64::from(max_instructions).saturating_mul(RUN_THROUGH_CYCLES_PER_STEP);
    let xtensa = family == MachineFamily::Xtensa;

    let (start_pc, start_sp) = pc_sp(target, sp_id);
    let frame = debug.step_frame(start_pc, view);
    if !over && frame.depth < frame.len {
        // Into a call inlined here: its body starts at this pc, so show it
        // one frame deeper without running anything.
        return Ok(StepOutcome {
            stop: StepStop::LineChanged,
            pc: start_pc,
            instructions: 0,
            depth: frame.depth + 1,
        });
    }
    // Started in a shallower inline frame than the innermost, the line
    // being stepped is that frame's: the call site of what is inlined there.
    let start_line = if frame.depth < frame.len {
        frame
            .pos
            .as_ref()
            .and_then(|(file, line)| debug.map.key_of(file, *line))
    } else {
        debug.map.line_key(u64::from(start_pc))
    };
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
        depth: if stop == StepStop::Breakpoint {
            debug.default_depth(pc)
        } else {
            debug.stop_depth(&frame, pc, over)
        },
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
        match if over {
            debug.over_inline(&frame, pc)
        } else {
            InlineVerdict::Defer
        } {
            InlineVerdict::Continue => {
                left_file = true;
                continue;
            }
            InlineVerdict::Pass => continue,
            InlineVerdict::Stop => {
                return Ok(outcome(StepStop::LineChanged, pc, stepped, ran_through));
            }
            InlineVerdict::Defer => {}
        }
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
    view: Option<usize>,
) -> Result<StepOutcome, String> {
    match kind {
        StepKind::Into => step_source_line(
            target,
            debug,
            family,
            false,
            max_instructions,
            breakpoints,
            view,
        ),
        StepKind::Over => step_source_line(
            target,
            debug,
            family,
            true,
            max_instructions,
            breakpoints,
            view,
        ),
        StepKind::Out => {
            let start = target.debug_cpu().get_pc();
            let function = debug.function_range(start);
            let depth = debug.step_frame(start, view).depth;
            let mut view = view;
            let mut total = 0u32;
            for _ in 0..STEP_OUT_MAX_LINES {
                let mut out = step_source_line(
                    target,
                    debug,
                    family,
                    true,
                    max_instructions,
                    breakpoints,
                    view,
                )?;
                total = total.saturating_add(out.instructions);
                out.instructions = total;
                // Out of an inlined function is back in the frame that
                // inlined it; out of a physical one is in another function.
                if out.stop != StepStop::LineChanged
                    || debug.function_range(out.pc) != function
                    || out.depth < depth
                {
                    return Ok(out);
                }
                view = Some(out.depth);
            }
            let pc = target.debug_cpu().get_pc();
            Ok(StepOutcome {
                stop: StepStop::Cap,
                pc,
                instructions: total,
                depth: debug.default_depth(pc),
            })
        }
    }
}
