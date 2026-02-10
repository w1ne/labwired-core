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
fn test_cli_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .arg("--help")
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("LabWired Simulator"));
}

#[test]
fn test_cli_load_missing_file() {
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .arg("-f")
        .arg("non_existent_file.elf")
        .output()
        .expect("Failed to execute command");

    // It should fail because file is missing
    assert!(!output.status.success());
}

#[test]
fn test_cli_test_mode_passes_with_zero_steps() {
    let script = write_temp_file(
        "script-pass",
        r#"
schema_version: "1.0"
inputs:
  firmware: "../../tests/fixtures/uart-ok-thumbv7m.elf"
limits:
  max_steps: 1
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

    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn test_cli_test_mode_assertion_fail_exit_1() {
    let script = write_temp_file(
        "script-fail",
        r#"
schema_version: "1.0"
inputs:
  firmware: "../../tests/fixtures/uart-ok-thumbv7m.elf"
limits:
  max_steps: 1
assertions:
  - uart_contains: "this string will not be present"
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

#[test]
fn test_cli_test_mode_runtime_error_exit_3() {
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
        "script-runtime",
        r#"
schema_version: "1.0"
inputs:
  firmware: "../../tests/fixtures/uart-ok-thumbv7m.elf"
limits:
  max_steps: 1
assertions: []
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

    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn test_cli_version_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .arg("--version")
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Check if version is present (format usually "labwired x.y.z")
    assert!(stdout.starts_with("labwired"));
}

#[test]
fn test_cli_invalid_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .arg("--unknown-flag-xyz")
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("error: unexpected argument '--unknown-flag-xyz'"));
}

#[test]
fn test_cli_execution_limit_cycles() {
    let script = write_temp_file(
        "script-limit",
        r#"
schema_version: "1.0"
inputs:
  firmware: "../../tests/fixtures/uart-ok-thumbv7m.elf"
limits:
  max_steps: 100
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

    if !output.status.success() {
        println!("Stdout: {}", String::from_utf8_lossy(&output.stdout));
        println!("Stderr: {}", String::from_utf8_lossy(&output.stderr));
    }
    assert!(output.status.success());
}
