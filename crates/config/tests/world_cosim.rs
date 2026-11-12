use labwired_config::EnvironmentManifest;

const WORLD: &str = r#"
name: shared-circuit
nodes:
  - { id: esc_a, system: a.yaml, firmware: a.bin }
  - { id: esc_b, system: b.yaml, firmware: b.bin }
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 1000
    inputs:
      drive_a: node.esc_a.board.gpio_pad_volts.pa8
      drive_b: node.esc_b.board.gpio_pad_volts.pa8
    outputs:
      sense_a: node.esc_a.adc.adc1.0_volts
      sense_b: node.esc_b.adc.adc1.0_volts
    config:
      netlist_text: "V1 supply 0 dc 3.3\nR1 supply 0 1k\n.end\n"
      probes: { sense_a: "v(supply)", sense_b: "v(supply)" }
"#;

#[test]
fn world_accepts_one_explicit_shared_analog_model() {
    let manifest: EnvironmentManifest = serde_yaml::from_str(WORLD)
        .expect("the world owns one shared circuit with instance-qualified routes");
    manifest
        .validate()
        .expect("qualified routes name existing nodes");
}

#[test]
fn legacy_worlds_default_to_no_shared_model() {
    let value: EnvironmentManifest =
        serde_yaml::from_str("name: old\nnodes: [{id: a, system: a.yaml, firmware: a.bin}]\n")
            .unwrap();
    value.validate().unwrap();
    assert!(value.cosim_models.is_empty());
}

#[test]
fn worlds_reject_ambiguous_and_unsupported_model_contracts() {
    for (old, new, reason) in [
        ("node.esc_b.board", "node.absent.board", "unknown node"),
        ("node.esc_a.board", "board", "must name node."),
        ("adapter: analog", "adapter: mock", "only adapter: analog"),
        ("step_ns: 1000", "step_ns: 0", "positive"),
        ("netlist_text:", "netlist:", "inline config.netlist_text"),
    ] {
        let manifest: EnvironmentManifest = serde_yaml::from_str(&WORLD.replace(old, new)).unwrap();
        let error = manifest.validate().unwrap_err().to_string();
        assert!(error.contains(reason), "{error}");
    }
    let mut manifest: EnvironmentManifest = serde_yaml::from_str(WORLD).unwrap();
    manifest.cosim_models.push(manifest.cosim_models[0].clone());
    assert!(manifest
        .validate()
        .unwrap_err()
        .to_string()
        .contains("exactly one"));
}

#[test]
fn reset_metadata_preserves_absolute_limits_and_rejects_ambiguous_shapes() {
    let yaml = "pin: NRST\nactive_low: true\nvil_max_v: 0.72\nvih_min_v: 2.0\nmin_assert_ns: 80000\ninvalid_pulse_ns: 40000\npull_up_ohms: 40000\n";
    let config: labwired_config::ResetInputConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(
        config.thresholds,
        labwired_config::ResetInputThresholds::Absolute {
            vil_max_v: 0.72,
            vih_min_v: 2.0
        }
    );
    assert_eq!(config.pull_up_ohms, Some(40000.0));
    let round_trip: labwired_config::ResetInputConfig =
        serde_yaml::from_str(&serde_yaml::to_string(&config).unwrap()).unwrap();
    assert_eq!(config, round_trip);
    for invalid in [
        format!("{yaml}vil: 0.3\nvih: 0.7\n"),
        yaml.replace("vih_min_v: 2.0", "vih_min_v: 0.1"),
        yaml.replace("pull_up_ohms: 40000", "pull_up_ohms: 0"),
        yaml.replace("min_assert_ns: 80000", "min_assert_ns: 30000"),
    ] {
        assert!(
            serde_yaml::from_str::<labwired_config::ResetInputConfig>(&invalid).is_err(),
            "{invalid}"
        );
    }
}
