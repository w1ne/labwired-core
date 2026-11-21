// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CPU-free, sampled digital blocks over the existing declarative rule machine.
//!
//! This adapter reuses DeviceBehavior, RuleMachine and PinOnlyCtx. It owns no
//! instruction interpreter, register file, physical pin bus or second clock.
//! CosimRunner supplies sampling boundaries. At each boundary every input is
//! sampled before changed-input events fire in lexical alias order. The first
//! sample fires every declared input once, including a zero/false input.
//!
//! The supported behavior subset is primitive: gpio_device, states, vars,
//! outputs, pin_defaults and rules. Rules accept input events and goto, var and
//! pin actions. The existing integer expression semantics apply: scalar inputs
//! use PinOnlyCtx's rounding with scale 1.0. Timers, protocol framing, registers
//! and schedules are refused rather than accepted without an implementation.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, ensure, Context, Result};
use labwired_config::expr::Expr;
use labwired_config::{
    compile_rules, validate_rule_names, CompiledAction, CosimModelConfig, DeviceBehavior, Event,
    InputPrecision, RuleNames,
};
use serde_yaml::{Mapping, Value};

use super::{
    CosimAdapter, CosimInputKind, CosimSignalValue, CosimSignals, CosimStep, CosimStepResult,
};
use crate::peripherals::components::rule_machine::{PinOnlyCtx, RuleMachine};
use crate::{SimResult, SimulationError};

/// Construct one sampled behavior block. Its input/output map keys are local
/// aliases; CosimRunner owns the mapping to shared signal paths.
pub fn build_behavior_adapter(config: &CosimModelConfig) -> SimResult<Box<dyn CosimAdapter>> {
    if let Some(device_type) = config.config.get("device_type").and_then(Value::as_str) {
        let gpio = super::gpio_device::catalog_gpio_device(device_type).map_err(|error| {
            SimulationError::Other(format!("behavior model '{}': {error:#}", config.id))
        })?;
        if let Some(descriptor) = gpio {
            return super::gpio_device::build_gpio_device_adapter(config, descriptor);
        }
        return super::analog_source::build_analog_source_adapter(config);
    }
    BehaviorAdapter::from_config(config)
        .map(|adapter| Box::new(adapter) as Box<dyn CosimAdapter>)
        .map_err(|error| {
            SimulationError::Other(format!("behavior model '{}': {error:#}", config.id))
        })
}

struct BehaviorAdapter {
    id: String,
    machine: RuleMachine,
    inputs: Vec<String>,
    outputs: Vec<String>,
    slots: BTreeMap<String, f64>,
    sampled: Vec<f64>,
    changed: Vec<usize>,
    initialized: bool,
    expr_scale: BTreeMap<String, f64>,
    expr_precision: BTreeMap<String, InputPrecision>,
    time_ns: u64,
}

impl BehaviorAdapter {
    fn from_config(config: &CosimModelConfig) -> Result<Self> {
        ensure!(config.model.is_none(), "model paths are not supported");
        ensure!(config.step_ns > 0, "step_ns must be greater than zero");
        for key in config.config.keys() {
            ensure!(key == "behavior", "unsupported config key '{key}'");
        }
        let source = config
            .config
            .get("behavior")
            .context("config.behavior is required")?;
        validate_shape(source)?;
        let behavior: DeviceBehavior =
            serde_yaml::from_value(source.clone()).context("invalid config.behavior")?;
        ensure!(
            behavior.primitive == "gpio_device",
            "behavior.primitive must be gpio_device for a sampled digital block"
        );

        let mut inputs: Vec<_> = config.inputs.keys().cloned().collect();
        let mut outputs: Vec<_> = config.outputs.keys().cloned().collect();
        inputs.sort();
        outputs.sort();
        validate_names("input aliases", &inputs)?;
        validate_names("output aliases", &outputs)?;
        validate_names("behavior.outputs", &behavior.outputs)?;
        validate_names("behavior.states", &behavior.states)?;
        let vars: Vec<_> = behavior.vars.keys().cloned().collect();
        validate_names("behavior.vars", &vars)?;
        for output in &outputs {
            ensure!(
                behavior.outputs.contains(output),
                "output alias '{output}' is not declared in behavior.outputs"
            );
        }
        for pin in behavior.pin_defaults.keys() {
            ensure!(
                behavior.outputs.contains(pin),
                "behavior.pin_defaults names undeclared output '{pin}'"
            );
        }

        let names = RuleNames {
            registers: &[],
            fields: &[],
            states: &behavior.states,
            vars: &vars,
            fifos: &[],
            timers: &[],
            data_ready: &[],
            outputs: &behavior.outputs,
            inputs: &inputs,
            pins: &[],
            frames: None,
        };
        validate_rule_names(&behavior.rules, &names)?;
        // Shared name validation covers actions and pins. Validate the remaining
        // expression names against this register-free subset as well, so an
        // unknown input/variable cannot silently evaluate to zero.
        for (index, rule) in compile_rules(&behavior.rules)?.iter().enumerate() {
            if let Some(expr) = &rule.when {
                validate_expr(expr, &names)
                    .with_context(|| format!("behavior.rules[{index}].when"))?;
            }
            for (action_index, action) in rule.actions.iter().enumerate() {
                match action {
                    CompiledAction::Goto { .. } => {}
                    CompiledAction::Var { value, .. }
                    | CompiledAction::Pin { level: value, .. } => {
                        validate_expr(value, &names).with_context(|| {
                            format!("behavior.rules[{index}].do[{action_index}]")
                        })?;
                    }
                    _ => bail!("behavior.rules[{index}] uses an unsupported action"),
                }
            }
        }

        let mut machine = RuleMachine::from_behavior(&behavior)?
            .context("behavior must declare at least one state, variable, output or rule")?;
        for output in &behavior.outputs {
            machine.drive_pin(
                output,
                behavior.pin_defaults.get(output).copied().unwrap_or(false),
            );
        }
        // Initial levels are stored in the same machine used for later writes.
        // This adapter publishes a boundary snapshot, not a queue of pad edges.
        machine.take_pin_drives();
        Ok(Self {
            id: config.id.clone(),
            machine,
            slots: inputs.iter().map(|name| (name.clone(), 0.0)).collect(),
            sampled: vec![0.0; inputs.len()],
            changed: Vec::with_capacity(inputs.len()),
            initialized: false,
            inputs,
            outputs,
            expr_scale: BTreeMap::new(),
            expr_precision: BTreeMap::new(),
            time_ns: 0,
        })
    }

    fn advance(
        &mut self,
        time_ns: u64,
        dt_ns: u64,
        inputs: &CosimSignals,
        outputs: &mut CosimSignals,
    ) -> Result<()> {
        ensure!(time_ns >= self.time_ns, "time cannot move backwards");
        ensure!(dt_ns <= time_ns, "dt_ns exceeds time_ns");

        // Validate the whole sample before changing slots, state, time or the
        // caller's output buffer. Missing routes and bad values never become
        // fabricated zeros or a partially applied input update.
        for name in inputs.keys() {
            ensure!(
                self.inputs.binary_search(name).is_ok(),
                "unknown input alias '{name}'"
            );
        }
        self.changed.clear();
        for (index, name) in self.inputs.iter().enumerate() {
            let value = numeric_input(
                inputs
                    .get(name)
                    .with_context(|| format!("missing input alias '{name}'"))?,
            )
            .with_context(|| format!("input '{name}'"))?;
            if !self.initialized || self.slots.get(name) != Some(&value) {
                self.changed.push(index);
            }
            self.sampled[index] = value;
        }

        for (name, value) in self.inputs.iter().zip(&self.sampled) {
            // Every input slot was allocated and validated at construction.
            *self.slots.get_mut(name).expect("declared input slot") = *value;
        }
        self.machine
            .advance_time_us(time_ns / 1_000 - self.time_ns / 1_000);
        self.time_ns = time_ns;
        for &index in &self.changed {
            let mut ctx = PinOnlyCtx {
                slots: &mut self.slots,
                expr_scale: &self.expr_scale,
                expr_precision: &self.expr_precision,
            };
            self.machine.fire(
                &Event::Input {
                    key: self.inputs[index].clone(),
                },
                0,
                &mut ctx,
            );
        }
        self.initialized = true;
        self.machine.take_pin_drives();
        let notes = self.machine.fidelity_notes();
        ensure!(
            notes.is_empty(),
            "rule evaluation reported: {}",
            notes.join("; ")
        );

        outputs.retain(|name, _| self.outputs.binary_search(name).is_ok());
        for name in &self.outputs {
            let level = self
                .machine
                .pin_level(name)
                .with_context(|| format!("output '{name}' has no level"))?;
            match outputs.get_mut(name) {
                Some(value) => *value = CosimSignalValue::Bool(level),
                None => {
                    outputs.insert(name.clone(), CosimSignalValue::Bool(level));
                }
            }
        }
        Ok(())
    }
}

impl CosimAdapter for BehaviorAdapter {
    fn step(&mut self, step: CosimStep) -> SimResult<CosimStepResult> {
        let mut outputs = CosimSignals::new();
        self.step_into(step.time_ns, step.dt_ns, &step.inputs, &mut outputs)?;
        Ok(CosimStepResult { outputs })
    }

    fn step_into(
        &mut self,
        time_ns: u64,
        dt_ns: u64,
        inputs: &CosimSignals,
        outputs: &mut CosimSignals,
    ) -> SimResult<()> {
        self.advance(time_ns, dt_ns, inputs, outputs)
            .map_err(|error| {
                SimulationError::Other(format!("behavior model '{}': {error:#}", self.id))
            })
    }

    fn input_initial_value(&self, name: &str) -> Option<CosimSignalValue> {
        self.inputs
            .iter()
            .any(|input| input == name)
            .then_some(CosimSignalValue::F64(0.0))
    }

    fn input_kind(&self, name: &str) -> Option<CosimInputKind> {
        self.inputs
            .iter()
            .any(|input| input == name)
            .then_some(CosimInputKind::Number)
    }
}

fn numeric_input(value: &CosimSignalValue) -> Result<f64> {
    // PinOnlyCtx stores engineering values as f64. Refuse integer magnitudes
    // that cannot be represented exactly there, instead of truncating bits.
    const MAX_EXACT: i64 = (1_i64 << 53) - 1;
    match value {
        CosimSignalValue::Bool(value) => Ok(f64::from(u8::from(*value))),
        CosimSignalValue::I64(value) => {
            ensure!(
                (-MAX_EXACT..=MAX_EXACT).contains(value),
                "integer exceeds the exact scalar range"
            );
            Ok(*value as f64)
        }
        CosimSignalValue::F64(value) => {
            ensure!(value.is_finite(), "number must be finite");
            ensure!(
                value.abs() <= MAX_EXACT as f64,
                "number exceeds the exact scalar range"
            );
            Ok(*value)
        }
        CosimSignalValue::Text(_) => bail!("expected a Boolean or numeric scalar, received text"),
    }
}

fn validate_names(label: &str, names: &[String]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for name in names {
        ensure!(!name.trim().is_empty(), "{label} contains an empty name");
        ensure!(seen.insert(name), "{label} duplicates '{name}'");
    }
    Ok(())
}

fn keys<'a>(value: &'a Value, allowed: &[&str], path: &str) -> Result<&'a Mapping> {
    let map = value
        .as_mapping()
        .with_context(|| format!("{path} must be a mapping"))?;
    for key in map.keys() {
        let key = key
            .as_str()
            .with_context(|| format!("{path} keys must be strings"))?;
        ensure!(allowed.contains(&key), "{path} has unsupported key '{key}'");
    }
    Ok(map)
}

fn at<'a>(map: &'a Mapping, name: &str) -> Option<&'a Value> {
    map.get(Value::String(name.to_string()))
}

fn validate_shape(source: &Value) -> Result<()> {
    let behavior = keys(
        source,
        &[
            "primitive",
            "states",
            "vars",
            "outputs",
            "pin_defaults",
            "rules",
        ],
        "config.behavior",
    )?;
    let Some(rules) = at(behavior, "rules") else {
        return Ok(());
    };
    let rules = rules
        .as_sequence()
        .context("config.behavior.rules must be a sequence")?;
    for (index, rule) in rules.iter().enumerate() {
        let path = format!("config.behavior.rules[{index}]");
        let rule = keys(rule, &["on", "when", "do"], &path)?;
        let on = at(rule, "on").with_context(|| format!("{path}.on is required"))?;
        let event = keys(on, &["input"], &format!("{path}.on"))?;
        ensure!(
            at(event, "input").and_then(Value::as_str).is_some(),
            "{path}.on must name an input"
        );
        if let Some(actions) = at(rule, "do") {
            let actions = actions
                .as_sequence()
                .with_context(|| format!("{path}.do must be a sequence"))?;
            for (action_index, action) in actions.iter().enumerate() {
                let path = format!("{path}.do[{action_index}]");
                let map = action
                    .as_mapping()
                    .with_context(|| format!("{path} must be a mapping"))?;
                if at(map, "goto").is_some() {
                    keys(action, &["goto"], &path)?;
                } else if let Some(pin) = at(map, "pin") {
                    if pin.is_mapping() {
                        keys(action, &["pin"], &path)?;
                        keys(pin, &["name", "level"], &format!("{path}.pin"))?;
                    } else {
                        keys(action, &["pin", "level"], &path)?;
                    }
                } else if let Some(var) = at(map, "var") {
                    if var.is_mapping() {
                        keys(action, &["var"], &path)?;
                        keys(var, &["name", "value"], &format!("{path}.var"))?;
                    } else {
                        keys(action, &["var", "value"], &path)?;
                    }
                } else {
                    bail!("{path} requires a goto, pin or var action");
                }
            }
        }
    }
    Ok(())
}

fn validate_expr(expr: &Expr, names: &RuleNames<'_>) -> Result<()> {
    let has = |set: &[String], name: &str| set.iter().any(|entry| entry == name);
    match expr {
        Expr::Int(_) => {}
        Expr::Input(name) | Expr::InputNegative(name) => {
            ensure!(has(names.inputs, name), "undeclared input '{name}'");
        }
        Expr::Var(name) => ensure!(has(names.vars, name), "undeclared variable '{name}'"),
        Expr::Pin(name) => ensure!(has(names.outputs, name), "undeclared output '{name}'"),
        Expr::StateIs { name, .. } => {
            ensure!(has(names.states, name), "undeclared state '{name}'");
        }
        Expr::Unary(_, inner) => validate_expr(inner, names)?,
        Expr::Binary(_, lhs, rhs) => {
            validate_expr(lhs, names)?;
            validate_expr(rhs, names)?;
        }
        _ => bail!("unsupported expression in a sampled digital block: {expr:?}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(behavior: &str) -> CosimModelConfig {
        CosimModelConfig {
            id: "gate".into(),
            // The builder is tested directly, independently of registry dispatch.
            adapter: labwired_config::CosimAdapter::Mock,
            model: None,
            step_ns: 1_000,
            inputs: [("enable".into(), "control.enable".into())].into(),
            outputs: [("ready".into(), "device.ready".into())].into(),
            config: [("behavior".into(), serde_yaml::from_str(behavior).unwrap())].into(),
        }
    }

    const GATE: &str = r#"
primitive: gpio_device
states: [idle, ready]
vars: {events: 0}
outputs: [ready]
pin_defaults: {ready: false}
rules:
  - on: {input: enable}
    do: [{var: events, value: "var(events) + 1"}]
  - on: {input: enable}
    when: input(enable) != 0
    do: [{goto: ready}, {pin: ready, level: 1}]
  - on: {input: enable}
    when: input(enable) == 0
    do: [{goto: idle}, {pin: ready, level: 0}]
"#;

    fn sample(
        adapter: &mut BehaviorAdapter,
        time_ns: u64,
        value: CosimSignalValue,
    ) -> CosimSignals {
        let mut outputs = CosimSignals::new();
        adapter
            .step_into(
                time_ns,
                time_ns.saturating_sub(adapter.time_ns),
                &[("enable".into(), value)].into(),
                &mut outputs,
            )
            .unwrap();
        outputs
    }

    #[test]
    fn first_sample_and_changes_use_the_shared_state_machine() {
        let mut adapter = BehaviorAdapter::from_config(&config(GATE)).unwrap();
        assert_eq!(adapter.machine.pin_level("ready"), Some(false));
        assert_eq!(
            sample(&mut adapter, 0, CosimSignalValue::Bool(false))["ready"],
            CosimSignalValue::Bool(false)
        );
        assert_eq!(adapter.machine.var("events"), 1);
        sample(&mut adapter, 1_000, CosimSignalValue::I64(0));
        assert_eq!(adapter.machine.var("events"), 1);
        assert_eq!(
            sample(&mut adapter, 2_000, CosimSignalValue::Bool(true))["ready"],
            CosimSignalValue::Bool(true)
        );
        assert_eq!(adapter.machine.state(), "ready");
        sample(&mut adapter, 3_000, CosimSignalValue::Bool(false));
        assert_eq!(adapter.machine.state(), "idle");
        assert_eq!(adapter.machine.var("events"), 3);
    }

    #[test]
    fn all_inputs_are_sampled_before_events_in_lexical_order() {
        let mut cfg = config(
            r#"
primitive: gpio_device
vars: {order: 0}
outputs: [ready]
rules:
  - on: {input: z}
    do: [{var: order, value: "var(order) * 10 + 2"}]
  - on: {input: a}
    do:
      - {var: order, value: "var(order) * 10 + 1"}
      - {pin: ready, level: "input(z)"}
"#,
        );
        cfg.inputs = [("z".into(), "z".into()), ("a".into(), "a".into())].into();
        let mut adapter = BehaviorAdapter::from_config(&cfg).unwrap();
        let inputs = [
            ("z".into(), CosimSignalValue::Bool(true)),
            ("a".into(), CosimSignalValue::Bool(true)),
        ]
        .into();
        let mut outputs = CosimSignals::new();
        adapter.step_into(0, 0, &inputs, &mut outputs).unwrap();
        assert_eq!(adapter.machine.var("order"), 12);
        assert_eq!(outputs["ready"], CosimSignalValue::Bool(true));
    }

    #[test]
    fn numeric_inputs_follow_existing_integer_rounding() {
        let mut adapter = BehaviorAdapter::from_config(&config(GATE)).unwrap();
        assert_eq!(
            sample(&mut adapter, 0, CosimSignalValue::F64(0.4))["ready"],
            CosimSignalValue::Bool(false)
        );
        assert_eq!(
            sample(&mut adapter, 1_000, CosimSignalValue::F64(0.6))["ready"],
            CosimSignalValue::Bool(true)
        );
    }

    #[test]
    fn invalid_samples_do_not_mutate_state_time_or_outputs() {
        let mut adapter = BehaviorAdapter::from_config(&config(GATE)).unwrap();
        sample(&mut adapter, 0, CosimSignalValue::Bool(false));
        let sentinel: CosimSignals = [("sentinel".into(), CosimSignalValue::I64(9))].into();
        let mut bad_samples = vec![CosimSignals::new()];
        for value in [
            CosimSignalValue::F64(f64::NAN),
            CosimSignalValue::F64(f64::INFINITY),
            CosimSignalValue::Text("yes".into()),
            CosimSignalValue::I64(i64::MAX),
        ] {
            bad_samples.push([("enable".into(), value)].into());
        }
        bad_samples.push(
            [
                ("enable".into(), CosimSignalValue::Bool(true)),
                ("typo".into(), CosimSignalValue::I64(1)),
            ]
            .into(),
        );
        for inputs in bad_samples {
            let mut outputs = sentinel.clone();
            assert!(adapter
                .step_into(1_000, 1_000, &inputs, &mut outputs)
                .is_err());
            assert_eq!(adapter.machine.state(), "idle");
            assert_eq!(adapter.machine.var("events"), 1);
            assert_eq!(adapter.time_ns, 0);
            assert_eq!(outputs, sentinel);
        }
        sample(&mut adapter, 1_000, CosimSignalValue::Bool(true));
        assert_eq!(adapter.machine.state(), "ready");
    }

    #[test]
    fn unsupported_and_misspelled_yaml_is_rejected_before_execution() {
        for bad in [
            GATE.replace("vars:", "timers: []\nvars:"),
            GATE.replace("primitive:", "primtive:"),
            GATE.replace("on: {input: enable}", "on: {input: enable, typo: 1}"),
            GATE.replace("level: 1", "level: 1, typo: 2"),
            GATE.replace("when: input(enable)", "whne: input(enable)"),
            GATE.replace("input(enable)", "input(typo)"),
            GATE.replace("var(events) + 1", "var(typo) + 1"),
            GATE.replace("input(enable) != 0", "reg(CR) != 0"),
            GATE.replace("input(enable) != 0", "fifo_len(q) != 0"),
            GATE.replace("input(enable) != 0", "state == typo"),
            GATE.replace("{goto: ready}", "{goto: ready, pin: ready, level: 1}"),
            GATE.replace("[idle, ready]", "[idle, ready, ready]"),
        ] {
            assert!(
                BehaviorAdapter::from_config(&config(&bad)).is_err(),
                "accepted unsupported behavior: {bad}"
            );
        }
        let mut cfg = config(GATE);
        cfg.outputs.insert("typo".into(), "device.typo".into());
        assert!(BehaviorAdapter::from_config(&cfg).is_err());
        let mut cfg = config(GATE);
        cfg.config.insert("typo".into(), Value::Bool(true));
        assert!(BehaviorAdapter::from_config(&cfg).is_err());
    }

    #[test]
    fn evaluation_faults_and_backwards_time_are_errors() {
        let mut adapter = BehaviorAdapter::from_config(&config(
            &GATE.replace("var(events) + 1", "1 / input(enable)"),
        ))
        .unwrap();
        let inputs = [("enable".into(), CosimSignalValue::I64(0))].into();
        assert!(adapter
            .step_into(0, 0, &inputs, &mut CosimSignals::new())
            .is_err());

        let mut adapter = BehaviorAdapter::from_config(&config(GATE)).unwrap();
        sample(&mut adapter, 1_000, CosimSignalValue::Bool(false));
        assert!(adapter
            .step_into(0, 0, &inputs, &mut CosimSignals::new())
            .is_err());
        assert_eq!(adapter.time_ns, 1_000);
    }

    #[test]
    fn nested_actions_reuse_the_existing_grammar() {
        let source = GATE
            .replace(
                "{pin: ready, level: 1}",
                "{pin: {name: ready, level: true}}",
            )
            .replace(
                "{var: events, value: \"var(events) + 1\"}",
                "{var: {name: events, value: \"var(events) + 1\"}}",
            );
        let mut adapter = BehaviorAdapter::from_config(&config(&source)).unwrap();
        assert_eq!(
            sample(&mut adapter, 0, CosimSignalValue::Bool(true))["ready"],
            CosimSignalValue::Bool(true)
        );
    }
}
