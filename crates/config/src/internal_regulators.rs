//! Declarative internal regulator roles and explicit averaged-model assumptions.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_INTERNAL_REGULATORS: usize = 16;
pub const MAX_REGULATOR_REVISION: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegulatorConfiguration {
    pub enabled: bool,
    pub target_voltage_v: f64,
    pub current_limit_a: f64,
    pub rated_load_a: f64,
}
impl RegulatorConfiguration {
    pub fn validate(&self) -> Result<(), String> {
        if [
            self.target_voltage_v,
            self.current_limit_a,
            self.rated_load_a,
        ]
        .iter()
        .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("regulator target voltage, current limit and rated load must be finite and positive".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InternalRegulatorKind {
    Buck,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InternalRegulatorPins {
    #[serde(deserialize_with = "deserialize_regulator_labels")]
    pub switch_output: Vec<String>,
    #[serde(deserialize_with = "deserialize_regulator_labels")]
    pub feedback: Vec<String>,
    #[serde(deserialize_with = "deserialize_regulator_labels")]
    pub ground: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuckModelConfig {
    pub max_duty: f64,
    pub quiescent_current_a: f64,
    pub output_resistance_ohms: f64,
    pub efficiency: f64,
    pub open_loop_gain: f64,
}
impl BuckModelConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(self.max_duty.is_finite()
            && self.max_duty > 0.0
            && self.max_duty <= 1.0
            && self.quiescent_current_a.is_finite()
            && self.quiescent_current_a >= 0.0
            && self.output_resistance_ohms.is_finite()
            && self.output_resistance_ohms > 0.0
            && self.output_resistance_ohms.recip().is_finite()
            && self.efficiency.is_finite()
            && self.efficiency > 0.0
            && self.efficiency <= 1.0
            && self.open_loop_gain.is_finite()
            && self.open_loop_gain > 1.0)
        {
            return Err("BUCK model requires finite 0 < max_duty <= 1, IQ >= 0, ROUT > 0, 0 < efficiency <= 1 and gain > 1".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "InternalRegulatorYaml", into = "InternalRegulatorYaml")]
pub struct InternalRegulatorConfig {
    pub id: String,
    pub kind: InternalRegulatorKind,
    pub control_peripheral: String,
    pub input_domain: String,
    pub enable_domain: String,
    pub pins: InternalRegulatorPins,
    pub initial: RegulatorConfiguration,
    pub model: BuckModelConfig,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InternalRegulatorYaml {
    #[serde(deserialize_with = "deserialize_regulator_string")]
    id: String,
    kind: InternalRegulatorKind,
    #[serde(deserialize_with = "deserialize_regulator_string")]
    control_peripheral: String,
    #[serde(deserialize_with = "deserialize_regulator_string")]
    input_domain: String,
    #[serde(deserialize_with = "deserialize_regulator_string")]
    enable_domain: String,
    pins: InternalRegulatorPins,
    initial: RegulatorConfiguration,
    model: BuckModelConfig,
}

impl InternalRegulatorConfig {
    pub fn validate(&self) -> Result<(), String> {
        for name in [&self.id, &self.input_domain, &self.enable_domain] {
            if !crate::valid_power_domain_name(name) {
                return Err(format!("invalid internal regulator identifier '{name}'"));
            }
        }
        if self.control_peripheral.is_empty()
            || self.control_peripheral.trim() != self.control_peripheral
        {
            return Err("internal regulator requires an exact nonempty control_peripheral".into());
        }
        self.initial.validate()?;
        self.model.validate()?;
        let mut pins = BTreeSet::new();
        for role in [
            &self.pins.switch_output,
            &self.pins.feedback,
            &self.pins.ground,
        ] {
            if !(1..=64).contains(&role.len()) {
                return Err("internal regulator pin roles require 1..64 explicit labels".into());
            }
            for pin in role {
                if pin.is_empty()
                    || pin.len() > 64
                    || pin.trim() != pin
                    || !pins.insert(pin.to_ascii_lowercase())
                {
                    return Err(format!(
                        "invalid or repeated internal regulator pin '{pin}'"
                    ));
                }
            }
        }
        Ok(())
    }
}

pub fn validate_internal_regulators(
    regulators: &[InternalRegulatorConfig],
    domains: Option<&crate::PowerDomainsConfig>,
) -> Result<(), String> {
    if regulators.is_empty() {
        return Ok(());
    }
    if regulators.len() > MAX_INTERNAL_REGULATORS {
        return Err("internal_regulators exceeds 16 explicit stages".into());
    }
    let domains = domains.ok_or("internal_regulators requires power_domains")?;
    domains.validate()?;
    let mut ids = BTreeSet::new();
    let mut controllers = BTreeSet::new();
    let rail_pins: BTreeSet<_> = domains
        .rails
        .values()
        .flat_map(|r| &r.pins)
        .map(|p| p.to_ascii_lowercase())
        .collect();
    let mut functional_pins = BTreeSet::new();
    for regulator in regulators {
        regulator.validate()?;
        if !ids.insert(&regulator.id) || !controllers.insert(&regulator.control_peripheral) {
            return Err("internal regulators require distinct IDs and control peripherals".into());
        }
        for domain in [&regulator.input_domain, &regulator.enable_domain] {
            let rail = domains
                .rails
                .get(domain)
                .ok_or_else(|| format!("regulator references undeclared rail '{domain}'"))?;
            if rail.pins.is_empty() {
                return Err(format!(
                    "regulator rail '{domain}' needs physical pin labels"
                ));
            }
        }
        for pin in regulator
            .pins
            .switch_output
            .iter()
            .chain(&regulator.pins.feedback)
            .chain(&regulator.pins.ground)
        {
            let key = pin.to_ascii_lowercase();
            if rail_pins.contains(&key) || !functional_pins.insert(key) {
                return Err(format!(
                    "regulator functional pin '{pin}' aliases another physical role"
                ));
            }
        }
    }
    Ok(())
}

impl TryFrom<InternalRegulatorYaml> for InternalRegulatorConfig {
    type Error = String;
    fn try_from(v: InternalRegulatorYaml) -> Result<Self, Self::Error> {
        let out = Self {
            id: v.id,
            kind: v.kind,
            control_peripheral: v.control_peripheral,
            input_domain: v.input_domain,
            enable_domain: v.enable_domain,
            pins: v.pins,
            initial: v.initial,
            model: v.model,
        };
        out.validate()?;
        Ok(out)
    }
}
impl From<InternalRegulatorConfig> for InternalRegulatorYaml {
    fn from(v: InternalRegulatorConfig) -> Self {
        Self {
            id: v.id,
            kind: v.kind,
            control_peripheral: v.control_peripheral,
            input_domain: v.input_domain,
            enable_domain: v.enable_domain,
            pins: v.pins,
            initial: v.initial,
            model: v.model,
        }
    }
}

/// Absence is the legacy empty default; an explicitly present list is bounded.
pub fn deserialize_internal_regulators<'de, D>(
    deserializer: D,
) -> Result<Vec<InternalRegulatorConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Vec::<InternalRegulatorConfig>::deserialize(deserializer)?;
    if !(1..=MAX_INTERNAL_REGULATORS).contains(&values.len()) {
        return Err(serde::de::Error::custom(
            "internal_regulators requires 1..16 stages when present",
        ));
    }
    Ok(values)
}

pub fn deserialize_regulator_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match serde_yaml::Value::deserialize(deserializer)? {
        serde_yaml::Value::String(value) => Ok(value),
        _ => Err(serde::de::Error::custom("regulator names must be strings")),
    }
}
fn deserialize_regulator_labels<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Vec::<serde_yaml::Value>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|value| match value {
            serde_yaml::Value::String(value) => Ok(value),
            _ => Err(serde::de::Error::custom(
                "regulator physical labels must be strings",
            )),
        })
        .collect()
}
