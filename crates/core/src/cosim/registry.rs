use crate::analog::{AnalogChannel, AnalogCosimAdapter, AnalogTraceBatch, AnalogTraceRegistry};
use crate::cosim::{
    CosimAdapter, CosimInputKind, CosimSignalValue, CosimSignals, CosimStep, CosimStepResult,
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
        ManifestCosimAdapter::Behavior => super::behavior::build_behavior_adapter(config),
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
            if wants_shm(config)? {
                let mut in_names: Vec<String> = config.inputs.keys().cloned().collect();
                let mut out_names: Vec<String> = config.outputs.keys().cloned().collect();
                in_names.sort();
                out_names.sort();
                let handshake_ms = config
                    .config
                    .get("shm_handshake_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(1000)
                    .clamp(10, 30_000);
                return Ok(Box::new(ExternalProcessCosimAdapter::spawn_shm(
                    &program,
                    &arg_refs,
                    Default::default(),
                    &in_names,
                    &out_names,
                    std::time::Duration::from_millis(handshake_ms),
                )?));
            }
            Ok(Box::new(ExternalProcessCosimAdapter::spawn(
                &program, &arg_refs,
            )?))
        }
        ManifestCosimAdapter::Analog => {
            let adapter =
                AnalogCosimAdapter::from_manifest_config(config, base_dir).map_err(|err| {
                    SimulationError::Other(format!("analog co-sim model '{}': {err}", config.id))
                })?;
            Ok(Box::new(adapter))
        }
        ManifestCosimAdapter::Ngspice => Err(SimulationError::NotImplemented(
            "co-sim adapter ngspice needs a host-provided solver".to_string(),
        )),
        ManifestCosimAdapter::Fmi => Err(SimulationError::NotImplemented(
            "co-sim adapter 'fmi' is declared but FMI import is not wired yet".to_string(),
        )),
    }
}

/// Validate every `adapter: analog` entry by actually parsing its netlist.
///
/// This is where an unsupported element becomes a manifest error rather than a
/// runtime surprise: the netlist parser lives in the core crate, so the config
/// crate cannot see it, and a manifest that names a diode should be rejected
/// before any firmware runs. Returns one issue string per problem, in manifest
/// order, prefixed with `cosim_models[i]` the way
/// [`labwired_config::SystemManifest::validate_cosim_models`] does.
pub fn validate_analog_models(configs: &[CosimModelConfig], base_dir: &Path) -> Vec<String> {
    let mut issues = Vec::new();
    for (index, config) in configs.iter().enumerate() {
        if config.adapter != ManifestCosimAdapter::Analog {
            continue;
        }
        if let Err(err) = AnalogCosimAdapter::from_manifest_config(config, base_dir) {
            issues.push(format!("cosim_models[{index}] (id '{}'): {err}", config.id));
        }
    }
    issues
}

/// `config.transport: shm` (or `protocol: shm`) offers the shared-memory
/// transport; anything else, or nothing, is the JSONL pipe protocol.
fn wants_shm(config: &CosimModelConfig) -> SimResult<bool> {
    let value = config
        .config
        .get("transport")
        .or_else(|| config.config.get("protocol"));
    match value.map(Value::as_str) {
        None => Ok(false),
        Some(Some("jsonl")) => Ok(false),
        Some(Some("shm")) => Ok(true),
        _ => Err(SimulationError::Other(format!(
            "co-sim model '{}': config.transport must be `jsonl` or `shm`",
            config.id
        ))),
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
    /// `(model input name, store path)`, resolved once from the manifest map.
    in_routes: Vec<(String, String)>,
    /// `(model output name, store path)`, sorted by model output name so the
    /// last of two outputs routed to one path wins, as it always has.
    out_routes: Vec<(String, String)>,
    /// Reused per step: the routed inputs handed to the adapter.
    inputs_buf: CosimSignals,
    /// Reused per step: the adapter's outputs.
    outputs_buf: CosimSignals,
}

impl CosimRunnerModel {
    pub fn new(config: CosimModelConfig, adapter: Box<dyn CosimAdapter>) -> Self {
        let next_step_ns = config.step_ns;
        let in_routes = config
            .inputs
            .iter()
            .map(|(name, path)| (name.clone(), path.clone()))
            .collect();
        let mut out_routes: Vec<(String, String)> = config
            .outputs
            .iter()
            .map(|(name, path)| (name.clone(), path.clone()))
            .collect();
        out_routes.sort();
        Self {
            config,
            adapter,
            next_step_ns,
            in_routes,
            out_routes,
            inputs_buf: CosimSignals::new(),
            outputs_buf: CosimSignals::new(),
        }
    }

    /// Fill `inputs_buf` from the store: no allocation once the buffer holds
    /// every routed name.
    fn gather_inputs(&mut self, signals: &CosimSignals) {
        for (model_signal, source_path) in &self.in_routes {
            match signals.get(source_path.as_str()) {
                Some(value) => match self.inputs_buf.get_mut(model_signal.as_str()) {
                    Some(slot) => slot.clone_from(value),
                    None => {
                        self.inputs_buf.insert(model_signal.clone(), value.clone());
                    }
                },
                None => {
                    self.inputs_buf.remove(model_signal.as_str());
                }
            }
        }
    }

    /// Write the adapter's routed outputs into the store. When `record` is
    /// set, also return them keyed by store path.
    fn scatter_outputs(&self, signals: &mut CosimSignals, record: bool) -> Option<CosimSignals> {
        let mut routed = record.then(CosimSignals::new);
        for (model_signal, target_path) in &self.out_routes {
            let Some(value) = self.outputs_buf.get(model_signal.as_str()) else {
                continue;
            };
            match signals.get_mut(target_path.as_str()) {
                Some(slot) => slot.clone_from(value),
                None => {
                    signals.insert(target_path.clone(), value.clone());
                }
            }
            if let Some(routed) = routed.as_mut() {
                routed.insert(target_path.clone(), value.clone());
            }
        }
        routed
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
    analog_trace: AnalogTraceRegistry,
    /// Whether [`Self::step_until_with_signals`] builds the per-step report.
    /// Off, the step makes no allocation; the signal store is still updated.
    record_routed: bool,
    /// Indices of the models due at the boundary being stepped.
    due_scratch: Vec<usize>,
}

impl CosimRunner {
    pub fn new(mut models: Vec<CosimRunnerModel>) -> Self {
        let analog_trace = AnalogTraceRegistry::new();
        let handle = analog_trace.handle();
        // Every channel is `<model id>.<name>`, however many analog models
        // share the ring. A name that depends on how many OTHER models exist
        // cannot be computed by whoever wires the probe: the playground names a
        // scope channel `circuit.v_<net>` from the canvas alone, and adding a
        // second circuit must not rename the first one's channels under it.
        for model in &mut models {
            let prefix = format!("{}.", model.config.id);
            model.adapter.attach_analog_trace(&handle, &prefix);
        }
        Self {
            models,
            analog_trace,
            record_routed: true,
            due_scratch: Vec::new(),
        }
    }

    /// Choose whether [`Self::step_until_with_signals`] returns the per-step
    /// routed outputs (default) or an empty list. The signal store receives the
    /// same values either way; skipping the report removes its allocations from
    /// the hot path for a caller that never reads it.
    pub fn set_record_routed(&mut self, record: bool) {
        self.record_routed = record;
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

    /// How many models this runner steps.
    pub fn model_count(&self) -> usize {
        self.models.len()
    }

    /// Every signal-store path some model's `inputs:` reads, sorted and
    /// deduplicated.
    pub fn input_paths(&self) -> Vec<&str> {
        let paths: std::collections::BTreeSet<&str> = self
            .models
            .iter()
            .flat_map(|model| model.config.inputs.values().map(String::as_str))
            .collect();
        paths.into_iter().collect()
    }

    /// Does any model's `inputs:` read `path`?
    pub fn reads_path(&self, path: &str) -> bool {
        self.models
            .iter()
            .any(|model| model.config.inputs.values().any(|source| source == path))
    }

    /// The value shape the models reading `path` expect: [`CosimInputKind::Bool`]
    /// only when every model that can say calls it a logic level, `Number` when
    /// any calls it a number, `None` when no reader can say.
    pub fn input_kind(&self, path: &str) -> Option<CosimInputKind> {
        let mut kind = None;
        for model in &self.models {
            for (name, source) in &model.config.inputs {
                if source != path {
                    continue;
                }
                match model.adapter.input_kind(name) {
                    Some(CosimInputKind::Number) => return Some(CosimInputKind::Number),
                    Some(CosimInputKind::Bool) => kind = Some(CosimInputKind::Bool),
                    None => {}
                }
            }
        }
        kind
    }

    /// Check numeric routes against the receiving adapter's known units.
    /// Boolean-to-voltage conversion remains an explicit adapter capability.
    pub(crate) fn validate_numeric_input_units(
        &self,
        signals: &CosimSignals,
        units: &std::collections::BTreeMap<String, String>,
    ) -> SimResult<()> {
        for model in &self.models {
            for (name, path) in &model.config.inputs {
                if !matches!(
                    signals.get(path),
                    Some(CosimSignalValue::F64(_) | CosimSignalValue::I64(_))
                ) {
                    continue;
                }
                if let Some(expected) = model.adapter.input_unit(name) {
                    if units.get(path).map(String::as_str) != Some(expected) {
                        return Err(SimulationError::Other(format!(
                            "model '{}' input '{name}' requires unit {expected}; signal '{path}' declares {:?}; no implicit unit conversion",
                            model.config.id, units.get(path)
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// An explicit initial value must agree across every reader of the path.
    /// A reader with no declared initial value is treated as zero/false;
    /// incompatible fanout is rejected rather than silently driving it.
    pub fn input_initial_value(&self, path: &str) -> SimResult<Option<CosimSignalValue>> {
        let mut values = Vec::new();
        let mut explicit = false;
        for model in &self.models {
            for (name, source) in &model.config.inputs {
                if source != path {
                    continue;
                }
                if let Some(value) = model.adapter.input_initial_value(name) {
                    explicit = true;
                    values.push(value);
                } else {
                    values.push(match model.adapter.input_kind(name) {
                        Some(CosimInputKind::Bool) => CosimSignalValue::Bool(false),
                        _ => CosimSignalValue::F64(0.0),
                    });
                }
            }
        }
        if !explicit {
            return Ok(None);
        }
        let first = values.first().expect("explicit default implies a reader");
        if values.iter().any(|value| value != first) {
            return Err(SimulationError::Other(format!(
                "conflicting initial values for routed input `{path}`"
            )));
        }
        Ok(Some(first.clone()))
    }

    /// The shared analog-sample ring, for a machine to publish to instruments.
    pub fn analog_trace_registry(&self) -> AnalogTraceRegistry {
        self.analog_trace.clone()
    }

    /// Channel table of the analog trace (empty when no analog-producing model
    /// is declared).
    pub fn analog_channels(&self) -> Vec<AnalogChannel> {
        self.analog_trace.channels()
    }

    /// Analog samples newer than `cursor`, by sample sequence number.
    pub fn analog_trace_snapshot(&self, cursor: u64) -> AnalogTraceBatch {
        self.analog_trace.snapshot(cursor)
    }

    /// Read the declared initial operating point before any world CPU runs.
    /// This does not advance model time or consume the first scheduled step.
    pub(crate) fn prime_with_signals(&mut self, signals: &mut CosimSignals) -> SimResult<()> {
        // Every model reads the store as it was before any of them answered;
        // outputs are written back only after all have stepped.
        for model in self.models.iter_mut() {
            if model.next_step_ns != model.config.step_ns {
                return Err(SimulationError::Other(
                    "co-sim initialization after time advanced".to_string(),
                ));
            }
            model.gather_inputs(signals);
            let CosimRunnerModel {
                adapter,
                inputs_buf,
                outputs_buf,
                ..
            } = model;
            adapter.step_into(0, 0, inputs_buf, outputs_buf)?;
        }
        for model in &self.models {
            model.scatter_outputs(signals, false);
        }
        Ok(())
    }

    /// Initialize a routed graph in dependency order without advancing time.
    /// This is used by portable firmware, whose inputs are explicit values,
    /// rather than CPU pads. Every model sees its producers' actual initial
    /// outputs. Algebraic feedback requires an explicit initialization model;
    /// it must not silently become a missing-input zero.
    pub(crate) fn prime_routed_with_signals(
        &mut self,
        signals: &mut CosimSignals,
    ) -> SimResult<()> {
        let mut initialized = vec![false; self.models.len()];
        let mut remaining = initialized.len();
        while remaining > 0 {
            let before = remaining;
            for (index, model) in self.models.iter_mut().enumerate() {
                if initialized[index]
                    || model
                        .config
                        .inputs
                        .values()
                        .any(|path| !signals.contains_key(path))
                {
                    continue;
                }
                if model.next_step_ns != model.config.step_ns {
                    return Err(SimulationError::Other(
                        "co-sim initialization after time advanced".to_string(),
                    ));
                }
                model.gather_inputs(signals);
                model
                    .adapter
                    .step_into(0, 0, &model.inputs_buf, &mut model.outputs_buf)?;
                for alias in model.config.outputs.keys() {
                    if !model.outputs_buf.contains_key(alias) {
                        return Err(SimulationError::Other(format!(
                            "model '{}' did not initialize output '{alias}'",
                            model.config.id
                        )));
                    }
                }
                model.scatter_outputs(signals, false);
                initialized[index] = true;
                remaining -= 1;
            }
            if remaining == before {
                let pending: Vec<_> = self
                    .models
                    .iter()
                    .zip(&initialized)
                    .filter_map(|(model, ready)| (!ready).then_some(model.config.id.as_str()))
                    .collect();
                return Err(SimulationError::Other(format!(
                    "cannot initialize routed models [{}]: cyclic or missing initial inputs",
                    pending.join(", ")
                )));
            }
        }
        Ok(())
    }

    pub fn step_until(
        &mut self,
        time_ns: u64,
        inputs: CosimSignals,
    ) -> SimResult<Vec<CosimModelStep>> {
        let mut results = Vec::new();
        while let Some(step_time) = next_due_time(&self.models, time_ns) {
            let mut due = Vec::new();
            due_models_at(&self.models, step_time, &mut due);
            for index in due {
                let model = &mut self.models[index];
                let next = next_model_time(model)?;
                let result = model.adapter.step(CosimStep {
                    time_ns: step_time,
                    dt_ns: model.config.step_ns,
                    inputs: inputs.clone(),
                })?;
                results.push(CosimModelStep {
                    model_id: model.config.id.clone(),
                    result,
                });
                model.next_step_ns = next;
            }
        }
        Ok(results)
    }

    pub fn step_until_with_signals(
        &mut self,
        time_ns: u64,
        signals: &mut CosimSignals,
    ) -> SimResult<Vec<CosimRoutedModelStep>> {
        self.step_until_with_signals_observed(time_ns, signals, |_, _| {})
    }

    /// Observe each committed model boundary using the same nanosecond clock
    /// as the analog trace. All due outputs are scattered before the callback.
    /// A caller can disable routed reports and capture edges without allocating
    /// a second per-step signal history or skipping intermediate transitions.
    pub(crate) fn step_until_with_signals_observed(
        &mut self,
        time_ns: u64,
        signals: &mut CosimSignals,
        mut observe: impl FnMut(u64, &CosimSignals),
    ) -> SimResult<Vec<CosimRoutedModelStep>> {
        let mut results = Vec::new();
        let mut due = std::mem::take(&mut self.due_scratch);
        let outcome = (|| {
            while let Some(step_time) = next_due_time(&self.models, time_ns) {
                due_models_at(&self.models, step_time, &mut due);
                // Outputs are written back only after every due model has
                // stepped, so each reads the store as it stood before any of
                // them answered; no snapshot copy is needed for that.
                for &index in &due {
                    let model = &mut self.models[index];
                    let next = next_model_time(model)?;
                    model.gather_inputs(signals);
                    let dt_ns = model.config.step_ns;
                    let CosimRunnerModel {
                        adapter,
                        inputs_buf,
                        outputs_buf,
                        ..
                    } = model;
                    adapter.step_into(step_time, dt_ns, inputs_buf, outputs_buf)?;
                    model.next_step_ns = next;
                }
                for &index in &due {
                    let model = &self.models[index];
                    if let Some(outputs) = model.scatter_outputs(signals, self.record_routed) {
                        results.push(CosimRoutedModelStep {
                            model_id: model.config.id.clone(),
                            outputs,
                        });
                    }
                }
                observe(step_time, signals);
            }
            Ok(())
        })();
        self.due_scratch = due;
        outcome.map(|()| results)
    }
}

fn next_due_time(models: &[CosimRunnerModel], time_ns: u64) -> Option<u64> {
    models
        .iter()
        .map(|model| model.next_step_ns)
        .filter(|&time| time <= time_ns)
        .min()
}

fn due_models_at(models: &[CosimRunnerModel], time_ns: u64, due: &mut Vec<usize>) {
    due.clear();
    due.extend(
        models
            .iter()
            .enumerate()
            .filter_map(|(index, model)| (model.next_step_ns == time_ns).then_some(index)),
    );
}

fn next_model_time(model: &CosimRunnerModel) -> SimResult<u64> {
    if model.config.step_ns == 0 {
        return Err(SimulationError::Other(format!(
            "co-sim model {} step_ns must be greater than zero",
            model.config.id
        )));
    }
    model
        .next_step_ns
        .checked_add(model.config.step_ns)
        .ok_or_else(|| {
            SimulationError::Other(format!("co-sim model {} time overflow", model.config.id))
        })
}
