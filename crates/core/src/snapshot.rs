// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// On-disk schema version for `MachineSnapshot`. Bump whenever the wire
/// format changes in a non-backwards-compatible way (new required fields,
/// renamed variants, changed semantics). Older snapshots whose version
/// does not match are rejected by `Machine::apply_snapshot` rather than
/// silently ignored — a zero-filled field is worse than a clean refusal.
pub const SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    // Treat a missing field as v0 (pre-versioned snapshots). `apply_snapshot`
    // will reject anything that isn't the current SCHEMA_VERSION, so this
    // default means "old snapshot, please re-record".
    0
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MachineSnapshot {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub cpu: CpuSnapshot,
    pub peripherals: HashMap<String, serde_json::Value>,
    // Future: metrics
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CpuSnapshot {
    Arm(ArmCpuSnapshot),
    RiscV(RiscVCpuSnapshot),
    Xtensa(XtensaCpuSnapshot),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ArmCpuSnapshot {
    pub registers: Vec<u32>,
    pub pc: u32,
    pub xpsr: u32,
    pub primask: bool,
    pub pending_exceptions: u64,
    pub vtor: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RiscVCpuSnapshot {
    pub registers: Vec<u32>,
    pub pc: u32,

    pub mstatus: u32,
    pub mie: u32,
    pub mip: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,

    pub mtime: u64,
    pub mtimecmp: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct XtensaCpuSnapshot {
    pub registers: Vec<u32>,
    pub pc: u32,

    pub ps: u32,
    pub sar: u32,
    pub lbeg: u32,
    pub lend: u32,
    pub lcount: u32,
    pub vecbase: u32,

    pub epc1: u32,
    pub exccause: u32,
    pub excsave1: u32,
    pub excvaddr: u32,
}
