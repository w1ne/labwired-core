use labwired_core::analog::{AnalogConfig, AnalogCosimAdapter, ThermistorConfig};
use labwired_core::cosim::{CosimAdapter, CosimSignalValue, CosimStep};
use std::collections::BTreeMap;

fn thermistor() -> ThermistorConfig {
    ThermistorConfig {
        resistor: "Rntc".into(),
        r_ref_ohms: 10_000.0,
        beta_kelvin: 3950.0,
        reference_c: 25.0,
        min_c: 0.0,
        max_c: 85.0,
    }
}
fn config() -> AnalogConfig {
    AnalogConfig {
        probes: BTreeMap::from([("voltage".into(), "v(out)".into())]),
        thermistors: BTreeMap::from([("temperature".into(), thermistor())]),
        ..AnalogConfig::default()
    }
}
fn create(config: &AnalogConfig) -> Result<AnalogCosimAdapter, labwired_core::analog::AnalogError> {
    AnalogCosimAdapter::from_netlist("V1 in 0 dc 5\nRfixed in out 10k\nRntc out 0 10k\n", config)
}
fn step(
    adapter: &mut AnalogCosimAdapter,
    time_ns: u64,
    value: CosimSignalValue,
) -> Result<f64, String> {
    let result = adapter
        .step(CosimStep {
            time_ns,
            dt_ns: 100_000,
            inputs: BTreeMap::from([("temperature".into(), value)]),
        })
        .map_err(|e| e.to_string())?;
    match result.outputs["voltage"] {
        CosimSignalValue::F64(value) => Ok(value),
        ref value => panic!("unexpected {value:?}"),
    }
}
#[test]
fn beta_curve_changes_the_connected_divider() {
    let mut adapter = create(&config()).unwrap();
    assert!((step(&mut adapter, 100_000, CosimSignalValue::F64(25.0)).unwrap() - 2.5).abs() < 1e-6);
    let r0 = 10_000.0 * (3950.0_f64 * (1.0 / 273.15 - 1.0 / 298.15)).exp();
    let cold = step(&mut adapter, 200_000, CosimSignalValue::F64(0.0)).unwrap();
    assert!((cold - 5.0 * r0 / (10_000.0 + r0)).abs() < 1e-6);
    let r85 = 10_000.0 * (3950.0_f64 * (1.0 / 358.15 - 1.0 / 298.15)).exp();
    let expected = 5.0 * r85 / (10_000.0 + r85);
    assert!(
        (step(&mut adapter, 300_000, CosimSignalValue::F64(85.0)).unwrap() - expected).abs() < 1e-6
    );
}
#[test]
fn rejects_temperatures_outside_the_declared_curve_range() {
    let mut adapter = create(&config()).unwrap();
    for value in [-1.0, 86.0, f64::NAN, f64::INFINITY] {
        assert!(step(&mut adapter, 100_000, CosimSignalValue::F64(value)).is_err());
    }
    assert!(step(&mut adapter, 100_000, CosimSignalValue::Bool(true)).is_err());
    assert!((step(&mut adapter, 100_000, CosimSignalValue::F64(25.0)).unwrap() - 2.5).abs() < 1e-6);
}
#[test]
fn rejects_invalid_models_and_duplicate_resistor_owners() {
    let mut bad = config();
    bad.thermistors.get_mut("temperature").unwrap().beta_kelvin = -1.0;
    assert!(create(&bad).is_err());
    let mut bad = config();
    bad.thermistors.get_mut("temperature").unwrap().min_c = 26.0;
    assert!(create(&bad).is_err());
    let mut bad = config();
    bad.thermistors.get_mut("temperature").unwrap().r_ref_ohms = 1234.0;
    assert!(create(&bad).is_err());
    let mut bad = config();
    bad.thermistors.get_mut("temperature").unwrap().beta_kelvin = 1e308;
    assert!(create(&bad).is_err());
    let mut bad = config();
    bad.thermistors.get_mut("temperature").unwrap().min_c = -273.15;
    assert!(create(&bad).is_err());
    let mut bad = config();
    bad.thermistors
        .insert("other_temperature".into(), thermistor());
    assert!(create(&bad).is_err());
    let mut bad = config();
    bad.sources.insert("resistance".into(), "Rntc".into());
    assert!(create(&bad).is_err());
}

#[test]
fn explicit_yaml_model_is_parsed_and_unknown_fields_are_rejected() {
    let yaml = r#"
thermistors:
  temperature:
    resistor: Rntc
    r_ref_ohms: 10000
    beta_kelvin: 3950
    reference_c: 25
    min_c: 0
    max_c: 85
"#;
    let values = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(
        AnalogConfig::from_yaml(&values).unwrap().thermistors["temperature"],
        thermistor()
    );
    let missing = serde_yaml::from_str(&yaml.replace("    beta_kelvin: 3950\n", "")).unwrap();
    assert!(AnalogConfig::from_yaml(&missing).is_err());
    let bad = serde_yaml::from_str(&(yaml.to_string() + "    typo: 1\n")).unwrap();
    assert!(AnalogConfig::from_yaml(&bad).is_err());
}

#[test]
fn ui_temperature_starts_at_reference_and_drives_the_circuit() {
    use labwired_config::SystemManifest;
    use labwired_core::bus::SystemBus;
    use labwired_core::cosim::CosimSession;
    use std::path::Path;
    let manifest: SystemManifest = serde_yaml::from_str(r#"
name: thermistor-ui
chip: fixture
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 100000
    inputs: { temperature: ui.sensor.temperature }
    outputs: { voltage: analog.voltage }
    config:
      netlist_text: |
        V1 in 0 dc 5
        Rfixed in out 10k
        Rntc out 0 10k
      probes: { voltage: "v(out)" }
      thermistors:
        temperature: { resistor: Rntc, r_ref_ohms: 10000, beta_kelvin: 3950, reference_c: 25, min_c: 0, max_c: 85 }
"#).unwrap();
    let mut bus = SystemBus::new();
    let mut session = CosimSession::new(&manifest.cosim_models, Path::new("."), &bus)
        .unwrap()
        .unwrap();
    assert_eq!(
        session.signals()["ui.sensor.temperature"],
        CosimSignalValue::F64(25.0)
    );
    let (initial, _) = session.advance_to(10_000, &mut bus).unwrap();
    let CosimSignalValue::F64(first) = initial.last().unwrap().outputs["analog.voltage"] else {
        panic!("unexpected output")
    };
    assert!((first - 2.5).abs() < 1e-6);
    session
        .set_signal_number("ui.sensor.temperature", 85.0)
        .unwrap();
    let (updated, _) = session.advance_to(20_000, &mut bus).unwrap();
    let CosimSignalValue::F64(second) = updated.last().unwrap().outputs["analog.voltage"] else {
        panic!("unexpected output")
    };
    let r85 = 10_000.0 * (3950.0_f64 * (1.0 / 358.15 - 1.0 / 298.15)).exp();
    assert!((second - 5.0 * r85 / (10_000.0 + r85)).abs() < 1e-6);
}
