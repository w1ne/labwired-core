// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// End-to-end coverage for `labwired run --system --stimulus`: guard rails,
// fail-fast validation with a machine-readable channel inventory, and the
// KW41Z cow-activity happy path (the command-line twin of the
// examples/kw41z-cow-activity/stimulus-shake.yaml test-script run).

use std::path::PathBuf;
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(repo_root())
        .args(args)
        .output()
        .expect("execute labwired")
}

const KW41Z_SYSTEM: &str = "configs/systems/frdm-kw41z-lcd.yaml";
const KW41Z_FIRMWARE: &str = "tests/fixtures/kw41z-lcd-activity.elf";
const SHAKE: &str = r#"{"component":"fxos8700","channel":"x","value":2.0,"after_cycles":3000000}"#;

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn stimulus_requires_a_system_manifest() {
    let out = run_cli(&[
        "run",
        "--chip",
        "configs/chips/stm32f103.yaml",
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "10",
        "--stimulus",
        SHAKE,
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(stderr_of(&out).contains("--system"), "{}", stderr_of(&out));
}

#[test]
fn system_run_requires_an_explicit_step_budget() {
    let out = run_cli(&[
        "run",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(
        stderr_of(&out).contains("--max-steps"),
        "{}",
        stderr_of(&out)
    );
}

#[test]
fn gpio_trace_is_rejected_on_the_system_driver() {
    let out = run_cli(&[
        "run",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "10",
        "--gpio-trace",
        "/tmp/labwired-gpio-should-not-exist.jsonl",
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(
        stderr_of(&out).contains("gpio-trace"),
        "{}",
        stderr_of(&out)
    );
}

#[test]
fn unsupported_arch_is_rejected() {
    let out = run_cli(&[
        "run",
        "--system",
        "configs/systems/esp32-wroom-32.yaml",
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "10",
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(stderr_of(&out).contains("Xtensa"), "{}", stderr_of(&out));
}

#[test]
fn unknown_channel_fails_fast_with_the_available_inventory() {
    let out = run_cli(&[
        "run",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "10",
        "--json",
        "--stimulus",
        r#"{"channel":"nope","value":1.0}"#,
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout_of(&out)).expect("--json error payload on stdout");
    assert_eq!(parsed["exit_code"], 2);
    let available = parsed["details"]["available"]
        .as_array()
        .expect("available inventory");
    assert!(
        available
            .iter()
            .any(|e| e["component"] == "fxos8700" && e["channel"] == "x"),
        "{}",
        stdout_of(&out)
    );
}

#[test]
fn out_of_range_value_fails_fast() {
    let out = run_cli(&[
        "run",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "10",
        "--stimulus",
        r#"{"component":"fxos8700","channel":"x","value":99.0}"#,
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(stderr_of(&out).contains("value"), "{}", stderr_of(&out));
}

#[test]
fn kw41z_cow_reacts_to_a_command_line_stimulus() {
    let started = std::time::Instant::now();
    let out = run_cli(&[
        "run",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "6000000",
        "--stimulus",
        SHAKE,
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(
        stderr_of(&out).contains("stimulus: x = 2"),
        "expected the applied-stimulus log\n{}",
        stderr_of(&out)
    );
    let uart = stdout_of(&out);
    assert!(uart.contains("MOOD=CALM"), "{uart}");
    assert!(uart.contains("MOOD=ACTIVE"), "{uart}");
    let first_calm = uart.find("MOOD=CALM").unwrap();
    let first_active = uart.find("MOOD=ACTIVE").unwrap();
    assert!(first_calm < first_active, "CALM must precede ACTIVE");
    eprintln!(
        "kw41z command-line stimulus run took {:?}",
        started.elapsed()
    );
}

#[test]
fn chip_only_debug_flags_are_rejected_on_the_system_driver() {
    for (flag, value) in [
        ("--rom-boot", None),
        ("--break-at", Some("0x2000")),
        ("--watch-mem", Some("0x20000000")),
    ] {
        let mut args = vec![
            "run",
            "--system",
            KW41Z_SYSTEM,
            "--firmware",
            KW41Z_FIRMWARE,
            "--max-steps",
            "10",
            flag,
        ];
        if let Some(value) = value {
            args.push(value);
        }
        let out = run_cli(&args);
        assert_eq!(out.status.code(), Some(2), "{flag}: {}", stderr_of(&out));
        assert!(
            stderr_of(&out).contains(flag),
            "{flag} should be named in the error: {}",
            stderr_of(&out)
        );
    }
}

#[test]
fn stimulus_is_rejected_on_chip_paths() {
    let out = run_cli(&[
        "run",
        "--chip",
        "configs/chips/mkw41z4.yaml",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "10",
        "--stimulus",
        SHAKE,
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(
        stderr_of(&out).contains("--stimulus") && stderr_of(&out).contains("--system"),
        "{}",
        stderr_of(&out)
    );
}

#[test]
fn at_start_stimulus_applies_before_the_loop() {
    let out = run_cli(&[
        "run",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "200",
        "--stimulus",
        r#"{"component":"fxos8700","channel":"x","value":2.0}"#,
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(
        stderr_of(&out).contains("stimulus: x = 2"),
        "{}",
        stderr_of(&out)
    );
}

#[test]
fn unfired_stimulus_is_reported_at_end_of_run() {
    let out = run_cli(&[
        "run",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "100",
        "--stimulus",
        r#"{"component":"fxos8700","channel":"x","value":2.0,"after_cycles":1000000}"#,
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(
        stderr_of(&out).contains("never fired"),
        "{}",
        stderr_of(&out)
    );
    assert!(
        stderr_of(&out).contains("stimulus[0]"),
        "{}",
        stderr_of(&out)
    );
}
