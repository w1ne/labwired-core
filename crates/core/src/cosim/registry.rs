use crate::cosim::{
    CosimAdapter, CosimSignalValue, CosimSignals, CosimStep, CosimStepResult,
    ExternalProcessCosimAdapter, StaticCosimAdapter,
};
use crate::{SimResult, SimulationError};
use labwired_config::{CosimAdapter as ManifestCosimAdapter, CosimModelConfig};
use serde_yaml::Value;
use std::path::{Path, PathBuf};

pub fn build_cosim_adapter(config: &CosimModelConfig) -> SimResult<Box<dyn CosimAdapter>> {
    build_cosim_adapter_with_base(config, Path::new("."))
}

pub fn build_cosim_adapter_with_base(
    config: &CosimModelConfig,
    base_dir: &Path,
) -> SimResult<Box<dyn CosimAdapter>> {
    match config.adapter {
        ManifestCosimAdapter::Mock => Ok(Box::new(StaticCosimAdapter::new(mock_outputs(config)?))),
        ManifestCosimAdapter::ExternalProcess => {
            let model = config.model.as_deref().ok_or_else(|| {
                SimulationError::Other(format!(
                    "co-sim model '{}' uses external_process but has no model path",
                    config.id
                ))
            })?;
            let model = resolve_model_path(model, base_dir);
            let model = model.to_string_lossy();
            let (program, args) = external_process_command(&model);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            Ok(Box::new(ExternalProcessCosimAdapter::spawn(
                &program, &arg_refs,
            )?))
        }
        ManifestCosimAdapter::Fmi => Err(SimulationError::NotImplemented(
            "co-sim adapter 'fmi' is declared but FMI import is not wired yet".to_string(),
        )),
    }
}

fn resolve_model_path(model: &str, base_dir: &Path) -> PathBuf {
    let path = Path::new(model);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn external_process_command(model: &str) -> (String, Vec<String>) {
    if Path::new(model).extension().and_then(|ext| ext.to_str()) == Some("py") {
        ("python3".to_string(), vec![model.to_string()])
    } else {
        (model.to_string(), Vec::new())
    }
}

fn mock_outputs(config: &CosimModelConfig) -> SimResult<CosimSignals> {
    let outputs = config.config.get("outputs").ok_or_else(|| {
        SimulationError::Other(format!(
            "mock co-sim model '{}' requires config.outputs",
            config.id
        ))
    })?;
    let map = outputs.as_mapping().ok_or_else(|| {
        SimulationError::Other(format!(
            "mock co-sim model '{}'.config.outputs must be a mapping",
            config.id
        ))
    })?;

    let mut signals = CosimSignals::new();
    for (key, value) in map {
        let key = key.as_str().ok_or_else(|| {
            SimulationError::Other(format!(
                "mock co-sim model '{}'.config.outputs keys must be strings",
                config.id
            ))
        })?;
        signals.insert(key.to_string(), yaml_to_signal(value)?);
    }
    Ok(signals)
}

fn yaml_to_signal(value: &Value) -> SimResult<CosimSignalValue> {
    match value {
        Value::Bool(value) => Ok(CosimSignalValue::Bool(*value)),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(CosimSignalValue::I64(value))
            } else if let Some(value) = value.as_f64() {
                Ok(CosimSignalValue::F64(value))
            } else {
                Err(SimulationError::Other(format!(
                    "unsupported co-sim numeric value '{value:?}'"
                )))
            }
        }
        Value::String(value) => Ok(CosimSignalValue::Text(value.clone())),
        _ => Err(SimulationError::Other(format!(
            "unsupported co-sim signal value '{value:?}'"
        ))),
    }
}

pub struct CosimRunnerModel {
    config: CosimModelConfig,
    adapter: Box<dyn CosimAdapter>,
    next_step_ns: u64,
}

impl CosimRunnerModel {
    pub fn new(config: CosimModelConfig, adapter: Box<dyn CosimAdapter>) -> Self {
        let next_step_ns = config.step_ns;
        Self {
            config,
            adapter,
            next_step_ns,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CosimModelStep {
    pub model_id: String,
    pub result: CosimStepResult,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CosimRoutedModelStep {
    pub model_id: String,
    pub outputs: CosimSignals,
}

#[derive(Default)]
pub struct CosimRunner {
    models: Vec<CosimRunnerModel>,
}

impl CosimRunner {
    pub fn new(models: Vec<CosimRunnerModel>) -> Self {
        Self { models }
    }

    pub fn from_configs(configs: &[CosimModelConfig]) -> SimResult<Self> {
        Self::from_configs_with_base(configs, Path::new("."))
    }

    pub fn from_configs_with_base(
        configs: &[CosimModelConfig],
        base_dir: &Path,
    ) -> SimResult<Self> {
        let mut models = Vec::new();
        for config in configs {
            models.push(CosimRunnerModel::new(
                config.clone(),
                build_cosim_adapter_with_base(config, base_dir)?,
            ));
        }
        Ok(Self::new(models))
    }

    pub fn step_until(
        &mut self,
        time_ns: u64,
        inputs: CosimSignals,
    ) -> SimResult<Vec<CosimModelStep>> {
        let mut results = Vec::new();
        for model in &mut self.models {
            while time_ns >= model.next_step_ns {
                let step_time = model.next_step_ns;
                let result = model.adapter.step(CosimStep {
                    time_ns: step_time,
                    dt_ns: model.config.step_ns,
                    inputs: inputs.clone(),
                })?;
                results.push(CosimModelStep {
                    model_id: model.config.id.clone(),
                    result,
                });
                model.next_step_ns = model.next_step_ns.saturating_add(model.config.step_ns);
            }
        }
        Ok(results)
    }

    pub fn step_until_with_signals(
        &mut self,
        time_ns: u64,
        signals: &mut CosimSignals,
    ) -> SimResult<Vec<CosimRoutedModelStep>> {
        let mut results = Vec::new();
        for model in &mut self.models {
            while time_ns >= model.next_step_ns {
                let step_time = model.next_step_ns;
                let inputs = routed_inputs(&model.config, signals);
                let result = model.adapter.step(CosimStep {
                    time_ns: step_time,
                    dt_ns: model.config.step_ns,
                    inputs,
                })?;
                let outputs = route_outputs(&model.config, result.outputs, signals);
                results.push(CosimRoutedModelStep {
                    model_id: model.config.id.clone(),
                    outputs,
                });
                model.next_step_ns = model.next_step_ns.saturating_add(model.config.step_ns);
            }
        }
        Ok(results)
    }
}

fn routed_inputs(config: &CosimModelConfig, signals: &CosimSignals) -> CosimSignals {
    let mut inputs = CosimSignals::new();
    for (model_signal, source_path) in &config.inputs {
        if let Some(value) = signals.get(source_path) {
            inputs.insert(model_signal.clone(), value.clone());
        }
    }
    inputs
}

fn route_outputs(
    config: &CosimModelConfig,
    outputs: CosimSignals,
    signals: &mut CosimSignals,
) -> CosimSignals {
    let mut routed = CosimSignals::new();
    for (model_signal, value) in outputs {
        if let Some(target_path) = config.outputs.get(&model_signal) {
            signals.insert(target_path.clone(), value.clone());
            routed.insert(target_path.clone(), value);
        }
    }
    routed
}
