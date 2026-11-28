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
//! Requires `libngspice0` and the `ngspice` executable for the reference runs.
//! Local runs skip the missing half with an explicit message; set
//! `LABWIRED_REQUIRE_NGSPICE=1` to make either missing backend fail the test.

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

/// Make optional local coverage mandatory in the CI job that provisions
/// ngspice. A missing reference engine must not look like a differential pass.
fn backend_availability(available: bool, backend: &str, required: bool) -> Result<bool, String> {
    if available || !required {
        Ok(available)
    } else {
        Err(format!(
            "LABWIRED_REQUIRE_NGSPICE is set, but {backend} is unavailable"
        ))
    }
}

fn require_backend_if_configured(available: bool, backend: &str) -> bool {
    backend_availability(
        available,
        backend,
        std::env::var_os("LABWIRED_REQUIRE_NGSPICE").is_some(),
    )
    .unwrap_or_else(|message| panic!("{message}"))
}

fn assert_finite_samples(label: &str, samples: &[f64]) {
    assert!(
        samples.iter().all(|sample| sample.is_finite()),
        "{label} contains a non-finite sample: {samples:?}"
    );
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
    require_backend_if_configured(
        matches!(probe, Ok(output) if output.status.success()),
        "libngspice",
    )
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
    let series = parsed
        .as_array()
        .expect("a JSON array of steps")
        .iter()
        .map(|step| {
            step["outputs"]["board.analog.pa0_volts"]
                .as_f64()
                .expect("v_out is routed to board.analog.pa0_volts")
        })
        .collect::<Vec<_>>();
    assert_finite_samples(
        &format!("{} board.analog.pa0_volts", manifest.display()),
        &series,
    );
    series
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

/// Run `cosim-step <manifest> --steps N --json` with optional `--set` seeds and
/// return, for each step, the value `model` publishes on `signal`.
fn cosim_series(manifest: &Path, steps: u64, sets: &[&str], model: &str, signal: &str) -> Vec<f64> {
    let mut command = Command::new(cli_binary());
    command.arg("cosim-step").arg(manifest);
    for set in sets {
        command.arg("--set").arg(set);
    }
    let output = command
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
    let series = parsed
        .as_array()
        .expect("a JSON array of steps")
        .iter()
        .filter(|step| step["model"] == model)
        .map(|step| {
            step["outputs"][signal]
                .as_f64()
                .unwrap_or_else(|| panic!("{model} does not publish {signal}"))
        })
        .collect::<Vec<_>>();
    assert_finite_samples(&format!("{} {model}.{signal}", manifest.display()), &series);
    series
}

/// The generic Beta NTC divider and Shockley diode load line that 16 board
/// examples share, solved by `adapter: ngspice` through the same manifest
/// routing the in-core `adapter: analog` uses. This is the coverage evidence
/// for the largest class of in-core decks, not only for the Flex fixtures.
///
/// Both decks are algebraic: their settled value is compared at three
/// temperatures, and the temperature must move the NTC output, so agreement on
/// a flat wrong value cannot pass. The diode needs ngspice's temperature pinned
/// to the in-core engine's 300.00 K (26.85 degC); the default 27 degC shifts a
/// forward-biased diode by 0.3 mV, about 1,500 times the agreement measured
/// here.
#[test]
fn ngspice_solves_the_generic_ntc_and_diode_decks_the_in_core_engine_solves() {
    const STEPS: u64 = 4;
    const NTC_SIGNAL: &str = "board.analog.pa0_volts";
    const DIODE_SIGNAL: &str = "board.analog.pa1_volts";
    let root = repo_root();
    let analog = root.join("examples/nucleo-f401re/analog-ntc-diode-system.yaml");
    let spiced = root.join("examples/nucleo-f401re/analog-ntc-diode-ngspice.yaml");

    // The in-core half always runs: a missing ngspice must not hide a deck the
    // engine can no longer solve.
    let mut in_core_ntc = Vec::new();
    for celsius in ["0", "25", "60"] {
        let set = format!("ui.ntc.temperature_c={celsius}");
        let series = cosim_series(&analog, STEPS, &[&set], "ntc_divider", NTC_SIGNAL);
        assert_eq!(series.len() as u64, STEPS);
        assert!(
            series.iter().all(|volts| volts.is_finite()),
            "{celsius} degC: non-finite in-core sample"
        );
        in_core_ntc.push(series);
    }
    let in_core_diode = cosim_series(&analog, STEPS, &[], "diode_load_line", DIODE_SIGNAL);
    assert_eq!(in_core_diode.len() as u64, STEPS);
    // No stimulus must fall back to the netlist's 25 degC default, not 0 degC.
    let in_core_default = cosim_series(&analog, STEPS, &[], "ntc_divider", NTC_SIGNAL);

    // The three temperatures must not agree with each other, or a differential
    // on a temperature-insensitive deck would pass while proving nothing.
    for pair in in_core_ntc.windows(2) {
        let (cold, warm) = (pair[0].last().unwrap(), pair[1].last().unwrap());
        assert!(
            (warm - cold).abs() > 0.1,
            "the NTC deck does not respond to temperature: {cold} V vs {warm} V"
        );
    }

    if !ngspice_available() {
        eprintln!(
            "SKIP: libngspice is not loadable on this host, so the ngspice half of the \
             NTC/diode differential cannot run. Install it (Debian/Ubuntu: \
             `apt install libngspice0`) or set LABWIRED_NGSPICE_LIB. The in-core decks \
             produced {STEPS} finite samples each."
        );
        return;
    }

    for (index, celsius) in ["0", "25", "60"].iter().enumerate() {
        let set = format!("ui.ntc.temperature_c={celsius}");
        let theirs = cosim_series(&spiced, STEPS, &[&set], "ntc_divider", NTC_SIGNAL);
        assert_eq!(theirs.len() as u64, STEPS);
        let ours = &in_core_ntc[index];
        let worst = ours
            .iter()
            .zip(&theirs)
            .map(|(a, n)| (a - n).abs())
            .fold(0.0_f64, f64::max);
        assert!(
            worst < 1e-5,
            "{celsius} degC: in-core NTC {ours:?} V vs ngspice {theirs:?} V, \
             worst {worst:e} V"
        );
        eprintln!(
            "{celsius} degC NTC: in-core {:.9} V, ngspice {:.9} V, worst {worst:e} V",
            ours.last().unwrap(),
            theirs.last().unwrap()
        );
    }

    let theirs = cosim_series(&spiced, STEPS, &[], "diode_load_line", DIODE_SIGNAL);
    assert_eq!(theirs.len() as u64, STEPS);
    let worst = in_core_diode
        .iter()
        .zip(&theirs)
        .map(|(a, n)| (a - n).abs())
        .fold(0.0_f64, f64::max);
    assert!(
        worst < 1e-5,
        "diode load line: in-core {in_core_diode:?} V vs ngspice {theirs:?} V, \
         worst {worst:e} V"
    );
    eprintln!(
        "diode load line: in-core {:.9} V, ngspice {:.9} V, worst {worst:e} V",
        in_core_diode.last().unwrap(),
        theirs.last().unwrap()
    );

    let default_spiced = cosim_series(&spiced, STEPS, &[], "ntc_divider", NTC_SIGNAL);
    assert_eq!(default_spiced.len() as u64, STEPS);
    let worst = in_core_default
        .iter()
        .zip(&default_spiced)
        .map(|(a, n)| (a - n).abs())
        .fold(0.0_f64, f64::max);
    assert!(
        worst < 1e-5,
        "NTC default stimulus: in-core {in_core_default:?} V vs ngspice {default_spiced:?} V, \
         worst {worst:e} V; an absent input must be 25 degC on both engines"
    );
}

/// The same generic NTC and diode decks on the WCH CH32V003EVT, the only
/// non-ARM board in the analog inventory. The pair differs from the NUCLEO
/// pair only in the pads the descriptor bonds (PA2/PA1 instead of PA0/PA1);
/// the netlists and the ngspice decks are the generic ones. This proves
/// `adapter: ngspice` resolves `board.analog.<pad>_volts` through a second
/// ADC width and a non-ARM descriptor, not only the 12-bit STM32 map.
#[test]
fn ngspice_solves_the_wch_ch32v003_generic_decks_the_in_core_engine_solves() {
    const STEPS: u64 = 4;
    const NTC_SIGNAL: &str = "board.analog.pa2_volts";
    const DIODE_SIGNAL: &str = "board.analog.pa1_volts";
    const TEMPERATURES: [&str; 3] = ["-20", "25", "85"];
    let root = repo_root();
    let analog = root.join("examples/wch-ch32v003evt/analog-ntc-diode-system.yaml");
    let spiced = root.join("examples/wch-ch32v003evt/analog-ntc-diode-ngspice.yaml");

    // The in-core half always runs: a missing ngspice must not hide a deck the
    // engine can no longer solve.
    let mut in_core_ntc = Vec::new();
    for celsius in TEMPERATURES {
        let set = format!("ui.ntc.temperature_c={celsius}");
        let series = cosim_series(&analog, STEPS, &[&set], "ntc_divider", NTC_SIGNAL);
        assert_eq!(series.len() as u64, STEPS);
        assert!(
            series.iter().all(|volts| volts.is_finite()),
            "{celsius} degC: non-finite in-core sample"
        );
        in_core_ntc.push(series);
    }
    let in_core_diode = cosim_series(&analog, STEPS, &[], "diode_load_line", DIODE_SIGNAL);
    assert_eq!(in_core_diode.len() as u64, STEPS);

    // The temperatures must not agree with each other, or a differential on a
    // temperature-insensitive deck would pass while proving nothing.
    for pair in in_core_ntc.windows(2) {
        let (cold, warm) = (pair[0].last().unwrap(), pair[1].last().unwrap());
        assert!(
            (warm - cold).abs() > 0.1,
            "the NTC deck does not respond to temperature: {cold} V vs {warm} V"
        );
    }

    if !ngspice_available() {
        eprintln!(
            "SKIP: libngspice is not loadable on this host, so the ngspice half of the \
             CH32V003 differential cannot run. Install it (Debian/Ubuntu: \
             `apt install libngspice0`) or set LABWIRED_NGSPICE_LIB. The in-core decks \
             produced {STEPS} finite samples each."
        );
        return;
    }

    for (index, celsius) in TEMPERATURES.iter().enumerate() {
        let set = format!("ui.ntc.temperature_c={celsius}");
        let theirs = cosim_series(&spiced, STEPS, &[&set], "ntc_divider", NTC_SIGNAL);
        assert_eq!(theirs.len() as u64, STEPS);
        let ours = &in_core_ntc[index];
        let worst = ours
            .iter()
            .zip(&theirs)
            .map(|(a, n)| (a - n).abs())
            .fold(0.0_f64, f64::max);
        assert!(
            worst < 1e-5,
            "{celsius} degC: in-core NTC {ours:?} V vs ngspice {theirs:?} V, \
             worst {worst:e} V"
        );
        eprintln!(
            "{celsius} degC NTC: in-core {:.9} V, ngspice {:.9} V, worst {worst:e} V",
            ours.last().unwrap(),
            theirs.last().unwrap()
        );
    }

    let theirs = cosim_series(&spiced, STEPS, &[], "diode_load_line", DIODE_SIGNAL);
    assert_eq!(theirs.len() as u64, STEPS);
    let worst = in_core_diode
        .iter()
        .zip(&theirs)
        .map(|(a, n)| (a - n).abs())
        .fold(0.0_f64, f64::max);
    assert!(
        worst < 1e-5,
        "diode load line: in-core {in_core_diode:?} V vs ngspice {theirs:?} V, \
         worst {worst:e} V"
    );
    eprintln!(
        "CH32V003 diode load line: in-core {:.9} V, ngspice {:.9} V, worst {worst:e} V",
        in_core_diode.last().unwrap(),
        theirs.last().unwrap()
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
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-analog-subckt");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(
        dir.join("diode.cir"),
        // A subcircuit call: still outside the in-core subset now that `D` is
        // inside it. Subcircuits need a model library and a flattener, which
        // is exactly what the ngspice wrapper is for.
        "* a subcircuit is outside the in-core subset\n\
         Vin in 0 dc 0\nR1 in out 1k\nX1 out 0 opamp\n.end\n",
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

// ---------------------------------------------------------------------------
// Devices and reactive elements: the same deck, both engines
// ---------------------------------------------------------------------------

/// Tolerance on every sample of these differentials, as a fraction of the
/// circuit's full scale.
///
/// Full scale rather than the sample: near a zero crossing a relative-to-sample
/// error is unbounded and meaningless, and what a user sees is an ADC reading
/// against a rail.
///
/// **5e-4 (0.05 %), not the 2 % a differential like this is usually written
/// with.** 2 % was measured to be vacuous here. The decks below actually
/// agree with ngspice to 2.5e-3 % (rectifier), 4.0e-5 % (common emitter),
/// 1.0e-6 % (NMOS inverter), 1.1e-3 % (RL low-pass), 6.1e-5 % (JFET
/// follower), 1.2e-3 % (pulsed sag into an RC), 7.3e-3 % (pulsed sag into a
/// bypassed load), 5.9e-4 % (coupled-inductor transformer), 8.0e-5 % (PNP
/// follower), 2.6e-3 % (CMOS inverter) and 6.1e-5 % (P-channel JFET
/// follower) of full scale, and perturbing the engine's thermal voltage by
/// 1 % — a wrong physical constant, the exact class of bug this test exists
/// to catch — moves the rectifier by 0.13 % and the amplifier by 0.24 %. At
/// 2 % that sabotage passes. At 0.05 % it fails with 2.5x margin, while still
/// leaving 7x headroom over the worst honest disagreement, which is the
/// fixed-step-versus-adaptive-step difference between the two integrators.
/// The common-emitter deck is sine-driven: it used to restart trapezoidal on
/// every sample, so its 2.6e-5 % was measured under backward Euler; with the
/// corner rule it keeps trapezoidal and measures 4.0e-5 %. The 1 %
/// thermal-voltage sabotage still moves it by 0.24 %, 4.8x the tolerance.
/// The RL deck's 1.1e-3 % is the start-up sample, where one backward-Euler
/// step follows the initial operating point; after the first 200 samples it is
/// 2.0e-4 %.
///
/// The two pulsed-sag decks are the pulse-driven pair, and they are the ones
/// that hold the corner rule: restarting backward Euler wherever a source
/// changes value — the rule before it — measured 6.5e-2 % and 9.8e-2 % on
/// them, both over the tolerance. Trapezoidal across the same ramps measures
/// 1.2e-3 % and 7.3e-3 %, and that is the direction the browser's
/// `analog-regulator/native.json` moved when it was regenerated.
///
/// The MOSFET deck is the weak one and is named as such: a level-1 FET has no
/// thermal voltage, so that sabotage moves it by 1e-6 %. Its guard is the
/// operating-point hand calculation in
/// `crates/core/tests/analog/analog_devices.rs`, which checks `VTO`, `KP` and
/// `LAMBDA` against the closed form directly.
const DEVICE_TOLERANCE: f64 = 5e-4;

/// One deck, run by both engines, compared at every sample.
struct Differential {
    /// What the failure message calls it.
    name: &'static str,
    /// SPICE text. Both engines parse this exact string; the in-core engine
    /// ignores the `.model` parameters it has no term for and the analysis
    /// cards, and ngspice ignores nothing.
    deck: &'static str,
    /// Node to compare.
    node: &'static str,
    /// Sample interval, seconds.
    sample: f64,
    /// Internal solver steps per sample. ngspice is told the same number as
    /// its maximum step, so neither engine is integrating on a coarser grid
    /// than the other.
    substeps: u32,
    /// Samples to compare.
    samples: usize,
    /// Denominator of the error, volts.
    full_scale: f64,
    /// The compared trace must swing at least this far, or the differential is
    /// agreeing about a flat line and proving nothing.
    minimum_swing: f64,
    /// True when the deck holds a device stamp (D/Q/M/J) and therefore takes
    /// the Newton–Raphson path. False decks cover the linear stamps — in
    /// particular the inductor, which carries its own branch unknown.
    nonlinear: bool,
}

const DIFFERENTIALS: &[Differential] = &[
    Differential {
        name: "half-wave rectifier",
        deck: "* half-wave rectifier: D + R + VSIN\n\
               Vin in 0 SIN(0 5 1k)\n\
               D1 in out DMOD\n\
               R1 out 0 1k\n\
               .model DMOD D(IS=2.52n N=1.752 RS=0.568)\n",
        node: "out",
        sample: 20e-6,
        substeps: 20,
        samples: 100,
        full_scale: 5.0,
        minimum_swing: 4.0,
        nonlinear: true,
    },
    Differential {
        name: "common-emitter amplifier",
        // The coupling capacitor is not decoration: without it the ideal
        // source sits across the bias divider through its own impedance and
        // pulls the base to 0.23 V, and the stage is cut off. Both engines
        // agree about that to seven digits, which is exactly why the swing
        // assertion below exists — a differential on a dead circuit passes.
        deck: "* common-emitter amplifier: Q + resistors + VDC + VSIN\n\
               Vcc vcc 0 dc 12\n\
               Vin in 0 SIN(0 0.25 1k)\n\
               Cin in b 10u\n\
               Rb1 vcc b 47k\n\
               Rb2 b 0 10k\n\
               Rc vcc c 2.2k\n\
               Re e 0 470\n\
               Q1 c b e QMOD\n\
               .model QMOD NPN(IS=1e-14 BF=200 BR=2 NF=1 NR=1 VAF=80 VAR=120)\n",
        node: "c",
        sample: 20e-6,
        substeps: 20,
        samples: 100,
        full_scale: 12.0,
        minimum_swing: 1.0,
        nonlinear: true,
    },
    Differential {
        name: "NMOS inverter",
        deck: "* NMOS inverter: M + R + VPULSE\n\
               Vdd vdd 0 dc 5\n\
               Vg g 0 PULSE(0 5 2u 200n 200n 18u 40u)\n\
               Rd vdd d 10k\n\
               M1 d g 0 0 MMOD\n\
               .model MMOD NMOS(VTO=1 KP=20u LAMBDA=0.02 W=20u L=2u)\n",
        node: "d",
        sample: 200e-9,
        substeps: 10,
        samples: 400,
        full_scale: 5.0,
        minimum_swing: 4.0,
        nonlinear: true,
    },
    Differential {
        name: "RL low-pass",
        // Driven at the corner (R = ωL = 100 Ω), so the inductor's branch
        // unknown carries a real amplitude and phase, not a small correction.
        deck: "* RL low-pass at its corner: L + R + VSIN\n\
               Vin in 0 SIN(0 1 1591.55)\n\
               L1 in out 10m\n\
               R1 out 0 100\n",
        node: "out",
        sample: 10e-6,
        substeps: 20,
        samples: 400,
        full_scale: 1.0,
        minimum_swing: 1.0,
        nonlinear: false,
    },
    Differential {
        name: "JFET source follower",
        deck: "* JFET source follower: J + R + VDC + VSIN\n\
               Vdd vdd 0 dc 12\n\
               Vin in 0 SIN(0 2 1k)\n\
               Rg in g 100k\n\
               J1 vdd g s JMOD\n\
               Rs s 0 1k\n\
               .model JMOD NJF(VTO=-2 BETA=1m LAMBDA=0.02)\n",
        node: "s",
        sample: 20e-6,
        substeps: 20,
        samples: 100,
        full_scale: 12.0,
        minimum_swing: 0.5,
        nonlinear: true,
    },
    Differential {
        name: "pulsed sag into an RC",
        // The drive the regulator browser fixture records: a 9 V rail that
        // steps to 3.6 V and back once a millisecond, with 50 µs edges — ten
        // steps, so each edge is a ramp and not a corner — into a 200 µs RC.
        // The trapezoidal rule carries the ramp's derivative across it. A
        // solver that restarted backward Euler on every value change measured
        // 6.5e-2 % here, over the tolerance; this deck is what holds the
        // corner rule.
        deck: "* sagged 9 V rail into a 200 us RC: PULSE + R + C\n\
               Vin in 0 PULSE(9 3.6 200u 50u 50u 500u 1m)\n\
               R1 in out 20\n\
               C1 out 0 10u\n\
               R2 out 0 330\n",
        node: "out",
        sample: 5e-6,
        substeps: 10,
        samples: 800,
        full_scale: 9.0,
        minimum_swing: 4.0,
        nonlinear: false,
    },
    Differential {
        name: "pulsed sag into a bypassed load",
        // The same drive into the 11 µs corner a 22 µF bypass and a 0.5 Ω
        // feed make. This is the shape whose inrush current the regulator
        // fixture records, and the stiffer corner is where a first-order rule
        // loses the most: backward Euler measured 9.8e-2 % here.
        deck: "* sagged 9 V rail into a bypassed load: PULSE + R + C\n\
               Vin in 0 PULSE(9 3.6 200u 50u 50u 500u 1m)\n\
               R1 in out 0.5\n\
               C1 out 0 22u\n\
               R2 out 0 5\n",
        node: "out",
        sample: 5e-6,
        substeps: 10,
        samples: 800,
        full_scale: 9.0,
        minimum_swing: 4.0,
        nonlinear: false,
    },
    Differential {
        name: "coupled-inductor transformer",
        // The mutual term is the only path to the secondary, so a missing or
        // sign-flipped `K` stamp leaves `sec` flat or inverted. k = 0.95, not
        // 1: a perfect coupling makes the inductance matrix singular.
        deck: "* 1:2 transformer at k = 0.95: K + L + R + VSIN\n\
               Vin in 0 SIN(0 1 1k)\n\
               Rp in pri 10\n\
               L1 pri 0 10m\n\
               L2 sec 0 40m\n\
               K1 L1 L2 0.95\n\
               Rl sec 0 1k\n",
        node: "sec",
        sample: 10e-6,
        substeps: 20,
        samples: 300,
        full_scale: 2.0,
        minimum_swing: 1.0,
        nonlinear: false,
    },
    Differential {
        name: "PNP emitter follower",
        // The mirrored polarity of the common-emitter deck: the emitter sits
        // one V_BE above the base and follows it.
        deck: "* PNP emitter follower: Q(PNP) + R + VDC + VSIN\n\
               Vee vee 0 dc -12\n\
               Vin in 0 SIN(-6 2 1k)\n\
               Rb in b 1k\n\
               Re 0 e 4.7k\n\
               Q1 vee b e QMOD\n\
               .model QMOD PNP(IS=1e-14 BF=150 BR=2 NF=1 NR=1 VAF=60 VAR=100)\n",
        node: "e",
        sample: 20e-6,
        substeps: 20,
        samples: 100,
        full_scale: 12.0,
        minimum_swing: 3.0,
        nonlinear: true,
    },
    Differential {
        name: "CMOS inverter",
        // A PMOS pull-up against an NMOS pull-down: both channels conduct
        // through the transition, so the output is set by the two level-1
        // curves against each other, not by one FET against a resistor.
        // 40 substeps, not 10: the 10 pF node slews fast mid-transition, and
        // at a 10 ns step both integrators measured 4.5e-2 % apart, inside the
        // tolerance by only 1.1x. At 2.5 ns they agree to 2.6e-3 %, so the
        // 10 ns gap was the step, not the device equations.
        deck: "* CMOS inverter: M(PMOS) + M(NMOS) + VPULSE + C\n\
               Vdd vdd 0 dc 5\n\
               Vg g 0 PULSE(0 5 2u 2u 2u 8u 20u)\n\
               M1 d g vdd vdd PMOD\n\
               M2 d g 0 0 NMOD\n\
               Cl d 0 10p\n\
               .model PMOD PMOS(VTO=-1 KP=10u LAMBDA=0.02 W=40u L=2u)\n\
               .model NMOD NMOS(VTO=1 KP=20u LAMBDA=0.02 W=20u L=2u)\n",
        node: "d",
        sample: 100e-9,
        substeps: 40,
        samples: 400,
        full_scale: 5.0,
        minimum_swing: 4.0,
        nonlinear: true,
    },
    Differential {
        name: "P-channel JFET source follower",
        deck: "* PJF source follower: J(PJF) + R + VDC + VSIN\n\
               Vss vss 0 dc -12\n\
               Vin in 0 SIN(0 2 1k)\n\
               Rg in g 100k\n\
               J1 vss g s JMOD\n\
               Rs s 0 1k\n\
               .model JMOD PJF(VTO=-2 BETA=1m LAMBDA=0.02)\n",
        node: "s",
        sample: 20e-6,
        substeps: 20,
        samples: 100,
        full_scale: 12.0,
        minimum_swing: 0.5,
        nonlinear: true,
    },
    Differential {
        name: "hysteretic voltage switch",
        // The control swings through both edges of the `VT ± VH` band, and
        // its negative half must keep the switch open: a magnitude-sensing
        // switch would close there and fail. The in-core engine switches at
        // a substep boundary and ngspice cuts its step at the crossing, so
        // they may disagree by one substep. The 10 ms charging time constant
        // keeps that offset well under the tolerance; at 1 ms it was 0.14 %.
        deck: "* voltage switch: S(SW) + R + C + VDC + VSIN\n\
               Vdd vdd 0 dc 5\n\
               Vc c 0 SIN(0 2 1k)\n\
               S1 vdd out c 0 SMOD\n\
               Rl out 0 10k\n\
               C1 out 0 10u\n\
               .model SMOD SW(VT=0.5 VH=0.3 RON=1k ROFF=1e9)\n",
        node: "out",
        sample: 20e-6,
        substeps: 20,
        samples: 500,
        full_scale: 5.0,
        minimum_swing: 0.5,
        nonlinear: false,
    },
    Differential {
        name: "hysteretic current switch",
        // The `W` twin of the row above: the switch senses the branch
        // current of `Vsense`, 2 mA peak through `Rs`, against the
        // `IT ± IH` band. Its negative half must keep the switch open, so a
        // flipped branch-current sign or a magnitude rule fails this row.
        deck: "* current switch: W(CSW) + R + C + VDC + VSIN\n\
               Vdd vdd 0 dc 5\n\
               Vc c 0 SIN(0 2 1k)\n\
               Rs c s 1k\n\
               Vsense s 0 dc 0\n\
               W1 vdd out Vsense WMOD\n\
               Rl out 0 10k\n\
               C1 out 0 10u\n\
               .model WMOD CSW(IT=0.5m IH=0.3m RON=1k ROFF=1e9)\n",
        node: "out",
        sample: 20e-6,
        substeps: 20,
        samples: 500,
        full_scale: 5.0,
        minimum_swing: 0.5,
        nonlinear: false,
    },
    Differential {
        name: "voltage switch initial state ON",
        // The control starts inside the `VT ± VH` band, so only the `ON`
        // keyword closes the switch at the operating point; without it the
        // output stays near 0 V. At 2 ms the control leaves the band below
        // and the switch opens; its return to the band at 5 ms keeps it open.
        deck: "* voltage switch ON: S(SW) ON + R + C + VDC + VPULSE\n\
               Vdd vdd 0 dc 5\n\
               Vc c 0 PULSE(0.5 -1 2m 10u 10u 3m 8m)\n\
               S1 vdd out c 0 SMOD ON\n\
               Rl out 0 10k\n\
               C1 out 0 1u\n\
               .model SMOD SW(VT=0.5 VH=0.3 RON=1k ROFF=1e9)\n",
        node: "out",
        sample: 20e-6,
        substeps: 20,
        samples: 400,
        full_scale: 5.0,
        minimum_swing: 0.5,
        nonlinear: false,
    },
    Differential {
        name: "voltage switch ON exactly at lower hysteresis edge",
        // ngspice preserves an ON seed at exactly VT − VH. The pulse then
        // falls below the lower edge to open the switch, and returns to that
        // edge while open. This pins the inclusive initial and transient
        // boundary behavior against the real ngspice transient.
        deck: "* voltage switch ON at lower edge: S(SW) + R + C + VPULSE\n\
               Vdd vdd 0 dc 5\n\
               Vc c 0 PULSE(0.5 -1 2m 10u 10u 3m 8m)\n\
               S1 vdd out c 0 SMOD ON\n\
               Rl out 0 10k\n\
               C1 out 0 1u\n\
               .model SMOD SW(VT=1 VH=0.5 RON=1 ROFF=1e9)\n",
        node: "out",
        sample: 20e-6,
        substeps: 20,
        samples: 400,
        full_scale: 5.0,
        minimum_swing: 0.5,
        nonlinear: false,
    },
];

/// Solve `deck` with the in-core engine, sampling `node` at every boundary.
fn in_core_series(case: &Differential) -> Vec<f64> {
    let circuit = labwired_core::analog::parse_netlist(case.deck)
        .unwrap_or_else(|error| panic!("{}: {error}", case.name));
    assert_eq!(
        circuit.is_nonlinear(),
        case.nonlinear,
        "{}: the deck and its declared `nonlinear` flag disagree",
        case.name
    );
    let mut solver = labwired_core::analog::Solver::new(
        circuit,
        labwired_core::analog::Integration::Trapezoidal,
    )
    .unwrap_or_else(|error| panic!("{}: {error}", case.name));
    let node = solver
        .circuit()
        .node(case.node)
        .unwrap_or_else(|| panic!("{}: no node `{}`", case.name, case.node));

    let h = case.sample / f64::from(case.substeps);
    let mut series = Vec::with_capacity(case.samples);
    for sample in 0..case.samples {
        for substep in 0..case.substeps {
            solver.advance(h).unwrap_or_else(|error| {
                panic!("{}: sample {sample} substep {substep}: {error}", case.name)
            });
        }
        series.push(solver.node_voltage(node));
    }
    assert_finite_samples(case.name, &series);
    series
}

/// Solve the same deck with ngspice, resampled onto the same uniform grid.
///
/// `linearize` is what makes the comparison honest: ngspice runs its own
/// adaptive-step transient and would otherwise report its own time points, so
/// a comparison would be measuring interpolation rather than physics. The
/// maximum internal step is pinned to the in-core substep, so ngspice is not
/// given a finer integration than the engine under test.
fn ngspice_series(case: &Differential) -> Vec<f64> {
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-ngspice-diff");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let data = dir.join("out.data");
    let cir = dir.join("deck.cir");
    let tmax = case.sample / f64::from(case.substeps);
    let tstop = case.sample * case.samples as f64;
    std::fs::write(
        &cir,
        format!(
            "{deck}\
             .options temp=26.85 tnom=26.85 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12\n\
             .control\n\
             tran {sample:e} {tstop:e} 0 {tmax:e}\n\
             linearize v({node})\n\
             wrdata {data} v({node})\n\
             .endc\n\
             .end\n",
            deck = case.deck,
            sample = case.sample,
            tstop = tstop,
            tmax = tmax,
            node = case.node,
            data = data.display(),
        ),
    )
    .expect("write deck");

    let output = Command::new("ngspice")
        .arg("-b")
        .arg(&cir)
        .current_dir(&dir)
        .output()
        .expect("ngspice runs");
    assert!(
        output.status.success(),
        "{}: ngspice failed: {}",
        case.name,
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&data).unwrap_or_else(|error| {
        panic!(
            "{}: ngspice wrote no data ({error}); stdout was {}",
            case.name,
            String::from_utf8_lossy(&output.stdout)
        )
    });
    // `wrdata` writes one `<time> <value>` pair per line. The first row is the
    // operating point at t = 0, which the in-core series does not include
    // because it reports the END of each step.
    let series = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let _time = fields.next()?;
            fields.next()?.parse::<f64>().ok()
        })
        .skip(1)
        .collect::<Vec<_>>();
    assert_finite_samples(&format!("{} ngspice", case.name), &series);
    series
}

/// True when the `ngspice` binary is on PATH.
fn ngspice_binary_available() -> bool {
    require_backend_if_configured(
        Command::new("ngspice")
            .arg("-v")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false),
        "the ngspice executable",
    )
}

#[test]
fn required_ngspice_reference_cannot_be_skipped() {
    assert_eq!(backend_availability(false, "libngspice", false), Ok(false));
    assert_eq!(backend_availability(true, "libngspice", true), Ok(true));
    assert!(backend_availability(false, "libngspice", true).is_err());
}

#[test]
fn nonlinear_devices_match_ngspice_on_the_same_deck() {
    if !ngspice_binary_available() {
        // Still exercise the in-core half, so a missing ngspice cannot hide a
        // solver that stopped converging.
        for case in DIFFERENTIALS {
            let series = in_core_series(case);
            assert_eq!(series.len(), case.samples);
            assert!(series.iter().all(|v| v.is_finite()));
        }
        eprintln!(
            "SKIP: `ngspice` is not on PATH, so the reference half of the \
             device differential cannot run. Install it (macOS: `brew install ngspice`; \
             Debian/Ubuntu: `apt install ngspice`). The in-core engine solved all \
             {} decks and is covered on its own by \
             `cargo test -p labwired-core --test analog analog_devices::`.",
            DIFFERENTIALS.len()
        );
        return;
    }

    for case in DIFFERENTIALS {
        let ours = in_core_series(case);
        let theirs = ngspice_series(case);
        assert!(
            theirs.len() >= case.samples,
            "{}: ngspice returned {} samples, wanted {}",
            case.name,
            theirs.len(),
            case.samples
        );

        let swing = {
            let max = ours.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let min = ours.iter().cloned().fold(f64::INFINITY, f64::min);
            max - min
        };
        assert!(
            swing >= case.minimum_swing,
            "{}: the trace only swings {swing} V, so agreeing with ngspice about it \
             proves nothing; expected at least {} V",
            case.name,
            case.minimum_swing
        );

        let mut worst = 0.0_f64;
        let mut worst_sample = 0;
        for (index, (a, n)) in ours.iter().zip(theirs.iter()).enumerate() {
            let error = (a - n).abs() / case.full_scale;
            if error > worst {
                worst = error;
                worst_sample = index;
            }
        }
        let (a, n) = (ours[worst_sample], theirs[worst_sample]);
        assert!(
            worst < DEVICE_TOLERANCE,
            "{}: worst disagreement at sample {} (t = {:e} s): in-core {a} V vs \
             ngspice {n} V — {:.3} % of {} V full scale",
            case.name,
            worst_sample + 1,
            (worst_sample as f64 + 1.0) * case.sample,
            worst * 100.0,
            case.full_scale
        );
        eprintln!(
            "{}: {} samples, swing {swing:.3} V, worst |delta| = {:e} % of full scale \
             (sample {}: in-core {a} V vs ngspice {n} V)",
            case.name,
            case.samples,
            worst * 100.0,
            worst_sample + 1
        );
    }
}
