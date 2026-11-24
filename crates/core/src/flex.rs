// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CPU-free functional hardware for portable firmware.
//!
//! Firmware compiled for its host (including WebAssembly) calls named ports.
//! YAML behavior blocks and circuits run on the existing `CosimRunner` clock.
//! No `Cpu`, `Machine`, MMIO address map, second rule engine, or analog solver
//! is constructed here. Target binaries keep the separate machine path.

use crate::analog::{
    AnalogCrossing, AnalogSample, AnalogSeries, AnalogTraceBatch, AnalogWindowTally,
};
use crate::cosim::{CosimRunner, CosimSignalValue, CosimSignals};
use crate::flex_console::{Console, FlexConsoleCapture, FlexConsoleSummary};
use crate::flex_i2c::{FlexI2cBusSummary, FlexI2cTraceBatch, FlexI2cTransfer, I2cBuses};
use crate::flex_sinks::{FlexSinkSummary, Sinks};
use crate::flex_spi::{FlexSpiBusSummary, FlexSpiTraceBatch, FlexSpiTransfer, SpiBuses};
use crate::logic_capture::{LogicCapture, PadEvent};
use crate::{SimResult, SimulationError};
use labwired_config::{
    CosimAdapter, CosimModelConfig, FlexManifest, FlexPort, FlexPortAccess, FlexPortKind,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

/// Work units of one boundary of `model`. An analog boundary can contain many
/// solver steps. Count them so a huge YAML substeps value cannot bypass the
/// synchronous budget.
fn model_step_cost(model: &CosimModelConfig) -> u64 {
    if model.adapter == CosimAdapter::Analog {
        model
            .config
            .get("substeps")
            .and_then(serde_yaml::Value::as_u64)
            .unwrap_or(10)
    } else {
        1
    }
}

/// Bound the amount of synchronous work in a host call. Portable firmware
/// should advance cooperatively; this is not an instruction-fuel mechanism.
const MAX_MODEL_STEPS_PER_ADVANCE: u64 = 100_000;
/// Serialized instrument timestamps must remain exact JavaScript integers.
const MAX_TIME_NS: u64 = labwired_config::FLEX_MAX_TIME_NS;

#[derive(Debug, Clone, Serialize)]
pub struct FlexDigitalChannel {
    pub id: u32,
    pub signal: String,
    /// Firmware-facing Boolean port names attached to this signal. Internal
    /// model signals have no aliases and remain available by their signal path.
    pub port_names: Vec<String>,
    pub initial: bool,
}

#[derive(Debug, Serialize)]
pub struct FlexDigitalEdge {
    pub ch: u32,
    pub time_ns: u64,
    pub value: u8,
}

/// The existing logic-analyzer capture, expressed in nanoseconds instead of
/// CPU cycles. These are sampled Boolean signals, not electrical pad states.
#[derive(Debug, Serialize)]
pub struct FlexDigitalTraceBatch {
    pub time_unit: &'static str,
    /// Monotonic edge cursor to pass to the next snapshot. This acknowledges
    /// retained edges through this cursor.
    pub cursor: u64,
    /// Cumulative edges lost to ring-buffer overflow since this runtime
    /// started. Non-zero means the oldest part of the trace has a gap.
    pub dropped: u64,
    pub channels: Vec<FlexDigitalChannel>,
    pub edges: Vec<FlexDigitalEdge>,
}

/// One oscilloscope channel with any firmware-facing numeric read ports that
/// observe the same model output. This keeps the reusable model name while
/// making the project's named port available to existing instrument views.
#[derive(Debug, Clone, Serialize)]
pub struct FlexAnalogChannel {
    pub name: String,
    pub unit: String,
    pub port_names: Vec<String>,
}

/// The existing co-simulation analog samples, annotated with Flex-facing
/// names and an explicit virtual-time unit for browser instruments.
#[derive(Debug, Serialize)]
pub struct FlexAnalogTraceBatch {
    pub time_unit: &'static str,
    pub channels: Vec<FlexAnalogChannel>,
    pub samples: Vec<AnalogSample>,
    /// Monotonic sample cursor to pass to the next snapshot. This cursor is
    /// independent from the digital edge cursor.
    pub next_cursor: u64,
    /// Cumulative samples lost to ring-buffer overflow since this runtime
    /// started. Non-zero means the oldest part of the trace has a gap.
    pub dropped: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlexSignalBinding {
    pub name: String,
    pub signal: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlexModelSummary {
    pub id: String,
    pub adapter: CosimAdapter,
    pub model_type: Option<String>,
    pub step_ns: u64,
    pub inputs: Vec<FlexSignalBinding>,
    pub outputs: Vec<FlexSignalBinding>,
    /// The validated data-driven model configuration, so project surfaces
    /// can show the exact behavior behind a run without reinterpreting YAML.
    pub config: BTreeMap<String, serde_yaml::Value>,
    /// The exact source bytes an adapter loaded, when it loaded any. An
    /// external netlist is outside the manifest hash, so this is what pins the
    /// circuit a run solved. Absent for models with no external source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::cosim::CosimSourcePin>,
}

/// Explicitly separate a portable behavior run from target-firmware evidence.
/// Flex cannot infer complete-board coverage or parity from a manifest, so
/// those claims stay unestablished in this runtime's report.
#[derive(Debug, Clone, Serialize)]
pub struct FlexExecutionScope {
    pub firmware: &'static str,
    pub original_target_firmware_executed: bool,
    pub detailed_target_execution_selected: bool,
    pub full_board_coverage: &'static str,
    pub chip_parity: &'static str,
}

/// Compact execution context plus the validated, reproducible model contract.
/// The browser binding and `labwired test` both emit this one report. Empty
/// scope notes remain visible through `scope_notes_complete` instead of being
/// treated as "no gaps".
#[derive(Debug, Clone, Serialize)]
pub struct FlexRunReport {
    pub execution_path: &'static str,
    pub target_cpu_emulated: bool,
    pub execution_scope: FlexExecutionScope,
    pub time_unit: &'static str,
    pub scope_notes_complete: bool,
    pub schema_version: u32,
    pub model_name: String,
    /// SHA-256 of the exact manifest bytes this runtime was built from.
    pub manifest_sha256: String,
    pub assumptions: Vec<String>,
    pub unsupported_checks: Vec<String>,
    pub ports: Vec<FlexPort>,
    pub models: Vec<FlexModelSummary>,
    /// The I2C buses and their catalog device models. Absent without a bus.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub i2c_buses: Vec<FlexI2cBusSummary>,
    /// The SPI buses and their catalog device models. Absent without a bus.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub spi_buses: Vec<FlexSpiBusSummary>,
    /// The console and what it leaves out. Absent without a console.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub console: Option<FlexConsoleSummary>,
    /// The listen-only devices. Absent without one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sinks: Vec<FlexSinkSummary>,
    /// Types the manifest's `parts:` packs define. Absent without one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub part_packs: Vec<String>,
}

fn signal_bindings(bindings: &HashMap<String, String>) -> Vec<FlexSignalBinding> {
    let mut result: Vec<_> = bindings
        .iter()
        .map(|(name, signal)| FlexSignalBinding {
            name: name.clone(),
            signal: signal.clone(),
        })
        .collect();
    result.sort_by(|left, right| left.name.cmp(&right.name));
    result
}

fn model_summary(
    model: &CosimModelConfig,
    source: Option<&crate::cosim::CosimSourcePin>,
) -> FlexModelSummary {
    FlexModelSummary {
        id: model.id.clone(),
        adapter: model.adapter,
        model_type: model
            .config
            .get("device_type")
            .and_then(serde_yaml::Value::as_str)
            .map(str::to_owned),
        step_ns: model.step_ns,
        inputs: signal_bindings(&model.inputs),
        outputs: signal_bindings(&model.outputs),
        config: model
            .config
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        source: source.cloned(),
    }
}

/// Exact running record of the edges of one logic-analyzer channel inside one
/// time window. The capture ring keeps the newest edges and drops the oldest;
/// a tally sees every edge at the moment it is recorded, so a long run does
/// not make its count unknowable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlexDigitalEdgeTally {
    pub channel: u32,
    /// The window is `from_ns <= time_ns <= to_ns`.
    pub from_ns: u64,
    pub to_ns: u64,
    /// Every edge of the channel inside the window.
    pub count: u64,
    /// The first edge times inside the window, at most the requested number.
    pub times_ns: Vec<u64>,
    #[serde(skip)]
    max_times: usize,
}

struct DigitalCapture {
    capture: LogicCapture,
    channels: Vec<FlexDigitalChannel>,
    levels: Vec<bool>,
    events: Vec<PadEvent>,
    tallies: Vec<FlexDigitalEdgeTally>,
    /// Edges recorded since the start, including those the ring dropped.
    recorded: u64,
}

impl DigitalCapture {
    fn new(signals: &CosimSignals, ports: &[FlexPort]) -> SimResult<Self> {
        let mut port_names_by_signal = BTreeMap::<String, Vec<String>>::new();
        for port in ports.iter().filter(|port| port.kind == FlexPortKind::Bool) {
            port_names_by_signal
                .entry(port.signal.clone())
                .or_default()
                .push(port.name.clone());
        }
        for names in port_names_by_signal.values_mut() {
            names.sort();
        }

        let mut channels = Vec::new();
        for (signal, value) in signals {
            if let CosimSignalValue::Bool(initial) = value {
                let id = u32::try_from(channels.len())
                    .map_err(|_| error("too many digital channels"))?;
                channels.push(FlexDigitalChannel {
                    id,
                    signal: signal.clone(),
                    port_names: port_names_by_signal
                        .get(signal)
                        .cloned()
                        .unwrap_or_default(),
                    initial: *initial,
                });
            }
        }
        let levels: Vec<_> = channels.iter().map(|channel| channel.initial).collect();
        let mut capture = LogicCapture::new();
        capture.install(
            &vec![None; channels.len()],
            &levels.iter().copied().map(Some).collect::<Vec<_>>(),
            &vec![true; channels.len()],
        );
        let events = Vec::with_capacity(channels.len());
        Ok(Self {
            capture,
            channels,
            levels,
            events,
            tallies: Vec::new(),
            recorded: 0,
        })
    }

    fn sample(&mut self, time_ns: u64, signals: &CosimSignals) {
        self.events.clear();
        for (channel, previous) in self.channels.iter().zip(&mut self.levels) {
            if let Some(CosimSignalValue::Bool(value)) = signals.get(&channel.signal) {
                if previous != value {
                    self.events.push(PadEvent {
                        ch: channel.id,
                        value: *value,
                        cycle: time_ns,
                        drive: None,
                    });
                    *previous = *value;
                }
            }
        }
        self.recorded += self.events.len() as u64;
        for tally in &mut self.tallies {
            if time_ns < tally.from_ns || time_ns > tally.to_ns {
                continue;
            }
            for event in self.events.iter().filter(|event| event.ch == tally.channel) {
                tally.count += 1;
                if tally.times_ns.len() < tally.max_times {
                    tally.times_ns.push(event.cycle);
                }
            }
        }
        // LogicCapture already handles bounded storage, overflow, cursors and
        // same-boundary channel order. One trace tick is exactly one ns here.
        self.capture.ingest_push(&self.events, time_ns, time_ns);
    }

    fn snapshot(&mut self, cursor: u64) -> FlexDigitalTraceBatch {
        let batch = self.capture.read_edges(cursor);
        FlexDigitalTraceBatch {
            time_unit: "ns",
            cursor: batch.cursor,
            dropped: batch.dropped,
            channels: self.channels.clone(),
            edges: batch
                .edges
                .into_iter()
                .map(|edge| FlexDigitalEdge {
                    ch: edge.ch,
                    time_ns: edge.cycle,
                    value: u8::from(edge.value),
                })
                .collect(),
        }
    }
}

pub struct FlexRuntime {
    manifest: FlexManifest,
    manifest_sha256: String,
    /// Model id -> the exact external source its adapter loaded.
    source_pins: BTreeMap<String, crate::cosim::CosimSourcePin>,
    ports: BTreeMap<String, u32>,
    runner: CosimRunner,
    signals: CosimSignals,
    digital: DigitalCapture,
    i2c: I2cBuses,
    spi: SpiBuses,
    sinks: Sinks,
    console: Option<Console>,
    time_ns: u64,
    failure: Option<String>,
}

fn error(message: impl Into<String>) -> SimulationError {
    SimulationError::Other(message.into())
}

fn model_device_type(model: &CosimModelConfig) -> Option<&str> {
    model
        .config
        .get("device_type")
        .and_then(serde_yaml::Value::as_str)
}

impl FlexRuntime {
    pub fn from_yaml(yaml: &str) -> SimResult<Self> {
        Self::from_yaml_with_base(yaml, std::path::Path::new("."))
    }

    /// [`Self::from_yaml`] with each relative `model:` resolved from
    /// `base_dir`, the manifest's directory.
    pub fn from_yaml_with_base(yaml: &str, base_dir: &std::path::Path) -> SimResult<Self> {
        Self::from_yaml_with_factory(yaml, |config| {
            crate::cosim::build_cosim_adapter_with_base(config, base_dir)
        })
    }

    /// [`Self::from_yaml`] with adapters supplied by the host, as
    /// `CosimSession::new_with_factory` does for a machine: the browser
    /// passes its ngspice callback here. Part-pack models are still built from
    /// their pack. Priming, scheduling and checks are unchanged.
    pub fn from_yaml_with_factory<F>(yaml: &str, mut factory: F) -> SimResult<Self>
    where
        F: FnMut(
            &labwired_config::CosimModelConfig,
        ) -> SimResult<Box<dyn crate::cosim::CosimAdapter>>,
    {
        let manifest =
            FlexManifest::from_yaml(yaml).map_err(|err| error(format!("Flex model: {err:#}")))?;
        crate::flex_parts::validate(&manifest.parts)?;
        // A model whose `config.device_type` names a part pack is built from
        // the pack; every other model is built by the host-supplied factory.
        let mut models = Vec::with_capacity(manifest.cosim_models.len());
        let mut source_pins = BTreeMap::new();
        for config in &manifest.cosim_models {
            let pack = model_device_type(config)
                .and_then(|device_type| crate::flex_parts::pack(&manifest.parts, device_type));
            let adapter = match pack {
                Some(pack) => crate::cosim::build_part_adapter(config, pack)?,
                None => factory(config)?,
            };
            // Ask before the adapter moves into the runner: a netlist file is
            // outside the manifest hash, and this is its identity.
            if let Some(pin) = adapter.source_pin() {
                source_pins.insert(config.id.clone(), pin);
            }
            models.push(crate::cosim::CosimRunnerModel::new(config.clone(), adapter));
        }
        let mut runner = CosimRunner::new(models);
        runner.set_record_routed(false);
        let mut signals = CosimSignals::new();
        let mut ports = BTreeMap::new();
        for (index, port) in manifest.ports.iter().enumerate() {
            let id = u32::try_from(index).map_err(|_| error("too many Flex ports"))?;
            ports.insert(port.name.clone(), id);
            if port.access == FlexPortAccess::Write {
                // The checked schema requires an explicit initial value.
                let initial = port
                    .initial
                    .ok_or_else(|| error(format!("port '{}' has no initial value", port.name)))?;
                signals.insert(port.signal.clone(), Self::port_value(port, initial)?);
            }
        }
        // Resolve the actual initial outputs before consumers sample them.
        // Cycles require an explicit initialization contract and are rejected.
        runner.prime_routed_with_signals(&mut signals)?;
        for model in &manifest.cosim_models {
            for path in model.inputs.values().chain(model.outputs.values()) {
                if !signals.contains_key(path) {
                    return Err(error(format!(
                        "model '{}' did not initialize routed signal '{path}'",
                        model.id
                    )));
                }
            }
        }
        let mut units = BTreeMap::new();
        for port in &manifest.ports {
            if port.access == FlexPortAccess::Write {
                if let Some(unit) = &port.unit {
                    units.insert(port.signal.clone(), unit.clone());
                }
            }
        }
        for channel in runner.analog_channels() {
            for model in &manifest.cosim_models {
                for (alias, signal) in &model.outputs {
                    if channel.name == format!("{}.{}", model.id, alias) {
                        units.insert(signal.clone(), channel.unit.clone());
                    }
                }
            }
        }
        for model in &manifest.cosim_models {
            let analog_source = model_device_type(model).is_some_and(|device_type| {
                match crate::flex_parts::descriptor(&manifest.parts, device_type) {
                    Ok(Some(descriptor)) => descriptor.behavior.primitive == "analog_source",
                    // A registered Rust analog-source kit has no descriptor.
                    _ => true,
                }
            });
            if model.adapter == CosimAdapter::Behavior && analog_source {
                for signal in model.outputs.values() {
                    units.insert(signal.clone(), "mV".to_owned());
                }
            }
        }
        for port in &manifest.ports {
            if port.access == FlexPortAccess::Read
                && port.kind == FlexPortKind::Number
                && units.get(&port.signal) != port.unit.as_ref()
            {
                return Err(error(format!(
                    "port '{}' declares unit {:?}, but '{}' produces {:?}; no implicit unit conversion",
                    port.name, port.unit, port.signal, units.get(&port.signal)
                )));
            }
        }
        runner.validate_numeric_input_units(&signals, &units)?;
        let mut i2c = I2cBuses::new(&manifest.i2c_buses, &units, &manifest.parts)?;
        i2c.sync_inputs(&signals)?;
        let mut spi = SpiBuses::new(&manifest.spi_buses, &units, &manifest.parts)?;
        spi.sync_inputs(&signals)?;
        let sinks = Sinks::new(&manifest.sinks, &manifest.parts, &signals)?;
        let digital = DigitalCapture::new(&signals, &manifest.ports)?;
        let console = manifest.console.as_ref().map(Console::new);
        let runtime = Self {
            manifest,
            manifest_sha256: format!("{:x}", Sha256::digest(yaml.as_bytes())),
            source_pins,
            ports,
            runner,
            signals,
            digital,
            i2c,
            spi,
            sinks,
            console,
            time_ns: 0,
            failure: None,
        };
        runtime.validate_observations()?;
        Ok(runtime)
    }

    pub fn manifest(&self) -> &FlexManifest {
        &self.manifest
    }

    pub fn time_ns(&self) -> u64 {
        self.time_ns
    }

    pub fn run_report(&self) -> FlexRunReport {
        let manifest = &self.manifest;
        FlexRunReport {
            execution_path: "flex_portable_wasm",
            target_cpu_emulated: false,
            execution_scope: FlexExecutionScope {
                firmware: "portable_behavior",
                original_target_firmware_executed: false,
                detailed_target_execution_selected: false,
                full_board_coverage: "not_established",
                chip_parity: "not_established",
            },
            time_unit: "ns",
            scope_notes_complete: !manifest.assumptions.is_empty()
                && !manifest.unsupported_checks.is_empty(),
            schema_version: manifest.schema_version,
            model_name: manifest.name.clone(),
            manifest_sha256: self.manifest_sha256.clone(),
            assumptions: manifest.assumptions.clone(),
            unsupported_checks: manifest.unsupported_checks.clone(),
            ports: manifest.ports.clone(),
            models: manifest
                .cosim_models
                .iter()
                .map(|model| model_summary(model, self.source_pins.get(&model.id)))
                .collect(),
            i2c_buses: self.i2c.summaries().to_vec(),
            spi_buses: self.spi.summaries().to_vec(),
            console: self.console.as_ref().map(Console::summary),
            sinks: self.sinks.summaries().to_vec(),
            part_packs: manifest
                .parts
                .iter()
                .filter_map(labwired_config::PartPack::descriptor)
                .map(|pack| pack.r#type.clone())
                .collect(),
        }
    }

    /// What every device holds at the current time, as inspect artifacts
    /// named by the manifest id: sinks, bus devices, then models. This is
    /// the device state a `device_state` assertion reads.
    pub fn device_artifacts(&self) -> Vec<crate::inspect::Artifact> {
        let mut artifacts = self.sinks.artifacts();
        artifacts.extend(self.i2c.artifacts());
        artifacts.extend(self.spi.artifacts());
        artifacts.extend(self.runner.model_artifacts());
        artifacts
    }

    pub fn port_id(&self, name: &str) -> SimResult<u32> {
        self.ports
            .get(name)
            .copied()
            .ok_or_else(|| error(format!("unknown Flex port '{name}'")))
    }

    fn ensure_healthy(&self) -> SimResult<()> {
        if let Some(failure) = &self.failure {
            return Err(error(format!("Flex runtime stopped: {failure}")));
        }
        Ok(())
    }

    fn port(&self, id: u32, access: FlexPortAccess) -> SimResult<&FlexPort> {
        self.ensure_healthy()?;
        let port = self
            .manifest
            .ports
            .get(id as usize)
            .ok_or_else(|| error(format!("unknown Flex port id {id}")))?;
        if port.access != access {
            return Err(error(format!(
                "port '{}' does not permit {access:?} access",
                port.name
            )));
        }
        Ok(port)
    }

    fn port_value(port: &FlexPort, value: f64) -> SimResult<CosimSignalValue> {
        if !value.is_finite() {
            return Err(error(format!(
                "port '{}' requires a finite value",
                port.name
            )));
        }
        match port.kind {
            FlexPortKind::Bool if value == 0.0 || value == 1.0 => {
                Ok(CosimSignalValue::Bool(value == 1.0))
            }
            FlexPortKind::Bool => Err(error(format!(
                "boolean port '{}' accepts only 0 or 1",
                port.name
            ))),
            FlexPortKind::Number => Ok(CosimSignalValue::F64(value)),
        }
    }

    pub fn write(&mut self, port_id: u32, value: f64) -> SimResult<()> {
        let port = self.port(port_id, FlexPortAccess::Write)?;
        let value = Self::port_value(port, value)?;
        // Firmware that refreshes every pin on every pass writes mostly
        // unchanged levels. No signal changes, so there is no edge to record.
        if self.signals.get(&port.signal) == Some(&value) {
            return Ok(());
        }
        self.signals.insert(port.signal.clone(), value);
        self.digital.sample(self.time_ns, &self.signals);
        self.sinks.sample(self.time_ns, &self.signals);
        // A bus device takes its input at the write, so the next transfer
        // reads it. A value the device rejects stops the runtime.
        if let Err(failure) = self
            .i2c
            .sync_inputs(&self.signals)
            .and_then(|_| self.spi.sync_inputs(&self.signals))
        {
            self.failure = Some(failure.to_string());
            return Err(failure);
        }
        Ok(())
    }

    pub fn read(&self, port_id: u32) -> SimResult<f64> {
        let port = self.port(port_id, FlexPortAccess::Read)?;
        match (port.kind, self.signals.get(&port.signal)) {
            (FlexPortKind::Bool, Some(CosimSignalValue::Bool(value))) => {
                Ok(if *value { 1.0 } else { 0.0 })
            }
            (FlexPortKind::Number, Some(CosimSignalValue::F64(value))) if value.is_finite() => {
                Ok(*value)
            }
            (FlexPortKind::Number, Some(CosimSignalValue::I64(value))) => Ok(*value as f64),
            _ => Err(error(format!(
                "port '{}' has no valid {:?} observation from '{}'",
                port.name, port.kind, port.signal
            ))),
        }
    }

    fn validate_observations(&self) -> SimResult<()> {
        for (path, value) in &self.signals {
            if matches!(value, CosimSignalValue::F64(number) if !number.is_finite()) {
                return Err(error(format!("signal '{path}' is non-finite")));
            }
        }
        for (index, port) in self.manifest.ports.iter().enumerate() {
            if port.access == FlexPortAccess::Read {
                self.read(index as u32)?;
            }
        }
        Ok(())
    }

    /// Advance the shared model scheduler by explicit simulated time.
    /// Models due at the same boundary read the same pre-boundary signals.
    /// An adapter error latches permanently: a partially advanced model may
    /// never be resumed as though that failed step had not happened.
    pub fn advance_ns(&mut self, delta_ns: u64) -> SimResult<()> {
        self.ensure_healthy()?;
        let target = self
            .time_ns
            .checked_add(delta_ns)
            .ok_or_else(|| error("Flex time overflow"))?;
        if target > MAX_TIME_NS {
            return Err(error("Flex time exceeds exact nanosecond trace range"));
        }
        let steps = self
            .manifest
            .cosim_models
            .iter()
            .map(|model| {
                let boundaries = target / model.step_ns - self.time_ns / model.step_ns;
                boundaries.saturating_mul(model_step_cost(model))
            })
            .fold(0u64, u64::saturating_add);
        if steps > MAX_MODEL_STEPS_PER_ADVANCE {
            return Err(error(format!("Flex advance requests {steps} simulation steps (including analog substeps); maximum per call is {MAX_MODEL_STEPS_PER_ADVANCE}")));
        }
        let digital = &mut self.digital;
        let sinks = &mut self.sinks;
        let result = self
            .runner
            .step_until_with_signals_observed(target, &mut self.signals, |time_ns, signals| {
                digital.sample(time_ns, signals);
                sinks.sample(time_ns, signals);
            })
            .and_then(|_| self.validate_observations())
            .and_then(|_| self.i2c.sync_inputs(&self.signals))
            .and_then(|_| self.spi.sync_inputs(&self.signals));
        if let Err(failure) = result {
            self.failure = Some(failure.to_string());
            return Err(failure);
        }
        self.i2c.advance_ns(delta_ns);
        self.spi.advance_ns(delta_ns);
        if !self.sinks.is_empty() {
            self.sinks.service(target);
        }
        self.time_ns = target;
        Ok(())
    }

    /// [`Self::advance_ns`] in pieces that each fit the per-call work
    /// budget. For a host that bounds the whole run in another way, as the
    /// script runner does with `limits.max_time_ns`: a firmware wait of one
    /// second is ordinary, and with a 1 µs model step it is 1,000,000 steps.
    /// A failed piece latches as in [`Self::advance_ns`].
    pub fn advance_ns_in_pieces(&mut self, delta_ns: u64) -> SimResult<()> {
        let models = self.manifest.cosim_models.len() as u64;
        // Each model gets an equal share of the budget. A piece of
        // `(share - 1) * step_ns` crosses at most `share` boundaries of it.
        let piece_ns = self
            .manifest
            .cosim_models
            .iter()
            .map(|model| {
                let share = MAX_MODEL_STEPS_PER_ADVANCE / models / model_step_cost(model).max(1);
                share.saturating_sub(1).saturating_mul(model.step_ns)
            })
            .min()
            .unwrap_or(u64::MAX);
        if piece_ns == 0 {
            // One boundary is already over the budget: report it.
            return self.advance_ns(delta_ns);
        }
        let mut remaining = delta_ns;
        loop {
            let piece = remaining.min(piece_ns);
            self.advance_ns(piece)?;
            remaining -= piece;
            if remaining == 0 {
                return Ok(());
            }
        }
    }

    pub fn i2c_bus_id(&self, name: &str) -> SimResult<u32> {
        self.i2c.bus_id(name)
    }

    /// One I2C transfer as the bus controller: write `write` to the 7-bit
    /// `address`, then read `read_len` bytes after a repeated START, then
    /// STOP. An address no device answers is `ack: false`, not an error.
    ///
    /// Virtual time does not move here. The caller advances it by the
    /// returned `duration_ns`, through its own time limits.
    pub fn i2c_transfer(
        &mut self,
        bus_id: u32,
        address: u32,
        write: &[u8],
        read_len: usize,
    ) -> SimResult<FlexI2cTransfer> {
        self.ensure_healthy()?;
        self.i2c
            .transfer(bus_id, self.time_ns, address, write, read_len)
    }

    /// Exact number of transfers on a bus since the start, optionally of one
    /// address and one outcome. `None`: no such bus.
    pub fn i2c_transfer_count(
        &self,
        bus_id: u32,
        address: Option<u8>,
        ack: Option<bool>,
    ) -> Option<u64> {
        self.i2c.transfer_count(bus_id, address, ack)
    }

    pub fn i2c_trace_snapshot(&self, cursor: u64) -> FlexI2cTraceBatch {
        self.i2c.snapshot(cursor)
    }

    pub fn spi_bus_id(&self, name: &str) -> SimResult<u32> {
        self.spi
            .bus_id(name)
            .ok_or_else(|| error(format!("unknown Flex SPI bus '{name}'")))
    }

    /// The id of a manifest bus of either kind, for a `{ bus: name }` entry
    /// argument. Bus names are unique across both kinds.
    pub fn bus_id(&self, name: &str) -> SimResult<u32> {
        self.i2c
            .bus_id(name)
            .or_else(|_| self.spi_bus_id(name))
            .map_err(|_| error(format!("unknown Flex bus '{name}'")))
    }

    /// One SPI transfer as the bus controller: select `cs`, shift out `mosi`
    /// one byte at a time while the device shifts in the returned `miso`, and
    /// release `cs`. A chip select with no device is `selected: false` and
    /// returns no MISO bytes, not an error.
    ///
    /// Virtual time does not move here. The caller advances it by the
    /// returned `duration_ns`, through its own time limits.
    pub fn spi_transfer(
        &mut self,
        bus_id: u32,
        cs: u32,
        mosi: &[u8],
    ) -> SimResult<FlexSpiTransfer> {
        self.ensure_healthy()?;
        self.spi.transfer(bus_id, self.time_ns, cs, mosi)
    }

    /// Exact number of transfers on a bus since the start, optionally of one
    /// chip select and one outcome. `None`: no such bus.
    pub fn spi_transfer_count(
        &self,
        bus_id: u32,
        cs: Option<u32>,
        selected: Option<bool>,
    ) -> Option<u64> {
        self.spi.transfer_count(bus_id, cs, selected)
    }

    pub fn spi_trace_snapshot(&self, cursor: u64) -> FlexSpiTraceBatch {
        self.spi.snapshot(cursor)
    }

    /// Append firmware output to the manifest console at the current virtual
    /// time. Virtual time does not move. A manifest without a console rejects
    /// the write: the output would have no declared destination.
    pub fn console_write(&mut self, bytes: &[u8]) -> SimResult<()> {
        self.ensure_healthy()?;
        let time_ns = self.time_ns;
        let console = self.console.as_mut().ok_or_else(|| {
            error("labwired.console_write: the Flex manifest declares no console")
        })?;
        console.write(time_ns, bytes);
        Ok(())
    }

    /// The console output so far. `None`: the manifest has no console.
    pub fn console_capture(&self) -> Option<FlexConsoleCapture> {
        self.console.as_ref().map(Console::capture)
    }

    pub fn digital_trace_snapshot(&mut self, cursor: u64) -> FlexDigitalTraceBatch {
        self.digital.snapshot(cursor)
    }

    /// Start an exact tally of the edges of logic-analyzer channel `channel`
    /// with `from_ns <= time_ns <= to_ns`, keeping at most `max_times` edge
    /// times. Returns the tally id. A tally cannot see the past, so it must
    /// start before the first edge is recorded.
    pub fn tally_digital_edges(
        &mut self,
        channel: u32,
        from_ns: u64,
        to_ns: u64,
        max_times: usize,
    ) -> SimResult<usize> {
        if self.digital.recorded != 0 {
            return Err(error(
                "a digital edge tally must start before the first edge is recorded",
            ));
        }
        if !self
            .digital
            .channels
            .iter()
            .any(|known| known.id == channel)
        {
            return Err(error(format!("unknown Flex digital channel id {channel}")));
        }
        if from_ns > to_ns {
            return Err(error("digital edge tally window is empty"));
        }
        self.digital.tallies.push(FlexDigitalEdgeTally {
            channel,
            from_ns,
            to_ns,
            count: 0,
            times_ns: Vec::new(),
            max_times,
        });
        Ok(self.digital.tallies.len() - 1)
    }

    pub fn digital_edge_tally(&self, id: usize) -> Option<&FlexDigitalEdgeTally> {
        self.digital.tallies.get(id)
    }

    /// Start an exact tally of oscilloscope channel `channel` (its index in
    /// [`FlexAnalogTraceBatch::channels`]) with `from_ns <= time_ns <= to_ns`:
    /// lowest and highest value, the newest sample, and with `level` the
    /// crossings of that level. Returns the tally id. Must start at 0 ns.
    pub fn tally_analog_window(
        &mut self,
        channel: usize,
        from_ns: u64,
        to_ns: u64,
        level: Option<(f32, AnalogCrossing)>,
        max_times: usize,
    ) -> SimResult<usize> {
        if self.time_ns != 0 {
            return Err(error("an analog tally must start at 0 ns"));
        }
        self.runner
            .analog_trace_registry()
            .tally_window(channel, from_ns, to_ns, level, max_times)
            .map_err(error)
    }

    /// The analog tally `id` as it stands at the current time.
    pub fn analog_window_tally(&self, id: usize) -> Option<AnalogWindowTally> {
        self.runner
            .analog_trace_registry()
            .window_tally(id, self.time_ns)
    }

    /// The run-long change-point series of every oscilloscope channel, the
    /// block the detailed path writes as `result.json`'s `analog_series`.
    /// Unlike the scope ring it still holds the start of a long run. A point
    /// is written when a value leaves the series deadband; the exact answers
    /// of the test assertions come from the tallies, not from this series.
    pub fn analog_series(&self) -> AnalogSeries {
        self.runner.analog_trace_registry().series()
    }

    /// The unannotated analog engine batch, for the exporters that already
    /// write `Machine::analog_trace_snapshot` (CSV and VCD).
    pub fn analog_engine_trace(&self, cursor: u64) -> AnalogTraceBatch {
        self.runner.analog_trace_snapshot(cursor)
    }

    pub fn analog_trace_snapshot(&self, cursor: u64) -> FlexAnalogTraceBatch {
        let batch: AnalogTraceBatch = self.runner.analog_trace_snapshot(cursor);
        let channels = batch
            .channels
            .into_iter()
            .map(|channel| {
                let signal = self.manifest.cosim_models.iter().find_map(|model| {
                    model.outputs.iter().find_map(|(output, signal)| {
                        (channel.name == format!("{}.{}", model.id, output))
                            .then_some(signal.as_str())
                    })
                });
                let mut port_names: Vec<_> = signal
                    .into_iter()
                    .flat_map(|signal| {
                        self.manifest.ports.iter().filter_map(move |port| {
                            (port.access == FlexPortAccess::Read
                                && port.kind == FlexPortKind::Number
                                && port.signal == signal)
                                .then_some(port.name.clone())
                        })
                    })
                    .collect();
                port_names.sort();
                FlexAnalogChannel {
                    name: channel.name,
                    unit: channel.unit,
                    port_names,
                }
            })
            .collect();
        FlexAnalogTraceBatch {
            time_unit: "ns",
            channels,
            samples: batch.samples,
            next_cursor: batch.next_cursor,
            dropped: batch.dropped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PULSE: &str = r#"
schema_version: 1
name: sampled pulse
ports:
  - {name: command, signal: command, access: write, kind: bool, initial: 0}
  - {name: pulse, signal: pulse, access: read, kind: bool}
cosim_models:
  # The consumer intentionally comes first: initialization follows connections.
  - id: pulse
    adapter: behavior
    step_ns: 1000
    inputs: { enable: command, delayed: ready }
    outputs: { pulse: pulse }
    config:
      behavior:
        primitive: gpio_device
        outputs: [pulse]
        pin_defaults: { pulse: false }
        rules:
          - on: { input: enable }
            do: [{ pin: pulse, level: 'input(enable) != input(delayed)' }]
          - on: { input: delayed }
            do: [{ pin: pulse, level: 'input(enable) != input(delayed)' }]
  - id: delay
    adapter: behavior
    step_ns: 1000
    inputs: { enable: command }
    outputs: { ready: ready }
    config:
      behavior:
        primitive: gpio_device
        outputs: [ready]
        pin_defaults: { ready: false }
        rules:
          - on: { input: enable }
            do: [{ pin: ready, level: 'input(enable)' }]
"#;

    #[test]
    fn digital_capture_keeps_an_intermediate_pulse_during_one_host_advance() {
        let mut runtime = FlexRuntime::from_yaml(PULSE).unwrap();
        let command = runtime.port_id("command").unwrap();
        let pulse = runtime.port_id("pulse").unwrap();
        assert_eq!(runtime.read(pulse).unwrap(), 0.0);
        runtime.write(command, 1.0).unwrap();
        runtime.advance_ns(5_000).unwrap();
        assert_eq!(runtime.read(pulse).unwrap(), 0.0);
        let trace = runtime.digital_trace_snapshot(0);
        assert_eq!(trace.time_unit, "ns");
        assert_eq!(trace.dropped, 0);
        let channel = trace
            .channels
            .iter()
            .find(|channel| channel.signal == "pulse")
            .unwrap();
        assert_eq!(channel.port_names, vec!["pulse"]);
        let command_channel = trace
            .channels
            .iter()
            .find(|channel| channel.signal == "command")
            .unwrap();
        assert_eq!(command_channel.port_names, vec!["command"]);
        let internal_channel = trace
            .channels
            .iter()
            .find(|channel| channel.signal == "ready")
            .unwrap();
        assert!(internal_channel.port_names.is_empty());
        let edges: Vec<_> = trace
            .edges
            .iter()
            .filter(|edge| edge.ch == channel.id)
            .map(|edge| (edge.time_ns, edge.value))
            .collect();
        assert_eq!(edges, [(1_000, 1), (2_000, 0)]);
        assert!(runtime
            .digital_trace_snapshot(trace.cursor)
            .edges
            .is_empty());
    }

    #[test]
    fn edge_tally_stays_exact_after_the_capture_ring_overflows() {
        use crate::logic_capture::LOGIC_RING_CAPACITY;

        let mut runtime = FlexRuntime::from_yaml(PULSE).unwrap();
        let command = runtime.port_id("command").unwrap();
        let channel = |runtime: &mut FlexRuntime, signal: &str| {
            let trace = runtime.digital_trace_snapshot(0);
            trace
                .channels
                .iter()
                .find(|channel| channel.signal == signal)
                .unwrap()
                .id
        };
        let command_channel = channel(&mut runtime, "command");
        let ready_channel = channel(&mut runtime, "ready");
        let whole = runtime
            .tally_digital_edges(command_channel, 0, MAX_TIME_NS, 4)
            .unwrap();
        // `ready` follows `command` one 1000 ns model step later.
        let window = runtime
            .tally_digital_edges(ready_channel, 3_000, 5_000, 8)
            .unwrap();
        assert!(runtime.tally_digital_edges(u32::MAX, 0, 1, 1).is_err());
        assert!(runtime
            .tally_digital_edges(command_channel, 2, 1, 1)
            .is_err());

        // Short run: the tally and the ring hold the same edges.
        for level in [1.0, 0.0, 1.0, 0.0, 1.0, 0.0] {
            runtime.write(command, level).unwrap();
            runtime.advance_ns(1_000).unwrap();
        }
        let trace = runtime.digital_trace_snapshot(0);
        assert_eq!(trace.dropped, 0);
        let ring_times = |ch: u32| -> Vec<u64> {
            trace
                .edges
                .iter()
                .filter(|edge| edge.ch == ch)
                .map(|edge| edge.time_ns)
                .collect()
        };
        let tally = runtime.digital_edge_tally(whole).unwrap();
        assert_eq!(tally.count, 6);
        assert_eq!(
            ring_times(command_channel),
            [0, 1_000, 2_000, 3_000, 4_000, 5_000]
        );
        // Only the first four times were requested.
        assert_eq!(tally.times_ns, [0, 1_000, 2_000, 3_000]);
        // Both window ends are inside the window.
        let tally = runtime.digital_edge_tally(window).unwrap();
        assert_eq!(
            ring_times(ready_channel),
            [1_000, 2_000, 3_000, 4_000, 5_000, 6_000]
        );
        assert_eq!(
            (tally.count, tally.times_ns.as_slice()),
            (3, &[3_000, 4_000, 5_000][..])
        );

        // Long run: the ring drops its oldest edges; the tally does not.
        let toggles = LOGIC_RING_CAPACITY as u64;
        for index in 0..toggles {
            runtime.write(command, f64::from(index % 2 == 0)).unwrap();
            runtime.advance_ns(1_000).unwrap();
        }
        let trace = runtime.digital_trace_snapshot(0);
        assert!(trace.dropped > 0);
        assert_eq!(trace.edges.len(), LOGIC_RING_CAPACITY);
        let tally = runtime.digital_edge_tally(whole).unwrap();
        assert_eq!(tally.count, 6 + toggles);
        assert_eq!(tally.times_ns.len(), 4);
        assert_eq!(runtime.digital_edge_tally(window).unwrap().count, 3);
        // A tally cannot see the past.
        assert!(runtime
            .tally_digital_edges(command_channel, 0, MAX_TIME_NS, 1)
            .is_err());
    }

    #[test]
    fn initial_dependency_cycles_are_rejected_without_default_values() {
        // delay now consumes pulse, and pulse consumes ready: neither has a
        // declared starting value. Firmware still drives pulse's enable input.
        let cyclic = PULSE.replace("inputs: { enable: command }", "inputs: { enable: pulse }");
        let error = FlexRuntime::from_yaml(&cyclic).err().expect("reject cycle");
        assert!(error
            .to_string()
            .contains("cyclic or missing initial inputs"));
    }

    #[test]
    fn excessive_work_and_inexact_trace_time_fail_before_advancing() {
        let mut runtime = FlexRuntime::from_yaml(PULSE).unwrap();
        assert!(runtime.advance_ns(100_001_000).is_err());
        assert_eq!(runtime.time_ns(), 0);
        assert!(runtime.advance_ns(MAX_TIME_NS + 1).is_err());
        assert_eq!(runtime.time_ns(), 0);
        // A rejected request is not a partially executed model failure.
        runtime.advance_ns(1_000).unwrap();
        assert_eq!(runtime.time_ns(), 1_000);
    }

    /// A firmware wait longer than one call's budget runs in pieces and gives
    /// the same trace as the same wait in small calls.
    #[test]
    fn advance_in_pieces_covers_a_wait_over_the_call_budget() {
        let wait_ns = 1_000_000_000;
        let mut whole = FlexRuntime::from_yaml(PULSE).unwrap();
        let command = whole.port_id("command").unwrap();
        whole.write(command, 1.0).unwrap();
        assert!(whole.advance_ns(wait_ns).is_err());
        assert_eq!(whole.time_ns(), 0);
        whole.advance_ns_in_pieces(wait_ns).unwrap();
        assert_eq!(whole.time_ns(), wait_ns);

        let mut small = FlexRuntime::from_yaml(PULSE).unwrap();
        small.write(command, 1.0).unwrap();
        for _ in 0..1_000 {
            small.advance_ns(1_000_000).unwrap();
        }
        let edges = |runtime: &mut FlexRuntime| {
            serde_json::to_string(&runtime.digital_trace_snapshot(0).edges).unwrap()
        };
        let whole_edges = edges(&mut whole);
        assert_ne!(whole_edges, "[]");
        assert_eq!(whole_edges, edges(&mut small));
    }

    /// An external source is outside the manifest hash. The run report must
    /// carry the adapter's own pin, and that pin must move when the source
    /// moves while the manifest stays byte-identical.
    #[test]
    fn an_external_source_is_pinned_in_the_run_report() {
        use crate::cosim::{CosimAdapter, CosimSourcePin, CosimStep, CosimStepResult};

        struct PinnedStub(&'static str);
        impl CosimAdapter for PinnedStub {
            fn step(&mut self, _step: CosimStep) -> SimResult<CosimStepResult> {
                let mut outputs = crate::cosim::CosimSignals::new();
                outputs.insert("out".into(), crate::cosim::CosimSignalValue::Bool(false));
                Ok(CosimStepResult { outputs })
            }
            fn source_pin(&self) -> Option<CosimSourcePin> {
                Some(CosimSourcePin::spice_netlist_file("./board.cir", self.0))
            }
        }

        let yaml = r#"
schema_version: 1
name: pinned source
ports:
  - {name: flag, signal: m.out, access: read, kind: bool}
cosim_models:
  - id: m
    adapter: behavior
    step_ns: 1000
    outputs: { out: m.out }
"#;
        let build = |netlist| {
            FlexRuntime::from_yaml_with_factory(yaml, |_model| Ok(Box::new(PinnedStub(netlist))))
        };
        let report = build("R1 1 0 1k\n").unwrap().run_report();
        let pin = report.models[0]
            .source
            .as_ref()
            .expect("an external source is pinned");
        assert_eq!(pin.kind, "spice_netlist");
        assert_eq!(pin.origin, "file");
        assert_eq!(pin.path.as_deref(), Some("./board.cir"));
        assert_eq!(
            pin.sha256,
            format!("{:x}", sha2::Sha256::digest(b"R1 1 0 1k\n"))
        );

        // Change only the source: the manifest digest stays, the source pin
        // does not. Without this field the two runs would look identical.
        let edited = build("R1 1 0 2k\n").unwrap().run_report();
        let edited_pin = edited.models[0].source.as_ref().unwrap();
        assert_eq!(edited.manifest_sha256, report.manifest_sha256);
        assert_ne!(edited_pin.sha256, pin.sha256);
    }

    /// An inline netlist is hashable too, and says so instead of naming a
    /// path it does not have.
    #[test]
    fn an_inline_netlist_is_pinned_without_a_path() {
        let yaml = r#"
schema_version: 1
name: inline rc
ports:
  - {name: volts, signal: rc.out, access: read, kind: number, unit: V}
cosim_models:
  - id: rc
    adapter: analog
    step_ns: 1000
    outputs: { out: rc.out }
    config:
      input_sampling: previous_boundary
      vdd: 3.3
      netlist_text: |
        Vsrc in 0 dc 0
        R1 in out 1k
        C1 out 0 1u
      probes: { out: 'v(out)' }
"#;
        let report = FlexRuntime::from_yaml(yaml).unwrap().run_report();
        let pin = report.models[0]
            .source
            .as_ref()
            .expect("an inline netlist is pinned");
        assert_eq!(pin.kind, "spice_netlist");
        assert_eq!(pin.origin, "inline");
        assert_eq!(pin.path, None);
        assert_eq!(
            pin.bytes,
            "Vsrc in 0 dc 0\nR1 in out 1k\nC1 out 0 1u\n".len()
        );
        assert_eq!(
            pin.sha256,
            format!(
                "{:x}",
                sha2::Sha256::digest(b"Vsrc in 0 dc 0\nR1 in out 1k\nC1 out 0 1u\n")
            )
        );
    }

    #[test]
    fn analog_solver_substeps_are_included_in_the_work_budget() {
        let yaml = r#"
schema_version: 1
name: bounded analog
ports:
  - {name: voltage, signal: voltage, access: read, kind: number, unit: V}
cosim_models:
  - id: rc
    adapter: analog
    step_ns: 1000
    outputs: { voltage: voltage }
    config:
      input_sampling: previous_boundary
      substeps: 4294967295
      netlist_text: |
        Vsrc in 0 dc 0
        R1 in out 10k
        C1 out 0 100n
      probes: { voltage: 'v(out)' }
"#;
        let mut runtime = FlexRuntime::from_yaml(yaml).unwrap();
        assert!(runtime.advance_ns(1_000).is_err());
        assert_eq!(runtime.time_ns(), 0);
        assert_eq!(runtime.analog_trace_snapshot(0).samples.len(), 1);
    }
}
