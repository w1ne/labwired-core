// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_config::{
    ChipDescriptor, CosimAdapter, DeviceDescriptor, MemoryValueDetails, MotorModelConfig,
    PeripheralConfig, SystemManifest,
};

#[test]
fn test_old_yaml_still_parses() {
    let yaml = r#"
name: "test-chip"
arch: "cortex-m3"
flash:
  base: 0x0
  size: "1MB"
ram:
  base: 0x20000000
  size: "128KB"
peripherals:
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
"#;
    let desc: ChipDescriptor = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(desc.peripherals.len(), 1);
    assert_eq!(desc.peripherals[0].id, "uart1");
    assert_eq!(desc.peripherals[0].size, None);
    assert_eq!(desc.peripherals[0].irq, None);
}

#[test]
fn test_new_fields_parse() {
    let yaml = r#"
name: "test-chip"
arch: "cortex-m3"
flash:
  base: 0x0
  size: "1MB"
ram:
  base: 0x20000000
  size: "128KB"
peripherals:
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
    size: "1KB"
    irq: 37
"#;
    let desc: ChipDescriptor = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(desc.peripherals.len(), 1);
    assert_eq!(desc.peripherals[0].id, "uart1");
    assert_eq!(desc.peripherals[0].size, Some("1KB".to_string()));
    assert_eq!(desc.peripherals[0].irq, Some(37));
}

/// `ns_alias_offset` accepts the same spellings `base_address` does and stays
/// `None` when absent, so every chip written before the key existed loads and
/// the bus never translates for it.
#[test]
fn ns_alias_offset_parses_lax_number_forms() {
    let base = r#"
name: "test-chip"
arch: "cortex-m3"
flash:
  base: 0x0
  size: "1MB"
ram:
  base: 0x20000000
  size: "128KB"
peripherals: []
"#;
    let absent: ChipDescriptor = serde_yaml::from_str(base).unwrap();
    assert_eq!(absent.ns_alias_offset, None);

    let underscored: ChipDescriptor =
        serde_yaml::from_str(&format!("{base}\nns_alias_offset: \"0x1000_0000\"\n")).unwrap();
    assert_eq!(underscored.ns_alias_offset, Some(0x1000_0000));

    let plain_hex: ChipDescriptor =
        serde_yaml::from_str(&format!("{base}\nns_alias_offset: \"0x10000000\"\n")).unwrap();
    assert_eq!(plain_hex.ns_alias_offset, Some(0x1000_0000));

    let decimal: ChipDescriptor =
        serde_yaml::from_str(&format!("{base}\nns_alias_offset: 268435456\n")).unwrap();
    assert_eq!(decimal.ns_alias_offset, Some(0x1000_0000));
}

#[test]
fn irq_accepts_bare_number() {
    let p: PeripheralConfig = serde_yaml::from_str(
        r#"
id: uart0
type: uart
base_address: 0x40002000
irq: 2
"#,
    )
    .unwrap();
    assert_eq!(p.irq, Some(2));
    assert_eq!(p.irq_controller.as_deref(), None);
}

#[test]
fn irq_accepts_controller_at_line() {
    let p: PeripheralConfig = serde_yaml::from_str(
        r#"
id: uart0
type: uart
base_address: 0x40002000
irq: nvic@2
"#,
    )
    .unwrap();
    assert_eq!(p.irq, Some(2));
    assert_eq!(p.irq_controller.as_deref(), Some("nvic"));
}

#[test]
fn irq_rejects_malformed_target() {
    let err = serde_yaml::from_str::<PeripheralConfig>(
        r#"
id: uart0
type: uart
base_address: 0x40002000
irq: nvic@
"#,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("irq"), "{msg}");
}

#[test]
fn flattened_keys_land_in_config() {
    let p: PeripheralConfig = serde_yaml::from_str(
        r#"
id: uart0
type: nrf52840_uart
base_address: 0x40002000
easyDMA: true
"#,
    )
    .unwrap();
    assert_eq!(
        p.config.get("easyDMA").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn nested_config_wins_over_flattened_key() {
    let p: PeripheralConfig = serde_yaml::from_str(
        r#"
id: uart0
type: uart
base_address: 0x4000
easyDMA: true
config:
  easyDMA: false
"#,
    )
    .unwrap();
    assert_eq!(
        p.config.get("easyDMA").and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[test]
fn system_manifest_parses_cosim_models() {
    let yaml = r#"
name: "plant-demo"
chip: "chips/stm32f103.yaml"
cosim_models:
  - id: "plant_model"
    adapter: "external_process"
    model: "./models/plant.jsonl"
    step_ns: 10000
    inputs:
      rem0_enable: "gpio.rem0"
      rem1_enable: "gpio.rem1"
    outputs:
      v_out: "scope.channel_a"
      i_out: "meter.output_current"
    config:
      protocol: "jsonl"
external_devices: []
"#;

    let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();

    assert_eq!(manifest.cosim_models.len(), 1);
    let model = &manifest.cosim_models[0];
    assert_eq!(model.id, "plant_model");
    assert_eq!(model.adapter, CosimAdapter::ExternalProcess);
    assert_eq!(model.model.as_deref(), Some("./models/plant.jsonl"));
    assert_eq!(model.step_ns, 10_000);
    assert_eq!(model.inputs["rem0_enable"], "gpio.rem0");
    assert_eq!(model.outputs["v_out"], "scope.channel_a");
    assert_eq!(
        model.config["protocol"],
        serde_yaml::Value::String("jsonl".to_string())
    );
}

#[test]
fn system_manifest_parses_the_analog_adapter() {
    let yaml = r#"
name: "rc"
chip: "chips/stm32f401.yaml"
cosim_models:
  - id: "rc_lowpass"
    adapter: "analog"
    step_ns: 100000
    inputs:
      gpio: "board.gpio.pa5"
    outputs:
      v_out: "board.analog.pa0_volts"
    config:
      netlist: "./rc.cir"
      probes:
        v_out: "v(out)"
      sources:
        gpio: "Vgpio"
external_devices: []
"#;

    let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();
    let model = &manifest.cosim_models[0];
    assert_eq!(model.adapter, CosimAdapter::Analog);
    // The in-core engine takes its circuit from `config`, not from a model
    // program, so no `model:` path is required of it.
    assert_eq!(model.model, None);
    assert!(
        manifest.validate_cosim_models().is_empty(),
        "{:?}",
        manifest.validate_cosim_models()
    );
}

#[test]
fn an_analog_model_must_declare_a_circuit_and_a_probe() {
    let yaml = r#"
name: "rc"
chip: "chips/stm32f401.yaml"
cosim_models:
  - id: "rc_lowpass"
    adapter: "analog"
    step_ns: 100000
external_devices: []
"#;

    let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();
    let issues = manifest.validate_cosim_models();
    assert!(
        issues.iter().any(|issue| issue.contains("netlist")),
        "a model with no circuit must say so, got {issues:?}"
    );
    assert!(
        issues.iter().any(|issue| issue.contains("probes")),
        "a model with no probe produces no outputs, got {issues:?}"
    );
}

#[test]
fn memory_value_details_constructor_is_externally_constructible_and_sparse() {
    let details = MemoryValueDetails::new(0x2001_0000, 1);
    assert_eq!(details.mask, None);
    assert_eq!(details.size, None);
    assert_eq!(details.node, None);

    let serialized = serde_yaml::to_string(&details).unwrap();
    assert!(
        !serialized.contains("node:"),
        "ordinary node-less details should stay sparse: {serialized}"
    );
}

#[test]
fn system_manifest_rejects_incomplete_cosim_model() {
    let yaml = r#"
name: "bad-cosim"
chip: "chips/stm32f103.yaml"
cosim_models:
  - id: ""
    adapter: "fmi"
    step_ns: 0
external_devices: []
"#;

    let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();
    let issues = manifest.validate_cosim_models();

    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("cosim_models[0].id")),
        "expected missing id validation issue, got {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("cosim_models[0].model")),
        "expected missing model validation issue, got {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("cosim_models[0].step_ns")),
        "expected invalid step validation issue, got {issues:?}"
    );
}

fn valid_dc_motor_yaml() -> &'static str {
    r#"
kind: dc
id: drive_motor
resistance_ohm: 1.2
inductance_h: 0.002
torque_constant_nm_per_a: 0.08
back_emf_constant_v_per_rad_s: 0.08
rotor_inertia_kg_m2: 0.00004
viscous_friction_nm_per_rad_s: 0.00001
supply_voltage_v: 12.0
load_torque_nm: -0.01
encoder_cpr: 1024
pwm_pin: PA8
direction_pin: PA9
brake_pin: PA10
enable_pin: PA11
encoder_a_pin: PB6
encoder_b_pin: PB7
encoder_index_pin: PB8
"#
}

#[test]
fn motor_model_parses_valid_dc_and_roundtrips_stably() {
    let model: MotorModelConfig = serde_yaml::from_str(valid_dc_motor_yaml()).unwrap();
    let MotorModelConfig::Dc(cfg) = &model else {
        panic!("expected dc motor");
    };
    assert_eq!(cfg.id, "drive_motor");
    assert_eq!(cfg.encoder_cpr, 1024);
    assert_eq!(cfg.encoder_index_pin.as_deref(), Some("PB8"));
    assert_eq!(cfg.simulation_clock_hz, 80_000_000);
    assert_eq!(cfg.fault_pin, None);
    assert!(model.validate().is_empty());

    let yaml = serde_yaml::to_string(&model).unwrap();
    assert!(yaml.starts_with("kind: dc\n"));
    assert_eq!(
        serde_yaml::from_str::<MotorModelConfig>(&yaml).unwrap(),
        model
    );
}

#[test]
fn motor_model_parses_valid_bldc_and_roundtrips_stably() {
    let yaml = r#"
kind: bldc
id: spindle
resistance_ohm: 0.35
inductance_h: 0.00018
torque_constant_nm_per_a: 0.04
back_emf_constant_v_per_rad_s: 0.04
rotor_inertia_kg_m2: 0.00002
viscous_friction_nm_per_rad_s: 0.000003
supply_voltage_v: 24.0
load_torque_nm: 0.015
encoder_cpr: 2048
pole_pairs: 7
current_limit_a: 40.0
overcurrent_trip_steps: 4
phase_a_high_pin: PA8
phase_a_low_pin: PA7
phase_b_high_pin: PA9
phase_b_low_pin: PB0
phase_c_high_pin: PA10
phase_c_low_pin: PB1
enable_pin: PB2
hall_a_pin: PC0
hall_b_pin: PC1
hall_c_pin: PC2
encoder_a_pin: PC6
encoder_b_pin: PC7
overcurrent_fault_pin: PB6
undervoltage_fault_pin: PB5
"#;
    let model: MotorModelConfig = serde_yaml::from_str(yaml).unwrap();
    let MotorModelConfig::Bldc(cfg) = &model else {
        panic!("expected bldc motor");
    };
    assert_eq!(cfg.id, "spindle");
    assert_eq!(cfg.pole_pairs, 7);
    assert_eq!(cfg.timer_name, "tim1");
    assert_eq!(cfg.encoder_index_pin, None);
    assert_eq!(cfg.simulation_clock_hz, 80_000_000);
    assert_eq!(cfg.motor_fault_pin, None);
    assert_eq!(cfg.inverter_fault_pin, None);
    assert_eq!(cfg.current_limit_a, Some(40.0));
    assert_eq!(cfg.overcurrent_trip_steps, 4);
    assert_eq!(cfg.overcurrent_fault_pin.as_deref(), Some("PB6"));
    assert_eq!(cfg.undervoltage_fault_pin.as_deref(), Some("PB5"));
    assert!(model.validate().is_empty());
    assert_eq!(
        serde_yaml::from_str::<MotorModelConfig>(&serde_yaml::to_string(&model).unwrap()).unwrap(),
        model
    );
}

#[test]
fn motor_model_bldc_accepts_non_tim1_timer_name() {
    let yaml = r#"
kind: bldc
id: spindle_tim8
resistance_ohm: 0.35
inductance_h: 0.00018
torque_constant_nm_per_a: 0.04
back_emf_constant_v_per_rad_s: 0.04
rotor_inertia_kg_m2: 0.00002
viscous_friction_nm_per_rad_s: 0.000003
supply_voltage_v: 24.0
load_torque_nm: 0.015
encoder_cpr: 2048
pole_pairs: 7
timer_name: tim8
phase_a_high_pin: PA8
phase_a_low_pin: PA7
phase_b_high_pin: PA9
phase_b_low_pin: PB0
phase_c_high_pin: PA10
phase_c_low_pin: PB1
enable_pin: PB2
hall_a_pin: PC0
hall_b_pin: PC1
hall_c_pin: PC2
encoder_a_pin: PC6
encoder_b_pin: PC7
"#;
    let model: MotorModelConfig = serde_yaml::from_str(yaml).unwrap();
    let MotorModelConfig::Bldc(cfg) = &model else {
        panic!("expected bldc motor");
    };
    assert_eq!(cfg.timer_name, "tim8");
    assert!(model.validate().is_empty());
}

#[test]
fn motor_model_bldc_rejects_blank_timer_name() {
    let yaml = r#"
kind: bldc
id: spindle
resistance_ohm: 0.35
inductance_h: 0.00018
torque_constant_nm_per_a: 0.04
back_emf_constant_v_per_rad_s: 0.04
rotor_inertia_kg_m2: 0.00002
viscous_friction_nm_per_rad_s: 0.000003
supply_voltage_v: 24.0
load_torque_nm: 0.015
encoder_cpr: 2048
pole_pairs: 7
timer_name: "   "
phase_a_high_pin: PA8
phase_a_low_pin: PA7
phase_b_high_pin: PA9
phase_b_low_pin: PB0
phase_c_high_pin: PA10
phase_c_low_pin: PB1
enable_pin: PB2
hall_a_pin: PC0
hall_b_pin: PC1
hall_c_pin: PC2
encoder_a_pin: PC6
encoder_b_pin: PC7
"#;
    let model: MotorModelConfig = serde_yaml::from_str(yaml).unwrap();
    let issues = model.validate();
    assert!(
        issues.iter().any(|issue| issue.contains("timer_name")),
        "expected blank timer_name issue, got {issues:?}"
    );
}

#[test]
fn motor_model_validation_issues_are_field_qualified() {
    let mut model: MotorModelConfig = serde_yaml::from_str(valid_dc_motor_yaml()).unwrap();
    let MotorModelConfig::Dc(dc) = &mut model else {
        unreachable!()
    };
    dc.resistance_ohm = 0.0;
    dc.rotor_inertia_kg_m2 = f64::NAN;
    dc.viscous_friction_nm_per_rad_s = -1.0;
    dc.load_torque_nm = f64::INFINITY;
    dc.encoder_cpr = 0;

    let issues = model.validate();
    for field in [
        "resistance_ohm",
        "rotor_inertia_kg_m2",
        "viscous_friction_nm_per_rad_s",
        "load_torque_nm",
        "encoder_cpr",
    ] {
        assert!(
            issues.iter().any(|issue| issue.contains(field)),
            "missing qualified issue for {field}: {issues:?}"
        );
    }
}

#[test]
fn motor_model_bldc_rejects_invalid_pole_pairs() {
    let yaml = valid_dc_motor_yaml()
        .replace("kind: dc", "kind: bldc")
        .replace("pwm_pin: PA8", "pole_pairs: 0\nphase_a_high_pin: PA8\nphase_a_low_pin: PA7\nphase_b_high_pin: PA9\nphase_b_low_pin: PB0\nphase_c_high_pin: PA10\nphase_c_low_pin: PB1\nhall_a_pin: PC0\nhall_b_pin: PC1\nhall_c_pin: PC2")
        .replace("direction_pin: PA9\n", "")
        .replace("brake_pin: PA10\n", "");
    let model: MotorModelConfig = serde_yaml::from_str(&yaml).unwrap();
    assert!(model
        .validate()
        .iter()
        .any(|issue| issue.contains("motor_models[drive_motor].pole_pairs")));
}

#[test]
fn motor_model_bldc_rejects_invalid_overcurrent_configuration() {
    let yaml = r#"
kind: bldc
id: spindle
resistance_ohm: 0.35
inductance_h: 0.00018
torque_constant_nm_per_a: 0.04
back_emf_constant_v_per_rad_s: 0.04
rotor_inertia_kg_m2: 0.00002
viscous_friction_nm_per_rad_s: 0.000003
supply_voltage_v: 24.0
load_torque_nm: 0.0
encoder_cpr: 2048
pole_pairs: 7
current_limit_a: 0.0
overcurrent_trip_steps: 0
phase_a_high_pin: PA8
phase_a_low_pin: PB13
phase_b_high_pin: PA9
phase_b_low_pin: PB14
phase_c_high_pin: PA10
phase_c_low_pin: PB15
enable_pin: PB0
hall_a_pin: PC0
hall_b_pin: PC1
hall_c_pin: PC2
encoder_a_pin: PC3
encoder_b_pin: PC4
"#;
    let model: MotorModelConfig = serde_yaml::from_str(yaml).unwrap();
    let issues = model.validate();
    assert!(issues.iter().any(|issue| issue.contains("current_limit_a")));
    assert!(issues
        .iter()
        .any(|issue| issue.contains("overcurrent_trip_steps")));
}

#[test]
fn motor_model_descriptors_define_unambiguous_required_pin_contracts() {
    let dc = DeviceDescriptor::embedded("dc-motor")
        .unwrap()
        .expect("dc descriptor");
    let dc_emit = dc.emit.expect("dc emit");
    let dc_pins: Vec<_> = dc_emit
        .config
        .iter()
        .filter_map(|entry| {
            entry
                .from_part_pin
                .as_ref()
                .map(|pins| (entry.key.as_str(), pins[0].as_str(), entry.required))
        })
        .collect();
    assert_eq!(
        dc_pins,
        vec![
            ("pwm_pin", "PWM", true),
            ("direction_pin", "DIRECTION", true),
            ("brake_pin", "BRAKE", true),
            ("enable_pin", "ENABLE", true),
            ("encoder_a_pin", "ENC_A", false),
            ("encoder_b_pin", "ENC_B", false),
            ("encoder_index_pin", "INDEX", false),
            ("fault_pin", "FAULT", false),
        ]
    );

    let bldc = DeviceDescriptor::embedded("bldc-motor")
        .unwrap()
        .expect("bldc descriptor");
    let emit = bldc.emit.expect("bldc emit");
    let required: Vec<_> = emit
        .config
        .iter()
        .filter(|entry| entry.from_part_pin.is_some() && entry.required)
        .map(|entry| entry.key.as_str())
        .collect();
    assert_eq!(
        required,
        [
            "phase_a_high_pin",
            "phase_a_low_pin",
            "phase_b_high_pin",
            "phase_b_low_pin",
            "phase_c_high_pin",
            "phase_c_low_pin",
            "enable_pin",
            "hall_a_pin",
            "hall_b_pin",
            "hall_c_pin",
        ]
    );
    assert!(emit
        .config
        .iter()
        .any(|entry| entry.key == "encoder_index_pin" && !entry.required));
    assert!(emit
        .config
        .iter()
        .any(|entry| entry.key == "motor_fault_pin" && !entry.required));
    assert!(emit
        .config
        .iter()
        .any(|entry| entry.key == "inverter_fault_pin" && !entry.required));
    assert!(emit
        .config
        .iter()
        .any(|entry| entry.key == "overcurrent_fault_pin" && !entry.required));
    assert!(emit
        .config
        .iter()
        .any(|entry| entry.key == "undervoltage_fault_pin" && !entry.required));
}

#[test]
fn canonical_external_motor_device_resolves_through_typed_boundary() {
    let yaml = format!(
        r#"
name: motor-demo
chip: inline
external_devices:
  - id: drive_motor
    type: dc-motor
    connection: gpio
    config:
{}
"#,
        valid_dc_motor_yaml()
            .lines()
            .filter(|line| !line.starts_with("kind:") && !line.starts_with("id:"))
            .map(|line| format!("      {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let manifest: SystemManifest = serde_yaml::from_str(&yaml).unwrap();
    let models = manifest.resolved_motor_models().unwrap();
    assert_eq!(models.len(), 1);
    assert!(matches!(
        &models[0],
        MotorModelConfig::Dc(config) if config.id == "drive_motor"
    ));
}

#[test]
fn external_motor_rejects_reserved_identity_keys_for_dc_and_bldc() {
    for (device_type, reserved_key) in [("dc-motor", "id"), ("bldc-motor", "kind")] {
        let yaml = format!(
            r#"
id: authoritative
type: {device_type}
connection: gpio
config:
  {reserved_key}: attacker-controlled
"#
        );
        let device: labwired_config::ExternalDevice = serde_yaml::from_str(&yaml).unwrap();
        let error = MotorModelConfig::from_external_device(&device)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&format!(
                "external_devices[authoritative].config.{reserved_key}"
            )),
            "{error}"
        );
    }
}

#[test]
fn resolved_motor_models_rejects_duplicate_ids_with_source_qualified_errors() {
    let typed = serde_yaml::to_string(
        &serde_yaml::from_str::<MotorModelConfig>(valid_dc_motor_yaml()).unwrap(),
    )
    .unwrap();
    let duplicate_top_level = format!(
        "name: duplicate\nchip: inline\nmotor_models:\n{}{}\n",
        typed
            .lines()
            .map(|line| format!("  - {line}\n"))
            .collect::<String>()
            .replace("\n  - ", "\n    "),
        typed
            .lines()
            .map(|line| format!("  - {line}\n"))
            .collect::<String>()
            .replace("\n  - ", "\n    ")
    );
    let manifest: SystemManifest = serde_yaml::from_str(&duplicate_top_level).unwrap();
    let error = manifest.resolved_motor_models().unwrap_err().to_string();
    assert!(error.contains("motor_models[1].id"), "{error}");
    assert!(error.contains("motor_models[0].id"), "{error}");

    let external_entry = valid_dc_motor_yaml()
        .lines()
        .filter(|line| !line.starts_with("kind:") && !line.starts_with("id:"))
        .map(|line| format!("        {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let duplicate_external = format!(
        r#"
name: duplicate
chip: inline
external_devices:
  - id: drive_motor
    type: dc-motor
    connection: gpio
    config:
{external_entry}
  - id: drive_motor
    type: dc-motor
    connection: gpio
    config:
{external_entry}
"#
    );
    let manifest: SystemManifest = serde_yaml::from_str(&duplicate_external).unwrap();
    let error = manifest.resolved_motor_models().unwrap_err().to_string();
    assert!(error.contains("external_devices[1].id"), "{error}");
    assert!(error.contains("external_devices[0].id"), "{error}");
}

#[test]
fn resolved_motor_models_rejects_cross_source_duplicate_ids() {
    let typed = valid_dc_motor_yaml()
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let external = valid_dc_motor_yaml()
        .lines()
        .filter(|line| !line.starts_with("kind:") && !line.starts_with("id:"))
        .map(|line| format!("        {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let yaml = format!(
        r#"
name: duplicate
chip: inline
motor_models:
  -
{typed}
external_devices:
  - id: drive_motor
    type: dc-motor
    connection: gpio
    config:
{external}
"#
    );
    let manifest: SystemManifest = serde_yaml::from_str(&yaml).unwrap();
    let error = manifest.resolved_motor_models().unwrap_err().to_string();
    assert!(error.contains("external_devices[0].id"), "{error}");
    assert!(error.contains("motor_models[0].id"), "{error}");
}

#[test]
fn motor_model_validation_rejects_blank_identity_and_bindings() {
    let mut model: MotorModelConfig = serde_yaml::from_str(valid_dc_motor_yaml()).unwrap();
    let MotorModelConfig::Dc(config) = &mut model else {
        unreachable!()
    };
    config.id = " ".to_owned();
    config.pwm_pin = "\t".to_owned();
    config.encoder_b_pin = Some(" ".to_owned());
    config.encoder_index_pin = Some(" ".to_owned());
    let issues = model.validate();
    for field in ["id", "pwm_pin", "encoder_b_pin", "encoder_index_pin"] {
        assert!(
            issues.iter().any(|issue| issue.contains(field)),
            "missing {field}: {issues:?}"
        );
    }
}

#[test]
fn memory_value_details_public_fields_remain_struct_literal_constructible() {
    // This is compiled as a downstream crate. Keep the public struct shape
    // usable by callers that construct a memory assertion directly.
    let details = MemoryValueDetails {
        address: 0x2001_0000,
        expected_value: 1,
        mask: None,
        size: None,
        node: None,
    };

    let serialized = serde_yaml::to_string(&details).unwrap();
    assert!(
        !serialized.contains("node:"),
        "ordinary node-less details should stay sparse: {serialized}"
    );
}

#[test]
fn chip_yaml_include_merges_peripherals_and_fills_scalar_gaps() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("common.yaml"),
        r#"
name: nrf52-common
arch: arm
cpu_hz: 64000000
flash:
  base: 0x0
  size: "1MB"
ram:
  base: 0x20000000
  size: "256KB"
peripherals:
  - id: uart0
    type: uart
    base_address: 0x40002000
    irq: 1
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("child.yaml"),
        r#"
include: common.yaml
name: nrf52840
peripherals:
  - id: uart0
    type: uart
    base_address: 0x40002000
    irq: 2
    size: 4096
  - id: uart1
    type: uart
    base_address: 0x40028000
    irq: 3
"#,
    )
    .unwrap();

    let chip = ChipDescriptor::from_file(dir.path().join("child.yaml")).unwrap();
    assert_eq!(chip.name, "nrf52840");
    assert_eq!(chip.cpu_hz, 64_000_000);
    assert_eq!(chip.arch, labwired_config::Arch::Arm);
    assert_eq!(chip.peripherals.len(), 2);
    let uart0 = chip
        .peripherals
        .iter()
        .find(|p| p.id == "uart0")
        .expect("uart0 from include, irq overridden locally");
    assert_eq!(uart0.irq, Some(2));
    assert_eq!(
        uart0.size.as_deref(),
        Some("4096"),
        "numeric YAML size must parse without a text round-trip"
    );
    let uart1 = chip
        .peripherals
        .iter()
        .find(|p| p.id == "uart1")
        .expect("uart1 added by child");
    assert_eq!(uart1.irq, Some(3));
}

#[test]
fn chip_yaml_missing_include_errors_with_the_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("child.yaml"),
        r#"
include: does-not-exist.yaml
name: nrf52840
arch: arm
flash:
  base: 0x0
  size: "1MB"
ram:
  base: 0x20000000
  size: "256KB"
peripherals: []
"#,
    )
    .unwrap();

    let err = ChipDescriptor::from_file(dir.path().join("child.yaml")).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("does-not-exist.yaml"),
        "missing include must name the path; got {msg}"
    );
}

#[test]
fn chip_yaml_include_cycle_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.yaml"),
        r#"
include: b.yaml
name: chip-a
arch: arm
flash:
  base: 0x0
  size: "1MB"
ram:
  base: 0x20000000
  size: "256KB"
peripherals: []
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("b.yaml"),
        r#"
include: a.yaml
name: chip-b
arch: arm
flash:
  base: 0x0
  size: "1MB"
ram:
  base: 0x20000000
  size: "256KB"
peripherals: []
"#,
    )
    .unwrap();

    let err = ChipDescriptor::from_file(dir.path().join("a.yaml")).unwrap_err();
    let msg = format!("{err:#}").to_ascii_lowercase();
    assert!(msg.contains("cycle"), "cycle must be named; got {msg}");
}

#[test]
fn wiring_sugar_child_fixture_loads_via_from_file() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wiring-sugar-child.yaml");
    let chip = ChipDescriptor::from_file(&path).unwrap();
    assert_eq!(chip.name, "nrf52840");
    assert_eq!(chip.cpu_hz, 64_000_000);
    assert_eq!(chip.arch, labwired_config::Arch::Arm);
    assert_eq!(chip.peripherals.len(), 2);
    let uart0 = chip
        .peripherals
        .iter()
        .find(|p| p.id == "uart0")
        .expect("uart0 from include, irq overridden locally");
    assert_eq!(uart0.irq, Some(2));
    assert_eq!(uart0.irq_controller.as_deref(), Some("nvic"));
    assert_eq!(
        uart0.config.get("easyDMA").and_then(|v| v.as_bool()),
        Some(true)
    );
    let uart1 = chip
        .peripherals
        .iter()
        .find(|p| p.id == "uart1")
        .expect("uart1 added by child");
    assert_eq!(uart1.irq, Some(3));
}
