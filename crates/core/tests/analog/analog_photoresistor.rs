// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bounded power-law photoresistor routed into the in-core analog engine.
//! The numbers are the GL5528 row of Senba's GL55 series manual (10-20 kΩ at
//! 10 lx, γ = 0.6 between 10 and 100 lx) in a top-leg divider over 10 kΩ.

use labwired_core::analog::{AnalogConfig, AnalogCosimAdapter, PhotoresistorConfig};
use labwired_core::cosim::{CosimAdapter, CosimSignalValue, CosimStep};
use std::collections::BTreeMap;

const NETLIST: &str = "V1 in 0 dc 3.3\nRldr in out 15k\nRfixed out 0 10k\n";

fn photoresistor() -> PhotoresistorConfig {
    PhotoresistorConfig {
        resistor: "Rldr".into(),
        r_ref_ohms: 15_000.0,
        reference_lux: 10.0,
        gamma: 0.6,
        min_lux: 1.0,
        max_lux: 100.0,
    }
}
fn config() -> AnalogConfig {
    AnalogConfig {
        probes: BTreeMap::from([("voltage".into(), "v(out)".into())]),
        photoresistors: BTreeMap::from([("lux".into(), photoresistor())]),
        ..AnalogConfig::default()
    }
}
fn create(config: &AnalogConfig) -> Result<AnalogCosimAdapter, labwired_core::analog::AnalogError> {
    AnalogCosimAdapter::from_netlist(NETLIST, config)
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
            inputs: BTreeMap::from([("lux".into(), value)]),
        })
        .map_err(|e| e.to_string())?;
    match result.outputs["voltage"] {
        CosimSignalValue::F64(value) => Ok(value),
        ref value => panic!("unexpected {value:?}"),
    }
}
fn divider(lux: f64) -> f64 {
    let ohms = 15_000.0 * (lux / 10.0_f64).powf(-0.6);
    3.3 * 10_000.0 / (ohms + 10_000.0)
}

#[test]
fn power_law_changes_the_connected_divider() {
    let mut adapter = create(&config()).unwrap();
    assert_eq!(adapter.input_unit("lux"), Some("lx"));
    assert_eq!(
        adapter.input_initial_value("lux"),
        Some(CosimSignalValue::F64(10.0))
    );
    let reference = step(&mut adapter, 100_000, CosimSignalValue::F64(10.0)).unwrap();
    assert!((reference - 1.32).abs() < 1e-6, "{reference}");
    let dim = step(&mut adapter, 200_000, CosimSignalValue::F64(1.0)).unwrap();
    assert!((dim - divider(1.0)).abs() < 1e-6, "{dim}");
    let bright = step(&mut adapter, 300_000, CosimSignalValue::F64(100.0)).unwrap();
    assert!((bright - divider(100.0)).abs() < 1e-6, "{bright}");
    // The cell is the top leg: more light, lower resistance, higher node.
    assert!(dim < reference && reference < bright);
}

#[test]
fn rejects_illuminance_outside_the_declared_curve_range() {
    let mut adapter = create(&config()).unwrap();
    for value in [0.0, 0.999, 100.001, -1.0, f64::NAN, f64::INFINITY] {
        let error = step(&mut adapter, 100_000, CosimSignalValue::F64(value)).unwrap_err();
        assert!(error.contains("lux"), "{value}: {error}");
    }
    let dark = step(&mut adapter, 100_000, CosimSignalValue::F64(0.0)).unwrap_err();
    assert!(
        dark.contains("photoresistor illuminance must be within 1..100 lx"),
        "{dark}"
    );
    assert!(step(&mut adapter, 100_000, CosimSignalValue::Bool(true)).is_err());
    // A refused input leaves the circuit at its last valid resistance.
    let reference = step(&mut adapter, 100_000, CosimSignalValue::F64(10.0)).unwrap();
    assert!((reference - 1.32).abs() < 1e-6, "{reference}");
}

#[test]
fn rejects_invalid_models_and_duplicate_resistor_owners() {
    let mutate = |change: fn(&mut PhotoresistorConfig)| {
        let mut bad = config();
        change(bad.photoresistors.get_mut("lux").unwrap());
        create(&bad)
    };
    assert!(mutate(|model| model.gamma = -0.6).is_err());
    assert!(mutate(|model| model.gamma = 0.0).is_err());
    assert!(mutate(|model| model.min_lux = 0.0).is_err());
    assert!(mutate(|model| model.min_lux = 11.0).is_err());
    assert!(mutate(|model| model.max_lux = 9.0).is_err());
    assert!(mutate(|model| model.r_ref_ohms = 1234.0).is_err());
    assert!(mutate(|model| model.gamma = 1e308).is_err());
    assert!(mutate(|model| model.resistor = "Rmissing".into()).is_err());
    let mut bad = config();
    bad.photoresistors.insert("other".into(), photoresistor());
    assert!(create(&bad).is_err());
    let mut bad = config();
    bad.sources.insert("resistance".into(), "Rldr".into());
    assert!(create(&bad).is_err());
}

#[test]
fn explicit_yaml_model_is_parsed_and_unknown_fields_are_rejected() {
    let yaml = r#"
photoresistors:
  lux:
    resistor: Rldr
    r_ref_ohms: 15000
    reference_lux: 10
    gamma: 0.6
    min_lux: 1
    max_lux: 100
"#;
    let values = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(
        AnalogConfig::from_yaml(&values).unwrap().photoresistors["lux"],
        photoresistor()
    );
    let missing = serde_yaml::from_str(&yaml.replace("    gamma: 0.6\n", "")).unwrap();
    assert!(AnalogConfig::from_yaml(&missing).is_err());
    let bad = serde_yaml::from_str(&(yaml.to_string() + "    dark_ohms: 1000000\n")).unwrap();
    assert!(AnalogConfig::from_yaml(&bad).is_err());
}
