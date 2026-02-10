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

fn write_tiny_system() -> PathBuf {
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

    system_path
}

#[test]
fn test_script_unknown_fields_exit_2() {
    let script = write_temp_file(
        "script-unknown",
        r#"
schema_version: "1.0"
inputs:
  firmware: "../../tests/fixtures/uart-ok-thumbv7m.elf"
limits:
  max_steps: 1
unexpected_field: 123
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--firmware",
            "../../tests/fixtures/uart-ok-thumbv7m.elf",
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
        ])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn test_expected_stop_reason_allows_sim_error_to_pass() {
    let system_path = write_tiny_system();
    let script = write_temp_file(
        "script-expected-stop",
        r#"
schema_version: "1.0"
inputs:
  firmware: "../../tests/fixtures/uart-ok-thumbv7m.elf"
  system: "system.yaml"
limits:
  max_steps: 1
assertions:
  - expected_stop_reason: memory_violation
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--firmware",
            "../../tests/fixtures/uart-ok-thumbv7m.elf",
            "--system",
            system_path.to_str().unwrap(),
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
        ])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn test_wall_time_stop_fails_without_expected_stop_reason() {
    let script = write_temp_file(
        "script-wall-time",
        r#"
schema_version: "1.0"
inputs:
  firmware: "../../tests/fixtures/uart-ok-thumbv7m.elf"
limits:
  max_steps: 1
  wall_time_ms: 0
assertions: []
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--firmware",
            "../../tests/fixtures/uart-ok-thumbv7m.elf",
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
        ])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(1));
}
