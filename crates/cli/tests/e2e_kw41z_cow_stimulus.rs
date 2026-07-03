// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// End-to-end coverage for declarative input stimuli (test-script schema 1.2)
// via the examples/kw41z-cow-activity demo.
//
// The FRDM-KW41Z + FXOS8700 + Nokia-5110 "cow" firmware reads the accelerometer
// live and prints `MOOD=CALM|ACTIVE`. These tests run the committed ELF through
// the committed scenario scripts with `labwired test`, exercising the exact
// path an author (or an agent over MCP) runs:
//
//   • calm.yaml           — no stimulus; the cow grazes (CALM) the whole run.
//   • stimulus-shake.yaml — an `after_cycles` stimulus drives X to +2 g partway
//                           through, flipping the cow to ACTIVE.
//
// This is what keeps the scriptable-input path honest: a regression in the
// generic `Machine::set_input` dispatch, the FXOS8700 `SimInput` impl, the
// cow firmware, or the schema-1.2 runner wiring fails the merge gate here.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

struct CowRun {
    exit_code: Option<i32>,
    stderr: String,
    uart: String,
    status: String,
}

/// Run one cow scenario script (relative to the repo root) through the
/// `labwired test` CLI and collect its result + UART log.
fn run_cow(script_rel: &str) -> CowRun {
    let root = repo_root();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out_dir = std::env::temp_dir().join(format!("labwired-cow-{nonce}"));
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .args([
            "test",
            "--script",
            script_rel,
            "--no-uart-stdout",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute labwired");

    let uart = std::fs::read_to_string(out_dir.join("uart.log")).unwrap_or_default();
    let result_json =
        std::fs::read_to_string(out_dir.join("result.json")).expect("read result.json");
    let result: serde_json::Value = serde_json::from_str(&result_json).expect("parse result.json");
    let status = result["status"].as_str().unwrap_or("").to_string();

    let _ = std::fs::remove_dir_all(&out_dir);
    CowRun {
        exit_code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        uart,
        status,
    }
}

#[test]
fn cow_grazes_calm_without_a_stimulus() {
    let run = run_cow("examples/kw41z-cow-activity/calm.yaml");
    assert_eq!(
        run.exit_code,
        Some(0),
        "expected exit 0 (assertions pass); stderr: {}",
        run.stderr
    );
    assert_eq!(run.status, "pass", "expected result.json status=pass");

    assert!(
        run.uart.contains("MOOD=CALM"),
        "baseline should report CALM\n--- uart ---\n{}",
        run.uart
    );
    // The control case: with nothing driving the accelerometer the cow never
    // pops its head up. If this starts printing ACTIVE, the idle pose drifted.
    assert!(
        !run.uart.contains("MOOD=ACTIVE"),
        "baseline (no stimulus) must never go ACTIVE\n--- uart ---\n{}",
        run.uart
    );
}

#[test]
fn stimulus_flips_the_cow_active_mid_run() {
    let run = run_cow("examples/kw41z-cow-activity/stimulus-shake.yaml");
    assert_eq!(
        run.exit_code,
        Some(0),
        "expected exit 0 (both MOOD assertions pass); stderr: {}",
        run.stderr
    );
    assert_eq!(run.status, "pass", "expected result.json status=pass");

    // The stimulus was actually applied (logged by the runner), not silently
    // swallowed by a bad channel name.
    assert!(
        run.stderr.contains("stimulus: x = 2"),
        "expected the runner to log the applied stimulus\n--- stderr ---\n{}",
        run.stderr
    );

    // The headline: grazing before the shake, head up after it.
    assert!(
        run.uart.contains("MOOD=CALM"),
        "expected CALM before the stimulus\n--- uart ---\n{}",
        run.uart
    );
    assert!(
        run.uart.contains("MOOD=ACTIVE"),
        "expected ACTIVE after the X=+2 g stimulus\n--- uart ---\n{}",
        run.uart
    );

    // Ordering: the first CALM precedes the first ACTIVE — the cow started calm
    // and the stimulus is what drove it active, not the reverse.
    let first_calm = run.uart.find("MOOD=CALM").unwrap();
    let first_active = run.uart.find("MOOD=ACTIVE").unwrap();
    assert!(
        first_calm < first_active,
        "CALM should appear before ACTIVE (stimulus drove the transition)"
    );
}
