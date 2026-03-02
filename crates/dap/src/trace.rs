pub use labwired_core::trace::{InstructionTrace, MemoryWrite};
use std::collections::VecDeque;

/// Checkpoint for time-travel debugging
#[derive(Debug, Clone)]
pub struct Checkpoint {
    /// Cycle count of this checkpoint
    pub cycle: u64,
    /// Index in trace buffer
    pub trace_index: usize,
}

/// Circular buffer for instruction trace
pub struct TraceBuffer {
    /// Instruction history (circular buffer)
    instructions: VecDeque<InstructionTrace>,
    /// Checkpoints for time travel (every 10K instructions)
    checkpoints: Vec<Checkpoint>,
    /// Maximum buffer size
    max_size: usize,
    /// Current cycle count
    current_cycle: u64,
}

impl TraceBuffer {
    pub fn new(max_size: usize) -> Self {
        Self {
            instructions: VecDeque::with_capacity(max_size),
            checkpoints: Vec::new(),
            max_size,
            current_cycle: 0,
        }
    }

    /// Record a new instruction execution
    pub fn record(&mut self, trace: InstructionTrace) {
        if self.instructions.len() >= self.max_size {
            self.instructions.pop_front();
            // Adjust checkpoint indices
            self.checkpoints.retain(|cp| cp.trace_index > 0);
            for cp in &mut self.checkpoints {
                cp.trace_index = cp.trace_index.saturating_sub(1);
            }
        }
        self.current_cycle = trace.cycle;
        self.instructions.push_back(trace);

        // Simple checkpointing every 10K cycles
        if self.current_cycle.is_multiple_of(10_000) {
            self.checkpoints.push(Checkpoint {
                cycle: self.current_cycle,
                trace_index: self.instructions.len() - 1,
            });
        }
    }

    /// Remove and return the latest trace record (for step-back)
    pub fn pop_trace(&mut self) -> Option<InstructionTrace> {
        let trace = self.instructions.pop_back()?;
        // Remove checkpoint if it points to the now-removed instruction
        if let Some(cp) = self.checkpoints.last() {
            if cp.trace_index >= self.instructions.len() {
                self.checkpoints.pop();
            }
        }
        Some(trace)
    }

    pub fn get_range(&self, start_cycle: u64, end_cycle: u64) -> Vec<&InstructionTrace> {
        self.instructions
            .iter()
            .filter(|t| t.cycle >= start_cycle && t.cycle <= end_cycle)
            .collect()
    }

    pub fn get_all(&self) -> Vec<InstructionTrace> {
        self.instructions.iter().cloned().collect()
    }
}
