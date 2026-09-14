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
fn test_stop_when_assertions_pass() {
    let script = r#"
schema_version: "1.0"
inputs:
  firmware: "__FIRMWARE__"
  system: "__SYSTEM__"
limits:
  max_steps: 1000000
  stop_when_assertions_pass: true
assertions:
  - uart_contains: "OK"
"#;
    let result = run_test("assertions_passed", script);
    assert_eq!(result["status"], "pass");
    assert_eq!(result["stop_reason"], "assertions_passed");
    assert!(
        result["steps_executed"].as_u64().unwrap() < 1000000,
        "runner should stop before max_steps once assertions pass"
    );
}

/// Regression guard for the print-then-crash false-pass hole.
///
/// The `uart-then-bkpt-thumbv7m.elf` fixture emits the acceptance token
/// ("OK\n") to UART1 and then executes `bkpt #0`, which the simulator surfaces
/// as `SimulationError::Halt` (the firmware-reachable abort path; a Rust
/// panic/abort lowers to the same trap). Before the settling window was added,
/// `stop_when_assertions_pass` declared victory the instant the token appeared
/// and never saw the crash. With a settle window, the machine keeps executing
/// past the first all-pass and hits the trap during the window, so the run must
/// report the fault (`halt`) rather than `assertions_passed` even though the
/// token is present in the UART log.
#[test]
fn test_stop_when_assertions_pass_does_not_mask_crash() {
    let temp_dir = std::env::temp_dir().join("labwired-stop-crash");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let firmware_path = workspace_root
        .join("tests/fixtures/uart-then-bkpt-thumbv7m.elf")
        .canonicalize()
        .expect("print-then-crash fixture must exist");
    let system_path = workspace_root
        .join("configs/systems/ci-fixture-uart1.yaml")
        .canonicalize()
        .unwrap();

    let script_content = format!(
        r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
limits:
  max_steps: 1000000
  stop_when_assertions_pass: true
  stop_when_assertions_pass_settle_steps: 50
assertions:
  - uart_contains: "OK"
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
        .output()
        .expect("Failed to run labwired");

    let result_json_path = temp_dir.join("result.json");
    let result_content = std::fs::read_to_string(&result_json_path).unwrap_or_else(|_| {
        panic!(
            "no result.json. Exit {:?}\nStdout: {}\nStderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let result: Value = serde_json::from_str(&result_content).expect("Failed to parse result.json");

    // The token IS present — the firmware printed its acceptance string.
    let uart_log = std::fs::read_to_string(temp_dir.join("uart.log")).unwrap_or_default();
    assert!(
        uart_log.contains("OK"),
        "fixture should have emitted the acceptance token; uart.log = {uart_log:?}"
    );

    // ...but the run crashed right after, so it must NOT certify as passed.
    assert_ne!(
        result["stop_reason"], "assertions_passed",
        "print-then-crash firmware must not certify as assertions_passed"
    );
    // The crash is surfaced as a fault stop reason (`halt` for the bkpt trap).
    let stop_reason = result["stop_reason"].as_str().unwrap();
    assert!(
        matches!(
            stop_reason,
            "halt" | "memory_violation" | "decode_error" | "exception"
        ),
        "expected a fault stop reason after the crash, got {stop_reason:?}"
    );
    assert_ne!(
        result["status"], "pass",
        "a run that crashed after the token must not report status=pass"
    );
}

/// Zephyr L0 hello on nRF52840 prints `LW_Z0_OK` then `k_sleep(K_FOREVER)`.
/// That parks the CPU on WFI; the next RTC event is overflow at 512 s of
/// 64 MHz time (~32e9 cycles). After the UART assertion latches, idle
/// fast-forward must stay off so settle is still "N more instructions"
/// (print-then-bkpt still works) — not a skip to RTC overflow.
#[cfg(feature = "event-scheduler")]
#[test]
fn zephyr_hello_nrf52840_does_not_fast_forward_rtc_overflow_to_settle() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let firmware = workspace_root
        .join("tests/fixtures/nrf52840-zephyr-l0-hello.elf")
        .canonicalize()
        .expect("pinned nRF52840 Zephyr L0 ELF must exist");
    let system_path = workspace_root
        .join("validation/zephyr-matrix/systems/nrf52840.yaml")
        .canonicalize()
        .unwrap();

    let temp_dir = std::env::temp_dir().join("labwired-nrf52840-z0-settle");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let script_path = temp_dir.join("script.yaml");
    std::fs::write(
        &script_path,
        format!(
            r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
limits:
  max_steps: 2000000
  stop_when_assertions_pass: true
  stop_when_assertions_pass_settle_steps: 1000
assertions:
  - uart_contains: "LW_Z0_OK"
"#,
            firmware.display(),
            system_path.display()
        ),
    )
    .unwrap();

    let started = std::time::Instant::now();
    let output = Command::new(get_labwired_bin())
        .arg("test")
        .arg("--script")
        .arg(&script_path)
        .arg("--output-dir")
        .arg(&temp_dir)
        .arg("--no-uart-stdout")
        .output()
        .expect("labwired test");
    let elapsed = started.elapsed();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "labwired test failed: {stderr}");
    let result: Value =
        serde_json::from_str(&std::fs::read_to_string(temp_dir.join("result.json")).unwrap())
            .unwrap();
    assert_eq!(result["status"], "pass");
    assert_eq!(result["stop_reason"], "assertions_passed");
    let steps = result["steps_executed"].as_u64().unwrap();
    assert!(
        elapsed.as_secs_f64() < 1.0,
        "hello+WFI settle must not skip 512s of RTC overflow; wall={elapsed:?}\n{stderr}"
    );
    // Settle is 1000 more instructions after first all-pass, not a parked
    // short-circuit. Boot is ~18k steps; the window must still be walked.
    assert!(
        steps >= 18_000 + 1_000,
        "settle_steps=1000 must still run after LW_Z0_OK (steps={steps})"
    );
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
