// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current on-disk schema version for `MachineSnapshot`. Bump this when any
/// field added / removed / reinterpreted would cause a silent mis-restore.
/// A v0 snapshot (missing field, thanks to `serde(default)`) is treated as
/// "pre-versioning" and rejected by `Machine::apply_snapshot`.
pub const SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    0
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MachineSnapshot {
    /// Schema version of this snapshot. Produced snapshots carry
    /// `SCHEMA_VERSION`; older JSON without the field deserializes to `0`
    /// via `serde(default)` and is rejected on restore.
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
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ArmCpuSnapshot {
    pub registers: Vec<u32>,
    pub xpsr: u32,
    pub primask: bool,
    pub pending_exceptions: u32,
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
