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

fn run_from_repo_root(script_rel: &str) -> std::process::Output {
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
    assert!(out_dir.join("result.json").exists());
    assert!(out_dir.join("uart.log").exists());
    assert!(out_dir.join("junit.xml").exists());

    let _ = std::fs::remove_dir_all(&out_dir);
    output
}

#[test]
fn golden_dummy_max_steps_runs_from_repo_root() {
    let output = run_from_repo_root("examples/ci/dummy-max-steps.yaml");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn golden_dummy_wall_time_runs_from_repo_root() {
    let output = run_from_repo_root("examples/ci/dummy-wall-time.yaml");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn golden_dummy_max_cycles_runs_from_repo_root() {
    let output = run_from_repo_root("examples/ci/dummy-max-cycles.yaml");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn golden_dummy_fail_uart_runs_from_repo_root() {
    let output = run_from_repo_root("examples/ci/dummy-fail-uart.yaml");
    assert_eq!(output.status.code(), Some(1));
}
