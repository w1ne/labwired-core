// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The manifest-facing half of the in-core analog engine: config keys, routed
//! inputs, probes, and the [`CosimAdapter`] implementation.
//!
//! ```yaml
//! cosim_models:
//!   - id: rc
//!     adapter: analog
//!     step_ns: 100000
//!     inputs:  { gpio: board.gpio.pa5 }
//!     outputs: { v_out: board.analog.pa0_volts }
//!     config:
//!       netlist: ./rc.cir          # or inline: netlist_text: |
//!       vdd: 3.3
//!       substeps: 10
//!       integration: be            # be | trap
//!       probes:  { v_out: "v(out)" }
//!       sources: { gpio: Vgpio }
//!       trace:   ["v(in)", "i(Vgpio)"]
//!       trace_samples: 20000
//! ```
//!
//! `netlist`, `vdd`, `probes` and `sources` are spelled exactly as the ngspice
//! wrapper spells them, so a manifest switches engines by changing `adapter:`
//! and nothing else.
//!
//! ## Step semantics
//!
//! [`CosimStep::time_ns`] is the END of the interval being simulated, the same
//! convention `tools/cosim/labwired_ngspice.py` follows. Inputs are applied
//! first and hold for the whole interval, then the solver integrates from the
//! previous boundary to `time_ns` in `substeps` equal internal steps. The
//! operating point is solved once at construction from the netlist's own DC
//! values, before any input exists — a pull-up sits at Vdd and a GPIO at 0,
//! exactly as on a board that has just come out of reset.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_yaml::Value;

use crate::cosim::{CosimAdapter, CosimInputKind, CosimSignalValue, CosimStep, CosimStepResult};
use crate::{SimResult, SimulationError};

use super::mna::Solver;
use super::netlist::{parse_netlist, AnalogError, NodeRef};
use super::trace::{AnalogChannel, AnalogTrace, AnalogTraceHandle};

pub use super::mna::Integration;

/// What one probe expression reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Probe {
    /// `v(node)` or a bare node name.
    NodeVoltage(NodeRef),
    /// `i(element)` — a voltage source's or inductor's branch current.
    BranchCurrent(usize),
}

impl Probe {
    fn unit(&self) -> &'static str {
        match self {
            Self::NodeVoltage(_) => "V",
            Self::BranchCurrent(_) => "A",
        }
    }
}

/// The `config:` block of an `adapter: analog` model.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalogConfig {
    /// Volts a boolean input maps to.
    pub vdd: f64,
    /// Internal solver steps per co-simulation step.
    pub substeps: u32,
    /// Integration rule.
    pub integration: Integration,
    /// LabWired output name → node or branch expression.
    pub probes: BTreeMap<String, String>,
    /// LabWired input name → circuit element (`V`, `I` or `S`).
    pub sources: BTreeMap<String, String>,
    /// Extra trace-only expressions, beyond the routed outputs.
    pub trace: Vec<String>,
    /// Ring depth for the oscilloscope trace.
    pub trace_samples: usize,
}

impl Default for AnalogConfig {
    fn default() -> Self {
        Self {
            vdd: 3.3,
            substeps: 10,
            integration: Integration::BackwardEuler,
            probes: BTreeMap::new(),
            sources: BTreeMap::new(),
            trace: Vec::new(),
            trace_samples: super::trace::DEFAULT_TRACE_SAMPLES,
        }
    }
}

impl AnalogConfig {
    /// Read the config keys out of a manifest `config:` mapping.
    pub fn from_yaml(config: &HashMap<String, Value>) -> Result<Self, AnalogError> {
        let mut parsed = Self::default();

        if let Some(value) = config.get("vdd") {
            parsed.vdd = value
                .as_f64()
                .or_else(|| value.as_i64().map(|v| v as f64))
                .ok_or_else(|| AnalogError::Config("config.vdd must be a number".to_string()))?;
        }
        if let Some(value) = config.get("substeps") {
            let substeps = value.as_u64().ok_or_else(|| {
                AnalogError::Config("config.substeps must be a positive integer".to_string())
            })?;
            if substeps == 0 || substeps > u32::MAX as u64 {
                return Err(AnalogError::Config(
                    "config.substeps must be a positive integer".to_string(),
                ));
            }
            parsed.substeps = substeps as u32;
        }
        if let Some(value) = config.get("integration") {
            let text = value.as_str().ok_or_else(|| {
                AnalogError::Config("config.integration must be `be` or `trap`".to_string())
            })?;
            parsed.integration = match text.trim().to_ascii_lowercase().as_str() {
                "be" | "backward_euler" | "euler" => Integration::BackwardEuler,
                "trap" | "trapezoidal" => Integration::Trapezoidal,
                other => {
                    return Err(AnalogError::Config(format!(
                        "config.integration `{other}` is not one of `be`, `trap`"
                    )))
                }
            };
        }
        if let Some(value) = config.get("probes") {
            parsed.probes = string_map(value, "config.probes")?;
        }
        if let Some(value) = config.get("sources") {
            parsed.sources = string_map(value, "config.sources")?;
        }
        if let Some(value) = config.get("trace") {
            let list = value.as_sequence().ok_or_else(|| {
                AnalogError::Config("config.trace must be a list of expressions".to_string())
            })?;
            parsed.trace = list
                .iter()
                .map(|item| {
                    item.as_str().map(str::to_string).ok_or_else(|| {
                        AnalogError::Config("config.trace entries must be strings".to_string())
                    })
                })
                .collect::<Result<_, _>>()?;
        }
        if let Some(value) = config.get("trace_samples") {
            let samples = value.as_u64().ok_or_else(|| {
                AnalogError::Config("config.trace_samples must be a positive integer".to_string())
            })?;
            if samples == 0 {
                return Err(AnalogError::Config(
                    "config.trace_samples must be a positive integer".to_string(),
                ));
            }
            parsed.trace_samples = samples as usize;
        }

        Ok(parsed)
    }
}

fn string_map(value: &Value, what: &str) -> Result<BTreeMap<String, String>, AnalogError> {
    let mapping = value
        .as_mapping()
        .ok_or_else(|| AnalogError::Config(format!("{what} must be a mapping")))?;
    let mut out = BTreeMap::new();
    for (key, item) in mapping {
        let key = key
            .as_str()
            .ok_or_else(|| AnalogError::Config(format!("{what} keys must be strings")))?;
        let item = item
            .as_str()
            .ok_or_else(|| AnalogError::Config(format!("{what}.{key} must be a string")))?;
        out.insert(key.to_string(), item.to_string());
    }
    Ok(out)
}

/// What a routed input drives.
#[derive(Debug, Clone, PartialEq)]
enum InputTarget {
    VoltageSource(usize),
    CurrentSource(usize),
    /// Every switch sharing one control name.
    Switches(Vec<usize>),
}

/// The in-core analog engine as a co-simulation adapter.
#[derive(Debug)]
pub struct AnalogCosimAdapter {
    solver: Solver,
    vdd: f64,
    substeps: u32,
    /// Routed outputs, in probe-name order.
    probes: Vec<(String, Probe)>,
    /// Trace-only expressions, in declaration order.
    trace_extra: Vec<(String, Probe)>,
    inputs: BTreeMap<String, InputTarget>,
    trace: AnalogTraceHandle,
    channel_base: usize,
    channel_prefix: String,
    trace_samples: usize,
    time_ns: u64,
    step_index: u64,
    sample_scratch: Vec<f64>,
}

impl AnalogCosimAdapter {
    /// Parse `netlist`, build the solver, resolve probes and sources, solve the
    /// operating point and seed the trace with it.
    pub fn from_netlist(netlist: &str, cfg: &AnalogConfig) -> Result<Self, AnalogError> {
        let circuit = parse_netlist(netlist)?;
        let solver = Solver::new(circuit, cfg.integration)?;

        let mut probes = Vec::with_capacity(cfg.probes.len());
        for (name, expression) in &cfg.probes {
            probes.push((name.clone(), resolve_probe(&solver, expression)?));
        }
        let mut trace_extra = Vec::with_capacity(cfg.trace.len());
        for expression in &cfg.trace {
            trace_extra.push((expression.clone(), resolve_probe(&solver, expression)?));
        }

        let mut inputs = BTreeMap::new();
        for (input, element) in &cfg.sources {
            let target = if let Some(index) = solver.voltage_source_index(element) {
                // A source that carries SIN()/PULSE() is driven by the clock.
                // Letting a routed input write to it too would give one element
                // two owners, and which one you saw would depend on the order
                // the co-simulation happened to call us in.
                if !solver.circuit().voltage_sources[index].wave.is_constant() {
                    return Err(AnalogError::Config(format!(
                        "config.sources.{input} routes an input to `{element}`, which the \
                         netlist already drives with a transient function; give `{element}` a \
                         plain `dc` value or route the input somewhere else"
                    )));
                }
                InputTarget::VoltageSource(index)
            } else if let Some(index) = solver.current_source_index(element) {
                if !solver.circuit().current_sources[index].wave.is_constant() {
                    return Err(AnalogError::Config(format!(
                        "config.sources.{input} routes an input to `{element}`, which the \
                         netlist already drives with a transient function; give `{element}` a \
                         plain `dc` value or route the input somewhere else"
                    )));
                }
                InputTarget::CurrentSource(index)
            } else if let Some(index) = solver.switch_index(element) {
                InputTarget::Switches(vec![index])
            } else {
                return Err(AnalogError::Config(format!(
                    "config.sources.{input} names `{element}`, which the netlist does not declare \
                     as a V, I or S element"
                )));
            };
            inputs.insert(input.clone(), target);
        }
        // A switch's `<ctrl>` is itself a routed input name, so it needs no
        // `sources:` entry. An explicit entry wins if a manifest declares both.
        for switch in &solver.circuit().switches {
            let indices = solver.switches_controlled_by(&switch.ctrl);
            inputs
                .entry(switch.ctrl.clone())
                .or_insert(InputTarget::Switches(indices));
        }

        let sample_width = probes.len() + trace_extra.len();
        let mut adapter = Self {
            solver,
            vdd: cfg.vdd,
            substeps: cfg.substeps,
            probes,
            trace_extra,
            inputs,
            trace: Arc::new(Mutex::new(AnalogTrace::new(cfg.trace_samples))),
            channel_base: 0,
            channel_prefix: String::new(),
            trace_samples: cfg.trace_samples,
            time_ns: 0,
            step_index: 0,
            sample_scratch: vec![0.0; sample_width],
        };
        adapter.register_channels();
        adapter.record_sample();
        Ok(adapter)
    }

    /// Build from a manifest entry, resolving a relative `config.netlist`
    /// against the manifest's directory.
    pub fn from_manifest_config(
        config: &labwired_config::CosimModelConfig,
        base_dir: &Path,
    ) -> Result<Self, AnalogError> {
        let netlist = netlist_source(&config.config, base_dir)?;
        let cfg = AnalogConfig::from_yaml(&config.config)?;
        if cfg.probes.is_empty() {
            return Err(AnalogError::Config(
                "config.probes is required: an analog model with no probe produces no outputs"
                    .to_string(),
            ));
        }
        Self::from_netlist(&netlist, &cfg)
    }

    /// The channel table this adapter contributes, prefix included.
    pub fn channels(&self) -> Vec<AnalogChannel> {
        self.probes
            .iter()
            .chain(self.trace_extra.iter())
            .map(|(name, probe)| AnalogChannel {
                name: format!("{}{name}", self.channel_prefix),
                unit: probe.unit().to_string(),
            })
            .collect()
    }

    /// Handle to the ring this adapter appends to.
    pub fn trace_handle(&self) -> AnalogTraceHandle {
        Arc::clone(&self.trace)
    }

    /// Simulated time of the last completed step, in nanoseconds.
    pub fn time_ns(&self) -> u64 {
        self.time_ns
    }

    /// The solver, for inspection in tests and diagnostics.
    pub fn solver(&self) -> &Solver {
        &self.solver
    }

    fn register_channels(&mut self) {
        let channels = self.channels();
        let mut trace = lock(&self.trace);
        self.channel_base = trace.register_channels(&channels);
    }

    fn read_probe(&self, probe: &Probe) -> f64 {
        match probe {
            Probe::NodeVoltage(node) => self.solver.node_voltage(*node),
            Probe::BranchCurrent(index) => self.solver.branch_current(*index),
        }
    }

    fn record_sample(&mut self) {
        let mut scratch = std::mem::take(&mut self.sample_scratch);
        for (slot, (_, probe)) in self
            .probes
            .iter()
            .chain(self.trace_extra.iter())
            .enumerate()
        {
            scratch[slot] = self.read_probe(probe);
        }
        {
            let mut trace = lock(&self.trace);
            trace.push(self.channel_base, self.step_index, self.time_ns, &scratch);
        }
        self.sample_scratch = scratch;
    }

    fn apply_input(&mut self, name: &str, value: &CosimSignalValue) -> SimResult<()> {
        let Some(target) = self.inputs.get(name).cloned() else {
            // Routed into the model but not a circuit source: ignore, exactly
            // like the ngspice wrapper. Manifests routinely route one signal
            // bundle into several models.
            return Ok(());
        };
        match target {
            InputTarget::VoltageSource(index) => {
                let volts = self.numeric(name, value, "volts")?;
                self.solver.set_voltage_source(index, volts);
            }
            InputTarget::CurrentSource(index) => {
                if matches!(value, CosimSignalValue::Bool(_)) {
                    return Err(SimulationError::Other(format!(
                        "analog input `{name}` drives a current source; a boolean has no \
                         current to map to — route a number in amps"
                    )));
                }
                let amps = self.numeric(name, value, "amps")?;
                self.solver.set_current_source(index, amps);
            }
            InputTarget::Switches(indices) => {
                let closed = match value {
                    CosimSignalValue::Bool(state) => *state,
                    other => {
                        // A numeric control is read as a logic level against
                        // half of Vdd, so an analog signal can gate a switch.
                        self.numeric(name, other, "volts")? >= self.vdd / 2.0
                    }
                };
                for index in indices {
                    self.solver.set_switch(index, closed);
                }
            }
        }
        Ok(())
    }

    fn numeric(&self, name: &str, value: &CosimSignalValue, unit: &str) -> SimResult<f64> {
        match value {
            CosimSignalValue::Bool(true) => Ok(self.vdd),
            CosimSignalValue::Bool(false) => Ok(0.0),
            CosimSignalValue::F64(value) => Ok(*value),
            CosimSignalValue::I64(value) => Ok(*value as f64),
            CosimSignalValue::Text(text) => text.trim().parse::<f64>().map_err(|_| {
                SimulationError::Other(format!(
                    "analog input `{name}`: cannot read `{text}` as {unit}"
                ))
            }),
        }
    }
}

impl CosimAdapter for AnalogCosimAdapter {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        for (name, value) in &step.inputs {
            self.apply_input(name, value)?;
        }

        // `time_ns` is the END of the step. A boundary already passed (a model
        // whose `step_ns` divides another's) re-reads the probes rather than
        // integrating backwards.
        if step.time_ns > self.time_ns {
            let span = (step.time_ns - self.time_ns) as f64 * 1e-9;
            let h = span / f64::from(self.substeps);
            for _ in 0..self.substeps {
                self.solver
                    .advance(h)
                    .map_err(|err| SimulationError::Other(err.to_string()))?;
            }
            self.time_ns = step.time_ns;
        }

        self.step_index += 1;
        self.record_sample();

        let mut outputs = BTreeMap::new();
        for (name, probe) in &self.probes {
            outputs.insert(name.clone(), CosimSignalValue::F64(self.read_probe(probe)));
        }
        Ok(CosimStepResult { outputs })
    }

    /// A switch control is a logic level; a voltage or current source takes
    /// its number as given. An input the circuit does not use has no kind.
    fn input_kind(&self, name: &str) -> Option<CosimInputKind> {
        Some(match self.inputs.get(name)? {
            InputTarget::Switches(_) => CosimInputKind::Bool,
            InputTarget::VoltageSource(_) | InputTarget::CurrentSource(_) => CosimInputKind::Number,
        })
    }

    fn attach_analog_trace(&mut self, trace: &AnalogTraceHandle, channel_prefix: &str) {
        self.channel_prefix = channel_prefix.to_string();
        self.trace = Arc::clone(trace);
        {
            let mut shared = lock(&self.trace);
            shared.reserve_capacity(self.trace_samples);
        }
        self.register_channels();
        // Seed the shared ring with the state this adapter is already in, so
        // the run's first row is the operating point rather than the first step.
        self.record_sample();
    }
}

fn lock(trace: &AnalogTraceHandle) -> std::sync::MutexGuard<'_, AnalogTrace> {
    match trace.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Resolve `v(out)`, `out`, `V(OUT)` or `i(Vgpio)` against a circuit.
///
/// Same spelling as the ngspice wrapper's probe map, so one manifest works with
/// either engine. An unknown node or element is an error at build time: a probe
/// that silently reads zero draws a confident flat line of the wrong signal.
pub fn resolve_probe(solver: &Solver, expression: &str) -> Result<Probe, AnalogError> {
    let text = expression.trim();
    let circuit = solver.circuit();

    if let Some(inner) = strip_call(text, 'v') {
        return circuit.node(inner).map(Probe::NodeVoltage).ok_or_else(|| {
            AnalogError::Config(format!(
                "probe `{expression}`: no node `{inner}` in the netlist"
            ))
        });
    }
    if let Some(inner) = strip_call(text, 'i') {
        return circuit
            .branch_index(inner)
            .map(Probe::BranchCurrent)
            .ok_or_else(|| {
                AnalogError::Config(format!(
                    "probe `{expression}`: `{inner}` is not a voltage source or inductor; only \
                     those carry a branch current"
                ))
            });
    }
    circuit.node(text).map(Probe::NodeVoltage).ok_or_else(|| {
        AnalogError::Config(format!(
            "probe `{expression}`: no node `{text}` in the netlist"
        ))
    })
}

fn strip_call(text: &str, letter: char) -> Option<&str> {
    let mut chars = text.chars();
    let first = chars.next()?;
    if !first.eq_ignore_ascii_case(&letter) {
        return None;
    }
    text[first.len_utf8()..]
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
}

/// The netlist text for a manifest entry: inline `netlist_text`, or the file
/// named by `netlist` resolved against the manifest's directory.
pub fn netlist_source(
    config: &HashMap<String, Value>,
    base_dir: &Path,
) -> Result<String, AnalogError> {
    if let Some(value) = config.get("netlist_text") {
        return value.as_str().map(str::to_string).ok_or_else(|| {
            AnalogError::Config("config.netlist_text must be a string".to_string())
        });
    }
    let Some(value) = config.get("netlist") else {
        return Err(AnalogError::Config(
            "requires config.netlist (a path) or config.netlist_text (inline)".to_string(),
        ));
    };
    let path = value
        .as_str()
        .ok_or_else(|| AnalogError::Config("config.netlist must be a path".to_string()))?;
    let resolved = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        base_dir.join(path)
    };
    std::fs::read_to_string(&resolved).map_err(|err| {
        AnalogError::Config(format!(
            "cannot read config.netlist {}: {err}",
            resolved.display()
        ))
    })
}
