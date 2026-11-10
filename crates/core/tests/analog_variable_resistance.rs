use labwired_core::analog::{AnalogConfig, AnalogCosimAdapter};
use labwired_core::cosim::{CosimAdapter, CosimSignalValue, CosimStep};
use std::collections::BTreeMap;

fn divider() -> AnalogCosimAdapter {
    let config = AnalogConfig {
        probes: BTreeMap::from([("voltage".into(), "v(out)".into())]),
        sources: BTreeMap::from([("resistance".into(), "Rvar".into())]),
        ..AnalogConfig::default()
    };
    AnalogCosimAdapter::from_netlist("V1 in 0 dc 5\nRfixed in out 1k\nRvar out 0 1k\n", &config)
        .unwrap()
}

fn step(
    adapter: &mut AnalogCosimAdapter,
    time: u64,
    value: CosimSignalValue,
) -> Result<f64, String> {
    let result = adapter
        .step(CosimStep {
            time_ns: time,
            dt_ns: 100_000,
            inputs: BTreeMap::from([("resistance".into(), value)]),
        })
        .map_err(|e| e.to_string())?;
    match result.outputs["voltage"] {
        CosimSignalValue::F64(value) => Ok(value),
        ref other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn routed_resistance_changes_the_actual_divider() {
    let mut adapter = divider();
    let first = step(&mut adapter, 100_000, CosimSignalValue::F64(1_000.0)).unwrap();
    let second = step(&mut adapter, 200_000, CosimSignalValue::F64(9_000.0)).unwrap();
    assert!((first - 2.5).abs() < 1e-6, "{first}");
    assert!((second - 4.5).abs() < 1e-6, "{second}");
}

#[test]
fn invalid_resistance_is_rejected_without_corrupting_the_next_step() {
    let mut adapter = divider();
    for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(step(&mut adapter, 100_000, CosimSignalValue::F64(invalid)).is_err());
    }
    assert!(step(&mut adapter, 100_000, CosimSignalValue::Bool(true)).is_err());
    assert!(
        (step(&mut adapter, 100_000, CosimSignalValue::F64(1000.0)).unwrap() - 2.5).abs() < 1e-6
    );
}

#[test]
fn ui_resistance_starts_at_the_netlist_value_and_can_be_changed() {
    use labwired_config::SystemManifest;
    use labwired_core::bus::SystemBus;
    use labwired_core::cosim::CosimSession;
    use std::path::Path;
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: resistor-ui
chip: fixture
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 100000
    inputs: { resistance: ui.sensor.ohms }
    outputs: { voltage: analog.voltage }
    config:
      netlist_text: |
        V1 in 0 dc 5
        Rfixed in out 1k
        Rvar out 0 1k
      probes: { voltage: "v(out)" }
      sources: { resistance: Rvar }
"#,
    )
    .unwrap();
    let mut bus = SystemBus::new();
    let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &bus)
        .unwrap()
        .unwrap();
    assert_eq!(
        session.signals()["ui.sensor.ohms"],
        CosimSignalValue::F64(1000.0)
    );
    let (initial, _) = session
        .advance_to(10_000, &mut bus)
        .expect("default resistance must not fail");
    let CosimSignalValue::F64(first) = initial.last().unwrap().outputs["analog.voltage"] else {
        panic!("unexpected output")
    };
    assert!((first - 2.5).abs() < 1e-6, "{first}");
    session.set_signal_number("ui.sensor.ohms", 9000.0).unwrap();
    let (updated, _) = session
        .advance_to(20_000, &mut bus)
        .expect("updated resistance must not fail");
    let CosimSignalValue::F64(second) = updated.last().unwrap().outputs["analog.voltage"] else {
        panic!("unexpected output")
    };
    assert!((second - 4.5).abs() < 1e-6, "{second}");
}

#[test]
fn resistance_changes_restart_transient_integration_after_settling() {
    let config = AnalogConfig {
        probes: BTreeMap::from([("voltage".into(), "v(out)".into())]),
        sources: BTreeMap::from([
            ("resistance".into(), "Rvar".into()),
            ("drive".into(), "V1".into()),
        ]),
        ..AnalogConfig::default()
    };
    let mut adapter =
        AnalogCosimAdapter::from_netlist("V1 in 0 dc 5\nRvar in out 1k\nC1 out 0 1u\n", &config)
            .unwrap();
    let mut value = 0.0;
    for n in 1..=200 {
        value = step(&mut adapter, n * 100_000, CosimSignalValue::F64(1000.0)).unwrap();
    }
    assert!((value - 5.0).abs() < 1e-5, "{value}");
    let same = step(&mut adapter, 20_100_000, CosimSignalValue::F64(1000.0)).unwrap();
    assert!((same - 5.0).abs() < 1e-5);
    let result = adapter
        .step(CosimStep {
            time_ns: 20_200_000,
            dt_ns: 100_000,
            inputs: BTreeMap::from([
                ("resistance".into(), CosimSignalValue::F64(9000.0)),
                ("drive".into(), CosimSignalValue::F64(0.0)),
            ]),
        })
        .unwrap();
    let CosimSignalValue::F64(changed) = result.outputs["voltage"] else {
        panic!("unexpected output")
    };
    assert!(changed > 4.94 && changed < 4.95, "{changed}");
}

#[test]
fn conflicting_ui_resistance_defaults_are_rejected() {
    use labwired_config::SystemManifest;
    use labwired_core::bus::SystemBus;
    use labwired_core::cosim::CosimSession;
    use std::path::Path;
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: conflict
chip: fixture
cosim_models:
  - id: first
    adapter: analog
    step_ns: 100000
    inputs: { resistance: ui.sensor.ohms }
    config:
      netlist_text: "V1 a 0 dc 1\nRvar a 0 1k\n"
      sources: { resistance: Rvar }
      probes: { voltage: "v(a)" }
  - id: second
    adapter: analog
    step_ns: 100000
    inputs: { resistance: ui.sensor.ohms }
    config:
      netlist_text: "V1 a 0 dc 1\nRvar a 0 2k\n"
      sources: { resistance: Rvar }
      probes: { voltage: "v(a)" }
"#,
    )
    .unwrap();
    let result = CosimSession::new(&manifest.cosim_models, Path::new("."), &SystemBus::new());
    assert!(
        matches!(result, Err(ref error) if error.to_string().contains("conflicting initial values"))
    );
}

#[test]
fn routed_resistor_requires_a_valid_initial_resistance() {
    let config = AnalogConfig {
        sources: BTreeMap::from([("resistance".into(), "Rvar".into())]),
        ..AnalogConfig::default()
    };
    let result = AnalogCosimAdapter::from_netlist("V1 a 0 dc 1\nRvar a 0 -1k\n", &config);
    assert!(matches!(result, Err(ref error) if error.to_string().contains("initial resistance")));
}
