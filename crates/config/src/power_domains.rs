//! Explicit physical MCU power domains. Operating limits qualify behavior;
//! they are deliberately separate from POR/BOD detector thresholds.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_MCU_POWER_DOMAINS: usize = 16;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerDomainConfig {
    pub nominal_v: f64,
    pub min_v: f64,
    pub max_v: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatchedPorConfig {
    pub sense_domain: String,
    pub detector_domain: String,
    pub rearm_below_v: f64,
    pub release_at_v: f64,
    pub release_delay_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainBrownOutConfig {
    pub sense_domain: String,
    pub detector_domain: String,
    pub enabled: bool,
    pub falling_v: f64,
    pub rising_v: f64,
    pub startup_delay_ns: u64,
    pub assert_delay_ns: u64,
    pub release_delay_ns: u64,
    pub enable_delay_ns: u64,
    pub reprogram_delay_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_peripheral: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerSystemResetScope {
    CoreOnly,
    Chip,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PowerDomainsYaml", into = "PowerDomainsYaml")]
pub struct PowerDomainsConfig {
    pub rails: BTreeMap<String, PowerDomainConfig>,
    pub io_domain: String,
    pub execution_domains: Vec<String>,
    pub system_reset_scope: PowerSystemResetScope,
    pub latched_por: Option<LatchedPorConfig>,
    pub brown_out: Option<DomainBrownOutConfig>,
    pub peripherals: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PowerDomainsYaml {
    rails: BTreeMap<String, PowerDomainConfig>,
    io_domain: String,
    execution_domains: Vec<String>,
    system_reset_scope: PowerSystemResetScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latched_por: Option<LatchedPorConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    brown_out: Option<DomainBrownOutConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    peripherals: BTreeMap<String, BTreeMap<String, String>>,
}

pub fn valid_power_domain_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl PowerDomainsConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_MCU_POWER_DOMAINS).contains(&self.rails.len()) {
            return Err("power_domains.rails must declare 1..16 physical rails".into());
        }
        let mut pins = BTreeSet::new();
        for (name, rail) in &self.rails {
            if !valid_power_domain_name(name) {
                return Err(format!("invalid power domain name '{name}'"));
            }
            if !(rail.min_v.is_finite()
                && rail.nominal_v.is_finite()
                && rail.max_v.is_finite()
                && 0.0 < rail.min_v
                && rail.min_v <= rail.nominal_v
                && rail.nominal_v <= rail.max_v)
            {
                return Err(format!(
                    "power_domains.rails.{name} requires finite 0 < min_v <= nominal_v <= max_v"
                ));
            }
            if rail.pins.len() > 64 {
                return Err(format!("power_domains.rails.{name}.pins exceeds 64 labels"));
            }
            for pin in &rail.pins {
                if !valid_power_domain_name(pin) || !pins.insert(pin.to_ascii_uppercase()) {
                    return Err(format!(
                        "power domain physical pin '{pin}' is invalid or assigned more than once"
                    ));
                }
            }
        }
        let declared = |domain: &str| -> Result<(), String> {
            if self.rails.contains_key(domain) {
                Ok(())
            } else {
                Err(format!(
                    "power_domains references undeclared rail '{domain}'"
                ))
            }
        };
        declared(&self.io_domain)?;
        if self.execution_domains.is_empty() || self.execution_domains.len() > self.rails.len() {
            return Err(
                "power_domains.execution_domains must name 1..16 distinct declared rails".into(),
            );
        }
        let mut execution = BTreeSet::new();
        for domain in &self.execution_domains {
            declared(domain)?;
            if !execution.insert(domain) {
                return Err(format!("execution domain '{domain}' is duplicated"));
            }
        }
        if let Some(por) = &self.latched_por {
            declared(&por.sense_domain)?;
            declared(&por.detector_domain)?;
            if !(por.rearm_below_v.is_finite()
                && por.release_at_v.is_finite()
                && por.rearm_below_v >= 0.0
                && por.release_at_v > 0.0
                && por.rearm_below_v < self.rails[&por.detector_domain].nominal_v
                && por.release_at_v <= self.rails[&por.sense_domain].nominal_v)
            {
                return Err("latched_por requires finite detector rearm and sense release thresholds below their nominal supplies".into());
            }
        }
        if let Some(bod) = &self.brown_out {
            declared(&bod.sense_domain)?;
            declared(&bod.detector_domain)?;
            if self.latched_por.is_none() {
                return Err(
                    "brown_out.startup_delay_ns requires an explicit latched_por origin".into(),
                );
            }
            if !(bod.falling_v.is_finite()
                && bod.rising_v.is_finite()
                && bod.falling_v > 0.0
                && bod.falling_v <= bod.rising_v)
            {
                return Err("brown_out requires finite 0 < falling_v <= rising_v".into());
            }
            if bod
                .control_peripheral
                .as_deref()
                .is_some_and(|id| id.is_empty())
            {
                return Err(
                    "brown_out.control_peripheral must be a nonempty exact peripheral ID".into(),
                );
            }
        }
        if self.peripherals.len() > 256 {
            return Err("power_domains.peripherals exceeds 256 explicit bindings".into());
        }
        for (peripheral, roles) in &self.peripherals {
            if peripheral.is_empty() || !(1..=16).contains(&roles.len()) {
                return Err(
                    "power domain peripheral bindings require a nonempty ID and 1..16 roles".into(),
                );
            }
            for (role, domain) in roles {
                if !valid_power_domain_name(role) {
                    return Err(format!("invalid power domain role '{role}'"));
                }
                declared(domain)?;
            }
        }
        Ok(())
    }
}

impl TryFrom<PowerDomainsYaml> for PowerDomainsConfig {
    type Error = String;
    fn try_from(raw: PowerDomainsYaml) -> Result<Self, Self::Error> {
        let config = Self {
            rails: raw.rails,
            io_domain: raw.io_domain,
            execution_domains: raw.execution_domains,
            system_reset_scope: raw.system_reset_scope,
            latched_por: raw.latched_por,
            brown_out: raw.brown_out,
            peripherals: raw.peripherals,
        };
        config.validate()?;
        Ok(config)
    }
}
impl From<PowerDomainsConfig> for PowerDomainsYaml {
    fn from(config: PowerDomainsConfig) -> Self {
        Self {
            rails: config.rails,
            io_domain: config.io_domain,
            execution_domains: config.execution_domains,
            system_reset_scope: config.system_reset_scope,
            latched_por: config.latched_por,
            brown_out: config.brown_out,
            peripherals: config.peripherals,
        }
    }
}
