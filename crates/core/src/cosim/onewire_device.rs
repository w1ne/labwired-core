// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Adapter for `onewire_device` parts (DS18B20, ...).
//!
//! The part is the [`OneWireDevice`] the descriptor describes. Its one pin
//! role is routed twice: the input alias is the level the host drives (a
//! released host pin is high), and the output alias publishes the line, the
//! wired-AND of the host and the part, which is what a pulled-up open-drain
//! line reads. Any other input alias names one of the part's input channels,
//! in the channel's own unit.
//!
//! The runner samples at `step_ns`, so a host edge reaches the part at the
//! next boundary. 1-Wire slots are tens of microseconds; a 1 µs step keeps
//! every slot inside the sampling window the AN126 timing allows.

use std::collections::BTreeMap;

use anyhow::{ensure, Context, Result};
use labwired_config::{CosimModelConfig, DeviceDescriptor, PinBinding};

use super::{CosimAdapter, CosimInputKind, CosimSignalValue, CosimStep, CosimStepResult};
use crate::peripherals::components::declarative_onewire::OneWireDevice;
use crate::sim_input::SimInput;
use crate::{SimResult, SimulationError};

pub(super) fn build_onewire_adapter(
    config: &CosimModelConfig,
    descriptor: &DeviceDescriptor,
) -> SimResult<Box<dyn CosimAdapter>> {
    OneWireAdapter::from_config(config, descriptor)
        .map(|adapter| Box::new(adapter) as Box<dyn CosimAdapter>)
        .map_err(|error| {
            SimulationError::Other(format!("onewire-device model '{}': {error:#}", config.id))
        })
}

struct OneWireAdapter {
    device: OneWireDevice,
    role: String,
    channel_units: BTreeMap<String, String>,
    channel_defaults: BTreeMap<String, f64>,
    channel_values: BTreeMap<String, Option<f64>>,
}

impl OneWireAdapter {
    fn from_config(config: &CosimModelConfig, descriptor: &DeviceDescriptor) -> Result<Self> {
        ensure!(config.model.is_none(), "model paths are not supported");
        ensure!(config.step_ns > 0, "step_ns must be greater than zero");
        let mut settings = BTreeMap::new();
        for (key, value) in &config.config {
            match key.as_str() {
                "device_type" => {}
                "serial_number" => {
                    settings.insert(
                        key.clone(),
                        value
                            .as_f64()
                            .context("config.serial_number must be an integer")?,
                    );
                }
                other => anyhow::bail!("unsupported config key '{other}' for an onewire_device"),
            }
        }
        let device = OneWireDevice::from_descriptor(config.id.clone(), descriptor, |key| {
            settings.get(key).copied()
        })?;
        let (role, binding) = descriptor
            .behavior
            .pins
            .iter()
            .next()
            .context("an onewire_device has one pin role")?;
        ensure!(
            matches!(binding, PinBinding::Scalar(_)),
            "pin role '{role}' must be a single pin"
        );
        ensure!(
            config.inputs.contains_key(role),
            "pin '{role}' is not routed: the part would never see the host"
        );
        ensure!(
            config.outputs.len() == 1 && config.outputs.contains_key(role),
            "an onewire_device publishes exactly its line '{role}'"
        );
        let mut channel_units = BTreeMap::new();
        let mut channel_defaults = BTreeMap::new();
        for alias in config.inputs.keys().filter(|alias| *alias != role) {
            let channel = device
                .input_channels()
                .iter()
                .find(|channel| channel.key == alias.as_str())
                .with_context(|| {
                    format!("input '{alias}' is neither the pin '{role}' nor an input channel")
                })?;
            let unit = if channel.unit.is_empty() {
                "1"
            } else {
                channel.unit.as_ref()
            };
            channel_units.insert(alias.clone(), unit.to_owned());
            if let Some(default) = channel.default {
                channel_defaults.insert(alias.clone(), default);
            }
        }
        let channel_values = channel_units.keys().map(|k| (k.clone(), None)).collect();
        Ok(Self {
            device,
            role: role.clone(),
            channel_units,
            channel_defaults,
            channel_values,
        })
    }
}

fn number(alias: &str, value: &CosimSignalValue) -> SimResult<f64> {
    match value {
        CosimSignalValue::F64(value) => Ok(*value),
        CosimSignalValue::I64(value) => Ok(*value as f64),
        CosimSignalValue::Bool(value) => Ok(f64::from(u8::from(*value))),
        other => Err(SimulationError::Other(format!(
            "onewire-device input '{alias}' requires a number, got {other:?}"
        ))),
    }
}

impl CosimAdapter for OneWireAdapter {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        for (alias, last) in &mut self.channel_values {
            let value = number(
                alias,
                step.inputs.get(alias).ok_or_else(|| {
                    SimulationError::Other(format!("missing onewire-device input '{alias}'"))
                })?,
            )?;
            if *last != Some(value) {
                self.device.set_input(alias, value).map_err(|error| {
                    SimulationError::Other(format!("onewire-device input '{alias}': {error}"))
                })?;
                *last = Some(value);
            }
        }
        let host = number(
            &self.role,
            step.inputs.get(&self.role).ok_or_else(|| {
                SimulationError::Other(format!("missing onewire-device pin '{}'", self.role))
            })?,
        )? != 0.0;
        self.device.host_level(step.time_ns, host);
        self.device.service(step.time_ns);
        let line = host && !self.device.pulls_low(step.time_ns);
        Ok(CosimStepResult {
            outputs: [(self.role.clone(), CosimSignalValue::Bool(line))]
                .into_iter()
                .collect(),
        })
    }

    fn input_kind(&self, name: &str) -> Option<CosimInputKind> {
        if name == self.role {
            Some(CosimInputKind::Bool)
        } else {
            self.channel_units
                .contains_key(name)
                .then_some(CosimInputKind::Number)
        }
    }

    fn input_unit(&self, name: &str) -> Option<&str> {
        self.channel_units.get(name).map(String::as_str)
    }

    fn input_initial_value(&self, name: &str) -> Option<CosimSignalValue> {
        if name == self.role {
            return Some(CosimSignalValue::Bool(true));
        }
        self.channel_defaults
            .get(name)
            .map(|value| CosimSignalValue::F64(*value))
    }

    fn artifacts(&self, id: &str) -> Vec<crate::inspect::Artifact> {
        self.device.artifacts(id)
    }
}
