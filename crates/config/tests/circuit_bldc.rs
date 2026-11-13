//! Three-lead circuit BLDC contracts. The netlist owns voltage, winding current
//! and inverter switching; no timer or synthetic Hall pins are required.
use labwired_config::MotorModelConfig;

const CIRCUIT: &str = "kind: bldc\nid: M1\narmature: circuit\nresistance_ohm: 0.35\ninductance_h: 0.00018\ntorque_constant_nm_per_a: 0.04\nback_emf_constant_v_per_rad_s: 0.04\nrotor_inertia_kg_m2: 0.0000001\nviscous_friction_nm_per_rad_s: 0.000003\nload_torque_nm: 0\nencoder_cpr: 2048\npole_pairs: 7\ncurrent_limit_a: 40\n";

#[test]
fn three_phase_circuit_requires_no_ideal_inverter_or_sensor_pins() {
    let model: MotorModelConfig = serde_yaml::from_str(CIRCUIT).unwrap();
    assert!(model.validate().is_empty(), "{:?}", model.validate());
    let text = serde_yaml::to_string(&model).unwrap();
    assert!(text.contains("armature: circuit"));
    for absent in [
        "timer_name",
        "supply_voltage_v",
        "phase_a_high_pin",
        "hall_a_pin",
        "enable_pin",
    ] {
        assert!(!text.contains(absent), "{text}");
    }
    let restored: MotorModelConfig = serde_yaml::from_str(&text).unwrap();
    assert_eq!(restored, model);
}

#[test]
fn circuit_rejects_ignored_drive_sensor_and_protection_configuration() {
    for (field, value) in [
        ("phase_a_high_pin", "PA8"),
        ("phase_a_low_pin", "PB13"),
        ("phase_b_high_pin", "PA9"),
        ("phase_b_low_pin", "PB14"),
        ("phase_c_high_pin", "PA10"),
        ("phase_c_low_pin", "PB15"),
        ("enable_pin", "PA1"),
        ("timer_name", "tim1"),
        ("hall_a_pin", "PA0"),
        ("hall_b_pin", "PA1"),
        ("hall_c_pin", "PA2"),
        ("motor_fault_pin", "PA3"),
        ("inverter_fault_pin", "PA4"),
        ("overcurrent_fault_pin", "PA5"),
        ("undervoltage_fault_pin", "PA6"),
        ("supply_voltage_v", "24"),
    ] {
        let model: MotorModelConfig =
            serde_yaml::from_str(&format!("{CIRCUIT}{field}: {value}\n")).unwrap();
        let issues = model.validate();
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains(field) && issue.contains("circuit")),
            "{field}: {issues:?}"
        );
    }
}

#[test]
fn circuit_requires_energy_consistent_phase_peak_constants() {
    let model: MotorModelConfig = serde_yaml::from_str(&CIRCUIT.replace(
        "torque_constant_nm_per_a: 0.04",
        "torque_constant_nm_per_a: 0.08",
    ))
    .unwrap();
    assert!(model
        .validate()
        .iter()
        .any(|issue| issue.contains("phase-peak") && issue.contains("torque_constant")));
}

#[test]
fn circuit_encoder_is_optional_but_incomplete_wiring_is_rejected() {
    let complete: MotorModelConfig = serde_yaml::from_str(&format!(
        "{CIRCUIT}encoder_a_pin: PA0\nencoder_b_pin: PA1\nencoder_index_pin: PA2\n"
    ))
    .unwrap();
    assert!(complete.validate().is_empty());
    for partial in ["encoder_a_pin: PA0\n", "encoder_index_pin: PA2\n"] {
        let model: MotorModelConfig = serde_yaml::from_str(&format!("{CIRCUIT}{partial}")).unwrap();
        assert!(!model.validate().is_empty(), "{partial}");
    }
}

#[test]
fn circuit_motor_ids_cannot_hide_route_segments() {
    for id in ["M.1", "M1.phase_a", "a/b", "M 1", ""] {
        let model: MotorModelConfig =
            serde_yaml::from_str(&CIRCUIT.replace("id: M1", &format!("id: \"{id}\""))).unwrap();
        assert!(
            model
                .validate()
                .iter()
                .any(|issue| issue.contains("[A-Za-z0-9_-]+")),
            "{id:?}"
        );
    }
}
