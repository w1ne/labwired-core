// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! A `Session` carries the manifest's co-simulation models (the analog island a
//! lowered diagram declares) and steps them in lockstep with the machine, so a
//! Python or Rust test reads the same waveform `labwired test --analog-trace`
//! writes. Before this a session ignored `cosim_models` without saying so.
//!
//! The circuit is a 10k/10k divider on the Nano's 5 V pin: 2.4987507 V at the
//! midpoint with a 10 MOhm meter across it. Firmware is the committed Nano
//! blinky, which has no part in the circuit.

use crate::common;
use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use std::time::Duration;

const NANO_ELF: &str = "tests/fixtures/avr/arduino-nano-blinky.elf";

const MODELS: &str = r#"
- id: "circuit"
  adapter: "analog"
  step_ns: 100000
  inputs: {}
  outputs: {}
  config:
    vdd: 5.0
    netlist_text: |
      Vdd v_net_mcu_5v 0 dc 5.0
      RR1 v_net_mcu_5v v_net_dmm_v 10k
      RR2 v_net_dmm_v 0 10k
      Rdmm_dmm v_net_dmm_v 0 10000000
    probes:
      dmm_vdc_p_dmm: "v(v_net_dmm_v)"
    sources: {}
"#;

fn open(with_models: bool) -> Session {
    let (chip, mut manifest) = common::system("configs/systems/arduino-nano.yaml");
    if with_models {
        manifest.cosim_models = serde_yaml::from_str(MODELS).expect("models");
    }
    let fw = common::committed(NANO_ELF);
    let blobs = BlobMap::new();
    Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .expect("open")
}

#[test]
fn the_analog_island_runs_with_the_machine() {
    let mut s = open(true);
    s.run_for(Duration::from_millis(5)).unwrap();
    let trace = s.analog_trace(0);
    assert_eq!(
        trace
            .channels
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["circuit.dmm_vdc_p_dmm"]
    );
    // One row per 100 us step, plus the operating point.
    assert_eq!(trace.samples.len(), 51);
    let last = trace.samples.last().unwrap();
    assert!(
        (f64::from(last.values[0]) - 2.4987507).abs() < 1e-5,
        "{last:?}"
    );
    assert_eq!(last.time_ns, 5_000_000);
}

#[test]
fn csv_is_the_cli_shape() {
    let mut s = open(true);
    s.run_for(Duration::from_micros(250)).unwrap();
    let mut out = Vec::new();
    s.analog_trace(0).write_csv(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("time_ns,circuit.dmm_vdc_p_dmm"));
    assert_eq!(lines.next(), Some("0,2.4987507"));
    assert_eq!(lines.next(), Some("100000,2.4987507"));
}

#[test]
fn a_session_without_models_has_no_trace_and_runs_as_before() {
    let mut s = open(false);
    s.run_for(Duration::from_millis(5)).unwrap();
    assert!(s.analog_trace(0).channels.is_empty());
    assert!(s.analog_trace(0).samples.is_empty());
}

#[test]
fn restore_rebuilds_the_models_with_the_machine() {
    let mut s = open(true);
    s.run_for(Duration::from_millis(2)).unwrap();
    let snap = s.snapshot();
    let at_snapshot = s.analog_trace(0).samples.len();
    s.run_for(Duration::from_millis(3)).unwrap();
    assert!(s.analog_trace(0).samples.len() > at_snapshot);
    s.restore(&snap).unwrap();
    assert_eq!(s.analog_trace(0).samples.len(), at_snapshot);
    s.run_for(Duration::from_millis(3)).unwrap();
    assert_eq!(s.analog_trace(0).samples.last().unwrap().time_ns, 5_000_000);
}

const SOURCE: &str = r#"
- id: "circuit"
  adapter: "analog"
  step_ns: 100000
  inputs: { lvl: "ui.level" }
  outputs: {}
  config:
    vdd: 5.0
    netlist_text: |
      Vsrc v_a 0 dc 0
      R1 v_a 0 1k
    probes: { out: "v(v_a)" }
    sources: { lvl: "Vsrc" }
"#;

#[test]
fn a_signal_set_mid_run_reaches_the_circuit_and_survives_restore() {
    let (chip, mut manifest) = common::system("configs/systems/arduino-nano.yaml");
    manifest.cosim_models = serde_yaml::from_str(SOURCE).expect("models");
    let fw = common::committed(NANO_ELF);
    let blobs = BlobMap::new();
    let mut s = Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .expect("open");
    let volts = |s: &Session| f64::from(s.analog_trace(0).samples.last().unwrap().values[0]);
    s.run_for(Duration::from_millis(1)).unwrap();
    assert_eq!(volts(&s), 0.0);
    s.set_signal("ui.level", 3.0).unwrap();
    s.run_for(Duration::from_millis(1)).unwrap();
    assert!((volts(&s) - 3.0).abs() < 1e-6);
    // Replay on restore re-applies the signal where it was set.
    let snap = s.snapshot();
    s.set_signal("ui.level", 1.0).unwrap();
    s.run_for(Duration::from_millis(1)).unwrap();
    s.restore(&snap).unwrap();
    s.run_for(Duration::from_millis(1)).unwrap();
    assert!((volts(&s) - 3.0).abs() < 1e-6);
    // A path nothing reads, and a board path the machine owns, are errors.
    assert!(s.set_signal("ui.nope", 1.0).is_err());
    assert!(s.set_signal("board.gpio.pd2", 1.0).is_err());
    // A session with no models has nothing to set.
    assert!(open(false).set_signal("ui.level", 1.0).is_err());
}

#[test]
fn an_unroutable_model_is_an_error_not_a_silent_circuit() {
    let (chip, mut manifest) = common::system("configs/systems/arduino-nano.yaml");
    manifest.cosim_models = serde_yaml::from_str(
        r#"
- id: "circuit"
  adapter: "analog"
  step_ns: 100000
  inputs: { drv: "board.gpio.zz9" }
  outputs: {}
  config:
    vdd: 5.0
    netlist_text: |
      Vdrv v_a 0 dc 0
      R1 v_a 0 1k
    probes: { p: "v(v_a)" }
    sources: { drv: "Vdrv" }
"#,
    )
    .expect("models");
    let fw = common::committed(NANO_ELF);
    let blobs = BlobMap::new();
    let err = Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .err()
    .expect("a model reading a pad that does not exist must not open");
    assert!(
        format!("{err:#}").contains("co-simulation models"),
        "{err:#}"
    );
}
