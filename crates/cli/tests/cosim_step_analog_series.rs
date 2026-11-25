// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired cosim-step --analog-series` runs a circuit with no MCU and writes
//! the run-long change-point series, in the shape `result.json` carries as
//! `analog_series` for a firmware run.

use std::path::PathBuf;
use std::process::Command;

const MANIFEST: &str = r#"name: "sine-into-rc"
chip: "none"
cosim_models:
  - id: "circuit"
    adapter: "analog"
    step_ns: 10000
    inputs: {}
    outputs: {}
    config:
      vdd: 5.0
      integration: "trap"
      netlist_text: |
        V1 in 0 SIN(2.5 1 1000)
        R1 in out 1k
        C1 out 0 10n
      probes:
        out: "v(out)"
"#;

#[test]
fn a_circuit_with_no_mcu_writes_its_whole_analog_series() {
    let dir = std::env::temp_dir().join(format!("lw-analog-series-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = dir.join("system.yaml");
    let series_path = dir.join("series.json");
    std::fs::write(&manifest, MANIFEST).unwrap();

    // 300 steps of 10 us = 3 ms, three periods of the 1 kHz source.
    let output = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_labwired")))
        .arg("cosim-step")
        .arg(&manifest)
        .arg("--steps")
        .arg("300")
        .arg("--analog-series")
        .arg(&series_path)
        .output()
        .expect("cosim-step should run");
    assert!(
        output.status.success(),
        "cosim-step failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let series: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&series_path).unwrap()).unwrap();
    assert_eq!(series["end_time_ns"].as_u64(), Some(3_000_000));
    let channel = series["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["name"] == "circuit.out")
        .expect("the probe is a channel named <model>.<probe>");
    assert_eq!(channel["unit"], "V");
    let values: Vec<f64> = channel["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|point| point[1].as_f64().unwrap())
        .collect();
    // The RC corner is 16 kHz, so at 1 kHz the output follows the 2.5 V +/- 1 V
    // source within about half a percent.
    let max = values.iter().cloned().fold(f64::MIN, f64::max);
    let min = values.iter().cloned().fold(f64::MAX, f64::min);
    assert!((3.45..3.51).contains(&max), "peak {max}");
    assert!((1.49..1.55).contains(&min), "trough {min}");
    std::fs::remove_dir_all(&dir).ok();
}
