//! Browser-safe multi-node World wrapper.

use crate::inspect::LogicRef;
use labwired_config::{
    BoardIoBinding, BoardIoKind, BoardIoSignal, ChipDescriptor, EnvironmentManifest, SystemManifest,
};
use labwired_core::machine::world_hooks::ObserverRef;
use labwired_core::system::node::NodeFirmware;
use labwired_core::world::{ResolvedWorldNode, World};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;

/// One node as the page hands it to [`WasmWorld::new_from_resolved`].
#[derive(Deserialize)]
struct ResolvedNodeInput {
    id: String,
    system_yaml: String,
    chip_yaml: String,
    firmware: Vec<u8>,
    /// Optional `{ name: Uint8Array }` — the same named-blob channel, under the
    /// same names, as the single-chip `new_from_config(system, chip,
    /// firmware, blobs)`: `esp32c3_irom` / `esp32c3_drom` and `esp32s3_irom` /
    /// `esp32s3_drom` carry the mask ROM an ESP node boots against. The
    /// browser bundle carries no ROM, so an ESP32-S3 flash-image node needs
    /// these, an S3 ELF node without them falls back to the thunk harness
    /// exactly as the single-chip S3 fast boot does, and a C3 node falls back
    /// to an image registered with `register_esp32c3_rom`.
    #[serde(default)]
    blobs: HashMap<String, Vec<u8>>,
}

#[wasm_bindgen]
pub struct WasmWorld {
    world: World,
    uart_sinks: HashMap<String, Arc<Mutex<Vec<u8>>>>,
    /// Each node's `board_io` bindings (LEDs, buttons), from its system YAML.
    board_io: HashMap<String, Vec<BoardIoBinding>>,
}

fn observer_refs(refs: JsValue) -> Option<Vec<ObserverRef>> {
    let refs: Vec<LogicRef> = serde_wasm_bindgen::from_value(refs).ok()?;
    Some(
        refs.into_iter()
            .map(|r| ObserverRef {
                kind: r.kind,
                peripheral: r.peripheral,
                pin: r.pin,
                line: r.line,
            })
            .collect(),
    )
}

/// Per-node instruments: the same logic-analyzer, input and board-IO surface
/// a single `WasmSimulator` has, addressed by node id, so the page's panels
/// run unchanged on a node of a world.
#[wasm_bindgen]
impl WasmWorld {
    /// Arm node `node_id`'s logic-analyzer watch set (`[{kind, peripheral,
    /// pin | line}]`), next to the world's own pad watches. Same rows as
    /// `WasmSimulator::watch_logic_signals`.
    pub fn watch_logic_signals(
        &mut self,
        node_id: &str,
        refs: JsValue,
    ) -> Result<JsValue, JsValue> {
        let parsed = observer_refs(refs.clone()).ok_or_else(|| {
            JsValue::from_str("watch_logic_signals: refs are not [{kind, peripheral, pin|line}]")
        })?;
        let machine = self
            .world
            .machines
            .get_mut(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?;
        let rows = machine.observer_watch(&parsed);
        let input: Vec<LogicRef> = serde_wasm_bindgen::from_value(refs).unwrap_or_default();
        let out: Vec<serde_json::Value> = input
            .iter()
            .zip(rows)
            .enumerate()
            .map(|(ch, (r, row))| {
                let mut o = serde_json::Map::new();
                o.insert("kind".into(), serde_json::json!(r.kind));
                o.insert("peripheral".into(), serde_json::json!(r.peripheral));
                if r.kind == "wire" {
                    o.insert("line".into(), serde_json::json!(r.line));
                } else {
                    o.insert("pin".into(), serde_json::json!(r.pin));
                }
                o.insert("ch".into(), serde_json::json!(ch));
                o.insert("value".into(), serde_json::json!(row.initial));
                if let Some(e) = row.error {
                    o.insert("error".into(), serde_json::json!(e));
                }
                serde_json::Value::Object(o)
            })
            .collect();
        serde_wasm_bindgen::to_value(&out)
            .map_err(|e| JsValue::from_str(&format!("watch_logic_signals: {e}")))
    }

    /// Level edges of node `node_id`'s watch set since `cursor`; same shape as
    /// `WasmSimulator::read_logic_edges`.
    pub fn read_logic_edges(&mut self, node_id: &str, cursor: f64) -> Result<JsValue, JsValue> {
        let machine = self
            .world
            .machines
            .get_mut(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?;
        let batch = machine.observer_edges(cursor as u64);
        let edges: Vec<serde_json::Value> = batch
            .edges
            .iter()
            .map(|e| serde_json::json!({ "ch": e.ch, "cycle": e.cycle as f64, "value": e.value }))
            .collect();
        let out = serde_json::json!({
            "cursor": batch.cursor as f64,
            "dropped": batch.dropped as f64,
            "nowCycle": machine.total_cycles() as f64,
            "edges": edges,
        });
        serde_wasm_bindgen::to_value(&out)
            .map_err(|e| JsValue::from_str(&format!("read_logic_edges: {e}")))
    }

    /// Four-state (`0`/`1`/`z`/`x`) edges of node `node_id`'s watch set; same
    /// shape as `WasmSimulator::read_logic_states`.
    pub fn read_logic_states(&mut self, node_id: &str, cursor: f64) -> Result<JsValue, JsValue> {
        let machine = self
            .world
            .machines
            .get_mut(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?;
        let (batch, initial) = machine.observer_states(cursor as u64);
        let initial: Vec<Option<String>> = initial
            .iter()
            .map(|s| s.map(|s| s.as_char().to_string()))
            .collect();
        let edges: Vec<serde_json::Value> = batch
            .edges
            .iter()
            .map(|e| {
                serde_json::json!({
                    "ch": e.ch,
                    "cycle": e.cycle as f64,
                    "state": e.state.as_char().to_string(),
                })
            })
            .collect();
        let out = serde_json::json!({
            "cursor": batch.cursor as f64,
            "dropped": batch.dropped as f64,
            "nowCycle": machine.total_cycles() as f64,
            "initial": initial,
            "edges": edges,
        });
        serde_wasm_bindgen::to_value(&out)
            .map_err(|e| JsValue::from_str(&format!("read_logic_states: {e}")))
    }

    /// The level each ref reads now on node `node_id` (`value: bool | null`).
    pub fn sample_logic_signals(&self, node_id: &str, refs: JsValue) -> Result<JsValue, JsValue> {
        let parsed = observer_refs(refs.clone()).ok_or_else(|| {
            JsValue::from_str("sample_logic_signals: refs are not [{kind, peripheral, pin|line}]")
        })?;
        let machine = self.machine(node_id)?;
        let levels = machine.observer_sample(&parsed);
        let input: Vec<LogicRef> = serde_wasm_bindgen::from_value(refs).unwrap_or_default();
        let out: Vec<serde_json::Value> = input
            .iter()
            .zip(levels)
            .map(|(r, v)| {
                if r.kind == "wire" {
                    serde_json::json!({ "kind": r.kind, "peripheral": r.peripheral, "line": r.line, "value": v })
                } else {
                    serde_json::json!({ "kind": r.kind, "peripheral": r.peripheral, "pin": r.pin, "value": v })
                }
            })
            .collect();
        serde_wasm_bindgen::to_value(&out)
            .map_err(|e| JsValue::from_str(&format!("sample_logic_signals: {e}")))
    }

    /// Drive a simulated input channel on node `node_id`.
    pub fn set_input(&mut self, node_id: &str, channel: &str, value: f64) -> Result<(), JsValue> {
        self.world
            .machines
            .get_mut(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?
            .set_input_channel(channel, value)
            .map_err(|e| JsValue::from_str(&e))
    }

    /// The drivable input channels on node `node_id`, as
    /// `WasmSimulator::list_inputs` reports them.
    pub fn list_inputs(&mut self, node_id: &str) -> Result<JsValue, JsValue> {
        let machine = self
            .world
            .machines
            .get_mut(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?;
        let entries: Vec<serde_json::Value> = machine
            .list_input_channels()
            .into_iter()
            .map(|(peripheral, ch)| {
                serde_json::json!({
                    "peripheral": peripheral,
                    "key": ch.key,
                    "label": ch.label,
                    "unit": ch.unit,
                    "min": ch.min,
                    "max": ch.max,
                })
            })
            .collect();
        serde_wasm_bindgen::to_value(&entries).map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Node `node_id`'s `board_io` bindings.
    pub fn get_board_io_config(&self, node_id: &str) -> Result<JsValue, JsValue> {
        let bindings = self
            .board_io
            .get(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?;
        serde_wasm_bindgen::to_value(bindings.as_slice())
            .map_err(|e| JsValue::from_str(&format!("get_board_io_config: {e}")))
    }

    /// Node `node_id`'s `board_io` states, `[{ id, active }]`.
    pub fn get_board_io_states(&self, node_id: &str) -> Result<JsValue, JsValue> {
        let machine = self.machine(node_id)?;
        let bindings = self
            .board_io
            .get(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?;
        let states: Vec<serde_json::Value> = bindings
            .iter()
            .map(|b| {
                let high = match b.kind {
                    BoardIoKind::Led | BoardIoKind::PwmOutput => {
                        machine.gpio_level(&b.peripheral, b.pin, true)
                    }
                    BoardIoKind::Button => machine.gpio_level(&b.peripheral, b.pin, false),
                    _ => None,
                };
                let active = match (b.kind, high) {
                    (
                        BoardIoKind::AdcInput
                        | BoardIoKind::I2cDevice
                        | BoardIoKind::SpiDevice
                        | BoardIoKind::UartDevice,
                        _,
                    ) => false,
                    (_, high) => {
                        let high = high.unwrap_or(false);
                        if b.active_high {
                            high
                        } else {
                            !high
                        }
                    }
                };
                serde_json::json!({ "id": b.id, "active": active })
            })
            .collect();
        serde_wasm_bindgen::to_value(&states)
            .map_err(|e| JsValue::from_str(&format!("get_board_io_states: {e}")))
    }

    /// Press or release an input `board_io` binding (a button) on node `node_id`.
    pub fn set_board_io_input(
        &mut self,
        node_id: &str,
        id: &str,
        active: bool,
    ) -> Result<(), JsValue> {
        let binding = self
            .board_io
            .get(node_id)
            .and_then(|all| {
                all.iter()
                    .find(|b| b.id == id && b.signal == BoardIoSignal::Input)
            })
            .cloned()
            .ok_or_else(|| JsValue::from_str(&format!("No input board_io binding '{id}'")))?;
        let machine = self
            .world
            .machines
            .get_mut(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?;
        let pin_high = if binding.active_high { active } else { !active };
        if machine.drive_gpio_input(&binding.peripheral, binding.pin, pin_high) {
            Ok(())
        } else {
            Err(JsValue::from_str(&format!(
                "Peripheral '{}' does not expose GPIO input control",
                binding.peripheral
            )))
        }
    }
}

#[wasm_bindgen]
impl WasmWorld {
    /// Build a world from the environment YAML and the page's resolved nodes:
    /// `[{ id, system_yaml, chip_yaml, firmware: Uint8Array, blobs? }]`, where
    /// `blobs` is an optional `{ name: Uint8Array }` map with the single-chip
    /// `new_from_config` names (an ESP node's mask ROM). A node's boot profile
    /// (`profile: arduino-esp32` for a classic-ESP32 Arduino sketch) is read
    /// from the environment YAML.
    #[wasm_bindgen(js_name = new_from_resolved)]
    pub fn new_from_resolved(environment_yaml: &str, nodes: JsValue) -> Result<WasmWorld, JsValue> {
        let manifest: EnvironmentManifest = serde_yaml::from_str(environment_yaml)
            .map_err(|error| JsValue::from_str(&format!("Environment YAML error: {error}")))?;
        let inputs: Vec<ResolvedNodeInput> = serde_wasm_bindgen::from_value(nodes)
            .map_err(|error| JsValue::from_str(&format!("Resolved nodes error: {error}")))?;
        Self::from_node_inputs(manifest, inputs).map_err(|error| JsValue::from_str(&error))
    }

    pub fn node_ids(&self) -> JsValue {
        let mut ids = self.world.machines.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        serde_wasm_bindgen::to_value(&ids).unwrap_or(JsValue::NULL)
    }

    pub fn step_batch(&mut self, rounds: u32) -> Result<u32, JsValue> {
        // `rounds` world steps; a GPIO-net world runs them in one go.
        for (id, result) in self.world.step_rounds(u64::from(rounds)) {
            result.map_err(|error| JsValue::from_str(&format!("node '{id}' step: {error:?}")))?;
        }
        Ok(rounds)
    }

    pub fn step_single(&mut self) -> Result<(), JsValue> {
        self.step_batch(1).map(|_| ())
    }

    pub fn get_pc(&self, node_id: &str) -> Result<u32, JsValue> {
        Ok(self.machine(node_id)?.get_pc())
    }

    pub fn get_register(&self, node_id: &str, id: u32) -> Result<u32, JsValue> {
        Ok(self.machine(node_id)?.get_register(id as usize))
    }

    pub fn get_register_names(&self, node_id: &str) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.machine(node_id)?.get_register_names())
            .map_err(|error| JsValue::from_str(&format!("node '{node_id}' registers: {error}")))
    }

    pub fn read_memory(&self, node_id: &str, address: u32, len: u32) -> Result<Vec<u8>, JsValue> {
        self.machine(node_id)?
            .read_memory(address, len as usize)
            .map_err(|error| JsValue::from_str(&format!("node '{node_id}' memory read: {error:?}")))
    }

    pub fn total_cycles(&self, node_id: &str) -> Result<u64, JsValue> {
        self.world
            .machines
            .get(node_id)
            .map(|machine| machine.total_cycles())
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))
    }

    pub fn read_u8(&self, node_id: &str, address: u32) -> Result<u8, JsValue> {
        self.world
            .machines
            .get(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?
            .read_u8(address as u64)
            .map_err(|error| JsValue::from_str(&format!("node '{node_id}' read: {error:?}")))
    }

    pub fn node_snapshot(&self, node_id: &str) -> Result<JsValue, JsValue> {
        let snapshot = self
            .world
            .machines
            .get(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?
            .snapshot()
            .ok_or_else(|| {
                JsValue::from_str(&format!("node '{node_id}' has no snapshot support"))
            })?;
        serde_wasm_bindgen::to_value(&snapshot)
            .map_err(|error| JsValue::from_str(&format!("node '{node_id}' snapshot: {error}")))
    }

    pub fn get_ssd1306_framebuffer(
        &self,
        node_id: &str,
        device_id: &str,
    ) -> Result<Box<[u8]>, JsValue> {
        let artifact = self
            .world
            .machines
            .get(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?
            .display_artifact(device_id, true)
            .ok_or_else(|| {
                JsValue::from_str(&format!("node '{node_id}' has no display '{device_id}'"))
            })?;
        let format = artifact.meta.get("format").and_then(|value| value.as_str());
        if format != Some(labwired_core::inspect::artifact_format::SSD1306_PAGE) {
            return Err(JsValue::from_str(&format!(
                "node '{node_id}' display '{device_id}' is not SSD1306"
            )));
        }
        Ok(artifact
            .bytes
            .ok_or_else(|| JsValue::from_str("SSD1306 artifact omitted framebuffer bytes"))?
            .into_boxed_slice())
    }

    pub fn bus_trace_snapshot(&self, node_id: &str) -> Result<JsValue, JsValue> {
        let trace = self
            .world
            .machines
            .get(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?
            .bus_trace_snapshot();
        serde_wasm_bindgen::to_value(&trace)
            .map_err(|error| JsValue::from_str(&format!("node '{node_id}' bus trace: {error}")))
    }

    pub fn fdcan_trace_snapshot(&self, node_id: &str) -> Result<JsValue, JsValue> {
        let trace = self
            .world
            .machines
            .get(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?
            .bus_trace_snapshot();
        let frames = labwired_core::peripherals::can_trace_snapshot_all(&trace);
        serde_wasm_bindgen::to_value(&frames)
            .map_err(|error| JsValue::from_str(&format!("node '{node_id}' CAN trace: {error}")))
    }

    pub fn air_trace_snapshot(&self) -> JsValue {
        serde_wasm_bindgen::to_value(
            &labwired_core::peripherals::nrf52::radio::virtual_air_trace_snapshot(),
        )
        .unwrap_or(JsValue::NULL)
    }

    /// Hand the page's ESP32-C3 mask ROM (IROM 384 KiB, DROM 128 KiB) to the
    /// engine for every C3 node built afterwards in this worker. The browser
    /// has no filesystem and no vendored copy.
    ///
    /// Kept for pages that already call it. The general mechanism is a node's
    /// own `blobs` in [`Self::new_from_resolved`] (`esp32c3_irom` /
    /// `esp32c3_drom`, the names the single-chip path takes), which covers the
    /// ESP32-S3 too and wins over this registration.
    #[wasm_bindgen(js_name = register_esp32c3_rom)]
    pub fn register_esp32c3_rom(irom: Vec<u8>, drom: Vec<u8>) -> Result<(), JsValue> {
        labwired_core::boot::esp32c3_rom::register_rom_images(irom, drom)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Simulated time of the slowest node, ns (a BLE world steps in time
    /// lockstep, so every node is within 10 µs of it).
    pub fn time_ns(&self) -> Result<f64, JsValue> {
        Ok(self.world_time_ns() as f64)
    }

    /// The scripted BLE centrals (`ble_central` interconnects): connection
    /// state, discovered GATT database, reads, writes, notifications and the
    /// full transcript. `[{ id, report }]`, manifest order.
    pub fn ble_centrals(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.ble_central_views())
            .map_err(|error| JsValue::from_str(&format!("BLE centrals: {error}")))
    }

    /// The world's BLE air, most recent first (at most 200 frames), each
    /// decoded: advertising, LL control, empty PDUs and ATT operations.
    pub fn ble_air_trace(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.ble_air_views())
            .map_err(|error| JsValue::from_str(&format!("BLE air trace: {error}")))
    }

    /// Whether this world runs a timed UART network (`uart_network`).
    pub fn has_uart_network(&self) -> Result<bool, JsValue> {
        Ok(self.world.uart_network_now_ps().is_some())
    }

    /// The timed UART network: per-link statistics (bytes, throughput,
    /// in-flight depth, overruns, framing errors, latency), tagged messages
    /// with per-node latency, and the unified timeline (TX start, delivery,
    /// RX interrupt, overrun, node reset, link changes, GPIO markers) from
    /// sequence number `since` on, ordered by world time. Times are
    /// picoseconds. `null` when the world has no `uart_network`.
    pub fn uart_network_report(&self, since: f64) -> Result<JsValue, JsValue> {
        let Some(report) = self.world.uart_network_report(since.max(0.0) as u64) else {
            return Ok(JsValue::NULL);
        };
        report
            .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            .map_err(|error| JsValue::from_str(&format!("UART network report: {error}")))
    }

    /// Whether this world has GPIO nets (`gpio_net` interconnects).
    pub fn has_gpio_nets(&self) -> Result<bool, JsValue> {
        Ok(!self.world.gpio_net_reports().is_empty())
    }

    /// Every GPIO net: its level, edge count, members and their drive, and
    /// the diagnostics (`GPIO_NET_CONTENTION`, `GPIO_NET_FLOATING`) with
    /// times in picoseconds, manifest order. `[]` without `gpio_net`s.
    pub fn gpio_net_report(&self) -> Result<JsValue, JsValue> {
        self.world
            .gpio_net_reports()
            .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            .map_err(|error| JsValue::from_str(&format!("GPIO net report: {error}")))
    }

    pub fn drain_uart_output(&self, node_id: &str) -> Result<Vec<u8>, JsValue> {
        let sink = self
            .uart_sinks
            .get(node_id)
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))?;
        let mut bytes = sink
            .lock()
            .map_err(|_| JsValue::from_str("world UART sink lock poisoned"))?;
        Ok(bytes.drain(..).collect())
    }

    fn machine(&self, node_id: &str) -> Result<&dyn labwired_core::world::MachineTrait, JsValue> {
        self.world
            .machines
            .get(node_id)
            .map(|machine| machine.as_ref())
            .ok_or_else(|| JsValue::from_str(&format!("unknown world node '{node_id}'")))
    }
}

/// One scripted central, as the page sees it.
#[derive(serde::Serialize)]
struct BleCentralView {
    id: String,
    report: labwired_core::peripherals::ble_central::CentralReport,
}

/// One frame on the world's BLE air, decoded for the page.
#[derive(serde::Serialize)]
struct BleAirFrameView {
    /// Air time of the first bit, ns (world time).
    air_ns: Option<u64>,
    channel: u8,
    access_address: u32,
    /// Transmitter: a node's radio or a scripted central (opaque number).
    source: u64,
    pdu: Vec<u8>,
    /// Human-readable decode (`ATT Read Request handle 0x002a`, …).
    text: String,
}

impl WasmWorld {
    fn world_time_ns(&self) -> u64 {
        self.world
            .machines
            .keys()
            .filter_map(|id| self.world.node_time_ns(id))
            .min()
            .unwrap_or(0)
    }

    fn ble_central_views(&self) -> Vec<BleCentralView> {
        self.world
            .ble_central_reports()
            .into_iter()
            .map(|(id, report)| BleCentralView { id, report })
            .collect()
    }

    fn ble_air_views(&self) -> Vec<BleAirFrameView> {
        let Some(air) = self.world.ble_air() else {
            return Vec::new();
        };
        air.trace_snapshot()
            .into_iter()
            .map(|f| BleAirFrameView {
                text: labwired_core::peripherals::ble_central::describe_pdu(&f),
                air_ns: f.air_ns,
                channel: f.channel,
                access_address: f.access_address,
                source: f.source,
                pdu: f.pdu,
            })
            .collect()
    }

    /// [`Self::new_from_resolved`] past the JS boundary, so a native test can
    /// reach it: `serde_wasm_bindgen` and `JsValue` only work in a wasm host.
    fn from_node_inputs(
        manifest: EnvironmentManifest,
        inputs: Vec<ResolvedNodeInput>,
    ) -> Result<WasmWorld, String> {
        let mut board_io = HashMap::new();
        let resolved = inputs
            .into_iter()
            .map(|input| {
                let system: SystemManifest = serde_yaml::from_str(&input.system_yaml)
                    .map_err(|error| format!("node '{}': system YAML: {error}", input.id))?;
                let chip: ChipDescriptor = serde_yaml::from_str(&input.chip_yaml)
                    .map_err(|error| format!("node '{}': chip YAML: {error}", input.id))?;
                board_io.insert(input.id.clone(), system.board_io.clone());
                Ok(ResolvedWorldNode {
                    id: input.id,
                    system,
                    chip,
                    firmware: NodeFirmware::from_bytes(input.firmware),
                    blobs: input.blobs,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut world = World::from_resolved(manifest, resolved)
            .map_err(|error| format!("World construction error: {error:#}"))?;
        let mut uart_sinks = HashMap::new();
        for (id, machine) in &mut world.machines {
            let sink = Arc::new(Mutex::new(Vec::new()));
            machine
                .attach_uart_tx_sink(sink.clone(), false)
                .map_err(|error| format!("node '{id}': UART sink: {error:#}"))?;
            uart_sinks.insert(id.clone(), sink);
        }
        Ok(Self {
            world,
            uart_sinks,
            board_io,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The browser's world path end to end, minus the JS boundary: a
    /// `WasmWorld` built from resolved inputs (as the page builds it) runs the
    /// stock Arduino `BLE_notify` GATT server on an ESP32-C3 and a
    /// `ble_central` "phone" that connects, discovers, reads, writes,
    /// subscribes and disconnects; the page reads it back through
    /// `ble_central_views` / `ble_air_views` (the `ble_centrals()` /
    /// `ble_air_trace()` bindings).
    ///
    /// Needs the fetched flash image (see
    /// `crates/core/tests/world_esp32c3_ble_gatt.rs`); `LABWIRED_REQUIRE_C3_BLE=1`
    /// makes its absence a failure. Release: a faithful ROM boot.
    #[test]
    #[ignore = "faithful C3 ROM boot; release + fetched fixture"]
    fn a_wasm_world_runs_a_scripted_phone_against_a_c3_gatt_server() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let flash =
            match std::fs::read(root.join("fixtures/esp32c3-ble/c3-ble-gatt-notify-flash.bin")) {
                Ok(bytes) => bytes,
                Err(_) if std::env::var("LABWIRED_REQUIRE_C3_BLE").as_deref() != Ok("1") => {
                    eprintln!(
                        "SKIP: fixtures/esp32c3-ble/c3-ble-gatt-notify-flash.bin not fetched"
                    );
                    return;
                }
                Err(e) => panic!("c3-ble-gatt-notify-flash.bin: {e} (LABWIRED_REQUIRE_C3_BLE=1)"),
            };
        // The page registers the ROM it fetched; do the same with the repo copy.
        WasmWorld::register_esp32c3_rom(
            std::fs::read(root.join("crates/core/roms/esp32c3/esp32c3_rom.bin")).unwrap(),
            std::fs::read(root.join("crates/core/roms/esp32c3/esp32c3_drom.bin")).unwrap(),
        )
        .map_err(|_| "rom")
        .unwrap();
        let uuid = "beb5483e-36e1-4688-b7f5-ea07361b26a8";
        let environment: EnvironmentManifest = serde_yaml::from_str(&format!(
            r#"
schema_version: "1.0"
name: phone-lab
nodes:
  - {{ id: server, system: s.yaml, firmware: f.bin }}
interconnects:
  - type: ble_central
    nodes: [server]
    config:
      id: phone
      target_name: ESP32
      script:
        - connect
        - discover
        - read: {uuid}
        - write: {{ uuid: {uuid}, text: hi }}
        - subscribe: {uuid}
        - wait_notify: {{ count: 2 }}
        - disconnect
"#
        ))
        .unwrap();
        let node = ResolvedNodeInput {
            id: "server".into(),
            system_yaml: include_str!("../../../configs/systems/esp32c3-devkit.yaml").into(),
            chip_yaml: include_str!("../../../configs/chips/esp32c3.yaml").into(),
            firmware: flash,
            blobs: HashMap::new(),
        };
        let mut world = WasmWorld::from_node_inputs(environment, vec![node]).expect("world");
        let mut batches = 0;
        while batches < 2_000 {
            world.step_batch(200_000).map_err(|_| "step").unwrap();
            batches += 1;
            if world
                .ble_central_views()
                .first()
                .is_some_and(|c| c.report.script_done)
            {
                break;
            }
        }
        let views = world.ble_central_views();
        let phone = &views[0];
        assert_eq!(phone.id, "phone");
        let r = &phone.report;
        assert!(
            r.script_done,
            "script did not finish in {batches} batches: {:?}",
            r.log
        );
        assert_eq!(r.reads.len(), 1);
        assert_eq!(r.writes_acked.len(), 1);
        assert!(r.notification_count >= 2);
        // The air trace the page draws carries decoded GATT traffic.
        let air = world.ble_air_views();
        assert!(!air.is_empty());
        assert!(
            air.iter()
                .any(|f| f.text.starts_with("ATT Handle Value Notification")),
            "no notification in the air trace"
        );
        assert!(world.world_time_ns() > 0);
    }

    /// The browser's world path runs a timed UART network: three STM32F401
    /// nodes of the `uart-chain` fixture, built from resolved inputs as the
    /// page builds them (UART sinks attached afterwards, as `from_node_inputs`
    /// does), deliver messages one hop = 49.5 bit times apart, and the report
    /// the page reads carries the timeline and per-link statistics.
    #[test]
    fn a_wasm_world_runs_a_timed_uart_chain() {
        let environment: EnvironmentManifest = serde_yaml::from_str(
            r#"
schema_version: "1.0"
name: uart-chain
nodes:
  - { id: n0, system: s.yaml, firmware: f.elf }
  - { id: n1, system: s.yaml, firmware: f.elf }
  - { id: n2, system: s.yaml, firmware: f.elf }
interconnects:
  - type: uart_network
    nodes: [n0, n1, n2]
    config:
      messages: { sync: 0xA5, length: 5, id_offset: 1, id_bytes: 2, hop_offset: 3, checksum_xor: true }
      markers:
        - { node: n2, peripheral: gpioa, pin: 5, name: app }
"#,
        )
        .expect("environment manifest");
        let fixture = |name: &str| {
            std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../core/tests/fixtures/uart-chain")
                    .join(name),
            )
            .expect("uart-chain fixture")
        };
        let node = |id: &str, fw: &str| ResolvedNodeInput {
            id: id.to_string(),
            system_yaml: include_str!("../../../configs/systems/nucleo-f401re.yaml").to_string(),
            chip_yaml: include_str!("../../../configs/chips/stm32f401.yaml").to_string(),
            firmware: fixture(fw),
            blobs: HashMap::new(),
        };
        let mut world = WasmWorld::from_node_inputs(
            environment,
            vec![
                node("n0", "uart-chain-source.elf"),
                node("n1", "uart-chain-relay.elf"),
                node("n2", "uart-chain-relay.elf"),
            ],
        )
        .expect("world");
        while world.world.uart_network_now_ps().unwrap() < 4_000_000_000 {
            world.step_batch(1).map_err(|_| "step").unwrap();
        }
        let report = world.world.uart_network_report(0).unwrap();
        let hop = 729u64 * 1_000_000_000_000 / 84_000_000 * 99 / 2;
        let m0 = report
            .messages
            .iter()
            .find(|m| m.id == 0)
            .expect("message 0");
        let at_n1 = m0.deliveries.iter().find(|d| d.node == "n1").unwrap();
        assert!(
            at_n1.latency_ps.unwrap().abs_diff(hop) < 30_000,
            "{at_n1:?}"
        );
        assert!(m0.deliveries.iter().any(|d| d.node == "n2"));
        assert_eq!(report.links.len(), 2);
        assert!(report.links[0].directions[0].stats.chars_delivered >= 10);
        use labwired_core::network::timed_uart::NetEventKind as K;
        for kind in [
            K::TxStart,
            K::Deliver,
            K::RxIrq,
            K::Marker,
            K::MessageDelivered,
        ] {
            assert!(report.events.iter().any(|e| e.kind == kind), "no {kind:?}");
        }
        assert!(world.world_time_ns() >= 4_000_000);
    }

    /// The browser's world path runs the same GPIO nets as the native one: an
    /// STM32G0B1 and an ATmega328P count each other's edges exactly.
    #[test]
    fn a_wasm_world_runs_gpio_nets_between_two_boards() {
        let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/gpio-net-two-boards");
        let environment: EnvironmentManifest = serde_yaml::from_str(
            &std::fs::read_to_string(example.join("env.yaml")).expect("env.yaml"),
        )
        .expect("environment manifest");
        let fw = |name: &str| std::fs::read(example.join("firmware").join(name)).expect("elf");
        let mut world = WasmWorld::from_node_inputs(
            environment,
            vec![
                ResolvedNodeInput {
                    id: "stm".into(),
                    system_yaml: include_str!("../../../examples/stm32g0b1re/system.yaml").into(),
                    chip_yaml: include_str!("../../../configs/chips/stm32g0b1re.yaml").into(),
                    firmware: fw("stm.elf"),
                    blobs: HashMap::new(),
                },
                ResolvedNodeInput {
                    id: "avr".into(),
                    system_yaml: include_str!("../../../configs/systems/arduino-uno.yaml").into(),
                    chip_yaml: include_str!("../../../configs/chips/atmega328p.yaml").into(),
                    firmware: fw("avr.elf"),
                    blobs: HashMap::new(),
                },
            ],
        )
        .expect("world");
        assert!(world.has_gpio_nets().unwrap());
        while world.world.round_now_ps().unwrap() < 30_000_000_000 {
            world.step_batch(1).map_err(|_| "step").unwrap();
        }
        let ram = world
            .world
            .machines
            .get("stm")
            .unwrap()
            .read_memory(0x2000_0100, 20)
            .unwrap();
        let counts: Vec<u32> = ram
            .chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(counts, vec![10, 10, 3, 3, 1]);
        let nets = world.world.gpio_net_reports();
        let edges: Vec<(&str, u64)> = nets.iter().map(|n| (n.name.as_str(), n.edges)).collect();
        assert_eq!(edges, vec![("irq", 20), ("ready", 14), ("alert", 16)]);
        let avr_text = world.drain_uart_output("avr").unwrap();
        assert_eq!(
            String::from_utf8_lossy(&avr_text),
            "AVR ready=7 alert f=5 r=5\n"
        );
    }

    /// An instrument watching pads that sit on GPIO nets must not change what
    /// the nets do, and must see every edge: arm the logic analyzer on both
    /// boards halfway through the run (the worst moment: a round may be in
    /// flight) and the counts still match the un-watched run exactly.
    #[test]
    fn a_logic_analyzer_on_net_pads_does_not_disturb_the_nets() {
        let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/gpio-net-two-boards");
        let environment: EnvironmentManifest = serde_yaml::from_str(
            &std::fs::read_to_string(example.join("env.yaml")).expect("env.yaml"),
        )
        .expect("environment manifest");
        let fw = |name: &str| std::fs::read(example.join("firmware").join(name)).expect("elf");
        let mut world = WasmWorld::from_node_inputs(
            environment,
            vec![
                ResolvedNodeInput {
                    id: "stm".into(),
                    system_yaml: include_str!("../../../examples/stm32g0b1re/system.yaml").into(),
                    chip_yaml: include_str!("../../../configs/chips/stm32g0b1re.yaml").into(),
                    firmware: fw("stm.elf"),
                    blobs: HashMap::new(),
                },
                ResolvedNodeInput {
                    id: "avr".into(),
                    system_yaml: include_str!("../../../configs/systems/arduino-uno.yaml").into(),
                    chip_yaml: include_str!("../../../configs/chips/atmega328p.yaml").into(),
                    firmware: fw("avr.elf"),
                    blobs: HashMap::new(),
                },
            ],
        )
        .expect("world");
        let pad = |peripheral: &str, pin: u8| ObserverRef {
            kind: "gpio".into(),
            peripheral: peripheral.into(),
            pin,
            line: None,
        };
        let stm_refs = [pad("gpiob", 0), pad("gpiob", 1), pad("gpiob", 4)];
        let avr_refs = [pad("portd", 2), pad("portd", 3), pad("portd", 4)];
        let mut armed = false;
        let mut stm_edges = 0usize;
        let mut avr_edges = 0usize;
        let (mut stm_cursor, mut avr_cursor) = (0u64, 0u64);
        while world.world.round_now_ps().unwrap() < 30_000_000_000 {
            world.step_batch(1).map_err(|_| "step").unwrap();
            if !armed && world.world.round_now_ps().unwrap() > 12_345_678 {
                for (id, refs) in [("stm", &stm_refs[..]), ("avr", &avr_refs[..])] {
                    let rows = world
                        .world
                        .machines
                        .get_mut(id)
                        .unwrap()
                        .observer_watch(refs);
                    assert!(rows.iter().all(|r| r.error.is_none()), "{rows:?}");
                }
                armed = true;
            }
            if armed {
                let b = world
                    .world
                    .machines
                    .get_mut("stm")
                    .unwrap()
                    .observer_edges(stm_cursor);
                stm_cursor = b.cursor;
                stm_edges += b.edges.len();
                let b = world
                    .world
                    .machines
                    .get_mut("avr")
                    .unwrap()
                    .observer_edges(avr_cursor);
                avr_cursor = b.cursor;
                avr_edges += b.edges.len();
            }
        }
        let ram = world
            .world
            .machines
            .get("stm")
            .unwrap()
            .read_memory(0x2000_0100, 20)
            .unwrap();
        let counts: Vec<u32> = ram
            .chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(counts, vec![10, 10, 3, 3, 1]);
        let nets = world.world.gpio_net_reports();
        let edges: Vec<(&str, u64)> = nets.iter().map(|n| (n.name.as_str(), n.edges)).collect();
        assert_eq!(edges, vec![("irq", 20), ("ready", 14), ("alert", 16)]);
        // The analyzer saw the edges that happened after it armed, on both boards.
        assert!(
            stm_edges > 0 && avr_edges > 0,
            "stm {stm_edges} avr {avr_edges}"
        );
    }

    /// An ESP32 node of a browser world is built by the same node factory the
    /// native world uses: the `env-esp32c6.yaml` lab (an ESP32-C6 in place of
    /// the ATmega328P) counts every edge through the C6's GPIO interrupts and
    /// interrupt matrix exactly as `world_multichip.rs` does natively.
    #[test]
    fn a_wasm_world_runs_an_esp32c6_on_gpio_nets_with_an_stm32() {
        let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/gpio-net-two-boards");
        let environment: EnvironmentManifest = serde_yaml::from_str(
            &std::fs::read_to_string(example.join("env-esp32c6.yaml")).expect("env-esp32c6.yaml"),
        )
        .expect("environment manifest");
        let fw = |name: &str| std::fs::read(example.join("firmware").join(name)).expect("elf");
        let mut world = WasmWorld::from_node_inputs(
            environment,
            vec![
                ResolvedNodeInput {
                    id: "stm".into(),
                    system_yaml: include_str!("../../../examples/stm32g0b1re/system.yaml").into(),
                    chip_yaml: include_str!("../../../configs/chips/stm32g0b1re.yaml").into(),
                    firmware: fw("stm.elf"),
                    blobs: HashMap::new(),
                },
                ResolvedNodeInput {
                    id: "c6".into(),
                    system_yaml: include_str!("../../../configs/systems/esp32c6-devkitc.yaml")
                        .into(),
                    chip_yaml: include_str!("../../../configs/chips/esp32c6.yaml").into(),
                    firmware: fw("esp32c6.elf"),
                    blobs: HashMap::new(),
                },
            ],
        )
        .expect("world");
        assert!(world.has_gpio_nets().unwrap());
        while world.world.round_now_ps().unwrap() < 30_000_000_000 {
            world.step_batch(1).map_err(|_| "step").unwrap();
        }
        let words = |id: &str, at: u32| -> Vec<u32> {
            world
                .world
                .machines
                .get(id)
                .unwrap()
                .read_memory(at, 20)
                .unwrap()
                .chunks(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        };
        assert_eq!(words("stm", 0x2000_0100), vec![10, 10, 3, 3, 1]);
        // [ready rising, alert falling, alert rising, interrupts taken, done]
        let c6 = words("c6", 0x4080_0100);
        assert_eq!(&c6[..3], &[7, 5, 5], "C6 interrupt counts {c6:?}");
        assert!(c6[3] >= 17, "at least one interrupt per edge {c6:?}");
        assert_eq!(c6[4], 1, "C6 finished {c6:?}");
        let nets = world.world.gpio_net_reports();
        let edges: Vec<(&str, u64)> = nets.iter().map(|n| (n.name.as_str(), n.edges)).collect();
        assert_eq!(edges, vec![("irq", 20), ("ready", 14), ("alert", 16)]);
        assert_eq!(
            String::from_utf8_lossy(&world.drain_uart_output("stm").unwrap()),
            "STM irq r=10 f=10 alert r=3 f=3\n"
        );
    }

    /// Two ESP32-C3 nodes of a browser world rally `PING`/`PONG` over a
    /// cross-linked UART1 (`examples/ci-two-c3-link`, the native
    /// `world_esp32c3_pingpong.rs` fixture), and each node's console is its
    /// own: the link's octets never reach the page's per-node UART sink.
    #[test]
    fn a_wasm_world_rallies_two_esp32c3_nodes_over_uart1() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/ci-two-c3-link");
        let environment: EnvironmentManifest = serde_yaml::from_str(
            &std::fs::read_to_string(root.join("env.yaml")).expect("env.yaml"),
        )
        .expect("environment manifest");
        let node = |id: &str| ResolvedNodeInput {
            id: id.into(),
            system_yaml: include_str!("../../../configs/systems/esp32c3-devkit.yaml").into(),
            chip_yaml: include_str!("../../../configs/chips/esp32c3.yaml").into(),
            firmware: std::fs::read(root.join("firmware").join(format!("{id}.elf"))).expect("elf"),
            blobs: HashMap::new(),
        };
        let mut world =
            WasmWorld::from_node_inputs(environment, vec![node("server"), node("client")])
                .expect("world");
        let (mut server, mut client) = (String::new(), String::new());
        let mut rounds = 0u32;
        // Run until the server is done, then as long again for the client to
        // finish printing its last line (its console is slower than the link).
        let mut done_at = None;
        while rounds < 20_000_000 && done_at.is_none_or(|at| rounds < 2 * at) {
            world.step_batch(100_000).map_err(|_| "step").unwrap();
            rounds += 100_000;
            server += &String::from_utf8_lossy(&world.drain_uart_output("server").unwrap());
            client += &String::from_utf8_lossy(&world.drain_uart_output("client").unwrap());
            if done_at.is_none() && server.contains("server done") {
                done_at = Some(rounds);
            }
        }
        assert!(
            server.contains("rally 3") && server.contains("server done"),
            "no three round trips in {rounds} rounds; server console:\n{server}"
        );
        assert!(!server.contains("no PONG"), "{server}");
        assert_eq!(client.matches("client: returned").count(), 3, "{client}");
        for (id, text) in [("server", &server), ("client", &client)] {
            assert!(text.contains(&format!("{id} up")), "{id}: {text}");
            assert!(
                !text.contains("PING\n") && !text.contains("PONG\n"),
                "link octets leaked into {id}'s console:\n{text}"
            );
        }
    }

    /// The browser's C3 flash-image node: the page registers the mask ROM it
    /// fetched, and a world node built from a merged flash image boots it from
    /// the reset vector (the hosted world's path for a hosted C3 build). The
    /// ROM's own banner on the node's UART0, and its hand-off to the 2nd-stage
    /// bootloader it read from this node's image, prove the registered ROM ran;
    /// the sketch's own `loop()` output proves the whole chain reached the app.
    #[test]
    fn a_wasm_world_boots_an_esp32c3_flash_image_through_the_registered_rom() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        WasmWorld::register_esp32c3_rom(
            std::fs::read(root.join("crates/core/roms/esp32c3/esp32c3_rom.bin")).unwrap(),
            std::fs::read(root.join("crates/core/roms/esp32c3/esp32c3_drom.bin")).unwrap(),
        )
        .map_err(|_| "rom")
        .unwrap();
        let environment: EnvironmentManifest = serde_yaml::from_str(
            r#"
schema_version: "1.0"
name: c3-flash-and-stm
nodes:
  - { id: c3, system: s.yaml, firmware: f.bin }
  - { id: stm, system: s.yaml, firmware: f.elf }
interconnects:
  - type: gpio_net
    nodes: [c3, stm]
    config:
      name: line
      pull: down
      members:
        - { node: c3, peripheral: gpio, pin: 4 }
        - { node: stm, peripheral: gpiob, pin: 0 }
"#,
        )
        .expect("environment manifest");
        let flash = std::fs::read(
            root.join("crates/core/tests/fixtures/esp32c3-uart0-console-control-flash.bin"),
        )
        .expect("flash fixture");
        assert!(
            !flash.starts_with(b"\x7fELF"),
            "the fixture is a flash image"
        );
        let mut world = WasmWorld::from_node_inputs(
            environment,
            vec![
                ResolvedNodeInput {
                    id: "c3".into(),
                    system_yaml: include_str!("../../../configs/systems/esp32c3-devkit.yaml")
                        .into(),
                    chip_yaml: include_str!("../../../configs/chips/esp32c3.yaml").into(),
                    firmware: flash,
                    blobs: HashMap::new(),
                },
                ResolvedNodeInput {
                    id: "stm".into(),
                    system_yaml: include_str!("../../../examples/stm32g0b1re/system.yaml").into(),
                    chip_yaml: include_str!("../../../configs/chips/stm32g0b1re.yaml").into(),
                    firmware: std::fs::read(
                        root.join("examples/gpio-net-two-boards/firmware/stm.elf"),
                    )
                    .unwrap(),
                    blobs: HashMap::new(),
                },
            ],
        )
        .expect("world");
        // The mask ROM starts at its reset vector, not at an app entry point.
        assert_eq!(world.get_pc("c3").map_err(|_| "pc").unwrap(), 0x4000_0000);
        let mut c3 = String::new();
        let mut rounds = 0u32;
        while rounds < 2_000_000 && !c3.contains("entry 0x") {
            world.step_batch(1).map_err(|_| "step").unwrap();
            rounds += 1;
            if rounds.is_multiple_of(1_000) {
                c3 += &String::from_utf8_lossy(&world.drain_uart_output("c3").unwrap());
            }
        }
        // The ROM printed its banner, read the 2nd-stage bootloader out of
        // this node's flash image and jumped to it.
        assert!(
            c3.starts_with("ESP-ROM:esp32c3") && c3.contains("load:0x") && c3.contains("entry 0x"),
            "the ROM did not load the bootloader in {rounds} rounds: {c3:?}"
        );
        // ... and the bootloader started this node's Arduino app.
        while rounds < 40_000_000 && !c3.contains("LW_CDC_LOOP 1\r\n") {
            world.step_batch(1).map_err(|_| "step").unwrap();
            rounds += 1;
            if rounds.is_multiple_of(10_000) {
                c3 += &String::from_utf8_lossy(&world.drain_uart_output("c3").unwrap());
            }
        }
        assert!(
            c3.contains("LW_CDC_SETUP") && c3.contains("LW_CDC_LOOP 1\r\n"),
            "the app did not reach its second loop() in {rounds} rounds: {c3:?}"
        );
    }

    /// A world steps its nodes without a co-simulation session, so a node that
    /// declares `cosim_models` must refuse to build rather than run with its
    /// models silently absent.
    #[test]
    fn a_node_with_cosim_models_refuses_to_build() {
        let environment: EnvironmentManifest = serde_yaml::from_str(
            r#"
schema_version: "1.0"
name: browser-world
nodes:
  - id: scope
    system: scope.yaml
    firmware: scope.elf
"#,
        )
        .expect("environment manifest");
        let node = ResolvedNodeInput {
            id: "scope".to_string(),
            system_yaml: r#"
name: "scope"
chip: "stm32f401"
external_devices: []
cosim_models:
  - id: rc
    adapter: analog
    step_ns: 100000
    inputs: { gpio: board.gpio.pa5 }
    outputs: { v_out: board.analog.pa0_volts }
    config:
      netlist_text: |
        Vgpio in 0 dc 0
        R1 in out 10k
        C1 out 0 100n
      probes: { v_out: "v(out)" }
      sources: { gpio: Vgpio }
"#
            .to_string(),
            chip_yaml: include_str!("../../../configs/chips/stm32f401.yaml").to_string(),
            firmware: include_bytes!("../../../tests/fixtures/stm32f401-blinky.elf").to_vec(),
            blobs: HashMap::new(),
        };

        let error = match WasmWorld::from_node_inputs(environment, vec![node]) {
            Ok(_) => panic!("a world whose node declares cosim_models must not build"),
            Err(error) => error,
        };
        assert!(
            error.contains(
                "co-simulation models are not supported in multi-node worlds yet; node 'scope' declares 1"
            ),
            "{error}"
        );
    }

    /// ESP nodes in the browser's world path, built exactly as the page builds
    /// them: chip and system YAML as text, firmware bytes, and the node's mask
    /// ROM passed in its `blobs` under the single-chip names (the wasm bundle
    /// carries no ROM). Each ESP node drives the `irq` net of an STM32G0B1
    /// running the gpio-net-two-boards firmware, which counts the edges.
    mod esp_nodes {
        use super::*;

        fn repo(rel: &str) -> std::path::PathBuf {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(rel)
        }

        fn bytes(rel: &str) -> Vec<u8> {
            std::fs::read(repo(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
        }

        fn text(rel: &str) -> String {
            std::fs::read_to_string(repo(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
        }

        fn rom_blobs(chip: &str) -> HashMap<String, Vec<u8>> {
            HashMap::from([
                (
                    format!("{chip}_irom"),
                    bytes(&format!("crates/core/roms/{chip}/{chip}_rom.bin")),
                ),
                (
                    format!("{chip}_drom"),
                    bytes(&format!("crates/core/roms/{chip}/{chip}_drom.bin")),
                ),
            ])
        }

        fn stm() -> ResolvedNodeInput {
            ResolvedNodeInput {
                id: "stm".into(),
                system_yaml: text("examples/stm32g0b1re/system.yaml"),
                chip_yaml: text("configs/chips/stm32g0b1re.yaml"),
                firmware: bytes("examples/gpio-net-two-boards/firmware/stm.elf"),
                blobs: HashMap::new(),
            }
        }

        fn world(
            esp: ResolvedNodeInput,
            pin: u8,
            profile: Option<&str>,
        ) -> Result<WasmWorld, String> {
            let profile = profile
                .map(|p| format!(", profile: {p}"))
                .unwrap_or_default();
            let environment: EnvironmentManifest = serde_yaml::from_str(&format!(
                r#"
schema_version: "1.0"
name: esp-node
nodes:
  - {{ id: stm, system: s.yaml, firmware: f.elf }}
  - {{ id: esp, system: s.yaml, firmware: f.elf{profile} }}
interconnects:
  - type: gpio_net
    nodes: [esp, stm]
    config:
      name: irq
      pull: down
      members:
        - {{ node: esp, peripheral: gpio, pin: {pin} }}
        - {{ node: stm, peripheral: gpiob, pin: 0 }}
"#
            ))
            .expect("environment manifest");
            WasmWorld::from_node_inputs(environment, vec![stm(), esp])
        }

        struct Run {
            world: WasmWorld,
            console: String,
        }

        impl Run {
            /// Step 1 µs rounds (as the page does) until `done` or `max_ms`.
            fn until(&mut self, max_ms: u64, done: impl Fn(&Run) -> bool) {
                while self.world.world.round_now_ps().unwrap() < max_ms * 1_000_000_000 {
                    self.world.step_batch(50).map_err(|_| "step").unwrap();
                    let out = self.world.drain_uart_output("esp").unwrap();
                    self.console.push_str(&String::from_utf8_lossy(&out));
                    if done(self) {
                        return;
                    }
                }
            }

            fn stm_irq(&self) -> [u32; 2] {
                let b = self.world.read_memory("stm", 0x2000_0100, 8).unwrap();
                [
                    u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                    u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
                ]
            }
        }

        fn run(world: WasmWorld) -> Run {
            Run {
                world,
                console: String::new(),
            }
        }

        #[test]
        fn an_esp32s3_elf_node() {
            let esp = ResolvedNodeInput {
                id: "esp".into(),
                system_yaml: text("configs/systems/esp32s3.yaml"),
                chip_yaml: text("configs/chips/esp32s3.yaml"),
                firmware: bytes("tests/fixtures/tier1/esp32s3.elf"),
                blobs: rom_blobs("esp32s3"),
            };
            let mut r = run(world(esp, 4, None).expect("world"));
            r.until(500, |r| r.console.contains("TIER1 done"));
            assert!(r.console.contains("TIER1 gpio PASS"), "{}", r.console);
            assert!(r.console.contains("TIER1 done"), "{}", r.console);
            assert_eq!(r.stm_irq(), [1, 1], "{}", r.console);
        }

        /// The hosted S3 build path: a merged flash image through the real
        /// mask ROM, which the browser can only supply as blobs.
        #[test]
        fn an_esp32s3_flash_image_node() {
            let esp = ResolvedNodeInput {
                id: "esp".into(),
                system_yaml: text("configs/systems/esp32s3.yaml"),
                chip_yaml: text("configs/chips/esp32s3.yaml"),
                firmware: bytes("tests/fixtures/source-debug/esp32s3-arduino-flash.bin"),
                blobs: rom_blobs("esp32s3"),
            };
            let mut r = run(world(esp, 2, None).expect("world"));
            r.until(3_000, |r| {
                r.stm_irq()[1] >= 1 && r.console.lines().any(|l| l.trim() == "1")
            });
            assert!(r.console.contains("ESP-ROM:esp32s3"), "{}", r.console);
            assert!(!r.console.contains("Detected size"), "{}", r.console);
            let [rise, fall] = r.stm_irq();
            assert!(rise >= 1 && fall >= 1, "{rise}/{fall}: {}", r.console);
        }

        /// A classic-ESP32 Arduino sketch on the node profile the environment
        /// YAML names (`profile: arduino-esp32`).
        #[test]
        fn a_classic_esp32_arduino_node() {
            let esp = ResolvedNodeInput {
                id: "esp".into(),
                system_yaml: text("configs/systems/esp32-wroom-32.yaml"),
                chip_yaml: text("configs/chips/esp32.yaml"),
                firmware: bytes("tests/fixtures/source-debug/esp32-arduino.elf"),
                blobs: HashMap::new(),
            };
            let mut r = run(world(esp, 2, Some("arduino-esp32")).expect("world"));
            r.until(3_000, |r| {
                r.stm_irq()[1] >= 3
                    && r.console.lines().filter(|l| !l.trim().is_empty()).count() >= 3
            });
            let printed: Vec<&str> = r
                .console
                .lines()
                .map(str::trim)
                .filter(|l| l.parse::<u32>().is_ok())
                .take(3)
                .collect();
            assert_eq!(printed, ["1", "3", "9"], "{:?}", r.console);
            let [rise, fall] = r.stm_irq();
            assert!(rise >= 3 && fall >= 3, "{rise}/{fall}");
        }

        fn c3(blobs: HashMap<String, Vec<u8>>) -> ResolvedNodeInput {
            ResolvedNodeInput {
                id: "esp".into(),
                system_yaml: text("configs/systems/esp32c3-devkit.yaml"),
                chip_yaml: text("configs/chips/esp32c3.yaml"),
                firmware: bytes("tests/fixtures/world-esp/esp32c3-esp-hal-pulses.elf"),
                blobs,
            }
        }

        #[test]
        fn an_esp32c3_esp_hal_elf_node() {
            let mut r = run(world(c3(rom_blobs("esp32c3")), 4, None).expect("world"));
            r.until(200, |r| r.console.contains("C3 PULSES DONE"));
            assert!(r.console.contains("C3 ESP-HAL BOOT"), "{}", r.console);
            assert!(r.console.contains("C3 PULSES DONE"), "{}", r.console);
            assert_eq!(r.stm_irq(), [10, 10], "{}", r.console);
        }

        /// The node's blobs are what it boots against: a zeroed "ROM" there
        /// wins over every fallback, and the esp-hal app, whose clock bring-up
        /// calls into the ROM, never gets to print.
        #[test]
        fn a_c3_node_boots_against_its_own_rom_blobs() {
            let zeroed = HashMap::from([
                ("esp32c3_irom".to_string(), vec![0u8; 0x6_0000]),
                ("esp32c3_drom".to_string(), vec![0u8; 0x2_0000]),
            ]);
            let mut w = world(c3(zeroed), 4, None).expect("world");
            let mut failed = false;
            // `step_batch` builds a `JsValue` error, which only a wasm host
            // can; step the world underneath it, as `step_batch` does.
            while w.world.round_now_ps().unwrap() < 20_000_000_000 {
                if w.world.step_rounds(50).values().any(Result::is_err) {
                    failed = true;
                    break;
                }
            }
            let out = String::from_utf8_lossy(&w.drain_uart_output("esp").unwrap()).into_owned();
            assert!(!out.contains("C3 PULSES DONE"), "{out}");
            assert!(failed || w.read_memory("stm", 0x2000_0100, 4).unwrap() == [0; 4]);
        }
    }
}
