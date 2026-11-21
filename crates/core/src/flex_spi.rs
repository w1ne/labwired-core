// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SPI buses of the CPU-free Flex path.
//!
//! Portable firmware is the only controller of a bus. It names a chip select
//! and exchanges whole bytes with the declarative catalog SPI models that the
//! detailed path attaches to an MCU's SPI controller ([`GenericSpiDevice`]).
//! No controller peripheral, pad, or SCK/MOSI/MISO waveform is built here:
//! the record of a bus is its list of transfers.

use crate::cosim::CosimSignals;
use crate::flex_i2c::{apply_inputs, bind_inputs, FlexBusInputSummary, InputBinding};
use crate::peripherals::components::declarative_i2c::owned_channels;
use crate::peripherals::components::declarative_spi::GenericSpiDevice;
use crate::peripherals::device::SpiDevice;
use crate::{SimResult, SimulationError};
use labwired_config::{DeviceDescriptor, FlexSpiBus};
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};

/// Most bytes one transfer exchanges.
pub const FLEX_SPI_MAX_TRANSFER_BYTES: usize = 4096;
/// Transfers the record keeps; the per-select counts stay exact beyond it.
const TRANSFER_RING_CAPACITY: usize = 4096;

/// One `spi_transfer`: CS low, `mosi.len()` full-duplex bytes, CS high.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlexSpiTransfer {
    pub bus: u32,
    /// Virtual time of the CS falling edge.
    pub time_ns: u64,
    /// Eight SCK periods for each byte at the bus frequency.
    pub duration_ns: u64,
    pub cs: u32,
    /// A device is declared on this chip select. `false`: nothing answered,
    /// and the firmware's receive buffer is not written.
    pub selected: bool,
    pub mosi: Vec<u8>,
    pub miso: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlexSpiBusChannel {
    pub id: u32,
    pub name: String,
}

/// The retained transfers of every bus in time order, for a protocol view.
#[derive(Debug, Serialize)]
pub struct FlexSpiTraceBatch {
    pub time_unit: &'static str,
    pub buses: Vec<FlexSpiBusChannel>,
    pub transfers: Vec<FlexSpiTransfer>,
    /// Monotonic transfer cursor to pass to the next snapshot.
    pub next_cursor: u64,
    /// Cumulative transfers lost to ring-buffer overflow since the start.
    pub dropped: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlexSpiDeviceSummary {
    pub id: String,
    pub model_type: String,
    pub cs: u32,
    pub config: BTreeMap<String, serde_yaml::Value>,
    pub inputs: Vec<FlexBusInputSummary>,
}

/// What a run report states about one bus, including what the path leaves out.
#[derive(Debug, Clone, Serialize)]
pub struct FlexSpiBusSummary {
    pub name: String,
    pub frequency_hz: u32,
    pub devices: Vec<FlexSpiDeviceSummary>,
    pub not_modeled: &'static [&'static str],
}

const NOT_MODELED: &[&str] = &[
    "sck_mosi_miso_waveform",
    "spi_mode_cpol_cpha",
    "cs_setup_and_hold_times",
    "miso_of_an_unselected_bus",
    "device_pin_outputs",
    "other_controllers",
];

struct Device {
    id: String,
    cs: u32,
    model: Box<dyn SpiDevice>,
    inputs: Vec<InputBinding>,
}

struct Bus {
    frequency_hz: u32,
    devices: Vec<Device>,
    /// Exact transfer counts by (cs, selected).
    counts: BTreeMap<(u32, bool), u64>,
}

pub(crate) struct SpiBuses {
    buses: Vec<Bus>,
    summaries: Vec<FlexSpiBusSummary>,
    ring: VecDeque<FlexSpiTransfer>,
    recorded: u64,
    /// Nanoseconds not yet handed to the device clocks, which count in µs.
    clock_remainder_ns: u64,
    /// Nanoseconds the device clocks already ran inside transfers, ahead of
    /// the `advance_ns` that the caller makes for each transfer.
    clock_ahead_ns: u64,
}

fn error(message: impl Into<String>) -> SimulationError {
    SimulationError::Other(message.into())
}

/// The declarative catalog model of `device_type`. Rust-backed SPI models
/// need an MCU controller and are not reachable here.
fn build_model(
    path: &str,
    device_type: &str,
    cs: u32,
    config: &BTreeMap<String, serde_yaml::Value>,
    parts: &[labwired_config::PartPack],
) -> SimResult<Box<dyn SpiDevice>> {
    // A manifest part pack claims its type before the catalog.
    let descriptor = crate::flex_parts::descriptor(parts, device_type)
        .map_err(|cause| error(format!("{path}: type '{device_type}': {cause}")))?
        .filter(|descriptor: &DeviceDescriptor| descriptor.behavior.primitive == "spi_device")
        .ok_or_else(|| {
            error(format!(
                "{path}: the catalog has no declarative SPI device model of type '{device_type}'"
            ))
        })?;
    let channels = owned_channels(&descriptor);
    let mut model = GenericSpiDevice::from_descriptor(&descriptor, format!("cs{cs}"), channels)
        .map_err(|cause| error(format!("{path}: {cause:#}")))?;
    // The same `config:` seeding the kit attach does.
    model.seed_from_config(|key| config.get(key).and_then(serde_yaml::Value::as_f64));
    Ok(Box::new(model))
}

impl SpiBuses {
    /// Build every declared device from the catalog. `units` maps a signal
    /// to the unit its producer declares; a device input takes only a signal
    /// in the channel's own unit.
    pub(crate) fn new(
        configs: &[FlexSpiBus],
        units: &BTreeMap<String, String>,
        parts: &[labwired_config::PartPack],
    ) -> SimResult<Self> {
        let mut buses = Vec::with_capacity(configs.len());
        let mut summaries = Vec::with_capacity(configs.len());
        for config in configs {
            let mut devices = Vec::with_capacity(config.devices.len());
            let mut device_summaries = Vec::with_capacity(config.devices.len());
            for device in &config.devices {
                let path = format!("SPI device '{}'", device.id);
                let mut model =
                    build_model(&path, &device.device_type, device.cs, &device.config, parts)?;
                let (inputs, input_summaries) = bind_inputs(
                    &path,
                    &device.id,
                    model.as_sim_input_mut(),
                    &device.inputs,
                    units,
                )?;
                device_summaries.push(FlexSpiDeviceSummary {
                    id: device.id.clone(),
                    model_type: device.device_type.clone(),
                    cs: device.cs,
                    config: device.config.clone(),
                    inputs: input_summaries,
                });
                devices.push(Device {
                    id: device.id.clone(),
                    cs: device.cs,
                    model,
                    inputs,
                });
            }
            summaries.push(FlexSpiBusSummary {
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

    pub(crate) fn summaries(&self) -> &[FlexSpiBusSummary] {
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

    pub(crate) fn bus_id(&self, name: &str) -> Option<u32> {
        self.summaries
            .iter()
            .position(|bus| bus.name == name)
            .map(|index| index as u32)
    }

    /// Hand every bound signal value that changed to its device model.
    pub(crate) fn sync_inputs(&mut self, signals: &CosimSignals) -> SimResult<()> {
        for device in self.buses.iter_mut().flat_map(|bus| &mut bus.devices) {
            let path = format!("SPI device '{}'", device.id);
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
        cs: u32,
        mosi: &[u8],
    ) -> SimResult<FlexSpiTransfer> {
        let bus = self
            .buses
            .get(bus_id as usize)
            .ok_or_else(|| error(format!("unknown Flex SPI bus id {bus_id}")))?;
        if mosi.len() > FLEX_SPI_MAX_TRANSFER_BYTES {
            return Err(error(format!(
                "an SPI transfer exchanges at most {FLEX_SPI_MAX_TRANSFER_BYTES} bytes"
            )));
        }
        let frequency_hz = u64::from(bus.frequency_hz);
        let bit_ns = |bits: u64| bits * 1_000_000_000 / frequency_hz;
        let bits = 8 * mosi.len() as u64;
        let device = bus.devices.iter().position(|device| device.cs == cs);
        let mut miso = Vec::new();
        if let Some(index) = device {
            // The device exchanges each byte at its last SCK edge, when the
            // MOSI byte is complete. Its clock runs between the bytes, so a
            // command's delay counts from the end of the command byte.
            self.buses[bus_id as usize].devices[index].model.cs_select();
            let mut at_ns = 0;
            for (byte_index, byte) in mosi.iter().enumerate() {
                let to_ns = bit_ns(8 * (byte_index as u64 + 1));
                self.run_clocks(to_ns - at_ns);
                self.clock_ahead_ns += to_ns - at_ns;
                at_ns = to_ns;
                let model = &mut self.buses[bus_id as usize].devices[index].model;
                miso.push(model.transfer(*byte));
            }
            let model = &mut self.buses[bus_id as usize].devices[index].model;
            model.cs_release();
            // Device pin outputs (DRDY, IRQ) have no pad to reach here.
            model.take_pin_drives();
        }
        let selected = device.is_some();
        let bus = &mut self.buses[bus_id as usize];
        let transfer = FlexSpiTransfer {
            bus: bus_id,
            time_ns,
            duration_ns: bit_ns(bits),
            cs,
            selected,
            mosi: mosi.to_vec(),
            miso,
        };
        *bus.counts.entry((cs, selected)).or_default() += 1;
        if self.ring.len() == TRANSFER_RING_CAPACITY {
            self.ring.pop_front();
        }
        self.ring.push_back(transfer.clone());
        self.recorded += 1;
        Ok(transfer)
    }

    /// Exact number of transfers on a bus since the start, optionally of one
    /// chip select and of one outcome.
    pub(crate) fn transfer_count(
        &self,
        bus_id: u32,
        cs: Option<u32>,
        selected: Option<bool>,
    ) -> Option<u64> {
        let bus = self.buses.get(bus_id as usize)?;
        Some(
            bus.counts
                .iter()
                .filter(|((counted, answered), _)| {
                    cs.is_none_or(|cs| cs == *counted)
                        && selected.is_none_or(|selected| selected == *answered)
                })
                .map(|(_, count)| count)
                .sum(),
        )
    }

    pub(crate) fn snapshot(&self, cursor: u64) -> FlexSpiTraceBatch {
        let dropped = self.recorded - self.ring.len() as u64;
        let skip = cursor.saturating_sub(dropped).min(self.ring.len() as u64) as usize;
        FlexSpiTraceBatch {
            time_unit: "ns",
            buses: self
                .summaries
                .iter()
                .enumerate()
                .map(|(index, bus)| FlexSpiBusChannel {
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

    const BOARD: &str =
        include_str!("../../wasm/tests/fixtures/flex-heypcb-esp32-max31855/system.yaml");

    fn error(yaml: &str) -> String {
        match FlexRuntime::from_yaml(yaml) {
            Ok(_) => panic!("manifest must be rejected"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn firmware_side_transfers_reach_the_declarative_catalog_model() {
        let mut runtime = FlexRuntime::from_yaml(BOARD).unwrap();
        let bus = runtime.spi_bus_id("sensor_spi").unwrap();
        assert_eq!(runtime.bus_id("sensor_spi").unwrap(), bus);
        assert!(runtime.spi_bus_id("missing").is_err());
        assert!(runtime.bus_id("missing").is_err());

        // 25 °C -> 100 counts in D31..D18; 22 °C -> 352 counts in D15..D4.
        let frame = runtime.spi_transfer(bus, 10, &[0; 4]).unwrap();
        assert!(frame.selected);
        assert_eq!(frame.miso, ((100u32 << 18) | (352 << 4)).to_be_bytes());
        // 32 SCK periods at 1 MHz. The runtime leaves time to its caller.
        assert_eq!((frame.time_ns, frame.duration_ns), (0, 32_000));
        assert_eq!(runtime.time_ns(), 0);

        // The stimulus reaches the model through the port, not a side door.
        let port = runtime.port_id("thermocouple_c").unwrap();
        runtime.write(port, -200.25).unwrap();
        let cold = runtime.spi_transfer(bus, 10, &[0; 4]).unwrap();
        let raw = u32::from_be_bytes(cold.miso.as_slice().try_into().unwrap());
        assert_eq!((raw as i32) >> 18, -801);

        // Nothing is wired to CS 11: no MISO bytes, but the bus time passes.
        let absent = runtime.spi_transfer(bus, 11, &[0xAA, 0x55]).unwrap();
        assert!(!absent.selected);
        assert!(absent.miso.is_empty());
        assert_eq!(absent.mosi, [0xAA, 0x55]);
        assert_eq!(absent.duration_ns, 16_000);

        assert_eq!(runtime.spi_transfer_count(bus, None, None), Some(3));
        assert_eq!(
            runtime.spi_transfer_count(bus, Some(10), Some(true)),
            Some(2)
        );
        assert_eq!(
            runtime.spi_transfer_count(bus, Some(10), Some(false)),
            Some(0)
        );
        assert_eq!(runtime.spi_transfer_count(bus, None, Some(false)), Some(1));
        assert_eq!(runtime.spi_transfer_count(bus + 1, None, None), None);

        let trace = runtime.spi_trace_snapshot(0);
        assert_eq!((trace.next_cursor, trace.dropped), (3, 0));
        assert_eq!(trace.buses[0].name, "sensor_spi");
        assert_eq!(trace.transfers, [frame, cold, absent.clone()]);
        assert_eq!(runtime.spi_trace_snapshot(2).transfers, [absent]);

        let error = runtime
            .spi_transfer(bus, 10, &vec![0; 4097])
            .unwrap_err()
            .to_string();
        assert!(error.contains("at most 4096 bytes"), "{error}");
        assert!(runtime.spi_transfer(bus + 1, 10, &[0]).is_err());
        // A rejected call is not a transfer.
        assert_eq!(runtime.spi_transfer_count(bus, None, None), Some(3));

        let report = serde_json::to_value(runtime.run_report()).unwrap();
        let device = &report["spi_buses"][0]["devices"][0];
        assert_eq!(device["model_type"], "max31855");
        assert_eq!(device["cs"], 10);
        assert_eq!(device["inputs"][0]["name"], "internal");
        assert!(report["spi_buses"][0]["not_modeled"]
            .as_array()
            .unwrap()
            .contains(&"spi_mode_cpol_cpha".into()));
    }

    #[test]
    fn a_manifest_names_only_what_a_flex_spi_bus_can_run() {
        let cases = [
            (
                "type: max31855",
                "type: sht40",
                "no declarative SPI device model of type 'sht40'",
            ),
            (
                "type: max31855",
                "type: mcp2515",
                "no declarative SPI device model of type 'mcp2515'",
            ),
            (
                "        cs: 10\n",
                "        cs: 10\n        config: { spi_mode: 1 }\n",
                "config 'spi_mode' is not supported",
            ),
            (
                "        cs: 10\n",
                "        cs: 10\n        config: { cs_pin: PA4 }\n",
                "config 'cs_pin' is not supported",
            ),
            (
                "frequency_hz: 1000000",
                "frequency_hz: 0",
                "frequency_hz must be within",
            ),
            (
                "internal: environment.cold_junction_temperature",
                "cold: environment.cold_junction_temperature",
                "no input channel 'cold'",
            ),
            (
                "spi_buses:\n",
                "i2c_buses:\n  - { name: sensor_spi, frequency_hz: 100000 }\nspi_buses:\n",
                "duplicate bus name 'sensor_spi'",
            ),
        ];
        for (from, to, expected) in cases {
            assert!(BOARD.contains(from), "{from}");
            let message = error(&BOARD.replacen(from, to, 1));
            assert!(message.contains(expected), "{expected}: {message}");
        }
        let last_input = "          internal: environment.cold_junction_temperature\n";
        assert!(BOARD.contains(last_input));
        let shared = BOARD.replacen(
            last_input,
            &format!("{last_input}      - {{ id: second, type: mcp3008, cs: 10 }}\n"),
            1,
        );
        let message = error(&shared);
        assert!(message.contains("share cs 10"), "{message}");
    }
}
