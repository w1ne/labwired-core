// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The Cortex-M fixed-point poll-loop skip changes no observable result.
//!
//! `fixtures/cortex_m_tick_wait` waits for a SysTick-incremented RAM word 100
//! times (the shape of `HAL_Delay` / Arduino `delay()`), toggling PA5 after
//! each wait, then runs a 20000-iteration register countdown (not a fixed
//! point), toggles once more and parks in `b .`. The run with the skip must
//! report exactly what the run without any skip (`LABWIRED_IDLE_FAST_FORWARD=0`)
//! reports, edges and final state included, while skipping most cycles. The
//! countdown is the negative control's target: a skip that ignored the
//! changing register would move the last edge.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run(name: &str, skip: bool) -> (serde_json::Value, String) {
    let root = workspace_root();
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-cm-poll-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\n  system: \"{}\"\nlimits:\n  max_steps: 10000000\n  max_cycles: 400000\n",
            root.join("crates/cli/tests/fixtures/cortex_m_tick_wait/tick_wait.elf").display(),
            root.join("configs/systems/nucleo-l476rg.yaml").display(),
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&dir)
        .args(["--no-uart-stdout", "--watch-gpio", "gpioa:5"])
        .env("LABWIRED_IDLE_FAST_FORWARD", if skip { "1" } else { "0" })
        .output()
        .expect("run labwired");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let result = std::fs::read_to_string(dir.join("result.json"))
        .unwrap_or_else(|_| panic!("no result.json; stderr:\n{stderr}"));
    (serde_json::from_str(&result).unwrap(), stderr)
}

fn skipped(stderr: &str) -> u64 {
    stderr
        .lines()
        .find_map(|l| l.split("idle_ff skipped ").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn observable(mut r: serde_json::Value) -> serde_json::Value {
    let o = r.as_object_mut().unwrap();
    o.get_mut("config")
        .and_then(|c| c.as_object_mut())
        .map(|c| c.remove("script"));
    // Counters of interpreted work (sampled per retired instruction, and the
    // memory and device accesses the skipped iterations would have made).
    let m = o
        .get_mut("metrics")
        .and_then(|m| m.as_object_mut())
        .unwrap();
    m.remove("pc_samples");
    m.remove("peripheral_accesses");
    m.remove("memory_reads");
    m.remove("memory_writes");
    r
}

#[test]
fn a_skipped_tick_wait_reports_the_same_run() {
    let (fast, fast_err) = run("skip", true);
    let (reference, _) = run("ref", false);
    let edges = fast["logic_edges"]["channels"][0]["transitions"]
        .as_array()
        .map_or(0, Vec::len);
    assert_eq!(edges, 101, "100 waits and the post-countdown toggle");
    assert!(
        skipped(&fast_err) > 300_000,
        "the waits and `b .` were not skipped:\n{fast_err}"
    );
    assert_eq!(
        observable(fast),
        observable(reference),
        "the skip changed the run"
    );
}
