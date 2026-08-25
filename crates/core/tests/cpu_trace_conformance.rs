// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Standardized instruction-trace instrumentation, proven per CPU core.
//!
//! Every core must emit the same per-instruction trace, in the same shape, so
//! that `--trace`, VCD export, coverage and the DAP adapter behave identically
//! no matter which chip is under the debugger. The contract itself is written
//! down on `SimulationObserver`; this file is what makes it true.
//!
//! It exists because it was NOT true. Xtensa — the core behind classic ESP32
//! and ESP32-S3 — ignored its `observers` argument entirely, so `--trace`
//! produced an empty file for those chips and every Xtensa debugging session
//! fell back to guesswork. Nothing failed, because nothing checked.
//!
//! Note the shape of the guard: a trace test that only asserts "some events
//! arrived" would have passed on a core that emits one bogus event, and a test
//! that hard-codes today's three cores would go quietly stale the day a fourth
//! lands. So this runs real instructions through each core and checks the
//! emissions against what the core actually did, and it derives the core list
//! from the source tree rather than trusting the list below.

use labwired_core::cpu::{Avr, CortexM, RiscV, XtensaLx7};
use labwired_core::{
    trace_sp_pc, Bus, Cpu, DmaRequest, SimResult, SimulationConfig, SimulationObserver,
};
use labwired_hw_trace::TraceEvent;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ── Harness ──────────────────────────────────────────────────────────────────

/// Flat byte-addressable RAM based at 0. Deliberately dumb: this test is about
/// what the CPU *emits*, so the bus must not be able to influence the outcome.
#[derive(Debug)]
struct RamBus {
    mem: Vec<u8>,
    config: SimulationConfig,
}

impl RamBus {
    fn with_program(entry: u64, program: &[u8]) -> Self {
        let mut mem = vec![0u8; 0x1_0000];
        mem[entry as usize..entry as usize + program.len()].copy_from_slice(program);
        Self {
            mem,
            config: SimulationConfig::default(),
        }
    }
}

impl Bus for RamBus {
    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        Ok(self.mem.get(addr as usize).copied().unwrap_or(0))
    }
    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
        if let Some(slot) = self.mem.get_mut(addr as usize) {
            *slot = value;
        }
        Ok(())
    }
    fn tick_peripherals(&mut self) -> Vec<u32> {
        Vec::new()
    }
    fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
        Ok(())
    }
    fn config(&self) -> &SimulationConfig {
        &self.config
    }
}

/// One emitted step, reassembled from the three callbacks.
#[derive(Debug, Clone)]
struct Step {
    start_pc: u32,
    start_opcode: u32,
    retired: Option<(u32, u32)>,
    registers: Vec<u32>,
}

#[derive(Debug, Default)]
struct Recorder {
    steps: Mutex<Vec<Step>>,
    /// Callbacks seen since the last `on_step_end`, so ordering is checkable.
    pending: Mutex<Option<Step>>,
}

impl Recorder {
    fn steps(&self) -> Vec<Step> {
        self.steps.lock().unwrap().clone()
    }
}

impl SimulationObserver for Recorder {
    fn on_step_start(&self, pc: u32, opcode: u32) {
        let mut pending = self.pending.lock().unwrap();
        assert!(
            pending.is_none(),
            "on_step_start fired twice with no on_step_end between: \
             the core is dropping the tail of the trace contract"
        );
        *pending = Some(Step {
            start_pc: pc,
            start_opcode: opcode,
            retired: None,
            registers: Vec::new(),
        });
    }

    fn on_trace_event(&self, event: TraceEvent) {
        if let TraceEvent::InstructionRetired { pc, opcode } = event {
            let mut pending = self.pending.lock().unwrap();
            let step = pending
                .as_mut()
                .expect("InstructionRetired arrived outside a step");
            step.retired = Some((pc, opcode));
        }
    }

    fn on_step_end(&self, _cycles: u32, registers: &[u32]) {
        let mut step = self
            .pending
            .lock()
            .unwrap()
            .take()
            .expect("on_step_end without a matching on_step_start");
        step.registers = registers.to_vec();
        self.steps.lock().unwrap().push(step);
    }
}

/// A core, plus the smallest real program that exercises it.
struct CoreUnderTest {
    /// Type name as it appears in `impl Cpu for …`; ties the row to the source.
    name: &'static str,
    entry: u32,
    /// Straight-line, side-effect-free instructions, little-endian.
    program: Vec<u8>,
    /// Bytes each instruction advances PC by.
    step_len: u32,
    /// Build the core with `program` already loaded at `entry`.
    ///
    /// Takes the program because not every core fetches from the bus. AVR is a
    /// Harvard machine: it executes from its OWN flash array, so a program
    /// written only into `RamBus` would leave it stepping over zeros.
    build: fn(u32, &[u8]) -> Box<dyn Cpu>,
}

/// Every core that implements `Cpu`. Completeness is enforced by
/// `every_cpu_core_is_covered_by_this_file`, not by reviewer diligence.
fn cores() -> Vec<CoreUnderTest> {
    vec![
        CoreUnderTest {
            name: "CortexM",
            entry: 0x100,
            // `movs r0, #1` ×4 (Thumb, 2 bytes each).
            program: vec![0x01, 0x20, 0x01, 0x20, 0x01, 0x20, 0x01, 0x20],
            step_len: 2,
            build: |_entry, _program| {
                let mut cpu = CortexM::new();
                cpu.set_sp(0x8000);
                Box::new(cpu)
            },
        },
        CoreUnderTest {
            name: "RiscV",
            entry: 0x100,
            // `addi x1, x1, 1` ×4 (4 bytes each).
            program: std::iter::repeat_n(0x00108093u32, 4)
                .flat_map(u32::to_le_bytes)
                .collect(),
            step_len: 4,
            build: |_entry, _program| {
                let mut cpu = RiscV::new();
                cpu.set_sp(0x8000);
                Box::new(cpu)
            },
        },
        CoreUnderTest {
            name: "Avr",
            entry: 0x100,
            // `ldi r16, 1` x4 (2 bytes each, little-endian in flash).
            //
            // LDI and not NOP on purpose: NOP encodes as 0x0000, so the
            // "opcode must be the real encoding" assertion below would hold
            // for a core that reported a hard-coded zero. 0xE001 cannot be
            // produced by accident.
            program: vec![0x01, 0xE0, 0x01, 0xE0, 0x01, 0xE0, 0x01, 0xE0],
            step_len: 2,
            build: |entry, program| {
                let mut cpu = Avr::new();
                // Harvard: instructions come from flash, not from the bus.
                cpu.load_flash(entry, program);
                cpu.set_sp(0x8000);
                Box::new(cpu)
            },
        },
        CoreUnderTest {
            name: "XtensaLx7",
            entry: 0x100,
            // `movi.n a2, 1` ×4 (narrow, 2 bytes each).
            program: vec![0x0C, 0x12, 0x0C, 0x12, 0x0C, 0x12, 0x0C, 0x12],
            step_len: 2,
            build: |_entry, _program| {
                let mut cpu = XtensaLx7::new();
                cpu.set_sp(0x8000);
                Box::new(cpu)
            },
        },
    ]
}

// ── The contract ─────────────────────────────────────────────────────────────

#[test]
fn every_core_emits_the_full_per_instruction_trace() {
    const STEPS: usize = 4;

    for core in cores() {
        let mut cpu = (core.build)(core.entry, &core.program);
        cpu.set_pc(core.entry);
        let mut bus = RamBus::with_program(core.entry as u64, &core.program);
        let recorder = Arc::new(Recorder::default());
        let observers: Vec<Arc<dyn SimulationObserver>> = vec![recorder.clone()];
        let config = SimulationConfig::default();

        for _ in 0..STEPS {
            cpu.step(&mut bus, &observers, &config)
                .unwrap_or_else(|e| panic!("{}: step failed: {e:?}", core.name));
        }

        let steps = recorder.steps();
        assert_eq!(
            steps.len(),
            STEPS,
            "{}: emitted {} traced steps for {STEPS} executed instructions — \
             a core that runs instructions without tracing them makes `--trace` \
             silently useless for every chip built on it",
            core.name,
            steps.len(),
        );

        for (i, step) in steps.iter().enumerate() {
            let expected_pc = core.entry + (i as u32) * core.step_len;
            assert_eq!(
                step.start_pc, expected_pc,
                "{}: step {i} reported pc={:#x}, executed {:#x}",
                core.name, step.start_pc, expected_pc
            );

            // The opcode must be the real encoding. A core that passes 0 (or a
            // decoded-enum discriminant) here produces a trace that cannot be
            // disassembled, which is most of what a trace is for.
            let width = core.step_len as usize;
            let mut expected_opcode = 0u32;
            for b in (0..width).rev() {
                expected_opcode = (expected_opcode << 8) | core.program[i * width + b] as u32;
            }
            assert_eq!(
                step.start_opcode, expected_opcode,
                "{}: step {i} reported opcode {:#x}, memory holds {:#x}",
                core.name, step.start_opcode, expected_opcode
            );

            assert_eq!(
                step.retired,
                Some((expected_pc, expected_opcode)),
                "{}: step {i} did not emit a matching InstructionRetired event",
                core.name
            );

            // Standardized trailer: SP then PC, regardless of core.
            let (sp, pc) = trace_sp_pc(&step.registers).unwrap_or_else(|| {
                panic!(
                    "{}: register slice of len {} is too short to carry the \
                     standard [.., SP, PC] trailer",
                    core.name,
                    step.registers.len()
                )
            });
            assert_eq!(
                pc,
                expected_pc + core.step_len,
                "{}: step {i} trailer PC is {:#x}, expected the next pc {:#x}",
                core.name,
                pc,
                expected_pc + core.step_len
            );
            assert_eq!(
                sp, 0x8000,
                "{}: step {i} trailer SP is {:#x}, expected the stack pointer \
                 we set ({:#x}) — the trailer is reporting some other register",
                core.name, sp, 0x8000u32
            );
        }
    }
}

#[test]
fn no_core_traces_when_nobody_is_observing() {
    // The gate that keeps the contract affordable. If a core stopped honouring
    // it the hot path would pay for a register snapshot on every instruction,
    // which is the reason the emission is conditional in the first place.
    for core in cores() {
        let mut cpu = (core.build)(core.entry, &core.program);
        cpu.set_pc(core.entry);
        let mut bus = RamBus::with_program(core.entry as u64, &core.program);
        let none: Vec<Arc<dyn SimulationObserver>> = Vec::new();
        let config = SimulationConfig::default();
        for _ in 0..4 {
            cpu.step(&mut bus, &none, &config)
                .unwrap_or_else(|e| panic!("{}: step failed: {e:?}", core.name));
        }
        assert_eq!(
            cpu.get_pc(),
            core.entry + 4 * core.step_len,
            "{}: unobserved run diverged from the observed one",
            core.name
        );
    }
}

#[test]
fn every_cpu_core_is_covered_by_this_file() {
    // Derived from the source tree, so a new core cannot ship untraced: adding
    // `impl Cpu for Foo` fails this test until Foo is in `cores()` above.
    let cpu_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cpu");
    let mut found: Vec<String> = Vec::new();
    let mut stack = vec![cpu_dir.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read src/cpu") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("read source");
            for line in src.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("impl Cpu for ") {
                    let name = rest.trim_end_matches('{').trim();
                    // Blanket impls over references/boxes forward to the
                    // concrete core and have nothing of their own to trace.
                    if name.starts_with('&') || name.starts_with("Box<") {
                        continue;
                    }
                    found.push(name.to_string());
                }
            }
        }
    }
    found.sort();
    found.dedup();
    assert!(
        !found.is_empty(),
        "found no `impl Cpu for` in {} — this guard has stopped guarding \
         anything; fix the scan before trusting a green run",
        cpu_dir.display()
    );

    let mut covered: Vec<String> = cores().iter().map(|c| c.name.to_string()).collect();
    covered.sort();

    assert_eq!(
        found, covered,
        "cores implementing `Cpu` and cores exercised by this file have \
         diverged. Every core must emit the standardized instruction trace; \
         add the missing one to `cores()` (and make it emit) rather than \
         relaxing this assertion."
    );
}
