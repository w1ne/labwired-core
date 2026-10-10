// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired test --heartbeat-file`: the liveness beacon a supervisor reads to
//! tell a run that is advancing from one that is hung (hosted MCP audit #8).

use std::process::Command;

fn run_with(heartbeat: Option<&std::path::Path>) -> std::process::Output {
    let fw = std::fs::canonicalize("../../tests/fixtures/uart-ok-thumbv7m.elf").unwrap();
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-tests-heartbeat");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\nlimits:\n  max_steps: 1000\nassertions: []\n",
            fw.display()
        ),
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_labwired"));
    cmd.args([
        "test",
        "--script",
        script.to_str().unwrap(),
        "--no-uart-stdout",
        "--output-dir",
    ])
    .arg(dir.join("out"));
    if let Some(p) = heartbeat {
        cmd.arg("--heartbeat-file").arg(p);
    }
    cmd.output().unwrap()
}

#[test]
fn heartbeat_file_is_written_once_the_loop_starts() {
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-tests-heartbeat-file");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let beat = dir.join("beat.json");
    let out = run_with(Some(&beat));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&beat).unwrap()).unwrap();
    assert!(v["steps"].is_u64() && v["cycles"].is_u64(), "{v}");
    assert!(
        !beat.with_extension("tmp").exists(),
        "temp file must be renamed away"
    );
}
