// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MachineSnapshot {
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
