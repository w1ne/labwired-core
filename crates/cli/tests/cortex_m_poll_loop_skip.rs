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
//!
//! `fixtures/cortex_m_idle_hook` is the same wait as an RTOS idle loop: each
//! iteration calls a hook that pushes to the stack and enters and leaves a
//! critical section through a RAM nesting counter, storing the bytes memory
//! already holds. Its negative control bumps a RAM idle counter through the
//! same hook (memory changes every iteration).

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

/// Bounded by device time: both runs end on the same cycle by construction.
const CYCLE_BOUND: &str = "  max_steps: 10000000\n  max_cycles: 400000\n";

fn run(name: &str, skip: bool) -> (serde_json::Value, String) {
    run_fixture("cortex_m_tick_wait/tick_wait.elf", name, skip)
}

fn run_fixture(elf: &str, name: &str, skip: bool) -> (serde_json::Value, String) {
    run_limited(elf, name, skip, CYCLE_BOUND)
}

fn run_limited(elf: &str, name: &str, skip: bool, limits: &str) -> (serde_json::Value, String) {
    let root = workspace_root();
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-cm-poll-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\n  system: \"{}\"\nlimits:\n{limits}",
            root.join("crates/cli/tests/fixtures").join(elf).display(),
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

#[test]
fn a_skipped_idle_hook_loop_that_stores_reports_the_same_run() {
    let elf = "cortex_m_idle_hook/idle_hook.elf";
    let (fast, fast_err) = run_fixture(elf, "idle-skip", true);
    let (reference, _) = run_fixture(elf, "idle-ref", false);
    let edges = fast["logic_edges"]["channels"][0]["transitions"]
        .as_array()
        .map_or(0, Vec::len);
    assert_eq!(edges, 101, "100 waits and the post-counter toggle");
    assert!(
        skipped(&fast_err) > 300_000,
        "the idle-hook waits and `b .` were not skipped:\n{fast_err}"
    );
    assert_eq!(
        observable(fast),
        observable(reference),
        "the skip changed the run"
    );
}

/// A run bounded by `max_steps` alone ends on the same cycle, with the same
/// edges, whether the waits are skipped or stepped. Each skipped poll-loop
/// instruction is one step, counted once: the skip used to be charged twice
/// (in `primary_steps` and again from `poll_loop_cycles_skipped`), so the same
/// budget stopped a skipped run at about half the device time.
#[test]
fn a_step_budget_buys_the_same_device_time_with_the_skip() {
    let elf = "cortex_m_tick_wait/tick_wait.elf";
    let limits = "  max_steps: 300000\n";
    let (fast, fast_err) = run_limited(elf, "steps-skip", true, limits);
    let (reference, _) = run_limited(elf, "steps-ref", false, limits);
    assert!(
        skipped(&fast_err) > 200_000,
        "the waits were not skipped:\n{fast_err}"
    );
    assert_eq!(fast["steps_executed"], 300_000);
    assert_eq!(
        fast["cycles"], reference["cycles"],
        "the same step budget bought different device time"
    );
    assert_eq!(
        observable(fast),
        observable(reference),
        "the skip changed the run"
    );
}
