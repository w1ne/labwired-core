// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("Failed to canonicalize repo root")
}

fn temp_artifacts_dir(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("labwired-golden-{}-{}", prefix, nonce));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

struct GoldenRun {
    output: std::process::Output,
    result: serde_json::Value,
    uart_log: String,
}

fn run_from_repo_root(script_rel: &str) -> GoldenRun {
    let root = repo_root();
    let out_dir = temp_artifacts_dir(
        PathBuf::from(script_rel)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("script"),
    );

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
        .expect("Failed to execute labwired");

    // Artifacts should be produced regardless of pass/fail when --output-dir is set.
    let result_path = out_dir.join("result.json");
    assert!(
        result_path.exists(),
        "result.json not produced for script {script_rel}"
    );
    assert!(
        out_dir.join("uart.log").exists(),
        "uart.log not produced for script {script_rel}"
    );
    assert!(
        out_dir.join("junit.xml").exists(),
        "junit.xml not produced for script {script_rel}"
    );

    let result_json = std::fs::read_to_string(&result_path).expect("read result.json");
    let result: serde_json::Value = serde_json::from_str(&result_json).expect("parse result.json");

    let uart_log = std::fs::read_to_string(out_dir.join("uart.log")).unwrap_or_default();

    let _ = std::fs::remove_dir_all(&out_dir);
    GoldenRun {
        output,
        result,
        uart_log,
    }
}

#[test]
fn golden_dummy_max_steps_runs_from_repo_root() {
    let run = run_from_repo_root("examples/ci/dummy-max-steps.yaml");
    assert_eq!(
        run.output.status.code(),
        Some(0),
        "expected exit 0; stderr: {}",
        String::from_utf8_lossy(&run.output.stderr)
    );
    assert_eq!(
        run.result["status"], "pass",
        "expected status=pass in result.json"
    );
    assert_eq!(
        run.result["stop_reason"], "max_steps",
        "expected stop_reason=max_steps"
    );
    assert!(
        run.result["steps_executed"].as_u64().unwrap_or(0) > 0,
        "expected steps_executed > 0"
    );
    let firmware_hash = run.result["firmware_hash"].as_str().unwrap_or("");
    assert!(
        !firmware_hash.is_empty(),
        "firmware_hash should be non-empty in result.json"
    );
}

#[test]
fn golden_dummy_wall_time_runs_from_repo_root() {
    let run = run_from_repo_root("examples/ci/dummy-wall-time.yaml");
    assert_eq!(
        run.output.status.code(),
        Some(0),
        "expected exit 0; stderr: {}",
        String::from_utf8_lossy(&run.output.stderr)
    );
    assert_eq!(run.result["status"], "pass");
    assert_eq!(run.result["stop_reason"], "wall_time");
}

#[test]
fn golden_dummy_max_cycles_runs_from_repo_root() {
    let run = run_from_repo_root("examples/ci/dummy-max-cycles.yaml");
    assert_eq!(
        run.output.status.code(),
        Some(0),
        "expected exit 0; stderr: {}",
        String::from_utf8_lossy(&run.output.stderr)
    );
    assert_eq!(run.result["status"], "pass");
    assert_eq!(run.result["stop_reason"], "max_cycles");
}

#[test]
fn golden_dummy_fail_uart_runs_from_repo_root() {
    let run = run_from_repo_root("examples/ci/dummy-fail-uart.yaml");
    assert_eq!(
        run.output.status.code(),
        Some(1),
        "expected exit 1 (assertion failure); stderr: {}",
        String::from_utf8_lossy(&run.output.stderr)
    );
    assert_eq!(
        run.result["status"], "fail",
        "expected status=fail in result.json"
    );
    // The fail case asserts uart_contains "ThisTextWillNeverBeFound" — verify the
    // failing assertion is recorded.
    let assertions = run.result["assertions"]
        .as_array()
        .expect("assertions array");
    assert!(
        !assertions.is_empty(),
        "assertions array should not be empty for fail case"
    );
    let first = &assertions[0];
    assert_eq!(
        first["passed"], false,
        "first assertion should be marked failed"
    );
    assert!(
        first["assertion"]["uart_contains"].as_str().is_some(),
        "failing assertion should record uart_contains key"
    );
    // Verify the uart log is present and note its content (it should be small or empty
    // since only 10 steps run before hitting max_steps).
    let _ = run.uart_log; // accessible for debugging if this test ever re-asserts content
}
