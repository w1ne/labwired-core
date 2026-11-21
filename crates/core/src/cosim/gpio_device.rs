// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Adapter for the catalog's pins-only `gpio_device` parts (DHT11, DHT22,
//! HC-SR04, ...).
//!
//! The part is the same [`DeclarativeGpioDevice`] the detailed path binds to
//! MCU pads. Here its pads are routed signals: an input alias that names a
//! descriptor pin role is the level the host drives, an output alias names a
//! driven role, and an input alias that names a descriptor input channel is a
//! stimulus in the channel's own unit. The device clock counts nanoseconds
//! (`cpu_hz` 1 GHz), so `timing: exact` schedules land on their own times.
//! An `open_drain_release` role publishes the wired-AND of the host and the
//! part, which is the line level with a pull-up.
//!
//! The runner samples at `step_ns`: a host level change reaches the part at the
//! next boundary, and a schedule edge between boundaries is published at the
//! boundary after it.

use std::collections::BTreeMap;

use anyhow::{bail, ensure, Context, Result};
use labwired_config::{CosimModelConfig, DeviceDescriptor, PinBinding};
use serde_yaml::Value;

use super::{CosimAdapter, CosimInputKind, CosimSignalValue, CosimStep, CosimStepResult};
use crate::bus::{BusResidentDevice, DevicePins};
use crate::peripherals::components::declarative_gpio::{
    validate_descriptor, BoundPin, DeclarativeGpioDevice,
};
use crate::sim_input::SimInput;
use crate::{SimResult, SimulationError};

/// The device clock: one cycle is one nanosecond of Flex time.
const DEVICE_HZ: u64 = 1_000_000_000;
/// Pad addresses of the driven roles start here; observed roles use their index.
const DRIVEN_BASE: u64 = 1 << 32;

/// The catalog descriptor of `device_type` when it is a `gpio_device` part.
pub(crate) fn catalog_gpio_device(device_type: &str) -> Result<Option<DeviceDescriptor>> {
    let lower = device_type.to_ascii_lowercase();
    let canonical = crate::peripherals::kit::registry::canonical_device_type(&lower);
    let descriptor = labwired_config::embedded_device_yaml(canonical)
        .map(DeviceDescriptor::from_yaml)
        .transpose()
        .with_context(|| format!("catalog type '{device_type}'"))?;
    Ok(descriptor.filter(|descriptor| descriptor.behavior.primitive == "gpio_device"))
}

pub(super) fn build_gpio_device_adapter(
    config: &CosimModelConfig,
    descriptor: DeviceDescriptor,
) -> SimResult<Box<dyn CosimAdapter>> {
    GpioDeviceAdapter::from_config(config, descriptor)
        .map(|adapter| Box::new(adapter) as Box<dyn CosimAdapter>)
        .map_err(|error| {
            SimulationError::Other(format!("gpio-device model '{}': {error:#}", config.id))
        })
}

/// Host levels of the observed roles and part levels of the driven roles.
struct Pads {
    observed: Vec<bool>,
    driven: Vec<bool>,
}

impl DevicePins for Pads {
    fn output_bit(&self, addr: u64, _bit: u8) -> Option<bool> {
        self.observed.get(addr as usize).copied()
    }

    fn drive_idr_bit(&mut self, addr: u64, _bit: u8, high: bool) {
        if let Some(level) = addr
            .checked_sub(DRIVEN_BASE)
            .and_then(|index| self.driven.get_mut(index as usize))
        {
            *level = high;
        }
    }

    fn drive_input_bit(&mut self, _addr: u64, _bit: u8, _high: bool) -> bool {
        false
    }
}

struct GpioDeviceAdapter {
    device: DeclarativeGpioDevice,
    pads: Pads,
    /// Observed pin roles, in pad order.
    observed_roles: Vec<String>,
    /// Driven roles, in pad order, and whether each one is routed.
    driven_roles: Vec<(String, bool)>,
    /// Input channel key → unit, for the routed channels.
    channel_units: BTreeMap<String, String>,
    channel_defaults: BTreeMap<String, f64>,
    /// Last value given to each routed channel; `None` before the first step.
    channel_values: BTreeMap<String, Option<f64>>,
}

impl GpioDeviceAdapter {
    fn from_config(config: &CosimModelConfig, descriptor: DeviceDescriptor) -> Result<Self> {
        ensure!(config.model.is_none(), "model paths are not supported");
        ensure!(config.step_ns > 0, "step_ns must be greater than zero");
        for key in config.config.keys() {
            ensure!(
                key == "device_type",
                "unsupported config key '{key}' for a catalog gpio_device"
            );
        }
        let device_type = config
            .config
            .get("device_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        validate_descriptor(&descriptor)?;

        let mut observed_roles = Vec::new();
        for (role, binding) in &descriptor.behavior.pins {
            if !matches!(binding, PinBinding::Scalar(_)) {
                bail!("pin role '{role}' of '{device_type}' is a pin list; only single pins are supported");
            }
            observed_roles.push(role.clone());
        }
        let driven_roles = descriptor.behavior.outputs.clone();
        ensure!(
            !config.outputs.is_empty(),
            "a gpio_device model must publish at least one driven role"
        );
        for alias in config.outputs.keys() {
            ensure!(
                driven_roles.contains(alias),
                "output '{alias}' is not a driven pin of '{device_type}' (driven: {driven_roles:?})"
            );
        }

        let channels = crate::peripherals::components::declarative_i2c::owned_channels(&descriptor);
        let mut channel_units = BTreeMap::new();
        let mut channel_defaults = BTreeMap::new();
        for alias in config.inputs.keys() {
            if observed_roles.contains(alias) {
                continue;
            }
            let channel = channels
                .iter()
                .find(|channel| channel.key == alias.as_str())
                .with_context(|| {
                    format!(
                        "input '{alias}' is neither a pin of '{device_type}' ({observed_roles:?}) \
                         nor one of its input channels"
                    )
                })?;
            // Dimensionless catalog channels use the unit `1`, as analog sources do.
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
        for role in &observed_roles {
            ensure!(
                config.inputs.contains_key(role),
                "pin '{role}' of '{device_type}' is not routed; an unrouted pin would read a level nobody drives"
            );
        }

        let observed = observed_roles
            .iter()
            .enumerate()
            .map(|(index, role)| BoundPin {
                role: role.clone(),
                addr: index as u64,
                bit: 0,
            })
            .collect();
        let driven = driven_roles
            .iter()
            .enumerate()
            .map(|(index, role)| BoundPin {
                role: role.clone(),
                addr: DRIVEN_BASE + index as u64,
                bit: 0,
            })
            .collect();
        let mut device = DeclarativeGpioDevice::new(
            config.id.clone(),
            &descriptor,
            observed,
            driven,
            DEVICE_HZ,
            channels,
        )?;
        // Unrouted channels keep the descriptor default, as on the detailed path.
        let specs = descriptor
            .metadata
            .as_ref()
            .map(|metadata| metadata.inputs.as_slice())
            .unwrap_or(&[]);
        for (key, value) in labwired_config::seeded_channel_values(specs, |_| None) {
            device.seed_input(&key, value);
        }
        let driven_roles: Vec<_> = driven_roles
            .into_iter()
            .map(|role| {
                let routed = config.outputs.contains_key(&role);
                (role, routed)
            })
            .collect();
        let pads = Pads {
            observed: vec![true; observed_roles.len()],
            driven: driven_roles
                .iter()
                .map(|(role, _)| {
                    descriptor
                        .behavior
                        .pin_defaults
                        .get(role)
                        .copied()
                        .unwrap_or(false)
                })
                .collect(),
        };
        let channel_values = channel_units
            .keys()
            .map(|key| (key.clone(), None))
            .collect();
        Ok(Self {
            device,
            pads,
            observed_roles,
            driven_roles,
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
            "gpio-device input '{alias}' requires a number, got {other:?}"
        ))),
    }
}

impl CosimAdapter for GpioDeviceAdapter {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        for (alias, last) in &mut self.channel_values {
            let value = number(
                alias,
                step.inputs.get(alias).ok_or_else(|| {
                    SimulationError::Other(format!("missing gpio-device input '{alias}'"))
                })?,
            )?;
            if *last != Some(value) {
                self.device.set_input(alias, value).map_err(|error| {
                    SimulationError::Other(format!("gpio-device input '{alias}': {error}"))
                })?;
                *last = Some(value);
            }
        }
        for (index, role) in self.observed_roles.iter().enumerate() {
            let value = step.inputs.get(role).ok_or_else(|| {
                SimulationError::Other(format!("missing gpio-device pin '{role}'"))
            })?;
            self.pads.observed[index] = number(role, value)? != 0.0;
        }
        self.device.service(&mut self.pads, step.time_ns);
        if let Some(fault) = self.device.schedule_fault() {
            return Err(SimulationError::Other(format!(
                "gpio-device schedule fault: {fault}"
            )));
        }
        let outputs = self
            .driven_roles
            .iter()
            .zip(&self.pads.driven)
            .filter(|((_, routed), _)| *routed)
            .map(|((role, _), level)| (role.clone(), CosimSignalValue::Bool(*level)))
            .collect();
        Ok(CosimStepResult { outputs })
    }

    fn input_kind(&self, name: &str) -> Option<CosimInputKind> {
        if self.observed_roles.iter().any(|role| role == name) {
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
        if self.observed_roles.iter().any(|role| role == name) {
            // A host pin that nobody drives yet is released to the pull-up.
            return Some(CosimSignalValue::Bool(true));
        }
        self.channel_defaults
            .get(name)
            .map(|value| CosimSignalValue::F64(*value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosim::build_cosim_adapter;

    fn dht11(extra: &str) -> SimResult<Box<dyn CosimAdapter>> {
        let config: CosimModelConfig = serde_yaml::from_str(&format!(
            "id: u4\nadapter: behavior\nstep_ns: 1000\n\
             inputs: {{ DATA: host, temperature: t, humidity: h{extra} }}\n\
             outputs: {{ DATA: line }}\nconfig: {{ device_type: dht11 }}\n"
        ))
        .unwrap();
        build_cosim_adapter(&config)
    }

    /// Step one model at 1 µs with the host level `host` and return the line.
    fn step(adapter: &mut dyn CosimAdapter, time_us: u64, host: bool, t: f64, h: f64) -> bool {
        let result = adapter
            .step(CosimStep {
                time_ns: time_us * 1000,
                dt_ns: 1000,
                inputs: [
                    ("DATA".to_owned(), CosimSignalValue::Bool(host)),
                    ("temperature".to_owned(), CosimSignalValue::F64(t)),
                    ("humidity".to_owned(), CosimSignalValue::F64(h)),
                ]
                .into(),
            })
            .unwrap();
        match result.outputs["DATA"] {
            CosimSignalValue::Bool(level) => level,
            ref other => panic!("line is {other:?}"),
        }
    }

    /// The line after a start pulse of `start_us`, one sample per µs.
    fn read(start_us: u64, t: f64, h: f64) -> Vec<bool> {
        let mut adapter = dht11("").unwrap();
        let mut line = Vec::new();
        for time_us in 0..start_us + 6000 {
            let host = !(10..10 + start_us).contains(&time_us);
            let level = step(adapter.as_mut(), time_us, host, t, h);
            if time_us >= 10 + start_us {
                line.push(level);
            }
        }
        line
    }

    /// Lengths of the runs of equal level, as (level, µs).
    fn runs(line: &[bool]) -> Vec<(bool, usize)> {
        let mut runs: Vec<(bool, usize)> = Vec::new();
        for &level in line {
            match runs.last_mut() {
                Some((last, count)) if *last == level => *count += 1,
                _ => runs.push((level, 1)),
            }
        }
        runs
    }

    #[test]
    fn a_catalog_dht11_answers_an_18_ms_start_with_its_frame_on_the_line() {
        let runs = runs(&read(18_000, 24.0, 55.0));
        // Response: 30 µs high, 80 low, 80 high; 40 bits of 50 low + 27/70
        // high; 50 low; idle high.
        assert_eq!(runs[0], (true, 30));
        assert_eq!(runs[1], (false, 80));
        assert_eq!(runs[2], (true, 80));
        let mut frame = 0u64;
        for bit in 0..40 {
            assert_eq!(runs[3 + 2 * bit], (false, 50), "bit {bit}");
            let (level, high_us) = runs[4 + 2 * bit];
            assert!(level);
            assert!(high_us == 27 || high_us == 70, "bit {bit}: {high_us} µs");
            frame = frame << 1 | u64::from(high_us == 70);
        }
        assert_eq!(runs[83], (false, 50));
        assert!(runs[84].0, "the line returns to the pull-up");
        assert_eq!(frame, 55 << 32 | 24 << 16 | 79);
    }

    #[test]
    fn a_start_pulse_shorter_than_one_ms_gets_no_answer() {
        assert!(read(500, 24.0, 55.0).iter().all(|level| *level));
    }

    #[test]
    fn the_host_holds_the_wired_and_line_low() {
        let mut adapter = dht11("").unwrap();
        assert!(step(adapter.as_mut(), 0, true, 24.0, 55.0));
        assert!(!step(adapter.as_mut(), 1, false, 24.0, 55.0));
    }

    #[test]
    fn wrong_routes_and_out_of_range_stimulus_are_errors() {
        let error = |result: SimResult<Box<dyn CosimAdapter>>| match result {
            Ok(_) => panic!("accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error(dht11(", pressure: p")).contains("neither a pin"));
        let config: CosimModelConfig = serde_yaml::from_str(
            "id: u4\nadapter: behavior\ninputs: { temperature: t }\n\
             outputs: { DATA: line }\nconfig: { device_type: dht11 }\n",
        )
        .unwrap();
        assert!(error(build_cosim_adapter(&config)).contains("pin 'DATA' of 'dht11' is not routed"));
        let mut adapter = dht11("").unwrap();
        let result = adapter.step(CosimStep {
            time_ns: 0,
            dt_ns: 0,
            inputs: [
                ("DATA".to_owned(), CosimSignalValue::Bool(true)),
                ("temperature".to_owned(), CosimSignalValue::F64(24.0)),
                ("humidity".to_owned(), CosimSignalValue::F64(120.0)),
            ]
            .into(),
        });
        assert!(
            result.is_err(),
            "120 %RH is outside the catalog channel range"
        );
    }
}
