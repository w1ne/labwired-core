// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Classic ESP32 fast boot must capture the UART selected by the board.
//! The same Arduino image prints on UART0 and toggles GPIO4 in every run.
//! Selecting UART1 must fail only the serial assertion, proving that the
//! firmware still executes instead of accepting a silent boot failure.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    exit_code: Option<i32>,
    result: serde_json::Value,
    uart: String,
    stderr: String,
}

fn run(console: Option<&str>) -> Run {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = labwired_cli::test_support::unique_temp_dir("esp32-fast-boot-console");
    std::fs::create_dir_all(&dir).expect("create output directory");
    let system = dir.join("system.yaml");
    let declaration = console
        .map(|name| format!("debug_uart: {name:?}\n"))
        .unwrap_or_default();
    std::fs::write(
        &system,
        format!(
            "name: console-selection\nchip: {:?}\n{declaration}\
             board_io:\n  - id: indicator\n    kind: led\n    peripheral: gpio\n    pin: 4\n    signal: output\n    active_high: true\n",
            root.join("configs/chips/esp32.yaml"),
        ),
    )
    .expect("write system");
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.2\"\n\
             inputs:\n  profile: arduino-esp32\n  firmware: {:?}\n  system: {system:?}\n\
             limits:\n  max_steps: 2000000\n\
             assertions:\n  - uart_contains: idf5-boot\n  - gpio_edges:\n      pin: gpio:4\n      min_edges: 2\n  - expected_stop_reason: max_steps\n",
            root.join("crates/cli/tests/fixtures/esp32_arduino_idf5_serial/firmware.elf"),
        ),
    )
    .expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&dir)
        .arg("--no-uart-stdout")
        .output()
        .expect("execute labwired");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let result = serde_json::from_slice(
        &std::fs::read(dir.join("result.json"))
            .unwrap_or_else(|err| panic!("missing result: {err}; stderr:\n{stderr}")),
    )
    .expect("parse result");
    let uart = std::fs::read_to_string(dir.join("uart.log")).unwrap_or_default();
    std::fs::remove_dir_all(&dir).expect("remove output directory");
    Run {
        exit_code: output.status.code(),
        result,
        uart,
        stderr,
    }
}

fn assertion_results(run: &Run) -> Vec<bool> {
    run.result["assertions"]
        .as_array()
        .expect("assertion results")
        .iter()
        .map(|assertion| assertion["passed"].as_bool().expect("assertion verdict"))
        .collect()
}

#[test]
fn declared_uart0_captures_the_running_firmware() {
    let run = run(Some("uart0"));
    assert_eq!(run.exit_code, Some(0), "{}", run.stderr);
    assert_eq!(assertion_results(&run), [true, true, true]);
    assert!(run.uart.contains("idf5-boot"), "{:?}", run.uart);
}

#[test]
fn wrong_uart_does_not_capture_uart0_while_gpio_keeps_running() {
    let run = run(Some("uart1"));
    assert_eq!(run.exit_code, Some(1), "{}", run.stderr);
    assert_eq!(run.result["status"], "fail");
    assert_eq!(assertion_results(&run), [false, true, true]);
    assert!(
        run.uart.is_empty(),
        "UART0 leaked into UART1: {:?}",
        run.uart
    );
}

#[test]
fn undeclared_console_preserves_uart_capture() {
    let run = run(None);
    assert_eq!(run.exit_code, Some(0), "{}", run.stderr);
    assert_eq!(assertion_results(&run), [true, true, true]);
    assert!(run.uart.contains("idf5-boot"), "{:?}", run.uart);
}

#[test]
fn missing_uart_fails_configuration_instead_of_substituting_uart0() {
    let run = run(Some("uart99"));
    assert_eq!(run.exit_code, Some(2), "{}", run.stderr);
    assert!(
        run.stderr.contains("this bus has no such UART"),
        "{}",
        run.stderr
    );
    assert!(run.uart.is_empty(), "{:?}", run.uart);
}
