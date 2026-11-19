// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CPU-free firmware ports connected to the existing co-simulation blocks.
//! This is a checked authoring contract, not a second peripheral or solver
//! runtime. Adapter construction still validates the actual model semantics.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::{HashMap, HashSet};

use crate::{CosimAdapter, CosimModelConfig};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct FlexManifest {
    pub schema_version: u32,
    pub name: String,
    /// User-visible assumptions that scope this functional model. Empty for
    /// older manifests; callers must surface that as incomplete metadata.
    #[serde(default)]
    pub assumptions: Vec<String>,
    /// Checks the model cannot establish, even when its modeled behavior runs.
    #[serde(default)]
    pub unsupported_checks: Vec<String>,
    pub ports: Vec<FlexPort>,
    pub cosim_models: Vec<CosimModelConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct FlexPort {
    pub name: String,
    pub signal: String,
    pub access: FlexPortAccess,
    pub kind: FlexPortKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlexPortAccess {
    Read,
    Write,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlexPortKind {
    Bool,
    Number,
}

impl FlexManifest {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let value: Value = serde_yaml::from_str(yaml).context("invalid Flex manifest YAML")?;
        check_cosim_keys(&value)?;
        let manifest: Self =
            serde_yaml::from_value(value).context("invalid Flex manifest schema")?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate port ownership and model connectivity before execution.
    ///
    /// Signal names are flat identities. They do not implicitly create a CPU,
    /// GPIO bank, ADC or other board endpoint. Every signal has a declared
    /// producer in this manifest. Adapter construction is responsible for
    /// netlist parsing and the behavioral model's own schema and type checks.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported Flex schema_version {}; expected 1",
            self.schema_version
        );
        ensure!(
            !self.name.trim().is_empty() && self.name == self.name.trim(),
            "Flex name must be a nonempty, trimmed string"
        );
        for (field, notes) in [
            ("assumptions", &self.assumptions),
            ("unsupported_checks", &self.unsupported_checks),
        ] {
            ensure!(
                notes
                    .iter()
                    .all(|note| !note.trim().is_empty() && note == note.trim()),
                "Flex {field} entries must be nonempty and trimmed"
            );
        }

        let mut port_names = HashSet::new();
        let mut write_signals = HashMap::new();
        for port in &self.ports {
            token(&port.name, "Flex port name")?;
            token(&port.signal, &format!("port '{}'.signal", port.name))?;
            ensure!(
                port_names.insert(port.name.as_str()),
                "duplicate Flex port name '{}'",
                port.name
            );
            match port.kind {
                FlexPortKind::Bool => {
                    ensure!(
                        port.unit.is_none(),
                        "boolean port '{}' must not declare a unit",
                        port.name
                    );
                }
                FlexPortKind::Number => {
                    let unit = port.unit.as_deref().with_context(|| {
                        format!("numeric port '{}' requires an explicit unit", port.name)
                    })?;
                    token(unit, &format!("port '{}'.unit", port.name))?;
                }
            }
            match port.access {
                FlexPortAccess::Write => {
                    let initial = port.initial.with_context(|| {
                        format!(
                            "write port '{}' requires an explicit initial value",
                            port.name
                        )
                    })?;
                    ensure!(
                        initial.is_finite(),
                        "write port '{}' initial value must be finite",
                        port.name
                    );
                    ensure!(
                        port.kind != FlexPortKind::Bool || initial == 0.0 || initial == 1.0,
                        "boolean write port '{}' initial value must be 0 or 1",
                        port.name
                    );
                    ensure!(
                        write_signals
                            .insert(port.signal.as_str(), port.name.as_str())
                            .is_none(),
                        "signal '{}' has multiple firmware writers",
                        port.signal
                    );
                }
                FlexPortAccess::Read => ensure!(
                    port.initial.is_none(),
                    "read port '{}' must not declare an initial value",
                    port.name
                ),
            }
        }

        let mut model_ids = HashSet::new();
        let mut outputs = HashMap::new();
        let mut consumed = HashSet::new();
        for model in &self.cosim_models {
            token(&model.id, "co-simulation model id")?;
            ensure!(
                model_ids.insert(model.id.as_str()),
                "duplicate co-simulation model id '{}'",
                model.id
            );
            ensure!(
                model.step_ns > 0,
                "model '{}' step_ns must be nonzero",
                model.id
            );
            ensure!(
                matches!(model.adapter, CosimAdapter::Analog | CosimAdapter::Behavior),
                "model '{}' adapter {:?} is unsupported in CPU-free Flex",
                model.id,
                model.adapter
            );
            ensure!(
                model.model.is_none(),
                "model '{}' must not declare an external model path",
                model.id
            );
            for (name, signal) in &model.inputs {
                token(name, &format!("model '{}' input name", model.id))?;
                token(
                    signal,
                    &format!("model '{}' input '{name}' signal", model.id),
                )?;
                consumed.insert(signal.as_str());
            }
            for (name, signal) in &model.outputs {
                token(name, &format!("model '{}' output name", model.id))?;
                token(
                    signal,
                    &format!("model '{}' output '{name}' signal", model.id),
                )?;
                ensure!(
                    !write_signals.contains_key(signal.as_str()),
                    "signal '{signal}' is driven by both firmware and model '{}'",
                    model.id
                );
                if let Some(previous) = outputs.insert(signal.as_str(), model.id.as_str()) {
                    bail!(
                        "signal '{signal}' has multiple model outputs ('{previous}' and '{}')",
                        model.id
                    );
                }
            }
            if model.adapter == CosimAdapter::Analog {
                validate_analog(model)?;
            }
        }

        for model in &self.cosim_models {
            for signal in model.inputs.values() {
                ensure!(
                    write_signals.contains_key(signal.as_str())
                        || outputs.contains_key(signal.as_str()),
                    "model '{}' input signal '{signal}' has no declared producer",
                    model.id
                );
            }
        }
        for port in &self.ports {
            match port.access {
                FlexPortAccess::Read => ensure!(
                    outputs.contains_key(port.signal.as_str()),
                    "read port '{}' signal '{}' is not a model output",
                    port.name,
                    port.signal
                ),
                FlexPortAccess::Write => ensure!(
                    consumed.contains(port.signal.as_str()),
                    "write port '{}' signal '{}' is not consumed by any model",
                    port.name,
                    port.signal
                ),
            }
        }
        Ok(())
    }
}

fn token(value: &str, path: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && !value.chars().any(|c| c.is_whitespace() || c.is_control()),
        "{path} must be nonempty and contain no whitespace or control characters"
    );
    Ok(())
}

fn check_cosim_keys(value: &Value) -> Result<()> {
    let Value::Mapping(root) = value else {
        bail!("Flex manifest must be a mapping");
    };
    let Some(models) = root.get("cosim_models") else {
        return Ok(()); // Canonical deserialization reports the missing field.
    };
    let Value::Sequence(models) = models else {
        bail!("cosim_models must be a sequence");
    };
    for (index, model) in models.iter().enumerate() {
        let Value::Mapping(model) = model else {
            bail!("cosim_models[{index}] must be a mapping");
        };
        for key in model.keys() {
            let Value::String(key) = key else {
                bail!("cosim_models[{index}] contains a non-string key");
            };
            ensure!(
                ["id", "adapter", "model", "step_ns", "inputs", "outputs", "config"]
                    .contains(&key.as_str()),
                "cosim_models[{index}] contains unknown field '{key}'"
            );
        }
    }
    Ok(())
}

fn validate_analog(model: &CosimModelConfig) -> Result<()> {
    let path = format!("analog model '{}'", model.id);
    for key in model.config.keys() {
        ensure!(
            [
                "netlist_text",
                "vdd",
                "probes",
                "sources",
                "substeps",
                "integration",
                "input_sampling",
                "photoresistors"
            ]
            .contains(&key.as_str()),
            "{path} has unsupported config key '{key}'; this Flex path requires an inline netlist_text"
        );
    }
    ensure!(
        model.config.get("input_sampling").and_then(Value::as_str)
            == Some("previous_boundary"),
        "{path} requires config.input_sampling: previous_boundary; Flex must integrate using the input held over the preceding interval"
    );
    let netlist = model
        .config
        .get("netlist_text")
        .and_then(Value::as_str)
        .with_context(|| format!("{path} requires inline config.netlist_text"))?;
    ensure!(
        !netlist.trim().is_empty(),
        "{path} config.netlist_text must not be empty"
    );
    if let Some(value) = model.config.get("vdd") {
        let volts = value
            .as_f64()
            .with_context(|| format!("{path} config.vdd must be a finite positive number"))?;
        ensure!(
            volts.is_finite() && volts > 0.0,
            "{path} config.vdd must be a finite positive number"
        );
    }
    if let Some(value) = model.config.get("substeps") {
        let steps = value
            .as_u64()
            .with_context(|| format!("{path} config.substeps must be a positive u32"))?;
        ensure!(
            steps > 0 && steps <= u64::from(u32::MAX),
            "{path} config.substeps must be a positive u32"
        );
    }
    if let Some(value) = model.config.get("integration") {
        let integration = value
            .as_str()
            .with_context(|| format!("{path} config.integration must be be or trap"))?;
        ensure!(
            matches!(
                integration.trim().to_ascii_lowercase().as_str(),
                "be" | "backward_euler" | "euler" | "trap" | "trapezoidal"
            ),
            "{path} config.integration must be be or trap"
        );
    }
    // A photoresistor entry binds its input by key, in place of a `sources`
    // entry. The analog adapter validates the model parameters themselves.
    let mut source_routes = model.inputs.clone();
    if let Some(value) = model.config.get("photoresistors") {
        let Value::Mapping(models) = value else {
            bail!("{path} config.photoresistors must be a mapping");
        };
        for name in models.keys() {
            let Value::String(name) = name else {
                bail!("{path} config.photoresistors names must be strings");
            };
            token(name, &format!("{path} config.photoresistors"))?;
            ensure!(
                source_routes.remove(name).is_some(),
                "{path} config.photoresistors.{name} has no declared model route"
            );
        }
    }
    check_analog_routes(model, "sources", &source_routes)?;
    check_analog_routes(model, "probes", &model.outputs)?;
    Ok(())
}

fn check_analog_routes(
    model: &CosimModelConfig,
    key: &str,
    routes: &HashMap<String, String>,
) -> Result<()> {
    let path = format!("analog model '{}' config.{key}", model.id);
    let Some(value) = model.config.get(key) else {
        ensure!(
            routes.is_empty(),
            "{path} is required for the declared model routes"
        );
        return Ok(());
    };
    let Value::Mapping(bindings) = value else {
        bail!("{path} must be a mapping");
    };
    for (name, target) in bindings {
        let Value::String(name) = name else {
            bail!("{path} names must be strings");
        };
        token(name, &path)?;
        ensure!(
            routes.contains_key(name),
            "{path}.{name} has no declared model route"
        );
        let target = target
            .as_str()
            .with_context(|| format!("{path}.{name} must be a string"))?;
        ensure!(
            !target.trim().is_empty() && target == target.trim(),
            "{path}.{name} must be nonempty and trimmed"
        );
    }
    for name in routes.keys() {
        ensure!(
            bindings.contains_key(name.as_str()),
            "{path} has no binding for model route '{name}'"
        );
    }
    Ok(())
}

/// A `labwired test` script that explicitly selects portable Flex execution.
///
/// `inputs.flex` names the Flex manifest and `inputs.firmware` the portable
/// WebAssembly module. Nothing here selects or implies an MCU model; a script
/// that wants target instructions names `inputs.system` or `inputs.chip`
/// instead and takes the detailed machine path.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct FlexTestScript {
    pub schema_version: String,
    pub inputs: FlexTestInputs,
    /// One firmware call. Omitted with no `sequence`, it is `run()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<FlexEntry>,
    /// Ordered firmware calls and test-bench stimuli on one firmware
    /// instance and one runtime. Excludes `entry`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequence: Vec<FlexStep>,
    pub limits: FlexTestLimits,
    #[serde(default)]
    pub assertions: Vec<FlexAssertion>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct FlexTestInputs {
    /// Flex manifest path, relative to the script.
    pub flex: String,
    /// Portable firmware WebAssembly module path, relative to the script.
    pub firmware: String,
}

/// A firmware export the runner calls, its arguments in order, and how many
/// times in a row it is called.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexEntry {
    #[serde(default = "default_flex_entry_export")]
    pub export: String,
    #[serde(default)]
    pub args: Vec<FlexEntryArg>,
    #[serde(default = "default_flex_entry_repeat")]
    pub repeat: u32,
}

impl Default for FlexEntry {
    fn default() -> Self {
        Self {
            export: default_flex_entry_export(),
            args: Vec::new(),
            repeat: default_flex_entry_repeat(),
        }
    }
}

fn default_flex_entry_export() -> String {
    "run".to_owned()
}

fn default_flex_entry_repeat() -> u32 {
    1
}

/// One step of a `sequence`: a firmware call, or a value the test bench
/// writes to a write port before the next call (a changed environment).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum FlexStep {
    Call(FlexCallStep),
    Set(FlexSetStep),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexCallStep {
    pub call: FlexEntry,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexSetStep {
    pub set: FlexSetDetails,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexSetDetails {
    pub port: String,
    /// In the port's declared unit.
    pub value: f64,
}

/// One entry argument: the id of a named manifest port, or a plain number.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum FlexEntryArg {
    Port(FlexEntryPortArg),
    Number(FlexEntryNumberArg),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexEntryPortArg {
    pub port: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexEntryNumberArg {
    pub number: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct FlexTestLimits {
    /// WebAssembly fuel for the firmware call. A run that exhausts it stops
    /// with `max_steps` and cannot pass.
    pub max_steps: u64,
    /// Virtual-time ceiling in nanoseconds. A firmware `advance_ns` past it
    /// stops the run with `max_time_ns` before any model steps.
    pub max_time_ns: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum FlexAssertion {
    EntryResult(FlexEntryResultAssertion),
    PortValue(FlexPortValueAssertion),
    DigitalEdges(FlexDigitalEdgesAssertion),
    AnalogSample(FlexAnalogSampleAssertion),
    FinalTime(FlexFinalTimeAssertion),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexEntryResultAssertion {
    pub entry_result: FlexEntryResultDetails,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexEntryResultDetails {
    /// The call's single i32 result, read as unsigned.
    pub equals: u32,
    /// Index of the `sequence` call step whose last result is checked.
    /// Default: the last call step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexPortValueAssertion {
    pub port_value: FlexPortValueDetails,
}

/// The value of a read port after the firmware call returns, in the port's
/// declared unit. At least one bound is required.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexPortValueDetails {
    pub port: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexDigitalEdgesAssertion {
    pub digital_edges: FlexDigitalEdgesDetails,
}

/// The edges of one logic-analyzer channel: their exact number, their exact
/// times, or both. The channel is a Boolean port, or a traced model signal
/// that has no port. `from_ns` and `to_ns` limit the assertion to the edges
/// with `from_ns <= time <= to_ns`; without them it covers the whole run.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexDigitalEdgesDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    /// Signal path of a logic-analyzer channel, as in `flex-trace.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    /// Every edge time in order, in nanoseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub times_ns: Option<Vec<u64>>,
    /// First nanosecond of the window. Default: the start of the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_ns: Option<u64>,
    /// Last nanosecond of the window. Default: the end of the run. A run that
    /// ends before `to_ns` did not observe the whole window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_ns: Option<u64>,
}

impl FlexDigitalEdgesDetails {
    /// The port or signal the assertion names, for messages.
    pub fn target(&self) -> &str {
        self.port
            .as_deref()
            .or(self.signal.as_deref())
            .unwrap_or("")
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexAnalogSampleAssertion {
    pub analog_sample: FlexAnalogSampleDetails,
}

/// The oscilloscope value of one channel at `at_ns`: the last captured sample
/// at or before that time, in the channel's unit. The channel is a numeric
/// read port, or an oscilloscope channel name that has no port. A time after
/// the end of the run has no sample. At least one bound is required.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexAnalogSampleDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    /// Oscilloscope channel name, as in `flex-trace.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    pub at_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

impl FlexAnalogSampleDetails {
    /// The port or channel the assertion names, for messages.
    pub fn target(&self) -> &str {
        self.port
            .as_deref()
            .or(self.channel.as_deref())
            .unwrap_or("")
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexFinalTimeAssertion {
    pub final_time_ns: FlexFinalTimeDetails,
}

/// The virtual time when the run ended. At least one bound is required.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlexFinalTimeDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u64>,
}

/// One of two names selects the instrument channel; `equals` excludes a range.
fn one_name(
    first: &Option<String>,
    second: &Option<String>,
    path: &str,
    names: &str,
) -> Result<()> {
    ensure!(
        first.is_some() != second.is_some(),
        "{path} names exactly one of {names}"
    );
    token(first.as_deref().or(second.as_deref()).unwrap_or(""), path)
}

fn bounds<T: PartialOrd>(
    equals: Option<T>,
    min: Option<T>,
    max: Option<T>,
    path: &str,
) -> Result<()> {
    ensure!(
        equals.is_some() || min.is_some() || max.is_some(),
        "{path} requires equals, min, or max"
    );
    ensure!(
        equals.is_none() || (min.is_none() && max.is_none()),
        "{path} cannot combine equals with min or max"
    );
    if let (Some(min), Some(max)) = (min, max) {
        ensure!(min <= max, "{path} min must not exceed max");
    }
    Ok(())
}

impl FlexTestScript {
    /// The steps the runner executes in order: the `sequence`, or the single
    /// `entry` call (`run()` when the script names neither).
    pub fn steps(&self) -> Vec<FlexStep> {
        if self.sequence.is_empty() {
            vec![FlexStep::Call(FlexCallStep {
                call: self.entry.clone().unwrap_or_default(),
            })]
        } else {
            self.sequence.clone()
        }
    }

    /// The script path of step `index`, for messages.
    pub fn step_path(&self, index: usize) -> String {
        match self.sequence.get(index) {
            None => "entry".to_owned(),
            Some(FlexStep::Call(_)) => format!("sequence[{index}].call"),
            Some(FlexStep::Set(_)) => format!("sequence[{index}].set"),
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == "1.0",
            "Unsupported schema_version '{}' for a Flex test script. Supported version: '1.0'",
            self.schema_version
        );
        for (field, value) in [
            ("inputs.flex", &self.inputs.flex),
            ("inputs.firmware", &self.inputs.firmware),
        ] {
            ensure!(!value.trim().is_empty(), "{field} must not be empty");
        }
        ensure!(
            self.entry.is_none() || self.sequence.is_empty(),
            "a Flex test script names entry or sequence, not both"
        );
        let steps = self.steps();
        ensure!(
            steps.iter().any(|step| matches!(step, FlexStep::Call(_))),
            "sequence must contain at least one call step"
        );
        for (index, step) in steps.iter().enumerate() {
            let path = self.step_path(index);
            match step {
                FlexStep::Call(step) => {
                    let call = &step.call;
                    token(&call.export, &format!("{path}.export"))?;
                    ensure!(call.repeat > 0, "{path}.repeat must be > 0");
                    for (index, arg) in call.args.iter().enumerate() {
                        match arg {
                            FlexEntryArg::Port(arg) => {
                                token(&arg.port, &format!("{path}.args[{index}].port"))?
                            }
                            FlexEntryArg::Number(arg) => ensure!(
                                arg.number.is_finite(),
                                "{path}.args[{index}].number must be finite"
                            ),
                        }
                    }
                }
                FlexStep::Set(step) => {
                    token(&step.set.port, &format!("{path}.port"))?;
                    ensure!(step.set.value.is_finite(), "{path}.value must be finite");
                }
            }
        }
        ensure!(self.limits.max_steps > 0, "limits.max_steps must be > 0");
        ensure!(
            self.limits.max_time_ns > 0 && self.limits.max_time_ns <= FLEX_MAX_TIME_NS,
            "limits.max_time_ns must be within 1..={FLEX_MAX_TIME_NS}"
        );
        for (index, assertion) in self.assertions.iter().enumerate() {
            match assertion {
                FlexAssertion::EntryResult(assertion) => {
                    if let Some(step) = assertion.entry_result.step {
                        ensure!(
                            !self.sequence.is_empty(),
                            "assertions[{index}].entry_result.step needs a sequence"
                        );
                        ensure!(
                            matches!(self.sequence.get(step), Some(FlexStep::Call(_))),
                            "assertions[{index}].entry_result.step {step} is not a call step of the sequence"
                        );
                    }
                }
                FlexAssertion::PortValue(assertion) => {
                    let details = &assertion.port_value;
                    token(
                        &details.port,
                        &format!("assertions[{index}].port_value.port"),
                    )?;
                    let path = format!("assertions[{index}].port_value");
                    ensure!(
                        [details.equals, details.min, details.max]
                            .iter()
                            .flatten()
                            .all(|bound| bound.is_finite()),
                        "{path} bounds must be finite"
                    );
                    bounds(details.equals, details.min, details.max, &path)?;
                }
                FlexAssertion::DigitalEdges(assertion) => {
                    let details = &assertion.digital_edges;
                    let path = format!("assertions[{index}].digital_edges");
                    one_name(&details.port, &details.signal, &path, "port or signal")?;
                    ensure!(
                        details.count.is_some() || details.times_ns.is_some(),
                        "{path} requires count or times_ns"
                    );
                    let from = details.from_ns.unwrap_or(0);
                    let to = details.to_ns.unwrap_or(FLEX_MAX_TIME_NS);
                    ensure!(from <= to, "{path}.from_ns must not exceed to_ns");
                    ensure!(
                        to <= FLEX_MAX_TIME_NS,
                        "{path}.to_ns must be within 0..={FLEX_MAX_TIME_NS}"
                    );
                    if let Some(times) = &details.times_ns {
                        ensure!(
                            times.windows(2).all(|pair| pair[0] <= pair[1]),
                            "{path}.times_ns must be in time order"
                        );
                        ensure!(
                            times.iter().all(|time| (from..=to).contains(time)),
                            "{path}.times_ns must be inside from_ns..=to_ns"
                        );
                    }
                }
                FlexAssertion::AnalogSample(assertion) => {
                    let details = &assertion.analog_sample;
                    let path = format!("assertions[{index}].analog_sample");
                    one_name(&details.port, &details.channel, &path, "port or channel")?;
                    ensure!(
                        [details.equals, details.min, details.max]
                            .iter()
                            .flatten()
                            .all(|bound| bound.is_finite()),
                        "{path} bounds must be finite"
                    );
                    bounds(details.equals, details.min, details.max, &path)?;
                }
                FlexAssertion::FinalTime(assertion) => {
                    let details = &assertion.final_time_ns;
                    bounds(
                        details.equals,
                        details.min,
                        details.max,
                        &format!("assertions[{index}].final_time_ns"),
                    )?;
                }
            }
        }
        Ok(())
    }
}

/// Largest virtual time a Flex run can reach: instrument timestamps must stay
/// exact JavaScript integers.
pub const FLEX_MAX_TIME_NS: u64 = (1 << 53) - 1;

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SCRIPT: &str = r#"
schema_version: "1.0"
inputs: { flex: system.yaml, firmware: firmware.wasm }
entry:
  args:
    - { port: lux }
    - { number: 100 }
limits: { max_steps: 1000, max_time_ns: 5000000 }
assertions:
  - entry_result: { equals: 2974 }
  - port_value: { port: sensor_v, min: 2.39, max: 2.40 }
  - digital_edges: { port: user_led_state, count: 2 }
"#;

    fn load_script(yaml: &str) -> Result<crate::LoadedTestScript> {
        let dir = std::env::temp_dir().join(format!(
            "labwired-flex-script-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("script.yaml");
        std::fs::write(&path, yaml).unwrap();
        let loaded = crate::load_test_script(&path);
        let _ = std::fs::remove_dir_all(dir);
        loaded
    }

    #[test]
    fn test_script_selects_flex_only_through_inputs_flex() {
        let Ok(crate::LoadedTestScript::Flex(script)) = load_script(TEST_SCRIPT) else {
            panic!("inputs.flex must load as a Flex test script");
        };
        let entry = script.entry.clone().expect("entry");
        assert_eq!(entry.export, "run");
        assert_eq!(entry.repeat, 1);
        assert_eq!(
            entry.args,
            [
                FlexEntryArg::Port(FlexEntryPortArg { port: "lux".into() }),
                FlexEntryArg::Number(FlexEntryNumberArg { number: 100.0 }),
            ]
        );
        assert_eq!(script.assertions.len(), 3);

        // The same script without `inputs.flex` is a detailed-target script
        // and must name its target; Flex is never the silent fallback.
        let detailed = TEST_SCRIPT.replace("flex: system.yaml, ", "");
        assert!(!matches!(
            load_script(&detailed),
            Ok(crate::LoadedTestScript::Flex(_))
        ));
    }

    #[test]
    fn test_script_rejects_mixed_targets_and_unchecked_values() {
        let cases = [
            (
                "inputs: { flex: system.yaml,",
                "inputs: { system: board.yaml, flex: system.yaml,",
                "unknown field",
            ),
            (
                "inputs: { flex: system.yaml,",
                "inputs: { chip: stm32f103, flex: system.yaml,",
                "unknown field",
            ),
            (
                "schema_version: \"1.0\"",
                "schema_version: \"1.2\"",
                "Unsupported schema_version",
            ),
            ("max_steps: 1000", "max_steps: 0", "max_steps must be > 0"),
            (
                "max_time_ns: 5000000",
                "max_time_ns: 0",
                "max_time_ns must be within",
            ),
            (
                "max_time_ns: 5000000",
                "max_time_ns: 9007199254740992",
                "max_time_ns must be within",
            ),
            ("{ number: 100 }", "{ number: .nan }", "must be finite"),
            (
                "{ number: 100 }",
                "{ number: 100, port: lux }",
                "Failed to parse Flex Test Script",
            ),
            (
                "min: 2.39, max: 2.40",
                "min: 2.40, max: 2.39",
                "min must not exceed max",
            ),
            (
                "port: sensor_v, min: 2.39, max: 2.40",
                "port: sensor_v",
                "requires equals, min, or max",
            ),
            (
                "min: 2.39, max: 2.40",
                "equals: 2.39, max: 2.40",
                "cannot combine equals",
            ),
            (
                "count: 2",
                "min_edges: 2",
                "Failed to parse Flex Test Script",
            ),
        ];
        for (from, to, expected) in cases {
            assert!(TEST_SCRIPT.contains(from), "fixture lost '{from}'");
            let error = load_script(&TEST_SCRIPT.replace(from, to))
                .expect_err(&format!("'{to}' must be rejected"));
            let message = format!("{error:#}");
            assert!(message.contains(expected), "'{to}': {message}");
        }
    }

    #[test]
    fn test_script_instrument_assertions_name_one_channel_and_one_expectation() {
        let edges = "  - digital_edges: { port: user_led_state, count: 2 }\n";
        assert!(TEST_SCRIPT.contains(edges), "fixture lost its edge count");
        let instruments = "  - digital_edges: { port: user_led_state, times_ns: [1000, 2000] }\n  - digital_edges: { signal: comparator.out, count: 1, from_ns: 500, to_ns: 1500 }\n  - analog_sample: { port: sensor_v, at_ns: 350000, min: 1.4, max: 1.6 }\n  - analog_sample: { channel: divider.v, at_ns: 0, equals: 0 }\n  - final_time_ns: { equals: 4050000 }\n";
        let Ok(crate::LoadedTestScript::Flex(script)) =
            load_script(&TEST_SCRIPT.replace(edges, instruments))
        else {
            panic!("instrument assertions must load");
        };
        assert_eq!(script.assertions.len(), 7);
        let FlexAssertion::DigitalEdges(timed) = &script.assertions[2] else {
            panic!("assertion 2 is digital_edges");
        };
        assert_eq!(timed.digital_edges.times_ns, Some(vec![1000, 2000]));
        assert_eq!(
            (timed.digital_edges.from_ns, timed.digital_edges.to_ns),
            (None, None)
        );
        let FlexAssertion::DigitalEdges(windowed) = &script.assertions[3] else {
            panic!("assertion 3 is digital_edges");
        };
        assert_eq!(
            (windowed.digital_edges.from_ns, windowed.digital_edges.to_ns),
            (Some(500), Some(1500))
        );
        assert_eq!(timed.digital_edges.count, None);
        let FlexAssertion::AnalogSample(sample) = &script.assertions[5] else {
            panic!("assertion 5 is analog_sample");
        };
        assert_eq!(sample.analog_sample.target(), "divider.v");

        let cases = [
            (
                "{ port: user_led_state, count: 2 }",
                "{ port: user_led_state, signal: comparator.out, count: 2 }",
                "exactly one of port or signal",
            ),
            (
                "{ port: user_led_state, count: 2 }",
                "{ count: 2 }",
                "exactly one of port or signal",
            ),
            (
                "{ port: user_led_state, count: 2 }",
                "{ port: user_led_state }",
                "requires count or times_ns",
            ),
            (
                "{ port: user_led_state, count: 2 }",
                "{ port: user_led_state, times_ns: [2000, 1000] }",
                "times_ns must be in time order",
            ),
            (
                "{ port: user_led_state, count: 2 }",
                "{ port: user_led_state, count: 2, from_ns: 2001, to_ns: 2000 }",
                "from_ns must not exceed to_ns",
            ),
            (
                "{ port: user_led_state, count: 2 }",
                "{ port: user_led_state, times_ns: [1000, 2001], from_ns: 1000, to_ns: 2000 }",
                "times_ns must be inside from_ns..=to_ns",
            ),
            (
                "{ port: user_led_state, count: 2 }",
                "{ port: user_led_state, count: 2, to_ns: 9007199254740992 }",
                "to_ns must be within",
            ),
            (
                "digital_edges: { port: user_led_state, count: 2 }",
                "analog_sample: { port: sensor_v, channel: divider.v, at_ns: 0, min: 1 }",
                "exactly one of port or channel",
            ),
            (
                "digital_edges: { port: user_led_state, count: 2 }",
                "analog_sample: { port: sensor_v, at_ns: 0 }",
                "requires equals, min, or max",
            ),
            (
                "digital_edges: { port: user_led_state, count: 2 }",
                "analog_sample: { port: sensor_v, min: 1 }",
                "Failed to parse Flex Test Script",
            ),
            (
                "digital_edges: { port: user_led_state, count: 2 }",
                "analog_sample: { port: sensor_v, at_ns: 0, min: .inf }",
                "bounds must be finite",
            ),
            (
                "digital_edges: { port: user_led_state, count: 2 }",
                "final_time_ns: { min: 2, max: 1 }",
                "min must not exceed max",
            ),
            (
                "digital_edges: { port: user_led_state, count: 2 }",
                "final_time_ns: {}",
                "requires equals, min, or max",
            ),
            (
                "digital_edges: { port: user_led_state, count: 2 }",
                "final_time_ns: { equals: -1 }",
                "Failed to parse Flex Test Script",
            ),
        ];
        for (from, to, expected) in cases {
            assert!(TEST_SCRIPT.contains(from), "fixture lost '{from}'");
            let error = load_script(&TEST_SCRIPT.replace(from, to))
                .expect_err(&format!("'{to}' must be rejected"));
            let message = format!("{error:#}");
            assert!(message.contains(expected), "'{to}': {message}");
        }
    }

    #[test]
    fn test_script_sequence_orders_calls_and_bench_stimuli() {
        let entry = "entry:\n  args:\n    - { port: lux }\n    - { number: 100 }\n";
        assert!(TEST_SCRIPT.contains(entry), "fixture lost its entry");
        let sequence = "sequence:\n  - call: { export: setup, args: [{ port: lux }] }\n  - set: { port: lux, value: 50 }\n  - call: { export: loop, repeat: 3 }\n";
        let script = TEST_SCRIPT.replace(entry, sequence);
        let Ok(crate::LoadedTestScript::Flex(loaded)) = load_script(&script) else {
            panic!("a sequence must load as a Flex test script");
        };
        assert!(loaded.entry.is_none());
        let steps = loaded.steps();
        assert_eq!(steps, loaded.sequence);
        assert_eq!(steps.len(), 3);
        let FlexStep::Call(setup) = &steps[0] else {
            panic!("step 0 is a call");
        };
        assert_eq!(
            (setup.call.export.as_str(), setup.call.repeat),
            ("setup", 1)
        );
        assert_eq!(
            steps[1],
            FlexStep::Set(FlexSetStep {
                set: FlexSetDetails {
                    port: "lux".into(),
                    value: 50.0
                }
            })
        );
        let FlexStep::Call(repeated) = &steps[2] else {
            panic!("step 2 is a call");
        };
        assert_eq!(
            (repeated.call.export.as_str(), repeated.call.repeat),
            ("loop", 3)
        );
        assert_eq!(loaded.step_path(1), "sequence[1].set");

        let cases = [
            (
                "sequence:\n",
                "entry: { export: run }\nsequence:\n",
                "entry or sequence, not both",
            ),
            (
                "repeat: 3",
                "repeat: 0",
                "sequence[2].call.repeat must be > 0",
            ),
            (
                "value: 50",
                "value: .inf",
                "sequence[1].set.value must be finite",
            ),
            (
                "  - call: { export: setup, args: [{ port: lux }] }\n",
                "",
                "", // still valid: one call step remains
            ),
            (
                "entry_result: { equals: 2974 }",
                "entry_result: { equals: 2974, step: 1 }",
                "entry_result.step 1 is not a call step",
            ),
            (
                "entry_result: { equals: 2974 }",
                "entry_result: { equals: 2974, step: 3 }",
                "entry_result.step 3 is not a call step",
            ),
            (
                "  - call: { export: loop, repeat: 3 }\n",
                "  - call: { export: loop, repeat: 3, until: 1 }\n",
                "Failed to parse Flex Test Script",
            ),
        ];
        for (from, to, expected) in cases {
            assert!(script.contains(from), "sequence fixture lost '{from}'");
            let loaded = load_script(&script.replace(from, to));
            if expected.is_empty() {
                loaded.expect("one remaining call step is a valid sequence");
                continue;
            }
            let message = format!(
                "{:#}",
                loaded.expect_err(&format!("'{to}' must be rejected"))
            );
            assert!(message.contains(expected), "'{to}': {message}");
        }

        // Stimuli alone run no firmware.
        let only_set = script
            .replace("  - call: { export: setup, args: [{ port: lux }] }\n", "")
            .replace("  - call: { export: loop, repeat: 3 }\n", "");
        let message = format!("{:#}", load_script(&only_set).expect_err("no call step"));
        assert!(message.contains("at least one call step"), "{message}");
        // `step` names a sequence index, so it needs a sequence.
        let message = format!(
            "{:#}",
            load_script(&TEST_SCRIPT.replace("equals: 2974", "equals: 2974, step: 0"))
                .expect_err("step without sequence")
        );
        assert!(message.contains("needs a sequence"), "{message}");
    }

    const RC: &str = r#"
schema_version: 1
name: RC firmware
ports:
  - { name: drive, signal: firmware.drive, access: write, kind: bool, initial: 0 }
  - { name: sample, signal: circuit.voltage, access: read, kind: number, unit: V }
cosim_models:
  - id: rc
    adapter: analog
    step_ns: 100000
    inputs: { drive: firmware.drive }
    outputs: { voltage: circuit.voltage }
    config:
      netlist_text: |
        Vdrive in 0 dc 0
        R1 in out 10k
        C1 out 0 100n
      vdd: 3.3
      substeps: 10
      input_sampling: previous_boundary
      integration: be
      sources: { drive: Vdrive }
      probes: { voltage: "v(out)" }
"#;

    fn manifest() -> FlexManifest {
        FlexManifest::from_yaml(RC).unwrap()
    }

    #[test]
    fn cpu_free_rc_manifest_round_trips_and_reuses_parameterized_models() {
        let mut manifest = manifest();
        assert_eq!(manifest.ports[0].kind, FlexPortKind::Bool);
        assert_eq!(manifest.ports[1].access, FlexPortAccess::Read);
        let mut second = manifest.cosim_models[0].clone();
        second.id = "rc_slow".into();
        second
            .outputs
            .insert("voltage".into(), "circuit.slow".into());
        second.config.insert(
            "netlist_text".into(),
            "Vdrive in 0 dc 0\nR1 in out 20k\nC1 out 0 100n\n".into(),
        );
        manifest.cosim_models.push(second);
        let mut read = manifest.ports[1].clone();
        read.name = "slow_sample".into();
        read.signal = "circuit.slow".into();
        manifest.ports.push(read);
        // One firmware writer may fan out to several models.
        manifest.validate().unwrap();
        let yaml = serde_yaml::to_string(&manifest).unwrap();
        let loaded = FlexManifest::from_yaml(&yaml).unwrap();
        assert_eq!(loaded.cosim_models.len(), 2);
        assert_eq!(loaded.ports.len(), 3);
    }

    #[test]
    fn unknown_schema_fields_are_not_discarded() {
        for yaml in [
            format!("{RC}\nchip: stm32f401\n"),
            RC.replace("name: drive,", "name: drive, optional: true,"),
            RC.replace(
                "    adapter: analog",
                "    adapter: analog\n    latency_ns: 4",
            ),
            RC.replace("      substeps: 10", "      substep: 10"),
        ] {
            assert!(FlexManifest::from_yaml(&yaml).is_err(), "{yaml}");
        }
    }

    #[test]
    fn scope_notes_round_trip_and_reject_blank_entries() {
        let mut manifest = manifest();
        manifest.assumptions = vec!["portable firmware uses named ports".into()];
        manifest.unsupported_checks = vec!["target instruction fidelity".into()];
        let loaded = FlexManifest::from_yaml(&serde_yaml::to_string(&manifest).unwrap()).unwrap();
        assert_eq!(loaded.assumptions, manifest.assumptions);
        assert_eq!(loaded.unsupported_checks, manifest.unsupported_checks);

        manifest.assumptions.push("  ".into());
        assert!(manifest
            .validate()
            .unwrap_err()
            .to_string()
            .contains("assumptions entries"));
        manifest.assumptions.pop();
        manifest.unsupported_checks.push("not trimmed ".into());
        assert!(manifest
            .validate()
            .unwrap_err()
            .to_string()
            .contains("unsupported_checks entries"));
    }

    #[test]
    fn analog_sampling_contract_is_required_with_a_specific_error() {
        for yaml in [
            RC.replace("      input_sampling: previous_boundary\n", ""),
            RC.replace(
                "input_sampling: previous_boundary",
                "input_sampling: current_boundary",
            ),
            RC.replace("input_sampling: previous_boundary", "input_sampling: true"),
        ] {
            let error = format!("{:#}", FlexManifest::from_yaml(&yaml).unwrap_err());
            assert!(
                error.contains("requires config.input_sampling: previous_boundary"),
                "{error}"
            );
        }
    }

    #[test]
    fn photoresistor_entries_bind_analog_inputs_in_place_of_sources() {
        let bound = RC
            .replace(
                "kind: bool, initial: 0",
                "kind: number, unit: lx, initial: 10",
            )
            .replace(
                "sources: { drive: Vdrive }",
                "photoresistors: { drive: { resistor: R1 } }",
            );
        assert_ne!(bound, RC);
        FlexManifest::from_yaml(&bound).unwrap();
        for (name, yaml, expected) in [
            (
                "unbound input",
                bound.replace("photoresistors: { drive: { resistor: R1 } }", ""),
                "config.sources is required",
            ),
            (
                "undeclared route",
                bound.replace("photoresistors: { drive:", "photoresistors: { typo:"),
                "config.photoresistors.typo has no declared model route",
            ),
            (
                "two owners",
                bound.replace(
                    "photoresistors:",
                    "sources: { drive: Vdrive }\n      photoresistors:",
                ),
                "config.sources.drive has no declared model route",
            ),
        ] {
            let error = format!("{:#}", FlexManifest::from_yaml(&yaml).unwrap_err());
            assert!(error.contains(expected), "{name}: {error}");
        }
    }

    #[test]
    fn ports_require_real_initial_state_and_explicit_numeric_units() {
        type Change = fn(&mut FlexManifest);
        let cases: &[(&str, Change)] = &[
            ("wrong version", |m| m.schema_version = 2),
            ("blank name", |m| m.name = " ".into()),
            ("blank port name", |m| m.ports[0].name.clear()),
            ("blank signal", |m| m.ports[0].signal.clear()),
            ("duplicate name", |m| m.ports[1].name = "drive".into()),
            ("missing write initial", |m| m.ports[0].initial = None),
            ("nonfinite write initial", |m| {
                m.ports[0].initial = Some(f64::NAN)
            }),
            ("nonboolean initial", |m| m.ports[0].initial = Some(2.0)),
            ("read initial", |m| m.ports[1].initial = Some(0.0)),
            ("missing numeric unit", |m| m.ports[1].unit = None),
            ("blank numeric unit", |m| m.ports[1].unit = Some(" ".into())),
            ("boolean unit", |m| m.ports[0].unit = Some("V".into())),
        ];
        for (label, change) in cases {
            let mut m = manifest();
            change(&mut m);
            assert!(m.validate().is_err(), "accepted {label}");
        }
        let mut m = manifest();
        m.ports[0].kind = FlexPortKind::Number;
        m.ports[0].unit = Some("V".into());
        m.ports[0].initial = Some(-0.5);
        m.validate().unwrap();
    }

    #[test]
    fn topology_rejects_missing_producers_and_conflicting_writers() {
        type Change = fn(&mut FlexManifest);
        let cases: &[(&str, Change)] = &[
            ("duplicate model id", |m| {
                let second = m.cosim_models[0].clone();
                m.cosim_models.push(second);
            }),
            ("duplicate firmware writer", |m| {
                let mut port = m.ports[0].clone();
                port.name = "second_drive".into();
                m.ports.push(port);
            }),
            ("firmware/model conflict", |m| {
                m.cosim_models[0]
                    .outputs
                    .insert("voltage".into(), "firmware.drive".into());
            }),
            ("multiple model outputs", |m| {
                let mut model = m.cosim_models[0].clone();
                model.id = "second".into();
                m.cosim_models.push(model);
            }),
            ("missing model input", |m| {
                m.cosim_models[0]
                    .inputs
                    .insert("drive".into(), "undeclared".into());
            }),
            ("unproduced read", |m| {
                m.ports[1].signal = "undeclared".into()
            }),
            ("unconsumed write", |m| {
                let mut port = m.ports[0].clone();
                port.name = "unused".into();
                port.signal = "firmware.unused".into();
                m.ports.push(port);
            }),
        ];
        for (label, change) in cases {
            let mut m = manifest();
            change(&mut m);
            assert!(m.validate().is_err(), "accepted {label}");
        }
    }

    #[test]
    fn analog_contract_rejects_external_or_ignored_configuration() {
        type Change = fn(&mut FlexManifest);
        let cases: &[(&str, Change)] = &[
            ("zero step", |m| m.cosim_models[0].step_ns = 0),
            ("external process", |m| {
                m.cosim_models[0].adapter = CosimAdapter::ExternalProcess
            }),
            ("mock adapter", |m| {
                m.cosim_models[0].adapter = CosimAdapter::Mock
            }),
            ("model path", |m| {
                m.cosim_models[0].model = Some("rc.py".into())
            }),
            ("missing sampling contract", |m| {
                m.cosim_models[0].config.remove("input_sampling");
            }),
            ("legacy sampling contract", |m| {
                m.cosim_models[0]
                    .config
                    .insert("input_sampling".into(), "current_boundary".into());
            }),
            ("file netlist", |m| {
                m.cosim_models[0].config.remove("netlist_text");
                m.cosim_models[0]
                    .config
                    .insert("netlist".into(), "rc.cir".into());
            }),
            ("empty netlist", |m| {
                m.cosim_models[0]
                    .config
                    .insert("netlist_text".into(), " ".into());
            }),
            ("nonfinite voltage", |m| {
                m.cosim_models[0]
                    .config
                    .insert("vdd".into(), f64::INFINITY.into());
            }),
            ("zero substeps", |m| {
                m.cosim_models[0].config.insert("substeps".into(), 0.into());
            }),
            ("wide substeps", |m| {
                m.cosim_models[0]
                    .config
                    .insert("substeps".into(), (u64::from(u32::MAX) + 1).into());
            }),
            ("unsupported integration", |m| {
                m.cosim_models[0]
                    .config
                    .insert("integration".into(), "guess".into());
            }),
            ("missing probe", |m| {
                m.cosim_models[0].config.remove("probes");
            }),
            ("unmapped source", |m| {
                m.cosim_models[0].config.get_mut("sources").unwrap()["unused"] = "Vextra".into();
            }),
        ];
        for (label, change) in cases {
            let mut m = manifest();
            change(&mut m);
            assert!(m.validate().is_err(), "accepted {label}");
        }
    }
}
