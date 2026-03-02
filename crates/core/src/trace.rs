use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

/// A single instruction execution trace point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionTrace {
    pub pc: u32,
    pub instruction: u32,
    pub cycle: u64,
    pub register_delta: HashMap<u8, (u32, u32)>,
    pub memory_writes: Vec<MemoryWrite>,
    pub mnemonic: Option<String>,
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

        let mut register_delta = HashMap::new();
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
            stack_depth: registers.get(13).copied().unwrap_or(0),
            function: None,
        });

        state.registers_before = registers.to_vec();
        state.total_cycles += cycles as u64;
    }
}
