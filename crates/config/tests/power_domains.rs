use labwired_config::{GpioInputThresholds, PowerDomainsConfig, ResetInputConfig};
use serde_yaml::Value;

const CONFIG: &str = "
rails:
  core: {nominal_v: 1.1, min_v: 1.05, max_v: 1.16, pins: [CORE]}
  io: {nominal_v: 3.3, min_v: 1.62, max_v: 3.63, pins: [IO]}
  detector: {nominal_v: 3.3, min_v: 3.135, max_v: 3.63, pins: [DETECTOR]}
  flash: {nominal_v: 3.3, min_v: 2.97, max_v: 3.63, pins: [FLASH]}
io_domain: io
execution_domains: [core]
system_reset_scope: core_only
latched_por: {sense_domain: core, detector_domain: detector, rearm_below_v: 1.1, release_at_v: 0.957, release_delay_ns: 3000}
brown_out: {sense_domain: core, detector_domain: detector, enabled: true, falling_v: 0.946, rising_v: 0.95546, startup_delay_ns: 55000, assert_delay_ns: 3000, release_delay_ns: 55000, enable_delay_ns: 35000, reprogram_delay_ns: 20000}
";

#[test]
fn distinct_domains_preserve_operating_limits_and_detector_thresholds() {
    let config: PowerDomainsConfig = serde_yaml::from_str(CONFIG).unwrap();
    assert_eq!(config.rails.len(), 4);
    assert!(config.latched_por.as_ref().unwrap().release_at_v < config.rails["core"].min_v);
    assert_eq!(
        serde_yaml::from_str::<PowerDomainsConfig>(&serde_yaml::to_string(&config).unwrap())
            .unwrap(),
        config
    );
}

#[test]
fn malformed_domain_ranges_names_and_unresolved_roles_are_refused() {
    let valid: Value = serde_yaml::from_str(CONFIG).unwrap();
    for (path, value) in [
        (vec!["rails", "core", "min_v"], Value::from(0.0)),
        (vec!["rails", "core", "nominal_v"], Value::from(2.0)),
        (vec!["rails", "io", "max_v"], Value::from(f64::NAN)),
        (
            vec!["rails", "io", "pins"],
            serde_yaml::from_str("[CORE]").unwrap(),
        ),
        (vec!["io_domain"], Value::from("missing")),
        (
            vec!["execution_domains"],
            serde_yaml::from_str("[core, core]").unwrap(),
        ),
        (
            vec!["execution_domains"],
            serde_yaml::from_str("[]").unwrap(),
        ),
        (vec!["latched_por", "sense_domain"], Value::from("missing")),
        (vec!["latched_por", "release_at_v"], Value::from(1.2)),
        (vec!["brown_out", "falling_v"], Value::from(2.0)),
        (vec!["system_reset_scope"], Value::from("anything")),
        (
            vec!["peripherals"],
            serde_yaml::from_str("{device: {analog: missing}}").unwrap(),
        ),
        (vec!["unknown"], Value::from(true)),
    ] {
        let mut yaml = valid.clone();
        let mut target = &mut yaml;
        for key in &path[..path.len() - 1] {
            target = &mut target[*key];
        }
        target[path[path.len() - 1]] = value;
        assert!(
            serde_yaml::from_value::<PowerDomainsConfig>(yaml).is_err(),
            "accepted {path:?}"
        );
    }
    for replacement in ["{}", "{bad.name: {nominal_v: 1, min_v: 0.9, max_v: 1.1}}"] {
        let mut yaml = valid.clone();
        yaml["rails"] = serde_yaml::from_str(replacement).unwrap();
        assert!(serde_yaml::from_value::<PowerDomainsConfig>(yaml).is_err());
    }
}

#[test]
fn absolute_limits_are_explicit_nominal_calibration_not_supply_ratios() {
    let limits: GpioInputThresholds = serde_yaml::from_str(
        "{vil_max_v: 0.8, vih_min_v: 2.0, nominal_io_voltage_v: 3.3, minimum_io_voltage_v: 2.5}",
    )
    .unwrap();
    assert!(matches!(
        limits,
        GpioInputThresholds::Absolute {
            vil_max_v: 0.8,
            vih_min_v: 2.0,
            nominal_io_voltage_v: 3.3,
            minimum_io_voltage_v: Some(2.5)
        }
    ));
    let serialized = serde_yaml::to_string(&limits).unwrap();
    assert!(!serialized
        .lines()
        .any(|line| line.starts_with("vil:") || line.starts_with("vih:")));
    assert_eq!(
        serde_yaml::from_str::<GpioInputThresholds>(&serialized).unwrap(),
        limits
    );
    let legacy: GpioInputThresholds = serde_yaml::from_str("{vil: 0.3, vih: 0.7}").unwrap();
    assert_eq!(
        serde_yaml::to_string(&legacy).unwrap(),
        "vil: 0.3\nvih: 0.7\n"
    );
}

#[test]
fn mixed_null_nonfinite_and_unqualified_absolute_forms_fail() {
    for yaml in [
        "{vil_max_v: 0.8, vih_min_v: 2.0}",
        "{vil_max_v: 2.0, vih_min_v: 0.8, nominal_io_voltage_v: 3.3}",
        "{vil_max_v: 0.8, vih_min_v: 3.5, nominal_io_voltage_v: 3.3}",
        "{vil_max_v: .nan, vih_min_v: 2.0, nominal_io_voltage_v: 3.3}",
        "{vil_max_v: 0.8, vih_min_v: 2.0, nominal_io_voltage_v: 3.3, minimum_io_voltage_v: 3.4}",
        "{vil_max_v: 0.8, vih_min_v: 2.0, nominal_io_voltage_v: 3.3, minimum_io_voltage_v: null}",
        "{vil_max_v: 0.8, vih_min_v: 2.0, nominal_io_voltage_v: 3.3, vil: 0.3}",
        "{vil_max_v: 0.8, vih_min_v: 2.0, nominal_io_voltage_v: 3.3, vil_offset_v: 0}",
        "{vil: 0.3, vih: 0.7, vil_offset_v: null}",
        "{vil: 0.3, vih: 0.7, minimum_io_voltage_v: 2.5}",
        "{vil: 0.3, vih: 0.7, surprise: 1}",
    ] {
        assert!(
            serde_yaml::from_str::<GpioInputThresholds>(yaml).is_err(),
            "accepted {yaml}"
        );
    }
}

#[test]
fn reset_domain_scope_is_explicit_and_zero_filter_is_valid() {
    let reset: ResetInputConfig = serde_yaml::from_str("{pin: RUN, active_low: true, vil_max_v: 0.8, vih_min_v: 2.0, io_domain: io, nominal_io_voltage_v: 3.3, min_assert_ns: 0, invalid_pulse_ns: 0}").unwrap();
    assert_eq!(reset.min_assert_ns, 0);
    assert_eq!(reset.io_domain.as_deref(), Some("io"));
    assert_eq!(
        serde_yaml::from_str::<ResetInputConfig>(&serde_yaml::to_string(&reset).unwrap()).unwrap(),
        reset
    );
    for suffix in [
        "io_domain: bad.name",
        "nominal_io_voltage_v: 3.3",
        "io_domain: io, nominal_io_voltage_v: .inf",
        "io_domain: null",
        "io_domain: io, nominal_io_voltage_v: null",
        "io_domain: io, nominal_io_voltage_v: 1.8",
    ] {
        let yaml = format!("{{pin: RUN, active_low: true, vil_max_v: 0.8, vih_min_v: 2.0, min_assert_ns: 0, invalid_pulse_ns: 0, {suffix}}}");
        assert!(
            serde_yaml::from_str::<ResetInputConfig>(&yaml).is_err(),
            "accepted {yaml}"
        );
    }
}
