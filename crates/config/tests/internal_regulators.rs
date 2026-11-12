use labwired_config::{
    validate_internal_regulators, ChipDescriptor, InternalRegulatorConfig, PowerDomainsConfig,
};
const REGULATOR:&str="id: converter
kind: buck
control_peripheral: witness
input_domain: input
enable_domain: enable
pins: {switch_output: [LX], feedback: [FB], ground: [PGND]}
initial: {enabled: true, target_voltage_v: 1.1, current_limit_a: 0.24, rated_load_a: 0.2}
model: {max_duty: 1, quiescent_current_a: 0, output_resistance_ohms: 0.01, efficiency: 1, open_loop_gain: 100000}
";
fn domains() -> PowerDomainsConfig {
    serde_yaml::from_str(
        "rails:
  input: {nominal_v: 3.3, min_v: 2.7, max_v: 5.5, pins: [VIN]}
  enable: {nominal_v: 3.3, min_v: 3.135, max_v: 3.63, pins: [AVDD]}
io_domain: enable
execution_domains: [input]
system_reset_scope: core_only
",
    )
    .unwrap()
}
#[test]
fn explicit_model_round_trips_and_keeps_independent_current_and_voltage_limits() {
    let mut value: InternalRegulatorConfig = serde_yaml::from_str(REGULATOR).unwrap();
    value.initial.rated_load_a = 2.0;
    value.initial.current_limit_a = 0.1;
    value.initial.target_voltage_v = 6.0;
    validate_internal_regulators(&[value.clone()], Some(&domains())).unwrap();
    let round: InternalRegulatorConfig =
        serde_yaml::from_str(&serde_yaml::to_string(&value).unwrap()).unwrap();
    assert_eq!(round, value);
}
#[test]
fn closed_fields_and_typed_labels_reject_null_numeric_or_unknown_keys() {
    for (old, new) in [
        ("id: converter", "id: null"),
        ("control_peripheral: witness", "control_peripheral: 123"),
        ("input_domain: input", "input_domain: null"),
        ("[LX]", "[null]"),
        ("[FB]", "[123]"),
        ("kind: buck", "kind: ldo"),
        ("enabled: true", "enabled: null"),
        ("max_duty: 1", "max_duty: 0"),
        ("efficiency: 1", "efficiency: 1.1"),
        ("open_loop_gain: 100000", "open_loop_gain: .nan"),
        (
            "output_resistance_ohms: 0.01",
            "output_resistance_ohms: 1e-320",
        ),
        ("rated_load_a: 0.2", "rated_load_a: .inf"),
    ] {
        assert!(
            serde_yaml::from_str::<InternalRegulatorConfig>(&REGULATOR.replace(old, new)).is_err(),
            "{old} -> {new}"
        );
    }
    assert!(serde_yaml::from_str::<InternalRegulatorConfig>(&format!(
        "{REGULATOR}extra: true
"
    ))
    .is_err());
}
#[test]
fn role_aliases_missing_domains_duplicate_owners_and_bounds_fail() {
    let base: InternalRegulatorConfig = serde_yaml::from_str(REGULATOR).unwrap();
    let power = domains();
    assert!(validate_internal_regulators(&[base.clone()], None).is_err());
    let mut item = base.clone();
    item.input_domain = "missing".into();
    assert!(validate_internal_regulators(&[item], Some(&power)).is_err());
    let mut item = base.clone();
    item.pins.feedback = vec!["lx".into()];
    assert!(item.validate().is_err());
    let mut item = base.clone();
    item.pins.feedback = vec!["VIN".into()];
    assert!(validate_internal_regulators(&[item], Some(&power)).is_err());
    assert!(validate_internal_regulators(&[base.clone(), base.clone()], Some(&power)).is_err());
    let mut second = base.clone();
    second.id = "second".into();
    assert!(validate_internal_regulators(&[base.clone(), second], Some(&power)).is_err());
    let mut item = base;
    item.pins.ground = vec!["G".into(); 65];
    assert!(item.validate().is_err());
}
#[test]
fn absent_chip_field_is_legacy_but_explicit_empty_or_null_is_rejected() {
    let chip = ChipDescriptor::from_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    assert!(chip.internal_regulators.is_empty());
    for value in [
        serde_yaml::Value::Null,
        serde_yaml::Value::Sequence(Vec::new()),
    ] {
        let mut document = serde_yaml::to_value(&chip).unwrap();
        document
            .as_mapping_mut()
            .unwrap()
            .insert("internal_regulators".into(), value);
        assert!(serde_yaml::from_value::<ChipDescriptor>(document).is_err());
    }
}
