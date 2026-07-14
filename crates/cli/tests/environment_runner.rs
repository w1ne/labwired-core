// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! Black-box contract tests for the released multi-node `labwired test` runner.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before Unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "labwired-environment-runner-{label}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temporary environment directory");
    dir
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates directory")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn write_two_node_environment(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let root = workspace_root();
    let firmware = std::fs::canonicalize(root.join("tests/fixtures/uart-ok-thumbv7m.elf"))
        .expect("fixture firmware");
    let system = std::fs::canonicalize(root.join("configs/systems/ci-fixture-uart1.yaml"))
        .expect("fixture system manifest");
    let environment = dir.join("two-node.yaml");

    // Deliberately list beta first: the runner contract serializes and captures
    // nodes in lexical id order, not input-manifest order.
    std::fs::write(
        &environment,
        format!(
            r#"schema_version: "1.0"
name: fixture-world
nodes:
  - id: beta
    system: "{}"
    firmware: "{}"
  - id: alpha
    system: "{}"
    firmware: "{}"
"#,
            system.display(),
            firmware.display(),
            system.display(),
            firmware.display(),
        ),
    )
    .expect("write environment manifest");

    (environment, firmware, system)
}

fn run_environment_script(dir: &Path, script: &str, extra_args: &[&str]) -> std::process::Output {
    let script_path = dir.join("gate.yaml");
    std::fs::write(&script_path, script).expect("write environment test script");
    let output_dir = dir.join("artifacts");

    let mut command = Command::new(env!("CARGO_BIN_EXE_labwired"));
    command
        .arg("test")
        .arg("--script")
        .arg(&script_path)
        .arg("--no-uart-stdout")
        .arg("--output-dir")
        .arg(&output_dir);
    command.args(extra_args);
    command.output().expect("run labwired environment test")
}

#[test]
fn environment_runner_writes_sorted_real_world_artifacts() {
    let dir = unique_dir("artifacts");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 100
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
      size: 8
      mask: 0xff
  - memory_value:
      node: beta
      address: 0x20000000
      expected_value: 0
      size: 16
      mask: 0xffff
"#,
        &[],
    );

    assert!(
        output.status.success(),
        "environment run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["status"], "pass");
    assert_eq!(result["stop_reason"], "max_steps");
    assert_eq!(result["steps_executed"], 100);
    assert_eq!(result["instructions"], 200);
    assert!(result["config"].get("firmware").is_none());
    assert!(result["config"]["environment"]
        .as_str()
        .expect("environment provenance")
        .ends_with("two-node.yaml"));
    let nodes = result["config"]["nodes"]
        .as_array()
        .expect("per-node provenance");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["id"], "alpha");
    assert_eq!(nodes[1]["id"], "beta");
    assert!(nodes
        .iter()
        .all(|node| node["firmware_hash"].as_str().is_some()));
    assert!(nodes
        .iter()
        .all(|node| node["system_hash"].as_str().is_some()));

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "environment");
    let snapshot_nodes = snapshot["nodes"].as_array().expect("environment nodes");
    assert_eq!(snapshot_nodes[0]["id"], "alpha");
    assert_eq!(snapshot_nodes[1]["id"], "beta");
    assert!(snapshot_nodes[0]["state"]["cpu"].is_object());

    let uart = std::fs::read_to_string(output_dir.join("uart.log")).expect("read uart.log");
    assert!(uart.starts_with("[node:alpha]\n"));
    let beta = uart.find("[node:beta]\n").expect("beta UART section");
    assert!(
        beta > 0,
        "UART sections must be sorted by node id: {uart:?}"
    );
    assert_eq!(uart.matches("[node:").count(), 2);
    assert!(uart.contains("OK\n"));

    let junit = std::fs::read_to_string(output_dir.join("junit.xml")).expect("read junit.xml");
    assert!(junit.contains("name=\"run\""));
    assert!(junit.contains("name=\"assertion 1:"));
    assert!(junit.contains("name=\"assertion 2:"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_unknown_assertion_node_is_a_config_error_with_environment_artifacts() {
    let dir = unique_dir("unknown-node");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1
assertions:
  - memory_value:
      node: missing
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(2));
    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["status"], "error");
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["message"]
        .as_str()
        .expect("config error message")
        .contains("nonexistent node 'missing'"));
    assert!(result["config"].get("firmware").is_none());
    assert_eq!(result["config"]["nodes"][0]["id"], "alpha");
    assert_eq!(result["config"]["nodes"][1]["id"], "beta");

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "environment");
    assert_eq!(snapshot["status"], "error");
    assert_eq!(snapshot["nodes"][0]["id"], "alpha");
    assert_eq!(snapshot["nodes"][1]["id"], "beta");

    let uart = std::fs::read_to_string(output_dir.join("uart.log")).expect("read uart.log");
    assert_eq!(uart, "[node:alpha]\n[node:beta]\n");
    let junit = std::fs::read_to_string(output_dir.join("junit.xml")).expect("read junit.xml");
    assert!(junit.contains("name=\"run\""));
    assert!(junit.contains("config error"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_parser_config_error_keeps_environment_provenance() {
    let dir = unique_dir("parser-config-error");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1
  no_progress_steps: 1
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(2));
    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["config"].get("firmware").is_none());
    assert!(result["config"]["environment"]
        .as_str()
        .expect("environment provenance")
        .ends_with("two-node.yaml"));
    assert_eq!(result["config"]["nodes"][0]["id"], "alpha");
    assert_eq!(result["config"]["nodes"][1]["id"], "beta");

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "environment");
    assert_eq!(
        std::fs::read_to_string(output_dir.join("uart.log")).expect("read uart.log"),
        "[node:alpha]\n[node:beta]\n"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_rejects_single_machine_firmware_and_system_overrides() {
    let dir = unique_dir("overrides");
    let (_environment, firmware, system) = write_two_node_environment(&dir);
    let firmware_text = firmware.display().to_string();
    let system_text = system.display().to_string();
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &["--firmware", &firmware_text, "--system", &system_text],
    );

    assert_eq!(output.status.code(), Some(2));
    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["stop_reason"], "config_error");
    let message = result["message"].as_str().expect("config error message");
    assert!(message.contains("--firmware"));
    assert!(message.contains("--system"));
    assert!(message.contains("topology comes exclusively from inputs.env"));
    assert!(result["config"].get("firmware").is_none());
    assert_eq!(result["config"]["nodes"].as_array().unwrap().len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_reports_world_cycle_and_total_uart_limits_truthfully() {
    let cycle_dir = unique_dir("max-cycles");
    let (_environment, _firmware, _system) = write_two_node_environment(&cycle_dir);
    let cycle_output = run_environment_script(
        &cycle_dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 100
  max_cycles: 2
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );
    assert!(
        cycle_output.status.success(),
        "cycle-limited world failed: {}",
        String::from_utf8_lossy(&cycle_output.stderr)
    );
    let cycle_result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(cycle_dir.join("artifacts/result.json"))
            .expect("read cycle result"),
    )
    .expect("parse cycle result");
    assert_eq!(cycle_result["stop_reason"], "max_cycles");
    assert_eq!(
        cycle_result["stop_reason_details"]["triggered_limit"]["name"],
        "max_cycles"
    );
    assert_eq!(
        cycle_result["stop_reason_details"]["triggered_limit"]["value"],
        2
    );
    assert!(cycle_result["cycles"].as_u64().unwrap() >= 2);
    assert_eq!(cycle_result["instructions"], 4);

    let uart_dir = unique_dir("max-uart");
    let (_environment, _firmware, _system) = write_two_node_environment(&uart_dir);
    let uart_output = run_environment_script(
        &uart_dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1000
  max_uart_bytes: 1
assertions:
  - memory_value:
      node: beta
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );
    assert!(
        uart_output.status.success(),
        "UART-limited world failed: {}",
        String::from_utf8_lossy(&uart_output.stderr)
    );
    let uart_result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(uart_dir.join("artifacts/result.json")).expect("read UART result"),
    )
    .expect("parse UART result");
    assert_eq!(uart_result["stop_reason"], "max_uart_bytes");
    assert_eq!(
        uart_result["stop_reason_details"]["triggered_limit"]["name"],
        "max_uart_bytes"
    );
    assert_eq!(
        uart_result["stop_reason_details"]["triggered_limit"]["value"],
        1
    );
    assert!(
        uart_result["stop_reason_details"]["observed"]["value"]
            .as_u64()
            .unwrap()
            >= 1
    );
    assert!(uart_result["instructions"].as_u64().unwrap() >= 2);

    let _ = std::fs::remove_dir_all(&cycle_dir);
    let _ = std::fs::remove_dir_all(&uart_dir);
}
