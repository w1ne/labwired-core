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
    assert!(fixtures.len() >= 11, "Flex fixtures went missing");
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
            assert_eq!(result["traces"]["analog_dropped"], 0, "{script:?}");
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
    assert!(scripts_run >= 26, "only {scripts_run} Flex scripts ran");
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
