//! Opt-in sampled control for the existing averaged BUCK macro. Electrical
//! qualification is evaluated inside MNA, including its first operating point.
use super::netlist::{
    Circuit, NodeRef, RegulatorModel, MACRO_GAIN_RESISTANCE, REGULATOR_LIMIT_GAIN,
};
use super::AnalogError;
use crate::regulator::{
    RegulatorCommand, RegulatorConfiguration, REGULATOR_INPUT_FIELDS, REGULATOR_OUTPUT_FIELDS,
};
use labwired_config::MAX_REGULATOR_REVISION;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct BuckMacro {
    pub name: String,
    pub input: NodeRef,
    pub output: NodeRef,
    pub reference: NodeRef,
    pub feedback: NodeRef,
    pub model: RegulatorModel,
    pub reference_source: usize,
    pub limit_source: usize,
    pub output_resistor: usize,
    pub power_sink: usize,
    pub quiescent_vccs: Option<usize>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct BuckStageBinding {
    pub macro_index: usize,
    pub enable_node: NodeRef,
    pub config: BuckStageConfig,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuckInitialCommand {
    pub enabled: bool,
    pub target_voltage_v: f64,
    pub current_limit_a: f64,
    pub rated_load_a: f64,
    pub command_revision: u64,
}
impl BuckInitialCommand {
    pub fn command(&self) -> RegulatorCommand {
        RegulatorCommand {
            configuration: RegulatorConfiguration {
                enabled: self.enabled,
                target_voltage_v: self.target_voltage_v,
                current_limit_a: self.current_limit_a,
                rated_load_a: self.rated_load_a,
            },
            command_revision: self.command_revision,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuckStageConfig {
    #[serde(deserialize_with = "labwired_config::deserialize_regulator_string")]
    pub element: String,
    #[serde(deserialize_with = "labwired_config::deserialize_regulator_string")]
    pub enable_node: String,
    pub input_min_v: f64,
    pub input_max_v: f64,
    pub enable_min_v: f64,
    pub enable_max_v: f64,
    pub initial: BuckInitialCommand,
}
impl BuckStageConfig {
    pub fn validate(&self) -> Result<(), AnalogError> {
        if self.element.trim().is_empty()
            || self.element.trim() != self.element
            || self.enable_node.trim().is_empty()
            || self.enable_node.trim() != self.enable_node
        {
            return Err(config_error(
                "regulator element and enable_node require exact nonempty names",
            ));
        }
        for (low, high) in [
            (self.input_min_v, self.input_max_v),
            (self.enable_min_v, self.enable_max_v),
        ] {
            if !(low.is_finite() && high.is_finite() && 0.0 < low && low <= high) {
                return Err(config_error(
                    "regulator supply bounds require finite 0 < min <= max",
                ));
            }
        }
        self.initial
            .command()
            .configuration
            .validate()
            .map_err(config_error)?;
        if self.initial.command_revision != 0 {
            return Err(config_error(
                "a regulator's initial command_revision must be zero",
            ));
        }
        Ok(())
    }
    pub fn supplies_qualified(&self, input: f64, enable: f64) -> bool {
        input.is_finite()
            && enable.is_finite()
            && self.input_min_v <= input
            && input <= self.input_max_v
            && self.enable_min_v <= enable
            && enable <= self.enable_max_v
    }
}
fn config_error(text: impl Into<String>) -> AnalogError {
    AnalogError::Config(text.into())
}

pub fn bind_stages(
    circuit: &mut Circuit,
    configs: &BTreeMap<String, BuckStageConfig>,
) -> Result<(), AnalogError> {
    if configs.len() > 16 {
        return Err(config_error("config.regulators exceeds 16 stages"));
    }
    let mut elements = BTreeSet::new();
    for (prefix, config) in configs {
        if !labwired_config::valid_power_domain_name(prefix) {
            return Err(config_error("regulator port prefixes require 1..64 ASCII alphanumeric, underscore or hyphen characters"));
        }
        config.validate()?;
        let macro_index = circuit
            .buck_macros
            .iter()
            .position(|stage| stage.name.eq_ignore_ascii_case(&config.element))
            .ok_or_else(|| {
                config_error(format!(
                    "regulator '{}' is not an adjustable BUCK X instance",
                    config.element
                ))
            })?;
        if !elements.insert(macro_index) {
            return Err(config_error("a BUCK stage has multiple control owners"));
        }
        let stage = &circuit.buck_macros[macro_index];
        if stage.model.vref != config.initial.target_voltage_v
            || stage.model.ilim != config.initial.current_limit_a
        {
            return Err(config_error(
                "regulator initial target/current must match the BUCK VREF/ILIM card",
            ));
        }
        validate_command(stage, config.initial.command())?;
        let enable_node = circuit.node(&config.enable_node).ok_or_else(|| {
            config_error(format!(
                "unknown regulator enable node '{}'",
                config.enable_node
            ))
        })?;
        if enable_node == stage.reference {
            return Err(config_error(
                "regulator enable supply cannot be its ground reference",
            ));
        }
        circuit.controlled_bucks.push(BuckStageBinding {
            macro_index,
            enable_node,
            config: config.clone(),
        });
    }
    Ok(())
}

pub fn validate_command(stage: &BuckMacro, command: RegulatorCommand) -> Result<(), AnalogError> {
    command.configuration.validate().map_err(config_error)?;
    let configuration = command.configuration;
    if command.command_revision > MAX_REGULATOR_REVISION
        || !(stage.model.aol / MACRO_GAIN_RESISTANCE * configuration.target_voltage_v).is_finite()
        || !(REGULATOR_LIMIT_GAIN * configuration.current_limit_a).is_finite()
        || !(stage.model.iq / configuration.target_voltage_v).is_finite()
    {
        return Err(config_error(
            "regulator command exceeds finite solver values or safe integer revision",
        ));
    }
    Ok(())
}

pub fn input_port(prefix: &str, field: &str) -> String {
    format!("{prefix}.{field}")
}
pub fn port_names(prefix: &str) -> impl Iterator<Item = String> + '_ {
    REGULATOR_INPUT_FIELDS
        .into_iter()
        .chain(REGULATOR_OUTPUT_FIELDS)
        .map(move |field| input_port(prefix, field))
}

pub fn command_inputs(command: RegulatorCommand) -> [crate::cosim::CosimSignalValue; 5] {
    use crate::cosim::CosimSignalValue::{Bool, F64};
    let c = command.configuration;
    [
        Bool(c.enabled),
        F64(c.target_voltage_v),
        F64(c.current_limit_a),
        F64(c.rated_load_a),
        F64(command.command_revision as f64),
    ]
}
pub fn observation_outputs(
    observation: crate::regulator::RegulatorObservation,
) -> [crate::cosim::CosimSignalValue; 7] {
    use crate::cosim::CosimSignalValue::{Bool, F64};
    [
        F64(observation.input_voltage_v),
        F64(observation.feedback_voltage_v),
        F64(observation.input_current_a),
        F64(observation.output_current_a),
        Bool(observation.output_enabled),
        Bool(observation.qualified),
        F64(observation.applied_revision as f64),
    ]
}
pub fn output_unit(index: usize) -> &'static str {
    match index {
        0 | 1 => "V",
        2 | 3 | 7 => "A",
        _ => "1",
    }
}
pub fn strict_number(value: &crate::cosim::CosimSignalValue) -> Result<f64, String> {
    use crate::cosim::CosimSignalValue::{F64, I64};
    let value = match value {
        F64(v) => *v,
        I64(v) => *v as f64,
        _ => {
            return Err(
                "regulator values must be finite numeric values, not booleans or text".into(),
            )
        }
    };
    if !value.is_finite() {
        return Err("regulator value must be finite".into());
    }
    Ok(value)
}
pub fn strict_revision(value: &crate::cosim::CosimSignalValue) -> Result<u64, String> {
    let number = strict_number(value)?;
    if number < 0.0 || number > MAX_REGULATOR_REVISION as f64 || number.fract() != 0.0 {
        return Err("regulator revision must be an integer in 0..9007199254740991".into());
    }
    Ok(number as u64)
}
pub fn strict_bool(value: &crate::cosim::CosimSignalValue) -> Result<bool, String> {
    match value {
        crate::cosim::CosimSignalValue::Bool(value) => Ok(*value),
        _ => Err("regulator boolean fields require a boolean value".into()),
    }
}
/// Missing whole frames retain the adapter's explicit initial/current command;
/// a partial frame is always an error and cannot mutate any control field.
pub fn parse_command(
    prefix: &str,
    signals: &crate::cosim::CosimSignals,
) -> Result<Option<RegulatorCommand>, String> {
    let values: Vec<_> = REGULATOR_INPUT_FIELDS
        .iter()
        .map(|field| signals.get(&input_port(prefix, field)))
        .collect();
    if values.iter().all(|value| value.is_none()) {
        return Ok(None);
    }
    if values.iter().any(|value| value.is_none()) {
        return Err(format!(
            "regulator '{prefix}' requires all five command fields in the same frame"
        ));
    }
    let values: Vec<_> = values.into_iter().map(Option::unwrap).collect();
    Ok(Some(RegulatorCommand {
        configuration: RegulatorConfiguration {
            enabled: strict_bool(values[0])?,
            target_voltage_v: strict_number(values[1])?,
            current_limit_a: strict_number(values[2])?,
            rated_load_a: strict_number(values[3])?,
        },
        command_revision: strict_revision(values[4])?,
    }))
}

/// Supply qualification is separate from the absolute load-rating diagnostic.
/// This comparison does not qualify reverse-current behavior or clamp current.
pub fn trace_outputs(
    observation: crate::regulator::RegulatorObservation,
    command: RegulatorCommand,
) -> [crate::cosim::CosimSignalValue; 9] {
    use crate::cosim::CosimSignalValue::{Bool, F64};
    let [a, b, c, d, e, f, g] = observation_outputs(observation);
    [
        a,
        b,
        c,
        d,
        e,
        f,
        g,
        F64(command.configuration.rated_load_a),
        Bool(observation.output_current_a.abs() <= command.configuration.rated_load_a),
    ]
}
pub const REGULATOR_TRACE_FIELDS: [&str; 9] = [
    "input_voltage_v",
    "feedback_voltage_v",
    "input_current_a",
    "output_current_a",
    "output_enabled",
    "qualified",
    "applied_revision",
    "rated_load_a",
    "load_within_rating",
];

pub fn owns_generated_primitive(circuit: &Circuit, element: &str) -> bool {
    let name = element.trim().to_ascii_lowercase();
    circuit.controlled_bucks.iter().any(|binding| {
        name.starts_with(&format!(
            "{}#",
            circuit.buck_macros[binding.macro_index]
                .name
                .to_ascii_lowercase()
        ))
    })
}
