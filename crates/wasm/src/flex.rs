// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Browser binding for portable firmware. The firmware is a separate Wasm
//! module whose `labwired.read/write/advance_ns` imports call this facade.
//! It executes directly in the browser; this object contains no emulated CPU.

use labwired_core::flex::FlexRuntime;
use wasm_bindgen::prelude::*;

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

    pub fn advance_ns(&mut self, delta_ns: u32) -> Result<(), JsValue> {
        self.runtime
            .advance_ns(u64::from(delta_ns))
            .map_err(js_error)
    }

    pub fn time_ns(&self) -> u64 {
        self.runtime.time_ns()
    }

    pub fn ports(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.runtime.manifest().ports).map_err(js_error)
    }

    pub fn digital_trace_snapshot(&mut self, cursor: u64) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.runtime.digital_trace_snapshot(cursor)).map_err(js_error)
    }

    pub fn analog_trace_snapshot(&self, cursor: u64) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.runtime.analog_trace_snapshot(cursor)).map_err(js_error)
    }
}
