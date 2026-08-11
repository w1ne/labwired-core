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

/// Per-stimulus outcome string for [`StimulusOutcome::outcome`]. These are the
/// three — and only three — fates a declarative stimulus can meet, and the
/// consumer MUST be able to tell them apart: "the input was never delivered" and
/// "the input was delivered and the firmware ignored it" are completely
/// different bugs, and before this block existed they looked identical (both a
/// green `status: "pass"` with the failure only ever reaching stderr).
pub(crate) const STIMULUS_APPLIED: &str = "applied";
/// The engine refused the stimulus (`set_input`/`set_input_on` returned `Err`):
/// unknown channel, unknown component, out of range, or ambiguous. The input
/// NEVER reached the device, so the run proved nothing about it. Fatal — see
/// `execute_test_loop`.
pub(crate) const STIMULUS_REJECTED: &str = "rejected";
/// The run ended before an `after_cycles` trigger's threshold was reached, so
/// the stimulus never fired. Also proves nothing about that input, but NOT
/// fatal: unlike a rejection this is a pacing question (a run may legitimately
/// stop early on `stop_when_assertions_pass`), and turning it fatal would flip
/// existing green runs red on a judgement call. It is reported instead.
pub(crate) const STIMULUS_NOT_REACHED: &str = "not_reached";

/// What actually became of one declarative input stimulus.
///
/// Emitted for EVERY stimulus a script declared, in `TestResult::stimuli`. This
/// exists because the runner used to only `error!` a failed stimulus into the
/// log and then carry on to report `status: "pass"` — the most expensive bug
/// class here, a surface that reports success having proved nothing.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct StimulusOutcome {
    /// The `sim_input` channel key the script asked to drive.
    pub(crate) channel: String,
    /// The disambiguating component the script named, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) component: Option<String>,
    /// The requested value, in the channel's engineering unit.
    pub(crate) value: f64,
    /// The declared trigger, echoed verbatim so the reader can see the pacing
    /// that produced a `not_reached`.
    pub(crate) trigger: labwired_config::FaultTrigger,
    /// One of [`STIMULUS_APPLIED`], [`STIMULUS_REJECTED`], [`STIMULUS_NOT_REACHED`].
    pub(crate) outcome: String,
    /// Engine cycle at which the stimulus was applied (or at which the run
    /// ended, for a `not_reached`).
    pub(crate) at_cycle: u64,
    /// The engine's rejection reason, present iff `outcome` is
    /// [`STIMULUS_REJECTED`] or [`STIMULUS_NOT_REACHED`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

impl StimulusOutcome {
    pub(crate) fn is_rejected(&self) -> bool {
        self.outcome == STIMULUS_REJECTED
    }

    /// One-line human rendering used in the run-level `message` and the log.
    pub(crate) fn describe(&self) -> String {
        let target = match &self.component {
            Some(c) => format!("{}.{}", c, self.channel),
            None => self.channel.clone(),
        };
        match &self.error {
            Some(e) => format!("{} = {}: {}", target, self.value, e),
            None => format!("{} = {}", target, self.value),
        }
    }
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
    /// The exit code the firmware itself reported through the `simctl` device.
    ///
    /// Present only when `stop_reason` is `firmware_exit` **and** the firmware
    /// named a code: `EXIT n` gives `n`, `ABRT` gives 1, and a bare `STOP` —
    /// which makes no pass/fail claim — omits the field entirely rather than
    /// reporting 0, so "the firmware stopped" can never be read as a pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) firmware_exit_code: Option<u32>,
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
    /// What became of every declarative input stimulus the script declared —
    /// applied, rejected by the engine, or never reached. Absent (and omitted)
    /// when the script declared none, so runs that never used the feature keep
    /// a byte-identical `result.json` (the release-contract golden reference
    /// and `tests/determinism.rs` both depend on that).
    ///
    /// A `rejected` entry means the input NEVER reached the device, which makes
    /// the run's verdict meaningless for that input; the runner therefore fails
    /// the run (`status: "error"`, exit `EXIT_CONFIG_ERROR`) and repeats the
    /// reason in the top-level `message`. Ordering is stable: `at_start`
    /// stimuli in script order, then time-triggered ones in firing order, then
    /// the ones that never fired.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) stimuli: Vec<StimulusOutcome>,
    /// ELF Berkeley-style flash/RAM footprint (text/data/bss). Absent when
    /// footprint was not computed for this run (e.g. config error, or not yet
    /// wired). Optional totals/pct fields omitted when device limits unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) footprint: Option<FootprintReport>,
    /// Main-stack paint / high-water report. Absent when not collected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) memory: Option<labwired_core::stack_paint::MainStackReport>,
    /// Always-on cheap execution metrics (cycles, bus accesses, PC samples).
    /// Present on successful machine runs; omitted on config-error paths that
    /// never built a machine. Top-level `cycles` / `instructions` /
    /// `steps_executed` remain for compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metrics: Option<ExecutionMetrics>,
}

/// Industry-standard execution counters for `result.json` (`metrics`).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct ExecutionMetrics {
    pub cycles: u64,
    pub instructions: u64,
    pub steps_executed: u64,
    pub memory_reads: u64,
    pub memory_writes: u64,
    pub peripheral_accesses: u64,
    /// Best-effort: counts `SimulationError::ExceptionRaised` stop paths in P1.
    /// Handled NVIC/exception entries that do not fault the run are not counted.
    pub exceptions: u64,
    /// Top PC histogram samples (descending by count). Empty when no samples.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pc_samples: Vec<PcSample>,
}

/// One hot PC from statistical sampling during the test loop.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct PcSample {
    pub pc: u64,
    pub count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

/// Berkeley-style firmware footprint for `result.json` (`footprint`).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct FootprintReport {
    pub method: String,
    pub text_bytes: u64,
    pub data_bytes: u64,
    pub bss_bytes: u64,
    pub flash_used_bytes: u64,
    pub ram_static_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flash_total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flash_used_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_static_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Percent used of total, half-up to 2 decimal places. Returns 0.0 if total is 0.
pub(crate) fn pct2(used: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let v = (used as f64) * 100.0 / (total as f64);
    (v * 100.0).round() / 100.0
}

/// Build a [`FootprintReport`] from loader ELF section totals and optional
/// device flash/RAM capacities.
pub(crate) fn footprint_from_elf_totals(
    totals: &labwired_loader::ElfSectionTotals,
    flash_total: Option<u64>,
    ram_total: Option<u64>,
) -> FootprintReport {
    let flash_used = totals.flash_used();
    let ram_static = totals.ram_static();
    FootprintReport {
        method: labwired_loader::FOOTPRINT_METHOD.to_string(),
        text_bytes: totals.text,
        data_bytes: totals.data,
        bss_bytes: totals.bss,
        flash_used_bytes: flash_used,
        ram_static_bytes: ram_static,
        flash_total_bytes: flash_total,
        ram_total_bytes: ram_total,
        flash_used_pct: flash_total.map(|t| pct2(flash_used, t)),
        ram_static_pct: ram_total.map(|t| pct2(ram_static, t)),
        notes: Vec::new(),
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) evidence: Option<AssertionEvidence>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AssertionEvidence {
    ShutdownLatency {
        stimulus_cycle: u64,
        token_cycle: u64,
        latency_cycles: u64,
        configured_max_cycles: u64,
    },
    ResourceBudget {
        name: String,
        measured: Option<u64>,
        limit: u64,
        method: String,
    },
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
