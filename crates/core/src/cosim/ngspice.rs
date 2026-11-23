// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `adapter: ngspice` — one manifest contract, two solver hosts.
//!
//! The browser engine hands each boundary to a JavaScript callback that owns an
//! ngspice WASM instance (`labwired-wasm`'s `ngspice` module). A native build
//! has no such callback, so [`build_native_adapter`] spawns the ngspice host in
//! `tools/cosim/labwired_ngspice.py` (embedded in this crate) and exchanges the
//! same boundaries over the existing `external_process` JSONL transport.
//!
//! Shared by both hosts, and kept here so they cannot drift:
//! - [`request_json`] / [`parse_response`]: the exchange frames,
//! - [`require_outputs`]: every declared output must come back,
//! - [`NgspiceTrace`]: which outputs reach the oscilloscope ring, in volts.
//!
//! The native host additionally takes a validated [`NgspiceModelSpec`]: the
//! same `config.input_sources` / `config.output_nodes` / `config.trace_volts`
//! keys the browser runtime reads, checked before any process starts.
//!
//! Neither host can capture or restore solver state: snapshot/restore is
//! refused for an ngspice session, not approximated.

use crate::cosim::{
    AnalogChannel, AnalogTraceHandle, CosimAdapter, CosimSignalValue, CosimSignals, CosimStep,
    CosimStepResult, ExternalProcessCosimAdapter, ExternalProcessCosimOptions, SignalPath,
};
use crate::{SimResult, SimulationError};
use labwired_config::CosimModelConfig;
use serde_json::{json, Value};
use serde_yaml::Value as Yaml;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The native ngspice host, run as `python3 -c <source> --bridge-spec <json>`.
const NATIVE_HOST_SOURCE: &str = include_str!("../../../../tools/cosim/labwired_ngspice.py");

/// Netlist limits shared with the browser runtime (`MAX_BROWSER_SPICE_NETLIST_*`).
pub const MAX_NETLIST_BYTES: usize = 256 * 1024;
pub const MAX_NETLIST_LINES: usize = 1024;

/// One native exchange may include libngspice start-up and netlist parsing.
const NATIVE_STEP_TIMEOUT: Duration = Duration::from_secs(20);

/// Serialize one boundary as the ngspice exchange request.
pub fn request_json(model_id: &str, step: &CosimStep) -> Result<String, String> {
    let mut inputs = serde_json::Map::new();
    for (name, value) in &step.inputs {
        let number = match value {
            CosimSignalValue::I64(value) => *value as f64,
            CosimSignalValue::F64(value) if value.is_finite() => *value,
            CosimSignalValue::Bool(value) => {
                inputs.insert(name.clone(), json!(value));
                continue;
            }
            CosimSignalValue::Text(_) => {
                return Err(format!(
                    "co-sim model '{}' ngspice callback input '{}' must be numeric or boolean",
                    model_id, name
                ));
            }
            CosimSignalValue::F64(_) => {
                return Err(format!(
                    "co-sim model '{}' ngspice callback input '{}' must be finite",
                    model_id, name
                ));
            }
        };
        inputs.insert(name.clone(), json!(number));
    }
    serde_json::to_string(&json!({
        "model_id": model_id,
        "time_ns": step.time_ns,
        "dt_ns": step.dt_ns,
        "inputs": inputs,
    }))
    .map_err(|error| {
        format!(
            "co-sim model '{}' ngspice callback request could not be serialized: {error}",
            model_id
        )
    })
}

/// Parse an exchange response: a JSON object of finite numbers and booleans.
pub fn parse_response(model_id: &str, response: &str) -> Result<CosimStepResult, String> {
    let value: Value = serde_json::from_str(response).map_err(|error| {
        format!(
            "co-sim model '{}' ngspice callback returned invalid JSON: {error}",
            model_id
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        format!(
            "co-sim model '{}' ngspice callback response must be a JSON object",
            model_id
        )
    })?;
    let mut outputs = CosimSignals::new();
    for (name, value) in object {
        outputs.insert(name.clone(), output_value(model_id, name, value)?);
    }
    Ok(CosimStepResult { outputs })
}

fn output_value(model_id: &str, name: &str, value: &Value) -> Result<CosimSignalValue, String> {
    if let Some(level) = value.as_bool() {
        return Ok(CosimSignalValue::Bool(level));
    }
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .map(CosimSignalValue::F64)
        .ok_or_else(|| {
            format!(
                "co-sim model '{}' ngspice callback output '{}' must be a finite number or boolean",
                model_id, name
            )
        })
}

/// Fail when the solver omitted a declared output.
pub fn require_outputs(
    model_id: &str,
    result: &CosimStepResult,
    expected: &[String],
) -> Result<(), String> {
    for name in expected {
        if !result.outputs.contains_key(name) {
            return Err(format!(
                "co-sim model {model_id} ngspice callback omitted output {name}"
            ));
        }
    }
    Ok(())
}

/// Every output the solver must return: the routed outputs plus the
/// scope-only names listed in `config.trace_volts`.
pub fn expected_outputs(config: &CosimModelConfig) -> Vec<String> {
    let mut names: Vec<_> = config.outputs.keys().cloned().collect();
    names.extend(NgspiceTrace::new(config).names);
    names.sort();
    names.dedup();
    names
}

/// `config.input_sampling` of an `adapter: ngspice` model: which inputs the
/// solver integrates the interval ending at `time_ns` with. Applied here, in
/// front of either host, so the browser exchange and the native host receive
/// the same frames.
///
/// - `current_boundary` (default): the inputs routed at this boundary, held
///   over the whole interval that ends at it.
/// - `previous_boundary`: the inputs latched at the previous boundary, as the
///   analog adapter's mode of the same name. An input written mid-interval
///   reaches the circuit from the next boundary on. Requires the runner's
///   `(time_ns = 0, dt_ns = 0)` initialization step first (Flex sends it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NgspiceInputSampling {
    #[default]
    CurrentBoundary,
    PreviousBoundary,
}

impl NgspiceInputSampling {
    pub fn from_config(config: &CosimModelConfig) -> Result<Self, String> {
        match config.config.get("input_sampling") {
            None => Ok(Self::CurrentBoundary),
            Some(Yaml::String(value)) if value == "current_boundary" => Ok(Self::CurrentBoundary),
            Some(Yaml::String(value)) if value == "previous_boundary" => {
                Ok(Self::PreviousBoundary)
            }
            Some(_) => Err(format!(
                "ngspice model '{}' config.input_sampling must be `current_boundary` or `previous_boundary`",
                config.id
            )),
        }
    }
}

/// Rewrites each boundary's inputs by [`NgspiceInputSampling`].
pub struct NgspiceInputLatch {
    model_id: String,
    sampling: NgspiceInputSampling,
    held: Option<CosimSignals>,
}

impl NgspiceInputLatch {
    pub fn new(config: &CosimModelConfig) -> Result<Self, String> {
        Ok(Self {
            model_id: config.id.clone(),
            sampling: NgspiceInputSampling::from_config(config)?,
            held: None,
        })
    }

    pub fn sample(&mut self, step: CosimStep) -> Result<CosimStep, String> {
        if self.sampling == NgspiceInputSampling::CurrentBoundary {
            return Ok(step);
        }
        let inputs = match self.held.replace(step.inputs.clone()) {
            Some(held) => held,
            // The initialization step applies its own (initial) inputs.
            None if step.time_ns == 0 && step.dt_ns == 0 => step.inputs,
            None => {
                return Err(format!(
                    "ngspice model '{}' previous_boundary sampling requires the (time_ns = 0, dt_ns = 0) initialization step first",
                    self.model_id
                ))
            }
        };
        Ok(CosimStep { inputs, ..step })
    }
}

/// Voltage routes are already expressed in volts by the manifest contract.
/// Boolean and unitless routes are deliberately not presented as scope volts.
pub struct NgspiceTrace {
    names: Vec<String>,
    handle: Option<AnalogTraceHandle>,
    base: usize,
    steps: u64,
}

impl NgspiceTrace {
    pub fn new(config: &CosimModelConfig) -> Self {
        let mut names: Vec<_> = config
            .outputs
            .iter()
            .filter_map(|(name, path)| match SignalPath::parse(path) {
                Some(
                    SignalPath::AnalogPad { .. }
                    | SignalPath::AdcChannel { .. }
                    | SignalPath::SupplyRail { .. },
                ) => Some(name.clone()),
                _ => None,
            })
            .collect();
        if let Some(extra) = config
            .config
            .get("trace_volts")
            .and_then(|v| v.as_sequence())
        {
            names.extend(extra.iter().filter_map(|v| v.as_str().map(str::to_owned)));
        }
        names.sort();
        names.dedup();
        Self {
            names,
            handle: None,
            base: 0,
            steps: 0,
        }
    }

    pub fn attach(&mut self, handle: &AnalogTraceHandle, prefix: &str) {
        let channels: Vec<_> = self
            .names
            .iter()
            .map(|name| AnalogChannel::volts(format!("{prefix}{name}")))
            .collect();
        let mut trace = handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.base = trace.register_channels(&channels);
        self.handle = Some(handle.clone());
    }

    pub fn record(&mut self, time_ns: u64, outputs: &CosimSignals) -> Result<(), String> {
        if self.names.is_empty() {
            return Ok(());
        }
        let mut values = Vec::with_capacity(self.names.len());
        for name in &self.names {
            match outputs.get(name) {
                Some(CosimSignalValue::F64(value))
                    if value.is_finite() && (*value as f32).is_finite() =>
                {
                    values.push(*value)
                }
                _ => {
                    return Err(format!(
                        "ngspice voltage output {name} must be a finite number"
                    ))
                }
            }
        }
        self.steps += 1;
        if let Some(handle) = &self.handle {
            handle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(self.base, self.steps, time_ns, &values);
        }
        Ok(())
    }
}

/// One `config.input_sources` entry: `volts = value * scale + offset`.
#[derive(Debug, Clone, PartialEq)]
pub struct NgspiceInputSource {
    pub element: String,
    pub scale: f64,
    pub offset: f64,
    pub default_value: Option<CosimSignalValue>,
    /// The route is a `board.gpio*` pin: the value must be a logic level.
    pub logic: bool,
}

/// One `config.output_nodes` entry: `value = raw * scale + offset`, compared
/// with `threshold` (`value >= threshold`) when one is given.
#[derive(Debug, Clone, PartialEq)]
pub struct NgspiceOutputNode {
    pub node: String,
    pub scale: f64,
    pub offset: f64,
    pub threshold: Option<f64>,
}

/// A validated `adapter: ngspice` model, ready for a solver host.
#[derive(Debug, Clone, PartialEq)]
pub struct NgspiceModelSpec {
    pub model_id: String,
    pub netlist_path: PathBuf,
    /// The model's boundary period; the host checks its priming point
    /// against it.
    pub step_ns: u64,
    pub inputs: BTreeMap<String, NgspiceInputSource>,
    pub outputs: BTreeMap<String, NgspiceOutputNode>,
    /// Applied by the adapter before the host sees a frame.
    pub input_sampling: NgspiceInputSampling,
}

impl NgspiceModelSpec {
    /// Check a manifest entry against the browser runtime's rules and read the
    /// netlist it names, relative to `base_dir`.
    ///
    /// Native differences, both deliberate: routes are not limited to board
    /// paths (a CPU-free Flex manifest uses flat signal names), and a
    /// `threshold` is allowed on any output (it makes the output a logic
    /// level); it stays required for a `board.gpio_in` route.
    pub fn from_config(config: &CosimModelConfig, base_dir: &Path) -> Result<Self, String> {
        let id = &config.id;
        let fail = |message: String| format!("ngspice model '{id}' {message}");
        let model = config
            .model
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| fail("requires a model path (the netlist file)".into()))?;
        if config.step_ns == 0 {
            return Err(fail("step_ns must be greater than zero".into()));
        }
        let netlist_path = {
            let path = Path::new(model);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                base_dir.join(path)
            }
        };
        let netlist = std::fs::read_to_string(&netlist_path)
            .map_err(|err| fail(format!("netlist '{}': {err}", netlist_path.display())))?;
        check_netlist(&netlist).map_err(fail)?;

        let no_sources = serde_yaml::Mapping::new();
        let input_sources = mapping(config, "input_sources")?.unwrap_or(&no_sources);
        let mut inputs = BTreeMap::new();
        for (name, route) in &config.inputs {
            let entry = input_sources
                .get(name.as_str())
                .and_then(Yaml::as_mapping)
                .ok_or_else(|| fail(format!("config.input_sources.{name} is required")))?;
            let (scale, offset) = scale_offset(entry).ok_or_else(|| {
                fail(format!(
                    "config.input_sources.{name} scale and offset must be finite numbers"
                ))
            })?;
            let element = entry
                .get("element")
                .and_then(Yaml::as_str)
                .filter(|element| valid_element(element))
                .ok_or_else(|| fail(format!("config.input_sources.{name}.element is invalid")))?;
            let logic = route.starts_with("board.gpio");
            let default_value = match entry.get("default_value") {
                None => None,
                Some(Yaml::Bool(level)) if logic => Some(CosimSignalValue::Bool(*level)),
                Some(value) if !logic && value.as_f64().is_some_and(f64::is_finite) => {
                    Some(CosimSignalValue::F64(value.as_f64().unwrap()))
                }
                Some(_) => {
                    return Err(fail(format!(
                    "config.input_sources.{name}.default_value must match its routed signal type"
                )))
                }
            };
            inputs.insert(
                name.clone(),
                NgspiceInputSource {
                    element: element.to_string(),
                    scale,
                    offset,
                    default_value,
                    logic,
                },
            );
        }
        if let Some(extra) = input_sources
            .keys()
            .filter_map(Yaml::as_str)
            .find(|key| !config.inputs.contains_key(*key))
        {
            return Err(fail(format!(
                "config.input_sources.{extra} has no matching declared route"
            )));
        }

        let output_names = expected_outputs_checked(config).map_err(fail)?;
        let output_nodes = mapping(config, "output_nodes")?
            .ok_or_else(|| fail("config.output_nodes must be a mapping".into()))?;
        let mut outputs = BTreeMap::new();
        for name in &output_names {
            let entry = output_nodes
                .get(name.as_str())
                .and_then(Yaml::as_mapping)
                .ok_or_else(|| fail(format!("config.output_nodes.{name} is required")))?;
            let (scale, offset) = scale_offset(entry).ok_or_else(|| {
                fail(format!(
                    "config.output_nodes.{name} scale and offset must be finite numbers"
                ))
            })?;
            let node = entry
                .get("node")
                .and_then(Yaml::as_str)
                .filter(|node| valid_node(node))
                .ok_or_else(|| fail(format!("config.output_nodes.{name}.node is invalid")))?;
            let threshold = match entry.get("threshold") {
                None => None,
                Some(value) => Some(value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| {
                    fail(format!(
                        "config.output_nodes.{name}.threshold must be a finite number"
                    ))
                })?),
            };
            let gpio_in = config
                .outputs
                .get(name)
                .is_some_and(|route| route.starts_with("board.gpio_in."));
            if gpio_in && threshold.is_none() {
                return Err(fail(format!(
                    "config.output_nodes.{name}.threshold is required for a board.gpio_in route"
                )));
            }
            outputs.insert(
                name.clone(),
                NgspiceOutputNode {
                    node: node.to_string(),
                    scale,
                    offset,
                    threshold,
                },
            );
        }
        if let Some(extra) = output_nodes
            .keys()
            .filter_map(Yaml::as_str)
            .find(|key| !output_names.iter().any(|name| name == key))
        {
            return Err(fail(format!(
                "config.output_nodes.{extra} has no matching declared route"
            )));
        }
        Ok(Self {
            model_id: id.clone(),
            netlist_path,
            step_ns: config.step_ns,
            inputs,
            outputs,
            input_sampling: NgspiceInputSampling::from_config(config)?,
        })
    }

    /// The `--bridge-spec` argument of the native host.
    pub fn host_json(&self) -> String {
        let signal = |value: &Option<CosimSignalValue>| match value {
            Some(CosimSignalValue::Bool(level)) => json!(level),
            Some(CosimSignalValue::F64(number)) => json!(number),
            _ => Value::Null,
        };
        let inputs: serde_json::Map<_, _> = self
            .inputs
            .iter()
            .map(|(name, source)| {
                (
                    name.clone(),
                    json!({
                        "element": source.element,
                        "scale": source.scale,
                        "offset": source.offset,
                        "default_value": signal(&source.default_value),
                        "logic": source.logic,
                    }),
                )
            })
            .collect();
        let outputs: serde_json::Map<_, _> = self
            .outputs
            .iter()
            .map(|(name, node)| {
                (
                    name.clone(),
                    json!({
                        "node": node.node,
                        "scale": node.scale,
                        "offset": node.offset,
                        "threshold": node.threshold,
                    }),
                )
            })
            .collect();
        json!({
            "model_id": self.model_id,
            "netlist_path": self.netlist_path.to_string_lossy(),
            "step_ns": self.step_ns,
            "inputs": inputs,
            "outputs": outputs,
        })
        .to_string()
    }
}

fn mapping<'a>(
    config: &'a CosimModelConfig,
    key: &str,
) -> Result<Option<&'a serde_yaml::Mapping>, String> {
    match config.config.get(key) {
        None => Ok(None),
        Some(value) => value.as_mapping().map(Some).ok_or_else(|| {
            format!(
                "ngspice model '{}' config.{key} must be a mapping",
                config.id
            )
        }),
    }
}

fn expected_outputs_checked(config: &CosimModelConfig) -> Result<Vec<String>, String> {
    if let Some(value) = config.config.get("trace_volts") {
        let list = value
            .as_sequence()
            .ok_or("config.trace_volts must be a list of output names")?;
        if list
            .iter()
            .any(|entry| entry.as_str().is_none_or(|name| name.trim().is_empty()))
        {
            return Err("config.trace_volts must be a list of output names".into());
        }
    }
    Ok(expected_outputs(config))
}

fn scale_offset(entry: &serde_yaml::Mapping) -> Option<(f64, f64)> {
    let finite = |key: &str| entry.get(key)?.as_f64().filter(|v| v.is_finite());
    Some((finite("scale")?, finite("offset")?))
}

fn valid_element(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn valid_node(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-'))
}

/// The netlist checks the browser runtime applies before loading a circuit.
fn check_netlist(netlist: &str) -> Result<(), String> {
    if netlist.len() > MAX_NETLIST_BYTES {
        return Err(format!("netlist exceeds {MAX_NETLIST_BYTES} bytes"));
    }
    if netlist.lines().count() > MAX_NETLIST_LINES {
        return Err(format!("netlist exceeds {MAX_NETLIST_LINES} lines"));
    }
    if netlist.contains('\0') {
        return Err("netlist contains a null byte".into());
    }
    let directive = |line: &str, name: &str| {
        let line = line.trim_start().to_ascii_lowercase();
        line.strip_prefix(name)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
    };
    if netlist
        .lines()
        .any(|line| directive(line, ".control") || directive(line, ".endc"))
    {
        return Err("netlist contains a control block; control blocks are not supported".into());
    }
    let has_tran = netlist.lines().any(|line| directive(line, ".tran"));
    let has_end = netlist
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case(".end"));
    if !has_tran || !has_end {
        return Err("netlist must contain .tran and .end directives".into());
    }
    Ok(())
}

/// The native `adapter: ngspice` model: a libngspice host process behind the
/// `external_process` transport, with the shared output and trace checks.
pub struct NativeNgspiceAdapter {
    process: ExternalProcessCosimAdapter,
    latch: NgspiceInputLatch,
    model_id: String,
    expected_outputs: Vec<String>,
    trace: NgspiceTrace,
}

impl CosimAdapter for NativeNgspiceAdapter {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        let step = self.latch.sample(step).map_err(SimulationError::Other)?;
        // The same input checks as the browser request, before the frame
        // leaves the process.
        request_json(&self.model_id, &step).map_err(SimulationError::Other)?;
        let time_ns = step.time_ns;
        let mut result = self.process.step(step)?;
        // The transport reads integral JSON numbers as I64; the ngspice
        // contract is volts, so every number is an F64, as in the browser.
        for (name, value) in result.outputs.iter_mut() {
            *value = match value {
                CosimSignalValue::I64(number) => CosimSignalValue::F64(*number as f64),
                CosimSignalValue::Bool(_) | CosimSignalValue::F64(_) => continue,
                CosimSignalValue::Text(_) => {
                    return Err(SimulationError::Other(format!(
                        "co-sim model '{}' ngspice callback output '{}' must be a finite number or boolean",
                        self.model_id, name
                    )))
                }
            };
        }
        require_outputs(&self.model_id, &result, &self.expected_outputs)
            .map_err(SimulationError::Other)?;
        self.trace
            .record(time_ns, &result.outputs)
            .map_err(SimulationError::Other)?;
        Ok(result)
    }

    fn attach_analog_trace(&mut self, handle: &AnalogTraceHandle, prefix: &str) {
        self.trace.attach(handle, prefix);
    }
}

/// Build the native solver host for an `adapter: ngspice` model.
///
/// Needs `python3` and libngspice on the host (`apt install libngspice0`, or
/// `LABWIRED_NGSPICE_LIB`). The manifest is validated first, so a bad entry is
/// reported without either installed.
pub fn build_native_adapter(
    config: &CosimModelConfig,
    base_dir: &Path,
) -> SimResult<Box<dyn CosimAdapter>> {
    let spec = NgspiceModelSpec::from_config(config, base_dir).map_err(SimulationError::Other)?;
    let latch = NgspiceInputLatch::new(config).map_err(SimulationError::Other)?;
    let spec_json = spec.host_json();
    let process = ExternalProcessCosimAdapter::spawn_with_options(
        "python3",
        &["-c", NATIVE_HOST_SOURCE, "--bridge-spec", &spec_json],
        ExternalProcessCosimOptions {
            timeout: NATIVE_STEP_TIMEOUT,
            ..ExternalProcessCosimOptions::default()
        },
    )
    .map_err(|err| SimulationError::Other(format!("ngspice model '{}': {err}", config.id)))?;
    Ok(Box::new(NativeNgspiceAdapter {
        process,
        latch,
        model_id: config.id.clone(),
        expected_outputs: expected_outputs(config),
        trace: NgspiceTrace::new(config),
    }))
}

/// Why a machine runtime snapshot must not be taken or applied with these
/// models: an ngspice solver's state lives outside the machine, so a snapshot
/// would restore the firmware against a circuit at another time. `None` when
/// no model is `adapter: ngspice`.
pub fn runtime_snapshot_refusal(configs: &[CosimModelConfig]) -> Option<&'static str> {
    configs
        .iter()
        .any(|config| config.adapter == labwired_config::CosimAdapter::Ngspice)
        .then_some("runtime snapshots cannot capture or restore externally hosted ngspice state")
}

/// Validate every `adapter: ngspice` entry without starting a solver.
/// Issues are prefixed `cosim_models[i]` like [`super::validate_analog_models`].
pub fn validate_ngspice_models(configs: &[CosimModelConfig], base_dir: &Path) -> Vec<String> {
    configs
        .iter()
        .enumerate()
        .filter(|(_, config)| config.adapter == labwired_config::CosimAdapter::Ngspice)
        .filter_map(|(index, config)| {
            NgspiceModelSpec::from_config(config, base_dir)
                .err()
                .map(|err| format!("cosim_models[{index}] (id '{}'): {err}", config.id))
        })
        .collect()
}
