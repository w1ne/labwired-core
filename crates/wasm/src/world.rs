//! Browser-safe multi-node World wrapper.

use labwired_config::{ChipDescriptor, EnvironmentManifest, SystemManifest};
use labwired_core::system::node::NodeFirmware;
use labwired_core::world::{ResolvedWorldNode, World};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;

#[derive(Deserialize)]
struct ResolvedNodeInput {
    id: String,
    system_yaml: String,
    chip_yaml: String,
    firmware: Vec<u8>,
}

#[wasm_bindgen]
pub struct WasmWorld {
    world: World,
    uart_sinks: HashMap<String, Arc<Mutex<Vec<u8>>>>,
}

#[wasm_bindgen]
impl WasmWorld {
    #[wasm_bindgen(js_name = new_from_resolved)]
    pub fn new_from_resolved(environment_yaml: &str, nodes: JsValue) -> Result<WasmWorld, JsValue> {
        let manifest: EnvironmentManifest = serde_yaml::from_str(environment_yaml)
            .map_err(|error| JsValue::from_str(&format!("Environment YAML error: {error}")))?;
        let inputs: Vec<ResolvedNodeInput> = serde_wasm_bindgen::from_value(nodes)
            .map_err(|error| JsValue::from_str(&format!("Resolved nodes error: {error}")))?;
        let resolved = inputs
            .into_iter()
            .map(|input| {
                let system: SystemManifest = serde_yaml::from_str(&input.system_yaml)
                    .map_err(|error| format!("node '{}': system YAML: {error}", input.id))?;
                let chip: ChipDescriptor = serde_yaml::from_str(&input.chip_yaml)
                    .map_err(|error| format!("node '{}': chip YAML: {error}", input.id))?;
                Ok(ResolvedWorldNode {
                    id: input.id,
                    system,
                    chip,
                    firmware: NodeFirmware::from_bytes(input.firmware),
                })
            })
            .collect::<Result<Vec<_>, String>>()
            .map_err(|error| JsValue::from_str(&error))?;
        let mut world = World::from_resolved(manifest, resolved)
            .map_err(|error| JsValue::from_str(&format!("World construction error: {error:#}")))?;
        let mut uart_sinks = HashMap::new();
        for (id, machine) in &mut world.machines {
            let sink = Arc::new(Mutex::new(Vec::new()));
            machine
                .attach_uart_tx_sink(sink.clone(), false)
                .map_err(|error| {
                    JsValue::from_str(&format!("node '{id}': UART sink: {error:#}"))
                })?;
            uart_sinks.insert(id.clone(), sink);
        }
        Ok(Self { world, uart_sinks })
    }

    pub fn node_ids(&self) -> JsValue {
        let mut ids = self.world.machines.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        serde_wasm_bindgen::to_value(&ids).unwrap_or(JsValue::NULL)
    }

    pub fn step_batch(&mut self, rounds: u32) -> Result<u32, JsValue> {
        for _ in 0..rounds {
            for (id, result) in self.world.step_all() {
                result
                    .map_err(|error| JsValue::from_str(&format!("node '{id}' step: {error:?}")))?;
            }
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
