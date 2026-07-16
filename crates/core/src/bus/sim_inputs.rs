// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SimInput discovery and dispatch on attached devices.

use super::*;

impl SystemBus {
    /// Walk every attached device that accepts simulated input, in bus order,
    /// calling `f(owner_name, device)` for each. `owner_name` is the owning
    /// peripheral's bus name for transport-attached devices (I²C / SPI / UART
    /// stream) and the sensor `id` for devices that live directly on the bus
    /// (HC-SR04). Stops early when `f` returns `true`.
    ///
    /// This is the ONE walk behind `list_inputs` / `set_input`, so a new
    /// transport (or directly-attached input device) added here is picked up
    /// by discovery, dispatch, the manifest schema consumers, and every
    /// external surface (test-script stimuli, MCP, WASM) at once.
    pub(crate) fn for_each_sim_input(
        &mut self,
        f: &mut dyn FnMut(&str, &mut dyn crate::sim_input::SimInput) -> bool,
    ) {
        for entry in self.peripherals.iter_mut() {
            let Some(any) = entry.dev.as_any_mut() else {
                continue;
            };
            if let Some(i2c) = any.downcast_ref::<crate::peripherals::i2c::I2c>() {
                for cell in i2c.attached_devices() {
                    let mut dev = cell.borrow_mut();
                    if let Some(si) = dev.as_sim_input_mut() {
                        if f(&entry.name, si) {
                            return;
                        }
                    }
                }
            } else if let Some(spi) = any.downcast_mut::<crate::peripherals::spi::Spi>() {
                for dev in spi.attached_devices.iter_mut() {
                    if let Some(si) = dev.as_sim_input_mut() {
                        if f(&entry.name, si) {
                            return;
                        }
                    }
                }
            } else if let Some(uart) = any.downcast_mut::<crate::peripherals::uart::Uart>() {
                for stream in uart.attached_streams.iter_mut() {
                    if let Some(si) = stream.as_sim_input_mut() {
                        if f(&entry.name, si) {
                            return;
                        }
                    }
                }
            }
        }
        for sensor in self.hcsr04.iter_mut() {
            let id = sensor.id.clone();
            if f(&id, sensor) {
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
