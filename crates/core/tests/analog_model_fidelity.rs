use labwired_config::SystemManifest;
use labwired_core::cosim::validate_analog_models;
use std::path::Path;

fn issues(parameters: &str, flag: &str) -> Vec<String> {
    let yaml = format!(
        r#"
name: fidelity-test
chip: fixture
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 100000
    config:
      netlist_text: |
        V1 in 0 dc 1
        R1 in out 1k
        D1 out 0 vendor
        .model vendor D(IS=2n N=2 {parameters})
      probes: {{ out: "v(out)" }}
      {flag}
"#
    );
    let manifest: SystemManifest = serde_yaml::from_str(&yaml).unwrap();
    validate_analog_models(&manifest.cosim_models, Path::new("."))
}

#[test]
fn unsupported_vendor_parameters_are_reported_before_simulation() {
    let found = issues("CJO=4p TT=3n", "");
    assert_eq!(found.len(), 1, "{found:?}");
    let message = &found[0];
    assert!(
        message.contains("vendor")
            && message.contains("CJO")
            && message.contains("TT")
            && message.contains("ngspice"),
        "{message}"
    );
}

#[test]
fn fully_supported_cards_do_not_need_a_waiver() {
    assert!(issues("RS=1 BV=100 IBV=1u", "").is_empty());
}

#[test]
fn an_opt_in_does_not_hide_unmodeled_behavior() {
    assert!(!issues("CJO=4p", "allow_ignored_model_parameters: true").is_empty());
}

#[test]
fn a_string_cannot_silently_authorize_an_approximation() {
    let found = issues("CJO=4p", "allow_ignored_model_parameters: 'true'");
    assert!(!found.is_empty(), "{found:?}");
}

#[test]
fn low_level_parser_records_only_used_model_omissions() {
    let circuit = labwired_core::analog::parse_netlist(
        "Q1 c b 0 qmod\nM1 d g 0 0 mmod\n.model qmod NPN(BF=100 VAF=10)\n\
         .model mmod NMOS(LEVEL=1 KP=20u CGS=4p)\n.model unused D(CJO=9p)\n",
    )
    .unwrap();
    let omitted: Vec<_> = circuit
        .ignored_model_parameters
        .iter()
        .map(|p| (p.model.as_str(), p.parameter.as_str(), p.line))
        .collect();
    assert_eq!(omitted, vec![("qmod", "VAF", 3), ("mmod", "CGS", 4)]);
}

#[test]
fn approximation_flag_is_rejected_even_for_a_fully_supported_card() {
    let found = issues("RS=1", "allow_ignored_model_parameters: 'yes'");
    assert!(
        found.iter().any(|s| s.contains("not supported")),
        "{found:?}"
    );
}

#[test]
fn direct_adapter_rejects_omitted_parameters() {
    use labwired_core::analog::{AnalogConfig, AnalogCosimAdapter};
    let result = AnalogCosimAdapter::from_netlist(
        "D1 a 0 vendor\n.model vendor D(CJO=4p)\n",
        &AnalogConfig::default(),
    );
    let error = result.expect_err("direct adapter must not silently drop the model parameter");
    assert!(error.to_string().contains("CJO"), "{error}");
}

#[test]
fn temperature_directive_cannot_silently_run_at_fixed_temperature() {
    let yaml = r#"
name: temperature-test
chip: fixture
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 100000
    config:
      netlist_text: |
        V1 in 0 dc 1
        R1 in out 1k
        D1 out 0 D
        .temp 125
      probes: { out: "v(out)" }
"#;
    let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();
    let found = validate_analog_models(&manifest.cosim_models, Path::new("."));
    assert!(
        found
            .iter()
            .any(|s| s.contains(".temp") && s.contains("ngspice")),
        "{found:?}"
    );
}

#[test]
fn control_script_and_temperature_options_cannot_silently_change_the_circuit() {
    use labwired_core::analog::{AnalogConfig, AnalogCosimAdapter};
    for deck in [
        "V1 in 0 dc 1\nR1 in 0 1k\n.control\nset temp=125\n.endc\n",
        "V1 in 0 dc 1\nR1 in 0 1k\n.options temp=125\n",
        "V1 in 0 dc 1\nR1 in 0 1k\n.option TNOM = 125\n",
    ] {
        let error = AnalogCosimAdapter::from_netlist(deck, &AnalogConfig::default())
            .expect_err("ignored temperature or script cannot run silently");
        assert!(error.to_string().contains("ngspice"), "{error}");
    }
}
