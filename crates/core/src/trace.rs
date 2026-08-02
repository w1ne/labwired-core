use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

/// A single instruction execution trace point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionTrace {
    pub pc: u32,
    pub instruction: u32,
    pub cycle: u64,
    pub register_delta: BTreeMap<u8, (u32, u32)>,
    pub memory_writes: Vec<MemoryWrite>,
    pub mnemonic: Option<String>,
    /// Stack pointer after the instruction retired, read from the standardized
    /// trailer of the observer register slice (see `SimulationObserver`).
    ///
    /// Named `stack_depth` for wire compatibility with existing `trace.json`
    /// readers. It has always been the SP value, never a depth — but until the
    /// trailer existed it was read from index 13, which is SP only on ARM. On
    /// RISC-V and Xtensa that index is an unrelated temporary, so every
    /// non-ARM trace ever emitted had a meaningless number here.
    pub stack_depth: u32,
    pub function: Option<String>,
}

/// A single memory write event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWrite {
    pub address: u64,
    pub old_value: u8,
    pub new_value: u8,
}

#[derive(Debug)]
struct TraceState {
    traces: Vec<InstructionTrace>,
    current_pc: u32,
    current_opcode: u32,
    current_writes: Vec<MemoryWrite>,
    registers_before: Vec<u32>,
    total_cycles: u64,
}

/// Capture instruction-level details during simulation
#[derive(Debug)]
pub struct TraceObserver {
    state: Mutex<TraceState>,
    max_traces: usize,
}

impl TraceObserver {
    pub fn new(max_traces: usize) -> Self {
        Self {
            state: Mutex::new(TraceState {
                traces: Vec::with_capacity(usize::min(max_traces, 1000)),
                current_pc: 0,
                current_opcode: 0,
                current_writes: Vec::new(),
                registers_before: vec![0; 33],
                total_cycles: 0,
            }),
            max_traces,
        }
    }

    pub fn take_traces(&self) -> Vec<InstructionTrace> {
        let mut state = self.state.lock().unwrap();
        std::mem::take(&mut state.traces)
    }
}

impl crate::SimulationObserver for TraceObserver {
    fn on_simulation_start(&self) {}
    fn on_simulation_stop(&self) {}

    fn on_step_start(&self, pc: u32, opcode: u32) {
        let mut state = self.state.lock().unwrap();
        state.current_pc = pc;
        state.current_opcode = opcode;
        state.current_writes.clear();
    }

    fn on_memory_write(&self, addr: u64, old: u8, new: u8) {
        let mut state = self.state.lock().unwrap();
        state.current_writes.push(MemoryWrite {
            address: addr,
            old_value: old,
            new_value: new,
        });
    }

    fn on_step_end(&self, cycles: u32, registers: &[u32]) {
        let mut state = self.state.lock().unwrap();
        if state.traces.len() >= self.max_traces {
            return;
        }

        let mut register_delta = BTreeMap::new();
        for (i, &current_val) in registers.iter().enumerate() {
            let prev_val = state.registers_before.get(i).copied().unwrap_or(0);
            if prev_val != current_val {
                register_delta.insert(i as u8, (prev_val, current_val));
            }
        }

        let pc = state.current_pc;
        let instruction = state.current_opcode;
        let cycle = state.total_cycles;
        let writes = state.current_writes.clone();

        state.traces.push(InstructionTrace {
            pc,
            instruction,
            cycle,
            register_delta,
            memory_writes: writes,
            mnemonic: None,
            stack_depth: crate::trace_sp_pc(registers).map(|(sp, _)| sp).unwrap_or(0),
            function: None,
        });

        state.registers_before = registers.to_vec();
        state.total_cycles += cycles as u64;
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// One retired instruction, in the ring.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetiredInstruction {
    /// Position in the run, counted from the first retired instruction. Kept
    /// because the ring drops the beginning: without it there is no way to
    /// tell "the last 4096 of 8 million" from "the only 4096 that ran".
    pub seq: u64,
    pub pc: u32,
    pub opcode: u32,
    /// Stack pointer after retirement, from the standardized register trailer.
    pub sp: u32,
}

/// The last N retired instructions, for any core.
///
/// [`TraceObserver`] keeps the *first* `max_traces` instructions, which is the
/// wrong end of the run when the thing being debugged is a fault: the useful
/// window is whatever executed immediately before it. This keeps a fixed-size
/// ring instead, so a multi-million-instruction run costs bounded memory and
/// still answers "what ran just before it died".
///
/// Arch-agnostic by construction — it reads PC and SP out of the standardized
/// trailer described on [`crate::SimulationObserver`], so it works unchanged on
/// every core rather than needing a per-chip variant.
#[derive(Debug)]
pub struct RetiredRing {
    capacity: usize,
    state: Mutex<RetiredRingState>,
}

#[derive(Debug)]
struct RetiredRingState {
    entries: VecDeque<RetiredInstruction>,
    /// Total retired, including those already dropped out of the ring.
    total: u64,
    /// Carried from `on_step_start` to the matching `on_step_end`.
    pending: Option<(u32, u32)>,
}

impl RetiredRing {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            state: Mutex::new(RetiredRingState {
                entries: VecDeque::with_capacity(capacity),
                total: 0,
                pending: None,
            }),
        }
    }

    /// Instructions still in the ring, oldest first.
    pub fn entries(&self) -> Vec<RetiredInstruction> {
        self.state.lock().unwrap().entries.iter().copied().collect()
    }

    /// Total retired over the whole run, including entries dropped from the
    /// ring. Compare against `entries().len()` to see how much was discarded.
    pub fn total_retired(&self) -> u64 {
        self.state.lock().unwrap().total
    }
}

impl crate::SimulationObserver for RetiredRing {
    fn on_step_start(&self, pc: u32, opcode: u32) {
        self.state.lock().unwrap().pending = Some((pc, opcode));
    }

    fn on_step_end(&self, _cycles: u32, registers: &[u32]) {
        let mut state = self.state.lock().unwrap();
        let Some((pc, opcode)) = state.pending.take() else {
            return;
        };
        let sp = crate::trace_sp_pc(registers).map(|(sp, _)| sp).unwrap_or(0);
        let seq = state.total;
        state.total += 1;
        if state.entries.len() == self.capacity {
            state.entries.pop_front();
        }
        state.entries.push_back(RetiredInstruction {
            seq,
            pc,
            opcode,
            sp,
        });
    }
}

#[cfg(test)]
mod ring_tests {
    use super::*;
    use crate::SimulationObserver;

    fn regs(sp: u32, pc: u32) -> Vec<u32> {
        vec![0, 0, 0, sp, pc]
    }

    #[test]
    fn ring_keeps_the_last_window_not_the_first() {
        let ring = RetiredRing::new(3);
        for i in 0..10u32 {
            ring.on_step_start(0x1000 + i * 4, 0xAA00 + i);
            ring.on_step_end(1, &regs(0x8000 - i, 0x1004 + i * 4));
        }
        let entries = ring.entries();
        assert_eq!(entries.len(), 3, "ring exceeded its capacity");
        assert_eq!(ring.total_retired(), 10);
        // The tail, not the head — this is the whole point of the type.
        assert_eq!(entries[0].pc, 0x1000 + 7 * 4);
        assert_eq!(entries[2].pc, 0x1000 + 9 * 4);
        // seq survives the drop, so the window is locatable in the run.
        assert_eq!(entries[0].seq, 7);
        assert_eq!(entries[2].seq, 9);
    }

    #[test]
    fn ring_reads_sp_from_the_standard_trailer() {
        let ring = RetiredRing::new(4);
        ring.on_step_start(0x2000, 0x1234);
        ring.on_step_end(1, &regs(0x7FF0, 0x2004));
        assert_eq!(ring.entries()[0].sp, 0x7FF0);
    }

    #[test]
    fn a_step_that_faults_before_retiring_is_not_recorded() {
        // on_step_start with no matching on_step_end is exactly what a faulting
        // instruction looks like. It must not leak into the next entry.
        let ring = RetiredRing::new(4);
        ring.on_step_start(0x3000, 0x1111);
        assert!(ring.entries().is_empty());
        ring.on_step_start(0x3004, 0x2222);
        ring.on_step_end(1, &regs(0x8000, 0x3008));
        let entries = ring.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].pc, 0x3004, "stale pending step bled through");
    }
}
