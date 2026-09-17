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
// Nonlinear devices: the same deck, both engines
// ---------------------------------------------------------------------------

/// Tolerance on every sample of the nonlinear differentials, as a fraction of
/// the circuit's full scale.
///
/// Full scale rather than the sample: near a zero crossing a relative-to-sample
/// error is unbounded and meaningless, and what a user sees is an ADC reading
/// against a rail.
///
/// **5e-4 (0.05 %), not the 2 % a differential like this is usually written
/// with.** 2 % was measured to be vacuous here. The three decks below actually
/// agree with ngspice to 2.5e-3 %, 2.6e-5 % and 1.0e-6 % of full scale, and
/// perturbing the engine's thermal voltage by 1 % — a wrong physical constant,
/// the exact class of bug this test exists to catch — moves the rectifier by
/// 0.13 % and the amplifier by 0.24 %. At 2 % that sabotage passes. At 0.05 %
/// it fails with 2.5x margin, while still leaving 20x headroom over the worst
/// honest disagreement, which is the fixed-step-versus-adaptive-step
/// difference between the two integrators.
///
/// The MOSFET deck is the weak one and is named as such: a level-1 FET has no
/// thermal voltage, so that sabotage moves it by 1e-6 %. Its guard is the
/// operating-point hand calculation in
/// `crates/core/tests/analog_devices.rs`, which checks `VTO`, `KP` and
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
               .model QMOD NPN(IS=1e-14 BF=200 BR=2 NF=1 NR=1)\n",
        node: "c",
        sample: 20e-6,
        substeps: 20,
        samples: 100,
        full_scale: 12.0,
        minimum_swing: 1.0,
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
    },
];

/// Solve `deck` with the in-core engine, sampling `node` at every boundary.
fn in_core_series(case: &Differential) -> Vec<f64> {
    let circuit = labwired_core::analog::parse_netlist(case.deck)
        .unwrap_or_else(|error| panic!("{}: {error}", case.name));
    assert!(
        circuit.is_nonlinear(),
        "{}: this differential is about a nonlinear device",
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
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let _time = fields.next()?;
            fields.next()?.parse::<f64>().ok()
        })
        .skip(1)
        .collect()
}

/// True when the `ngspice` binary is on PATH.
fn ngspice_binary_available() -> bool {
    Command::new("ngspice")
        .arg("-v")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
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
             `cargo test -p labwired-core --test analog_devices`.",
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
