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
    /// Dry (Coulomb) bearing and brush friction, N·m: a constant torque
    /// opposing rotation that also holds a resting shaft (stiction). Without
    /// it a shaft that only viscous drag slows coasts for many seconds after
    /// its drive is cut. Absent = 0, the frictionless shaft of older configs.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub coulomb_friction_nm: f64,
    /// Bridge supply. Required (and positive) for `armature: bridge`; must be
    /// absent for `armature: circuit`, where the analog netlist's own rail
    /// supplies the winding and a second, fixed figure here would disagree
    /// with it.
    #[serde(default)]
    pub supply_voltage_v: f64,
    pub load_torque_nm: f64,
    pub encoder_cpr: u32,
    #[serde(default = "default_motor_simulation_clock_hz")]
    pub simulation_clock_hz: u64,
    /// Who drives the winding. Absent = [`MotorArmature::Bridge`], the plant
    /// integrating its own current from pin levels as it always has.
    #[serde(default, skip_serializing_if = "MotorArmature::is_bridge")]
    pub armature: MotorArmature,
    /// Speed / enable input: the low-side switch gate, an H-bridge's ENA or
    /// PWMA. A rail label (`VCC`, `3V3`, ...) means "jumpered full on".
    /// Required for `armature: bridge`; must be absent for `armature: circuit`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pwm_pin: String,
    /// Single-pin direction (high = forward). Exactly one of `direction_pin` or
    /// the `in1_pin`/`in2_pin` pair selects direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction_pin: Option<String>,
    /// Terminal drive: the two bridge inputs that set the motor terminals
    /// (H-bridge IN1/IN2, AIN1/AIN2). Truth table: `10` forward, `01` reverse,
    /// `11` brake, `00` coast. The driver chip itself is not a core device;
    /// the canvas compiler follows the driver hop and lands its MCU pads here.
    /// When `pwm_pin` names the same pad as one of these (PWM applied to an
    /// IN pin), that input is the PWM signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in1_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in2_pin: Option<String>,
    /// What the bridge does with both inputs low (`00`). Driver-specific:
    /// L298N = `brake` (fast motor stop), TB6612FNG / DRV8833 = `coast`
    /// (outputs high-impedance). Absent = `coast`. `11` always brakes.
    /// Terminal drive only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub both_inputs_low: Option<BothInputsLow>,
    /// Brake input. Absent = no brake input is modeled; at zero duty the plant
    /// still decays current through the bridge (idle braking), which is what an
    /// H-bridge at zero drive does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brake_pin: Option<String>,
    /// Enable input. Absent = always enabled. An H-bridge drives its load
    /// through the PWM pin, so binding "enable" to that same pin would gate the
    /// plant at PWM rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_pin: Option<String>,
    /// Optional: unused encoder outputs may be left unwired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_a_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_b_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_index_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault_pin: Option<String>,
    /// Timer channel that drives the PWM pin (STM32-class hardware PWM). When
    /// present the plant reads the timer's duty instead of the pin latch — a
    /// timer-driven pin is in alternate-function mode and its ODR never moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer_channel: Option<u8>,
}

/// Who drives a brushed DC motor's winding.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MotorArmature {
    /// The plant integrates its own winding current from the bridge state its
    /// control pins select, at `supply_voltage_v`.
    #[default]
    Bridge,
    /// The winding is an element of an analog co-simulation netlist (series
    /// R + L + a back-EMF source, wired to whatever switch, diode and rail the
    /// circuit has). The netlist solves the current and routes it in on
    /// `board.motor.<id>.current_a`; the plant turns it into torque, integrates
    /// the shaft, and publishes `board.motor.<id>.back_emf_v` back to the
    /// netlist. No pin, bridge or supply setting applies.
    Circuit,
}

impl MotorArmature {
    pub fn is_bridge(&self) -> bool {
        *self == Self::Bridge
    }
}

/// Bridge behaviour with both terminal-drive inputs low.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BothInputsLow {
    Coast,
    Brake,
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
    /// Dry (Coulomb) bearing and brush friction, N·m: a constant torque
    /// opposing rotation that also holds a resting shaft (stiction). Without
    /// it a shaft that only viscous drag slows coasts for many seconds after
    /// its drive is cut. Absent = 0, the frictionless shaft of older configs.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub coulomb_friction_nm: f64,
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

pub(crate) fn default_motor_speed_unbounded_rpm() -> f64 {
    f64::MAX
}

pub(crate) fn motor_speed_is_unbounded(rpm: &f64) -> bool {
    *rpm == f64::MAX
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
                let circuit = config.armature == MotorArmature::Circuit;
                validate_motor_common(
                    &config.id,
                    config.resistance_ohm,
                    config.inductance_h,
                    config.torque_constant_nm_per_a,
                    config.back_emf_constant_v_per_rad_s,
                    config.rotor_inertia_kg_m2,
                    config.viscous_friction_nm_per_rad_s,
                    config.coulomb_friction_nm,
                    (!circuit).then_some(config.supply_voltage_v),
                    config.load_torque_nm,
                    config.encoder_cpr,
                    &mut issues,
                );
                if circuit {
                    validate_circuit_armature(config, &mut issues);
                    if config.simulation_clock_hz == 0 {
                        issues.push(format!(
                            "motor_models[{}].simulation_clock_hz must be greater than zero",
                            config.id
                        ));
                    }
                    return issues;
                }
                validate_required_motor_pins(
                    &config.id,
                    [("pwm_pin", config.pwm_pin.as_str())],
                    None,
                    &mut issues,
                );
                for (field, pin) in [
                    ("direction_pin", config.direction_pin.as_deref()),
                    ("in1_pin", config.in1_pin.as_deref()),
                    ("in2_pin", config.in2_pin.as_deref()),
                ] {
                    validate_optional_motor_pin(&config.id, field, pin, &mut issues);
                }
                match (
                    config.direction_pin.is_some(),
                    config.in1_pin.is_some(),
                    config.in2_pin.is_some(),
                ) {
                    (true, false, false) | (false, true, true) => {}
                    (false, false, false) => issues.push(format!(
                        "motor_models[{}] needs a direction source: direction_pin, or in1_pin and in2_pin",
                        config.id
                    )),
                    (true, _, _) => issues.push(format!(
                        "motor_models[{}].direction_pin and in1_pin/in2_pin are mutually exclusive",
                        config.id
                    )),
                    (false, _, _) => issues.push(format!(
                        "motor_models[{}].in1_pin and in2_pin must be wired together",
                        config.id
                    )),
                }
                if config.both_inputs_low.is_some() && config.in1_pin.is_none() {
                    issues.push(format!(
                        "motor_models[{}].both_inputs_low applies only to in1_pin/in2_pin terminal drive",
                        config.id
                    ));
                }
                if config.encoder_a_pin.is_some() != config.encoder_b_pin.is_some() {
                    issues.push(format!(
                        "motor_models[{}]: encoder A and B must be wired together, or both absent",
                        config.id
                    ));
                }
                if config.encoder_index_pin.is_some()
                    && !(config.encoder_a_pin.is_some() && config.encoder_b_pin.is_some())
                {
                    issues.push(format!(
                        "motor_models[{}]: encoder index requires both encoder A and B",
                        config.id
                    ));
                }
                if config.timer_name.is_some() != config.timer_channel.is_some() {
                    issues.push(format!(
                        "motor_models[{}].timer_name and timer_channel must be set together",
                        config.id
                    ));
                }
                validate_optional_motor_pin(
                    &config.id,
                    "brake_pin",
                    config.brake_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "enable_pin",
                    config.enable_pin.as_deref(),
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
                if let Some(channel) = config.timer_channel {
                    if !(1..=4).contains(&channel) {
                        issues.push(format!(
                            "motor_models[{}].timer_channel {channel} is outside the timer's 1..=4 channels",
                            config.id
                        ));
                    }
                }
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
                    config.coulomb_friction_nm,
                    Some(config.supply_voltage_v),
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

/// A circuit-driven winding takes nothing from pins or a bridge supply. Any
/// such field present is refused rather than ignored: it would read as if it
/// drove the motor while the netlist actually does.
fn validate_circuit_armature(config: &BrushedMotorConfig, issues: &mut Vec<String>) {
    let id = &config.id;
    let mut present: Vec<&str> = Vec::new();
    if !config.pwm_pin.is_empty() {
        present.push("pwm_pin");
    }
    for (field, set) in [
        ("direction_pin", config.direction_pin.is_some()),
        ("in1_pin", config.in1_pin.is_some()),
        ("in2_pin", config.in2_pin.is_some()),
        ("both_inputs_low", config.both_inputs_low.is_some()),
        ("brake_pin", config.brake_pin.is_some()),
        ("enable_pin", config.enable_pin.is_some()),
        ("timer_name", config.timer_name.is_some()),
        ("timer_channel", config.timer_channel.is_some()),
    ] {
        if set {
            present.push(field);
        }
    }
    if config.supply_voltage_v != 0.0 {
        present.push("supply_voltage_v");
    }
    for field in present {
        issues.push(format!(
            "motor_models[{id}].{field} does not apply to armature: circuit (the analog netlist \
             drives the winding); remove it"
        ));
    }
    for (field, pin) in [
        ("encoder_a_pin", config.encoder_a_pin.as_deref()),
        ("encoder_b_pin", config.encoder_b_pin.as_deref()),
        ("encoder_index_pin", config.encoder_index_pin.as_deref()),
        ("fault_pin", config.fault_pin.as_deref()),
    ] {
        validate_optional_motor_pin(id, field, pin, issues);
    }
    if config.encoder_a_pin.is_some() != config.encoder_b_pin.is_some() {
        issues.push(format!(
            "motor_models[{id}]: encoder A and B must be wired together, or both absent"
        ));
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

fn is_zero(value: &f64) -> bool {
    *value == 0.0
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
    coulomb_friction_nm: f64,
    // `None`: the motor has no supply of its own (a circuit-driven winding).
    supply_voltage_v: Option<f64>,
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
    ]
    .into_iter()
    .chain(supply_voltage_v.map(|v| ("supply_voltage_v", v)))
    {
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
    if !coulomb_friction_nm.is_finite() || coulomb_friction_nm < 0.0 {
        issues.push(format!(
            "{} must be finite and non-negative",
            path("coulomb_friction_nm")
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
