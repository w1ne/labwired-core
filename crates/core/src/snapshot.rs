// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use serde::{Deserialize, Serialize, Serializer};
use std::collections::{BTreeMap, HashMap};

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

fn serialize_peripherals<S>(
    peripherals: &HashMap<String, serde_json::Value>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let ordered = peripherals.iter().collect::<BTreeMap<_, _>>();
    ordered.serialize(serializer)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MachineSnapshot {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub cpu: CpuSnapshot,
    #[serde(serialize_with = "serialize_peripherals")]
    pub peripherals: HashMap<String, serde_json::Value>,
    // Future: metrics
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CpuSnapshot {
    Arm(ArmCpuSnapshot),
    /// Diagnostic visible state only; native Cortex-A restore is unsupported.
    CortexA(CortexACpuSnapshot),
    RiscV(RiscVCpuSnapshot),
    XtensaLx7(XtensaLx7CpuSnapshot),
    Avr(AvrCpuSnapshot),
    Pic24(Pic24CpuSnapshot),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CortexACpuSnapshot {
    pub registers: Vec<u32>,
    pub pc: u32,
    pub cpsr: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AvrCpuSnapshot {
    pub registers: Vec<u8>,
    pub pc: u32,
    pub sp: u16,
    pub sreg: u8,
}

/// PIC24/dsPIC33 visible state (word-addressed PC; W0–W15; UART1 latches;
/// M1a timers, interrupt latches and accumulators).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Pic24CpuSnapshot {
    pub registers: Vec<u16>,
    pub pc: u32,
    pub sr: u16,
    pub cycles: u64,
    pub u1mode: u16,
    pub u1sta: u16,
    #[serde(default)]
    pub t1con: u16,
    #[serde(default)]
    pub tmr1: u16,
    #[serde(default)]
    pub pr1: u16,
    #[serde(default)]
    pub t1_frac: u16,
    #[serde(default)]
    pub t2con: u16,
    #[serde(default)]
    pub tmr2: u16,
    #[serde(default)]
    pub pr2: u16,
    #[serde(default)]
    pub t2_frac: u16,
    #[serde(default)]
    pub t3con: u16,
    #[serde(default)]
    pub tmr3: u16,
    #[serde(default)]
    pub pr3: u16,
    #[serde(default)]
    pub t3_frac: u16,
    #[serde(default)]
    pub ifs0: u16,
    #[serde(default)]
    pub iec0: u16,
    #[serde(default)]
    pub ipc0: u16,
    #[serde(default)]
    pub ipc1: u16,
    #[serde(default)]
    pub ipc2: u16,
    #[serde(default)]
    pub intcon1: u16,
    #[serde(default)]
    pub intcon2: u16,
    #[serde(default)]
    pub corcon: u16,
    #[serde(default)]
    pub acc_a: i64,
    #[serde(default)]
    pub acc_b: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ArmCpuSnapshot {
    pub registers: Vec<u32>,
    pub pc: u32,
    pub xpsr: u32,
    pub primask: bool,
    pub pending_exceptions: u64,
    /// Exceptions 64..255 (words 1..3 of the widened bitmask). Optional so
    /// pre-widening snapshots still deserialize.
    #[serde(default)]
    pub pending_exceptions_hi: Vec<u64>,
    pub vtor: u32,
    #[serde(default)]
    pub waiting_for_event: bool,
    #[serde(default)]
    pub event_register: bool,
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

    /// Debug trigger module (hardware breakpoints / watchpoints). Absent in
    /// snapshots written before it was captured; those still load, with the
    /// trigger module at reset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triggers: Option<crate::cpu::riscv::trigger::TriggerSnapshot>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct XtensaLx7CpuSnapshot {
    /// Logical AR registers a0..a15 in the current window.
    pub registers: Vec<u32>,
    pub pc: u32,
    /// Raw PS register value.
    pub ps: u32,
    /// WindowBase (from ArFile).
    pub window_base: u8,
    /// WindowStart (from ArFile).
    pub window_start: u16,
    /// VECBASE SR value.
    pub vecbase: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm_cpu() -> CpuSnapshot {
        CpuSnapshot::Arm(ArmCpuSnapshot {
            registers: Vec::new(),
            pc: 0,
            xpsr: 0,
            primask: false,
            pending_exceptions: 0,
            pending_exceptions_hi: Vec::new(),
            vtor: 0,
            waiting_for_event: false,
            event_register: false,
        })
    }

    #[test]
    fn snapshot_json_orders_peripherals_by_name() {
        let mut peripherals = HashMap::new();
        for name in ["zeta", "beta", "alpha", "gamma"] {
            peripherals.insert(name.to_owned(), serde_json::json!({ "name": name }));
        }

        let snapshot = MachineSnapshot {
            schema_version: SCHEMA_VERSION,
            cpu: arm_cpu(),
            peripherals,
        };

        let json = serde_json::to_string(&snapshot).expect("serialize snapshot");
        let alpha = json.find("\"alpha\"").expect("alpha key");
        let beta = json.find("\"beta\"").expect("beta key");
        let gamma = json.find("\"gamma\"").expect("gamma key");
        let zeta = json.find("\"zeta\"").expect("zeta key");
        assert!(alpha < beta && beta < gamma && gamma < zeta, "{json}");
    }
}
