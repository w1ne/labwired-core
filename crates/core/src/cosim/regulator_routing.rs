//! One owner, complete control/observation frames and exact regulator model identity.
use super::{
    CosimModelConfig, CosimSignals, ManifestCosimAdapter, RoutingError, SignalPath, SignalRouter,
    SystemBus, WriteBinding,
};
use crate::analog::{controlled_buck, AnalogConfig};
use crate::regulator::{RegulatorObservation, REGULATOR_INPUT_FIELDS, REGULATOR_OUTPUT_FIELDS};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub(super) struct RegulatorBinding {
    pub id: String,
    pub step_ns: u64,
    pub last_sample_ns: Option<u64>,
    pub held_command: crate::regulator::RegulatorCommand,
    pub capture_epoch_ns: u64,
}
fn rejected(id: &str, reason: impl Into<String>) -> RoutingError {
    RoutingError::RegulatorRejected {
        path: format!("board.regulator.{id}"),
        reason: reason.into(),
    }
}

impl SignalRouter {
    /// Capture only at the actual CPU epoch beginning an owning interval.
    /// Faster unrelated models and mid-interval MMIO cannot rewrite a held frame.
    pub(super) fn refresh_regulator_commands(&mut self, bus: &SystemBus, time_ns: u64) {
        for binding in &mut self.regulators {
            if binding.capture_epoch_ns == time_ns {
                binding.held_command = bus
                    .regulators
                    .get(&binding.id)
                    .expect("bound regulator")
                    .control
                    .command();
            }
        }
    }
    pub(super) fn bind_regulators(
        &mut self,
        configs: &[CosimModelConfig],
        bus: &SystemBus,
        errors: &mut Vec<RoutingError>,
    ) {
        for (id, stage) in bus.regulators.iter() {
            let mut owners = Vec::new();
            for config in configs {
                let involved = config.inputs.values().chain(config.outputs.values()).any(|path| {
                    matches!(SignalPath::parse(path), Some(SignalPath::Regulator { regulator, .. }) if regulator == *id)
                });
                if involved {
                    owners.push(config);
                }
            }
            if owners.len() != 1 {
                errors.push(rejected(
                    id,
                    "every declared regulator requires exactly one complete analog model owner",
                ));
                continue;
            }
            match validate_owner(id, &stage.descriptor, owners[0], bus) {
                Ok(()) => self.regulators.push(RegulatorBinding {
                    id: id.clone(),
                    step_ns: owners[0].step_ns,
                    last_sample_ns: None,
                    held_command: crate::regulator::RegulatorCommand {
                        configuration: stage.descriptor.initial,
                        command_revision: 0,
                    },
                    capture_epoch_ns: 0,
                }),
                Err(reason) => errors.push(rejected(id, reason)),
            }
        }
    }
    pub(super) fn collect_regulator_observations(
        &self,
        bus: &SystemBus,
        signals: &CosimSignals,
        time_ns: u64,
    ) -> Result<Vec<(String, RegulatorObservation)>, RoutingError> {
        let mut batch = Vec::new();
        for binding in &self.regulators {
            let Some(sample) =
                sample_due(binding, time_ns).map_err(|reason| rejected(&binding.id, reason))?
            else {
                continue;
            };
            let mut values = vec![None; 7];
            for (path, sink) in &self.writes {
                if let WriteBinding::Regulator { id, field } = sink {
                    if id == &binding.id {
                        values[usize::from(*field)] = signals.get(path);
                    }
                }
            }
            if values.iter().any(|value| value.is_none()) {
                return Err(rejected(
                    &binding.id,
                    "every owning regulator boundary requires all seven fresh observation fields",
                ));
            }
            let values: Vec<_> = values.into_iter().map(Option::unwrap).collect();
            let parse = || -> Result<RegulatorObservation, String> {
                Ok(RegulatorObservation {
                    time_ns: sample,
                    input_voltage_v: controlled_buck::strict_number(values[0])?,
                    feedback_voltage_v: controlled_buck::strict_number(values[1])?,
                    input_current_a: controlled_buck::strict_number(values[2])?,
                    output_current_a: controlled_buck::strict_number(values[3])?,
                    output_enabled: controlled_buck::strict_bool(values[4])?,
                    qualified: controlled_buck::strict_bool(values[5])?,
                    applied_revision: controlled_buck::strict_revision(values[6])?,
                })
            };
            batch.push((
                binding.id.clone(),
                parse().map_err(|reason| rejected(&binding.id, reason))?,
            ));
        }
        bus.regulators
            .validate_observations(&batch)
            .map_err(|reason| rejected("", reason))?;
        Ok(batch)
    }
    pub(super) fn publish_regulator_observations(
        &mut self,
        bus: &mut SystemBus,
        batch: &[(String, RegulatorObservation)],
        boundary_epoch_ns: u64,
    ) -> Result<(), RoutingError> {
        bus.regulators
            .apply_observations(batch)
            .map_err(|reason| rejected("", reason))?;
        for (id, _) in batch {
            let peripheral = bus
                .regulators
                .get(id)
                .expect("bound regulator")
                .descriptor
                .control_peripheral
                .clone();
            let index = bus
                .find_peripheral_index_by_name(&peripheral)
                .expect("bound regulator control peripheral");
            bus.peripherals[index].dev.sync_regulator_control();
        }
        for (id, observation) in batch {
            let binding = self
                .regulators
                .iter_mut()
                .find(|binding| binding.id == *id)
                .expect("bound regulator");
            binding.last_sample_ns = Some(observation.time_ns);
            binding.held_command = bus
                .regulators
                .get(id)
                .expect("bound regulator")
                .control
                .command();
            binding.capture_epoch_ns = boundary_epoch_ns;
        }
        Ok(())
    }
}
fn sample_due(binding: &RegulatorBinding, time_ns: u64) -> Result<Option<u64>, String> {
    let step = binding.step_ns;
    let sample = time_ns / step * step;
    if (time_ns != 0 && sample == 0) || binding.last_sample_ns == Some(sample) {
        return Ok(None);
    }
    let expected = binding
        .last_sample_ns
        .map(|previous| previous.checked_add(step))
        .unwrap_or(Some(if sample == 0 { 0 } else { step }));
    if expected != Some(sample) {
        return Err("regulator routing skipped or reversed an owning model interval".into());
    }
    Ok(Some(sample))
}
fn validate_owner(
    id: &str,
    descriptor: &labwired_config::InternalRegulatorConfig,
    config: &CosimModelConfig,
    bus: &SystemBus,
) -> Result<(), String> {
    if config.adapter != ManifestCosimAdapter::Analog || config.step_ns == 0 {
        return Err(
            "internal regulator control requires an in-core analog model with positive step_ns"
                .into(),
        );
    }
    let mut prefixes = BTreeSet::new();
    for (fields, routes) in [
        (REGULATOR_INPUT_FIELDS.as_slice(), &config.inputs),
        (REGULATOR_OUTPUT_FIELDS.as_slice(), &config.outputs),
    ] {
        for field in fields {
            let expected = format!("board.regulator.{id}.{field}");
            let ports: Vec<_> = routes
                .iter()
                .filter(|(_, route)| **route == expected)
                .map(|(port, _)| port)
                .collect();
            if ports.len() != 1 {
                return Err(format!(
                    "regulator field '{field}' must appear exactly once in the owning model"
                ));
            }
            let prefix = ports[0]
                .strip_suffix(&format!(".{field}"))
                .ok_or_else(|| format!("regulator port must end in '.{field}'"))?;
            prefixes.insert(prefix);
        }
    }
    if prefixes.len() != 1 {
        return Err("all regulator ports must use one adapter prefix".into());
    }
    let parsed = AnalogConfig::from_yaml(&config.config).map_err(|error| error.to_string())?;
    let prefix = *prefixes.first().unwrap();
    let stage = parsed
        .regulators
        .get(prefix)
        .ok_or("regulator route prefix has no config.regulators entry")?;
    if stage.initial.command().configuration != descriptor.initial {
        return Err("regulator initial configuration differs from its chip descriptor".into());
    }
    for (domain, low, high) in [
        (
            &descriptor.input_domain,
            stage.input_min_v,
            stage.input_max_v,
        ),
        (
            &descriptor.enable_domain,
            stage.enable_min_v,
            stage.enable_max_v,
        ),
    ] {
        let rail = bus
            .mcu_power
            .handle(domain)
            .ok_or("regulator physical domain is absent")?
            .observation();
        if rail.min_v != low || rail.max_v != high {
            return Err(
                "regulator supply qualification differs from its physical domain envelope".into(),
            );
        }
    }
    // Inline circuit identity is inspectable during binding. A file/process
    // attachment must not silently skip these physical/model checks.
    let source = config
        .config
        .get("netlist_text")
        .and_then(serde_yaml::Value::as_str)
        .ok_or(
            "internal regulator routing requires an inline netlist_text for exact stage identity",
        )?;
    let mut circuit = crate::analog::parse_netlist(source).map_err(|error| error.to_string())?;
    controlled_buck::bind_stages(&mut circuit, &parsed.regulators)
        .map_err(|error| error.to_string())?;
    let macro_stage = circuit
        .buck_macros
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(&stage.element))
        .ok_or("controlled BUCK instance is absent")?;
    let model = macro_stage.model;
    let expected = descriptor.model;
    if model.dmax != expected.max_duty
        || model.iq != expected.quiescent_current_a
        || model.rout != expected.output_resistance_ohms
        || model.eff != expected.efficiency
        || model.aol != expected.open_loop_gain
        || model.vdo != 0.0
        || model.line != 0.0
    {
        return Err(
            "BUCK card differs from the descriptor's explicit averaged-model assumptions".into(),
        );
    }
    let enable = circuit
        .node(&stage.enable_node)
        .ok_or("regulator enable node is absent")?;
    if macro_stage.reference.is_some()
        || macro_stage.input == macro_stage.reference
        || macro_stage.output == macro_stage.reference
        || macro_stage.feedback == macro_stage.reference
        || macro_stage.output == macro_stage.feedback
        || [macro_stage.input, enable].contains(&macro_stage.output)
        || [macro_stage.input, enable].contains(&macro_stage.feedback)
    {
        return Err("internal regulator requires distinct SW/FB/ground roles and a common physical zero return".into());
    }
    for (domain, node) in [
        (&descriptor.input_domain, macro_stage.input),
        (&descriptor.enable_domain, enable),
    ] {
        let expected = format!("board.power.{domain}_volts");
        let ports: Vec<_> = config
            .outputs
            .iter()
            .filter(|(_, route)| **route == expected)
            .map(|(port, _)| port)
            .collect();
        if ports.len() != 1 {
            return Err(format!(
                "regulator domain '{domain}' requires one observation from the same model"
            ));
        }
        let expression = parsed
            .probes
            .get(ports[0])
            .ok_or("regulator supply must use its actual node-voltage probe")?;
        if voltage_node(&circuit, expression) != Some(node) {
            return Err(format!(
                "regulator domain '{domain}' is sampled from a different physical node"
            ));
        }
    }
    Ok(())
}
fn voltage_node(
    circuit: &crate::analog::Circuit,
    expression: &str,
) -> Option<crate::analog::NodeRef> {
    let text = expression.trim();
    let node = if text.len() > 3
        && matches!(text.as_bytes()[0], b'v' | b'V')
        && text.as_bytes()[1] == b'('
        && text.ends_with(')')
    {
        &text[2..text.len() - 1]
    } else {
        text
    };
    circuit.node(node)
}
