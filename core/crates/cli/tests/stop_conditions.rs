// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn get_labwired_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_labwired"))
}

fn run_test(name: &str, yaml_content: &str) -> Value {
    let temp_dir = std::env::temp_dir().join(format!("labwired-stop-{}", name));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let firmware_path = workspace_root.join("tests/fixtures/uart-ok-thumbv7m.elf");
    let system_path = workspace_root.join("configs/systems/ci-fixture-uart1.yaml");

    if !firmware_path.exists() {
        panic!("Firmware fixture not found at {:?}", firmware_path);
    }
    if !system_path.exists() {
        panic!("System fixture not found at {:?}", system_path);
    }
    let firmware_path = firmware_path.canonicalize().unwrap();
    let system_path = system_path.canonicalize().unwrap();

    // Inject firmware path
    let script_content = yaml_content
        .replace("__FIRMWARE__", &firmware_path.display().to_string())
        .replace("__SYSTEM__", &system_path.display().to_string());

    let script_path = temp_dir.join("script.yaml");
    std::fs::write(&script_path, script_content).unwrap();

    let output = Command::new(get_labwired_bin())
        .arg("test")
        .arg("--script")
        .arg(&script_path)
        .arg("--output-dir")
        .arg(&temp_dir)
        .arg("--no-uart-stdout")
        .output()
        .expect("Failed to run labwired");

    let result_json_path = temp_dir.join("result.json");
    if !result_json_path.exists() {
        panic!(
            "{} failed to produce result.json. Exit: {:?}\nStdout: {}\nStderr: {}",
            name,
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let result_content =
        std::fs::read_to_string(&result_json_path).expect("Failed to read result.json");
    let val: Value = serde_json::from_str(&result_content).expect("Failed to parse result.json");

    assert_eq!(val["result_schema_version"], "1.0");
    assert!(val["limits"]["max_steps"].is_number());
    assert!(val["stop_reason_details"]["triggered_stop_condition"].is_string());
    assert_eq!(
        val["stop_reason_details"]["triggered_stop_condition"],
        val["stop_reason"]
    );

    let uart_log_path = temp_dir.join("uart.log");
    let uart_log = if uart_log_path.exists() {
        std::fs::read_to_string(&uart_log_path).unwrap_or_default()
    } else {
        String::from("<no uart.log>")
    };

    if val["status"] != "pass" || val["stop_reason"] == "memory_violation" {
        println!("Test '{}' status: {}", name, val["status"]);
        println!("Stop reason: {}", val["stop_reason"]);
        println!("UART Log: {:?}", uart_log);
        println!("Stdout: {}", String::from_utf8_lossy(&output.stdout));
        println!("Stderr: {}", String::from_utf8_lossy(&output.stderr));
    }
    val
}

#[test]
fn test_max_steps_limit() {
    let script = r#"
schema_version: "1.0"
inputs:
  firmware: "__FIRMWARE__"
  system: "__SYSTEM__"
limits:
  max_steps: 10
assertions: []
"#;
    let result = run_test("max_steps", script);
    assert_eq!(result["stop_reason"], "max_steps");
    let steps = result["steps_executed"].as_u64().unwrap();
    assert!(steps <= 10);
}

#[test]
fn test_max_cycles_limit() {
    let script = r#"
schema_version: "1.0"
inputs:
  firmware: "__FIRMWARE__"
  system: "__SYSTEM__"
limits:
  max_steps: 20000
  max_cycles: 10
assertions: []
"#;
    let result = run_test("max_cycles", script);
    assert_eq!(result["stop_reason"], "max_cycles");
}

#[test]
fn test_cli_override_max_steps() {
    let temp_dir = std::env::temp_dir().join("labwired-cli-override");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let firmware_path = workspace_root.join("tests/fixtures/uart-ok-thumbv7m.elf");
    let system_path = workspace_root.join("configs/systems/ci-fixture-uart1.yaml");
    let firmware_path = firmware_path.canonicalize().unwrap();
    let system_path = system_path.canonicalize().unwrap();

    let script_content = format!(
        r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
limits:
  max_steps: 20000
assertions: []
"#,
        firmware_path.display(),
        system_path.display()
    );

    let script_path = temp_dir.join("script.yaml");
    std::fs::write(&script_path, script_content).unwrap();

    let output = Command::new(get_labwired_bin())
        .arg("test")
        .arg("--script")
        .arg(&script_path)
        .arg("--output-dir")
        .arg(&temp_dir)
        .arg("--no-uart-stdout")
        .arg("--max-steps")
        .arg("10")
        .output()
        .expect("Failed to run labwired");

    assert!(output.status.success());
    let result_json_path = temp_dir.join("result.json");
    let result_content =
        std::fs::read_to_string(&result_json_path).expect("Failed to read result.json");
    let result: Value = serde_json::from_str(&result_content).expect("Failed to parse");

    assert_eq!(result["result_schema_version"], "1.0");
    assert!(result["limits"]["max_steps"].is_number());
    assert_eq!(
        result["stop_reason_details"]["triggered_stop_condition"],
        result["stop_reason"]
    );

    assert_eq!(result["stop_reason"], "max_steps");
    assert_eq!(result["steps_executed"].as_u64().unwrap(), 10);
}

#[test]
fn test_uart_contains() {
    let script = r#"
schema_version: "1.0"
inputs:
  firmware: "__FIRMWARE__"
  system: "__SYSTEM__"
limits:
  max_steps: 100000
assertions:
  - uart_contains: "OK"
"#;
    let result = run_test("uart_pass", script);
    assert_eq!(result["status"], "pass");
}

#[test]
fn test_max_uart_bytes() {
    let script = r#"
schema_version: "1.0"
inputs:
  firmware: "__FIRMWARE__"
  system: "__SYSTEM__"
limits:
  max_steps: 100000
  max_uart_bytes: 2
assertions:
  - expected_stop_reason: max_uart_bytes
"#;
    let result = run_test("max_uart_bytes", script);
    assert_eq!(result["stop_reason"], "max_uart_bytes");
    assert_eq!(result["status"], "pass");
}

#[test]
fn test_no_progress_stuck() {
    let script = r#"
schema_version: "1.0"
inputs:
  firmware: "__FIRMWARE__"
  system: "__SYSTEM__"
limits:
  max_steps: 100000
  no_progress_steps: 100
assertions:
  - expected_stop_reason: no_progress
"#;
    let result = run_test("no_progress", script);
    assert_eq!(result["stop_reason"], "no_progress");
    assert_eq!(result["status"], "pass");
}
