// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! I2C buses of the CPU-free Flex path.
//!
//! Portable firmware is the only controller of a bus. It names a 7-bit
//! address and exchanges whole bytes with the catalog device models that the
//! detailed path attaches to an MCU's I2C controller ([`build_i2c_device`]).
//! No controller peripheral, pad, or SCL/SDA waveform is built here: the
//! record of a bus is its list of transfers.

use crate::cosim::{CosimSignalValue, CosimSignals};
use crate::peripherals::components::build_i2c_device;
use crate::peripherals::device::I2cDevice;
use crate::sim_input::SimInput;
use crate::{SimResult, SimulationError};
use labwired_config::FlexI2cBus;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, VecDeque};

/// Most bytes one transfer writes, and most bytes it reads.
pub const FLEX_I2C_MAX_TRANSFER_BYTES: usize = 4096;
/// Transfers the record keeps; the per-address counts stay exact beyond it.
const TRANSFER_RING_CAPACITY: usize = 4096;

/// One `i2c_transfer`: an optional write phase, an optional read phase after
/// a repeated START, and a STOP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlexI2cTransfer {
    pub bus: u32,
    /// Virtual time of the START.
    pub time_ns: u64,
    /// One SCL period for each bit, START and STOP at the bus frequency.
    pub duration_ns: u64,
    pub address: u8,
    /// A device answered the address. `false`: no device, nothing exchanged.
    pub ack: bool,
    pub write: Vec<u8>,
    pub read: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlexI2cBusChannel {
    pub id: u32,
    pub name: String,
}

/// The retained transfers of every bus in time order, for a protocol view.
#[derive(Debug, Serialize)]
pub struct FlexI2cTraceBatch {
    pub time_unit: &'static str,
    pub buses: Vec<FlexI2cBusChannel>,
    pub transfers: Vec<FlexI2cTransfer>,
    /// Monotonic transfer cursor to pass to the next snapshot.
    pub next_cursor: u64,
    /// Cumulative transfers lost to ring-buffer overflow since the start.
    pub dropped: u64,
}

/// A device input channel and the signal that drives it.
#[derive(Debug, Clone, Serialize)]
pub struct FlexBusInputSummary {
    pub name: String,
    pub signal: String,
    pub unit: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlexI2cDeviceSummary {
    pub id: String,
    pub model_type: String,
    pub address: u8,
    pub config: BTreeMap<String, serde_yaml::Value>,
    pub inputs: Vec<FlexBusInputSummary>,
}

/// What a run report states about one bus, including what the path leaves out.
#[derive(Debug, Clone, Serialize)]
pub struct FlexI2cBusSummary {
    pub name: String,
    pub frequency_hz: u32,
    pub devices: Vec<FlexI2cDeviceSummary>,
    pub not_modeled: &'static [&'static str],
}

const NOT_MODELED: &[&str] = &[
    "scl_sda_waveform",
    "pull_ups_and_bus_capacitance",
    "clock_stretching",
    "device_pin_outputs",
    "other_controllers",
];

pub(crate) struct InputBinding {
    channel: String,
    signal: String,
    /// The value the device model holds for this channel.
    applied: Option<f64>,
}

/// Bind each `channel -> signal` of a bus device to the model's input
/// channels. `units` maps a signal to the unit its producer declares; a
/// channel takes only a signal in its own unit.
pub(crate) fn bind_inputs(
    path: &str,
    id: &str,
    mut input: Option<&mut dyn SimInput>,
    declared: &BTreeMap<String, String>,
    units: &BTreeMap<String, String>,
) -> SimResult<(Vec<InputBinding>, Vec<FlexBusInputSummary>)> {
    if let Some(input) = input.as_deref_mut() {
        input.set_component_id(id.to_owned());
    }
    let channels = input
        .map(|input| input.input_channels().to_vec())
        .unwrap_or_default();
    let mut bindings = Vec::with_capacity(declared.len());
    let mut summaries = Vec::with_capacity(declared.len());
    for (channel, signal) in declared {
        let Some(found) = channels.iter().find(|c| c.key == channel.as_str()) else {
            return Err(error(format!(
                "{path} has no input channel '{channel}'; its channels are [{}]",
                channels
                    .iter()
                    .map(|c| c.key.as_ref())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        let produced = units.get(signal).map(String::as_str);
        if produced != Some(found.unit.as_ref()) {
            return Err(error(format!(
                "{path} input '{channel}' takes unit {:?}, but '{signal}' produces {produced:?}; no implicit unit conversion",
                found.unit
            )));
        }
        bindings.push(InputBinding {
            channel: channel.clone(),
            signal: signal.clone(),
            applied: None,
        });
        summaries.push(FlexBusInputSummary {
            name: channel.clone(),
            signal: signal.clone(),
            unit: found.unit.to_string(),
        });
    }
    Ok((bindings, summaries))
}

/// Hand every bound signal value that changed to a device model.
pub(crate) fn apply_inputs(
    path: &str,
    mut input: Option<&mut dyn SimInput>,
    bindings: &mut [InputBinding],
    signals: &CosimSignals,
) -> SimResult<()> {
    for binding in bindings {
        let value = match signals.get(&binding.signal) {
            Some(CosimSignalValue::F64(value)) => *value,
            Some(CosimSignalValue::I64(value)) => *value as f64,
            _ => {
                return Err(error(format!(
                    "{path} input '{}': signal '{}' has no numeric value",
                    binding.channel, binding.signal
                )))
            }
        };
        if binding.applied == Some(value) {
            continue;
        }
        let Some(input) = input.as_deref_mut() else {
            continue; // Bindings exist only for models with inputs.
        };
        input
            .set_input(&binding.channel, value)
            .map_err(|cause| error(format!("{path} input '{}': {cause}", binding.channel)))?;
        binding.applied = Some(value);
    }
    Ok(())
}

struct Device {
    id: String,
    model: Box<dyn I2cDevice>,
    inputs: Vec<InputBinding>,
}

struct Bus {
    frequency_hz: u32,
    devices: Vec<Device>,
    /// Exact transfer counts by (address, acknowledged).
    counts: BTreeMap<(u8, bool), u64>,
}

pub(crate) struct I2cBuses {
    buses: Vec<Bus>,
    summaries: Vec<FlexI2cBusSummary>,
    ring: VecDeque<FlexI2cTransfer>,
    recorded: u64,
    /// Nanoseconds not yet handed to the device clocks, which count in µs.
    clock_remainder_ns: u64,
    /// Nanoseconds the device clocks already ran inside transfers, ahead of
    /// the `advance_ns` that the caller makes for each transfer.
    clock_ahead_ns: u64,
}

fn device_model(buses: &mut [Bus], bus_id: u32, index: usize) -> &mut dyn I2cDevice {
    buses[bus_id as usize].devices[index].model.as_mut()
}

fn error(message: impl Into<String>) -> SimulationError {
    SimulationError::Other(message.into())
}

impl I2cBuses {
    /// Build every declared device from the catalog. `units` maps a signal
    /// to the unit its producer declares; a device input takes only a signal
    /// in the channel's own unit.
    pub(crate) fn new(
        configs: &[FlexI2cBus],
        units: &BTreeMap<String, String>,
        parts: &[labwired_config::PartPack],
    ) -> SimResult<Self> {
        let mut buses = Vec::with_capacity(configs.len());
        let mut summaries = Vec::with_capacity(configs.len());
        for config in configs {
            let mut devices = Vec::<Device>::with_capacity(config.devices.len());
            let mut device_summaries = Vec::with_capacity(config.devices.len());
            for device in &config.devices {
                let path = format!("I2C device '{}'", device.id);
                let settings: HashMap<String, serde_yaml::Value> = device
                    .config
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                // A manifest part pack claims its type before the catalog.
                let built = match crate::flex_parts::pack(parts, &device.device_type) {
                    Some(pack) => crate::peripherals::components::i2c_factory::build_i2c_device_from_descriptor(
                        pack,
                        Some(&device.id),
                        &settings,
                    )
                    .ok_or_else(|| {
                        error(format!(
                            "{path}: part pack '{}' is a {} part, not an I2C device",
                            device.device_type, pack.behavior.primitive
                        ))
                    })?,
                    None => build_i2c_device(&device.device_type, &settings).ok_or_else(|| {
                        error(format!(
                            "{path}: the catalog has no I2C device model of type '{}'",
                            device.device_type
                        ))
                    })?,
                };
                let mut model = built;
                let address = model.address();
                if address > 0x7f {
                    return Err(error(format!(
                        "{path}: model address {address:#04x} is not a 7-bit address"
                    )));
                }
                if let Some(other) = devices
                    .iter()
                    .find(|other| other.model.address() == address)
                {
                    return Err(error(format!(
                        "{path} and '{}' both answer address {address:#04x} on bus '{}'",
                        other.id, config.name
                    )));
                }
                let (inputs, input_summaries) = bind_inputs(
                    &path,
                    &device.id,
                    model.as_sim_input_mut(),
                    &device.inputs,
                    units,
                )?;
                device_summaries.push(FlexI2cDeviceSummary {
                    id: device.id.clone(),
                    model_type: device.device_type.clone(),
                    address,
                    config: device.config.clone(),
                    inputs: input_summaries,
                });
                devices.push(Device {
                    id: device.id.clone(),
                    model,
                    inputs,
                });
            }
            summaries.push(FlexI2cBusSummary {
                name: config.name.clone(),
                frequency_hz: config.frequency_hz,
                devices: device_summaries,
                not_modeled: NOT_MODELED,
            });
            buses.push(Bus {
                frequency_hz: config.frequency_hz,
                devices,
                counts: BTreeMap::new(),
            });
        }
        Ok(Self {
            buses,
            summaries,
            ring: VecDeque::new(),
            recorded: 0,
            clock_remainder_ns: 0,
            clock_ahead_ns: 0,
        })
    }

    pub(crate) fn summaries(&self) -> &[FlexI2cBusSummary] {
        &self.summaries
    }

    /// What every device holds, as inspect artifacts named by device id.
    pub(crate) fn artifacts(&self) -> Vec<crate::inspect::Artifact> {
        let opts = crate::inspect::InspectOpts::default();
        self.buses
            .iter()
            .flat_map(|bus| &bus.devices)
            .flat_map(|device| device.model.artifacts(&device.id, &opts))
            .collect()
    }

    pub(crate) fn bus_id(&self, name: &str) -> SimResult<u32> {
        self.summaries
            .iter()
            .position(|bus| bus.name == name)
            .map(|index| index as u32)
            .ok_or_else(|| error(format!("unknown Flex I2C bus '{name}'")))
    }

    /// Hand every bound signal value that changed to its device model.
    pub(crate) fn sync_inputs(&mut self, signals: &CosimSignals) -> SimResult<()> {
        for device in self.buses.iter_mut().flat_map(|bus| &mut bus.devices) {
            let path = format!("I2C device '{}'", device.id);
            apply_inputs(
                &path,
                device.model.as_sim_input_mut(),
                &mut device.inputs,
                signals,
            )?;
        }
        Ok(())
    }

    /// Advance the free-running clock of every device model. Time that the
    /// clocks already ran inside a transfer is not counted twice.
    pub(crate) fn advance_ns(&mut self, delta_ns: u64) {
        let ahead = self.clock_ahead_ns.min(delta_ns);
        self.clock_ahead_ns -= ahead;
        self.run_clocks(delta_ns - ahead);
    }

    /// Advance the device clocks inside a transfer, from `*at_ns` to
    /// `to_ns` after its START.
    fn run_clocks_in_transfer(&mut self, at_ns: &mut u64, to_ns: u64) {
        self.run_clocks(to_ns - *at_ns);
        self.clock_ahead_ns += to_ns - *at_ns;
        *at_ns = to_ns;
    }

    fn run_clocks(&mut self, delta_ns: u64) {
        let total = self.clock_remainder_ns + delta_ns;
        self.clock_remainder_ns = total % 1_000;
        let us = total / 1_000;
        if us == 0 {
            return;
        }
        for device in self.buses.iter_mut().flat_map(|bus| &mut bus.devices) {
            device.model.advance_time_us(us);
        }
    }

    /// Run one transfer at `time_ns`. The caller advances virtual time by the
    /// returned `duration_ns`.
    pub(crate) fn transfer(
        &mut self,
        bus_id: u32,
        time_ns: u64,
        address: u32,
        write: &[u8],
        read_len: usize,
    ) -> SimResult<FlexI2cTransfer> {
        let bus = self
            .buses
            .get(bus_id as usize)
            .ok_or_else(|| error(format!("unknown Flex I2C bus id {bus_id}")))?;
        if address > 0x7f {
            return Err(error(format!(
                "I2C address {address:#x} is not a 7-bit address"
            )));
        }
        let address = address as u8;
        if write.len() > FLEX_I2C_MAX_TRANSFER_BYTES || read_len > FLEX_I2C_MAX_TRANSFER_BYTES {
            return Err(error(format!(
                "an I2C transfer writes and reads at most {FLEX_I2C_MAX_TRANSFER_BYTES} bytes each"
            )));
        }
        let frequency_hz = u64::from(bus.frequency_hz);
        let bit_ns = |bits: u64| bits * 1_000_000_000 / frequency_hz;
        // A transfer with no read has a write phase, even with no data byte:
        // that is the address probe of a bus scan.
        let write_phase = !write.is_empty() || read_len == 0;
        let device = bus
            .devices
            .iter()
            .position(|device| device.model.claims_address(address));
        let ack = device.is_some();
        let mut read = Vec::new();
        // START, address byte and its ACK bit; STOP at the end.
        let mut bits = 10 + 1;
        if let Some(index) = device {
            // The device sees each event at its bit time on the bus: a
            // written byte after its ACK bit, a read byte at its first bit,
            // the STOP at the end. Its clock runs between the events, so a
            // command's delay counts from the command byte, not the START.
            let mut at_ns = 0;
            let mut position = 10;
            device_model(&mut self.buses, bus_id, index).select_address(address);
            if write_phase {
                device_model(&mut self.buses, bus_id, index).start();
                for byte in write {
                    position += 9;
                    self.run_clocks_in_transfer(&mut at_ns, bit_ns(position));
                    device_model(&mut self.buses, bus_id, index).write(*byte);
                }
                bits += 9 * write.len() as u64;
            }
            if read_len > 0 {
                // A repeated START and address byte follow a write phase.
                if write_phase {
                    position += 10;
                }
                device_model(&mut self.buses, bus_id, index).start();
                for _ in 0..read_len {
                    self.run_clocks_in_transfer(&mut at_ns, bit_ns(position));
                    read.push(device_model(&mut self.buses, bus_id, index).read());
                    position += 9;
                }
                bits += 9 * read_len as u64 + if write_phase { 10 } else { 0 };
            }
            self.run_clocks_in_transfer(&mut at_ns, bit_ns(bits));
            let model = device_model(&mut self.buses, bus_id, index);
            model.stop();
            // Device pin outputs (INT, DRDY) have no pad to reach here.
            model.take_pin_drives();
        }
        let bus = &mut self.buses[bus_id as usize];
        let transfer = FlexI2cTransfer {
            bus: bus_id,
            time_ns,
            duration_ns: bit_ns(bits),
            address,
            ack,
            write: if ack { write.to_vec() } else { Vec::new() },
            read,
        };
        *bus.counts.entry((address, ack)).or_default() += 1;
        if self.ring.len() == TRANSFER_RING_CAPACITY {
            self.ring.pop_front();
        }
        self.ring.push_back(transfer.clone());
        self.recorded += 1;
        Ok(transfer)
    }

    /// Exact number of transfers on a bus since the start, optionally of one
    /// address and of one outcome.
    pub(crate) fn transfer_count(
        &self,
        bus_id: u32,
        address: Option<u8>,
        ack: Option<bool>,
    ) -> Option<u64> {
        let bus = self.buses.get(bus_id as usize)?;
        Some(
            bus.counts
                .iter()
                .filter(|((counted, acked), _)| {
                    address.is_none_or(|address| address == *counted)
                        && ack.is_none_or(|ack| ack == *acked)
                })
                .map(|(_, count)| count)
                .sum(),
        )
    }

    pub(crate) fn snapshot(&self, cursor: u64) -> FlexI2cTraceBatch {
        let dropped = self.recorded - self.ring.len() as u64;
        let skip = cursor.saturating_sub(dropped).min(self.ring.len() as u64) as usize;
        FlexI2cTraceBatch {
            time_unit: "ns",
            buses: self
                .summaries
                .iter()
                .enumerate()
                .map(|(index, bus)| FlexI2cBusChannel {
                    id: index as u32,
                    name: bus.name.clone(),
                })
                .collect(),
            transfers: self.ring.iter().skip(skip).cloned().collect(),
            next_cursor: self.recorded,
            dropped,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::flex::FlexRuntime;

    const STATION: &str =
        include_str!("../../wasm/tests/fixtures/flex-heypcb-esp32-bme280/system.yaml");

    fn error(yaml: &str) -> String {
        match FlexRuntime::from_yaml(yaml) {
            Ok(_) => panic!("manifest must be rejected"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn firmware_side_transfers_reach_the_catalog_device_model() {
        let mut runtime = FlexRuntime::from_yaml(STATION).unwrap();
        let bus = runtime.i2c_bus_id("sensor_i2c").unwrap();
        assert!(runtime.i2c_bus_id("missing").is_err());

        // Pointer write, repeated START, one byte: the BME280 chip id.
        let id = runtime.i2c_transfer(bus, 0x76, &[0xD0], 1).unwrap();
        assert!(id.ack);
        assert_eq!(id.read, [0x60]);
        assert_eq!(id.write, [0xD0]);
        // 39 bit times at 100 kHz. The runtime leaves time to its caller.
        assert_eq!((id.time_ns, id.duration_ns), (0, 390_000));
        assert_eq!(runtime.time_ns(), 0);

        // The board straps 0x76; nothing answers 0x77.
        let absent = runtime.i2c_transfer(bus, 0x77, &[0xD0], 1).unwrap();
        assert!(!absent.ack);
        assert!(absent.read.is_empty() && absent.write.is_empty());
        assert_eq!(absent.duration_ns, 110_000);

        assert_eq!(runtime.i2c_transfer_count(bus, None, None), Some(2));
        assert_eq!(
            runtime.i2c_transfer_count(bus, Some(0x76), Some(true)),
            Some(1)
        );
        assert_eq!(
            runtime.i2c_transfer_count(bus, Some(0x76), Some(false)),
            Some(0)
        );
        assert_eq!(runtime.i2c_transfer_count(bus, None, Some(false)), Some(1));
        assert_eq!(runtime.i2c_transfer_count(bus + 1, None, None), None);

        let trace = runtime.i2c_trace_snapshot(0);
        assert_eq!((trace.next_cursor, trace.dropped), (2, 0));
        assert_eq!(trace.buses[0].name, "sensor_i2c");
        assert_eq!(trace.transfers, [id, absent.clone()]);
        assert_eq!(runtime.i2c_trace_snapshot(1).transfers, [absent]);
        assert!(runtime.i2c_trace_snapshot(2).transfers.is_empty());

        for (address, write, read, message) in [
            (0x80, 0, 0, "not a 7-bit address"),
            (0x76, 4097, 0, "at most 4096 bytes"),
            (0x76, 0, 4097, "at most 4096 bytes"),
        ] {
            let error = runtime
                .i2c_transfer(bus, address, &vec![0; write], read)
                .unwrap_err()
                .to_string();
            assert!(error.contains(message), "{error}");
        }
        assert!(runtime.i2c_transfer(bus + 1, 0x76, &[], 0).is_err());
        // A rejected call is not a transfer.
        assert_eq!(runtime.i2c_transfer_count(bus, None, None), Some(2));
    }

    #[test]
    fn a_device_input_follows_its_port_and_rejects_a_value_outside_its_range() {
        let mut runtime = FlexRuntime::from_yaml(STATION).unwrap();
        let bus = runtime.i2c_bus_id("sensor_i2c").unwrap();
        let ambient = runtime.port_id("ambient_c").unwrap();
        // A BME280 measures nothing until ctrl_meas selects an oversampling.
        let idle = runtime.i2c_transfer(bus, 0x76, &[0xFA], 3).unwrap().read;
        assert_eq!(idle, [0x80, 0x00, 0x00]);
        assert!(
            runtime
                .i2c_transfer(bus, 0x76, &[0xF4, 0x27], 0)
                .unwrap()
                .ack
        );
        let raw =
            |runtime: &mut FlexRuntime| runtime.i2c_transfer(bus, 0x76, &[0xFA], 3).unwrap().read;
        let at_25 = raw(&mut runtime);
        runtime.write(ambient, 40.0).unwrap();
        let at_40 = raw(&mut runtime);
        assert_ne!(at_25, at_40);
        runtime.write(ambient, 25.0).unwrap();
        assert_eq!(raw(&mut runtime), at_25);

        // The BME280 model takes -40..85 °C. The refusal stops the runtime.
        let error = runtime.write(ambient, 200.0).unwrap_err().to_string();
        assert!(
            error.contains("source-u4-bme280") && error.contains("out of range"),
            "{error}"
        );
        assert!(runtime.i2c_transfer(bus, 0x76, &[0xFA], 3).is_err());
    }

    /// A device sees each byte at its bit time, not at the START. The SHT40
    /// model answers 8.3 ms after its measurement command; at 100 kHz that
    /// byte is complete 190 µs after the START, and the first byte of a
    /// later read is sampled 100 µs after that read's START.
    #[test]
    fn a_command_delay_counts_from_the_command_byte_not_the_start() {
        const MONITOR: &str =
            include_str!("../../wasm/tests/fixtures/flex-heypcb-esp32-sht40/system.yaml");
        // Wait after the command transfer, then read; true: data is ready.
        let ready_after = |wait_ns: u64| {
            let mut runtime = FlexRuntime::from_yaml(MONITOR).unwrap();
            let bus = runtime.i2c_bus_id("sensor_i2c").unwrap();
            let command = runtime.i2c_transfer(bus, 0x44, &[0xFD], 0).unwrap();
            assert_eq!(command.duration_ns, 200_000);
            runtime.advance_ns(command.duration_ns + wait_ns).unwrap();
            let read = runtime.i2c_transfer(bus, 0x44, &[], 6).unwrap();
            assert_eq!(read.time_ns, 200_000 + wait_ns);
            read.read != [0xFF; 6]
        };
        // 190 µs + 8300 µs = 200 µs + wait + 100 µs.
        assert!(!ready_after(8_189_000));
        assert!(ready_after(8_190_000));
    }

    #[test]
    fn the_run_report_states_the_bus_and_what_it_does_not_model() {
        let runtime = FlexRuntime::from_yaml(STATION).unwrap();
        let report = serde_json::to_value(runtime.run_report()).unwrap();
        let bus = &report["i2c_buses"][0];
        assert_eq!(bus["name"], "sensor_i2c");
        assert_eq!(bus["frequency_hz"], 100_000);
        assert_eq!(bus["devices"][0]["id"], "source-u4-bme280");
        assert_eq!(bus["devices"][0]["model_type"], "bme280");
        assert_eq!(bus["devices"][0]["address"], 0x76);
        assert_eq!(bus["devices"][0]["inputs"][2]["name"], "temperature");
        assert_eq!(bus["devices"][0]["inputs"][2]["unit"], "°C");
        assert!(bus["not_modeled"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap == "scl_sda_waveform"));
        assert_eq!(report["execution_scope"]["chip_parity"], "not_established");
    }

    #[test]
    fn a_bus_with_the_sensor_removed_refuses_its_address() {
        let removed =
            STATION.split("    devices:").next().unwrap().to_owned() + "    devices: []\n";
        // Without U4 nothing consumes the three ambient ports.
        let error = error(&removed);
        assert!(
            error.contains("is not consumed by any model or bus device"),
            "{error}"
        );
        let removed = removed
            .lines()
            .filter(|line| !line.contains("signal: environment."))
            .collect::<Vec<_>>()
            .join("\n");
        let mut runtime = FlexRuntime::from_yaml(&removed).unwrap();
        let bus = runtime.i2c_bus_id("sensor_i2c").unwrap();
        let probe = runtime.i2c_transfer(bus, 0x76, &[0xD0], 1).unwrap();
        assert!(!probe.ack);
        assert_eq!(
            runtime.i2c_transfer_count(bus, Some(0x76), Some(false)),
            Some(1)
        );
    }

    #[test]
    fn a_bus_the_catalog_cannot_build_is_a_load_error() {
        for (from, to, message) in [
            ("type: bme280", "type: no_such_part", "no I2C device model of type 'no_such_part'"),
            (
                "temperature: environment.temperature",
                "altitude: environment.temperature",
                "has no input channel 'altitude'; its channels are [temperature, humidity, pressure]",
            ),
            (
                "unit: \"°C\"",
                "unit: K",
                "takes unit \"°C\", but 'environment.temperature' produces Some(\"K\"); no implicit unit conversion",
            ),
            (
                "temperature: environment.temperature",
                "temperature: environment.missing",
                "input signal 'environment.missing' has no declared producer",
            ),
            ("frequency_hz: 100000", "frequency_hz: 0", "frequency_hz must be within 1..=3400000"),
            ("initial: 25 }", "initial: 125 }", "out of range"),
        ] {
            assert!(STATION.contains(from), "{from}");
            let error = error(&STATION.replace(from, to));
            assert!(error.contains(message), "{from} -> {to}: {error}");
        }
        // The second device goes at the end of the bus, before `console`.
        let twice = STATION.replace(
            "console:\n",
            "      - id: second\n        type: bme280\n        config: { i2c_address: 0x76 }\nconsole:\n",
        );
        let error = error(&twice);
        assert!(
            error.contains("both answer address 0x76 on bus 'sensor_i2c'"),
            "{error}"
        );
    }

    #[test]
    fn a_manifest_without_a_bus_reports_none() {
        let no_bus = STATION.split("i2c_buses:").next().unwrap();
        let no_bus = no_bus
            .lines()
            .filter(|line| !line.contains("signal: environment."))
            .collect::<Vec<_>>()
            .join("\n");
        let runtime = FlexRuntime::from_yaml(&no_bus).unwrap();
        let report = serde_json::to_value(runtime.run_report()).unwrap();
        assert!(report.get("i2c_buses").is_none());
        assert!(runtime.i2c_trace_snapshot(0).buses.is_empty());
    }
}
