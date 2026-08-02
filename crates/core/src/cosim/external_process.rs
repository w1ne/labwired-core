use crate::cosim::{CosimAdapter, CosimSignalValue, CosimSignals, CosimStep, CosimStepResult};
use crate::{SimResult, SimulationError};
use serde_json::{Map, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

pub struct ExternalProcessCosimAdapter {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl ExternalProcessCosimAdapter {
    pub fn spawn(program: &str, args: &[&str]) -> SimResult<Self> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|err| {
                SimulationError::Other(format!("failed to spawn co-sim process '{program}': {err}"))
            })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            SimulationError::Other(format!(
                "failed to open stdin for co-sim process '{program}'"
            ))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            SimulationError::Other(format!(
                "failed to open stdout for co-sim process '{program}'"
            ))
        })?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }
}

impl Drop for ExternalProcessCosimAdapter {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl CosimAdapter for ExternalProcessCosimAdapter {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        let request = step_to_json(step);
        serde_json::to_writer(&mut self.stdin, &request).map_err(|err| {
            SimulationError::Other(format!("failed to encode co-sim step request: {err}"))
        })?;
        self.stdin.write_all(b"\n").map_err(|err| {
            SimulationError::Other(format!("failed to write co-sim step newline: {err}"))
        })?;
        self.stdin.flush().map_err(|err| {
            SimulationError::Other(format!("failed to flush co-sim step request: {err}"))
        })?;

        let mut line = String::new();
        let bytes = self.stdout.read_line(&mut line).map_err(|err| {
            SimulationError::Other(format!("failed to read co-sim step response: {err}"))
        })?;
        if bytes == 0 {
            return Err(SimulationError::Other(
                "co-sim process exited before returning a step response".to_string(),
            ));
        }

        let value: Value = serde_json::from_str(&line).map_err(|err| {
            SimulationError::Other(format!("failed to decode co-sim step response: {err}"))
        })?;
        json_to_step_result(value)
    }
}

fn step_to_json(step: CosimStep) -> Value {
    let mut root = Map::new();
    root.insert("time_ns".to_string(), Value::from(step.time_ns));
    root.insert("dt_ns".to_string(), Value::from(step.dt_ns));
    root.insert("inputs".to_string(), signals_to_json(step.inputs));
    Value::Object(root)
}

fn signals_to_json(signals: CosimSignals) -> Value {
    let mut map = Map::new();
    for (key, value) in signals {
        map.insert(key, signal_to_json(value));
    }
    Value::Object(map)
}

fn signal_to_json(value: CosimSignalValue) -> Value {
    match value {
        CosimSignalValue::Bool(value) => Value::Bool(value),
        CosimSignalValue::I64(value) => Value::from(value),
        CosimSignalValue::F64(value) => Value::from(value),
        CosimSignalValue::Text(value) => Value::String(value),
    }
}

fn json_to_step_result(value: Value) -> SimResult<CosimStepResult> {
    let object = value.as_object().ok_or_else(|| {
        SimulationError::Other("co-sim response must be a JSON object".to_string())
    })?;
    let outputs = object
        .get("outputs")
        .and_then(|value| value.as_object())
        .ok_or_else(|| {
            SimulationError::Other(
                "co-sim response must contain object field 'outputs'".to_string(),
            )
        })?;

    let mut result = CosimSignals::new();
    for (key, value) in outputs {
        result.insert(key.clone(), json_to_signal(value)?);
    }

    Ok(CosimStepResult { outputs: result })
}

fn json_to_signal(value: &Value) -> SimResult<CosimSignalValue> {
    match value {
        Value::Bool(value) => Ok(CosimSignalValue::Bool(*value)),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(CosimSignalValue::I64(value))
            } else if let Some(value) = value.as_f64() {
                Ok(CosimSignalValue::F64(value))
            } else {
                Err(SimulationError::Other(format!(
                    "unsupported co-sim numeric value '{value}'"
                )))
            }
        }
        Value::String(value) => Ok(CosimSignalValue::Text(value.clone())),
        _ => Err(SimulationError::Other(format!(
            "unsupported co-sim signal value '{value}'"
        ))),
    }
}
