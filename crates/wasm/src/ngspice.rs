use js_sys::Function;
use labwired_core::cosim::{
    CosimAdapter, CosimSignalValue, CosimSignals, CosimStep, CosimStepResult,
};
use labwired_core::{SimResult, SimulationError};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use wasm_bindgen::JsValue;

pub(crate) fn request_json(model_id: &str, step: &CosimStep) -> Result<String, String> {
    let mut inputs = serde_json::Map::new();
    for (name, value) in &step.inputs {
        let number = match value {
            CosimSignalValue::I64(value) => *value as f64,
            CosimSignalValue::F64(value) if value.is_finite() => *value,
            CosimSignalValue::Bool(value) => {
                inputs.insert(name.clone(), json!(value));
                continue;
            }
            CosimSignalValue::Text(_) => {
                return Err(format!(
                    "co-sim model '{}' ngspice callback input '{}' must be numeric or boolean",
                    model_id, name
                ));
            }
            CosimSignalValue::F64(_) => {
                return Err(format!(
                    "co-sim model '{}' ngspice callback input '{}' must be finite",
                    model_id, name
                ));
            }
        };
        inputs.insert(name.clone(), json!(number));
    }
    serde_json::to_string(&json!({
        "model_id": model_id,
        "time_ns": step.time_ns,
        "dt_ns": step.dt_ns,
        "inputs": inputs,
    }))
    .map_err(|error| {
        format!(
            "co-sim model '{}' ngspice callback request could not be serialized: {error}",
            model_id
        )
    })
}

pub(crate) fn parse_response(model_id: &str, response: &str) -> Result<CosimStepResult, String> {
    let value: Value = serde_json::from_str(response).map_err(|error| {
        format!(
            "co-sim model '{}' ngspice callback returned invalid JSON: {error}",
            model_id
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        format!(
            "co-sim model '{}' ngspice callback response must be a JSON object",
            model_id
        )
    })?;
    let mut outputs = CosimSignals::new();
    for (name, value) in object {
        if let Some(level) = value.as_bool() {
            outputs.insert(name.clone(), CosimSignalValue::Bool(level));
            continue;
        }
        let number = value
            .as_f64()
            .filter(|number| number.is_finite())
            .ok_or_else(|| {
                format!(
                    "co-sim model '{}' ngspice callback output '{}' must be a finite number or boolean",
                    model_id, name
                )
            })?;
        outputs.insert(name.clone(), CosimSignalValue::F64(number));
    }
    Ok(CosimStepResult { outputs })
}

thread_local! {
    static NGSPICE_CALLBACKS: RefCell<HashMap<u64, Function>> = RefCell::new(HashMap::new());
}

static NEXT_NGSPICE_CALLBACK_ID: AtomicU64 = AtomicU64::new(1);

struct NgspiceCallbackAdapter {
    callback_id: u64,
    model_id: String,
    expected_outputs: Vec<String>,
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
        Ok(result)
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
    model_id: &str,
    expected_outputs: Vec<String>,
    callback: &Function,
) -> Box<dyn CosimAdapter> {
    Box::new(NgspiceCallbackAdapter {
        callback_id: register_ngspice_callback(callback.clone()),
        model_id: model_id.to_string(),
        expected_outputs,
    })
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

pub(crate) fn require_outputs(
    model_id: &str,
    result: &CosimStepResult,
    expected: &[String],
) -> Result<(), String> {
    for name in expected {
        if !result.outputs.contains_key(name) {
            return Err(format!(
                "co-sim model {model_id} ngspice callback omitted output {name}"
            ));
        }
    }
    Ok(())
}
