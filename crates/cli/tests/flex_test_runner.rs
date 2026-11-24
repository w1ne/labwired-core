// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! Black-box contract tests for `labwired test` on portable Flex projects.
//!
//! The project under test is the pinned HeyPCB LDR sensor-network fixture the
//! browser harness runs: the same `system.yaml` and the same `firmware.wasm`
//! bytes, unchanged. No target CPU is emulated on this path.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::fs::canonicalize(root.join("../wasm/tests/fixtures/flex-heypcb-ldr-network"))
        .expect("pinned LDR network fixture")
}

fn unique_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before Unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "labwired-flex-runner-{label}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    dir
}

fn run(script: &Path, output_dir: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_labwired"))
        .arg("test")
        .arg("--script")
        .arg(script)
        .arg("--output-dir")
        .arg(output_dir)
        .arg("--no-key")
        .args(extra)
        .env_remove("LABWIRED_API_KEY")
        .output()
        .expect("run labwired test")
}

fn read_result(output_dir: &Path) -> Value {
    let text = std::fs::read_to_string(output_dir.join("result.json")).expect("result.json");
    serde_json::from_str(&text).expect("result.json is JSON")
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read JSON")).expect("parse JSON")
}

/// Oscilloscope samples as (time_ns, f32 bits). The engine stores f32; the
/// native reference prints each one widened to f64, so compare the f32.
fn scope_samples(samples: &Value) -> Vec<(u64, Vec<u32>)> {
    samples
        .as_array()
        .expect("samples array")
        .iter()
        .map(|sample| {
            let values = sample["values"].as_array().expect("values array");
            (
                sample["time_ns"].as_u64().expect("time_ns"),
                values
                    .iter()
                    .map(|value| (value.as_f64().expect("sample value") as f32).to_bits())
                    .collect(),
            )
        })
        .collect()
}

fn sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).expect("read")))
}

/// A script against the pinned fixture with one illuminance, written outside
/// the fixture so relative `inputs` paths are exercised as absolute ones.
fn ldr_script(dir: &Path, name: &str, lux: &str, limits: &str, assertions: &str) -> PathBuf {
    let fixture = fixture();
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  flex: \"{}\"\n  firmware: \"{}\"\nentry:\n  export: run\n  args:\n    - {{ port: lux }}\n    - {{ port: sensor_v }}\n    - {{ port: user_led_drive }}\n    - {{ port: user_led_state }}\n    - {{ number: {lux} }}\nlimits: {limits}\nassertions:\n{assertions}",
            fixture.join("system.yaml").display(),
            fixture.join("firmware.wasm").display(),
        ),
    )
    .expect("write script");
    path
}

const LIMITS: &str = "{ max_steps: 100000, max_time_ns: 10000000 }";
const BRIGHT_ASSERTIONS: &str =
    "  - entry_result: { equals: 2974 }\n  - digital_edges: { port: user_led_state, count: 2 }\n";

#[test]
fn committed_scripts_reproduce_the_pinned_native_reference() {
    let fixture = fixture();
    let native = json(&fixture.join("native.json"));
    for (case, entry_result, edges) in [("bright", 2974, 4), ("dim", 597, 0)] {
        let dir = unique_dir(case);
        let vcd = dir.join("digital.vcd");
        let csv = dir.join("analog.csv");
        let output = run(
            &fixture.join(format!("test-{case}.yaml")),
            &dir,
            &[
                "--vcd",
                vcd.to_str().unwrap(),
                "--analog-trace",
                csv.to_str().unwrap(),
            ],
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let result = read_result(&dir);
        assert_eq!(result["status"], "pass");
        assert_eq!(result["stop_reason"], "completed");
        assert_eq!(result["entry_result"], entry_result);
        assert_eq!(result["time_ns"], native[case]["time_ns"]);
        assert!(result["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|assertion| assertion["passed"] == true));
        let sensor = result["ports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|port| port["name"] == "sensor_v")
            .expect("sensor_v port");
        assert_eq!(sensor["unit"], "V");
        assert_eq!(sensor["value"], native[case]["sensor_v"]);

        // The run identifies itself and pins exactly what it ran.
        let report = &result["run_report"];
        assert_eq!(report["execution_path"], "flex_portable_wasm");
        assert_eq!(report["target_cpu_emulated"], false);
        assert_eq!(report["time_unit"], "ns");
        assert_eq!(
            report["execution_scope"]["original_target_firmware_executed"],
            false
        );
        assert_eq!(report["execution_scope"]["chip_parity"], "not_established");
        assert_eq!(
            report["execution_scope"]["full_board_coverage"],
            "not_established"
        );
        assert_eq!(
            report["manifest_sha256"],
            sha256(&fixture.join("system.yaml"))
        );
        assert_eq!(
            result["config"]["firmware_sha256"],
            sha256(&fixture.join("firmware.wasm"))
        );
        // The analog model's netlist is inline here, so it is also inside the
        // manifest hash; the report still names it as the circuit that ran.
        let analog = report["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["adapter"] == "analog")
            .expect("analog model");
        assert_eq!(analog["source"]["kind"], "spice_netlist");
        assert_eq!(analog["source"]["origin"], "inline");
        assert_eq!(analog["source"]["sha256"].as_str().unwrap().len(), 64);
        assert!(!report["assumptions"].as_array().unwrap().is_empty());
        assert!(!report["unsupported_checks"].as_array().unwrap().is_empty());

        // Same instrument captures as the committed native reference, which
        // the Chromium receipt is already compared against.
        let trace = json(&dir.join("flex-trace.json"));
        for key in ["time_unit", "dropped", "channels", "edges"] {
            assert_eq!(
                trace["digital"][key], native[case]["digital"][key],
                "{case} digital {key}"
            );
        }
        for key in ["time_unit", "dropped", "channels"] {
            assert_eq!(
                trace["analog"][key], native[case]["analog"][key],
                "{case} analog {key}"
            );
        }
        assert_eq!(
            scope_samples(&trace["analog"]["samples"]),
            scope_samples(&native[case]["analog"]["samples"]),
            "{case} analog samples"
        );
        assert_eq!(result["traces"]["digital_edges"], edges);
        assert_eq!(result["traces"]["analog_samples"], 81);

        // The existing exporters carry the named ports and nanoseconds.
        let vcd = std::fs::read_to_string(&vcd).expect("digital VCD");
        assert!(vcd.contains("$timescale 1 ns $end"));
        assert!(vcd.contains("user_led_state") && vcd.contains("user_led_drive"));
        assert_eq!(vcd.contains("#4001000"), case == "bright");
        let csv = std::fs::read_to_string(&csv).expect("analog CSV");
        assert_eq!(
            csv.lines().next(),
            Some("time_ns,source-sns-an1.voltage"),
            "analog CSV header"
        );
        assert_eq!(csv.lines().count(), 82);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn bright_expectations_fail_in_dim_light() {
    let dir = unique_dir("negative");
    let script = ldr_script(&dir, "dim.yaml", "1", LIMITS, BRIGHT_ASSERTIONS);
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(1));
    let result = read_result(&dir);
    assert_eq!(result["status"], "fail");
    assert_eq!(result["stop_reason"], "completed");
    assert_eq!(result["entry_result"], 597);
    let assertions = result["assertions"].as_array().unwrap();
    assert_eq!(assertions[0]["passed"], false);
    assert_eq!(assertions[0]["observed"], 597.0);
    assert_eq!(assertions[1]["passed"], false);
    assert_eq!(assertions[1]["observed"], 0.0);
    let junit = std::fs::read_to_string(dir.join("junit.xml")).expect("junit.xml");
    assert!(junit.contains("tests=\"3\" failures=\"2\" errors=\"0\""));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn out_of_range_illuminance_is_a_model_error_not_a_result() {
    for lux in ["0", "100000"] {
        let dir = unique_dir("range");
        let script = ldr_script(&dir, "range.yaml", lux, LIMITS, BRIGHT_ASSERTIONS);
        let output = run(&script, &dir, &[]);
        assert_eq!(output.status.code(), Some(3), "lux {lux}");
        let result = read_result(&dir);
        assert_eq!(result["status"], "error");
        assert_eq!(result["stop_reason"], "model_error");
        assert!(result["message"]
            .as_str()
            .unwrap()
            .contains("photoresistor illuminance must be within 1..100 lx"));
        assert_eq!(result["entry_result"], Value::Null);
        assert_eq!(result["time_ns"], 0);
        // The stopped runtime yields no observation, and no assertion passes.
        assert!(result["ports"]
            .as_array()
            .unwrap()
            .iter()
            .all(|port| port["value"].is_null()));
        assert!(result["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|assertion| assertion["passed"] == false));
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn limits_stop_the_run_and_cannot_pass() {
    // The firmware waits 4 ms for the divider; a 1 ms ceiling stops it first.
    let dir = unique_dir("time");
    let script = ldr_script(
        &dir,
        "time.yaml",
        "100",
        "{ max_steps: 100000, max_time_ns: 1000000 }",
        "",
    );
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(1));
    let result = read_result(&dir);
    assert_eq!(result["status"], "fail");
    assert_eq!(result["stop_reason"], "max_time_ns");
    assert_eq!(result["time_ns"], 0);
    let _ = std::fs::remove_dir_all(dir);

    // `(func (export "run") (loop (br 0)))`: never returns, never advances.
    let dir = unique_dir("fuel");
    let spin: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // header
        0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type () -> ()
        0x03, 0x02, 0x01, 0x00, // one function
        0x07, 0x07, 0x01, 0x03, b'r', b'u', b'n', 0x00, 0x00, // export "run"
        0x0a, 0x09, 0x01, 0x07, 0x00, 0x03, 0x40, 0x0c, 0x00, 0x0b, 0x0b, // loop br 0
    ];
    std::fs::write(dir.join("spin.wasm"), spin).expect("write spin.wasm");
    let script = dir.join("spin.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  flex: \"{}\"\n  firmware: spin.wasm\nlimits: {{ max_steps: 5000, max_time_ns: 1000 }}\n",
            fixture().join("system.yaml").display()
        ),
    )
    .expect("write script");
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(1));
    let result = read_result(&dir);
    assert_eq!(result["status"], "fail");
    assert_eq!(result["stop_reason"], "max_steps");
    assert_eq!(result["steps_executed"], 5000);
    assert_eq!(result["config"]["sequence"][0]["call"]["export"], "run");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn contract_mismatches_are_configuration_errors() {
    let fixture = fixture();
    let flex = fixture.join("system.yaml");
    let firmware = fixture.join("firmware.wasm");
    let head = format!(
        "schema_version: \"1.0\"\ninputs:\n  flex: \"{}\"\n  firmware: \"{}\"\n",
        flex.display(),
        firmware.display()
    );
    let limits = format!("limits: {LIMITS}\n");
    let args = "entry:\n  args:\n    - { port: lux }\n    - { port: sensor_v }\n    - { port: user_led_drive }\n    - { port: user_led_state }\n";
    let cases = [
        (
            "arity",
            format!("{head}{limits}"),
            "takes 5 parameters, but entry.args has 0",
        ),
        (
            "export",
            format!("{head}entry: {{ export: main }}\n{limits}"),
            "no exported function 'main'",
        ),
        (
            "entry-port",
            format!("{head}{args}    - {{ port: lumens }}\n{limits}"),
            "unknown Flex port 'lumens'",
        ),
        (
            "port-type",
            format!("{head}{args}    - {{ port: lux }}\n{limits}"),
            "port ids are i32",
        ),
        (
            "assertion-port",
            format!("{head}{args}    - {{ number: 100 }}\n{limits}assertions:\n  - digital_edges: {{ port: led, count: 1 }}\n"),
            "names unknown Flex port 'led'",
        ),
        (
            "assertion-kind",
            format!("{head}{args}    - {{ number: 100 }}\n{limits}assertions:\n  - digital_edges: {{ port: sensor_v, count: 1 }}\n"),
            "needs a bool port",
        ),
        (
            "write-port-value",
            format!("{head}{args}    - {{ number: 100 }}\n{limits}assertions:\n  - port_value: {{ port: lux, equals: 100 }}\n"),
            "needs a read port",
        ),
    ];
    for (name, script_text, expected) in cases {
        let dir = unique_dir(name);
        let script = dir.join("script.yaml");
        std::fs::write(&script, script_text).expect("write script");
        let output = run(&script, &dir, &[]);
        assert_eq!(output.status.code(), Some(2), "{name}");
        let result = read_result(&dir);
        assert_eq!(result["status"], "error", "{name}");
        assert_eq!(result["stop_reason"], "config_error", "{name}");
        let message = result["message"].as_str().unwrap();
        assert!(message.contains(expected), "{name}: {message}");
        assert_eq!(result["time_ns"], 0, "{name}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn flex_never_mixes_with_a_detailed_target() {
    let fixture = fixture();
    // Detailed-target options on a Flex script are refused, not ignored.
    let dir = unique_dir("options");
    let output = run(
        &fixture.join("test-bright.yaml"),
        &dir,
        &["--system", "configs/systems/ci-fixture-uart1.yaml"],
    );
    assert_eq!(output.status.code(), Some(2));
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["message"]
        .as_str()
        .unwrap()
        .contains("Flex test scripts do not support --system"));
    let _ = std::fs::remove_dir_all(dir);

    // A script cannot name both a Flex manifest and a target system or chip.
    for extra in ["system: board.yaml", "chip: stm32f103", "env: world.yaml"] {
        let dir = unique_dir("mixed");
        let script = dir.join("mixed.yaml");
        std::fs::write(
            &script,
            format!(
                "schema_version: \"1.0\"\ninputs:\n  flex: \"{}\"\n  firmware: \"{}\"\n  {extra}\nlimits: {LIMITS}\n",
                fixture.join("system.yaml").display(),
                fixture.join("firmware.wasm").display()
            ),
        )
        .expect("write script");
        let output = run(&script, &dir, &[]);
        assert_eq!(output.status.code(), Some(2), "{extra}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("unknown field"), "{extra}: {stderr}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn instrument_assertions_read_the_captured_traces() {
    // Bright light. Each assertion is wrong in one way, or names a time the
    // trace answers in a specific way.
    let dir = unique_dir("instruments");
    let assertions = concat!(
        "  - digital_edges: { port: user_led_drive, times_ns: [4000000, 4001000] }\n",
        "  - digital_edges: { port: user_led_drive, count: 2, times_ns: [4000000, 4001001] }\n",
        "  - digital_edges: { signal: source.D1.logic_state, times_ns: [4001000] }\n",
        "  - analog_sample: { port: sensor_v, at_ns: 325000, min: 1.961, max: 1.962 }\n",
        "  - analog_sample: { channel: source-sns-an1.voltage, at_ns: 350000, max: 2.0 }\n",
        "  - analog_sample: { port: sensor_v, at_ns: 5000000, min: 0 }\n",
        "  - final_time_ns: { equals: 4002000 }\n",
        "  - final_time_ns: { max: 4001999 }\n",
    );
    let script = ldr_script(&dir, "instruments.yaml", "100", LIMITS, assertions);
    let output = run(&script, &dir, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "completed");
    let assertions = result["assertions"].as_array().unwrap();
    let passed: Vec<bool> = assertions
        .iter()
        .map(|assertion| assertion["passed"].as_bool().unwrap())
        .collect();
    assert_eq!(
        passed,
        [true, false, false, true, false, false, true, false]
    );
    // A wrong edge time fails with every captured time reported.
    assert_eq!(
        assertions[1]["observed_times_ns"],
        serde_json::json!([4000000, 4001000])
    );
    assert_eq!(assertions[1]["observed"], 2.0);
    // A signal path selects the same channel as its port.
    assert_eq!(
        assertions[2]["observed_times_ns"],
        serde_json::json!([4001000, 4002000])
    );
    // Between two samples the oscilloscope holds the earlier one.
    assert_eq!(
        assertions[3]["observed_times_ns"],
        serde_json::json!([300000])
    );
    assert_eq!(
        (assertions[3]["observed"].as_f64().unwrap() as f32).to_bits(),
        1.961_360_5_f32.to_bits()
    );
    assert_eq!(
        (assertions[4]["observed"].as_f64().unwrap() as f32).to_bits(),
        2.033_488_5_f32.to_bits()
    );
    // The run ended at 4,002,000 ns; a later time has no sample and cannot
    // pass, whatever the bound.
    assert_eq!(assertions[5]["observed"], Value::Null);
    assert!(assertions[5].get("observed_times_ns").is_none());
    assert_eq!(assertions[7]["observed"], 4002000.0);
    let junit = std::fs::read_to_string(dir.join("junit.xml")).expect("junit.xml");
    assert!(junit.contains("tests=\"9\" failures=\"5\" errors=\"0\""));
    assert!(junit.contains("analog_sample sensor_v at 5000000 ns"));
    assert!(junit.contains("observed_times_ns=Some([4000000, 4001000])"));
    let _ = std::fs::remove_dir_all(dir);

    // Negative control: the bright transient expectation fails in dim light
    // on the same node.
    let dir = unique_dir("instruments-dim");
    let script = ldr_script(
        &dir,
        "dim.yaml",
        "1",
        LIMITS,
        "  - analog_sample: { port: sensor_v, at_ns: 350000, min: 2.033, max: 2.034 }\n",
    );
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(1));
    let result = read_result(&dir);
    assert_eq!(
        (result["assertions"][0]["observed"].as_f64().unwrap() as f32).to_bits(),
        1.070_439_6_f32.to_bits()
    );
    let _ = std::fs::remove_dir_all(dir);

    // A channel the instruments do not have is a configuration error before
    // the firmware runs, not an assertion that silently fails.
    for (assertion, expected) in [
        (
            "analog_sample: { port: user_led_state, at_ns: 0, min: 0 }",
            "assertions[0].analog_sample needs a numeric read port; 'user_led_state' is not one",
        ),
        (
            "analog_sample: { port: lux, at_ns: 0, min: 0 }",
            "assertions[0].analog_sample needs a numeric read port; 'lux' is not one",
        ),
        (
            "analog_sample: { port: nope, at_ns: 0, min: 0 }",
            "assertions[0].analog_sample names unknown Flex port 'nope'",
        ),
        (
            "analog_sample: { channel: nope.v, at_ns: 0, min: 0 }",
            "the oscilloscope has no channel for 'nope.v'; its channels are [source-sns-an1.voltage]",
        ),
        (
            "digital_edges: { signal: nope.out, count: 0 }",
            "the logic analyzer has no channel for 'nope.out'; its channels are [source.D1.logic_state, source.U3.GPIO48]",
        ),
        (
            "digital_edges: { port: sensor_v, times_ns: [] }",
            "assertions[0].digital_edges needs a bool port; 'sensor_v' is numeric",
        ),
    ] {
        let dir = unique_dir("instruments-config");
        let script = ldr_script(
            &dir,
            "config.yaml",
            "100",
            LIMITS,
            &format!("  - {assertion}\n"),
        );
        let output = run(&script, &dir, &[]);
        assert_eq!(output.status.code(), Some(2), "{assertion}");
        let result = read_result(&dir);
        assert_eq!(result["stop_reason"], "config_error", "{assertion}");
        assert_eq!(result["steps_executed"], 0, "{assertion}");
        let message = result["message"].as_str().unwrap();
        assert!(message.contains(expected), "{assertion}: {message}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn fixtures_root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::fs::canonicalize(root.join("../wasm/tests/fixtures")).expect("Flex fixtures")
}

/// Every committed `test-*.yaml` of every `flex-*` fixture, by fixture.
fn committed_scripts() -> Vec<(PathBuf, Vec<PathBuf>)> {
    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(fixtures_root())
        .expect("list fixtures")
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("flex-"))
        })
        .collect();
    fixtures.sort();
    fixtures
        .into_iter()
        .map(|fixture| {
            let mut scripts: Vec<PathBuf> = std::fs::read_dir(&fixture)
                .expect("list fixture")
                .map(|entry| entry.expect("script entry").path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("test-") && name.ends_with(".yaml"))
                })
                .collect();
            scripts.sort();
            (fixture, scripts)
        })
        .collect()
}

/// The value of a two-space-indented `key: value` line of a script.
fn script_value<'a>(text: &'a str, key: &str) -> &'a str {
    let prefix = format!("  {key}: ");
    text.lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("script has no '{key}' input"))
}

#[test]
fn every_flex_fixture_runs_unchanged_through_the_generic_runner() {
    let fixtures = committed_scripts();
    assert!(fixtures.len() >= 25, "Flex fixtures went missing");
    let mut scripts_run = 0;
    for (fixture, scripts) in fixtures {
        // A positive case alone proves nothing about the model.
        assert!(
            scripts.len() >= 2,
            "{fixture:?} needs a positive and a negative-control script"
        );
        for script in scripts {
            let text = std::fs::read_to_string(&script).expect("read script");
            let flex = fixture.join(script_value(&text, "flex"));
            let firmware = fixture.join(script_value(&text, "firmware"));
            let dir = unique_dir("matrix");
            let output = run(&script, &dir, &[]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{script:?}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let result = read_result(&dir);
            assert_eq!(result["status"], "pass", "{script:?}");
            assert_eq!(result["stop_reason"], "completed", "{script:?}");
            let assertions = result["assertions"].as_array().unwrap();
            assert!(!assertions.is_empty(), "{script:?} asserts nothing");
            assert!(assertions.iter().all(|a| a["passed"] == true));
            assert_eq!(result["traces"]["digital_dropped"], 0, "{script:?}");
            // A long run may overflow the scope ring only when the run-long
            // series still holds every change point.
            if result["traces"]["analog_dropped"] != 0 {
                assert_eq!(result["traces"]["analog_series_dropped"], 0, "{script:?}");
            } else {
                assert!(result["traces"]["analog_series_points"].is_null());
            }
            let report = &result["run_report"];
            assert_eq!(report["execution_path"], "flex_portable_wasm");
            assert_eq!(report["target_cpu_emulated"], false);
            assert_eq!(report["manifest_sha256"], sha256(&flex), "{script:?}");
            assert_eq!(
                result["config"]["firmware_sha256"],
                sha256(&firmware),
                "{script:?}"
            );
            let _ = std::fs::remove_dir_all(dir);

            // Control: the same run with one expected call result moved by
            // one must fail on exactly that assertion.
            let marker = "entry_result: { equals: ";
            let at = text.find(marker).unwrap_or_else(|| {
                panic!("{script:?} must assert a firmware call result");
            }) + marker.len();
            let digits = text[at..]
                .find(|c: char| !c.is_ascii_digit())
                .expect("entry_result value");
            let expected: u64 = text[at..at + digits].parse().expect("entry_result value");
            let mutated = format!("{}{}{}", &text[..at], expected + 1, &text[at + digits..])
                .replace(
                    &format!("  flex: {}", script_value(&text, "flex")),
                    &format!("  flex: \"{}\"", flex.display()),
                )
                .replace(
                    &format!("  firmware: {}", script_value(&text, "firmware")),
                    &format!("  firmware: \"{}\"", firmware.display()),
                );
            let dir = unique_dir("matrix-control");
            let control = dir.join("control.yaml");
            std::fs::write(&control, mutated).expect("write control script");
            let output = run(&control, &dir, &[]);
            assert_eq!(output.status.code(), Some(1), "{script:?} control");
            let result = read_result(&dir);
            assert_eq!(result["stop_reason"], "completed", "{script:?} control");
            let failed: Vec<&Value> = result["assertions"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|a| a["passed"] == false)
                .collect();
            assert_eq!(failed.len(), 1, "{script:?} control");
            assert_eq!(failed[0]["observed"], expected as f64, "{script:?} control");
            let _ = std::fs::remove_dir_all(dir);
            scripts_run += 1;
        }
    }
    assert!(scripts_run >= 65, "only {scripts_run} Flex scripts ran");
}

fn thermostat() -> PathBuf {
    fixtures_root().join("flex-ntc-thermostat")
}

/// The thermostat heat cycle with one text substitution, written outside the
/// fixture with absolute inputs.
fn thermostat_script(dir: &Path, from: &str, to: &str) -> PathBuf {
    let fixture = thermostat();
    let text = std::fs::read_to_string(fixture.join("test-heat-cycle.yaml")).expect("script");
    assert!(text.contains(from), "heat-cycle script lost '{from}'");
    let text = text
        .replace(from, to)
        .replace(
            "  flex: system.yaml",
            &format!("  flex: \"{}\"", fixture.join("system.yaml").display()),
        )
        .replace(
            "  firmware: firmware.wasm",
            &format!(
                "  firmware: \"{}\"",
                fixture.join("firmware.wasm").display()
            ),
        );
    let path = dir.join("thermostat.yaml");
    std::fs::write(&path, text).expect("write script");
    path
}

fn step_summary(result: &Value) -> Vec<(u64, Option<u64>, Option<u64>)> {
    result["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .map(|step| {
            (
                step["completed"].as_u64().expect("completed"),
                step["result"].as_u64(),
                step["time_ns"].as_u64(),
            )
        })
        .collect()
}

#[test]
fn sequence_keeps_firmware_state_between_calls_and_bench_stimuli() {
    let fixture = thermostat();
    let dir = unique_dir("sequence");
    let output = run(&fixture.join("test-heat-cycle.yaml"), &dir, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_result(&dir);
    assert_eq!(result["time_ns"], 8000);
    assert_eq!(result["entry_result"], 0);
    // setup, then two control periods after each temperature: 25, 50, 33, 25.
    // At 33 degC the sensor is inside the hysteresis band and the fan stays
    // on only because the firmware instance kept its state.
    assert_eq!(
        step_summary(&result),
        [
            (1, None, Some(0)),
            (2, Some(0), Some(2000)),
            (1, None, Some(2000)),
            (2, Some(1), Some(4000)),
            (1, None, Some(4000)),
            (2, Some(1), Some(6000)),
            (1, None, Some(6000)),
            (2, Some(0), Some(8000)),
        ]
    );
    assert_eq!(result["config"]["sequence"][2]["set"]["value"], 50.0);
    let trace = json(&dir.join("flex-trace.json"));
    let channel = |port: &str| {
        trace["digital"]["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|channel| channel["port_names"][0] == port)
            .expect("digital channel")["id"]
            .clone()
    };
    let edges: Vec<(u64, Value, u64)> = trace["digital"]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| {
            (
                edge["time_ns"].as_u64().unwrap(),
                edge["ch"].clone(),
                edge["value"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        edges,
        [
            (3000, channel("fan_drive"), 1),
            (4000, channel("fan_state"), 1),
            (7000, channel("fan_drive"), 0),
            (8000, channel("fan_state"), 0),
        ]
    );
    assert_eq!(
        trace["analog"]["channels"][0]["port_names"][0],
        "temperature_mv"
    );
    let millivolts: Vec<(u64, f64)> = trace["analog"]["samples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sample| {
            (
                sample["time_ns"].as_u64().unwrap(),
                sample["values"][0].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        millivolts,
        [
            (0, 1650.0),
            (1000, 1650.0),
            (2000, 1650.0),
            (3000, 2428.0),
            (4000, 2428.0),
            (5000, 1932.0),
            (6000, 1932.0),
            (7000, 1650.0),
            (8000, 1650.0),
        ]
    );
    let _ = std::fs::remove_dir_all(dir);

    // Negative control: the same 33 degC without the hot excursion.
    let dir = unique_dir("sequence-mild");
    let output = run(&fixture.join("test-mild.yaml"), &dir, &[]);
    assert_eq!(output.status.code(), Some(0));
    let result = read_result(&dir);
    assert_eq!(step_summary(&result)[3], (2, Some(0), Some(4000)));
    assert_eq!(result["traces"]["digital_edges"], 0);
    let _ = std::fs::remove_dir_all(dir);

    // The heat-cycle expectations must fail on the mild sequence.
    let dir = unique_dir("sequence-cross");
    let script = thermostat_script(&dir, "value: 50", "value: 33");
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(1));
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "completed");
    assert_eq!(result["assertions"][1]["passed"], false);
    assert_eq!(result["assertions"][1]["observed"], 0.0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn sequence_errors_and_limits_are_explicit() {
    // A contract mismatch anywhere in the sequence is found before step 0.
    for (from, to, expected) in [
        (
            "  - set: { port: temperature_c, value: 33 }",
            "  - set: { port: temperature_mv, value: 33 }",
            "sequence[4].set needs a write port; 'temperature_mv' is a read port",
        ),
        (
            "  - set: { port: temperature_c, value: 33 }",
            "  - set: { port: ambient_c, value: 33 }",
            "sequence[4].set",
        ),
        (
            "  - set: { port: temperature_c, value: 25 }\n  - call: { export: loop, repeat: 2 }",
            "  - set: { port: temperature_c, value: 25 }\n  - call: { export: tick, repeat: 2 }",
            "firmware has no exported function 'tick' (sequence[7].call.export)",
        ),
        (
            "export: setup, args: [{ port: temperature_mv }, { port: fan_drive }]",
            "export: setup, args: [{ port: temperature_mv }]",
            "takes 2 parameters, but sequence[0].call.args has 1",
        ),
    ] {
        let dir = unique_dir("sequence-config");
        let output = run(&thermostat_script(&dir, from, to), &dir, &[]);
        assert_eq!(output.status.code(), Some(2), "{to}");
        let result = read_result(&dir);
        assert_eq!(result["stop_reason"], "config_error", "{to}");
        assert!(
            result["message"].as_str().unwrap().contains(expected),
            "{to}: {}",
            result["message"]
        );
        assert_eq!(result["time_ns"], 0, "{to}");
        assert!(result["steps"].as_array().unwrap().is_empty(), "{to}");
        let _ = std::fs::remove_dir_all(dir);
    }

    // The model refuses a stimulus outside its declared range. Earlier steps
    // keep their results; later steps are reported as not reached.
    let dir = unique_dir("sequence-range");
    let output = run(
        &thermostat_script(&dir, "value: 33", "value: 100000"),
        &dir,
        &[],
    );
    assert_eq!(output.status.code(), Some(3));
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "model_error");
    assert!(result["message"]
        .as_str()
        .unwrap()
        .contains("out of range [-55, 300]"));
    assert_eq!(result["entry_result"], Value::Null);
    let steps = step_summary(&result);
    assert_eq!(steps[3], (2, Some(1), Some(4000)));
    assert_eq!(steps[5], (0, None, Some(4000)));
    assert_eq!(steps[6], (0, None, None));
    assert_eq!(steps[7], (0, None, None));
    assert!(result["ports"]
        .as_array()
        .unwrap()
        .iter()
        .all(|port| port["value"].is_null()));
    let _ = std::fs::remove_dir_all(dir);

    // A Boolean port refuses a bench value other than 0 or 1.
    let dir = unique_dir("sequence-bool");
    let script = thermostat_script(
        &dir,
        "set: { port: temperature_c, value: 33 }",
        "set: { port: fan_drive, value: 2 }",
    );
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(3));
    let result = read_result(&dir);
    assert!(result["message"]
        .as_str()
        .unwrap()
        .contains("accepts only 0 or 1"));
    assert_eq!(step_summary(&result)[4], (0, None, Some(4000)));
    let _ = std::fs::remove_dir_all(dir);

    // A limit inside a repeated call stops the sequence. The interrupted
    // step has no result, even though its first call returned.
    let dir = unique_dir("sequence-time");
    let script = thermostat_script(&dir, "max_time_ns: 10000", "max_time_ns: 5000");
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(1));
    let result = read_result(&dir);
    assert_eq!(result["status"], "fail");
    assert_eq!(result["stop_reason"], "max_time_ns");
    assert_eq!(result["time_ns"], 5000);
    assert_eq!(result["entry_result"], Value::Null);
    assert_eq!(step_summary(&result)[5], (1, Some(1), Some(5000)));
    let assertions = result["assertions"].as_array().unwrap();
    assert_eq!(assertions[1]["passed"], true);
    assert_eq!(assertions[2]["passed"], false);
    assert_eq!(assertions[2]["observed"], Value::Null);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn edge_windows_include_both_ends_and_need_the_whole_window() {
    // Bright light: `user_led_state` has edges at 4,001,000 and 4,002,000 ns
    // and the run ends at 4,002,000 ns.
    let dir = unique_dir("edge-windows");
    let assertions = concat!(
        "  - digital_edges: { port: user_led_state, count: 2, from_ns: 4001000, to_ns: 4002000 }\n",
        "  - digital_edges: { port: user_led_state, times_ns: [4001000], from_ns: 4001000, to_ns: 4001000 }\n",
        "  - digital_edges: { port: user_led_state, times_ns: [4002000], from_ns: 4001001 }\n",
        "  - digital_edges: { port: user_led_state, count: 0, to_ns: 4000999 }\n",
        "  - digital_edges: { port: user_led_state, count: 0, from_ns: 4001001, to_ns: 4001999 }\n",
        "  - digital_edges: { port: user_led_state, count: 2, from_ns: 4001001 }\n",
        "  - digital_edges: { port: user_led_state, count: 2, to_ns: 4002001 }\n",
    );
    let script = ldr_script(&dir, "windows.yaml", "100", LIMITS, assertions);
    let output = run(&script, &dir, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_result(&dir);
    let assertions = result["assertions"].as_array().unwrap();
    let passed: Vec<bool> = assertions
        .iter()
        .map(|assertion| assertion["passed"].as_bool().unwrap())
        .collect();
    assert_eq!(passed, [true, true, true, true, true, false, false]);
    // A wrong count reports the edges of the window, not of the run.
    assert_eq!(assertions[5]["observed"], 1.0);
    assert_eq!(
        assertions[5]["observed_times_ns"],
        serde_json::json!([4002000])
    );
    // The run ended 1 ns before the end of this window: it was not observed.
    assert_eq!(assertions[6]["observed"], Value::Null);
    assert!(assertions[6].get("observed_times_ns").is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Edges of `min0` in 12 s of the idle face. With the 474 edges each of
/// `buzz` and BZ1 and the equal count of D3, the run records 71,244 edges.
const IDLE_12S_MIN0_EDGES: u64 = 35_148;

fn pomodoro_timer() -> PathBuf {
    fixtures_root().join("flex-heypcb-pomodoro-timer")
}

#[test]
fn edge_assertions_stay_exact_after_the_logic_capture_overflows() {
    // 12 s of the idle face: minute LED 0 breathes through a soft PWM, and
    // the logic analyzer records more edges than its 65,536-edge capture
    // holds. Only four channels move: `min0`, the D3 model that follows it,
    // `buzz` and the BZ1 model (start-up chirp).
    let fixture = pomodoro_timer();
    let dir = unique_dir("overflow");
    let script = dir.join("overflow.yaml");
    std::fs::write(
        &script,
        format!(
            concat!(
                "schema_version: \"1.0\"\n",
                "inputs:\n  flex: \"{}\"\n  firmware: \"{}\"\n",
                "sequence:\n",
                "  - call: {{ export: setup, args: [{{ port: min0 }}, {{ port: leaf }}, {{ port: buzz }}, {{ port: touch }}] }}\n",
                "  - call: {{ export: loop, repeat: 562500 }}\n",
                "limits: {{ max_steps: 1000000000, max_time_ns: 13000000000 }}\n",
                "assertions:\n",
                "  - entry_result: {{ equals: 0 }}\n",
                "  - digital_edges: {{ port: min0, count: {min0} }}\n",
                "  - digital_edges: {{ port: d3, count: {min0} }}\n",
                "  - digital_edges: {{ port: buzz, count: 474 }}\n",
                "  - digital_edges: {{ port: bz1, count: 474 }}\n",
                // The first edges of the run left the capture long ago.
                "  - digital_edges: {{ port: buzz, times_ns: [0, 189000, 378000], to_ns: 378000 }}\n",
                "  - digital_edges: {{ port: min0, count: 0, to_ns: 149585999 }}\n",
                // Wrong by one: fails with the exact count, not with `null`.
                "  - digital_edges: {{ port: min0, count: {wrong} }}\n",
                "  - final_time_ns: {{ equals: 12149398500 }}\n",
            ),
            fixture.join("system.yaml").display(),
            fixture.join("firmware.wasm").display(),
            min0 = IDLE_12S_MIN0_EDGES,
            wrong = IDLE_12S_MIN0_EDGES + 1,
        ),
    )
    .expect("write script");
    let output = run(&script, &dir, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "completed");
    let assertions = result["assertions"].as_array().unwrap();
    let passed: Vec<bool> = assertions
        .iter()
        .map(|assertion| assertion["passed"].as_bool().unwrap())
        .collect();
    assert_eq!(
        passed,
        [true, true, true, true, true, true, true, false, true]
    );
    assert_eq!(assertions[7]["observed"], IDLE_12S_MIN0_EDGES as f64);
    // The list of times stops at 4,096; the count does not.
    assert_eq!(assertions[1]["observed_times_truncated"], true);
    assert_eq!(
        assertions[1]["observed_times_ns"].as_array().unwrap().len(),
        4096
    );
    assert!(assertions[5].get("observed_times_truncated").is_none());

    // The capture's own drop counter agrees with the tallies: every recorded
    // edge is either among the newest 65,536 or counted as dropped.
    let recorded = 2 * IDLE_12S_MIN0_EDGES + 2 * 474;
    assert!(recorded > 65_536, "the run must overflow the capture");
    let dropped = result["traces"]["digital_dropped"].as_u64().unwrap();
    assert_eq!(result["traces"]["digital_edges"], 65_536);
    assert_eq!(dropped + 65_536, recorded);
    let trace = json(&dir.join("flex-trace.json"));
    assert_eq!(trace["digital"]["dropped"], dropped);
    assert_eq!(trace["digital"]["edges"].as_array().unwrap().len(), 65_536);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn analog_windows_and_crossings_read_every_sample() {
    // Bright light: SNS_AN1 rises from 1.32 V to 2.397 V through C7.
    let dir = unique_dir("analog-window");
    let assertions = concat!(
        "  - analog_crossings: { port: sensor_v, level: 2.001, direction: rising, times_ns: [350000] }\n",
        "  - analog_crossings: { port: sensor_v, level: 2.001, direction: rising, count: 1, times_ns: [300000] }\n",
        "  - analog_crossings: { channel: source-sns-an1.voltage, level: 2.001, direction: falling, count: 0 }\n",
        "  - analog_window: { port: sensor_v, from_ns: 2000000, min: 2.395, max: 2.398 }\n",
        "  - analog_window: { port: sensor_v, max: 2.39 }\n",
        "  - analog_window: { port: sensor_v, from_ns: 325000, to_ns: 349999, min: 1.961, max: 1.962 }\n",
        "  - analog_window: { port: sensor_v, to_ns: 5000000, min: 0 }\n",
        "  - analog_crossings: { port: sensor_v, level: 2.001, direction: rising, count: 0, from_ns: 350001 }\n",
    );
    let script = ldr_script(&dir, "window.yaml", "100", LIMITS, assertions);
    let output = run(&script, &dir, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "completed");
    let assertions = result["assertions"].as_array().unwrap();
    let passed: Vec<bool> = assertions
        .iter()
        .map(|assertion| assertion["passed"].as_bool().unwrap())
        .collect();
    assert_eq!(passed, [true, false, true, true, false, true, false, true]);
    // A wrong crossing time fails with the observed time reported.
    assert_eq!(assertions[1]["observed"], 1.0);
    assert_eq!(
        assertions[1]["observed_times_ns"],
        serde_json::json!([350000])
    );
    // A window that fails reports its lowest and highest value and their
    // times: the 1.32 V start and the settled end.
    assert_eq!(
        (assertions[4]["observed"].as_f64().unwrap() as f32).to_bits(),
        1.32_f32.to_bits()
    );
    let highest = assertions[4]["observed_max"].as_f64().unwrap();
    assert!((2.396..2.398).contains(&highest), "{highest}");
    assert_eq!(assertions[4]["observed_times_ns"][0], 0);
    // A window between two samples holds the earlier sample from its start.
    assert_eq!(
        assertions[5]["observed_times_ns"],
        serde_json::json!([325000, 325000])
    );
    assert_eq!(
        (assertions[5]["observed"].as_f64().unwrap() as f32).to_bits(),
        1.961_360_5_f32.to_bits()
    );
    assert_eq!(assertions[5]["observed"], assertions[5]["observed_max"]);
    // The run ended before 5,000,000 ns: the window was not observed.
    assert_eq!(assertions[6]["observed"], Value::Null);
    assert!(assertions[6].get("observed_max").is_none());
    let junit = std::fs::read_to_string(dir.join("junit.xml")).expect("junit.xml");
    assert!(junit.contains("tests=\"9\" failures=\"3\" errors=\"0\""));
    assert!(junit.contains("analog_crossings sensor_v Rising 2.001"));
    let _ = std::fs::remove_dir_all(dir);

    // Negative control: in dim light the same node falls and never crosses.
    let dir = unique_dir("analog-window-dim");
    let script = ldr_script(
        &dir,
        "dim.yaml",
        "1",
        LIMITS,
        "  - analog_crossings: { port: sensor_v, level: 2.001, direction: rising, times_ns: [350000] }\n",
    );
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(1));
    let result = read_result(&dir);
    assert_eq!(result["assertions"][0]["observed"], 0.0);
    assert_eq!(
        result["assertions"][0]["observed_times_ns"],
        serde_json::json!([])
    );
    let _ = std::fs::remove_dir_all(dir);

    for (assertion, expected) in [
        (
            "analog_window: { port: user_led_state, min: 0 }",
            "assertions[0].analog_window needs a numeric read port; 'user_led_state' is not one",
        ),
        (
            "analog_window: { port: nope, min: 0 }",
            "assertions[0].analog_window names unknown Flex port 'nope'",
        ),
        (
            "analog_crossings: { channel: nope.v, level: 1, direction: rising, count: 0 }",
            "assertions[0].analog_crossings: the oscilloscope has no channel for 'nope.v'; its channels are [source-sns-an1.voltage]",
        ),
    ] {
        let dir = unique_dir("analog-window-config");
        let script = ldr_script(
            &dir,
            "config.yaml",
            "100",
            LIMITS,
            &format!("  - {assertion}\n"),
        );
        let output = run(&script, &dir, &[]);
        assert_eq!(output.status.code(), Some(2), "{assertion}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{assertion}: {stderr}");
        assert_eq!(read_result(&dir)["stop_reason"], "config_error");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn analog_assertions_stay_exact_after_the_oscilloscope_capture_overflows() {
    // 30,004 thermostat passes of 1,000 ns each: the oscilloscope records
    // more samples than its 20,000-sample capture holds. The heat arrives at
    // 2,000 ns and leaves at 15,004,000 ns.
    let fixture = thermostat();
    let dir = unique_dir("analog-overflow");
    let script = dir.join("overflow.yaml");
    std::fs::write(
        &script,
        format!(
            concat!(
                "schema_version: \"1.0\"\n",
                "inputs:\n  flex: \"{}\"\n  firmware: \"{}\"\n",
                "sequence:\n",
                "  - call: {{ export: setup, args: [{{ port: temperature_mv }}, {{ port: fan_drive }}] }}\n",
                "  - call: {{ export: loop, repeat: 2 }}\n",
                "  - set: {{ port: temperature_c, value: 50 }}\n",
                "  - call: {{ export: loop, repeat: 15000 }}\n",
                "  - set: {{ port: temperature_c, value: 33 }}\n",
                "  - call: {{ export: loop, repeat: 2 }}\n",
                "  - set: {{ port: temperature_c, value: 25 }}\n",
                "  - call: {{ export: loop, repeat: 15000 }}\n",
                "limits: {{ max_steps: 1000000000, max_time_ns: 40000000 }}\n",
                "assertions:\n",
                "  - entry_result: {{ equals: 0 }}\n",
                // The capture no longer holds the samples of the first 10 ms.
                "  - analog_sample: {{ port: temperature_mv, at_ns: 3000, equals: 2428 }}\n",
                "  - analog_crossings: {{ port: temperature_mv, level: 2000, direction: rising, times_ns: [3000] }}\n",
                "  - analog_crossings: {{ port: temperature_mv, level: 1800, direction: falling, times_ns: [15005000] }}\n",
                "  - analog_window: {{ port: temperature_mv, from_ns: 3000, to_ns: 15002000, min: 2428, max: 2428 }}\n",
                // Wrong by 1 mV: fails with the exact extremes, not with `null`.
                "  - analog_window: {{ port: temperature_mv, max: 2427 }}\n",
                "  - digital_edges: {{ port: fan_drive, times_ns: [3000, 15005000] }}\n",
                "  - final_time_ns: {{ equals: 30004000 }}\n",
            ),
            fixture.join("system.yaml").display(),
            fixture.join("firmware.wasm").display(),
        ),
    )
    .expect("write script");
    let output = run(&script, &dir, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "completed");
    let assertions = result["assertions"].as_array().unwrap();
    let passed: Vec<bool> = assertions
        .iter()
        .map(|assertion| assertion["passed"].as_bool().unwrap())
        .collect();
    assert_eq!(passed, [true, true, true, true, true, false, true, true]);
    assert_eq!(assertions[5]["observed"], 1650.0);
    assert_eq!(assertions[5]["observed_max"], 2428.0);
    assert_eq!(
        assertions[5]["observed_times_ns"],
        serde_json::json!([0, 3000])
    );

    // The capture's own counters show that it no longer holds those samples.
    let dropped = result["traces"]["analog_dropped"].as_u64().unwrap();
    let kept = result["traces"]["analog_samples"].as_u64().unwrap();
    assert_eq!(kept, 20_000);
    assert_eq!(
        dropped + kept,
        30_005,
        "one sample per pass and the 0 ns row"
    );
    let trace = json(&dir.join("flex-trace.json"));
    let oldest = trace["analog"]["samples"][0]["time_ns"].as_u64().unwrap();
    // The newest 20,000 samples start after the heat arrived at 3,000 ns.
    assert_eq!(oldest, 10_005_000);
    // The run-long series still holds the whole run: 25 °C at 0 ns, 50 °C
    // from 3,000 ns, 33 °C from 15,003,000 ns, 25 °C from 15,005,000 ns.
    assert_eq!(result["traces"]["analog_series_points"], 4);
    assert_eq!(result["traces"]["analog_series_dropped"], 0);
    let series = &trace["analog_series"]["channels"][0];
    assert_eq!(series["name"], trace["analog"]["channels"][0]["name"]);
    assert_eq!(
        series["points"],
        serde_json::json!([
            [0, 1650.0],
            [3000, 2428.0],
            [15003000, 1932.0],
            [15005000, 1650.0]
        ])
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The NTC alarm firmware (`run`: write, 1000 ns, write, 1000 ns, return the
/// sensor millivolts) with `steps` as its sequence, outside the fixture.
fn ntc_sequence(dir: &Path, steps: &str, limits: &str) -> PathBuf {
    let fixture = fixtures_root().join("flex-analog-ntc");
    let text = format!(
        "schema_version: \"1.0\"\ninputs:\n  flex: \"{}\"\n  firmware: \"{}\"\nsequence:\n{}limits: {limits}\nassertions:\n  - digital_edges: {{ port: hot_alarm_drive, times_ns: [1000] }}\n",
        fixture.join("system.yaml").display(),
        fixture.join("firmware.wasm").display(),
        steps.replace(
            "RUN",
            "export: run, args: [{ port: temperature_c }, { port: temperature_mv }, { port: hot_alarm_drive }, { number: 50 }]"
        ),
    );
    let path = dir.join("ntc.yaml");
    std::fs::write(&path, text).expect("write script");
    path
}

fn suspended(result: &Value) -> Vec<bool> {
    result["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .map(|step| step["suspended"] == true)
        .collect()
}

const NTC_LIMITS: &str = "{ max_steps: 100000, max_time_ns: 10000 }";

#[test]
fn until_ns_suspends_a_call_for_the_test_bench_and_resume_continues_it() {
    // The second `advance_ns(1000)` of `run` starts at 1000 ns. The call is
    // suspended inside it at 1500 ns, again at 1800 ns, and returns at 2000 ns.
    // The bench cools the sensor while the firmware is suspended: the firmware
    // then returns the 0 degC reading, with the alarm it set at 50 degC.
    let dir = unique_dir("until");
    let script = ntc_sequence(
        &dir,
        "  - call: { RUN, until_ns: 1500 }\n  - set: { port: temperature_c, value: 0 }\n  - resume: { until_ns: 1800 }\n  - resume: { until_ns: 9000 }\n",
        NTC_LIMITS,
    );
    let output = run(&script, &dir, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "completed");
    assert_eq!(
        step_summary(&result),
        [
            (0, None, Some(1500)),
            (1, None, Some(1500)),
            (0, None, Some(1800)),
            (1, Some(756), Some(2000)),
        ]
    );
    assert_eq!(suspended(&result), [true, false, true, false]);
    assert_eq!(result["entry_result"], 756);
    assert_eq!(result["time_ns"], 2000);
    let port = |name: &str| {
        result["ports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|port| port["name"] == name)
            .unwrap()["value"]
            .clone()
    };
    assert_eq!(port("temperature_mv"), 756.0);
    assert_eq!(port("hot_alarm_state"), 1.0);
    let _ = std::fs::remove_dir_all(dir);

    // A run can end with the firmware suspended: the last step has no result.
    let dir = unique_dir("until-end");
    let script = ntc_sequence(&dir, "  - call: { RUN, until_ns: 1500 }\n", NTC_LIMITS);
    assert_eq!(run(&script, &dir, &[]).status.code(), Some(0));
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "completed");
    assert_eq!(step_summary(&result), [(0, None, Some(1500))]);
    assert_eq!(result["entry_result"], Value::Null);
    let _ = std::fs::remove_dir_all(dir);

    // A call that returns before its `until_ns` is an ordinary call.
    let dir = unique_dir("until-returned");
    let script = ntc_sequence(&dir, "  - call: { RUN, until_ns: 9000 }\n", NTC_LIMITS);
    assert_eq!(run(&script, &dir, &[]).status.code(), Some(0));
    let result = read_result(&dir);
    assert_eq!(step_summary(&result), [(1, Some(2428), Some(2000))]);
    assert_eq!(suspended(&result), [false]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn until_ns_and_resume_errors_are_explicit() {
    for (steps, limits, code, stop_reason, message, summary) in [
        // The firmware returned at 2000 ns; nothing is left to resume.
        (
            "  - call: { RUN, until_ns: 9000 }\n  - resume: { until_ns: 9500 }\n",
            NTC_LIMITS,
            3,
            "sequence_error",
            "sequence[1].resume has no suspended call: the firmware returned before 2000 ns",
            vec![(1, Some(2428), Some(2000)), (0, None, Some(2000))],
        ),
        // The first call ends at 2000 ns.
        (
            "  - call: { RUN }\n  - call: { RUN, until_ns: 1500 }\n",
            NTC_LIMITS,
            3,
            "sequence_error",
            "sequence[1].call.until_ns (1500 ns) is not after the current time (2000 ns)",
            vec![(1, Some(2428), Some(2000)), (0, None, Some(2000))],
        ),
        // A model error inside a call with `until_ns` stops the run; it is
        // not a suspension.
        (
            "  - set: { port: temperature_c, value: 25 }\n  - call: { export: run, args: [{ port: temperature_c }, { port: temperature_mv }, { port: hot_alarm_drive }, { number: 100000 }], until_ns: 1500 }\n  - resume: { until_ns: 9000 }\n",
            NTC_LIMITS,
            3,
            "model_error",
            "out of range [-55, 300]",
            vec![(1, None, Some(0)), (0, None, Some(0)), (0, None, None)],
        ),
        // The step budget still ends a call that never returns.
        (
            "  - call: { RUN, until_ns: 1500 }\n  - resume: { until_ns: 9000 }\n",
            "{ max_steps: 3, max_time_ns: 10000 }",
            1,
            "max_steps",
            "firmware used all limits.max_steps (3) WebAssembly fuel",
            vec![(0, None, Some(0)), (0, None, None)],
        ),
    ] {
        let dir = unique_dir("until-error");
        let output = run(&ntc_sequence(&dir, steps, limits), &dir, &[]);
        let result = read_result(&dir);
        assert_eq!(output.status.code(), Some(code), "{steps}: {result}");
        assert_eq!(result["stop_reason"], stop_reason, "{steps}");
        assert!(
            result["message"].as_str().unwrap().contains(message),
            "{steps}: {}",
            result["message"]
        );
        assert_eq!(step_summary(&result), summary, "{steps}");
        assert!(suspended(&result).iter().all(|suspended| !suspended));
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn pomodoro_c() -> PathBuf {
    fixtures_root().join("flex-heypcb-pomodoro-c")
}

#[test]
fn the_pinned_c_firmware_source_is_unchanged_and_its_main_never_returns() {
    // The compiled source is byte for byte the file the source map pins.
    let fixture = pomodoro_c();
    let map = json(&fixture.join("source-map.json"));
    for (key, file) in [
        ("original_firmware_source", "firmware/pomodoro.c"),
        ("original_firmware_config", "firmware/funconfig.h"),
        ("original_firmware_license", "firmware/LICENSE"),
    ] {
        let pinned = &map["source"]["files"][key];
        assert_eq!(pinned["path"], file);
        assert_eq!(pinned["sha256"], sha256(&fixture.join(file)), "{file}");
    }
    assert_eq!(
        map["firmware_wasm"]["sha256"],
        sha256(&fixture.join("firmware.wasm"))
    );

    // `main` is an endless loop: with no `until_ns` only a limit ends it, and
    // a limit cannot pass.
    let text = std::fs::read_to_string(fixture.join("test-untouched-stays-idle.yaml"))
        .expect("script")
        .replace(
            "  flex: system.yaml",
            &format!("  flex: \"{}\"", fixture.join("system.yaml").display()),
        )
        .replace(
            "  firmware: firmware.wasm",
            &format!(
                "  firmware: \"{}\"",
                fixture.join("firmware.wasm").display()
            ),
        );
    let from = "  - call: { export: __main_void, until_ns: 200000000 }\n  - resume: { until_ns: 250000000 }\n  - resume: { until_ns: 300000000 }\n";
    assert!(text.contains(from), "script lost its main call");
    let dir = unique_dir("pomodoro-c-endless");
    let script = dir.join("endless.yaml");
    std::fs::write(
        &script,
        text.replace(from, "  - call: { export: __main_void }\n"),
    )
    .expect("write script");
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(1));
    let result = read_result(&dir);
    assert_eq!(result["status"], "fail");
    assert_eq!(result["stop_reason"], "max_time_ns");
    // The idle loop starts at 149,606,000 ns and takes 5,000 ns for each
    // pass: 30,078 whole passes end before the limit.
    assert_eq!(result["time_ns"], 299_996_000u64);
    let _ = std::fs::remove_dir_all(dir);
}

/// The BME280 station firmware with `steps` as its sequence, outside the
/// fixture. `SETUP` is the `setup` call on the board's sensor address.
fn bme280_sequence(dir: &Path, steps: &str, assertions: &str) -> PathBuf {
    let fixture = fixtures_root().join("flex-heypcb-esp32-bme280");
    let text = format!(
        "schema_version: \"1.0\"\ninputs:\n  flex: \"{}\"\n  firmware: \"{}\"\nsequence:\n{}limits: {{ max_steps: 1000000, max_time_ns: 100000000 }}\nassertions:\n{assertions}",
        fixture.join("system.yaml").display(),
        fixture.join("firmware.wasm").display(),
        steps.replace(
            "SETUP",
            "export: setup, args: [{ bus: sensor_i2c }, { port: user_led_drive }, { number: 0x76 }]"
        ),
    );
    let path = dir.join("bme280.yaml");
    std::fs::write(&path, text).expect("write script");
    path
}

#[test]
fn i2c_transfers_reach_the_catalog_sensor_and_are_recorded() {
    let dir = unique_dir("i2c");
    let script = bme280_sequence(
        &dir,
        "  - call: { SETUP }\n  - call: { export: loop }\n",
        "  - entry_result: { equals: 12500 }\n  - i2c_transfers: { bus: sensor_i2c, address: 0x76, ack: true, equals: 8 }\n",
    );
    let output = run(&script, &dir, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_result(&dir);
    // `setup`: 504 bit times at 100 kHz. `loop`: 102 bit times and 1 ms.
    assert_eq!(
        step_summary(&result),
        [
            (1, Some(0), Some(5_040_000)),
            (1, Some(12_500), Some(7_060_000))
        ]
    );
    assert_eq!(result["traces"]["i2c_transfers"], 8);
    assert_eq!(result["assertions"][1]["observed"], 8.0);
    let bus = &result["run_report"]["i2c_buses"][0];
    assert_eq!(bus["devices"][0]["model_type"], "bme280");
    assert_eq!(bus["devices"][0]["address"], 0x76);

    let trace = json(&dir.join("flex-trace.json"));
    let transfers = trace["i2c"]["transfers"].as_array().unwrap();
    assert_eq!(transfers.len(), 8);
    assert_eq!(trace["i2c"]["buses"][0]["name"], "sensor_i2c");
    // Chip id, then the 24 calibration bytes that start with dig_T1 = 28528.
    assert_eq!(transfers[0]["write"], serde_json::json!([0xD0]));
    assert_eq!(transfers[0]["read"], serde_json::json!([0x60]));
    assert_eq!(transfers[0]["time_ns"], 0);
    assert_eq!(transfers[1]["time_ns"], 390_000);
    assert_eq!(transfers[1]["write"], serde_json::json!([0x88]));
    assert_eq!(transfers[1]["read"][0], 28528 & 0xff);
    assert_eq!(transfers[1]["read"][1], 28528 >> 8);
    // ctrl_hum before ctrl_meas, as the datasheet requires.
    assert_eq!(transfers[4]["write"], serde_json::json!([0xF2, 0x01]));
    assert_eq!(transfers[5]["write"], serde_json::json!([0xF4, 0x27]));
    assert_eq!(transfers[7]["write"], serde_json::json!([0xF7]));
    assert_eq!(transfers[7]["read"].as_array().unwrap().len(), 8);
    assert!(transfers.iter().all(|transfer| transfer["ack"] == true));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn until_ns_suspends_inside_an_i2c_transfer_and_resume_returns_its_status() {
    // The chip-id transfer of `setup` runs from 0 to 390,000 ns. The call is
    // suspended inside it, and again inside the calibration transfer. The
    // resumed transfers return their ACK status: `setup` still returns 0 at
    // the time an unsuspended `setup` returns. A wrong status would make
    // `setup` return 1 after the first transfer.
    let dir = unique_dir("i2c-until");
    let script = bme280_sequence(
        &dir,
        "  - call: { SETUP, until_ns: 200000 }\n  - resume: { until_ns: 1000000 }\n  - resume: { until_ns: 90000000 }\n",
        "  - entry_result: { equals: 0 }\n  - i2c_transfers: { bus: sensor_i2c, ack: true, equals: 7 }\n",
    );
    let output = run(&script, &dir, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_result(&dir);
    assert_eq!(
        step_summary(&result),
        [
            (0, None, Some(200_000)),
            (0, None, Some(1_000_000)),
            (1, Some(0), Some(5_040_000)),
        ]
    );
    assert_eq!(suspended(&result), [true, true, false]);
    assert_eq!(result["traces"]["i2c_transfers"], 7);
    // The two transfers that were suspended keep their own start times.
    let trace = json(&dir.join("flex-trace.json"));
    assert_eq!(trace["i2c"]["transfers"][0]["time_ns"], 0);
    assert_eq!(trace["i2c"]["transfers"][1]["time_ns"], 390_000);
    assert_eq!(trace["i2c"]["transfers"][2]["time_ns"], 2_850_000);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn i2c_contract_mismatches_and_limits_are_explicit() {
    // An unknown bus in a call argument or an assertion is exit 2 before any
    // firmware instruction.
    for (steps, assertions, message) in [
        (
            "  - call: { export: setup, args: [{ bus: missing }, { port: user_led_drive }, { number: 0x76 }] }\n",
            "  - entry_result: { equals: 0 }\n",
            // A call argument names an I2C or an SPI bus.
            "unknown Flex bus 'missing'",
        ),
        (
            "  - call: { SETUP }\n",
            "  - i2c_transfers: { bus: missing, equals: 0 }\n",
            "assertions[0].i2c_transfers: Simulation error: unknown Flex I2C bus 'missing'",
        ),
    ] {
        let dir = unique_dir("i2c-config");
        let script = bme280_sequence(&dir, steps, assertions);
        let output = run(&script, &dir, &[]);
        assert_eq!(output.status.code(), Some(2), "{message}");
        let result = read_result(&dir);
        assert_eq!(result["stop_reason"], "config_error");
        assert!(
            result["message"].as_str().unwrap().contains(message),
            "{result:#}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    // A script-level schema error: an address outside 7 bits.
    let dir = unique_dir("i2c-schema");
    let script = bme280_sequence(
        &dir,
        "  - call: { SETUP }\n",
        "  - i2c_transfers: { bus: sensor_i2c, address: 128, equals: 0 }\n",
    );
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("address must be a 7-bit address"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(dir);

    // A bench value the sensor model refuses stops the run as a model error.
    let dir = unique_dir("i2c-range");
    let script = bme280_sequence(
        &dir,
        "  - call: { SETUP }\n  - set: { port: ambient_c, value: 200 }\n  - call: { export: loop }\n",
        "  - entry_result: { equals: 0 }\n",
    );
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(3));
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "model_error");
    assert!(
        result["message"].as_str().unwrap().contains("out of range"),
        "{result:#}"
    );
    assert_eq!(result["entry_result"], Value::Null);
    let _ = std::fs::remove_dir_all(dir);

    // The transfer time counts against `limits.max_time_ns`.
    let dir = unique_dir("i2c-time");
    let script = bme280_sequence(
        &dir,
        "  - call: { SETUP }\n",
        "  - entry_result: { equals: 0 }\n",
    );
    let text = std::fs::read_to_string(&script)
        .unwrap()
        .replace("max_time_ns: 100000000", "max_time_ns: 300000");
    std::fs::write(&script, text).unwrap();
    let output = run(&script, &dir, &[]);
    assert_eq!(output.status.code(), Some(1));
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "max_time_ns");
    assert_eq!(result["time_ns"], 0);
    assert_eq!(result["traces"]["i2c_transfers"], 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn console_output_is_kept_with_the_time_of_each_write() {
    let dir = unique_dir("console");
    let script = bme280_sequence(
        &dir,
        "  - call: { SETUP }\n  - call: { export: loop }\n",
        "  - entry_result: { equals: 12500 }\n  - console_contains: \"T=25.00 C P=101325 Pa\"\n  - console_contains: \"T=26.00\"\n",
    );
    let output = run(&script, &dir, &[]);
    // The second `console_contains` names text the firmware did not print.
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_result(&dir);
    assert_eq!(result["stop_reason"], "completed");
    let text = "BME280 ready at 0x76\nT=25.00 C P=101325 Pa H=50.00 %RH\n";
    let console = &result["console"];
    assert_eq!(console["name"], "usb_serial");
    assert_eq!(console["text"], text);
    assert_eq!(console["bytes"], text.len());
    assert_eq!(console["dropped_bytes"], 0);
    // `setup` prints after its seven transfers (5.04 ms). `loop` prints after
    // its sample transfer and before its 1 ms period.
    assert_eq!(
        console["writes"],
        serde_json::json!([
            { "time_ns": 5_040_000, "offset": 0, "len": 21 },
            { "time_ns": 6_060_000, "offset": 21, "len": 34 },
        ])
    );
    assert_eq!(result["assertions"][1]["passed"], true);
    assert_eq!(result["assertions"][1]["observed"], 21.0);
    assert_eq!(result["assertions"][2]["passed"], false);
    assert_eq!(result["assertions"][2]["observed"], Value::Null);
    let report = &result["run_report"]["console"];
    assert_eq!(report["name"], "usb_serial");
    assert!(report["not_modeled"]
        .as_array()
        .unwrap()
        .contains(&Value::from("baud_rate_and_transfer_time")));
    assert_eq!(
        std::fs::read_to_string(dir.join("console.log")).unwrap(),
        text
    );
    let trace = json(&dir.join("flex-trace.json"));
    assert_eq!(trace["console"], result["console"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn console_output_needs_a_declared_console() {
    // The same manifest and firmware without the `console` block.
    let fixture = fixtures_root().join("flex-heypcb-esp32-bme280");
    let manifest = std::fs::read_to_string(fixture.join("system.yaml")).unwrap();
    let (without, _) = manifest
        .split_once("console:\n")
        .expect("fixture console block");
    for (assertions, code, stop_reason, message) in [
        (
            "  - entry_result: { equals: 0 }\n",
            3,
            "model_error",
            "labwired.console_write: the Flex manifest declares no console",
        ),
        (
            "  - console_contains: \"BME280\"\n",
            2,
            "config_error",
            "assertions[0].console_contains: the Flex manifest declares no console",
        ),
    ] {
        let dir = unique_dir("console-missing");
        let manifest_path = dir.join("system.yaml");
        std::fs::write(&manifest_path, without).unwrap();
        let script = bme280_sequence(&dir, "  - call: { SETUP }\n", assertions);
        let text = std::fs::read_to_string(&script).unwrap().replace(
            &fixture.join("system.yaml").display().to_string(),
            &manifest_path.display().to_string(),
        );
        std::fs::write(&script, text).unwrap();
        let output = run(&script, &dir, &[]);
        assert_eq!(output.status.code(), Some(code), "{message}");
        let result = read_result(&dir);
        assert_eq!(result["stop_reason"], stop_reason);
        assert!(
            result["message"].as_str().unwrap().contains(message),
            "{result:#}"
        );
        assert!(result.get("console").is_none());
        assert!(!dir.join("console.log").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_seconds_long_run_of_a_microsecond_analog_model_keeps_its_whole_series() {
    // The pinned MCP9700 board with U4 evaluated every 1 µs instead of 1 ms:
    // 4 s of virtual time write 4,000,001 oscilloscope rows into a 20,000-row
    // ring. A bench step set at k s reaches the output at k s + 1 µs.
    let fixture = fixtures_root().join("flex-heypcb-esp32-mcp9700");
    let dir = unique_dir("mcp9700-1us");
    let manifest = std::fs::read_to_string(fixture.join("system.yaml")).expect("manifest");
    let fine = manifest.replacen(
        "adapter: behavior\n    step_ns: 1000000",
        "adapter: behavior\n    step_ns: 1000",
        1,
    );
    assert_ne!(fine, manifest, "the U4 step is no longer 1 ms");
    std::fs::write(dir.join("system.yaml"), fine).expect("write manifest");
    std::fs::copy(fixture.join("firmware.wasm"), dir.join("firmware.wasm")).expect("firmware");
    let script = dir.join("alarm-1us.yaml");
    std::fs::write(
        &script,
        concat!(
            "schema_version: \"1.0\"\n",
            "inputs: { flex: system.yaml, firmware: firmware.wasm }\n",
            "sequence:\n",
            "  - call: { export: setup, args: [{ port: sensor_mv }, { port: user_led_drive }] }\n",
            "  - call: { export: loop }\n",
            "  - set: { port: ambient_c, value: 41 }\n",
            "  - call: { export: loop }\n",
            "  - set: { port: ambient_c, value: 39 }\n",
            "  - call: { export: loop }\n",
            "  - set: { port: ambient_c, value: 37 }\n",
            "  - call: { export: loop }\n",
            "limits: { max_steps: 1000000, max_time_ns: 10000000000 }\n",
            "assertions:\n",
            "  - entry_result: { equals: 103700 }\n",
            "  - analog_sample: { port: sensor_mv, at_ns: 1000, equals: 750 }\n",
            "  - analog_crossings: { port: sensor_mv, level: 900, direction: rising, times_ns: [1000001000] }\n",
            "  - analog_crossings: { port: sensor_mv, level: 880, direction: falling, times_ns: [3000001000] }\n",
            "  - final_time_ns: { equals: 4000000000 }\n",
        ),
    )
    .expect("write script");
    let output = run(&script, &dir, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_result(&dir);
    assert_eq!(result["status"], "pass");
    let traces = &result["traces"];
    assert_eq!(traces["analog_samples"], 20_000);
    assert_eq!(traces["analog_dropped"], 3_980_001);
    assert_eq!(traces["analog_series_points"], 4);
    assert_eq!(traces["analog_series_dropped"], 0);
    let trace = json(&dir.join("flex-trace.json"));
    // The ring holds the newest 20,000 rows; the series starts at 0 ns.
    assert_eq!(trace["analog"]["samples"][0]["time_ns"], 3_980_001_000u64);
    assert_eq!(trace["analog_series"]["end_time_ns"], 4_000_000_000u64);
    let series = &trace["analog_series"]["channels"][0];
    assert_eq!(series["name"], "source-u4-mcp9700.voltage_mv");
    assert_eq!(series["unit"], "mV");
    assert_eq!(
        series["points"],
        serde_json::json!([
            [0, 750.0],
            [1000001000u64, 910.0],
            [2000001000u64, 890.0],
            [3000001000u64, 870.0]
        ])
    );
    let _ = std::fs::remove_dir_all(dir);
}
