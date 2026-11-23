// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Browser binding for portable firmware. The firmware is a separate Wasm
//! module whose `labwired.read/write/advance_ns` imports call this facade.
//! It executes directly in the browser; this object contains no emulated CPU.
//!
//! `WasmFlexTestRun` runs a `labwired test` Flex script with the runner the
//! CLI uses. There the firmware module runs in the `wasmi` interpreter inside
//! this package, so fuel limits and `until_ns` suspension match the CLI.
//! `WasmFlexTestRun::run_with_ngspice_callback` runs the same scripts with
//! `adapter: ngspice` models solved by a host-owned callback.

use labwired_config::{FlexTestLimits, FlexTestScript};
use labwired_core::flex::FlexRuntime;
use labwired_core::flex_script::{
    run_script_with_factory, FlexResultConfig, FlexScriptRun, FlexTestResult,
};
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

    /// The run-long change-point series of every oscilloscope channel, as
    /// `flex-trace.json`'s `analog_series`. It still holds the start of a run
    /// that the scope ring no longer does.
    pub fn analog_series(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.runtime.analog_series()).map_err(js_error)
    }

    pub fn i2c_bus_id(&self, name: &str) -> Result<u32, JsValue> {
        self.runtime.i2c_bus_id(name).map_err(js_error)
    }

    /// One I2C transfer as the bus controller: write `write` to the 7-bit
    /// `address`, read `read_len` bytes after a repeated START, STOP. Returns
    /// the transfer (`ack`, `read`, `duration_ns`) and advances virtual time
    /// by its `duration_ns`. An address no device answers is `ack: false`.
    pub fn i2c_transfer(
        &mut self,
        bus: u32,
        address: u32,
        write: &[u8],
        read_len: u32,
    ) -> Result<JsValue, JsValue> {
        let transfer = self
            .runtime
            .i2c_transfer(bus, address, write, read_len as usize)
            .map_err(js_error)?;
        self.runtime
            .advance_ns(transfer.duration_ns)
            .map_err(js_error)?;
        serde_wasm_bindgen::to_value(&transfer).map_err(js_error)
    }

    /// The retained transfers of every I2C bus, for a protocol view.
    pub fn i2c_trace_snapshot(&self, cursor: f64) -> Result<JsValue, JsValue> {
        let cursor = js_trace_cursor(cursor)?;
        serde_wasm_bindgen::to_value(&self.runtime.i2c_trace_snapshot(cursor)).map_err(js_error)
    }

    pub fn spi_bus_id(&self, name: &str) -> Result<u32, JsValue> {
        self.runtime.spi_bus_id(name).map_err(js_error)
    }

    /// One SPI transfer as the bus controller: select `cs`, exchange `mosi`
    /// byte for byte, release `cs`. Returns the transfer (`selected`, `miso`,
    /// `duration_ns`) and advances virtual time by its `duration_ns`. A chip
    /// select with no device is `selected: false` with no MISO bytes.
    pub fn spi_transfer(&mut self, bus: u32, cs: u32, mosi: &[u8]) -> Result<JsValue, JsValue> {
        let transfer = self.runtime.spi_transfer(bus, cs, mosi).map_err(js_error)?;
        self.runtime
            .advance_ns(transfer.duration_ns)
            .map_err(js_error)?;
        serde_wasm_bindgen::to_value(&transfer).map_err(js_error)
    }

    /// The retained transfers of every SPI bus, for a protocol view.
    pub fn spi_trace_snapshot(&self, cursor: f64) -> Result<JsValue, JsValue> {
        let cursor = js_trace_cursor(cursor)?;
        serde_wasm_bindgen::to_value(&self.runtime.spi_trace_snapshot(cursor)).map_err(js_error)
    }

    /// Append firmware output to the manifest console at the current virtual
    /// time. An error when the manifest declares no console.
    pub fn console_write(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
        self.runtime.console_write(bytes).map_err(js_error)
    }

    /// The console text and the time of each write; `undefined` without a
    /// console.
    pub fn console_capture(&self) -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&self.runtime.console_capture()).map_err(js_error)
    }
}

/// Parse and validate a Flex test script, then run it with the shared runner.
/// The result names the script by `script_name` and its inputs as the script
/// writes them. An `adapter: ngspice` model is a `config_error` here (no
/// solver callback), as in `WasmFlexRuntime::new_from_yaml`.
pub(crate) fn run_test_script(
    script_name: &str,
    script_yaml: &str,
    manifest_yaml: &str,
    firmware: &[u8],
) -> Result<FlexScriptRun, String> {
    run_browser_test_script(script_name, script_yaml, manifest_yaml, firmware, None)
}

/// [`run_test_script`] with `adapter: ngspice` models that exchange each
/// boundary with `callback`.
pub(crate) fn run_test_script_with_ngspice_callback(
    script_name: &str,
    script_yaml: &str,
    manifest_yaml: &str,
    firmware: &[u8],
    callback: &js_sys::Function,
) -> Result<FlexScriptRun, String> {
    run_browser_test_script(
        script_name,
        script_yaml,
        manifest_yaml,
        firmware,
        Some(callback),
    )
}

/// Both test-script entry points run on the browser adapter factory, so no
/// script reaches the native ngspice host or a filesystem path.
fn run_browser_test_script(
    script_name: &str,
    script_yaml: &str,
    manifest_yaml: &str,
    firmware: &[u8],
    callback: Option<&js_sys::Function>,
) -> Result<FlexScriptRun, String> {
    let (script, config) = parse_test_script(script_name, script_yaml)?;
    Ok(run_script_with_factory(
        &script,
        config,
        manifest_yaml,
        firmware,
        |model| crate::ngspice::browser_adapter(model, callback),
    ))
}

fn parse_test_script(
    script_name: &str,
    script_yaml: &str,
) -> Result<(FlexTestScript, FlexResultConfig), String> {
    let script: FlexTestScript = serde_yaml::from_str(script_yaml)
        .map_err(|error| format!("Failed to parse Flex Test Script YAML: {error}"))?;
    script.validate().map_err(|error| error.to_string())?;
    let config = FlexResultConfig {
        script: script_name.into(),
        flex: script.inputs.flex.clone().into(),
        firmware: script.inputs.firmware.clone().into(),
        firmware_sha256: None,
        sequence: script.steps(),
        limits: FlexTestLimits {
            max_steps: script.limits.max_steps,
            max_time_ns: script.limits.max_time_ns,
        },
    };
    Ok((script, config))
}

/// One finished run of a Flex test script: the same `result.json` content
/// `labwired test` writes, and the runtime whose instruments hold the capture.
#[wasm_bindgen]
pub struct WasmFlexTestRun {
    result: FlexTestResult,
    runtime: Option<FlexRuntime>,
}

#[wasm_bindgen]
impl WasmFlexTestRun {
    /// Run a script on the manifest text and firmware bytes it names. The
    /// caller resolves `inputs.flex` and `inputs.firmware`; the result keeps
    /// those names as written. A script that does not parse or validate is an
    /// error here; every later failure is a result with a `stop_reason`.
    ///
    /// The call returns when the script ends. A long script blocks its
    /// thread, so a page calls this from a worker.
    pub fn run(
        script_name: &str,
        script_yaml: &str,
        manifest_yaml: &str,
        firmware: &[u8],
    ) -> Result<WasmFlexTestRun, JsValue> {
        let run =
            run_test_script(script_name, script_yaml, manifest_yaml, firmware).map_err(js_error)?;
        Ok(Self {
            result: run.result,
            runtime: run.runtime,
        })
    }

    /// [`Self::run`] with `adapter: ngspice` models solved by the host-owned
    /// `callback`, as `WasmFlexRuntime::new_from_yaml_with_ngspice_callback`
    /// builds them; its first exchange is the `(0, 0)` priming step. A
    /// callback that throws or returns a malformed or incomplete response at
    /// a boundary ends the run with `stop_reason: model_error`; one that fails
    /// while the models are built is a `config_error`. A `callback` that is
    /// not a function is an error here.
    pub fn run_with_ngspice_callback(
        script_name: &str,
        script_yaml: &str,
        manifest_yaml: &str,
        firmware: &[u8],
        callback: JsValue,
    ) -> Result<WasmFlexTestRun, JsValue> {
        use wasm_bindgen::JsCast;
        let callback = callback
            .dyn_into::<js_sys::Function>()
            .map_err(|_| JsValue::from_str("ngspice callback must be a function"))?;
        let run = run_test_script_with_ngspice_callback(
            script_name,
            script_yaml,
            manifest_yaml,
            firmware,
            &callback,
        )
        .map_err(js_error)?;
        Ok(Self {
            result: run.result,
            runtime: run.runtime,
        })
    }

    /// The result as the JSON text of the CLI's `result.json`. Text keeps
    /// every 64-bit count and time exact.
    pub fn result_json(&self) -> Result<String, JsValue> {
        serde_json::to_string_pretty(&self.result).map_err(js_error)
    }

    /// Hand the run's runtime to the existing instruments. An error when the
    /// run stopped on a configuration error, or the runtime was taken before.
    pub fn take_runtime(&mut self) -> Result<WasmFlexRuntime, JsValue> {
        self.runtime
            .take()
            .map(|runtime| WasmFlexRuntime { runtime })
            .ok_or_else(|| {
                JsValue::from_str(
                    "this run has no runtime: it stopped before the first step, or the runtime was already taken",
                )
            })
    }
}
