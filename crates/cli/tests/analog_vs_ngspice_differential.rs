// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Differential oracle for the in-core analog engine.
//!
//! `examples/cosim-spice-rc` declares the same RC low-pass twice: `system.yaml`
//! solves it with ngspice through the `external_process` adapter, and
//! `system-analog.yaml` solves it in-core with `adapter: analog`. Same netlist,
//! same routing, same probe, same `step_ns`. The two `v_out` traces must agree
//! to within 1 % at every step, which is what makes "switch engines by changing
//! `adapter:`" a claim rather than a hope.
//!
//! ngspice is the reference here, not a second implementation of the same
//! approximation: it runs its own adaptive-step transient with its own
//! integration, so agreement is evidence about the physics, not about shared
//! code. There is none — the in-core parser and solver share nothing with it.
//!
//! Requires `libngspice0`. Without it the ngspice half cannot run and the test
//! skips with a message naming what is missing, rather than passing quietly.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Steps compared. 20 × 100 µs = 2 ms, two RC time constants — the part of the
/// curve where the two integrators can actually disagree.
const STEPS: u64 = 20;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn cli_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_labwired"))
}

/// True when libngspice can actually be loaded by the wrapper.
fn ngspice_available() -> bool {
    let probe = Command::new("python3")
        .arg("-c")
        .arg(
            "import ctypes, ctypes.util, sys;\
             name = ctypes.util.find_library('ngspice') or 'libngspice.so.0';\
             ctypes.CDLL(name)",
        )
        .output();
    matches!(probe, Ok(output) if output.status.success())
}

/// Run `labwired cosim-step <manifest> --steps N --set board.gpio.pa5=true
/// --json` and return the `board.analog.pa0_volts` value of each step.
fn v_out_series(manifest: &Path, steps: u64) -> Vec<f64> {
    let output = Command::new(cli_binary())
        .arg("cosim-step")
        .arg(manifest)
        .arg("--set")
        .arg("board.gpio.pa5=true")
        .arg("--steps")
        .arg(steps.to_string())
        .arg("--json")
        .current_dir(repo_root())
        .output()
        .expect("cosim-step should run");
    assert!(
        output.status.success(),
        "cosim-step {} failed: {}",
        manifest.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cosim-step --json emits JSON");
    parsed
        .as_array()
        .expect("a JSON array of steps")
        .iter()
        .map(|step| {
            step["outputs"]["board.analog.pa0_volts"]
                .as_f64()
                .expect("v_out is routed to board.analog.pa0_volts")
        })
        .collect()
}

#[test]
fn in_core_analog_matches_ngspice_on_the_rc_example() {
    let root = repo_root();
    let analog_manifest = root.join("examples/cosim-spice-rc/system-analog.yaml");
    let ngspice_manifest = root.join("examples/cosim-spice-rc/system.yaml");

    let analog = v_out_series(&analog_manifest, STEPS);
    assert_eq!(analog.len() as u64, STEPS, "one routed output per step");

    if !ngspice_available() {
        eprintln!(
            "SKIP: libngspice is not loadable on this host, so the ngspice half of the \
             differential cannot run. Install it (Debian/Ubuntu: `apt install libngspice0`) \
             or set LABWIRED_NGSPICE_LIB. The in-core engine produced {} samples and is \
             covered on its own by `cargo test -p labwired-core analog`.",
            analog.len()
        );
        return;
    }

    let ngspice = v_out_series(&ngspice_manifest, STEPS);
    assert_eq!(ngspice.len(), analog.len());

    let mut worst = 0.0_f64;
    let mut worst_step = 0;
    for (index, (a, n)) in analog.iter().zip(ngspice.iter()).enumerate() {
        // Relative to the 3.3 V rail rather than to the sample: early samples
        // are near zero, where a relative-to-sample error is meaningless and a
        // full-scale error is what an ADC would actually see.
        let error = (a - n).abs() / 3.3;
        if error > worst {
            worst = error;
            worst_step = index + 1;
        }
        assert!(
            error < 0.01,
            "step {} (t = {} us): in-core {a} V vs ngspice {n} V — {:.3} % of full scale",
            index + 1,
            (index as u64 + 1) * 100,
            error * 100.0
        );
    }
    eprintln!(
        "in-core vs ngspice over {STEPS} steps: worst |delta| = {:.4} % of full scale \
         (step {worst_step})",
        worst * 100.0
    );
}

#[test]
fn the_analog_manifest_writes_a_waveform_trace() {
    let root = repo_root();
    let manifest = root.join("examples/cosim-spice-rc/system-analog.yaml");
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-analog-trace");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let csv = dir.join("rc.csv");

    let output = Command::new(cli_binary())
        .arg("cosim-step")
        .arg(&manifest)
        .arg("--set")
        .arg("board.gpio.pa5=true")
        .arg("--steps")
        .arg(STEPS.to_string())
        .arg("--analog-trace")
        .arg(&csv)
        .current_dir(&root)
        .output()
        .expect("cosim-step should run");
    assert!(
        output.status.success(),
        "cosim-step failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let text = std::fs::read_to_string(&csv).expect("trace file");
    let mut lines = text.lines();
    assert_eq!(
        lines.next().expect("header"),
        "time_ns,rc_lowpass.v_out,rc_lowpass.v(in),rc_lowpass.i(Vgpio)",
        "probes first, then the manifest's extra `trace:` expressions, each named \
         `<model id>.<name>`"
    );
    let rows: Vec<&str> = lines.collect();
    assert_eq!(
        rows.len() as u64,
        STEPS + 1,
        "the operating point plus one row per step"
    );
    let first: Vec<&str> = rows[0].split(',').collect();
    assert_eq!(first[0], "0", "the first row is the operating point at t=0");
    let last: Vec<&str> = rows[STEPS as usize].split(',').collect();
    assert_eq!(last[0], (STEPS * 100_000).to_string());
    let final_volts: f64 = last[1].parse().expect("v_out is a number");
    assert!(
        final_volts > 2.7 && final_volts < 3.0,
        "two tau charges to ~86 % of 3.3 V, got {final_volts} V"
    );

    // The same run as VCD, so the analog curve opens beside the logic capture.
    let vcd = dir.join("rc.vcd");
    let output = Command::new(cli_binary())
        .arg("cosim-step")
        .arg(&manifest)
        .arg("--set")
        .arg("board.gpio.pa5=true")
        .arg("--steps")
        .arg(STEPS.to_string())
        .arg("--analog-trace")
        .arg(&vcd)
        .current_dir(&root)
        .output()
        .expect("cosim-step should run");
    assert!(output.status.success());
    let text = std::fs::read_to_string(&vcd).expect("vcd file");
    assert!(text.contains("$timescale 1 ns"), "{text}");
    assert!(text.contains("$var real 64 "), "real vars: {text}");
    assert!(
        text.contains("rc_lowpass_v_out_V"),
        "model id, channel name and unit: {text}"
    );
}

#[test]
fn an_unsupported_element_fails_manifest_validation() {
    let root = repo_root();
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-analog-diode");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(
        dir.join("diode.cir"),
        "* a diode is outside the in-core subset\n\
         Vin in 0 dc 0\nR1 in out 1k\nD1 out 0 dmod\n.end\n",
    )
    .expect("write netlist");
    let manifest = dir.join("system.yaml");
    let yaml = format!(
        "name: diode-check\n\
         chip: \"{chip}\"\n\
         cosim_models:\n\
         \x20 - id: rc\n\
         \x20   adapter: analog\n\
         \x20   step_ns: 100000\n\
         \x20   outputs:\n\
         \x20     v_out: board.analog.pa0_volts\n\
         \x20   config:\n\
         \x20     netlist: ./diode.cir\n\
         \x20     probes:\n\
         \x20       v_out: \"v(out)\"\n",
        chip = root.join("configs/chips/stm32f401.yaml").display()
    );
    std::fs::write(&manifest, yaml).expect("write manifest");

    let output = Command::new(cli_binary())
        .arg("cosim-step")
        .arg(&manifest)
        .output()
        .expect("cosim-step should run");
    assert!(
        !output.status.success(),
        "a netlist outside the subset must not run"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("needs ngspice")
            && stderr.contains("adapter: external_process")
            && stderr.contains("labwired_ngspice.py"),
        "the error must name the adapter that can run it: {stderr}"
    );
}
