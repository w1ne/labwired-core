// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Adapter for the existing data-driven `analog_source` PeripheralKits.
//!
//! The kit registry owns the part models and `SimInput` owns their stimulus
//! validation. This adapter only connects declared scalar signals to that
//! existing model and publishes its millivolt output into CosimRunner.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, MutexGuard};

use labwired_config::CosimModelConfig;
use serde_yaml::Value;

use crate::analog::{AnalogChannel, AnalogTrace, AnalogTraceHandle};
use crate::bus::sim_inputs::AnalogSource;
use crate::cosim::{CosimAdapter, CosimInputKind, CosimSignalValue, CosimStep, CosimStepResult};
use crate::peripherals::kit::registry::build_analog_source;
use crate::{SimResult, SimulationError};

/// Construct one registered analog-source kit. The config keys are local
/// channel aliases; each must name one channel exposed by the part descriptor.
pub(super) fn build_analog_source_adapter(
    config: &CosimModelConfig,
) -> SimResult<Box<dyn CosimAdapter>> {
    AnalogSourceAdapter::from_config(config, build_analog_source)
        .map(|adapter| Box::new(adapter) as Box<dyn CosimAdapter>)
        .map_err(|error| {
            SimulationError::Other(format!("analog-source model '{}': {error:#}", config.id))
        })
}

/// [`build_analog_source_adapter`] for a part described by `descriptor`
/// rather than registered by type: a manifest part pack.
pub(super) fn build_analog_source_adapter_from(
    config: &CosimModelConfig,
    descriptor: &labwired_config::DeviceDescriptor,
) -> SimResult<Box<dyn CosimAdapter>> {
    let build = |_: &str, channel: u8| -> anyhow::Result<Box<dyn AnalogSource>> {
        use crate::peripherals::kit::PeripheralKit;
        let yaml = serde_yaml::to_string(descriptor)?;
        let kit =
            crate::peripherals::components::declarative_analog::DeclarativeAnalogKit::from_yaml(
                &yaml,
            )?;
        kit.build_analog_source(channel)?
            .ok_or_else(|| anyhow::anyhow!("'{}' is not an analog_source", descriptor.r#type))
    };
    AnalogSourceAdapter::from_config(config, build)
        .map(|adapter| Box::new(adapter) as Box<dyn CosimAdapter>)
        .map_err(|error| {
            SimulationError::Other(format!("analog-source model '{}': {error:#}", config.id))
        })
}

struct AnalogSourceAdapter {
    source: Box<dyn AnalogSource>,
    input_units: HashMap<String, String>,
    input_defaults: HashMap<String, CosimSignalValue>,
    output_alias: String,
    trace: Option<(AnalogTraceHandle, usize)>,
    trace_cycle: u64,
}

impl AnalogSourceAdapter {
    fn from_config(
        config: &CosimModelConfig,
        build: impl FnOnce(&str, u8) -> anyhow::Result<Box<dyn AnalogSource>>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(config.model.is_none(), "model paths are not supported");
        anyhow::ensure!(config.step_ns > 0, "step_ns must be greater than zero");
        anyhow::ensure!(
            config
                .config
                .keys()
                .all(|key| key == "device_type" || key == "channel")
                && config.config.contains_key("device_type"),
            "config must contain device_type and may contain channel"
        );
        let device_type = config
            .config
            .get("device_type")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("config.device_type must be a nonempty string"))?;
        anyhow::ensure!(
            config.outputs.len() == 1,
            "an analog_source must declare exactly one output (millivolts)"
        );

        let channel =
            match config.config.get("channel") {
                None => 0,
                Some(value) => u8::try_from(value.as_u64().ok_or_else(|| {
                    anyhow::anyhow!("config.channel must be an integer in 0..=255")
                })?)
                .map_err(|_| anyhow::anyhow!("config.channel must be an integer in 0..=255"))?,
            };
        let source = build(device_type, channel)?;
        let channels = source.input_channels();
        anyhow::ensure!(
            !channels.is_empty(),
            "registered analog_source '{device_type}' has no input channels"
        );
        anyhow::ensure!(
            config.inputs.len() == channels.len(),
            "model declares {} inputs but analog_source '{device_type}' requires {} channels",
            config.inputs.len(),
            channels.len()
        );

        let mut input_units = HashMap::new();
        let mut input_defaults = HashMap::new();
        for channel in channels {
            anyhow::ensure!(
                config.inputs.contains_key(channel.key.as_ref()),
                "model is missing analog_source input '{}'",
                channel.key
            );
            // Catalog descriptors use an empty unit for dimensionless numeric
            // inputs (for example `usb_present` on `lipo_charger`). Flex ports
            // require an explicit unit token, so publish that same contract as
            // the conventional dimensionless unit `1`.
            let unit = if channel.unit.is_empty() {
                "1"
            } else {
                channel.unit.as_ref()
            };
            input_units.insert(channel.key.to_string(), unit.to_owned());
            if let Some(default) = channel.default {
                input_defaults.insert(channel.key.to_string(), CosimSignalValue::F64(default));
            }
        }
        for alias in config.inputs.keys() {
            anyhow::ensure!(
                input_units.contains_key(alias),
                "'{alias}' is not an input channel on analog_source '{device_type}'"
            );
        }

        Ok(Self {
            source,
            input_units,
            input_defaults,
            output_alias: config.outputs.keys().next().unwrap().clone(),
            trace: None,
            trace_cycle: 0,
        })
    }
}

impl CosimAdapter for AnalogSourceAdapter {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        for channel in self.input_units.keys() {
            let value = step.inputs.get(channel).ok_or_else(|| {
                SimulationError::Other(format!("missing analog-source input '{channel}'"))
            })?;
            let number = match value {
                CosimSignalValue::F64(value) => *value,
                CosimSignalValue::I64(value) => *value as f64,
                other => {
                    return Err(SimulationError::Other(format!(
                        "analog-source input '{channel}' requires a number, got {other:?}"
                    )))
                }
            };
            self.source.set_input(channel, number).map_err(|error| {
                SimulationError::Other(format!("analog-source input '{channel}': {error}"))
            })?;
        }

        let output_mv = self.source.output_mv();
        if let Some((trace, channel_base)) = &self.trace {
            lock_analog_trace(trace).push(
                *channel_base,
                self.trace_cycle,
                step.time_ns,
                &[f64::from(output_mv)],
            );
        }
        self.trace_cycle = self.trace_cycle.saturating_add(1);

        Ok(CosimStepResult {
            outputs: BTreeMap::from([(
                self.output_alias.clone(),
                CosimSignalValue::I64(i64::from(output_mv)),
            )]),
        })
    }

    fn attach_analog_trace(&mut self, trace: &AnalogTraceHandle, channel_prefix: &str) {
        let channel = AnalogChannel {
            name: format!("{channel_prefix}{}", self.output_alias),
            unit: "mV".to_owned(),
        };
        let channel_base = lock_analog_trace(trace).register_channels(&[channel]);
        self.trace = Some((Arc::clone(trace), channel_base));
    }

    fn input_kind(&self, name: &str) -> Option<CosimInputKind> {
        self.input_units
            .contains_key(name)
            .then_some(CosimInputKind::Number)
    }

    fn input_unit(&self, name: &str) -> Option<&str> {
        self.input_units.get(name).map(String::as_str)
    }

    fn input_initial_value(&self, name: &str) -> Option<CosimSignalValue> {
        self.input_defaults.get(name).cloned()
    }
}

fn lock_analog_trace(trace: &AnalogTraceHandle) -> MutexGuard<'_, AnalogTrace> {
    match trace.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
