// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CPU-free functional hardware for portable firmware.
//!
//! Firmware compiled for its host (including WebAssembly) calls named ports.
//! YAML behavior blocks and circuits run on the existing `CosimRunner` clock.
//! No `Cpu`, `Machine`, MMIO address map, second rule engine, or analog solver
//! is constructed here. Target binaries keep the separate machine path.

use crate::analog::AnalogTraceBatch;
use crate::cosim::{CosimRunner, CosimSignalValue, CosimSignals};
use crate::logic_capture::{LogicCapture, PadEvent};
use crate::{SimResult, SimulationError};
use labwired_config::{CosimAdapter, FlexManifest, FlexPort, FlexPortAccess, FlexPortKind};
use serde::Serialize;
use std::collections::BTreeMap;

/// Bound the amount of synchronous work in a host call. Portable firmware
/// should advance cooperatively; this is not an instruction-fuel mechanism.
const MAX_MODEL_STEPS_PER_ADVANCE: u64 = 100_000;
/// Serialized instrument timestamps must remain exact JavaScript integers.
const MAX_TIME_NS: u64 = (1 << 53) - 1;

#[derive(Debug, Clone, Serialize)]
pub struct FlexDigitalChannel {
    pub id: u32,
    pub signal: String,
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
    pub cursor: u64,
    pub dropped: u64,
    pub channels: Vec<FlexDigitalChannel>,
    pub edges: Vec<FlexDigitalEdge>,
}

struct DigitalCapture {
    capture: LogicCapture,
    channels: Vec<FlexDigitalChannel>,
    levels: Vec<bool>,
    events: Vec<PadEvent>,
}

impl DigitalCapture {
    fn new(signals: &CosimSignals) -> SimResult<Self> {
        let mut channels = Vec::new();
        for (signal, value) in signals {
            if let CosimSignalValue::Bool(initial) = value {
                let id = u32::try_from(channels.len())
                    .map_err(|_| error("too many digital channels"))?;
                channels.push(FlexDigitalChannel {
                    id,
                    signal: signal.clone(),
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
    ports: BTreeMap<String, u32>,
    runner: CosimRunner,
    signals: CosimSignals,
    digital: DigitalCapture,
    time_ns: u64,
    failure: Option<String>,
}

fn error(message: impl Into<String>) -> SimulationError {
    SimulationError::Other(message.into())
}

impl FlexRuntime {
    pub fn from_yaml(yaml: &str) -> SimResult<Self> {
        Self::from_yaml_with_factory(yaml, |config| {
            crate::cosim::build_cosim_adapter_with_base(config, std::path::Path::new("."))
        })
    }

    /// [`Self::from_yaml`] with adapters supplied by the host, as
    /// `CosimSession::new_with_factory` does for a machine: the browser
    /// passes its ngspice callback here. Priming, scheduling and checks are
    /// unchanged.
    pub fn from_yaml_with_factory<F>(yaml: &str, factory: F) -> SimResult<Self>
    where
        F: FnMut(
            &labwired_config::CosimModelConfig,
        ) -> SimResult<Box<dyn crate::cosim::CosimAdapter>>,
    {
        let manifest =
            FlexManifest::from_yaml(yaml).map_err(|err| error(format!("Flex model: {err:#}")))?;
        let mut runner = CosimRunner::from_configs_with_factory(&manifest.cosim_models, factory)?;
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
        let digital = DigitalCapture::new(&signals)?;
        let runtime = Self {
            manifest,
            ports,
            runner,
            signals,
            digital,
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
        self.signals.insert(port.signal.clone(), value);
        self.digital.sample(self.time_ns, &self.signals);
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
        let mut steps = 0u64;
        for model in &self.manifest.cosim_models {
            let boundaries = target / model.step_ns - self.time_ns / model.step_ns;
            // An analog boundary can contain many solver steps. Count them so
            // a huge YAML substeps value cannot bypass the synchronous budget.
            let cost = if model.adapter == CosimAdapter::Analog {
                model
                    .config
                    .get("substeps")
                    .and_then(serde_yaml::Value::as_u64)
                    .unwrap_or(10)
            } else {
                1
            };
            steps = steps.saturating_add(boundaries.saturating_mul(cost));
        }
        if steps > MAX_MODEL_STEPS_PER_ADVANCE {
            return Err(error(format!("Flex advance requests {steps} simulation steps (including analog substeps); maximum per call is {MAX_MODEL_STEPS_PER_ADVANCE}")));
        }
        let digital = &mut self.digital;
        let result = self
            .runner
            .step_until_with_signals_observed(target, &mut self.signals, |time_ns, signals| {
                digital.sample(time_ns, signals)
            })
            .and_then(|_| self.validate_observations());
        if let Err(failure) = result {
            self.failure = Some(failure.to_string());
            return Err(failure);
        }
        self.time_ns = target;
        Ok(())
    }

    pub fn digital_trace_snapshot(&mut self, cursor: u64) -> FlexDigitalTraceBatch {
        self.digital.snapshot(cursor)
    }

    pub fn analog_trace_snapshot(&self, cursor: u64) -> AnalogTraceBatch {
        self.runner.analog_trace_snapshot(cursor)
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
