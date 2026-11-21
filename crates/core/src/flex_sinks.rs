// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Listen-only devices of the CPU-free Flex path (`sinks:`).
//!
//! A sink is a part that only receives pin levels: an addressable LED strip,
//! a display driver clocked by the firmware (TM1637), a shift register that
//! only lights LEDs. It drives no signal, so it is not a co-simulation model
//! that must publish an output. It takes every edge of its pins at the exact
//! virtual time the signal changed (a firmware write, or the model boundary
//! that published it), so a protocol timed in nanoseconds (WS2812 NRZ bits)
//! decodes without stepping a model every few nanoseconds. What it holds is
//! its device state: the inspect artifacts of the same part model the
//! detailed path attaches to MCU pads.

use std::collections::BTreeMap;

use labwired_config::{DeviceDescriptor, FlexSink, LedStripWire, PartPack, PinBinding};

use crate::bus::{BusResidentDevice, DevicePins};
use crate::cosim::{CosimSignalValue, CosimSignals};
use crate::peripherals::components::declarative_gpio::{BoundPin, DeclarativeGpioDevice};
use crate::peripherals::components::declarative_led_strip::GenericLedStrip;
use crate::peripherals::device::GpioObserver;
use crate::{SimResult, SimulationError};

/// The device clock of a sink: one cycle is one nanosecond of Flex time.
const DEVICE_HZ: u64 = 1_000_000_000;
/// Changed LED-strip frames a sink keeps, oldest dropped first and counted.
const FRAME_HISTORY_CAPACITY: usize = 1024;

fn error(message: impl Into<String>) -> SimulationError {
    SimulationError::Other(message.into())
}

/// Host levels of a gpio sink's pins. A sink drives nothing.
struct Pads {
    observed: Vec<bool>,
}

impl DevicePins for Pads {
    fn output_bit(&self, addr: u64, _bit: u8) -> Option<bool> {
        self.observed.get(addr as usize).copied()
    }

    fn drive_idr_bit(&mut self, _addr: u64, _bit: u8, _high: bool) {}

    fn drive_input_bit(&mut self, _addr: u64, _bit: u8, _high: bool) -> bool {
        false
    }
}

enum Model {
    Strip(GenericLedStrip),
    Gpio {
        device: Box<DeclarativeGpioDevice>,
        pads: Pads,
    },
}

struct Sink {
    id: String,
    /// `(signal, last level)` per pad, in pad order.
    pins: Vec<(String, bool)>,
    model: Model,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FlexSinkSummary {
    pub id: String,
    pub model_type: String,
    pub primitive: String,
    pub pins: BTreeMap<String, String>,
}

pub(crate) struct Sinks {
    sinks: Vec<Sink>,
    summaries: Vec<FlexSinkSummary>,
}

fn level(signals: &CosimSignals, signal: &str) -> SimResult<bool> {
    match signals.get(signal) {
        Some(CosimSignalValue::Bool(value)) => Ok(*value),
        other => Err(error(format!(
            "sink signal '{signal}' must be a Boolean level, got {other:?}"
        ))),
    }
}

fn strip(config: &FlexSink, descriptor: &DeviceDescriptor) -> SimResult<Model> {
    let path = format!("sink '{}'", config.id);
    let mut strip = GenericLedStrip::from_descriptor(descriptor)
        .map_err(|cause| error(format!("{path}: {cause:#}")))?;
    let spec = descriptor.behavior.led_strip.as_ref().expect("validated");
    if spec.wire != LedStripWire::NrzGpio {
        return Err(error(format!(
            "{path}: '{}' is a clocked (spi_frames) strip; only a single-wire strip is a pin sink",
            descriptor.r#type
        )));
    }
    if config.pins.len() != 1 {
        return Err(error(format!(
            "{path}: a single-wire strip has one data pin, got {}",
            config.pins.len()
        )));
    }
    for key in config.config.keys() {
        if key != "num_pixels" {
            return Err(error(format!(
                "{path}: config key '{key}' is not supported (num_pixels is)"
            )));
        }
    }
    if let Some(value) = config.config.get("num_pixels") {
        let pixels = value
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(|| error(format!("{path}: num_pixels must be a positive integer")))?;
        strip.set_num_pixels(pixels as usize);
    }
    strip.set_cpu_hz(DEVICE_HZ);
    strip.keep_frame_history(FRAME_HISTORY_CAPACITY);
    strip.set_component_id(config.id.clone());
    strip.set_data_pin(0);
    Ok(Model::Strip(strip))
}

fn gpio(config: &FlexSink, descriptor: &DeviceDescriptor) -> SimResult<(Model, Vec<String>)> {
    let path = format!("sink '{}'", config.id);
    crate::peripherals::components::declarative_gpio::validate_descriptor(descriptor)
        .map_err(|cause| error(format!("{path}: {cause:#}")))?;
    let mut roles = Vec::new();
    for (role, binding) in &descriptor.behavior.pins {
        if !matches!(binding, PinBinding::Scalar(_)) {
            return Err(error(format!(
                "{path}: pin role '{role}' is a pin list; only single pins are supported"
            )));
        }
        if !config.pins.contains_key(role) {
            return Err(error(format!(
                "{path}: pin '{role}' of '{}' is not bound; an unbound pin would read a level nobody drives",
                descriptor.r#type
            )));
        }
        roles.push(role.clone());
    }
    for role in config.pins.keys() {
        if !roles.contains(role) {
            return Err(error(format!(
                "{path}: '{role}' is not a pin of '{}' ({roles:?})",
                descriptor.r#type
            )));
        }
    }
    let observed = roles
        .iter()
        .enumerate()
        .map(|(index, role)| BoundPin {
            role: role.clone(),
            addr: index as u64,
            bit: 0,
        })
        .collect();
    // Driven roles bind to no pad: a sink publishes no signal.
    let channels = crate::peripherals::components::declarative_i2c::owned_channels(descriptor);
    let mut device = DeclarativeGpioDevice::new(
        config.id.clone(),
        descriptor,
        observed,
        Vec::new(),
        DEVICE_HZ,
        channels,
    )
    .map_err(|cause| error(format!("{path}: {cause:#}")))?;
    let specs = descriptor
        .metadata
        .as_ref()
        .map(|metadata| metadata.inputs.as_slice())
        .unwrap_or(&[]);
    let settings = &config.config;
    for (key, value) in labwired_config::seeded_channel_values(specs, |key| {
        settings.get(key).and_then(serde_yaml::Value::as_f64)
    }) {
        device.seed_input(&key, value);
    }
    let pads = Pads {
        observed: vec![true; roles.len()],
    };
    Ok((
        Model::Gpio {
            device: Box::new(device),
            pads,
        },
        roles,
    ))
}

impl Sinks {
    pub(crate) fn new(
        configs: &[FlexSink],
        parts: &[PartPack],
        signals: &CosimSignals,
    ) -> SimResult<Self> {
        let mut sinks = Vec::with_capacity(configs.len());
        let mut summaries = Vec::with_capacity(configs.len());
        for config in configs {
            let path = format!("sink '{}'", config.id);
            let descriptor = crate::flex_parts::descriptor(parts, &config.device_type)?
                .ok_or_else(|| {
                    error(format!(
                        "{path}: no part pack or catalog descriptor of type '{}'",
                        config.device_type
                    ))
                })?;
            let primitive = descriptor.behavior.primitive.clone();
            let (model, roles) = match primitive.as_str() {
                "led_strip" => (
                    strip(config, &descriptor)?,
                    config.pins.keys().cloned().collect(),
                ),
                "gpio_device" => {
                    if !config.config.is_empty() {
                        for key in config.config.keys() {
                            let declared = descriptor
                                .metadata
                                .as_ref()
                                .is_some_and(|m| m.inputs.iter().any(|i| {
                                    i.config_key.as_deref().unwrap_or(&i.key) == key
                                }));
                            if !declared {
                                return Err(error(format!(
                                    "{path}: config key '{key}' is not an input start value of '{}'",
                                    descriptor.r#type
                                )));
                            }
                        }
                    }
                    gpio(config, &descriptor)?
                }
                other => {
                    return Err(error(format!(
                        "{path}: '{}' is a {other} part; a sink is a listen-only led_strip or gpio_device",
                        descriptor.r#type
                    )))
                }
            };
            let mut pins = Vec::with_capacity(roles.len());
            for role in &roles {
                let signal = config.pins[role].clone();
                let initial = level(signals, &signal)?;
                pins.push((signal, initial));
            }
            let mut sink = Sink {
                id: config.id.clone(),
                pins,
                model,
            };
            sink.prime();
            summaries.push(FlexSinkSummary {
                id: config.id.clone(),
                model_type: config.device_type.clone(),
                primitive,
                pins: config.pins.clone(),
            });
            sinks.push(sink);
        }
        Ok(Self { sinks, summaries })
    }

    pub(crate) fn summaries(&self) -> &[FlexSinkSummary] {
        &self.summaries
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }

    /// Hand every pin whose signal changed to its sink, at `time_ns`.
    pub(crate) fn sample(&mut self, time_ns: u64, signals: &CosimSignals) {
        for sink in &mut self.sinks {
            sink.sample(time_ns, signals);
        }
    }

    /// Run the sinks' own clocks (timers, schedules) to `time_ns`.
    pub(crate) fn service(&mut self, time_ns: u64) {
        for sink in &mut self.sinks {
            if let Model::Gpio { device, pads } = &mut sink.model {
                device.service(pads, time_ns);
            }
        }
    }

    /// What every sink holds, as inspect artifacts named by sink id.
    pub(crate) fn artifacts(&self) -> Vec<crate::inspect::Artifact> {
        use crate::inspect::DeviceEvidence;
        let opts = crate::inspect::InspectOpts::default();
        self.sinks
            .iter()
            .flat_map(|sink| match &sink.model {
                Model::Strip(strip) => strip.artifacts(&sink.id, &opts),
                Model::Gpio { device, .. } => device.artifacts(&sink.id, &opts),
            })
            .collect()
    }
}

impl Sink {
    /// Start the part from the initial levels, as a pad that was always there.
    fn prime(&mut self) {
        match &mut self.model {
            Model::Strip(strip) => {
                if self.pins[0].1 {
                    strip.on_pin_change(0, false, true, 0);
                }
            }
            Model::Gpio { device, pads } => {
                for (index, (_, initial)) in self.pins.iter().enumerate() {
                    pads.observed[index] = *initial;
                }
                device.service(pads, 0);
            }
        }
    }

    fn sample(&mut self, time_ns: u64, signals: &CosimSignals) {
        let mut changed = false;
        for (index, (signal, last)) in self.pins.iter_mut().enumerate() {
            let Some(CosimSignalValue::Bool(value)) = signals.get(signal) else {
                continue;
            };
            if *value == *last {
                continue;
            }
            let from = *last;
            *last = *value;
            changed = true;
            match &mut self.model {
                Model::Strip(strip) => strip.on_pin_change(0, from, *value, time_ns),
                Model::Gpio { pads, .. } => pads.observed[index] = *value,
            }
        }
        if changed {
            if let Model::Gpio { device, pads } = &mut self.model {
                device.service(pads, time_ns);
            }
        }
    }
}
