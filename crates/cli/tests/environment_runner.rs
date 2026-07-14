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
    // Deliberately list beta first: the runner contract serializes and captures
    // nodes in lexical id order, not input-manifest order.
    write_two_node_environment_in_order(dir, &["beta", "alpha"])
}

fn write_two_node_environment_in_order(
    dir: &Path,
    node_order: &[&str],
) -> (PathBuf, PathBuf, PathBuf) {
    assert_eq!(node_order.len(), 2, "fixture world must contain two nodes");
    let root = workspace_root();
    let firmware = std::fs::canonicalize(root.join("tests/fixtures/uart-ok-thumbv7m.elf"))
        .expect("fixture firmware");
    let system = std::fs::canonicalize(root.join("configs/systems/ci-fixture-uart1.yaml"))
        .expect("fixture system manifest");
    let environment = dir.join("two-node.yaml");

    let nodes = node_order
        .iter()
        .map(|id| {
            format!(
                "  - id: {id}\n    system: \"{}\"\n    firmware: \"{}\"\n",
                system.display(),
                firmware.display(),
            )
        })
        .collect::<String>();
    std::fs::write(
        &environment,
        format!(
            r#"schema_version: "1.0"
name: fixture-world
nodes:
{}"#,
            nodes,
        ),
    )
    .expect("write environment manifest");

    (environment, firmware, system)
}

/// A deliberately under-modelled world: the fixture writes UART MMIO, while
/// this chip exposes no peripherals. It gives the environment runner a real
/// unmapped-MMIO fidelity event and a runtime stop without mocking either.
fn write_tiny_two_node_environment(dir: &Path) -> PathBuf {
    let root = workspace_root();
    let firmware = std::fs::canonicalize(root.join("tests/fixtures/uart-ok-thumbv7m.elf"))
        .expect("fixture firmware");
    let chip = dir.join("tiny-chip.yaml");
    let system = dir.join("tiny-system.yaml");
    let environment = dir.join("tiny-two-node.yaml");

    std::fs::write(
        &chip,
        r#"name: "tiny"
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
    .expect("write tiny chip");
    std::fs::write(
        &system,
        r#"name: "tiny-system"
chip: "tiny-chip.yaml"
"#,
    )
    .expect("write tiny system");
    std::fs::write(
        &environment,
        format!(
            r#"schema_version: "1.0"
name: tiny-world
nodes:
  - id: alpha
    system: "tiny-system.yaml"
    firmware: "{}"
  - id: beta
    system: "tiny-system.yaml"
    firmware: "{}"
"#,
            firmware.display(),
            firmware.display(),
        ),
    )
    .expect("write tiny environment manifest");

    environment
}

fn assert_sha256(value: &serde_json::Value) {
    let value = value.as_str().expect("SHA-256 string");
    assert_eq!(value.len(), 64, "SHA-256 must be 64 hex characters");
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "SHA-256 must be lowercase hexadecimal: {value}"
    );
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
    assert_eq!(result["result_schema_version"], "1.0-environment");
    assert_eq!(result["run_type"], "environment");
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
    assert_sha256(&result["config"]["world_firmware_hash"]);

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "environment");
    let snapshot_nodes = snapshot["nodes"].as_array().expect("environment nodes");
    assert_eq!(snapshot_nodes[0]["id"], "alpha");
    assert_eq!(snapshot_nodes[1]["id"], "beta");
    assert!(snapshot_nodes[0]["state"]["cpu"].is_object());
    assert_eq!(snapshot_nodes[0]["cycles"], result["cycles"]);
    assert_eq!(snapshot_nodes[1]["cycles"], result["cycles"]);
    assert!(snapshot_nodes
        .iter()
        .all(|node| node["cycles"].as_u64().is_some_and(|cycles| cycles > 0)));

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
    assert_eq!(snapshot["nodes"][0]["cycles"], 0);
    assert_eq!(snapshot["nodes"][1]["cycles"], 0);

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
fn environment_runner_unusable_explicit_env_values_keep_environment_artifacts() {
    let mut empty_world_hashes = Vec::new();

    for (label, env_value) in [("null", "null"), ("number", "42")] {
        let dir = unique_dir(&format!("invalid-env-{label}"));
        let output = run_environment_script(
            &dir,
            &format!(
                r#"schema_version: "1.0"
inputs:
  env: {env_value}
limits:
  max_steps: 1
assertions: []
"#
            ),
            &[],
        );

        assert_eq!(
            output.status.code(),
            Some(2),
            "{label} env value: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output_dir = dir.join("artifacts");
        let result: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
        )
        .expect("parse result.json");
        assert_eq!(result["stop_reason"], "config_error");
        assert!(result["config"].get("firmware").is_none());
        assert!(result["config"]["environment"]
            .as_str()
            .expect("placeholder environment provenance")
            .ends_with("__labwired_invalid_inputs_env__.yaml"));
        assert!(result["config"]["nodes"]
            .as_array()
            .expect("environment nodes")
            .is_empty());
        assert_sha256(&result["config"]["world_firmware_hash"]);
        empty_world_hashes.push(
            result["config"]["world_firmware_hash"]
                .as_str()
                .expect("world firmware hash")
                .to_owned(),
        );

        let snapshot: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
        )
        .expect("parse snapshot.json");
        assert_eq!(snapshot["type"], "environment");
        assert_eq!(snapshot["status"], "error");
        assert!(snapshot["nodes"]
            .as_array()
            .expect("snapshot nodes")
            .is_empty());
        assert_eq!(
            std::fs::read_to_string(output_dir.join("uart.log")).expect("read uart.log"),
            ""
        );
        let junit = std::fs::read_to_string(output_dir.join("junit.xml")).expect("read junit.xml");
        assert!(junit.contains("config error"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    assert_eq!(
        empty_world_hashes[0], empty_world_hashes[1],
        "an empty environment world has deterministic provenance"
    );
}

#[test]
fn environment_runner_world_firmware_hash_is_order_independent() {
    let first_dir = unique_dir("world-firmware-hash-first");
    let second_dir = unique_dir("world-firmware-hash-second");
    write_two_node_environment_in_order(&first_dir, &["beta", "alpha"]);
    write_two_node_environment_in_order(&second_dir, &["alpha", "beta"]);
    let script = r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 1
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#;

    let first_output = run_environment_script(&first_dir, script, &[]);
    let second_output = run_environment_script(&second_dir, script, &[]);
    assert!(
        first_output.status.success(),
        "first world run failed: {}",
        String::from_utf8_lossy(&first_output.stderr)
    );
    assert!(
        second_output.status.success(),
        "second world run failed: {}",
        String::from_utf8_lossy(&second_output.stderr)
    );

    let first: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(first_dir.join("artifacts/result.json"))
            .expect("read first result.json"),
    )
    .expect("parse first result.json");
    let second: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(second_dir.join("artifacts/result.json"))
            .expect("read second result.json"),
    )
    .expect("parse second result.json");
    assert_sha256(&first["config"]["world_firmware_hash"]);
    assert_sha256(&second["config"]["world_firmware_hash"]);
    assert_eq!(
        first["config"]["world_firmware_hash"], second["config"]["world_firmware_hash"],
        "manifest declaration order must not change the world firmware identity"
    );

    let _ = std::fs::remove_dir_all(&first_dir);
    let _ = std::fs::remove_dir_all(&second_dir);
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
fn environment_runner_memory_assertions_keep_single_node_u32_mask_semantics() {
    let dir = unique_dir("memory-mask-semantics");
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
      node: alpha
      address: 0x20000000
      expected_value: 0x100
      size: 8
"#,
        &[],
    );

    assert_eq!(
        output.status.code(),
        Some(1),
        "an 8-bit zero must not equal an unmasked u32 expected value: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["status"], "fail");
    assert_eq!(result["assertions"][0]["passed"], false);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_rejects_explicit_default_trace_max() {
    let dir = unique_dir("explicit-trace-max");
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
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &["--trace-max", "100000"],
    );

    assert_eq!(output.status.code(), Some(2));
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["message"]
        .as_str()
        .expect("config error message")
        .contains("--trace/--vcd/--trace-max"));
    assert!(result["config"].get("firmware").is_none());
    assert_eq!(result["config"]["nodes"].as_array().unwrap().len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_environment_script_with_recognizable_env_keeps_environment_artifacts() {
    let dir = unique_dir("malformed-environment-script");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: [1
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
fn malformed_inline_environment_script_keeps_environment_artifacts() {
    let dir = unique_dir("malformed-inline-environment-script");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs: { env: "two-node.yaml" }
limits:
  max_steps: [1
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(2));
    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["run_type"], "environment");
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["config"].get("firmware").is_none());
    assert!(result["config"]["environment"]
        .as_str()
        .expect("environment provenance")
        .ends_with("two-node.yaml"));

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "environment");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_script_without_a_direct_inputs_env_keeps_legacy_artifacts() {
    let dir = unique_dir("ambiguous-malformed-script");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"notes: |
  inputs:
    env: "two-node.yaml"
limits:
  max_steps: [1
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(2));
    let output_dir = dir.join("artifacts");
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert!(
        result["config"].get("firmware").is_some(),
        "a scalar mentioning inputs.env must not be reclassified as an environment run"
    );

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output_dir.join("snapshot.json")).expect("read snapshot.json"),
    )
    .expect("parse snapshot.json");
    assert_eq!(snapshot["type"], "config_error");

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
    assert_eq!(
        uart_output.status.code(),
        Some(1),
        "a max_uart_bytes safety stop must fail even when memory assertions pass: {}",
        String::from_utf8_lossy(&uart_output.stderr)
    );
    let uart_result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(uart_dir.join("artifacts/result.json")).expect("read UART result"),
    )
    .expect("parse UART result");
    assert_eq!(uart_result["status"], "fail");
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
    let junit =
        std::fs::read_to_string(uart_dir.join("artifacts/junit.xml")).expect("read UART junit");
    assert!(junit.contains("failures=\"1\""));
    assert!(junit.contains("errors=\"0\""));

    let _ = std::fs::remove_dir_all(&cycle_dir);
    let _ = std::fs::remove_dir_all(&uart_dir);
}

#[test]
fn environment_runner_wall_time_safety_stop_fails_even_when_assertions_pass() {
    let dir = unique_dir("wall-time-safety-stop");
    let (_environment, _firmware, _system) = write_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "two-node.yaml"
limits:
  max_steps: 100
  wall_time_ms: 0
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["status"], "fail");
    assert_eq!(result["stop_reason"], "wall_time");
    assert_eq!(result["assertions"][0]["passed"], true);
    let junit = std::fs::read_to_string(dir.join("artifacts/junit.xml")).expect("read junit");
    assert!(junit.contains("failures=\"1\""));
    assert!(junit.contains("errors=\"0\""));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_prioritizes_failed_assertions_over_runtime_errors() {
    let dir = unique_dir("assertion-before-runtime-error");
    write_tiny_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "tiny-two-node.yaml"
limits:
  max_steps: 100
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 1
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    assert_eq!(result["status"], "fail");
    assert_eq!(result["stop_reason"], "memory_violation");
    assert_eq!(result["assertions"][0]["passed"], false);
    let junit = std::fs::read_to_string(dir.join("artifacts/junit.xml")).expect("read junit");
    assert!(junit.contains("failures=\"1\""));
    assert!(junit.contains("errors=\"0\""));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn environment_runner_surfaces_fidelity_gaps_from_world_execution() {
    let dir = unique_dir("fidelity");
    write_tiny_two_node_environment(&dir);
    let output = run_environment_script(
        &dir,
        r#"schema_version: "1.0"
inputs:
  env: "tiny-two-node.yaml"
limits:
  max_steps: 100
assertions:
  - memory_value:
      node: alpha
      address: 0x20000000
      expected_value: 0
"#,
        &[],
    );

    assert_eq!(output.status.code(), Some(3));
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("artifacts/result.json")).expect("read result.json"),
    )
    .expect("parse result.json");
    let fidelity = result["fidelity"]
        .as_array()
        .expect("environment result must surface fidelity gaps");
    let mmio = fidelity
        .iter()
        .find(|gap| gap["kind"] == "unmapped_mmio")
        .expect("tiny world must report its unmapped UART MMIO access");
    assert!(mmio["address"]
        .as_str()
        .is_some_and(|address| address.starts_with("0x")));
    assert_eq!(mmio["detail"], "write");

    let _ = std::fs::remove_dir_all(&dir);
}
