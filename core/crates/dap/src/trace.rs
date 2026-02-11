use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Single instruction execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionTrace {
    /// Program counter
    pub pc: u32,

    /// Instruction bytes (2 or 4 bytes for Thumb-2)
    pub instruction: u32,

    /// Cycle count when this instruction executed
    pub cycle: u64,

    /// Function name (from debug symbols, if available)
    pub function: Option<String>,

    /// Register changes (only registers that changed)
    pub register_delta: HashMap<u8, u32>,

    /// Memory writes (address, old_value, new_value)
    pub memory_writes: Vec<MemoryWrite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWrite {
    pub address: u64,
    pub old_value: u8,
    pub new_value: u8,
}

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

            // Remove old checkpoints
            self.checkpoints.retain(|cp| cp.trace_index > 0);
            for cp in &mut self.checkpoints {
                cp.trace_index = cp.trace_index.saturating_sub(1);
            }
        }

        self.current_cycle = trace.cycle;
        self.instructions.push_back(trace);

        // Create checkpoint every 10K instructions
        if self.current_cycle.is_multiple_of(10_000) {
            self.checkpoints.push(Checkpoint {
                cycle: self.current_cycle,
                trace_index: self.instructions.len() - 1,
            });
        }
    }

    /// Get trace records in a cycle range
    pub fn get_range(&self, start_cycle: u64, end_cycle: u64) -> Vec<&InstructionTrace> {
        self.instructions
            .iter()
            .filter(|t| t.cycle >= start_cycle && t.cycle <= end_cycle)
            .collect()
    }

    /// Get all traces (for initial timeline render)
    pub fn get_all(&self) -> Vec<InstructionTrace> {
        self.instructions.iter().cloned().collect()
    }

    /// Get current cycle count
    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// Get total number of traces
    pub fn len(&self) -> usize {
        self.instructions.len()
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_buffer_record() {
        let mut buffer = TraceBuffer::new(10);

        for i in 0..5 {
            buffer.record(InstructionTrace {
                pc: i * 4,
                instruction: 0,
                cycle: i as u64,
                function: None,
                register_delta: HashMap::new(),
                memory_writes: Vec::new(),
            });
        }

        assert_eq!(buffer.len(), 5);
        assert_eq!(buffer.current_cycle(), 4);
    }

    #[test]
    fn test_trace_buffer_circular() {
        let mut buffer = TraceBuffer::new(3);

        for i in 0..5 {
            buffer.record(InstructionTrace {
                pc: i * 4,
                instruction: 0,
                cycle: i as u64,
                function: None,
                register_delta: HashMap::new(),
                memory_writes: Vec::new(),
            });
        }

        // Should only keep last 3
        assert_eq!(buffer.len(), 3);

        let traces = buffer.get_all();
        assert_eq!(traces[0].cycle, 2);
        assert_eq!(traces[1].cycle, 3);
        assert_eq!(traces[2].cycle, 4);
    }

    #[test]
    fn test_trace_buffer_get_range() {
        let mut buffer = TraceBuffer::new(100);

        for i in 0..10 {
            buffer.record(InstructionTrace {
                pc: i * 4,
                instruction: 0,
                cycle: i as u64,
                function: None,
                register_delta: HashMap::new(),
                memory_writes: Vec::new(),
            });
        }

        let traces = buffer.get_range(3, 6);
        assert_eq!(traces.len(), 4); // cycles 3, 4, 5, 6
        assert_eq!(traces[0].cycle, 3);
        assert_eq!(traces[3].cycle, 6);
    }
}
