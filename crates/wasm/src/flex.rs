// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Browser binding for portable firmware. The firmware is a separate Wasm
//! module whose `labwired.read/write/advance_ns` imports call this facade.
//! It executes directly in the browser; this object contains no emulated CPU.

use labwired_config::{CosimAdapter, CosimModelConfig, FlexPort};
use labwired_core::flex::FlexRuntime;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use wasm_bindgen::prelude::*;

/// The core runtime caps virtual time at JavaScript's largest exact integer;
/// trace cursors share that Number-based contract with the existing browser
/// instruments.
const MAX_SAFE_JS_INTEGER: f64 = 9_007_199_254_740_991.0;

fn js_trace_cursor(cursor: f64) -> Result<u64, JsValue> {
    if !cursor.is_finite() || cursor < 0.0 || cursor.fract() != 0.0 || cursor > MAX_SAFE_JS_INTEGER
    {
        return Err(JsValue::from_str(
            "trace cursor must be a nonnegative safe integer Number",
        ));
    }
    Ok(cursor as u64)
}

fn js_advance_delta(delta_ns: f64) -> Result<u64, JsValue> {
    if !delta_ns.is_finite()
        || delta_ns < 0.0
        || delta_ns.fract() != 0.0
        || delta_ns > MAX_SAFE_JS_INTEGER
    {
        return Err(JsValue::from_str(
            "advance delta must be a nonnegative safe integer Number",
        ));
    }
    Ok(delta_ns as u64)
}

#[derive(Serialize)]
struct BrowserSignalBinding {
    name: String,
    signal: String,
}

#[derive(Serialize)]
struct BrowserModelSummary {
    id: String,
    adapter: CosimAdapter,
    model_type: Option<String>,
    step_ns: u64,
    inputs: Vec<BrowserSignalBinding>,
    outputs: Vec<BrowserSignalBinding>,
    /// The validated data-driven model configuration, so project surfaces
    /// can show the exact behavior behind a run without reinterpreting YAML.
    config: BTreeMap<String, serde_yaml::Value>,
}

fn signal_bindings(bindings: &HashMap<String, String>) -> Vec<BrowserSignalBinding> {
    let mut result: Vec<_> = bindings
        .iter()
        .map(|(name, signal)| BrowserSignalBinding {
            name: name.clone(),
            signal: signal.clone(),
        })
        .collect();
    result.sort_by(|left, right| left.name.cmp(&right.name));
    result
}

fn model_summary(model: &CosimModelConfig) -> BrowserModelSummary {
    BrowserModelSummary {
        id: model.id.clone(),
        adapter: model.adapter,
        model_type: model
            .config
            .get("device_type")
            .and_then(serde_yaml::Value::as_str)
            .map(str::to_owned),
        step_ns: model.step_ns,
        inputs: signal_bindings(&model.inputs),
        outputs: signal_bindings(&model.outputs),
        config: model
            .config
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    }
}

#[derive(Serialize)]
struct BrowserRunReport {
    execution_path: &'static str,
    target_cpu_emulated: bool,
    execution_scope: BrowserExecutionScope,
    time_unit: &'static str,
    scope_notes_complete: bool,
    schema_version: u32,
    model_name: String,
    manifest_sha256: String,
    assumptions: Vec<String>,
    unsupported_checks: Vec<String>,
    ports: Vec<FlexPort>,
    models: Vec<BrowserModelSummary>,
}

/// Explicitly separate a portable behavior run from target-firmware evidence.
/// Flex cannot infer complete-board coverage or parity from a manifest, so
/// those claims stay unestablished in this runtime's report.
#[derive(Serialize)]
struct BrowserExecutionScope {
    firmware: &'static str,
    original_target_firmware_executed: bool,
    detailed_target_execution_selected: bool,
    full_board_coverage: &'static str,
    chip_parity: &'static str,
}

#[wasm_bindgen]
pub struct WasmFlexRuntime {
    runtime: FlexRuntime,
    manifest_sha256: String,
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[wasm_bindgen]
impl WasmFlexRuntime {
    pub fn new_from_yaml(yaml: &str) -> Result<WasmFlexRuntime, JsValue> {
        Ok(Self {
            runtime: FlexRuntime::from_yaml(yaml).map_err(js_error)?,
            manifest_sha256: format!("{:x}", Sha256::digest(yaml.as_bytes())),
        })
    }

    pub fn port_id(&self, name: &str) -> Result<u32, JsValue> {
        self.runtime.port_id(name).map_err(js_error)
    }

    pub fn read(&self, port: u32) -> Result<f64, JsValue> {
        self.runtime.read(port).map_err(js_error)
    }

    pub fn write(&mut self, port: u32, value: f64) -> Result<(), JsValue> {
        self.runtime.write(port, value).map_err(js_error)
    }

    pub fn advance_ns(&mut self, delta_ns: f64) -> Result<(), JsValue> {
        let delta_ns = js_advance_delta(delta_ns)?;
        self.runtime.advance_ns(delta_ns).map_err(js_error)
    }

    pub fn time_ns(&self) -> f64 {
        // FlexRuntime enforces the JavaScript safe-integer time ceiling.
        self.runtime.time_ns() as f64
    }

    pub fn ports(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.runtime.manifest().ports).map_err(js_error)
    }

    /// Compact execution context plus the validated, reproducible model
    /// contract for the existing project UI. Empty scope notes remain visible
    /// through `scope_notes_complete` instead of being treated as "no gaps".
    pub fn run_report(&self) -> Result<JsValue, JsValue> {
        let manifest = self.runtime.manifest();
        let report = BrowserRunReport {
            execution_path: "flex_portable_wasm",
            target_cpu_emulated: false,
            execution_scope: BrowserExecutionScope {
                firmware: "portable_behavior",
                original_target_firmware_executed: false,
                detailed_target_execution_selected: false,
                full_board_coverage: "not_established",
                chip_parity: "not_established",
            },
            time_unit: "ns",
            scope_notes_complete: !manifest.assumptions.is_empty()
                && !manifest.unsupported_checks.is_empty(),
            schema_version: manifest.schema_version,
            model_name: manifest.name.clone(),
            manifest_sha256: self.manifest_sha256.clone(),
            assumptions: manifest.assumptions.clone(),
            unsupported_checks: manifest.unsupported_checks.clone(),
            ports: manifest.ports.clone(),
            models: manifest.cosim_models.iter().map(model_summary).collect(),
        };
        // Config maps are part of the public JS report contract. Serialize
        // them as ordinary objects so the existing project UI can render and
        // persist the exact validated model settings.
        let serializer = serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true);
        report.serialize(&serializer).map_err(js_error)
    }

    pub fn digital_trace_snapshot(&mut self, cursor: f64) -> Result<JsValue, JsValue> {
        let cursor = js_trace_cursor(cursor)?;
        serde_wasm_bindgen::to_value(&self.runtime.digital_trace_snapshot(cursor)).map_err(js_error)
    }

    pub fn analog_trace_snapshot(&self, cursor: f64) -> Result<JsValue, JsValue> {
        let cursor = js_trace_cursor(cursor)?;
        serde_wasm_bindgen::to_value(&self.runtime.analog_trace_snapshot(cursor)).map_err(js_error)
    }
}
