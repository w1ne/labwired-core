// SPDX-License-Identifier: MIT
//! One shared MNA circuit on the world's timeline.
//!
//! Every hardware input is sampled at the start of an interval. The circuit
//! holds those values for the interval, solves once, and applies all outputs
//! before any node advances into the next interval. This is explicit sampled
//! coupling, not edge-exact PWM. A node reaches the first instruction boundary
//! at or after the world boundary, as the existing timed world scheduler does.

use super::MachineTrait;
use crate::cosim::{CosimInputKind, CosimRunner, CosimSignalValue, CosimSignals, SignalRouter};
use crate::{SimResult, SimulationError};
use labwired_config::{CosimModelConfig, GpioNetConfig};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorldCosimStatus {
    pub model_id: String,
    pub step_ns: u64,
    pub completed_time_ns: u64,
    pub next_boundary_ns: u64,
    pub model_steps: u64,
    pub coupling: &'static str,
    pub failure: Option<String>,
    pub reset_inputs: BTreeMap<String, crate::reset_input::ResetInputStatus>,
}

pub(super) struct WorldCosim {
    runner: CosimRunner,
    routers: BTreeMap<String, SignalRouter>,
    signals: CosimSignals,
    status: WorldCosimStatus,
    sample_due: bool,
}

type Machines = HashMap<String, Box<dyn MachineTrait>>;

impl WorldCosim {
    pub(super) fn new(
        configs: &[CosimModelConfig],
        machines: &mut Machines,
        clocks: &HashMap<String, u64>,
    ) -> anyhow::Result<Self> {
        let config = &configs[0]; // EnvironmentManifest validates one inline analog model.
        let mut routers = BTreeMap::new();
        let mut ids: Vec<_> = machines.keys().cloned().collect();
        ids.sort();
        for id in ids {
            if clocks.get(&id).is_none_or(|hz| *hz == 0) {
                anyhow::bail!("world analog node '{id}' has no declared CPU clock");
            }
            let router = machines
                .get_mut(&id)
                .unwrap()
                .bind_world_cosim(&id, configs)?;
            routers.insert(id, router);
        }
        let mut runner = CosimRunner::from_configs(configs)?;
        let mut signals = CosimSignals::new();
        for path in runner.input_paths() {
            if path.starts_with("ui.") {
                let initial = runner.input_initial_value(path)?.unwrap_or_else(|| {
                    match runner.input_kind(path) {
                        Some(CosimInputKind::Bool) => CosimSignalValue::Bool(false),
                        _ => CosimSignalValue::F64(0.0),
                    }
                });
                signals.insert(path.to_string(), initial);
            }
        }
        runner.prime_with_signals(&mut signals)?;
        for (probe, route) in &config.outputs {
            if !signals.contains_key(route) {
                anyhow::bail!(
                    "world analog output '{probe}' has no solved probe for route '{route}'"
                );
            }
        }
        let mut circuit = Self {
            runner,
            routers,
            signals,
            status: WorldCosimStatus {
                model_id: config.id.clone(),
                step_ns: config.step_ns,
                completed_time_ns: 0,
                next_boundary_ns: config.step_ns,
                model_steps: 0,
                coupling: "interval-start-zero-order-hold",
                failure: None,
                reset_inputs: BTreeMap::new(),
            },
            sample_due: true,
        };
        circuit.apply(machines, 0)?;
        Ok(circuit)
    }

    pub(super) fn check_gpio_ownership(
        &self,
        machines: &Machines,
        nets: &[GpioNetConfig],
    ) -> anyhow::Result<()> {
        for net in nets {
            for member in &net.members {
                if let (Some(machine), Some(router)) =
                    (machines.get(&member.node), self.routers.get(&member.node))
                {
                    if machine.world_cosim_claims_gpio(router, &member.peripheral, member.pin) {
                        anyhow::bail!(
                            "node '{}' pad {}.{} belongs to both gpio_net and world analog",
                            member.node,
                            member.peripheral,
                            member.pin
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn sample_interval(&mut self, machines: &Machines) -> SimResult<()> {
        if !self.sample_due {
            return Ok(());
        }
        // Missing live hardware values must not retain a previous sample.
        for router in self.routers.values() {
            for path in router.read_paths() {
                self.signals.remove(path);
            }
        }
        for (id, router) in &self.routers {
            machines[id].sample_world_cosim(router, &mut self.signals);
        }
        if let Some(path) = self
            .runner
            .input_paths()
            .into_iter()
            .find(|path| !self.signals.contains_key(*path))
        {
            let error = format!("world analog input '{path}' has no live sample");
            self.status.failure = Some(error.clone());
            return Err(SimulationError::Other(error));
        }
        self.sample_due = false;
        Ok(())
    }

    fn apply(&mut self, machines: &mut Machines, time_ns: u64) -> SimResult<()> {
        for (id, router) in &mut self.routers {
            machines
                .get_mut(id)
                .unwrap()
                .apply_world_cosim(router, &self.signals, time_ns)?;
        }
        Ok(())
    }

    pub(super) fn complete_interval(
        &mut self,
        machines: &mut Machines,
        target_ps: u64,
    ) -> SimResult<()> {
        if target_ps < self.next_boundary_ps() {
            return Ok(());
        }
        if target_ps != self.next_boundary_ps() || self.sample_due {
            let message =
                "world analog boundary was crossed without synchronized samples".to_string();
            self.status.failure = Some(message.clone());
            return Err(SimulationError::Other(message));
        }
        let now = self.status.next_boundary_ns;
        let result = (|| {
            let steps = self
                .runner
                .step_until_with_signals(now, &mut self.signals)?;
            if steps.len() != 1 {
                return Err(SimulationError::Other(
                    "shared analog model did not step exactly once".to_string(),
                ));
            }
            self.apply(machines, now)?;
            self.status.completed_time_ns = now;
            self.status.model_steps += 1;
            self.status.next_boundary_ns = now
                .checked_add(self.status.step_ns)
                .filter(|ns| *ns <= u64::MAX / 1_000)
                .ok_or_else(|| SimulationError::Other("world analog time overflow".to_string()))?;
            self.sample_due = true;
            Ok(())
        })();
        if let Err(error) = &result {
            self.status.failure = Some(error.to_string());
        }
        result
    }

    pub(super) fn set_signal(&mut self, path: &str, value: CosimSignalValue) -> SimResult<()> {
        if !path.starts_with("ui.") || !self.runner.reads_path(path) {
            return Err(SimulationError::Other(format!(
                "world input '{path}' must name a declared ui. route"
            )));
        }
        if matches!(&value, CosimSignalValue::F64(v) if !v.is_finite())
            || matches!(&value, CosimSignalValue::Text(_))
        {
            return Err(SimulationError::Other(format!(
                "world input '{path}' requires a finite scalar"
            )));
        }
        // Changes during an interval belong to the next interval, not its past.
        if !self.sample_due {
            return Err(SimulationError::Other(
                "world inputs can change only at a completed analog boundary".to_string(),
            ));
        }
        let value = if self.runner.input_kind(path) == Some(CosimInputKind::Bool) {
            match value {
                CosimSignalValue::F64(v) => CosimSignalValue::Bool(v != 0.0),
                CosimSignalValue::I64(v) => CosimSignalValue::Bool(v != 0),
                value => value,
            }
        } else {
            value
        };
        self.signals.insert(path.to_string(), value);
        Ok(())
    }

    pub(super) fn next_boundary_ps(&self) -> u64 {
        self.status.next_boundary_ns * 1_000
    }
    pub(super) fn latch_failure(&mut self, message: String) {
        self.status.failure = Some(message);
    }
    pub(super) fn failure(&self) -> Option<&str> {
        self.status.failure.as_deref()
    }
    pub(super) fn status(&self) -> WorldCosimStatus {
        self.status.clone()
    }
    pub(super) fn signals(&self) -> &CosimSignals {
        &self.signals
    }
    pub(super) fn channels(&self) -> Vec<crate::analog::AnalogChannel> {
        self.runner.analog_channels()
    }
    pub(super) fn series(&self) -> crate::analog::AnalogSeries {
        self.runner.analog_trace_registry().series()
    }
    pub(super) fn trace(&self, cursor: u64) -> crate::analog::AnalogTraceBatch {
        self.runner.analog_trace_snapshot(cursor)
    }
}

pub(super) fn fail_nodes(
    ids: &[String],
    results: &mut HashMap<String, SimResult<()>>,
    message: &str,
) {
    for id in ids {
        results.insert(id.clone(), Err(SimulationError::Other(message.to_string())));
    }
}
