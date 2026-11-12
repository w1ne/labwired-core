//! Boundary-controlled internal regulators. Register encodings belong to the
//! peripheral; this shared handle owns ordered commands and solved observations.
pub use labwired_config::RegulatorConfiguration;
use labwired_config::{InternalRegulatorConfig, MAX_REGULATOR_REVISION};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

const MAX_PENDING_COMMANDS: usize = 32;

pub fn require_model_presence(declared_stages: usize, has_models: bool) -> Result<(), String> {
    if declared_stages != 0 && !has_models {
        return Err("declared internal regulators require complete analog circuit ownership; no co-simulation model was supplied".into());
    }
    Ok(())
}

pub const REGULATOR_INPUT_FIELDS: [&str; 5] = [
    "enabled",
    "target_voltage_v",
    "current_limit_a",
    "rated_load_a",
    "command_revision",
];
pub const REGULATOR_OUTPUT_FIELDS: [&str; 7] = [
    "input_voltage_v",
    "feedback_voltage_v",
    "input_current_a",
    "output_current_a",
    "output_enabled",
    "qualified",
    "applied_revision",
];

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct RegulatorCommand {
    #[serde(flatten)]
    pub configuration: RegulatorConfiguration,
    pub command_revision: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct RegulatorObservation {
    pub time_ns: u64,
    pub input_voltage_v: f64,
    pub feedback_voltage_v: f64,
    pub input_current_a: f64,
    pub output_current_a: f64,
    pub output_enabled: bool,
    pub qualified: bool,
    pub applied_revision: u64,
}
impl RegulatorObservation {
    pub fn validate(&self) -> Result<(), String> {
        if [
            self.input_voltage_v,
            self.feedback_voltage_v,
            self.input_current_a,
            self.output_current_a,
        ]
        .iter()
        .any(|v| !v.is_finite())
            || self.applied_revision > MAX_REGULATOR_REVISION
        {
            return Err(
                "regulator observation needs finite voltages/currents and a safe integer revision"
                    .into(),
            );
        }
        if self.output_enabled && !self.qualified {
            return Err("an unqualified regulator cannot report an enabled output".into());
        }
        Ok(())
    }
}

#[derive(Debug)]
struct ControlState {
    initial: RegulatorConfiguration,
    accepted: RegulatorCommand,
    exported: RegulatorCommand,
    sampled_source: bool,
    pending: VecDeque<RegulatorCommand>,
    observation: Option<RegulatorObservation>,
}
#[derive(Debug, Clone)]
pub struct RegulatorControl {
    inner: Arc<Mutex<ControlState>>,
}
impl RegulatorControl {
    fn new(initial: RegulatorConfiguration) -> Self {
        let command = RegulatorCommand {
            configuration: initial,
            command_revision: 0,
        };
        Self {
            inner: Arc::new(Mutex::new(ControlState {
                initial,
                accepted: command,
                exported: command,
                sampled_source: false,
                pending: VecDeque::new(),
                observation: None,
            })),
        }
    }
    pub fn configuration(&self) -> RegulatorConfiguration {
        self.inner
            .lock()
            .expect("regulator control poisoned")
            .accepted
            .configuration
    }
    pub fn pending_update(&self) -> bool {
        !self
            .inner
            .lock()
            .expect("regulator control poisoned")
            .pending
            .is_empty()
    }
    pub fn observation(&self) -> Option<RegulatorObservation> {
        self.inner
            .lock()
            .expect("regulator control poisoned")
            .observation
    }
    pub fn request(&self, configuration: RegulatorConfiguration) -> Result<(), String> {
        configuration.validate()?;
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "regulator control poisoned")?;
        if configuration == state.accepted.configuration {
            return Ok(());
        }
        if !state.sampled_source {
            return Err(
                "state-changing regulator control requires an attached sampled circuit".into(),
            );
        }
        if state.pending.len() >= MAX_PENDING_COMMANDS {
            return Err("regulator command queue is full; no request was accepted".into());
        }
        let command_revision = next_revision(state.accepted.command_revision)?;
        let command = RegulatorCommand {
            configuration,
            command_revision,
        };
        state.pending.push_back(command);
        state.accepted = command;
        Ok(())
    }
    /// Chip reset requests the initial stage configuration. The last physical
    /// observation and any in-flight exported revision remain observable.
    pub fn reset_to_defaults(&self) -> Result<(), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "regulator control poisoned")?;
        if !state.sampled_source {
            state.accepted.configuration = state.initial;
            state.pending.clear();
            return Ok(());
        }
        let command = RegulatorCommand {
            configuration: state.initial,
            command_revision: next_revision(state.accepted.command_revision)?,
        };
        state.pending.clear();
        state.pending.push_back(command);
        state.accepted = command;
        Ok(())
    }
    /// One ordered frame is held by the circuit over its next interval.
    pub fn command(&self) -> RegulatorCommand {
        let mut state = self.inner.lock().expect("regulator control poisoned");
        let frame = state.pending.front().copied().unwrap_or(state.accepted);
        state.exported = frame;
        frame
    }
    fn attach_sampled_source(&self) {
        self.inner
            .lock()
            .expect("regulator control poisoned")
            .sampled_source = true;
    }
    fn validate_observation(&self, observation: RegulatorObservation) -> Result<(), String> {
        observation.validate()?;
        let state = self
            .inner
            .lock()
            .map_err(|_| "regulator control poisoned")?;
        validate_observation_state(&state, observation)
    }
    fn apply_observation(&self, observation: RegulatorObservation) -> Result<(), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "regulator control poisoned")?;
        validate_observation_state(&state, observation)?;
        if state
            .pending
            .front()
            .is_some_and(|frame| frame.command_revision == observation.applied_revision)
        {
            state.pending.pop_front();
        }
        state.observation = Some(observation);
        Ok(())
    }
}
fn next_revision(previous: u64) -> Result<u64, String> {
    previous
        .checked_add(1)
        .filter(|v| *v <= MAX_REGULATOR_REVISION)
        .ok_or_else(|| "regulator command revision exhausted".into())
}
fn validate_observation_state(
    state: &ControlState,
    observation: RegulatorObservation,
) -> Result<(), String> {
    observation.validate()?;
    if !state.sampled_source {
        return Err("regulator observation has no attached sampled circuit".into());
    }
    if let Some(last) = state.observation {
        if observation.time_ns < last.time_ns
            || observation.applied_revision < last.applied_revision
        {
            return Err("regulator observation reverses time or revision".into());
        }
        if observation.time_ns == last.time_ns && observation != last {
            return Err("regulator observation changes an already completed boundary".into());
        }
    }
    let previous = state.observation.map_or(0, |last| last.applied_revision);
    if observation.applied_revision != state.exported.command_revision
        && observation.applied_revision != previous
    {
        return Err("regulator observation acknowledges an unexported command revision".into());
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct RegulatorStage {
    pub descriptor: InternalRegulatorConfig,
    pub control: RegulatorControl,
}
#[derive(Debug, Default)]
pub struct RegulatorRegistry {
    stages: BTreeMap<String, RegulatorStage>,
    sampled_complete: std::sync::atomic::AtomicBool,
}
impl RegulatorRegistry {
    pub fn new(configs: &[InternalRegulatorConfig]) -> Self {
        Self {
            sampled_complete: std::sync::atomic::AtomicBool::new(false),
            stages: configs
                .iter()
                .map(|config| {
                    (
                        config.id.clone(),
                        RegulatorStage {
                            descriptor: config.clone(),
                            control: RegulatorControl::new(config.initial),
                        },
                    )
                })
                .collect(),
        }
    }
    /// Shared setup admission for CLI, Wasm and World; empty legacy chips pass.
    pub fn require_models(&self, has_models: bool) -> Result<(), String> {
        require_model_presence(self.stages.len(), has_models)
    }

    /// Defense at the authoritative CPU advance seam, after construction.
    pub fn require_sampled_source(&self) -> Result<(), String> {
        if !self.is_empty()
            && !self
                .sampled_complete
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("declared internal regulators have no complete attached sampled circuit; CPU execution refused".into());
        }
        Ok(())
    }
    pub fn get(&self, id: &str) -> Option<&RegulatorStage> {
        self.stages.get(id)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &RegulatorStage)> {
        self.stages.iter()
    }
    pub fn index(&self, id: &str) -> Option<usize> {
        self.stages.keys().position(|key| key == id)
    }
    pub fn at(&self, index: usize) -> Option<&RegulatorStage> {
        self.stages.values().nth(index)
    }
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
    pub fn attach_sampled_sources(&self, ids: &[String]) -> Result<(), String> {
        if ids.len() != self.stages.len() || self.stages.keys().any(|id| !ids.contains(id)) {
            return Err(
                "every declared internal regulator requires one complete sampled circuit binding"
                    .into(),
            );
        }
        for id in ids {
            self.stages[id].control.attach_sampled_source();
        }
        self.sampled_complete
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    pub fn validate_observations(
        &self,
        batch: &[(String, RegulatorObservation)],
    ) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        for (id, observation) in batch {
            if !ids.insert(id) {
                return Err(format!("duplicate regulator observation '{id}'"));
            }
            let stage = self
                .stages
                .get(id)
                .ok_or_else(|| format!("unknown internal regulator '{id}'"))?;
            stage.control.validate_observation(*observation)?;
        }
        Ok(())
    }
    /// Validate the complete batch before publishing any observation.
    pub fn apply_observations(
        &self,
        batch: &[(String, RegulatorObservation)],
    ) -> Result<(), String> {
        self.validate_observations(batch)?;
        for (id, observation) in batch {
            self.stages[id].control.apply_observation(*observation)?;
        }
        Ok(())
    }
}
