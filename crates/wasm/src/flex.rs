// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Browser binding for portable firmware. The firmware is a separate Wasm
//! module whose `labwired.read/write/advance_ns` imports call this facade.
//! It executes directly in the browser; this object contains no emulated CPU.

use labwired_core::flex::FlexRuntime;
use serde::Serialize;
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

#[wasm_bindgen]
pub struct WasmFlexRuntime {
    runtime: FlexRuntime,
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[wasm_bindgen]
impl WasmFlexRuntime {
    pub fn new_from_yaml(yaml: &str) -> Result<WasmFlexRuntime, JsValue> {
        Ok(Self {
            runtime: FlexRuntime::from_yaml(yaml).map_err(js_error)?,
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

    pub fn time_ns(&self) -> Result<f64, JsValue> {
        // FlexRuntime enforces the JavaScript safe-integer time ceiling.
        Ok(self.runtime.time_ns() as f64)
    }

    pub fn ports(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.runtime.manifest().ports).map_err(js_error)
    }

    /// Compact execution context plus the validated, reproducible model
    /// contract for the existing project UI. Empty scope notes remain visible
    /// through `scope_notes_complete` instead of being treated as "no gaps".
    pub fn run_report(&self) -> Result<JsValue, JsValue> {
        let report = self.runtime.run_report();
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
