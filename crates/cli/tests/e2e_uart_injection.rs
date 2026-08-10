// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// End-to-end coverage for declarative UART RX byte injection (test-script
// schema 1.2, `uart_injections`) via `firmware-uart-echo-fixture`: a minimal
// Cortex-M3 program that prints "READY", then echoes every byte it reads off
// UART1 (with "Q" replying "BYE" instead of an echo).
//
// This is what keeps the byte-injection path honest end to end: a regression
// in `UartInjectionSpec` parsing, `attach_uart_rx_source_named`, or the
// `labwired test` run-loop wiring fails the merge gate here.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

/// Build `firmware-uart-echo-fixture` for thumbv6m-none-eabi if its release
/// binary isn't already sitting in `target/` (mirrors what
/// `examples/ci-fixture-arm/ci/build.sh` does for the sibling CI fixture).
fn ensure_fixture_built(root: &std::path::Path) -> PathBuf {
    let bin = labwired_core::test_support::target_dir()
        .join("thumbv6m-none-eabi/release/firmware-uart-echo-fixture");
    if bin.exists() {
        return bin;
    }
    let status = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "-p",
            "firmware-uart-echo-fixture",
            "--release",
            "--target",
            "thumbv6m-none-eabi",
        ])
        .status()
        .expect("execute cargo build");
    assert!(
        status.success(),
        "failed to build firmware-uart-echo-fixture"
    );
    assert!(bin.exists(), "expected binary at {:?}", bin);
    bin
}

struct InjectionRun {
    exit_code: Option<i32>,
    stderr: String,
    uart: String,
    status: String,
}

fn run_script(root: &std::path::Path, script_yaml: &str) -> InjectionRun {
    // Timestamp PLUS a process-wide counter. `as_nanos()` is not actually
    // nanosecond-granular on macOS, so sibling tests starting in the same
    // microsecond used to land in the SAME directory and overwrite each
    // other's script.yaml/result.json — a random subset of this file failed
    // on every parallel run, a different subset each time, while
    // `--test-threads=1` always passed. The counter makes the name unique
    // regardless of what the clock resolves to.
    let out_dir = labwired_cli::test_support::unique_temp_dir("labwired-uart-injection");
    std::fs::create_dir_all(&out_dir).expect("create out dir");
    let script_path = out_dir.join("script.yaml");
    std::fs::write(&script_path, script_yaml).expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(root)
        .args([
            "test",
            "--script",
            script_path.to_str().unwrap(),
            "--no-uart-stdout",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute labwired");

    let uart = std::fs::read_to_string(out_dir.join("uart.log")).unwrap_or_default();
    let status = std::fs::read_to_string(out_dir.join("result.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["status"].as_str().map(str::to_string))
        .unwrap_or_default();

    let _ = std::fs::remove_dir_all(&out_dir);
    InjectionRun {
        exit_code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        uart,
        status,
    }
}

fn script(root: &std::path::Path, fw: &std::path::Path, extra: &str) -> String {
    format!(
        r#"
schema_version: "1.2"
inputs:
  firmware: "{}"
  system: "{}"
limits:
  max_steps: 20000
{extra}
"#,
        fw.display(),
        root.join("configs/systems/ci-fixture-uart1.yaml").display(),
    )
}

#[test]
fn injected_bytes_are_echoed_back() {
    let root = repo_root();
    let fw = ensure_fixture_built(&root);
    let yaml = script(
        &root,
        &fw,
        r#"uart_injections:
  - uart: "uart1"
    bytes: "AB"
assertions:
  - uart_contains: "READY"
  - uart_contains: "AB"
"#,
    );
    let run = run_script(&root, &yaml);
    assert_eq!(
        run.exit_code,
        Some(0),
        "expected exit 0 (assertions pass); stderr: {}\nuart: {}",
        run.stderr,
        run.uart
    );
    assert_eq!(run.status, "pass");
    assert!(
        run.uart.contains("AB"),
        "expected echoed bytes\n{}",
        run.uart
    );
}

/// Discriminating half of the above: feeding the WRONG bytes must fail the
/// exact same assertion that passes on correct input — proves the assertion
/// is actually reading the injected/echoed bytes, not vacuously true.
#[test]
fn wrong_injected_bytes_fail_the_same_assertion() {
    let root = repo_root();
    let fw = ensure_fixture_built(&root);
    let yaml = script(
        &root,
        &fw,
        r#"uart_injections:
  - uart: "uart1"
    bytes: "XY"
assertions:
  - uart_contains: "READY"
  - uart_contains: "AB"
"#,
    );
    let run = run_script(&root, &yaml);
    assert_eq!(
        run.exit_code,
        Some(1),
        "expected exit 1 (uart_contains 'AB' assertion fails); stderr: {}\nuart: {}",
        run.stderr,
        run.uart
    );
    assert_eq!(run.status, "fail");
    assert!(
        run.uart.contains("XY"),
        "expected the wrong bytes to actually be echoed (proves they were delivered)\n{}",
        run.uart
    );
    assert!(
        !run.uart.contains("AB"),
        "must not contain the expected bytes when the wrong ones were injected\n{}",
        run.uart
    );
}

/// Command-byte branching, not just plain echo: 'Q' gets a fixed "BYE" reply
/// instead of an echo, proving firmware can branch on injected input.
#[test]
fn command_byte_triggers_a_different_reply_than_echo() {
    let root = repo_root();
    let fw = ensure_fixture_built(&root);
    let yaml = script(
        &root,
        &fw,
        r#"uart_injections:
  - uart: "uart1"
    bytes: "Q"
assertions:
  - uart_contains: "BYE"
"#,
    );
    let run = run_script(&root, &yaml);
    assert_eq!(
        run.exit_code,
        Some(0),
        "stderr: {}\nuart: {}",
        run.stderr,
        run.uart
    );
    assert!(
        !run.uart.contains("QQQ"),
        "'Q' must not be echoed literally"
    );
}

/// Timing semantics: a byte injected `at_start` (the default trigger, applied
/// before the firmware's first instruction) is NOT dropped even though the
/// firmware hasn't executed a single instruction yet — it sits in the UART's
/// RX queue until the firmware reads it. This is the empirically-verified
/// "buffered, not dropped" behavior (`Uart::read`: RX presence is derived
/// from the queue being non-empty; there is no enable-bit gate on injection).
#[test]
fn at_start_injection_before_any_firmware_instruction_is_buffered_not_dropped() {
    let root = repo_root();
    let fw = ensure_fixture_built(&root);
    // Default trigger (omitted) is `at_start`: applied before `load_firmware`
    // even begins stepping the CPU.
    let yaml = script(
        &root,
        &fw,
        r#"uart_injections:
  - uart: "uart1"
    bytes: "Z"
assertions:
  - uart_contains: "READY"
  - uart_contains: "Z"
"#,
    );
    let run = run_script(&root, &yaml);
    assert_eq!(
        run.exit_code,
        Some(0),
        "an at_start injection must still be observed once firmware starts polling; stderr: {}\nuart: {}",
        run.stderr,
        run.uart
    );
    assert_eq!(run.status, "pass");
}

/// An `after_cycles` injection delivered well into the run is observed too —
/// same buffered semantics, just delivered later instead of pre-boot.
#[test]
fn after_cycles_injection_mid_run_is_also_observed() {
    let root = repo_root();
    let fw = ensure_fixture_built(&root);
    let yaml = script(
        &root,
        &fw,
        r#"uart_injections:
  - uart: "uart1"
    bytes: "M"
    trigger: !after_cycles { cycles: 200 }
assertions:
  - uart_contains: "READY"
  - uart_contains: "M"
"#,
    );
    let run = run_script(&root, &yaml);
    assert_eq!(
        run.exit_code,
        Some(0),
        "stderr: {}\nuart: {}",
        run.stderr,
        run.uart
    );
    assert!(
        run.stderr.contains("uart_injection: 1 byte(s) delivered"),
        "expected the runner to log the applied injection\n--- stderr ---\n{}",
        run.stderr
    );
}

/// Naming a UART peripheral that doesn't exist on the built machine is a
/// hard config error (exit code 2), not a silent no-op — a script relying on
/// injected input that never arrives must not report a false pass.
#[test]
fn unknown_uart_id_is_a_hard_config_error() {
    let root = repo_root();
    let fw = ensure_fixture_built(&root);
    let yaml = script(
        &root,
        &fw,
        r#"uart_injections:
  - uart: "does-not-exist"
    bytes: "A"
assertions:
  - uart_contains: "READY"
"#,
    );
    let run = run_script(&root, &yaml);
    assert_eq!(
        run.exit_code,
        Some(2),
        "expected the config-error exit code; stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("not found on the bus"),
        "expected a clear diagnostic naming the missing peripheral\n{}",
        run.stderr
    );
}
