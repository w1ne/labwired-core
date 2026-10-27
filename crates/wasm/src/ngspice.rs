use labwired_core::cosim::{CosimSignalValue, CosimSignals, CosimStep, CosimStepResult};
use serde_json::{json, Value};

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
