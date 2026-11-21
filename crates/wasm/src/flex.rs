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
    /// An `adapter: ngspice` model is refused here (no solver callback); use
    /// [`Self::new_from_yaml_with_ngspice_callback`].
    pub fn new_from_yaml(yaml: &str) -> Result<WasmFlexRuntime, JsValue> {
        Self::build(yaml, None).map_err(js_error)
    }

    /// Construct a Flex runtime whose `adapter: ngspice` models call the
    /// host-owned solver callback, as
    /// `WasmSimulator::new_from_config_with_ngspice_callback` does. The first
    /// exchange is the `(time_ns = 0, dt_ns = 0)` priming step.
    pub fn new_from_yaml_with_ngspice_callback(
        yaml: &str,
        callback: JsValue,
    ) -> Result<WasmFlexRuntime, JsValue> {
        use wasm_bindgen::JsCast;
        let callback = callback
            .dyn_into::<js_sys::Function>()
            .map_err(|_| JsValue::from_str("ngspice callback must be a function"))?;
        Self::build(yaml, Some(&callback)).map_err(js_error)
    }
}

impl WasmFlexRuntime {
    #[cfg(test)]
    pub(crate) fn build_for_test(yaml: &str) -> labwired_core::SimResult<WasmFlexRuntime> {
        Self::build(yaml, None)
    }

    fn build(
        yaml: &str,
        callback: Option<&js_sys::Function>,
    ) -> labwired_core::SimResult<WasmFlexRuntime> {
        Ok(Self {
            runtime: FlexRuntime::from_yaml_with_factory(yaml, |config| {
                crate::ngspice::browser_adapter(config, callback)
            })?,
        })
    }
}

#[wasm_bindgen]
impl WasmFlexRuntime {
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

    pub fn time_ns(&self) -> Result<u64, JsValue> {
        Ok(self.runtime.time_ns())
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
