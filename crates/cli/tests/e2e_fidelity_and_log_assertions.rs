// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `fidelity_clean` and `peripheral_log` assertions through `labwired test`.
//!
//! Two runs of the same firmware. On the CI fixture system the firmware
//! writes "OK" to `uart1`, which records each byte in the bus trace, and hits
//! no fidelity gap. On a chip with no peripherals and a 1-byte flash the same
//! firmware hits unmapped MMIO. Each assertion has a case that must pass and a
//! case that must fail, so a check that always passes cannot hide here.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FIRMWARE: &str = "../../tests/fixtures/uart-ok-thumbv7m.elf";
const CLEAN_SYSTEM: &str = "../../configs/systems/ci-fixture-uart1.yaml";

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("labwired-tests")
        .join(labwired_cli::test_support::unique_name(name));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A system whose chip maps no peripheral and only 1 byte of flash.
fn gap_system(dir: &Path) -> PathBuf {
    std::fs::write(
        dir.join("chip.yaml"),
        "name: \"tiny\"\narch: \"cortex-m3\"\nflash:\n  base: 0x0\n  size: \"1B\"\n\
         ram:\n  base: 0x20000000\n  size: \"1KB\"\nperipherals: []\n",
    )
    .unwrap();
    let system = dir.join("system.yaml");
    std::fs::write(&system, "name: \"tiny-system\"\nchip: \"chip.yaml\"\n").unwrap();
    system
}

/// Run `labwired test` with `assertions` (a YAML list body) against `system`.
fn run(dir: &Path, system: &Path, assertions: &str, strict: bool) -> Output {
    let firmware = std::fs::canonicalize(FIRMWARE).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\nlimits:\n  max_steps: 1000\n\
             assertions:\n{assertions}",
            firmware.display()
        ),
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_labwired"));
    cmd.args(["test", "--system"])
        .arg(system)
        .arg("--script")
        .arg(&script)
        .args(["--no-uart-stdout", "--output-dir"])
        .arg(dir.join("out"));
    if strict {
        cmd.env("LABWIRED_STRICT_FIDELITY", "1");
    } else {
        cmd.env_remove("LABWIRED_STRICT_FIDELITY");
    }
    cmd.output().unwrap()
}

fn result(dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(dir.join("out/result.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn clean_system() -> PathBuf {
    std::fs::canonicalize(CLEAN_SYSTEM).unwrap()
}

#[test]
fn fidelity_clean_true_passes_a_run_without_gaps() {
    let dir = work_dir("fidelity-clean-pass");
    let out = run(&dir, &clean_system(), "  - fidelity_clean: true\n", false);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(result(&dir)["status"], "pass");
}

#[test]
fn fidelity_clean_true_fails_a_run_with_unmapped_mmio() {
    let dir = work_dir("fidelity-clean-fail");
    let out = run(&dir, &gap_system(&dir), "  - fidelity_clean: true\n", false);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let r = result(&dir);
    assert_eq!(r["status"], "fail");
    assert_eq!(r["assertions"][0]["passed"], false);
    assert!(
        stderr(&out).contains("fidelity_clean: true, but fidelity:"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn fidelity_clean_false_passes_only_when_a_gap_happened() {
    let dir = work_dir("fidelity-dirty-pass");
    let out = run(
        &dir,
        &gap_system(&dir),
        "  - fidelity_clean: false\n  - expected_stop_reason: memory_violation\n",
        false,
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let dir = work_dir("fidelity-dirty-fail");
    let out = run(&dir, &clean_system(), "  - fidelity_clean: false\n", false);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("the run hit no fidelity gap"),
        "{}",
        stderr(&out)
    );
}

/// `LABWIRED_STRICT_FIDELITY` stops the process at the first gap. It aborts:
/// no result.json, a non-zero exit, and the gap in the panic message.
#[test]
fn strict_fidelity_env_fails_a_run_with_a_gap() {
    let dir = work_dir("fidelity-strict-gap");
    let out = run(&dir, &gap_system(&dir), "  - fidelity_clean: true\n", true);
    assert!(!out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("LABWIRED_STRICT_FIDELITY: unmapped MMIO"),
        "{}",
        stderr(&out)
    );
    assert!(!dir.join("out/result.json").exists());

    // Negative control: the same mode lets a clean run pass.
    let dir = work_dir("fidelity-strict-clean");
    let out = run(&dir, &clean_system(), "  - fidelity_clean: true\n", true);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn peripheral_log_matches_bus_trace_lines() {
    let dir = work_dir("peripheral-log-pass");
    let out = run(
        &dir,
        &clean_system(),
        "  - peripheral_log: {peripheral: uart1, log: bus_trace, contains: \"tx 0x4f\"}\n\
         \x20 - peripheral_log: {peripheral: uart1, log: bus_trace, contains: \"tx 0x\", min_count: 2}\n",
        false,
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let r = result(&dir);
    assert_eq!(r["assertions"][0]["passed"], true);
    assert_eq!(r["assertions"][1]["passed"], true);
}

#[test]
fn peripheral_log_fails_when_no_line_matches() {
    let dir = work_dir("peripheral-log-fail");
    let out = run(
        &dir,
        &clean_system(),
        "  - peripheral_log: {peripheral: uart1, log: bus_trace, contains: \"tx 0x5a\"}\n\
         \x20 - peripheral_log: {peripheral: uart1, log: bus_trace, contains: \"tx 0x4f\", min_count: 100}\n",
        false,
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let r = result(&dir);
    assert_eq!(r["status"], "fail");
    assert_eq!(r["assertions"][0]["passed"], false);
    assert_eq!(r["assertions"][1]["passed"], false);
    assert!(
        stderr(&out).contains("uart1.bus_trace has 0 line(s) with \"tx 0x5a\", need 1"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn peripheral_log_unknown_names_are_config_errors() {
    let dir = work_dir("peripheral-log-no-log");
    let out = run(
        &dir,
        &clean_system(),
        "  - peripheral_log: {peripheral: uart1, log: host, contains: \"x\"}\n",
        false,
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("peripheral 'uart1' has no log 'host'. Its logs: bus_trace"),
        "{}",
        stderr(&out)
    );

    let dir = work_dir("peripheral-log-no-peripheral");
    let out = run(
        &dir,
        &clean_system(),
        "  - peripheral_log: {peripheral: usb9, log: bus_trace, contains: \"x\"}\n",
        false,
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("no peripheral or attached device named 'usb9'"),
        "{}",
        stderr(&out)
    );
}
