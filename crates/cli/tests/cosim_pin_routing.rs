// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `labwired test` steps a manifest's `cosim_models` in lockstep with the
//! firmware, routing pins in both directions.
//!
//! The unit tests in `cosim::routing` cover the grammar, and
//! `crates/core/tests/cosim/cosim_pin_routing.rs` covers the routing against a real
//! bus. Neither can catch the thing this file exists for: that the run loop
//! actually CALLS any of it. Before the hook existed, every one of those tests
//! passed while `labwired test` stepped no model at all.
//!
//! The firmware is the committed NUCLEO-F401RE Arduino blink image, which
//! drives PA5 (LD2) — so the sampled input is a level the firmware really
//! produced, not one the test poked in.

use sha2::Digest;
use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

struct Run {
    status: Option<i32>,
    stderr: String,
    result: serde_json::Value,
}

/// Run `labwired test` over a temp-dir manifest + script, with the co-sim
/// routing log turned on.
fn run(name: &str, system_yaml: &str, script_yaml: &str) -> Run {
    run_firmware(
        name,
        "tests/fixtures/stm32f401-blinky.elf",
        system_yaml,
        script_yaml,
    )
}

/// [`run`] with another committed firmware image.
fn run_firmware(name: &str, firmware_rel: &str, system_yaml: &str, script_yaml: &str) -> Run {
    run_with_args(name, firmware_rel, system_yaml, script_yaml, &[])
}

/// [`run_firmware`] with extra `labwired test` arguments.
fn run_with_args(
    name: &str,
    firmware_rel: &str,
    system_yaml: &str,
    script_yaml: &str,
    extra: &[&str],
) -> Run {
    run_with_files(name, firmware_rel, system_yaml, script_yaml, extra, &[])
}

/// [`run_with_args`] with `(name, contents)` files written beside
/// `system.yaml`, for models whose `model:` must stay inside its directory.
fn run_with_files(
    name: &str,
    firmware_rel: &str,
    system_yaml: &str,
    script_yaml: &str,
    extra: &[&str],
    files: &[(&str, &str)],
) -> Run {
    let temp_dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-cosim-{name}"));
    std::fs::create_dir_all(&temp_dir).unwrap();
    for (file, contents) in files {
        std::fs::write(temp_dir.join(file), contents).unwrap();
    }

    let firmware = workspace_root().join(firmware_rel);
    assert!(
        firmware.exists(),
        "firmware fixture not found at {firmware:?}"
    );

    std::fs::write(temp_dir.join("system.yaml"), system_yaml).unwrap();
    let script_path = temp_dir.join("script.yaml");
    std::fs::write(
        &script_path,
        script_yaml.replace("__FIRMWARE__", &firmware.display().to_string()),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .arg("test")
        .arg("--script")
        .arg(&script_path)
        .arg("--output-dir")
        .arg(&temp_dir)
        .arg("--no-uart-stdout")
        .args(extra)
        // The routed values are the evidence; without the `cosim` directive
        // they are filtered out before they reach stderr. `info` keeps the
        // runner's own default level, which a bare `cosim=debug` would drop.
        .env("RUST_LOG", "info,cosim=debug")
        .output()
        .expect("failed to run labwired");

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let result_path = temp_dir.join("result.json");
    let result: serde_json::Value = if result_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&result_path).unwrap())
            .expect("parse result.json")
    } else {
        serde_json::Value::Null
    };
    let run = Run {
        status: output.status.code(),
        stderr,
        result,
    };
    let _ = std::fs::remove_dir_all(&temp_dir);
    run
}

/// A model whose static output is routed onto PC13's input register, and whose
/// input is sourced from the LED pad the firmware drives.
const ROUTED_SYSTEM: &str = r#"
name: "cosim-pin-routing"
chip: "stm32f401"
cosim_models:
  - id: "pin_probe"
    adapter: "mock"
    step_ns: 100000
    inputs:
      led: "board.gpio.pa5"
    outputs:
      contact: "board.gpio_in.pc13"
      node_volts: "board.analog.pa0_volts"
    config:
      outputs:
        contact: true
        node_volts: 1.65
external_devices: []
"#;

/// GPIOC IDR on an F401 is base 0x40020800 + 0x10; bit 13 is PC13.
const ROUTED_SCRIPT: &str = r#"
schema_version: "1.2"
inputs:
  firmware: "__FIRMWARE__"
  system: "./system.yaml"
limits:
  max_steps: 1500000
assertions:
  - uart_contains: "LED ON"
  - memory_value:
      address: 0x40020810
      expected_value: 0x2000
      mask: 0x2000
"#;

/// Both directions, through the real `labwired test` loop: the model's output
/// reaches the GPIO input register the firmware would sample, and the pad the
/// firmware drives reaches the model.
#[test]
fn labwired_test_routes_pins_in_both_directions() {
    let run = run("both-ways", ROUTED_SYSTEM, ROUTED_SCRIPT);

    assert_eq!(
        run.result["status"], "pass",
        "run did not pass.\nresult: {}\nstderr: {}",
        run.result, run.stderr
    );
    assert_eq!(run.status, Some(0));

    // Machine → model: the Arduino sketch drives LD2 high in loop(), and that
    // is the level the model was handed.
    assert!(
        run.stderr.contains("board.gpio.pa5 = true"),
        "PA5 high never reached the model.\nstderr: {}",
        run.stderr
    );
    // Model → machine, analog: the routed volts were applied with no error.
    assert!(
        run.stderr.contains("board.analog.pa0_volts = 1.65"),
        "the analog route never fired.\nstderr: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("co-sim path"),
        "a routed path failed at runtime.\nstderr: {}",
        run.stderr
    );
}

/// A pad that does not exist on the chip fails the run at startup. A co-sim
/// whose pin never reached the firmware must not be able to print a pass.
#[test]
fn an_unroutable_pad_fails_the_run_as_a_config_error() {
    const BAD_SYSTEM: &str = r#"
name: "cosim-bad-pad"
chip: "stm32f401"
cosim_models:
  - id: "pin_probe"
    adapter: "mock"
    step_ns: 100000
    inputs:
      led: "board.gpio.pz9"
    outputs: {}
    config:
      outputs: {}
external_devices: []
"#;
    const SCRIPT: &str = r#"
schema_version: "1.2"
inputs:
  firmware: "__FIRMWARE__"
  system: "./system.yaml"
limits:
  max_steps: 50000
assertions:
  - uart_contains: "LabWired"
"#;
    let run = run("bad-pad", BAD_SYSTEM, SCRIPT);
    assert_eq!(
        run.status,
        Some(2),
        "expected the config-error exit.\nstderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("pad 'pz9' does not resolve"),
        "the failure must name the pad.\nstderr: {}",
        run.stderr
    );
}

/// A model output routed onto a pad the FIRMWARE drives is a config error, not
/// a write that silently does nothing — the manifest author meant
/// `board.gpio_in.<pad>`.
#[test]
fn driving_a_firmware_output_pad_is_rejected() {
    const BAD_SYSTEM: &str = r#"
name: "cosim-bad-direction"
chip: "stm32f401"
cosim_models:
  - id: "pin_probe"
    adapter: "mock"
    step_ns: 100000
    inputs: {}
    outputs:
      contact: "board.gpio.pc13"
    config:
      outputs:
        contact: true
external_devices: []
"#;
    const SCRIPT: &str = r#"
schema_version: "1.2"
inputs:
  firmware: "__FIRMWARE__"
  system: "./system.yaml"
limits:
  max_steps: 50000
assertions:
  - uart_contains: "LabWired"
"#;
    let run = run("bad-direction", BAD_SYSTEM, SCRIPT);
    assert_eq!(run.status, Some(2), "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("board.gpio_in.<pad>"),
        "the failure must point at the writable form.\nstderr: {}",
        run.stderr
    );
}

/// An Arduino Nano whose PD2 sits on a circuit node driven by a test-script
/// voltage: `ui.bench.level` feeds a source, 1 k / 1 M put 99.9 % of it on the
/// pad, and the pad voltage reaches PD2 through the ATmega's thresholds.
///
/// The firmware is the committed Nano blink sketch, so this is also the AVR run
/// going through the same lockstep co-simulation advance the ARM runs use.
const NANO_BENCH_SYSTEM: &str = r#"
name: "cosim-signal-nano"
chip: "__CHIP__"
cosim_models:
  - id: "circuit"
    adapter: "analog"
    step_ns: 100000
    inputs:
      level: "ui.bench.level"
      led: "board.gpio.pb5"
    outputs:
      in_pd2: "board.gpio_in.pd2"
    config:
      vdd: 5.0
      netlist_text: |
        Vbench in 0 dc 0
        R1 in pad 1k
        R2 pad 0 1meg
      probes: { in_pd2: "v(pad)" }
      sources: { level: Vbench }
external_devices: []
"#;

/// The model's routed PD2 voltage at every boundary, from the `cosim` log.
fn routed_pd2_volts(stderr: &str) -> Vec<(u64, f64)> {
    stderr
        .lines()
        .filter_map(|line| {
            let rest = line.split("circuit -> board.gpio_in.pd2 = ").nth(1)?;
            let (volts, rest) = rest.split_once(" (cycle=")?;
            let cycle = rest.split(')').next()?;
            Some((cycle.parse().ok()?, volts.trim().parse().ok()?))
        })
        .collect()
}

/// A `cosim_signal` stimulus lands in the session's signal store when its
/// trigger fires, and the model sees it at the first boundary after that — not
/// before, and not a period late. The pad then reads high on PIND, which the
/// `memory_value` assertion checks through the PORTD window.
#[test]
fn a_cosim_signal_stimulus_reaches_the_model_at_the_next_boundary() {
    const SCRIPT: &str = r#"
schema_version: "1.2"
inputs:
  firmware: "__FIRMWARE__"
  system: "./system.yaml"
limits:
  max_steps: 240000
stimuli:
  - cosim_signal: { path: ui.bench.level, value: 5 }
    trigger: !after_cycles { cycles: 120000 }
assertions:
  # PIND as the bus-side PORTD window holds it: PD2 (bit 2) is an input the
  # circuit now holds high.
  - memory_value:
      address: 0x00010029
      expected_value: 0x04
      mask: 0x04
"#;
    let chip = workspace_root().join("configs/chips/atmega328p.yaml");
    let system = NANO_BENCH_SYSTEM.replace("__CHIP__", &chip.display().to_string());
    let run = run_firmware(
        "signal-timing",
        "tests/fixtures/avr/arduino-nano-blinky.elf",
        &system,
        SCRIPT,
    );
    assert_eq!(
        run.result["status"], "pass",
        "run did not pass.
result: {}
stderr: {}",
        run.result, run.stderr
    );

    let stimuli = run.result["stimuli"].as_array().expect("a stimuli block");
    assert_eq!(stimuli.len(), 1, "{}", run.result);
    let stimulus = &stimuli[0];
    assert_eq!(stimulus["outcome"], "applied", "{stimulus}");
    assert_eq!(stimulus["cosim_signal"], true, "{stimulus}");
    assert_eq!(stimulus["channel"], "ui.bench.level", "{stimulus}");
    let applied_at = stimulus["at_cycle"].as_u64().expect("at_cycle");
    assert!(applied_at > 0, "{stimulus}");

    let volts = routed_pd2_volts(&run.stderr);
    assert!(
        volts.len() > 10,
        "no routed boundary in the log:\n{}",
        run.stderr
    );
    // 16 MHz: a 100 us boundary is 1600 cycles, and the machine lands on a
    // boundary at most one AVR step (3 cycles) past it.
    let boundary_cycles = 1_600;
    let one_step_late = 3;
    for (cycle, v) in &volts {
        if *cycle <= applied_at {
            assert!(
                *v < 0.01,
                "the model saw the signal at cycle {cycle}, before it was set at {applied_at}"
            );
        }
    }
    let (first_high, v) = *volts
        .iter()
        .find(|(_, v)| *v > 4.9)
        .unwrap_or_else(|| panic!("the pad never rose:\n{volts:?}"));
    assert!(
        first_high > applied_at && first_high <= applied_at + boundary_cycles + one_step_late,
        "set at cycle {applied_at}, first seen at cycle {first_high} ({v} V): not the next \
         boundary"
    );
}

/// A `cosim_signal` that no model reads never reached anything, so the run is
/// invalid rather than a pass.
#[test]
fn a_cosim_signal_no_model_reads_is_a_rejected_stimulus() {
    const SCRIPT: &str = r#"
schema_version: "1.2"
inputs:
  firmware: "__FIRMWARE__"
  system: "./system.yaml"
limits:
  max_steps: 20000
stimuli:
  - cosim_signal: { path: ui.bench.levle, value: 5 }
assertions:
  - expected_stop_reason: max_steps
"#;
    let chip = workspace_root().join("configs/chips/atmega328p.yaml");
    let system = NANO_BENCH_SYSTEM.replace("__CHIP__", &chip.display().to_string());
    let run = run_firmware(
        "signal-typo",
        "tests/fixtures/avr/arduino-nano-blinky.elf",
        &system,
        SCRIPT,
    );
    assert_eq!(run.result["status"], "error", "{}", run.result);
    assert_eq!(run.status, Some(2), "stderr: {}", run.stderr);
    let stimulus = &run.result["stimuli"][0];
    assert_eq!(stimulus["outcome"], "rejected", "{stimulus}");
    assert!(
        stimulus["error"]
            .as_str()
            .is_some_and(|e| e.contains("no model input reads it")),
        "{stimulus}"
    );
}

/// A machine snapshot cannot carry an ngspice solver's state, so
/// `--capture-app-entry` with an `adapter: ngspice` model is a config error
/// before any solver starts (no python3 or libngspice needed). The same run
/// without the flag is not refused (negative control: it gets past the
/// refusal and fails only for its own reasons, if any).
#[test]
fn a_runtime_snapshot_with_an_ngspice_model_is_refused() {
    // `model:` stays inside the manifest directory: the netlist is copied there.
    let netlist =
        std::fs::read_to_string(workspace_root().join("examples/cosim-ngspice-ldr/ldr.cir"))
            .unwrap();
    let files = [("ldr.cir", netlist.as_str())];
    let system = r#"
name: "ngspice-snapshot"
chip: "stm32f401"
cosim_models:
  - id: "ldr"
    adapter: "ngspice"
    model: "./ldr.cir"
    step_ns: 100000
    inputs: { supply: "board.gpio.pa5" }
    outputs: { v_out: "board.analog.pa0_volts" }
    config:
      input_sources: { supply: { element: "Vsup", scale: 3.3, offset: 0 } }
      output_nodes: { v_out: { node: "out", scale: 1, offset: 0 } }
external_devices: []
"#;
    const SCRIPT: &str = r#"
schema_version: "1.2"
inputs:
  firmware: "__FIRMWARE__"
  system: "./system.yaml"
limits:
  max_steps: 20000
assertions:
  - uart_contains: "LabWired"
"#;
    let capture = labwired_cli::test_support::unique_temp_dir("labwired-ngspice-capture")
        .join("app-entry.lwrs");
    let refused = run_with_files(
        "ngspice-snapshot",
        "tests/fixtures/stm32f401-blinky.elf",
        system,
        SCRIPT,
        &["--capture-app-entry", capture.to_str().unwrap()],
        &files,
    );
    assert_eq!(
        refused.status,
        Some(2),
        "expected the config-error exit.\nstderr: {}",
        refused.stderr
    );
    assert!(
        refused.stderr.contains(
            "runtime snapshots cannot capture or restore externally hosted ngspice state"
        ),
        "the refusal must say why.\nstderr: {}",
        refused.stderr
    );
    assert!(!capture.exists(), "no snapshot may be written");

    let plain = run_with_files(
        "ngspice-no-snapshot",
        "tests/fixtures/stm32f401-blinky.elf",
        system,
        SCRIPT,
        &[],
        &files,
    );
    assert!(
        !plain.stderr.contains("refused: runtime snapshots"),
        "without a snapshot flag the run is not refused.\nstderr: {}",
        plain.stderr
    );
    assert!(
        !plain.stderr.contains("manifest directory"),
        "the copied netlist is inside the manifest directory.\nstderr: {}",
        plain.stderr
    );
}

/// The zero-change guarantee: the same firmware and limits, with no
/// `cosim_models:` in the manifest, must retire exactly the same instructions
/// and cycles. Co-simulation is opt-in, and a manifest that does not ask for it
/// must not pay for it — in speed OR in scheduling.
#[test]
fn a_manifest_without_cosim_models_is_unchanged() {
    const PLAIN_SYSTEM: &str = r#"
name: "no-cosim"
chip: "stm32f401"
external_devices: []
"#;
    const SCRIPT: &str = r#"
schema_version: "1.2"
inputs:
  firmware: "__FIRMWARE__"
  system: "./system.yaml"
limits:
  max_steps: 200000
assertions:
  - uart_contains: "LabWired"
"#;
    let first = run("plain-a", PLAIN_SYSTEM, SCRIPT);
    let second = run("plain-b", PLAIN_SYSTEM, SCRIPT);
    assert_eq!(first.result["status"], "pass", "stderr: {}", first.stderr);
    assert_eq!(
        first.result["metrics"]["cycles"], second.result["metrics"]["cycles"],
        "a run with no co-sim models must stay deterministic"
    );
    assert!(
        !first.stderr.contains("co-sim:"),
        "no co-sim machinery should start.\nstderr: {}",
        first.stderr
    );
}

/// `--analog-trace` on `labwired test` records the waveform the co-simulation
/// session produces. The in-core `adapter: analog` model fills a ring that the
/// session publishes on the machine; before that, `test` had no runner attached
/// and wrote a header with no rows.
///
/// The circuit is `examples/cosim-spice-rc/system-analog.yaml`: the blink
/// sketch holds PA5 high (it drives LD2 at cycle 6072 and the blink delay is
/// 500 ms), so across this run the RC node must charge monotonically toward the
/// rail. `v(in)` is a trace channel of that manifest, so the CSV itself says
/// when the pin was high.
#[test]
fn analog_trace_records_the_session_waveform() {
    let temp_dir = labwired_cli::test_support::unique_temp_dir("labwired-cosim-analog-trace");
    std::fs::create_dir_all(&temp_dir).unwrap();
    let root = workspace_root();
    let firmware = root.join("tests/fixtures/stm32f401-blinky.elf");
    let system = root.join("examples/cosim-spice-rc/system-analog.yaml");
    let script_path = temp_dir.join("script.yaml");
    std::fs::write(
        &script_path,
        format!(
            "schema_version: \"1.2\"\ninputs:\n  firmware: \"{}\"\n  system: \"{}\"\n\
             limits:\n  max_steps: 1500000\nassertions:\n  - uart_contains: \"LED ON\"\n",
            firmware.display(),
            system.display()
        ),
    )
    .unwrap();
    let csv_path = temp_dir.join("rc.csv");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .arg("test")
        .arg("--script")
        .arg(&script_path)
        .arg("--output-dir")
        .arg(&temp_dir)
        .arg("--no-uart-stdout")
        .arg("--analog-trace")
        .arg(&csv_path)
        .output()
        .expect("failed to run labwired");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    assert!(
        !stderr.contains("so the trace has no channels"),
        "the session's ring was not attached.\nstderr: {stderr}"
    );

    let csv = std::fs::read_to_string(&csv_path).expect("read analog trace CSV");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let mut lines = csv.lines();
    let header: Vec<&str> = lines.next().expect("CSV header").split(',').collect();
    let column = |name: &str| {
        header
            .iter()
            .position(|h| *h == name)
            .unwrap_or_else(|| panic!("no `{name}` column in {header:?}"))
    };
    let (v_out_col, v_in_col) = (column("rc_lowpass.v_out"), column("rc_lowpass.v(in)"));
    let rows: Vec<Vec<f64>> = lines
        .map(|line| {
            line.split(',')
                .map(|field| field.parse::<f64>().expect("numeric CSV field"))
                .collect()
        })
        .collect();
    assert!(
        rows.len() > 100,
        "expected more than 100 samples, got {}",
        rows.len()
    );

    // Row 0 is the t = 0 operating point, solved before any routed input
    // applies, so the source still sits at 0 V. From the first model boundary
    // on, the sketch has PA5 high for the rest of this run.
    assert_eq!(rows[0][0], 0.0, "first row is the t = 0 operating point");
    assert_eq!(rows[0][v_in_col], 0.0);
    let high: Vec<&Vec<f64>> = rows.iter().filter(|row| row[v_in_col] > 1.65).collect();
    assert_eq!(
        high.len(),
        rows.len() - 1,
        "PA5 should be high at every boundary after t = 0"
    );
    for pair in high.windows(2) {
        // Backward Euler approaches the rail from below; 1 nV absorbs the last
        // ulp of rounding once the node has settled.
        assert!(
            pair[1][v_out_col] >= pair[0][v_out_col] - 1e-9,
            "v_out fell while PA5 was high: {} -> {}",
            pair[0][v_out_col],
            pair[1][v_out_col]
        );
    }
    let (first, last) = (high[0][v_out_col], high[high.len() - 1][v_out_col]);
    assert!(
        first > 0.0 && first < 1.0,
        "the first high sample should be early in the charge: {first}"
    );
    assert!(
        last > 3.2,
        "the node should have settled near 3.3 V: {last}"
    );
}

/// Whether python3 can load libngspice. Without it the caller skips, unless
/// `LABWIRED_REQUIRE_NGSPICE` is set, as in
/// `crates/core/tests/cosim/cosim_ngspice.rs`.
fn ngspice_ready() -> bool {
    let probe = Command::new("python3")
        .arg("-c")
        .arg(
            "import ctypes, ctypes.util, os;\
             ctypes.CDLL(os.environ.get('LABWIRED_NGSPICE_LIB') or \
             ctypes.util.find_library('ngspice') or 'libngspice.so.0')",
        )
        .output();
    let ready = matches!(probe, Ok(output) if output.status.success());
    if !ready {
        assert!(
            std::env::var_os("LABWIRED_REQUIRE_NGSPICE").is_none(),
            "LABWIRED_REQUIRE_NGSPICE is set, but python3 cannot load libngspice"
        );
        eprintln!(
            "SKIP: python3 cannot load libngspice (apt install libngspice0, or set \
             LABWIRED_NGSPICE_LIB); the labwired test ngspice run did not run"
        );
    }
    ready
}

/// `labwired test --script` on the committed Flex ngspice scripts, run from a
/// working directory that is not the fixture's: `model: ./rc.cir` resolves
/// from the directory of `system-ngspice.yaml`. The enable run reaches 63.2 %
/// at 1025 µs; the disabled run is the negative control and stays at 0 V.
#[test]
fn labwired_test_runs_flex_ngspice_scripts_from_any_working_directory() {
    if !ngspice_ready() {
        return;
    }
    let fixture = workspace_root().join("crates/wasm/tests/fixtures/flex-mixed-signal");
    for (script, entry_result) in [("ngspice-enable.yaml", 1025), ("ngspice-disabled.yaml", 0)] {
        let temp_dir =
            labwired_cli::test_support::unique_temp_dir(&format!("labwired-flex-{script}"));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
            .current_dir(&temp_dir)
            .arg("test")
            .arg("--script")
            .arg(fixture.join(script))
            .arg("--output-dir")
            .arg(&temp_dir)
            .arg("--no-uart-stdout")
            .output()
            .expect("failed to run labwired");
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let result: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(temp_dir.join("result.json"))
                .unwrap_or_else(|error| panic!("{script}: no result.json ({error})\n{stderr}")),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&temp_dir);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{script}: {result:#}\n{stderr}"
        );
        assert_eq!(result["status"], "pass", "{script}: {result:#}");
        assert_eq!(result["stop_reason"], "completed", "{script}: {result:#}");
        assert_eq!(result["entry_result"], entry_result, "{script}: {result:#}");
    }
}

/// The reference-solver variant of the source-pinned HeyPCB ESP32-S3 NTC
/// fixture. `ngspice-*.yaml` run the same committed firmware, ports and model
/// sequence as `test-*.yaml`, but the R6/U4/C7 network is solved by ngspice
/// from `ntc.cir` instead of the in-core engine. Both engines must produce the
/// same user-visible result: every firmware `loop` result, the console text and
/// the instrument counts. The run report must pin the deck it solved, and 61 °C
/// must stop both engines with their own explicit range error.
#[test]
fn labwired_test_runs_the_heypcb_ntc_deck_on_ngspice() {
    if !ngspice_ready() {
        return;
    }
    let fixture = workspace_root().join("crates/wasm/tests/fixtures/flex-heypcb-esp32-ntc");
    let run_script = |script: &std::path::Path| -> (Option<i32>, serde_json::Value, String) {
        let dir = labwired_cli::test_support::unique_temp_dir(&format!(
            "labwired-heypcb-ntc-{}",
            script.file_stem().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
            .current_dir(&dir)
            .arg("test")
            .arg("--script")
            .arg(script)
            .arg("--output-dir")
            .arg(&dir)
            .arg("--no-uart-stdout")
            .output()
            .expect("failed to run labwired");
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let result: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("result.json"))
                .unwrap_or_else(|error| panic!("{script:?}: no result.json ({error})\n{stderr}")),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        (output.status.code(), result, stderr)
    };

    for (deck_script, in_core_script) in [
        ("ngspice-overtemp-alarm.yaml", "test-overtemp-alarm.yaml"),
        ("ngspice-range-ends.yaml", "test-range-ends.yaml"),
        ("ngspice-below-threshold.yaml", "test-below-threshold.yaml"),
    ] {
        let (code, deck, stderr) = run_script(&fixture.join(deck_script));
        assert_eq!(code, Some(0), "{deck_script}: {stderr}\n{deck:#}");
        assert_eq!(deck["status"], "pass", "{deck_script}: {deck:#}");
        assert_eq!(deck["stop_reason"], "completed", "{deck_script}");

        let (code, in_core, stderr) = run_script(&fixture.join(in_core_script));
        assert_eq!(code, Some(0), "{in_core_script}: {stderr}\n{in_core:#}");
        // The firmware observable and the instrument capture, not just "both
        // passed": the same scripts assert the same numbers on both engines.
        assert_eq!(
            deck["entry_result"], in_core["entry_result"],
            "{deck_script} vs {in_core_script}"
        );
        assert_eq!(
            deck["console"], in_core["console"],
            "{deck_script} vs {in_core_script}"
        );
        assert_eq!(
            deck["traces"], in_core["traces"],
            "{deck_script} vs {in_core_script}"
        );
    }

    // Reproducibility: the report pins the deck, not just the manifest.
    let (code, deck, stderr) = run_script(&fixture.join("ngspice-range-ends.yaml"));
    assert_eq!(code, Some(0), "{stderr}\n{deck:#}");
    let netlist = std::fs::read(fixture.join("ntc.cir")).expect("read ntc.cir");
    let mut hasher = sha2::Sha256::new();
    hasher.update(&netlist);
    let digest = format!("{:x}", hasher.finalize());
    let model = deck["run_report"]["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "source-sns-an1")
        .expect("the NTC model is in the run report");
    assert_eq!(model["source"]["kind"], "spice_netlist");
    assert_eq!(model["source"]["origin"], "file");
    assert_eq!(model["source"]["path"], "./ntc.cir");
    assert_eq!(model["source"]["bytes"], netlist.len() as u64);
    assert_eq!(model["source"]["sha256"], digest);

    // 61 °C is outside the declared 10..60 °C domain. Each engine refuses it
    // in its own words before the interval is integrated.
    for (manifest, expected) in [
        (
            "system-ngspice.yaml",
            "ngspice model 'source-sns-an1' input 'temperature' is 61, outside the declared 10..60 °C range",
        ),
        (
            "system.yaml",
            "analog input `temperature`: thermistor temperature must be within 10..60 C",
        ),
    ] {
        let dir = labwired_cli::test_support::unique_temp_dir("labwired-heypcb-ntc-range-stop");
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("range-stop.yaml");
        let manifest_path = fixture.join(manifest);
        let firmware = fixture.join("firmware.wasm");
        let text = [
            "schema_version: \"1.0\"".to_string(),
            "inputs:".to_string(),
            format!("  flex: \"{}\"", manifest_path.display()),
            format!("  firmware: \"{}\"", firmware.display()),
            "sequence:".to_string(),
            "  - call: { export: setup, args: [{ port: sensor_v }, { port: user_led_drive }] }"
                .to_string(),
            "  - set: { port: ambient_c, value: 61 }".to_string(),
            "  - call: { export: loop }".to_string(),
            "limits:".to_string(),
            "  max_steps: 1000000".to_string(),
            "  max_time_ns: 1000000000".to_string(),
        ]
        .join("\n");
        std::fs::write(&script, text + "\n").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
            .current_dir(&dir)
            .arg("test")
            .arg("--script")
            .arg(&script)
            .arg("--output-dir")
            .arg(&dir)
            .arg("--no-uart-stdout")
            .output()
            .expect("failed to run labwired");
        let result: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("result.json")).expect("result.json"),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(output.status.code(), Some(3), "{manifest}: {result:#}");
        assert_eq!(result["status"], "error", "{manifest}");
        assert_eq!(result["stop_reason"], "model_error", "{manifest}");
        let message = result["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(expected),
            "{manifest}: {message:?} does not state {expected:?}"
        );
    }
}

/// The reference-solver variant of the source-pinned HeyPCB ESP32-S3 LDR
/// sensor-network fixture. `ngspice-*.yaml` run the same committed firmware,
/// ports and model sequence as `test-*.yaml`, but the R6/U4/C7 network is
/// solved by ngspice from `ldr.cir` instead of the in-core engine. Both engines
/// must produce the same user-visible result: every firmware ADC count and the
/// instrument counts. The run report must pin the deck it solved, and 0,
/// 0.5 and 100000 lx must stop both engines with their own explicit range
/// error.
#[test]
fn labwired_test_runs_the_heypcb_ldr_network_deck_on_ngspice() {
    if !ngspice_ready() {
        return;
    }
    let fixture = workspace_root().join("crates/wasm/tests/fixtures/flex-heypcb-ldr-network");
    let run_script = |script: &std::path::Path| -> (Option<i32>, serde_json::Value, String) {
        let dir = labwired_cli::test_support::unique_temp_dir(&format!(
            "labwired-heypcb-ldr-{}",
            script.file_stem().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
            .current_dir(&dir)
            .arg("test")
            .arg("--script")
            .arg(script)
            .arg("--output-dir")
            .arg(&dir)
            .arg("--no-uart-stdout")
            .output()
            .expect("failed to run labwired");
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let result: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("result.json"))
                .unwrap_or_else(|error| panic!("{script:?}: no result.json ({error})\n{stderr}")),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        (output.status.code(), result, stderr)
    };

    for (deck_script, in_core_script) in [
        ("ngspice-bright.yaml", "test-bright.yaml"),
        ("ngspice-dim.yaml", "test-dim.yaml"),
    ] {
        let (code, deck, stderr) = run_script(&fixture.join(deck_script));
        assert_eq!(code, Some(0), "{deck_script}: {stderr}\n{deck:#}");
        assert_eq!(deck["status"], "pass", "{deck_script}: {deck:#}");
        assert_eq!(deck["stop_reason"], "completed", "{deck_script}");

        let (code, in_core, stderr) = run_script(&fixture.join(in_core_script));
        assert_eq!(code, Some(0), "{in_core_script}: {stderr}\n{in_core:#}");
        // The firmware observable and the instrument capture, not just "both
        // passed": the same scripts assert the same numbers on both engines.
        assert_eq!(
            deck["entry_result"], in_core["entry_result"],
            "{deck_script} vs {in_core_script}"
        );
        assert_eq!(
            deck["traces"], in_core["traces"],
            "{deck_script} vs {in_core_script}"
        );
    }

    // Reproducibility: the report pins the deck, not just the manifest.
    let (code, deck, stderr) = run_script(&fixture.join("ngspice-bright.yaml"));
    assert_eq!(code, Some(0), "{stderr}\n{deck:#}");
    let netlist = std::fs::read(fixture.join("ldr.cir")).expect("read ldr.cir");
    let mut hasher = sha2::Sha256::new();
    hasher.update(&netlist);
    let digest = format!("{:x}", hasher.finalize());
    let model = deck["run_report"]["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "source-sns-an1")
        .expect("the LDR model is in the run report");
    assert_eq!(model["source"]["kind"], "spice_netlist");
    assert_eq!(model["source"]["origin"], "file");
    assert_eq!(model["source"]["path"], "./ldr.cir");
    assert_eq!(model["source"]["bytes"], netlist.len() as u64);
    assert_eq!(model["source"]["sha256"], digest);

    // 0, 0.5 and 100000 lx are outside the declared 1..100 lx domain. Each
    // engine refuses them in its own words before the interval is integrated.
    for (manifest, expected) in [
        (
            "system-ngspice.yaml",
            "ngspice model 'source-sns-an1' input 'lux' is",
        ),
        (
            "system.yaml",
            "analog input `lux`: photoresistor illuminance must be within 1..100 lx",
        ),
    ] {
        for lux in ["0", "0.5", "100000"] {
            let dir = labwired_cli::test_support::unique_temp_dir("labwired-heypcb-ldr-range-stop");
            std::fs::create_dir_all(&dir).unwrap();
            let script = dir.join("range-stop.yaml");
            let manifest_path = fixture.join(manifest);
            let firmware = fixture.join("firmware.wasm");
            let text = [
                "schema_version: \"1.0\"".to_string(),
                "inputs:".to_string(),
                format!("  flex: \"{}\"", manifest_path.display()),
                format!("  firmware: \"{}\"", firmware.display()),
                "entry:".to_string(),
                format!(
                    "  export: run\n  args: [{{ port: lux }}, {{ port: sensor_v }}, {{ port: user_led_drive }}, {{ port: user_led_state }}, {{ number: {lux} }}]"
                ),
                "limits:".to_string(),
                "  max_steps: 1000000".to_string(),
                "  max_time_ns: 1000000000".to_string(),
            ]
            .join("\n");
            std::fs::write(&script, text + "\n").unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
                .current_dir(&dir)
                .arg("test")
                .arg("--script")
                .arg(&script)
                .arg("--output-dir")
                .arg(&dir)
                .arg("--no-uart-stdout")
                .output()
                .expect("failed to run labwired");
            let result: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(dir.join("result.json")).expect("result.json"),
            )
            .unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            assert_eq!(
                output.status.code(),
                Some(3),
                "{manifest} at {lux} lx: {result:#}"
            );
            assert_eq!(result["status"], "error", "{manifest} at {lux} lx");
            assert_eq!(
                result["stop_reason"], "model_error",
                "{manifest} at {lux} lx"
            );
            let message = result["message"].as_str().unwrap_or_default();
            assert!(
                message.contains(expected),
                "{manifest} at {lux} lx: {message:?} does not state {expected:?}"
            );
        }
    }
}
