// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Native Cortex-A7 routing must never fall through to Cortex-M. The GNU-built
//! fixture computes 7 + 5, stores 12 in shared RAM, then loops at PC 0x1010.
//! These tests run in both builds: with `qemu-arm` the modern routes execute;
//! without it they must report a configuration error naming the required
//! feature. Legacy interactive and chip-only routes reject A7 in either build.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FIRMWARE: &str = "tests/fixtures/cortex-a7-core-smoke.elf";
const SYSTEM: &str = "examples/cortex-a7-core/system.yaml";
const CHIP: &str = "examples/cortex-a7-core/chip.yaml";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve workspace root")
}

fn cli() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_labwired"));
    command.current_dir(workspace_root());
    command
}

fn diagnostics(output: &Output) -> String {
    format!(
        "exit={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = labwired_cli::test_support::unique_temp_dir("labwired-cortex-a-cli");
        std::fs::create_dir(&path).expect("create owned scratch directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn modern_system_run_executes_a7_or_names_the_missing_native_feature() {
    let output = cli()
        .args([
            "run",
            "--firmware",
            FIRMWARE,
            "--system",
            SYSTEM,
            "--max-steps",
            "10",
        ])
        .output()
        .expect("execute system-aware run");
    let evidence = diagnostics(&output);

    #[cfg(feature = "qemu-arm")]
    {
        assert_eq!(output.status.code(), Some(0), "{evidence}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("reached --max-steps 10"), "{evidence}");
        assert!(stderr.contains("pc=0x00001010"), "{evidence}");
    }
    #[cfg(not(feature = "qemu-arm"))]
    {
        assert_eq!(output.status.code(), Some(2), "{evidence}");
        assert!(evidence.contains("Cortex-A7"), "{evidence}");
        assert!(evidence.contains("qemu-arm"), "{evidence}");
    }
}

fn write_smoke_script(scratch: &Scratch, firmware_path: &Path) -> PathBuf {
    let root = workspace_root();
    // JSON strings are also YAML strings, including escaping spaces and
    // platform-specific separators in the absolute fixture paths.
    let firmware = serde_json::to_string(firmware_path).expect("serialize firmware path");
    let system = serde_json::to_string(&root.join(SYSTEM)).expect("serialize system path");
    let script = scratch.0.join("smoke.yaml");
    std::fs::write(
        &script,
        format!(
            r#"schema_version: "1.0"
inputs:
  firmware: {firmware}
  system: {system}
limits:
  max_steps: 10
assertions:
  - memory_value:
      address: 0x20000000
      expected_value: 12
      size: 32
"#
        ),
    )
    .expect("write deterministic memory assertion script");
    script
}

fn scripted_run(scratch: &Scratch, extra: &[&str]) -> (Output, Value) {
    scripted_run_firmware(scratch, &workspace_root().join(FIRMWARE), extra)
}

fn scripted_run_firmware(scratch: &Scratch, firmware: &Path, extra: &[&str]) -> (Output, Value) {
    let script = write_smoke_script(scratch, firmware);
    let output_dir = scratch.0.join("result");
    let output = cli()
        .args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&output_dir)
        .arg("--no-uart-stdout")
        .args(extra)
        .output()
        .expect("execute scripted run");
    let evidence = diagnostics(&output);
    let result_path = output_dir.join("result.json");
    let result: Value = serde_json::from_slice(
        &std::fs::read(&result_path)
            .unwrap_or_else(|error| panic!("read {}: {error}\n{evidence}", result_path.display())),
    )
    .unwrap_or_else(|error| panic!("parse result.json: {error}\n{evidence}"));
    (output, result)
}

#[test]
fn a7_descriptor_rejects_a_non_arm_elf_before_execution_in_both_routes() {
    let scratch = Scratch::new();
    let original = std::fs::read(workspace_root().join(FIRMWARE)).expect("read GNU ARM fixture");
    assert_eq!(&original[..4], b"\x7fELF");
    assert_eq!(
        &original[18..20],
        &40u16.to_le_bytes(),
        "fixture must be EM_ARM"
    );
    let mut mismatched = original.clone();
    mismatched[18..20].copy_from_slice(&243u16.to_le_bytes()); // EM_RISCV
    assert_eq!(&mismatched[..18], &original[..18]);
    assert_eq!(
        &mismatched[20..],
        &original[20..],
        "ARM payload stays unchanged"
    );
    let firmware = scratch.0.join("arm-payload-riscv-tag.elf");
    std::fs::write(&firmware, mismatched).expect("write owned mismatched ELF");

    let run = cli()
        .args(["run", "--firmware"])
        .arg(&firmware)
        .args(["--system", SYSTEM, "--max-steps", "10", "--json"])
        .output()
        .expect("execute mismatched system-aware run");
    let evidence = diagnostics(&run);
    assert_eq!(run.status.code(), Some(2), "{evidence}");
    let error: Value = serde_json::from_slice(&run.stdout)
        .unwrap_or_else(|parse_error| panic!("parse error response: {parse_error}\n{evidence}"));
    assert_eq!(error["error_type"], "ConfigError", "{error:#}");
    let message = error["message"].as_str().expect("ELF mismatch message");
    assert!(message.contains("Cortex-A7"), "{error:#}");
    assert!(message.contains("ARM ELF"), "{error:#}");

    let (test, result) = scripted_run_firmware(&scratch, &firmware, &[]);
    assert_eq!(test.status.code(), Some(2), "{}", diagnostics(&test));
    assert_eq!(result["status"], "error", "{result:#}");
    assert_eq!(result["stop_reason"], "config_error", "{result:#}");
    let message = result["message"].as_str().expect("ELF mismatch message");
    assert!(message.contains("Cortex-A7"), "{result:#}");
    assert!(message.contains("ARM ELF"), "{result:#}");
}

#[test]
fn scripted_a7_run_checks_the_ram_result_or_reports_feature_gating() {
    let scratch = Scratch::new();
    let (output, result) = scripted_run(&scratch, &[]);
    let evidence = diagnostics(&output);
    assert_eq!(result["limits"]["max_steps"], 10, "{result:#}\n{evidence}");
    #[cfg(feature = "qemu-arm")]
    {
        assert_eq!(output.status.code(), Some(0), "{result:#}\n{evidence}");
        assert_eq!(result["status"], "pass", "{result:#}");
        assert_eq!(result["stop_reason"], "max_steps", "{result:#}");
        assert_eq!(result["steps_executed"], 10, "{result:#}");
        let assertions = result["assertions"].as_array().expect("assertion results");
        assert_eq!(assertions.len(), 1, "{result:#}");
        assert_eq!(assertions[0]["passed"], true, "{result:#}");
    }
    #[cfg(not(feature = "qemu-arm"))]
    {
        assert_eq!(output.status.code(), Some(2), "{result:#}\n{evidence}");
        assert_eq!(result["status"], "error", "{result:#}");
        assert_eq!(result["stop_reason"], "config_error", "{result:#}");
        let message = result["message"]
            .as_str()
            .expect("configuration error message");
        assert!(message.contains("Cortex-A7"), "{result:#}");
        assert!(message.contains("qemu-arm"), "{result:#}");
    }
}

#[test]
fn scripted_a7_rom_boot_is_rejected_instead_of_running_the_elf() {
    let scratch = Scratch::new();
    let (output, result) = scripted_run(&scratch, &["--rom-boot"]);
    assert_eq!(output.status.code(), Some(2), "{}", diagnostics(&output));
    assert_eq!(result["status"], "error", "{result:#}");
    assert_eq!(result["stop_reason"], "config_error", "{result:#}");
    let message = result["message"].as_str().expect("boot rejection message");
    assert!(message.contains("Cortex-A7"), "{result:#}");
    assert!(message.contains("rom-boot"), "{result:#}");
}

#[test]
fn scripted_a7_resume_is_rejected_instead_of_silently_starting_fresh() {
    let scratch = Scratch::new();
    let missing = scratch.0.join("nonexistent.lwrs");
    assert!(!missing.exists());
    let (output, result) = scripted_run(
        &scratch,
        &[
            "--resume-snapshot",
            missing.to_str().expect("UTF-8 scratch path"),
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{}", diagnostics(&output));
    assert_eq!(result["status"], "error", "{result:#}");
    assert_eq!(result["stop_reason"], "config_error", "{result:#}");
    let message = result["message"]
        .as_str()
        .expect("snapshot rejection message");
    assert!(message.contains("Cortex-A7"), "{result:#}");
    assert!(message.contains("snapshot"), "{result:#}");
}

#[test]
fn legacy_interactive_driver_rejects_a7_before_cortex_m_execution() {
    let output = cli()
        .args([
            "--firmware",
            FIRMWARE,
            "--system",
            SYSTEM,
            "--max-steps",
            "10",
            "--json",
        ])
        .output()
        .expect("execute legacy interactive route");
    let evidence = diagnostics(&output);
    assert_eq!(output.status.code(), Some(2), "{evidence}");
    let error: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|parse_error| panic!("parse error response: {parse_error}\n{evidence}"));
    assert_eq!(error["error_type"], "ConfigError", "{error:#}");
    let message = error["message"].as_str().expect("error message");
    assert!(message.contains("Cortex-A7"), "{error:#}");
    assert!(message.contains("run --firmware"), "{error:#}");
    assert!(message.contains("--system"), "{error:#}");
}

#[test]
fn chip_only_run_rejects_a7_and_points_to_the_system_route() {
    let output = cli()
        .args([
            "run",
            "--chip",
            CHIP,
            "--firmware",
            FIRMWARE,
            "--max-steps",
            "10",
        ])
        .output()
        .expect("execute chip-only route");
    let evidence = diagnostics(&output);
    assert_eq!(output.status.code(), Some(2), "{evidence}");
    assert!(evidence.contains("Cortex-A7"), "{evidence}");
    assert!(evidence.contains("--system"), "{evidence}");
}
