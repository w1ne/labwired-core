// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

mod analog_source;
mod behavior;
mod external_process;
mod gpio_device;
mod onewire_device;
pub(crate) use behavior::{build_part_adapter, catalog_descriptor};
mod registry;
pub mod routing;
pub mod shm;

// `adapter: ngspice`: the contract shared by the browser and native hosts.
pub mod ngspice;

pub use external_process::{ExternalProcessCosimAdapter, ExternalProcessCosimOptions};
pub use registry::{
    build_cosim_adapter, build_cosim_adapter_with_base, validate_analog_models, CosimModelStep,
    CosimRoutedModelStep, CosimRunner, CosimRunnerModel,
};

pub use crate::analog::{
    AnalogChannel, AnalogCosimAdapter, AnalogSample, AnalogTrace, AnalogTraceBatch,
    AnalogTraceHandle, AnalogTraceRegistry,
};
pub use routing::{
    CosimAdvance, CosimAdvanceError, CosimSession, InputThresholds, RoutingError, SignalPath,
    SignalRouter, UI_SIGNAL_PREFIX,
};

use crate::{Peripheral, PeripheralTickResult, SimResult};
use serde::Serialize;
use std::any::Any;
use std::collections::BTreeMap;

/// The exact source an adapter loaded for a model, so a run can be pinned to
/// the circuit it actually solved.
///
/// The manifest hash covers the manifest text. An external netlist lives in a
/// second file, and editing that file must change something a run records.
/// Adapters that read a source report it here; adapters that own no external
/// source leave [`CosimAdapter::source_pin`] at its `None` default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CosimSourcePin {
    /// What the source is. Today the only value is `spice_netlist`.
    pub kind: &'static str,
    /// Where the bytes came from: `file` or `inline`.
    pub origin: &'static str,
    /// The source's size in bytes.
    pub bytes: usize,
    /// SHA-256 of the exact bytes the adapter parsed.
    pub sha256: String,
    /// The path as the manifest declared it, for a file source. Absent inline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl CosimSourcePin {
    /// A SPICE netlist read from a file the manifest names.
    pub fn spice_netlist_file(path: impl Into<String>, netlist: &str) -> Self {
        Self {
            kind: "spice_netlist",
            origin: "file",
            bytes: netlist.len(),
            sha256: netlist_sha256(netlist),
            path: Some(path.into()),
        }
    }

    /// A SPICE netlist carried inline in the manifest.
    pub fn spice_netlist_inline(netlist: &str) -> Self {
        Self {
            kind: "spice_netlist",
            origin: "inline",
            bytes: netlist.len(),
            sha256: netlist_sha256(netlist),
            path: None,
        }
    }
}

fn netlist_sha256(netlist: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(netlist.as_bytes()))
}

/// Scalar value exchanged at a co-simulation boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum CosimSignalValue {
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
}

impl std::fmt::Display for CosimSignalValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(value) => write!(f, "{value}"),
            Self::I64(value) => write!(f, "{value}"),
            Self::F64(value) => write!(f, "{value}"),
            Self::Text(value) => write!(f, "{value}"),
        }
    }
}

pub type CosimSignals = BTreeMap<String, CosimSignalValue>;

/// The value shape a model input expects, as far as its adapter can say.
///
/// Signals set from outside the engine arrive as plain numbers (a canvas press
/// is `1`, a test stimulus is `value: 1`), and the same number means different
/// things to different inputs: to a switch control it is a logic level, to a
/// voltage source it is volts. The adapter that owns the input is the only
/// party that knows which, so it answers through
/// [`CosimAdapter::input_kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CosimInputKind {
    /// A logic level: 0 is false, anything else true.
    Bool,
    /// A number taken as given (volts, amps, …).
    Number,
}

/// One deterministic handoff from LabWired into an external model.
#[derive(Debug, Clone, PartialEq)]
pub struct CosimStep {
    pub time_ns: u64,
    pub dt_ns: u64,
    pub inputs: CosimSignals,
}

/// Outputs produced by an external model after a co-simulation step.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CosimStepResult {
    pub outputs: CosimSignals,
}

/// Runtime contract implemented by external-process, FMI, or in-process
/// adapters. LabWired owns time advancement; adapters consume one bounded
/// step and return observable values.
pub trait CosimAdapter: Send {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult>;

    /// [`Self::step`] over borrowed buffers: the runner's hot path. `outputs`
    /// holds the adapter's previous answer and must be left holding exactly
    /// this step's. The default clones `inputs` into a [`CosimStep`]; an
    /// adapter that wants a step without allocation overrides it.
    fn step_into(
        &mut self,
        time_ns: u64,
        dt_ns: u64,
        inputs: &CosimSignals,
        outputs: &mut CosimSignals,
    ) -> SimResult<()> {
        let result = self.step(CosimStep {
            time_ns,
            dt_ns,
            inputs: inputs.clone(),
        })?;
        *outputs = result.outputs;
        Ok(())
    }

    /// Adopt the runner's shared analog-sample ring, naming this model's
    /// channels `<channel_prefix><name>`.
    ///
    /// The circuit solver records its probes, and registered analog-source
    /// adapters record their modeled scalar output. Discrete behavior adapters
    /// leave this empty: a Boolean signal alone is not an electrical waveform.
    /// The runner calls this on every model so each producer can publish into
    /// the existing oscilloscope ring.
    fn attach_analog_trace(&mut self, trace: &AnalogTraceHandle, channel_prefix: &str) {
        let _ = (trace, channel_prefix);
    }

    /// Optional initial value for a routed input whose physical default is not zero.
    fn input_initial_value(&self, name: &str) -> Option<CosimSignalValue> {
        let _ = name;
        None
    }

    /// Unit of a numeric input, when the adapter knows it. Values are passed
    /// without implicit conversion; Boolean voltage drives use their explicit
    /// adapter Vdd conversion instead.
    fn input_unit(&self, name: &str) -> Option<&str> {
        let _ = name;
        None
    }

    /// The value shape model input `name` expects, or `None` when this adapter
    /// cannot say (it ignores the input, or passes it to a process it does not
    /// understand). See [`CosimInputKind`].
    fn input_kind(&self, name: &str) -> Option<CosimInputKind> {
        let _ = name;
        None
    }

    /// The source bytes this adapter loaded, when it loaded any. A manifest's
    /// hash cannot cover an external netlist, so a reproducibility report asks
    /// the adapter that read it. Adapters with no such source return `None`.
    fn source_pin(&self) -> Option<CosimSourcePin> {
        None
    }

    /// What the model's part holds, as inspect artifacts: a part's state that
    /// is not a routed signal (a display's text, a sensor's memory). Most
    /// adapters publish everything as signals and leave this empty.
    fn artifacts(&self, id: &str) -> Vec<crate::inspect::Artifact> {
        let _ = id;
        Vec::new()
    }
}

/// Deterministic adapter used by tests and manifest dry-runs before a real
/// external simulator is wired in.
#[derive(Debug, Clone)]
pub struct StaticCosimAdapter {
    outputs: CosimSignals,
}

impl StaticCosimAdapter {
    pub fn new(outputs: CosimSignals) -> Self {
        Self { outputs }
    }
}

impl CosimAdapter for StaticCosimAdapter {
    fn step(&mut self, _step: CosimStep) -> SimResult<CosimStepResult> {
        Ok(CosimStepResult {
            outputs: self.outputs.clone(),
        })
    }
}

/// A peripheral that proxies its operations to an external process via IPC.
/// This is used for high-performance co-simulation with RTL models (e.g. Verilator).
#[derive(Debug)]
pub struct CosimPeripheral {
    pub name: String,
    // Add IPC transport (e.g. SharedMemory)
}

impl Peripheral for CosimPeripheral {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // TODO: Propose transaction over IPC and wait for result
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // TODO: Propose transaction over IPC
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // TODO: Sync simulation time with external process
        PeripheralTickResult::default()
    }

    // `needs_legacy_walk` is deliberately NOT overridden to `false` here, even
    // though today's `tick` is literally `PeripheralTickResult::default()`.
    // This type is scaffolding: nothing in the workspace constructs it, so a
    // `false` banks nothing on any bus — and the TODO above says the intended
    // body syncs simulation time with an external process, i.e. real walk work.
    // A `false` left standing would silently starve the peripheral of ticks the
    // moment someone fills that in. Per the `Peripheral::needs_legacy_walk`
    // contract, the honest direction under that doubt is to leave it `true`.

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}
