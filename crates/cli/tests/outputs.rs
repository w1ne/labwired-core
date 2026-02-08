// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn write_temp_file(prefix: &str, contents: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push("labwired-tests");
    let _ = std::fs::create_dir_all(&dir);

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = dir.join(format!("{}-{}.yaml", prefix, nonce));
    std::fs::write(&path, contents).expect("Failed to write temp file");
    path
}

#[test]
fn test_cli_test_mode_outputs() {
    let mut dir = std::env::temp_dir();
    dir.push("labwired-tests-outputs");
    let _ = std::fs::create_dir_all(&dir);

    // Copy fixture ELF to the temp dir to test relative path resolution
    let fw_path = dir.join("fixture.elf");
    std::fs::copy("../../tests/fixtures/uart-ok-thumbv7m.elf", &fw_path)
        .expect("Failed to copy fixture.elf");

    let script_path = dir.join("script.yaml");
    let script_content = r#"
schema_version: "1.0"
inputs:
  firmware: "fixture.elf"
limits:
  max_steps: 10
assertions:
  - uart_regex: ".*"
  - expected_stop_reason: max_steps
"#;
    std::fs::write(&script_path, script_content).expect("Failed to write script");

    let output_dir = dir.join("artifacts");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            script_path.to_str().unwrap(),
            "--no-uart-stdout",
            "--output-dir",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let result_path = output_dir.join("result.json");
    assert!(result_path.exists());

    let snapshot_path = output_dir.join("snapshot.json");
    assert!(snapshot_path.exists());

    let junit_path = output_dir.join("junit.xml");
    assert!(junit_path.exists());
    let junit = std::fs::read_to_string(&junit_path).unwrap();
    assert!(junit.contains("<testsuite"));
    assert!(junit.contains("<testcase"));
    assert!(junit.contains("name=\"run\""));
    assert!(junit.contains("name=\"assertion 1:"));
    assert!(junit.contains("name=\"assertion 2:"));

    let result_content = std::fs::read_to_string(&result_path).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result_content).unwrap();

    assert_eq!(result["result_schema_version"], "1.0");
    assert_eq!(result["status"], "pass");
    assert_eq!(result["stop_reason"], "max_steps");
    assert_eq!(
        result["stop_reason_details"]["triggered_stop_condition"],
        "max_steps"
    );
    assert!(result["stop_reason_details"]["triggered_limit"]["name"].is_string());
    assert!(result["stop_reason_details"]["triggered_limit"]["value"].is_number());
    assert!(result["stop_reason_details"]["observed"]["name"].is_string());
    assert!(result["stop_reason_details"]["observed"]["value"].is_number());
    assert_eq!(result["limits"]["max_steps"], 10);
    assert!(result["firmware_hash"].as_str().is_some());
    assert!(result["config"]["firmware"]
        .as_str()
        .unwrap()
        .contains("fixture.elf"));

    let snapshot_content = std::fs::read_to_string(&snapshot_path).unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot_content).unwrap();
    assert_eq!(snapshot["type"], "standard");
    assert!(snapshot["cpu"]["registers"].is_array());
    assert_eq!(snapshot["cpu"]["registers"].as_array().unwrap().len(), 16);
    assert!(snapshot["cpu"]["registers"][15].is_number());

    // Clean up
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_cli_test_mode_outputs_on_config_error() {
    let script = write_temp_file(
        "script-config-error",
        r#"
schema_version: "1.0"
inputs:
  firmware: "fixture.elf"
limits:
  max_steps: 1
bad_field: 123
"#,
    );

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let output_dir = std::env::temp_dir().join(format!("labwired-tests-config-error-{}", nonce));
    let _ = std::fs::remove_dir_all(&output_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
            "--output-dir",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2)); // EXIT_CONFIG_ERROR

    let result_path = output_dir.join("result.json");
    assert!(result_path.exists());
    let result_content = std::fs::read_to_string(&result_path).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result_content).unwrap();
    assert_eq!(result["status"], "error");
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["message"]
        .as_str()
        .unwrap_or_default()
        .contains("Failed to parse"));

    let junit_path = output_dir.join("junit.xml");
    assert!(junit_path.exists());
    let junit = std::fs::read_to_string(&junit_path).unwrap();
    assert!(junit.contains("<testsuite"));
    assert!(junit.contains("config error"));

    let uart_path = output_dir.join("uart.log");
    assert!(uart_path.exists());

    let snapshot_path = output_dir.join("snapshot.json");
    assert!(snapshot_path.exists());
    let snapshot_content = std::fs::read_to_string(&snapshot_path).unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot_content).unwrap();
    assert_eq!(snapshot["type"], "config_error");
    assert!(snapshot["message"]
        .as_str()
        .unwrap_or_default()
        .contains("Failed"));

    let _ = std::fs::remove_dir_all(&output_dir);
}

#[test]
fn test_cli_test_mode_junit_flag_writes_file() {
    let fw_abs = std::fs::canonicalize("../../tests/fixtures/uart-ok-thumbv7m.elf").unwrap();
    let script = write_temp_file(
        "script-junit-path",
        &format!(
            r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
limits:
  max_steps: 1
assertions:
  - expected_stop_reason: max_steps
"#,
            fw_abs.to_str().unwrap()
        ),
    );

    let junit_path = std::env::temp_dir().join("labwired-junit-flag.xml");
    let _ = std::fs::remove_file(&junit_path);

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
            "--junit",
            junit_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    assert!(junit_path.exists());

    let junit = std::fs::read_to_string(&junit_path).unwrap();
    assert!(junit.contains("<testsuite"));
    assert!(junit.contains("name=\"run\""));
    assert!(junit.contains("name=\"assertion 1:"));
}

#[test]
fn test_cli_test_mode_wall_time() {
    let fw_abs = std::fs::canonicalize("../../tests/fixtures/uart-ok-thumbv7m.elf").unwrap();
    let script = write_temp_file(
        "script-walltime",
        &format!(
            r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
limits:
  max_steps: 10000000
  wall_time_ms: 0
assertions:
  - expected_stop_reason: wall_time
"#,
            fw_abs.to_str().unwrap()
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
        ])
        .output()
        .expect("Failed to execute command");

    // Should pass because we expect wall_time stop reason
    assert!(output.status.success());
}

#[test]
fn test_cli_test_mode_memory_violation() {
    let fw_abs = std::fs::canonicalize("../../tests/fixtures/uart-ok-thumbv7m.elf").unwrap();
    let base_dir = std::env::temp_dir()
        .join("labwired-tests")
        .join(format!("system-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base_dir);

    let chip_path = base_dir.join("chip.yaml");
    std::fs::write(
        &chip_path,
        r#"
name: "tiny"
arch: "cortex-m3"
flash:
  base: 0x0
  size: "1B"
ram:
  base: 0x20000000
  size: "1KB"
peripherals: []
"#,
    )
    .unwrap();

    let system_path = base_dir.join("system.yaml");
    std::fs::write(
        &system_path,
        r#"
name: "tiny-system"
chip: "chip.yaml"
"#,
    )
    .unwrap();
    let script = write_temp_file(
        "script-memviol",
        &format!(
            r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
limits:
  max_steps: 1000
assertions:
  - expected_stop_reason: memory_violation
"#,
            fw_abs.to_str().unwrap()
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--system",
            system_path.to_str().unwrap(),
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
        ])
        .output()
        .expect("Failed to execute command");

    // Should pass because we expect memory_violation stop reason
    assert!(output.status.success());
}

#[test]
fn test_cli_test_mode_max_steps_guard() {
    let fw_abs = std::fs::canonicalize("../../tests/fixtures/uart-ok-thumbv7m.elf").unwrap();
    let script = write_temp_file(
        "script-huge",
        &format!(
            r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
limits:
  max_steps: 60000000
"#,
            fw_abs.to_str().unwrap()
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args(["test", "--script", script.to_str().unwrap()])
        .output()
        .expect("Failed to execute command");

    // Should fail due to MAX_ALLOWED_STEPS guard
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2)); // EXIT_CONFIG_ERROR
}

#[test]
fn test_cli_test_mode_regex_fail() {
    let fw_abs = std::fs::canonicalize("../../tests/fixtures/uart-ok-thumbv7m.elf").unwrap();
    let script = write_temp_file(
        "script-regex-fail",
        &format!(
            r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
limits:
  max_steps: 10
assertions:
  - uart_regex: "^ThisTextWillNeverBeFound$"
"#,
            fw_abs.to_str().unwrap()
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args(["test", "--script", script.to_str().unwrap()])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1)); // EXIT_ASSERT_FAIL
}

#[test]
fn test_cli_test_mode_junit_splits_assertion_failures() {
    let fw_abs = std::fs::canonicalize("../../tests/fixtures/uart-ok-thumbv7m.elf").unwrap();
    let script = write_temp_file(
        "script-junit-assertion-failures",
        &format!(
            r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
limits:
  max_steps: 10
assertions:
  - uart_contains: "NEVER_PRESENT_1"
  - uart_contains: "NEVER_PRESENT_2"
  - expected_stop_reason: max_steps
"#,
            fw_abs.to_str().unwrap()
        ),
    );

    let junit_path = std::env::temp_dir().join("labwired-junit-assertion-failures.xml");
    let _ = std::fs::remove_file(&junit_path);

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
            "--junit",
            junit_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(1));
    assert!(junit_path.exists());

    let junit = std::fs::read_to_string(&junit_path).unwrap();
    assert!(junit.contains("<testsuite"));
    assert!(junit.contains("name=\"run\""));
    assert!(junit.contains("name=\"assertion 1: uart_contains: NEVER_PRESENT_1\""));
    assert!(junit.contains("name=\"assertion 2: uart_contains: NEVER_PRESENT_2\""));
    assert!(junit.contains("name=\"assertion 3: expected_stop_reason: MaxSteps\""));

    // Two failing assertions => two separate <failure> entries.
    assert_eq!(
        junit
            .matches("<failure message=\"assertion failed\">")
            .count(),
        2
    );
}
