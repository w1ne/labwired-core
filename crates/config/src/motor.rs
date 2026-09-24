// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#![allow(dead_code)]
use crate::*;

/// Typed, unit-explicit configuration for deterministic motor plants.
///
/// These DTOs live in `labwired-config` because the physics crate already
/// depends on this crate. The engine converts them to its `*MotorParams` types
/// at the construction boundary; raw YAML maps never reach the plant models.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MotorModelConfig {
    Dc(Box<BrushedMotorConfig>),
    Bldc(Box<BldcMotorConfig>),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BrushedMotorConfig {
    pub id: String,
    pub resistance_ohm: f64,
    pub inductance_h: f64,
    pub torque_constant_nm_per_a: f64,
    pub back_emf_constant_v_per_rad_s: f64,
    pub rotor_inertia_kg_m2: f64,
    pub viscous_friction_nm_per_rad_s: f64,
    pub supply_voltage_v: f64,
    pub load_torque_nm: f64,
    pub encoder_cpr: u32,
    #[serde(default = "default_motor_simulation_clock_hz")]
    pub simulation_clock_hz: u64,
    pub pwm_pin: String,
    pub direction_pin: String,
    pub brake_pin: String,
    pub enable_pin: String,
    /// Optional: unused encoder outputs may be left unwired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_a_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_b_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_index_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault_pin: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BldcMotorConfig {
    pub id: String,
    pub resistance_ohm: f64,
    pub inductance_h: f64,
    pub torque_constant_nm_per_a: f64,
    pub back_emf_constant_v_per_rad_s: f64,
    pub rotor_inertia_kg_m2: f64,
    pub viscous_friction_nm_per_rad_s: f64,
    pub supply_voltage_v: f64,
    pub load_torque_nm: f64,
    pub encoder_cpr: u32,
    pub pole_pairs: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_limit_a: Option<f64>,
    #[serde(default = "default_overcurrent_trip_steps")]
    pub overcurrent_trip_steps: u32,
    #[serde(default = "default_motor_simulation_clock_hz")]
    pub simulation_clock_hz: u64,
    /// Chip-descriptor peripheral name for the advanced timer that owns the
    /// six complementary PWM legs (default `tim1` for STM32 advanced timers).
    #[serde(default = "default_bldc_timer_name")]
    pub timer_name: String,
    pub phase_a_high_pin: String,
    pub phase_a_low_pin: String,
    pub phase_b_high_pin: String,
    pub phase_b_low_pin: String,
    pub phase_c_high_pin: String,
    pub phase_c_low_pin: String,
    pub enable_pin: String,
    pub hall_a_pin: String,
    pub hall_b_pin: String,
    pub hall_c_pin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_a_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_b_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_index_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motor_fault_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverter_fault_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overcurrent_fault_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undervoltage_fault_pin: Option<String>,
}

pub(crate) fn default_motor_simulation_clock_hz() -> u64 {
    80_000_000
}

pub(crate) fn default_bldc_timer_name() -> String {
    "tim1".to_owned()
}

pub(crate) fn default_overcurrent_trip_steps() -> u32 {
    3
}

impl MotorModelConfig {
    /// Converts the canonical `external_devices` representation at the loader
    /// boundary into the typed plant DTO consumed by the engine.
    pub fn from_external_device(device: &ExternalDevice) -> Result<Option<Self>> {
        let kind = match device.r#type.as_str() {
            "dc-motor" | "dc_motor" => "dc",
            "bldc-motor" | "bldc_motor" => "bldc",
            _ => return Ok(None),
        };
        for reserved in ["kind", "id"] {
            if device.config.contains_key(reserved) {
                return Err(anyhow::anyhow!(
                    "external_devices[{}].config.{reserved} is reserved; motor identity and type come from the external device",
                    device.id
                ));
            }
        }
        let mut mapping = serde_yaml::Mapping::new();
        mapping.insert(
            serde_yaml::Value::String("kind".to_owned()),
            serde_yaml::Value::String(kind.to_owned()),
        );
        mapping.insert(
            serde_yaml::Value::String("id".to_owned()),
            serde_yaml::Value::String(device.id.clone()),
        );
        for (key, value) in &device.config {
            mapping.insert(serde_yaml::Value::String(key.clone()), value.clone());
        }
        let config: Self = serde_yaml::from_value(serde_yaml::Value::Mapping(mapping))
            .with_context(|| format!("invalid {} motor config '{}'", kind, device.id))?;
        let issues = config.validate();
        if issues.is_empty() {
            Ok(Some(config))
        } else {
            Err(anyhow::anyhow!(issues.join("; ")))
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Dc(config) => &config.id,
            Self::Bldc(config) => &config.id,
        }
    }

    /// Returns every configuration issue with a stable, field-qualified path.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        match self {
            Self::Dc(config) => {
                validate_motor_common(
                    &config.id,
                    config.resistance_ohm,
                    config.inductance_h,
                    config.torque_constant_nm_per_a,
                    config.back_emf_constant_v_per_rad_s,
                    config.rotor_inertia_kg_m2,
                    config.viscous_friction_nm_per_rad_s,
                    config.supply_voltage_v,
                    config.load_torque_nm,
                    config.encoder_cpr,
                    &mut issues,
                );
                validate_required_motor_pins(
                    &config.id,
                    [
                        ("pwm_pin", config.pwm_pin.as_str()),
                        ("direction_pin", config.direction_pin.as_str()),
                        ("brake_pin", config.brake_pin.as_str()),
                        ("enable_pin", config.enable_pin.as_str()),
                    ],
                    None,
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "encoder_a_pin",
                    config.encoder_a_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "encoder_b_pin",
                    config.encoder_b_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "encoder_index_pin",
                    config.encoder_index_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "fault_pin",
                    config.fault_pin.as_deref(),
                    &mut issues,
                );
                if config.simulation_clock_hz == 0 {
                    issues.push(format!(
                        "motor_models[{}].simulation_clock_hz must be greater than zero",
                        config.id
                    ));
                }
            }
            Self::Bldc(config) => {
                validate_motor_common(
                    &config.id,
                    config.resistance_ohm,
                    config.inductance_h,
                    config.torque_constant_nm_per_a,
                    config.back_emf_constant_v_per_rad_s,
                    config.rotor_inertia_kg_m2,
                    config.viscous_friction_nm_per_rad_s,
                    config.supply_voltage_v,
                    config.load_torque_nm,
                    config.encoder_cpr,
                    &mut issues,
                );
                if config.pole_pairs == 0 {
                    issues.push(format!(
                        "motor_models[{}].pole_pairs must be between 1 and 255 inclusive",
                        config.id
                    ));
                }
                if config.timer_name.trim().is_empty() {
                    issues.push(format!(
                        "motor_models[{}].timer_name must be nonblank",
                        config.id
                    ));
                }
                if config
                    .current_limit_a
                    .is_some_and(|limit| !limit.is_finite() || limit <= 0.0)
                {
                    issues.push(format!(
                        "motor_models[{}].current_limit_a must be finite and greater than zero",
                        config.id
                    ));
                }
                if config.current_limit_a.is_some() && config.overcurrent_trip_steps == 0 {
                    issues.push(format!(
                        "motor_models[{}].overcurrent_trip_steps must be greater than zero",
                        config.id
                    ));
                }
                validate_required_motor_pins(
                    &config.id,
                    [
                        ("phase_a_high_pin", config.phase_a_high_pin.as_str()),
                        ("phase_a_low_pin", config.phase_a_low_pin.as_str()),
                        ("phase_b_high_pin", config.phase_b_high_pin.as_str()),
                        ("phase_b_low_pin", config.phase_b_low_pin.as_str()),
                        ("phase_c_high_pin", config.phase_c_high_pin.as_str()),
                        ("phase_c_low_pin", config.phase_c_low_pin.as_str()),
                        ("enable_pin", config.enable_pin.as_str()),
                        ("hall_a_pin", config.hall_a_pin.as_str()),
                        ("hall_b_pin", config.hall_b_pin.as_str()),
                        ("hall_c_pin", config.hall_c_pin.as_str()),
                    ],
                    None,
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "encoder_a_pin",
                    config.encoder_a_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "encoder_b_pin",
                    config.encoder_b_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "encoder_index_pin",
                    config.encoder_index_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "motor_fault_pin",
                    config.motor_fault_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "overcurrent_fault_pin",
                    config.overcurrent_fault_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "undervoltage_fault_pin",
                    config.undervoltage_fault_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "inverter_fault_pin",
                    config.inverter_fault_pin.as_deref(),
                    &mut issues,
                );
                if config.simulation_clock_hz == 0 {
                    issues.push(format!(
                        "motor_models[{}].simulation_clock_hz must be greater than zero",
                        config.id
                    ));
                }
            }
        }
        issues
    }
}

pub(crate) fn validate_optional_motor_pin(
    id: &str,
    field: &str,
    pin: Option<&str>,
    issues: &mut Vec<String>,
) {
    if pin.is_some_and(|pin| pin.trim().is_empty()) {
        issues.push(format!(
            "motor_models[{id}].{field} must be nonblank when present"
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_motor_common(
    id: &str,
    resistance_ohm: f64,
    inductance_h: f64,
    torque_constant_nm_per_a: f64,
    back_emf_constant_v_per_rad_s: f64,
    rotor_inertia_kg_m2: f64,
    viscous_friction_nm_per_rad_s: f64,
    supply_voltage_v: f64,
    load_torque_nm: f64,
    encoder_cpr: u32,
    issues: &mut Vec<String>,
) {
    let path = |field: &str| format!("motor_models[{id}].{field}");
    if id.trim().is_empty() {
        issues.push(format!("{} must be nonblank", path("id")));
    }
    for (field, value) in [
        ("resistance_ohm", resistance_ohm),
        ("inductance_h", inductance_h),
        ("torque_constant_nm_per_a", torque_constant_nm_per_a),
        (
            "back_emf_constant_v_per_rad_s",
            back_emf_constant_v_per_rad_s,
        ),
        ("rotor_inertia_kg_m2", rotor_inertia_kg_m2),
        ("supply_voltage_v", supply_voltage_v),
    ] {
        if !value.is_finite() || value <= 0.0 {
            issues.push(format!(
                "{} must be finite and greater than zero",
                path(field)
            ));
        }
    }
    if !viscous_friction_nm_per_rad_s.is_finite() || viscous_friction_nm_per_rad_s < 0.0 {
        issues.push(format!(
            "{} must be finite and non-negative",
            path("viscous_friction_nm_per_rad_s")
        ));
    }
    if !load_torque_nm.is_finite() {
        issues.push(format!("{} must be finite", path("load_torque_nm")));
    }
    if !(1..=1_000_000).contains(&encoder_cpr) {
        issues.push(format!(
            "{} must be between 1 and 1000000 inclusive",
            path("encoder_cpr")
        ));
    }
}

pub(crate) fn validate_required_motor_pins<'a>(
    id: &str,
    pins: impl IntoIterator<Item = (&'a str, &'a str)>,
    encoder_index_pin: Option<&str>,
    issues: &mut Vec<String>,
) {
    for (field, pin) in pins {
        if pin.trim().is_empty() {
            issues.push(format!("motor_models[{id}].{field} must be nonblank"));
        }
    }
    if encoder_index_pin.is_some_and(|pin| pin.trim().is_empty()) {
        issues.push(format!(
            "motor_models[{id}].encoder_index_pin must be nonblank when present"
        ));
    }
}
