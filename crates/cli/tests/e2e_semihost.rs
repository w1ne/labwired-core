// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// `labwired test` coverage for the semihosting stream: pass, fail,
// flag-without-assertion writes semihosting.log, and assertion-without-flag
// enables capture. A non-Cortex-M script fails closed.

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
        .join("thumbv7em-none-eabi/release/firmware-semihost-smoke");
    let status = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "-p",
            "firmware-semihost-smoke",
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
        "failed to build firmware-semihost-smoke (needs the thumbv7em target)"
    );
    assert!(bin.exists(), "expected binary at {:?}", bin);
    bin
}

struct SemiRun {
    exit_code: Option<i32>,
    status: String,
    log: String,
    stdout: String,
    stderr: String,
    result: serde_json::Value,
}

fn run_arm(
    root: &std::path::Path,
    fw: &std::path::Path,
    yaml_body: &str,
    extra: &[&str],
) -> SemiRun {
    let system = root.join("examples/nrf52840-rtt-lab/system.yaml");
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
    run_script(root, &script_yaml, extra)
}

fn run_script(root: &std::path::Path, script_yaml: &str, extra: &[&str]) -> SemiRun {
    let out_dir = labwired_cli::test_support::unique_temp_dir("labwired-semihost-e2e");
    std::fs::create_dir_all(&out_dir).expect("create out dir");
    let script_path = out_dir.join("script.yaml");
    std::fs::write(&script_path, script_yaml).expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(root)
        .args(extra)
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

    let log = std::fs::read_to_string(out_dir.join("semihosting.log")).unwrap_or_default();
    let result = std::fs::read_to_string(out_dir.join("result.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or(serde_json::Value::Null);
    let status = result["status"].as_str().unwrap_or_default().to_string();
    let run = SemiRun {
        exit_code: output.status.code(),
        status,
        log,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        result,
    };
    let _ = std::fs::remove_dir_all(&out_dir);
    run
}

fn arm_limits() -> &'static str {
    "limits:\n  max_steps: 200000\n"
}

#[test]
fn semihost_banner_passes_and_exits_zero() {
    let root = repo_root();
    let fw = ensure_firmware_built(&root);
    let run = run_arm(
        &root,
        &fw,
        &format!(
            "{}assertions:\n  - semihosting_contains: \"semihost hello\"\n  - firmware_exit: 0\n",
            arm_limits()
        ),
        &[],
    );
    assert_eq!(
        run.exit_code,
        Some(0),
        "expected exit 0; stdout: {}\nstderr: {}\nlog: {}",
        run.stdout,
        run.stderr,
        run.log
    );
    assert_eq!(run.status, "pass");
    assert!(run.log.contains("semihost hello"), "log: {}", run.log);
    assert_eq!(run.result["firmware_exit_code"].as_u64(), Some(0));
    assert_eq!(
        run.result["semihosting"]["observable"].as_bool(),
        Some(true)
    );
    assert!(
        run.result["semihosting"]["bytes_drained"]
            .as_u64()
            .unwrap_or(0)
            > 0
    );
    assert!(run
        .result
        .get("rtt")
        .and_then(|r| r.get("observable"))
        .is_none());
}

#[test]
fn wrong_semihost_assertion_fails() {
    let root = repo_root();
    let fw = ensure_firmware_built(&root);
    let run = run_arm(
        &root,
        &fw,
        &format!(
            "{}assertions:\n  - semihosting_contains: \"this string is never printed\"\n  - firmware_exit: 0\n",
            arm_limits()
        ),
        &[],
    );
    assert_eq!(
        run.exit_code,
        Some(1),
        "expected exit 1; stdout: {}\nstderr: {}\nlog: {}",
        run.stdout,
        run.stderr,
        run.log
    );
    assert_eq!(run.status, "fail");
    assert!(
        run.log.contains("semihost hello"),
        "capture must still hold the banner\n{}",
        run.log
    );
}

#[test]
fn flag_without_assertion_writes_the_log() {
    let root = repo_root();
    let fw = ensure_firmware_built(&root);
    let run = run_arm(
        &root,
        &fw,
        &format!("{}assertions:\n  - firmware_exit: 0\n", arm_limits()),
        &["--semihosting"],
    );
    assert_eq!(
        run.exit_code,
        Some(0),
        "stdout: {}\nstderr: {}\nlog: {}",
        run.stdout,
        run.stderr,
        run.log
    );
    assert!(run.log.contains("semihost hello"), "log: {}", run.log);
    assert_eq!(
        run.result["semihosting"]["observable"].as_bool(),
        Some(true)
    );
}

#[test]
fn assertion_without_flag_enables_capture() {
    let root = repo_root();
    let fw = ensure_firmware_built(&root);
    let run = run_arm(
        &root,
        &fw,
        &format!(
            "{}assertions:\n  - semihosting_contains: \"semihost hello\"\n  - firmware_exit: 0\n",
            arm_limits()
        ),
        &[],
    );
    assert_eq!(
        run.exit_code,
        Some(0),
        "assertion alone must enable capture; stdout: {}\nstderr: {}\nlog: {}",
        run.stdout,
        run.stderr,
        run.log
    );
    assert!(run.log.contains("semihost hello"), "log: {}", run.log);
    assert!(
        run.result["semihosting"]["bytes_drained"]
            .as_u64()
            .unwrap_or(0)
            > 0
    );
}

#[test]
fn non_cortex_m_semihosting_contains_exits_nonzero() {
    let root = repo_root();
    let fw = root.join("tests/fixtures/esp32c3-demo.elf");
    assert!(fw.exists(), "missing {}", fw.display());
    let chip = root.join("configs/chips/esp32c3.yaml");
    let out_dir = labwired_cli::test_support::unique_temp_dir("labwired-semihost-riscv");
    std::fs::create_dir_all(&out_dir).unwrap();
    let system = out_dir.join("system.yaml");
    std::fs::write(
        &system,
        format!(
            "name: \"c3-semihost-failclosed\"\nchip: \"{}\"\nexternal_devices: []\nboard_io: []\n",
            chip.display()
        ),
    )
    .unwrap();
    let script = format!(
        r#"schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
limits:
  max_steps: 20000
assertions:
  - semihosting_contains: "semihost hello"
"#,
        fw.display(),
        system.display()
    );
    let run = run_script(&root, &script, &[]);
    assert_ne!(
        run.exit_code,
        Some(0),
        "non-Cortex-M semihosting_contains must fail closed; stdout: {}\nstderr: {}\nresult: {}",
        run.stdout,
        run.stderr,
        run.result
    );
    if run.result["semihosting"].is_object() {
        assert_eq!(
            run.result["semihosting"]["observable"].as_bool(),
            Some(false)
        );
    }
    let _ = std::fs::remove_dir_all(&out_dir);
}
