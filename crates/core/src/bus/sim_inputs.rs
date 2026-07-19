// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SimInput discovery and dispatch on attached devices.

use super::*;

/// A component whose stimulus manifests as an analog level on an ADC channel
/// rather than as bytes on a bus — a potentiometer wiper, a thermistor
/// divider.
///
/// The device-physics math (voltage divider, Steinhart-Hart / beta equation)
/// stays inside the component; this trait is only the seam that lets the bus
/// ask "given your current state, what level is on the pin?" after a
/// [`crate::sim_input::SimInput`] set. That keeps ONE stimulus path — the
/// component owns its units and its physics, the bus owns the wiring.
pub trait AnalogSource: crate::sim_input::SimInput + Send {
    /// The channel level in millivolts for the source's current state.
    fn output_mv(&self) -> u16;
}

/// One analog source wired to a specific ADC channel.
pub struct AnalogInputSource {
    /// system.yaml `connection:` — the ADC controller that owns the channel.
    pub connection: String,
    /// Channel index within that controller.
    pub channel: u8,
    pub source: Box<dyn AnalogSource>,
}

impl SystemBus {
    /// Write `millivolts` into `channel` of the ADC named `connection`.
    ///
    /// Two unrelated ADC controller models exist ([`crate::peripherals::adc::Adc`]
    /// for STM32-class parts, `Esp32s3Sens` for the ESP32-S3 SAR ADC), so this
    /// is the single choke point that hides which one a given system.yaml
    /// wired up — the analog kits are controller-agnostic, exactly as the I²C
    /// kits are via `attach_i2c_slave_with_route`.
    ///
    /// Falls back to a bus scan when `connection` names no ADC: S3 manifests
    /// declare `connection: "sar_adc_s3"` while the peripheral is registered as
    /// `sens_s3`, and that mismatch predates this path.
    pub(crate) fn seed_adc_channel(
        &mut self,
        connection: &str,
        channel: u8,
        millivolts: u16,
    ) -> bool {
        if let Some(idx) = self.find_peripheral_index_by_name(connection) {
            if Self::seed_adc_at(self, idx, channel, millivolts) {
                return true;
            }
        }
        for idx in 0..self.peripherals.len() {
            if Self::seed_adc_at(self, idx, channel, millivolts) {
                return true;
            }
        }
        false
    }

    fn seed_adc_at(&mut self, idx: usize, channel: u8, millivolts: u16) -> bool {
        let Some(any) = self.peripherals[idx].dev.as_any_mut() else {
            return false;
        };
        if let Some(adc) = any.downcast_mut::<crate::peripherals::adc::Adc>() {
            adc.set_channel_input(channel, millivolts);
            return true;
        }
        if let Some(sens) = any.downcast_mut::<crate::peripherals::esp32s3::sens::Esp32s3Sens>() {
            sens.set_channel_input(channel, millivolts);
            return true;
        }
        false
    }

    /// Push every analog source's current level onto its ADC channel.
    ///
    /// Called right after a stimulus set rather than per tick: these levels
    /// only change when someone drives them, so a per-cycle pass would be pure
    /// overhead on the hot loop.
    pub(crate) fn sync_analog_inputs(&mut self) {
        if self.analog_inputs.is_empty() {
            return;
        }
        // Snapshot first: seeding borrows `self.peripherals` mutably.
        let pending: Vec<(String, u8, u16)> = self
            .analog_inputs
            .iter()
            .map(|a| (a.connection.clone(), a.channel, a.source.output_mv()))
            .collect();
        for (connection, channel, mv) in pending {
            self.seed_adc_channel(&connection, channel, mv);
        }
    }
}

impl SystemBus {
    /// Walk every attached device that accepts simulated input, in bus order,
    /// calling `f(owner_name, device)` for each. `owner_name` is the owning
    /// peripheral's bus name for transport-attached devices (I²C / SPI / UART
    /// stream) and the sensor `id` for devices that live directly on the bus
    /// (HC-SR04). Stops early when `f` returns `true`.
    ///
    /// This is the ONE walk behind `list_inputs` / `set_input`, so a device
    /// reachable here is reachable from discovery, dispatch, the manifest
    /// schema consumers, and every external surface (test-script stimuli, MCP,
    /// WASM) at once.
    ///
    /// What that guarantees, precisely: this walk reaches every attached device
    /// whose controller implements
    /// [`Peripheral::for_each_attached_sim_input`](crate::Peripheral::for_each_attached_sim_input),
    /// plus the bus-resident HC-SR04 sensors and analog sources below. It does
    /// NOT reach devices on a controller that has not implemented that seam —
    /// nothing here can, because the walk deliberately does no downcasting and
    /// so holds no list of controller types.
    ///
    /// It used to hold exactly such a list: a downcast chain over `I2c` / `Spi`
    /// / `Uart`, which meant the ESP32-C3 and ESP32-S3 controllers hosted
    /// devices that were invisible to every stimulus API despite passing their
    /// own unit tests. The chain is gone so that adding a controller can no
    /// longer silently subtract reachability; `crates/core/tests/
    /// sim_input_reachability.rs` is the gate that holds the line per chip.
    pub(crate) fn for_each_sim_input(
        &mut self,
        f: &mut dyn FnMut(&str, &mut dyn crate::sim_input::SimInput) -> bool,
    ) {
        for entry in self.peripherals.iter_mut() {
            // `entry.name` is borrowed for the closure while `entry.dev` is
            // borrowed mutably by the seam, so split the borrow explicitly.
            let name = &entry.name;
            let stop = entry.dev.for_each_attached_sim_input(&mut |si| f(name, si));
            if stop {
                return;
            }
        }
        for sensor in self.hcsr04.iter_mut() {
            let id = sensor.id.clone();
            if f(&id, sensor) {
                return;
            }
        }
        for analog in self.analog_inputs.iter_mut() {
            let connection = analog.connection.clone();
            if f(&connection, analog.source.as_mut()) {
                return;
            }
        }
    }

    /// Whether one walked device answers to `component`: its stamped
    /// system.yaml id first (the name an author writes), falling back to the
    /// owning peripheral's bus name.
    pub(crate) fn component_matches(
        component: Option<&str>,
        owner: &str,
        si: &dyn crate::sim_input::SimInput,
    ) -> bool {
        component.is_none_or(|c| si.component_id() == Some(c) || c == owner)
    }

    /// Discover every drivable input channel across attached devices, tagged
    /// with the owning component — the "what can an agent drive?" query
    /// behind `labwired_list_inputs` / the browser panel / the MCP stimulus
    /// surface. The owner is the device's system.yaml `external_devices` id
    /// when stamped (so discovery speaks the SAME vocabulary `set_input`'s
    /// `component` accepts), falling back to the peripheral's bus name.
    pub fn list_inputs(&mut self) -> Vec<(String, crate::sim_input::InputChannel)> {
        let mut out = Vec::new();
        self.for_each_sim_input(&mut |name, si| {
            let owner = si.component_id().unwrap_or(name).to_string();
            for ch in si.input_channels() {
                out.push((owner.clone(), *ch));
            }
            false
        });
        out
    }

    /// Drive `channel` to `value` (in the channel's engineering unit) on the
    /// unique attached input device that exposes it. Generic over device type
    /// via [`crate::sim_input::SimInput`] — no per-type dispatch.
    ///
    /// `component`, when given, narrows resolution to the named device — the
    /// disambiguator when two devices expose the same channel key (e.g. two
    /// accelerometers on one bus, or `distance` on both a VL53L1X and an
    /// HC-SR04). It matches the external-device id from system.yaml
    /// (`fxos8700`, stamped onto the model at attach) or the owning
    /// peripheral's bus name (`i2c1`). Errors if no device (or more than one,
    /// after narrowing) exposes the channel, or the value is out of range.
    pub fn set_input(
        &mut self,
        component: Option<&str>,
        channel: &str,
        value: f64,
    ) -> Result<(), crate::sim_input::SimInputError> {
        use crate::sim_input::SimInputError;
        // Count matches first so ambiguity is a typed error, not a silent
        // "first wins".
        let mut matches = 0usize;
        self.for_each_sim_input(&mut |name, si| {
            if Self::component_matches(component, name, si)
                && si.input_channels().iter().any(|c| c.key == channel)
            {
                matches += 1;
            }
            false
        });
        if matches == 0 {
            let missing = match component {
                Some(c) => format!("{c}/{channel}"),
                None => channel.to_string(),
            };
            return Err(SimInputError::NoDevice(missing));
        }
        if matches > 1 {
            return Err(SimInputError::Ambiguous {
                channel: channel.to_string(),
                matches,
            });
        }
        let mut result = Ok(());
        self.for_each_sim_input(&mut |name, si| {
            if Self::component_matches(component, name, si)
                && si.input_channels().iter().any(|c| c.key == channel)
            {
                result = si.set_input(channel, value);
                true
            } else {
                false
            }
        });
        // An analog source's new state only reaches the firmware once its level
        // is on the ADC channel; do it here, at the single apply point, so no
        // caller can drive one and observe a stale conversion.
        if result.is_ok() {
            self.sync_analog_inputs();
        }
        result
    }

    /// Apply several input sets as ONE transaction: every set is resolved and
    /// range-checked first, and only if ALL are valid are any applied — so a
    /// multi-channel pose (an accelerometer's x/y/z, a GPS lat+lon) can never
    /// be half-applied, and the firmware can never observe a torn update
    /// (nothing steps between the applications). All-or-nothing: the first
    /// error aborts the whole batch with nothing written.
    pub fn set_inputs(
        &mut self,
        sets: &[(Option<&str>, &str, f64)],
    ) -> Result<(), crate::sim_input::SimInputError> {
        use crate::sim_input::SimInputError;
        // Validate pass: unique resolution + range for every set.
        for &(component, channel, value) in sets {
            let mut matches = 0usize;
            let mut range: Result<(), SimInputError> = Ok(());
            self.for_each_sim_input(&mut |name, si| {
                if Self::component_matches(component, name, si)
                    && si.input_channels().iter().any(|c| c.key == channel)
                {
                    matches += 1;
                    range = si.require_channel(channel, value).map(|_| ());
                }
                false
            });
            if matches == 0 {
                let missing = match component {
                    Some(c) => format!("{c}/{channel}"),
                    None => channel.to_string(),
                };
                return Err(SimInputError::NoDevice(missing));
            }
            if matches > 1 {
                return Err(SimInputError::Ambiguous {
                    channel: channel.to_string(),
                    matches,
                });
            }
            range?;
        }
        // Apply pass — validated above, so failures here can't strand a
        // partial batch short of a device error, which set_input surfaces.
        for &(component, channel, value) in sets {
            self.set_input(component, channel, value)?;
        }
        Ok(())
    }

    /// Snapshot of the universal bus-transaction trace (logic analyzer):
    /// every I²C/SPI byte recorded so far by peripherals wired to
    /// `self.bus_trace` (see `crate::bus::bus_trace`), oldest first.
    pub fn bus_trace_snapshot(&self) -> Vec<bus_trace::BusTraceEvent> {
        self.bus_trace.snapshot()
    }
}
