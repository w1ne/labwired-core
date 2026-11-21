//! Browser host of `adapter: ngspice`: each boundary goes to a JavaScript
//! callback that owns the ngspice WASM instance. The exchange frames, output
//! checks and scope trace are shared with the native host in
//! `labwired_core::cosim::ngspice`.

use js_sys::Function;
pub(crate) use labwired_core::cosim::ngspice::{
    expected_outputs, parse_response, request_json, require_outputs, NgspiceTrace,
};
use labwired_core::cosim::{CosimAdapter, CosimStep, CosimStepResult};
use labwired_core::{SimResult, SimulationError};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use wasm_bindgen::JsValue;

thread_local! {
    static NGSPICE_CALLBACKS: RefCell<HashMap<u64, Function>> = RefCell::new(HashMap::new());
}

static NEXT_NGSPICE_CALLBACK_ID: AtomicU64 = AtomicU64::new(1);

struct NgspiceCallbackAdapter {
    callback_id: u64,
    model_id: String,
    expected_outputs: Vec<String>,
    trace: NgspiceTrace,
}

impl CosimAdapter for NgspiceCallbackAdapter {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        let request = request_json(&self.model_id, &step).map_err(SimulationError::Other)?;
        let callback =
            NGSPICE_CALLBACKS.with(|callbacks| callbacks.borrow().get(&self.callback_id).cloned());
        let callback = callback.ok_or_else(|| {
            SimulationError::Other(format!(
                "co-sim model '{}' ngspice callback is no longer registered",
                self.model_id
            ))
        })?;
        let response = callback
            .call1(&JsValue::NULL, &JsValue::from_str(&request))
            .map_err(|error| {
                SimulationError::Other(format!(
                    "co-sim model '{}' ngspice callback threw: {}",
                    self.model_id,
                    js_error_message(&error)
                ))
            })?;
        let response = response.as_string().ok_or_else(|| {
            SimulationError::Other(format!(
                "co-sim model '{}' ngspice callback must return a JSON string",
                self.model_id
            ))
        })?;
        let result = parse_response(&self.model_id, &response).map_err(SimulationError::Other)?;
        require_outputs(&self.model_id, &result, &self.expected_outputs)
            .map_err(SimulationError::Other)?;
        self.trace
            .record(step.time_ns, &result.outputs)
            .map_err(SimulationError::Other)?;
        Ok(result)
    }

    fn attach_analog_trace(
        &mut self,
        handle: &labwired_core::cosim::AnalogTraceHandle,
        prefix: &str,
    ) {
        self.trace.attach(handle, prefix);
    }
}

fn register_ngspice_callback(callback: Function) -> u64 {
    let id = NEXT_NGSPICE_CALLBACK_ID.fetch_add(1, Ordering::Relaxed);
    NGSPICE_CALLBACKS.with(|callbacks| callbacks.borrow_mut().insert(id, callback));
    id
}

fn unregister_ngspice_callback(id: u64) {
    NGSPICE_CALLBACKS.with(|callbacks| callbacks.borrow_mut().remove(&id));
}

impl Drop for NgspiceCallbackAdapter {
    fn drop(&mut self) {
        unregister_ngspice_callback(self.callback_id);
    }
}

pub(crate) fn adapter(
    config: &labwired_config::CosimModelConfig,
    callback: &Function,
) -> Box<dyn CosimAdapter> {
    Box::new(NgspiceCallbackAdapter {
        callback_id: register_ngspice_callback(callback.clone()),
        model_id: config.id.clone(),
        expected_outputs: expected_outputs(config),
        trace: NgspiceTrace::new(config),
    })
}

/// The browser adapter factory shared by `WasmSimulator` and
/// `WasmFlexRuntime`: `adapter: ngspice` goes to the host callback, every
/// other adapter to the generic core factory. Callers check the models with
/// [`crate::cosim::check_browser_models`] first, so no model reads a path.
pub(crate) fn browser_adapter(
    config: &labwired_config::CosimModelConfig,
    callback: Option<&Function>,
) -> SimResult<Box<dyn CosimAdapter>> {
    if config.adapter == labwired_config::CosimAdapter::Ngspice {
        let callback = callback.ok_or_else(|| {
            SimulationError::Other(format!(
                "co-sim model {} needs an ngspice callback",
                config.id
            ))
        })?;
        Ok(adapter(config, callback))
    } else {
        labwired_core::cosim::build_cosim_adapter_with_base(config, std::path::Path::new("."))
    }
}

fn js_error_message(error: &JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(error, &JsValue::from_str("message"))
                .ok()
                .and_then(|message| message.as_string())
        })
        .unwrap_or_else(|| format!("{error:?}"))
}
