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
//!       input_sampling: previous_boundary # optional; legacy default: current_boundary
//!       probes:  { v_out: "v(out)" }
//!       sources: { gpio: Vgpio, temperature_c: Rntc }
//!       trace:   ["v(in)", "i(Vgpio)"]
//!       trace_samples: 20000
//! ```
//!
//! `netlist`, `vdd`, `probes` and `sources` share their spelling with the
//! ngspice wrapper. A routed resistor is an in-core extension: its numeric
//! input is resistance in ohms and requires a finite positive value.
//! A bounded beta-curve NTC can instead be configured as:
//!
//! ```yaml
//! thermistors:
//!   temperature:
//!     resistor: Rntc
//!     r_ref_ohms: 10000
//!     beta_kelvin: 3950
//!     reference_c: 25
//!     min_c: 0
//!     max_c: 85
//! ```
//!
//! The input is numeric Celsius and starts at `reference_c`. All parameters
//! and the valid temperature range must be supplied from the chosen part's
//! data. The initial netlist resistance must equal `r_ref_ohms`; a resistor
//! may have only one routed owner. This beta approximation does not model
//! tolerance, self-heating or behavior outside the declared range.
//!
//! A bounded power-law photoresistor (CdS cell) follows the same pattern:
//!
//! ```yaml
//! photoresistors:
//!   lux:
//!     resistor: Rldr
//!     r_ref_ohms: 15000
//!     reference_lux: 10
//!     gamma: 0.6
//!     min_lux: 1
//!     max_lux: 100
//! ```
//!
//! The input is numeric illuminance in lux and starts at `reference_lux`. The
//! resistance is `r_ref_ohms * (lux / reference_lux)^-gamma`, where `gamma` is
//! the data-sheet log-log slope. An input outside `min_lux..max_lux` is an
//! error, not an extrapolation; darkness (0 lx) is therefore outside every
//! valid range. The law is static: it does not model the cell's rise and decay
//! time, light history, spectral response, temperature or tolerance.
//!
//! ## Step semantics
//!
//! [`CosimStep::time_ns`] is the END of the interval being simulated, the same
//! convention `tools/cosim/labwired_ngspice.py` follows. The default
//! `input_sampling: current_boundary` applies supplied inputs before integrating
//! the preceding interval, preserving the existing CPU/world contract.
//! `input_sampling: previous_boundary` instead integrates with inputs held from
//! the previous sample, publishes that interval's endpoint, then latches the
//! supplied inputs for the next interval. A change between samples therefore
//! takes effect at the following sample; it cannot affect elapsed time.
//!
//! The operating point is solved once at construction from the netlist's DC
//! values. Previous-boundary sampling requires an explicit `(time_ns, dt_ns) =
//! (0, 0)` initialization whose inputs agree with those initial source, resistor
//! and open-switch values. An inconsistent initial value is an error, rather
//! than a changed source paired with a stale operating-point voltage.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_yaml::Value;

use crate::cosim::{
    CosimAdapter, CosimInputKind, CosimSignalValue, CosimSignals, CosimStep, CosimStepResult,
};
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
    /// `i(element)` — a voltage source, inductor, E or H branch current.
    BranchCurrent(usize),
    /// `i(diode)` — anode-to-cathode current at the solved junction voltage.
    DiodeCurrent(usize),
}

impl Probe {
    fn unit(&self) -> &'static str {
        match self {
            Self::NodeVoltage(_) => "V",
            Self::BranchCurrent(_) | Self::DiodeCurrent(_) => "A",
        }
    }
}

/// A bounded beta approximation for one physical NTC resistor.
/// The caller must supply coefficients and the range in which they apply.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThermistorConfig {
    pub resistor: String,
    pub r_ref_ohms: f64,
    pub beta_kelvin: f64,
    pub reference_c: f64,
    pub min_c: f64,
    pub max_c: f64,
}

impl ThermistorConfig {
    fn validate(&self) -> Result<(), AnalogError> {
        if self.resistor.trim().is_empty()
            || !self.r_ref_ohms.is_finite()
            || self.r_ref_ohms <= 0.0
            || !self.beta_kelvin.is_finite()
            || self.beta_kelvin <= 0.0
            || !self.reference_c.is_finite()
            || !self.min_c.is_finite()
            || !self.max_c.is_finite()
            || self.min_c <= -273.15
            || self.min_c > self.reference_c
            || self.reference_c > self.max_c
        {
            return Err(AnalogError::Config("thermistor requires positive finite coefficients and a valid temperature range containing the reference".to_string()));
        }
        self.resistance(self.min_c)?;
        self.resistance(self.max_c)?;
        Ok(())
    }

    fn resistance(&self, temperature_c: f64) -> Result<f64, AnalogError> {
        if !temperature_c.is_finite() || temperature_c < self.min_c || temperature_c > self.max_c {
            return Err(AnalogError::Config(format!(
                "thermistor temperature must be within {}..{} C",
                self.min_c, self.max_c
            )));
        }
        let ohms = self.r_ref_ohms
            * libm::exp(
                self.beta_kelvin
                    * (1.0 / (temperature_c + 273.15) - 1.0 / (self.reference_c + 273.15)),
            );
        if !ohms.is_finite() || ohms <= 0.0 || !ohms.recip().is_finite() {
            return Err(AnalogError::Config(
                "thermistor curve produces invalid resistance".to_string(),
            ));
        }
        Ok(ohms)
    }
}

/// A bounded power-law approximation for one physical photoresistor.
/// The caller must supply coefficients and the range in which they apply.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhotoresistorConfig {
    pub resistor: String,
    pub r_ref_ohms: f64,
    pub reference_lux: f64,
    pub gamma: f64,
    pub min_lux: f64,
    pub max_lux: f64,
}

impl PhotoresistorConfig {
    fn validate(&self) -> Result<(), AnalogError> {
        if self.resistor.trim().is_empty()
            || !self.r_ref_ohms.is_finite()
            || self.r_ref_ohms <= 0.0
            || !self.gamma.is_finite()
            || self.gamma <= 0.0
            || !self.reference_lux.is_finite()
            || !self.min_lux.is_finite()
            || !self.max_lux.is_finite()
            || self.min_lux <= 0.0
            || self.min_lux > self.reference_lux
            || self.reference_lux > self.max_lux
        {
            return Err(AnalogError::Config("photoresistor requires positive finite coefficients and a valid positive illuminance range containing the reference".to_string()));
        }
        self.resistance(self.min_lux)?;
        self.resistance(self.max_lux)?;
        Ok(())
    }

    fn resistance(&self, lux: f64) -> Result<f64, AnalogError> {
        if !lux.is_finite() || lux < self.min_lux || lux > self.max_lux {
            return Err(AnalogError::Config(format!(
                "photoresistor illuminance must be within {}..{} lx",
                self.min_lux, self.max_lux
            )));
        }
        let ohms = self.r_ref_ohms * libm::pow(lux / self.reference_lux, -self.gamma);
        if !ohms.is_finite() || ohms <= 0.0 || !ohms.recip().is_finite() {
            return Err(AnalogError::Config(
                "photoresistor curve produces invalid resistance".to_string(),
            ));
        }
        Ok(ohms)
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
    /// LabWired output name → node voltage or supported element-current expression.
    pub probes: BTreeMap<String, String>,
    /// LabWired input name → `V`, `I`, or `S`; an `R` accepts ohms, or Celsius when temperature-dependent.
    pub sources: BTreeMap<String, String>,
    /// Input name → explicitly bounded beta-curve thermistor.
    pub thermistors: BTreeMap<String, ThermistorConfig>,
    /// Input name → explicitly bounded power-law photoresistor.
    pub photoresistors: BTreeMap<String, PhotoresistorConfig>,
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
            thermistors: BTreeMap::new(),
            photoresistors: BTreeMap::new(),
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
        if let Some(value) = config.get("thermistors") {
            parsed.thermistors = serde_yaml::from_value(value.clone())
                .map_err(|error| AnalogError::Config(format!("config.thermistors: {error}")))?;
        }
        if let Some(value) = config.get("photoresistors") {
            parsed.photoresistors = serde_yaml::from_value(value.clone())
                .map_err(|error| AnalogError::Config(format!("config.photoresistors: {error}")))?;
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
    Resistance(usize),
    Thermistor(usize, ThermistorConfig),
    Photoresistor(usize, PhotoresistorConfig),
    /// A temperature-dependent resistor described directly on the SPICE line.
    TemperatureDependentResistor(usize),
    /// Every switch sharing one control name.
    Switches(Vec<usize>),
}

/// When routed inputs take effect relative to the completed interval.
/// Kept on the adapter so existing `AnalogConfig` / `from_netlist` callers
/// retain their legacy sampling contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputSampling {
    CurrentBoundary,
    PreviousBoundary,
}

impl InputSampling {
    fn from_yaml(config: &HashMap<String, Value>) -> Result<Self, AnalogError> {
        match config.get("input_sampling") {
            None => Ok(Self::CurrentBoundary),
            Some(Value::String(value)) if value == "current_boundary" => Ok(Self::CurrentBoundary),
            Some(Value::String(value)) if value == "previous_boundary" => {
                Ok(Self::PreviousBoundary)
            }
            Some(_) => Err(AnalogError::Config(
                "config.input_sampling must be `current_boundary` or `previous_boundary`".into(),
            )),
        }
    }
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
    input_sampling: InputSampling,
    inputs_initialized: bool,
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
            } else if let Some(index) = solver.temperature_dependent_resistor_index(element) {
                InputTarget::TemperatureDependentResistor(index)
            } else if let Some(index) = solver.resistor_index(element) {
                let ohms = solver.circuit().resistors[index].ohms;
                if !ohms.is_finite() || ohms <= 0.0 || !ohms.recip().is_finite() {
                    return Err(AnalogError::Config(format!("routed resistor `{element}` must have a positive finite initial resistance")));
                }
                InputTarget::Resistance(index)
            } else if let Some(index) = solver.switch_index(element) {
                InputTarget::Switches(vec![index])
            } else {
                return Err(AnalogError::Config(format!(
                    "config.sources.{input} names `{element}`, which the netlist does not declare \
                     as a V, I, S, or resistor element (temperature-dependent resistors accept Celsius inputs)"
                )));
            };
            inputs.insert(input.clone(), target);
        }
        // A switch's `<ctrl>` is itself a routed input name, so it needs no
        // `sources:` entry. An explicit entry wins if a manifest declares both.
        // Voltage-controlled switches have an empty ctrl and are driven by the
        // MNA engine from their sense nodes, so they are not routed inputs.
        for switch in &solver.circuit().switches {
            if switch.voltage.is_some() || switch.ctrl.is_empty() {
                continue;
            }
            let indices = solver.switches_controlled_by(&switch.ctrl);
            inputs
                .entry(switch.ctrl.clone())
                .or_insert(InputTarget::Switches(indices));
        }

        for (input, model) in &cfg.thermistors {
            model.validate()?;
            if inputs.contains_key(input) {
                return Err(AnalogError::Config(format!(
                    "input `{input}` is assigned more than once"
                )));
            }
            let index = solver.resistor_index(&model.resistor).ok_or_else(|| {
                AnalogError::Config(format!(
                    "thermistor `{input}` names missing resistor `{}`",
                    model.resistor
                ))
            })?;
            if resistor_is_routed(&inputs, index) {
                return Err(AnalogError::Config(format!(
                    "resistor `{}` has multiple routed owners",
                    model.resistor
                )));
            }
            let initial = solver.circuit().resistors[index].ohms;
            if !initial.is_finite() || (initial - model.r_ref_ohms).abs() > model.r_ref_ohms * 1e-9
            {
                return Err(AnalogError::Config(format!(
                    "thermistor `{input}` initial resistor must match r_ref_ohms"
                )));
            }
            inputs.insert(input.clone(), InputTarget::Thermistor(index, model.clone()));
        }

        for (input, model) in &cfg.photoresistors {
            model.validate()?;
            if inputs.contains_key(input) {
                return Err(AnalogError::Config(format!(
                    "input `{input}` is assigned more than once"
                )));
            }
            let index = solver.resistor_index(&model.resistor).ok_or_else(|| {
                AnalogError::Config(format!(
                    "photoresistor `{input}` names missing resistor `{}`",
                    model.resistor
                ))
            })?;
            if resistor_is_routed(&inputs, index) {
                return Err(AnalogError::Config(format!(
                    "resistor `{}` has multiple routed owners",
                    model.resistor
                )));
            }
            let initial = solver.circuit().resistors[index].ohms;
            if !initial.is_finite() || (initial - model.r_ref_ohms).abs() > model.r_ref_ohms * 1e-9
            {
                return Err(AnalogError::Config(format!(
                    "photoresistor `{input}` initial resistor must match r_ref_ohms"
                )));
            }
            inputs.insert(
                input.clone(),
                InputTarget::Photoresistor(index, model.clone()),
            );
        }

        let sample_width = probes.len() + trace_extra.len();
        let mut adapter = Self {
            solver,
            vdd: cfg.vdd,
            substeps: cfg.substeps,
            probes,
            trace_extra,
            inputs,
            input_sampling: InputSampling::CurrentBoundary,
            inputs_initialized: false,
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
        let input_sampling = InputSampling::from_yaml(&config.config)?;
        let netlist = netlist_source(&config.config, base_dir)?;
        let cfg = AnalogConfig::from_yaml(&config.config)?;
        if cfg.probes.is_empty() {
            return Err(AnalogError::Config(
                "config.probes is required: an analog model with no probe produces no outputs"
                    .to_string(),
            ));
        }
        let mut adapter = Self::from_netlist(&netlist, &cfg)?;
        adapter.input_sampling = input_sampling;
        if input_sampling == InputSampling::PreviousBoundary {
            if !cfg.vdd.is_finite() || cfg.vdd <= 0.0 {
                return Err(AnalogError::Config(
                    "previous_boundary sampling requires positive finite config.vdd".into(),
                ));
            }
            for (name, target) in &adapter.inputs {
                if let InputTarget::Switches(indices) = target {
                    if indices
                        .iter()
                        .any(|&index| adapter.solver.circuit().switches[index].voltage.is_some())
                    {
                        return Err(AnalogError::Config(format!(
                            "analog input `{name}` cannot drive a voltage-controlled switch"
                        )));
                    }
                }
            }
        }
        Ok(adapter)
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
            Probe::DiodeCurrent(index) => self.solver.diode_current(*index),
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

    /// Validate a whole sampled input set before advancing time or changing a
    /// source. Initial values must preserve the already-solved operating point.
    fn validate_held_inputs(&self, inputs: &CosimSignals, initial: bool) -> SimResult<()> {
        for (name, value) in inputs {
            let target = self
                .inputs
                .get(name)
                .ok_or_else(|| SimulationError::Other(format!("unknown analog input `{name}`")))?;
            let number = match value {
                CosimSignalValue::Bool(value)
                    if matches!(
                        target,
                        InputTarget::VoltageSource(_) | InputTarget::Switches(_)
                    ) =>
                {
                    if *value {
                        self.vdd
                    } else {
                        0.0
                    }
                }
                CosimSignalValue::I64(value) => *value as f64,
                CosimSignalValue::F64(value) if value.is_finite() => *value,
                _ => {
                    return Err(SimulationError::Other(format!(
                        "analog input `{name}` requires a finite number{}",
                        if matches!(
                            target,
                            InputTarget::VoltageSource(_) | InputTarget::Switches(_)
                        ) {
                            " or a boolean logic level"
                        } else {
                            ""
                        }
                    )));
                }
            };
            let circuit = self.solver.circuit();
            let expected = match target {
                InputTarget::VoltageSource(index) => circuit.voltage_sources[*index].dc,
                InputTarget::CurrentSource(index) => circuit.current_sources[*index].dc,
                InputTarget::Resistance(index) => {
                    validate_resistance(name, number)?;
                    circuit.resistors[*index].ohms
                }
                InputTarget::Thermistor(_, model) => {
                    model.resistance(number).map_err(|error| {
                        SimulationError::Other(format!("analog input `{name}`: {error}"))
                    })?;
                    model.reference_c
                }
                InputTarget::Photoresistor(_, model) => {
                    model.resistance(number).map_err(|error| {
                        SimulationError::Other(format!("analog input `{name}`: {error}"))
                    })?;
                    model.reference_lux
                }
                InputTarget::TemperatureDependentResistor(index) => {
                    let resistor = &circuit.resistors[*index];
                    let (expected, resistance) = if let Some(mut model) = resistor.ntc {
                        let expected = model.temperature_celsius;
                        model.temperature_celsius = number;
                        (expected, model.resistance_ohms(resistor.ohms))
                    } else if let Some(mut model) = resistor.temperature_coefficients {
                        let expected = model.temperature_celsius;
                        model.temperature_celsius = number;
                        (expected, model.resistance_ohms(resistor.ohms))
                    } else {
                        return Err(SimulationError::Other(format!(
                            "analog input `{name}` has no resistor temperature model"
                        )));
                    };
                    if number <= -273.15 {
                        return Err(SimulationError::Other(format!(
                            "analog input `{name}` must be above absolute zero in degC"
                        )));
                    }
                    validate_resistance(name, resistance)?;
                    expected
                }
                InputTarget::Switches(_) => {
                    let closed = match value {
                        CosimSignalValue::Bool(state) => *state,
                        _ => number >= self.vdd / 2.0,
                    };
                    if initial && closed {
                        return Err(SimulationError::Other(format!(
                            "analog input `{name}` at t=0 must match the netlist's initial open switch (false)"
                        )));
                    }
                    continue;
                }
            };
            // Scale only by the compared values: a fixed absolute epsilon
            // would accept a nonzero current against a zero-current source.
            let tolerance = 16.0 * f64::EPSILON * number.abs().max(expected.abs());
            if initial && (!expected.is_finite() || (number - expected).abs() > tolerance) {
                return Err(SimulationError::Other(format!(
                    "analog input `{name}` at t=0 is {number}; it must match the netlist initial value {expected} {}",
                    self.input_unit(name).unwrap_or("")
                )));
            }
        }
        Ok(())
    }

    fn apply_input(&mut self, name: &str, value: &CosimSignalValue) -> SimResult<()> {
        let vdd = self.vdd;
        let Some(target) = self.inputs.get(name) else {
            // Routed into the model but not a circuit source: ignore, exactly
            // like the ngspice wrapper. Manifests routinely route one signal
            // bundle into several models.
            return Ok(());
        };
        match target {
            InputTarget::VoltageSource(index) => {
                let index = *index;
                let volts = numeric(vdd, name, value, "volts")?;
                self.solver.set_voltage_source(index, volts);
            }
            InputTarget::CurrentSource(index) => {
                let index = *index;
                if matches!(value, CosimSignalValue::Bool(_)) {
                    return Err(SimulationError::Other(format!(
                        "analog input `{name}` drives a current source; a boolean has no \
                         current to map to — route a number in amps"
                    )));
                }
                let amps = numeric(vdd, name, value, "amps")?;
                self.solver.set_current_source(index, amps);
            }
            InputTarget::Resistance(index) => {
                let index = *index;
                if matches!(value, CosimSignalValue::Bool(_)) {
                    return Err(SimulationError::Other(format!(
                        "analog input `{name}` drives a resistor; a boolean has no resistance in ohms"
                    )));
                }
                let ohms = numeric(vdd, name, value, "ohms")?;
                self.solver.set_resistance(index, ohms).map_err(|error| {
                    SimulationError::Other(format!("analog input `{name}`: {error}"))
                })?;
            }
            InputTarget::Thermistor(index, model) => {
                let index = *index;
                if matches!(value, CosimSignalValue::Bool(_)) {
                    return Err(SimulationError::Other(format!("analog input `{name}` drives a thermistor; temperature must be numeric Celsius")));
                }
                let celsius = numeric(vdd, name, value, "Celsius")?;
                let ohms = model.resistance(celsius).map_err(|error| {
                    SimulationError::Other(format!("analog input `{name}`: {error}"))
                })?;
                self.solver.set_resistance(index, ohms).map_err(|error| {
                    SimulationError::Other(format!("analog input `{name}`: {error}"))
                })?;
            }
            InputTarget::Photoresistor(index, model) => {
                let index = *index;
                if matches!(value, CosimSignalValue::Bool(_)) {
                    return Err(SimulationError::Other(format!("analog input `{name}` drives a photoresistor; illuminance must be numeric lux")));
                }
                let lux = numeric(vdd, name, value, "lux")?;
                let ohms = model.resistance(lux).map_err(|error| {
                    SimulationError::Other(format!("analog input `{name}`: {error}"))
                })?;
                self.solver.set_resistance(index, ohms).map_err(|error| {
                    SimulationError::Other(format!("analog input `{name}`: {error}"))
                })?;
            }
            InputTarget::TemperatureDependentResistor(index) => {
                let index = *index;
                if matches!(value, CosimSignalValue::Bool(_)) {
                    return Err(SimulationError::Other(format!(
                        "analog input `{name}` drives a resistor temperature; a boolean has no \
                         temperature to map to — route a number in degrees Celsius"
                    )));
                }
                let celsius = numeric(vdd, name, value, "degrees Celsius")?;
                if !self.solver.set_resistor_temperature(index, celsius) {
                    return Err(SimulationError::Other(format!(
                        "analog input `{name}` is not a finite resistor temperature above absolute zero"
                    )));
                }
            }
            InputTarget::Switches(indices) => {
                let closed = match value {
                    CosimSignalValue::Bool(state) => *state,
                    other => {
                        // A numeric control is read as a logic level against
                        // half of Vdd, so an analog signal can gate a switch.
                        numeric(vdd, name, other, "volts")? >= vdd / 2.0
                    }
                };
                for &index in indices {
                    self.solver.set_switch(index, closed);
                }
            }
        }
        Ok(())
    }
}

fn resistor_is_routed(inputs: &BTreeMap<String, InputTarget>, index: usize) -> bool {
    inputs.values().any(|target| {
        matches!(
            target,
            InputTarget::Resistance(i)
                | InputTarget::Thermistor(i, _)
                | InputTarget::Photoresistor(i, _)
                | InputTarget::TemperatureDependentResistor(i)
            if *i == index
        )
    })
}

fn validate_resistance(name: &str, ohms: f64) -> SimResult<()> {
    if !ohms.is_finite() || ohms <= 0.0 || !ohms.recip().is_finite() {
        return Err(SimulationError::Other(format!(
            "analog input `{name}` must produce positive finite resistance and finite conductance"
        )));
    }
    Ok(())
}

fn numeric(vdd: f64, name: &str, value: &CosimSignalValue, unit: &str) -> SimResult<f64> {
    match value {
        CosimSignalValue::Bool(true) => Ok(vdd),
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

impl CosimAdapter for AnalogCosimAdapter {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        let mut outputs = BTreeMap::new();
        self.step_into(step.time_ns, step.dt_ns, &step.inputs, &mut outputs)?;
        Ok(CosimStepResult { outputs })
    }

    fn step_into(
        &mut self,
        time_ns: u64,
        dt_ns: u64,
        inputs: &CosimSignals,
        outputs: &mut CosimSignals,
    ) -> SimResult<()> {
        if self.input_sampling == InputSampling::PreviousBoundary {
            if !self.inputs_initialized && (time_ns != 0 || dt_ns != 0) {
                return Err(SimulationError::Other(
                    "analog previous_boundary sampling requires t=0, dt=0 initialization before advancing time".into(),
                ));
            }
            if time_ns < self.time_ns || (time_ns == 0 && dt_ns != 0) {
                return Err(SimulationError::Other(
                    "analog previous_boundary sampling requires monotonic time and dt=0 at t=0"
                        .into(),
                ));
            }
            self.validate_held_inputs(inputs, time_ns == 0)?;
        } else {
            for (name, value) in inputs {
                self.apply_input(name, value)?;
            }
        }

        // `time_ns` is the END of the step. A boundary already passed (a model
        // whose `step_ns` divides another's) re-reads the probes rather than
        // integrating backwards.
        if time_ns > self.time_ns {
            let span = (time_ns - self.time_ns) as f64 * 1e-9;
            let h = span / f64::from(self.substeps);
            for _ in 0..self.substeps {
                self.solver
                    .advance(h)
                    .map_err(|err| SimulationError::Other(err.to_string()))?;
            }
            self.time_ns = time_ns;
        }

        // A world reads the declared operating point before its first CPU
        // instruction. That initial read is not an integration step and must
        // not duplicate the t=0 trace row or advance its step counter.
        if time_ns != 0 || dt_ns != 0 || self.time_ns != 0 {
            self.step_index += 1;
            self.record_sample();
        }

        for (name, probe) in &self.probes {
            let value = CosimSignalValue::F64(self.read_probe(probe));
            match outputs.get_mut(name.as_str()) {
                Some(slot) => *slot = value,
                None => {
                    outputs.insert(name.clone(), value);
                }
            }
        }
        if self.input_sampling == InputSampling::PreviousBoundary {
            // Probes and the trace above describe the completed interval.
            // Newly sampled sources take effect only when the solver next
            // advances, including when a source directly feeds a probe.
            for (name, value) in inputs {
                self.apply_input(name, value)?;
            }
            self.inputs_initialized = true;
        }
        Ok(())
    }

    fn input_initial_value(&self, name: &str) -> Option<CosimSignalValue> {
        match self.inputs.get(name)? {
            InputTarget::Thermistor(_, model) => Some(CosimSignalValue::F64(model.reference_c)),
            InputTarget::Photoresistor(_, model) => {
                Some(CosimSignalValue::F64(model.reference_lux))
            }
            InputTarget::TemperatureDependentResistor(index) => {
                let resistor = &self.solver.circuit().resistors[*index];
                resistor
                    .ntc
                    .map(|ntc| ntc.temperature_celsius)
                    .or_else(|| {
                        resistor
                            .temperature_coefficients
                            .map(|model| model.temperature_celsius)
                    })
                    .map(CosimSignalValue::F64)
            }
            InputTarget::Resistance(index) => Some(CosimSignalValue::F64(
                self.solver.circuit().resistors[*index].ohms,
            )),
            _ => None,
        }
    }

    /// A switch is boolean; sources and resistors are numeric. NTC numbers are
    /// degrees Celsius and photoresistor numbers are lux. An input the circuit does not use has no kind.
    fn input_kind(&self, name: &str) -> Option<CosimInputKind> {
        Some(match self.inputs.get(name)? {
            InputTarget::Switches(_) => CosimInputKind::Bool,
            InputTarget::VoltageSource(_)
            | InputTarget::CurrentSource(_)
            | InputTarget::Resistance(_)
            | InputTarget::Thermistor(_, _)
            | InputTarget::Photoresistor(_, _)
            | InputTarget::TemperatureDependentResistor(_) => CosimInputKind::Number,
        })
    }

    fn input_unit(&self, name: &str) -> Option<&str> {
        match self.inputs.get(name)? {
            InputTarget::VoltageSource(_) => Some("V"),
            InputTarget::CurrentSource(_) => Some("A"),
            InputTarget::Resistance(_) => Some("ohm"),
            // The spelling of the catalog `analog_source` temperature
            // channels, so one Flex port can feed either model.
            InputTarget::Thermistor(_, _) | InputTarget::TemperatureDependentResistor(_) => {
                Some("°C")
            }
            InputTarget::Photoresistor(_, _) => Some("lx"),
            InputTarget::Switches(_) => Some("V"),
        }
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

/// Resolve `v(out)`, `out`, `V(OUT)`, `i(Vgpio)` or `i(Dled)` against a circuit.
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
            .or_else(|| {
                circuit
                    .diodes
                    .iter()
                    .position(|diode| diode.name.eq_ignore_ascii_case(inner.trim()))
                    .map(Probe::DiodeCurrent)
            })
            .ok_or_else(|| {
                AnalogError::Config(format!(
                    "probe `{expression}`: `{inner}` is not a voltage source, inductor, E, H \
                     or diode element; current probes support only those elements"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosim::CosimRunner;

    fn manifest() -> labwired_config::CosimModelConfig {
        serde_yaml::from_str(
            r#"
id: circuit
adapter: analog
step_ns: 20
inputs: {drive: firmware.drive}
outputs: {volts: circuit.volts}
config:
  input_sampling: previous_boundary
  substeps: 1
  netlist_text: |
    Vdrive in 0 dc 0
    R1 in out 1k
    C1 out 0 20p
  sources: {drive: Vdrive}
  probes: {volts: v(out)}
  trace: [v(in)]
"#,
        )
        .unwrap()
    }

    fn build(config: &labwired_config::CosimModelConfig) -> AnalogCosimAdapter {
        AnalogCosimAdapter::from_manifest_config(config, Path::new(".")).unwrap()
    }

    fn input(value: CosimSignalValue) -> CosimSignals {
        BTreeMap::from([("drive".into(), value)])
    }

    fn volts(signals: &CosimSignals, name: &str) -> f64 {
        match signals[name] {
            CosimSignalValue::F64(value) => value,
            ref other => panic!("expected voltage, got {other:?}"),
        }
    }

    #[test]
    fn a_write_at_19_ns_cannot_charge_the_preceding_20_ns_interval() {
        let mut runner = CosimRunner::from_configs(&[manifest()]).unwrap();
        let mut signals =
            BTreeMap::from([("firmware.drive".into(), CosimSignalValue::Bool(false))]);
        runner.prime_routed_with_signals(&mut signals).unwrap();
        runner.step_until_with_signals(19, &mut signals).unwrap();
        signals.insert("firmware.drive".into(), CosimSignalValue::Bool(true));
        runner.step_until_with_signals(20, &mut signals).unwrap();
        assert_eq!(volts(&signals, "circuit.volts"), 0.0);

        runner.step_until_with_signals(40, &mut signals).unwrap();
        // One backward-Euler step with h = R*C = 20 ns gives Vdd/2.
        assert!((volts(&signals, "circuit.volts") - 1.65).abs() < 1e-9);
        let trace = runner.analog_trace_snapshot(0);
        let direct_source = trace
            .channels
            .iter()
            .position(|channel| channel.name == "circuit.v(in)")
            .unwrap();
        let at_20 = trace
            .samples
            .iter()
            .find(|sample| sample.time_ns == 20)
            .unwrap();
        let at_40 = trace
            .samples
            .iter()
            .find(|sample| sample.time_ns == 40)
            .unwrap();
        assert_eq!(
            at_20.values[direct_source], 0.0,
            "record before latching the next source"
        );
        assert!((at_40.values[direct_source] - 3.3).abs() < 1e-6);
    }

    #[test]
    fn the_default_retains_current_boundary_sampling() {
        for explicit in [false, true] {
            let mut config = manifest();
            if explicit {
                config.config.insert(
                    "input_sampling".into(),
                    Value::String("current_boundary".into()),
                );
            } else {
                config.config.remove("input_sampling");
            }
            let mut adapter = build(&config);
            let mut outputs = CosimSignals::new();
            // Legacy callers may start at a positive boundary without priming.
            adapter
                .step_into(20, 20, &input(CosimSignalValue::Bool(true)), &mut outputs)
                .unwrap();
            assert!((volts(&outputs, "volts") - 1.65).abs() < 1e-9);
        }
    }

    #[test]
    fn inconsistent_initial_voltage_fails_without_changing_the_operating_point() {
        let mut adapter = build(&manifest());
        let mut outputs = BTreeMap::from([("sentinel".into(), CosimSignalValue::I64(7))]);
        let before = outputs.clone();
        let error = adapter
            .step_into(0, 0, &input(CosimSignalValue::Bool(true)), &mut outputs)
            .unwrap_err();
        assert!(error.to_string().contains("t=0"));
        assert!(error.to_string().contains("netlist initial value 0 V"));
        assert_eq!(outputs, before);
        assert_eq!(adapter.time_ns(), 0);
        assert!(!adapter.inputs_initialized);
        assert_eq!(adapter.step_index, 0);

        adapter
            .step_into(0, 0, &input(CosimSignalValue::Bool(false)), &mut outputs)
            .unwrap();
        assert_eq!(volts(&outputs, "volts"), 0.0);
        let snapshot = lock(&adapter.trace).snapshot(0);
        assert_eq!(
            snapshot.samples.len(),
            1,
            "initialization must not add a second t=0 row"
        );
    }

    #[test]
    fn boolean_initial_voltage_uses_vdd_before_comparing_to_dc() {
        let mut config = manifest();
        let netlist = config.config["netlist_text"]
            .as_str()
            .unwrap()
            .replace("dc 0", "dc 3.3");
        config
            .config
            .insert("netlist_text".into(), Value::String(netlist));
        let mut adapter = build(&config);
        let mut outputs = CosimSignals::new();
        adapter
            .step_into(0, 0, &input(CosimSignalValue::Bool(true)), &mut outputs)
            .unwrap();
        assert!((volts(&outputs, "volts") - 3.3).abs() < 1e-9);
        let error = build(&config)
            .step_into(0, 0, &input(CosimSignalValue::Bool(false)), &mut outputs)
            .unwrap_err();
        assert!(error.to_string().contains("netlist initial value 3.3 V"));
    }

    #[test]
    fn sampling_requires_initialization_and_rejects_invalid_values_before_advancing() {
        let mut adapter = build(&manifest());
        let mut outputs = CosimSignals::new();
        assert!(adapter
            .step_into(20, 20, &input(CosimSignalValue::Bool(false)), &mut outputs)
            .unwrap_err()
            .to_string()
            .contains("initialization"));
        adapter
            .step_into(0, 0, &input(CosimSignalValue::Bool(false)), &mut outputs)
            .unwrap();
        let before = outputs.clone();
        for invalid in [
            CosimSignalValue::F64(f64::NAN),
            CosimSignalValue::F64(f64::INFINITY),
            CosimSignalValue::Text("3.3".into()),
        ] {
            assert!(adapter
                .step_into(20, 20, &input(invalid), &mut outputs)
                .is_err());
            assert_eq!(adapter.time_ns(), 0);
            assert_eq!(outputs, before);
        }
        let unknown = BTreeMap::from([("typo".into(), CosimSignalValue::Bool(true))]);
        assert!(adapter
            .step_into(20, 20, &unknown, &mut outputs)
            .unwrap_err()
            .to_string()
            .contains("unknown analog input"));
        adapter
            .step_into(20, 20, &input(CosimSignalValue::Bool(false)), &mut outputs)
            .unwrap();
        assert!(adapter
            .step_into(19, 1, &input(CosimSignalValue::Bool(false)), &mut outputs)
            .unwrap_err()
            .to_string()
            .contains("monotonic time"));
    }

    #[test]
    fn initial_resistance_temperature_current_and_switch_states_are_checked_in_their_units() {
        let netlist = "Vdrive in 0 dc 3.3\nIload out 0 dc 0.001\nRload in out 1k\nRntc out 0 10k beta=3950\nRtempco in 0 1k tc1=0.001 tnom=25 temp=25\nRbounded in 0 10k\nS1 out 0 gate ron=1 roff=1meg\n";
        let cfg = AnalogConfig {
            probes: BTreeMap::from([("volts".into(), "v(out)".into())]),
            sources: BTreeMap::from([
                ("drive".into(), "Vdrive".into()),
                ("current".into(), "Iload".into()),
                ("resistance".into(), "Rload".into()),
                ("ntc".into(), "Rntc".into()),
                ("tempco".into(), "Rtempco".into()),
            ]),
            thermistors: BTreeMap::from([(
                "bounded".into(),
                ThermistorConfig {
                    resistor: "Rbounded".into(),
                    r_ref_ohms: 10_000.0,
                    beta_kelvin: 3950.0,
                    reference_c: 25.0,
                    min_c: 0.0,
                    max_c: 85.0,
                },
            )]),
            ..AnalogConfig::default()
        };
        let mut adapter = AnalogCosimAdapter::from_netlist(netlist, &cfg).unwrap();
        adapter.input_sampling = InputSampling::PreviousBoundary;
        let initial = BTreeMap::from([
            ("drive".into(), CosimSignalValue::Bool(true)),
            ("current".into(), CosimSignalValue::F64(0.001)),
            ("resistance".into(), CosimSignalValue::F64(1_000.0)),
            ("ntc".into(), CosimSignalValue::F64(25.0)),
            ("tempco".into(), CosimSignalValue::F64(25.0)),
            ("bounded".into(), CosimSignalValue::F64(25.0)),
            ("gate".into(), CosimSignalValue::Bool(false)),
        ]);
        let mut outputs = CosimSignals::new();
        adapter.step_into(0, 0, &initial, &mut outputs).unwrap();
        for (name, unit, invalid) in [
            ("drive", Some("V"), CosimSignalValue::Bool(false)),
            ("current", Some("A"), CosimSignalValue::F64(0.002)),
            ("resistance", Some("ohm"), CosimSignalValue::F64(900.0)),
            ("ntc", Some("°C"), CosimSignalValue::F64(26.0)),
            ("tempco", Some("°C"), CosimSignalValue::F64(26.0)),
            ("bounded", Some("°C"), CosimSignalValue::F64(26.0)),
            ("gate", Some("V"), CosimSignalValue::Bool(true)),
        ] {
            assert_eq!(adapter.input_unit(name), unit);
            let mut inconsistent = initial.clone();
            inconsistent.insert(name.into(), invalid);
            let error = adapter
                .step_into(0, 0, &inconsistent, &mut outputs)
                .unwrap_err();
            assert!(error.to_string().contains("t=0"), "{name}: {error}");
        }
        assert_eq!(adapter.input_unit("unknown"), None);
    }

    #[test]
    fn sampling_mode_and_vdd_must_be_valid() {
        for invalid in [Value::Bool(true), Value::String("previous".into())] {
            let mut config = manifest();
            config.config.insert("input_sampling".into(), invalid);
            let error =
                AnalogCosimAdapter::from_manifest_config(&config, Path::new(".")).unwrap_err();
            assert!(error.to_string().contains("config.input_sampling"));
        }
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut config = manifest();
            config
                .config
                .insert("vdd".into(), serde_yaml::to_value(value).unwrap());
            let error =
                AnalogCosimAdapter::from_manifest_config(&config, Path::new(".")).unwrap_err();
            assert!(error.to_string().contains("positive finite config.vdd"));
        }
    }
}
