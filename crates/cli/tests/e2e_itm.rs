// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// End-to-end coverage for ITM stimulus port 0 through `labwired test`:
// `--itm` (or an `itm_contains` assertion) enables capture, `itm.log` is the
// port-0 byte stream, and `result.json`'s `itm` block reports `observable`
// plus `bytes_drained`. The firmware is `firmware-itm-smoke`, which stores one
// byte while ITMENA is clear and then "ITM hello" once the port is enabled.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn ensure_firmware_built(root: &std::path::Path) -> PathBuf {
    let bin = labwired_core::test_support::target_dir()
        .join("thumbv7em-none-eabi/release/firmware-itm-smoke");
    let status = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "-p",
            "firmware-itm-smoke",
            "--release",
            "--target",
            "thumbv7em-none-eabi",
        ])
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .status()
        .expect("execute cargo build");
    assert!(
        status.success(),
        "failed to build firmware-itm-smoke (needs the thumbv7em-none-eabi target)"
    );
    assert!(bin.exists(), "expected binary at {:?}", bin);
    bin
}

struct ItmRun {
    exit_code: Option<i32>,
    status: String,
    itm: String,
    stdout: String,
    result: serde_json::Value,
}

fn run_script_args(
    root: &std::path::Path,
    fw: &std::path::Path,
    system: &std::path::Path,
    yaml_body: &str,
    extra_args: &[&str],
) -> ItmRun {
    let script_yaml = format!(
        r#"schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
{}"#,
        fw.display(),
        system.display(),
        yaml_body
    );

    let out_dir = labwired_cli::test_support::unique_temp_dir("labwired-itm-e2e");
    std::fs::create_dir_all(&out_dir).expect("create out dir");
    let script_path = out_dir.join("script.yaml");
    std::fs::write(&script_path, script_yaml).expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(root)
        .args(extra_args)
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

    let itm = std::fs::read_to_string(out_dir.join("itm.log")).unwrap_or_default();
    let result = std::fs::read_to_string(out_dir.join("result.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or(serde_json::Value::Null);
    let status = result["status"].as_str().unwrap_or_default().to_string();

    let _ = std::fs::remove_dir_all(&out_dir);
    ItmRun {
        exit_code: output.status.code(),
        status,
        itm,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        result,
    }
}

fn run_cortex(
    root: &std::path::Path,
    fw: &std::path::Path,
    yaml_body: &str,
    extra: &[&str],
) -> ItmRun {
    let system = root.join("examples/nrf52840-rtt-lab/system.yaml");
    run_script_args(root, fw, &system, yaml_body, extra)
}

/// Assertion without `--itm`: the contains check enables capture on its own.
#[test]
fn itm_assertion_without_flag_passes() {
    let root = repo_root();
    let fw = ensure_firmware_built(&root);
    let run = run_cortex(
        &root,
        &fw,
        r#"limits:
  max_steps: 100000
assertions:
  - itm_contains: "ITM hello"
  - expected_stop_reason: max_steps
"#,
        &[],
    );
    assert_eq!(
        run.exit_code,
        Some(0),
        "expected exit 0 (itm_contains passes); stdout: {}\nitm.log: {}",
        run.stdout,
        run.itm
    );
    assert_eq!(
        run.status, "pass",
        "stdout: {}\nitm.log: {}",
        run.stdout, run.itm
    );
    assert_eq!(
        run.itm, "ITM hello",
        "the disabled 'X' store must not be in the stream\n--- itm.log ---\n{}",
        run.itm
    );
    assert_eq!(run.result["itm"]["observable"].as_bool(), Some(true));
    assert_eq!(run.result["itm"]["bytes_drained"].as_u64(), Some(9));
    assert!(
        run.result.get("rtt").is_none() || run.result["rtt"].is_null(),
        "ITM must not be reported on the rtt object\n{}",
        run.result
    );
}

#[test]
fn wrong_itm_assertion_fails() {
    let root = repo_root();
    let fw = ensure_firmware_built(&root);
    let run = run_cortex(
        &root,
        &fw,
        r#"limits:
  max_steps: 100000
assertions:
  - itm_contains: "this string is never printed"
  - expected_stop_reason: max_steps
"#,
        &[],
    );
    assert_eq!(
        run.exit_code,
        Some(1),
        "expected exit 1; stdout: {}\nitm.log: {}",
        run.stdout,
        run.itm
    );
    assert_eq!(run.status, "fail");
    assert_eq!(run.itm, "ITM hello");
    assert_eq!(run.result["itm"]["observable"].as_bool(), Some(true));
}

/// `--itm` alone, with no `itm_contains`, writes the log. The same script
/// without the flag leaves `itm.log` empty and omits the `itm` key.
#[test]
fn itm_flag_without_assertion_writes_the_log() {
    let root = repo_root();
    let fw = ensure_firmware_built(&root);
    let body = r#"limits:
  max_steps: 100000
assertions:
  - expected_stop_reason: max_steps
"#;

    let with_flag = run_cortex(&root, &fw, body, &["--itm"]);
    assert_eq!(
        with_flag.exit_code,
        Some(0),
        "expected exit 0 with --itm; stdout: {}\nitm.log: {}",
        with_flag.stdout,
        with_flag.itm
    );
    assert_eq!(with_flag.itm, "ITM hello");
    assert_eq!(with_flag.result["itm"]["observable"].as_bool(), Some(true));
    assert_eq!(with_flag.result["itm"]["bytes_drained"].as_u64(), Some(9));

    let without_flag = run_cortex(&root, &fw, body, &[]);
    assert_eq!(
        without_flag.exit_code,
        Some(0),
        "stdout: {}",
        without_flag.stdout
    );
    assert!(
        without_flag.itm.is_empty(),
        "without --itm (and without itm_contains) itm.log must stay empty\n{}",
        without_flag.itm
    );
    assert!(
        without_flag.result.get("itm").is_none(),
        "without ITM enabled result.json must not carry an `itm` key\n{}",
        without_flag.result
    );
}

/// A RISC-V script cannot produce ITM. The assertion fails and `observable`
/// is false. The check does not search UART.
#[test]
fn riscv_itm_contains_fails_and_is_not_observable() {
    let root = repo_root();
    let fw = root.join("tests/fixtures/esp32c3-demo.elf");
    let system = root.join("configs/systems/esp32c3-devkit.yaml");
    assert!(fw.exists(), "missing {}", fw.display());
    let run = run_script_args(
        &root,
        &fw,
        &system,
        r#"limits:
  max_steps: 1000
assertions:
  - itm_contains: "ITM hello"
"#,
        &[],
    );
    assert_eq!(
        run.exit_code,
        Some(1),
        "expected exit 1; stdout: {}\nresult: {}",
        run.stdout,
        run.result
    );
    assert_eq!(run.status, "fail", "result: {}", run.result);
    assert!(run.itm.is_empty(), "itm.log: {}", run.itm);
    assert_eq!(
        run.result["itm"]["observable"].as_bool(),
        Some(false),
        "result: {}",
        run.result
    );
    assert_eq!(run.result["itm"]["bytes_drained"].as_u64(), Some(0));
}
