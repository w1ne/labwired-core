//! Trace event types shared between the simulator, debuggers, and hardware runners.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceEvent {
    InstructionRetired {
        pc: u32,
        opcode: u32,
    },
    BranchEdge {
        src: u32,
        target: u32,
        taken: bool,
    },
    MemoryWrite {
        addr: u64,
        old: u8,
        new: u8,
    },
    ExceptionEntry {
        vector: u32,
        handler_pc: u32,
    },
    FaultInjected {
        kind: String,
        target: String,
        at_step: u64,
        at_pc: u32,
        effect: FaultEffect,
    },
}

impl TraceEvent {
    pub fn pc(&self) -> Option<u32> {
        match self {
            Self::InstructionRetired { pc, .. } => Some(*pc),
            Self::BranchEdge { src, .. } => Some(*src),
            Self::ExceptionEntry { handler_pc, .. } => Some(*handler_pc),
            Self::FaultInjected { at_pc, .. } => Some(*at_pc),
            Self::MemoryWrite { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FaultEffect {
    DroppedClockGate,
    RegisterBitFlip { addr: u64, bit: u8 },
    MemoryBitFlip { addr: u64, bit: u8 },
    PeripheralStall,
    Custom(String),
}
