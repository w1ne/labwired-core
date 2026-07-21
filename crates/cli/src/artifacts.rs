// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_config::{StopReason, TestAssertion, TestLimits};
use labwired_core::snapshot::CpuSnapshot;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

// Preserve the root command's tracing target after this behavior-preserving move.
macro_rules! error {
    ($($arg:tt)*) => {
        tracing::error!(target: "labwired", $($arg)*)
    };
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TestResult {
    pub(crate) result_schema_version: String,
    pub(crate) status: String,
    pub(crate) steps_executed: u64,
    pub(crate) cycles: u64,
    pub(crate) instructions: u64,
    pub(crate) stop_reason: StopReason,
    pub(crate) stop_reason_details: StopReasonDetails,
    pub(crate) limits: TestLimits,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
    pub(crate) assertions: Vec<AssertionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cpu_state: Option<labwired_core::snapshot::CpuSnapshot>,
    pub(crate) firmware_hash: String,
    pub(crate) config: TestConfig,
    /// Universal inspect block: final-state decoded register + artifact
    /// metadata for every peripheral (summary mode — framebuffer bytes omitted,
    /// hashed via `meta.generation`). Absent on config-error runs that never
    /// built a machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) inspect: Option<labwired_core::inspect::MachineInspect>,
    /// Structured coverage gaps the model hit during the run: unmapped MMIO and
    /// undecoded instructions, flattened from core's thread-local
    /// `FidelityReport`. Empty (and omitted) on a clean run, so honest runs stay
    /// clean. The builder maps this into `/run`'s `unmodeled_access[]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) fidelity: Vec<labwired_core::fidelity::FidelityGap>,
    /// Deterministic logic-analyzer edge capture for the pads named by
    /// `--watch-gpio`, drained from the SAME in-engine `LogicTap` the wasm
    /// `read_logic_edges` accessor uses (byte-for-byte parity). Per-channel
    /// transitions on the engine-cycle axis + a run-level `dropped` overflow
    /// count. Absent (and omitted) unless at least one pad was watched — the
    /// builder maps this into the oracle's `gpio` edge evidence for the
    /// prove-blink `gpio_edges`/`gpio_period`/`gpio_duty` clauses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) logic_edges: Option<labwired_core::logic_capture::LogicEdgesResult>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct StopReasonDetails {
    pub(crate) triggered_stop_condition: StopReason,
    pub(crate) triggered_limit: Option<NamedU64>,
    pub(crate) observed: Option<NamedU64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct NamedU64 {
    pub(crate) name: String,
    pub(crate) value: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct AssertionResult {
    pub(crate) assertion: TestAssertion,
    pub(crate) passed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TestConfig {
    pub(crate) firmware: PathBuf,
    pub(crate) system: Option<PathBuf>,
    pub(crate) script: PathBuf,
}

/// Resolved provenance for one node in an environment run. This is deliberately
/// not folded into [`TestConfig`]: a multi-node world has no meaningful single
/// `firmware` field, and emitting one would make a report look like a
/// single-machine result.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct EnvironmentNodeProvenance {
    pub(crate) id: String,
    pub(crate) system: PathBuf,
    pub(crate) firmware: PathBuf,
    pub(crate) system_hash: String,
    pub(crate) firmware_hash: String,
}

/// Provenance for a multi-node environment run.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct EnvironmentConfig {
    pub(crate) script: PathBuf,
    pub(crate) environment: PathBuf,
    /// SHA-256 identity of the sorted `(node id, firmware path, firmware
    /// content)` world topology. This lets CI compare a whole environment
    /// without inventing a misleading single-firmware field.
    pub(crate) world_firmware_hash: String,
    /// Sorted lexically by `id`, independent of manifest declaration order.
    pub(crate) nodes: Vec<EnvironmentNodeProvenance>,
}

/// Report-compatible result for a multi-node environment run.
///
/// The outer fields deliberately match [`TestResult`] so the released Action
/// report renderer has one stable result contract. The config shape is explicit
/// and environment-specific rather than pretending a world has one firmware.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct EnvironmentTestResult {
    pub(crate) result_schema_version: String,
    /// Explicitly distinguishes the environment result union arm from the
    /// single-machine v1.0 result contract.
    pub(crate) run_type: String,
    pub(crate) status: String,
    pub(crate) steps_executed: u64,
    pub(crate) cycles: u64,
    pub(crate) instructions: u64,
    pub(crate) stop_reason: StopReason,
    pub(crate) stop_reason_details: StopReasonDetails,
    pub(crate) limits: TestLimits,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
    pub(crate) assertions: Vec<AssertionResult>,
    /// Structured model-fidelity gaps observed across the world run. The
    /// monitor is thread-local, so the environment runner drains it before it
    /// writes artifacts just as the single-machine runner does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) fidelity: Vec<labwired_core::fidelity::FidelityGap>,
    pub(crate) config: EnvironmentConfig,
}

/// One final machine state in an environment snapshot.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct EnvironmentNodeSnapshot {
    pub(crate) id: String,
    /// The node-local final cycle count. The environment-level snapshot cycle
    /// count remains the world maximum for limit/reporting compatibility.
    pub(crate) cycles: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) state: Option<labwired_core::snapshot::MachineSnapshot>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PeripheralSnapshot {
    pub(crate) name: String,
    base: u64,
    size: u64,
    irq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) state: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct InteractiveSnapshotConfig {
    pub(crate) firmware: PathBuf,
    pub(crate) system: Option<PathBuf>,
    pub(crate) max_steps: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Snapshot {
    Standard {
        cpu: CpuSnapshot,
        steps_executed: u64,
        cycles: u64,
        instructions: u64,
        stop_reason: StopReason,
        stop_reason_details: StopReasonDetails,
        limits: TestLimits,
        firmware_hash: String,
        config: TestConfig,
    },
    ConfigError {
        message: String,
        stop_reason_details: StopReasonDetails,
        limits: TestLimits,
        config: TestConfig,
    },
    /// Multi-node state, used for both completed environment runs and their
    /// configuration failures. A config error before a world can be built has
    /// an empty `nodes` vector, but still carries environment-shaped provenance.
    Environment {
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        steps_executed: u64,
        cycles: u64,
        instructions: u64,
        stop_reason: StopReason,
        stop_reason_details: StopReasonDetails,
        limits: TestLimits,
        config: EnvironmentConfig,
        nodes: Vec<EnvironmentNodeSnapshot>,
    },
    Interactive {
        snapshot_schema_version: String,
        status: String,
        steps_executed: u64,
        cycles: u64,
        instructions: u64,
        stop_reason: StopReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        firmware_hash: String,
        cpu: CpuSnapshot,
        peripherals: Vec<PeripheralSnapshot>,
        config: InteractiveSnapshotConfig,
    },
}

// snapshot_cortexm_cpu removed, use cpu.snapshot() directly

pub(crate) struct InteractiveSnapshotInputs<'a> {
    pub(crate) firmware_path: &'a Path,
    pub(crate) system_path: Option<&'a PathBuf>,
    pub(crate) max_steps: usize,
    pub(crate) steps_executed: u64,
    pub(crate) stop_reason: StopReason,
    pub(crate) message: Option<String>,
}

pub(crate) fn write_interactive_snapshot<C: labwired_core::Cpu>(
    path: &Path,
    metrics: &labwired_core::metrics::PerformanceMetrics,
    machine: &labwired_core::Machine<C>,
    inputs: InteractiveSnapshotInputs<'_>,
) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            error!("Failed to create snapshot parent dir {:?}: {}", parent, e);
            return;
        }
    }

    let firmware_hash = match std::fs::read(inputs.firmware_path) {
        Ok(bytes) => {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            format!("{:x}", hasher.finalize())
        }
        Err(e) => {
            error!(
                "Failed to read firmware for snapshot hash {:?}: {}",
                inputs.firmware_path, e
            );
            String::new()
        }
    };

    let machine_snapshot = machine.snapshot();
    let peripherals = machine
        .bus
        .peripherals
        .iter()
        .map(|p| {
            let state = machine_snapshot.peripherals.get(&p.name).cloned();
            PeripheralSnapshot {
                name: p.name.clone(),
                base: p.base,
                size: p.size,
                irq: p.irq,
                state,
            }
        })
        .collect::<Vec<_>>();

    let cpu_snapshot = machine.cpu.snapshot();

    let snapshot = Snapshot::Interactive {
        snapshot_schema_version: "1.0".to_string(),
        status: if matches!(
            inputs.stop_reason,
            StopReason::MemoryViolation | StopReason::DecodeError
        ) {
            "error".to_string()
        } else {
            "ok".to_string()
        },
        steps_executed: inputs.steps_executed,
        cycles: metrics.get_cycles(),
        instructions: metrics.get_instructions(),
        stop_reason: inputs.stop_reason,
        message: inputs.message,
        firmware_hash,
        cpu: cpu_snapshot,
        peripherals,
        config: InteractiveSnapshotConfig {
            firmware: inputs.firmware_path.to_path_buf(),
            system: inputs.system_path.cloned(),
            max_steps: inputs.max_steps,
        },
    };

    match std::fs::File::create(path) {
        Ok(f) => {
            if let Err(e) = serde_json::to_writer_pretty(f, &snapshot) {
                error!("Failed to write snapshot {:?}: {}", path, e);
            }
        }
        Err(e) => error!("Failed to create snapshot {:?}: {}", path, e),
    }
}
