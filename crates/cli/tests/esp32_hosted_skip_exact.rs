// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `labwired test` on a classic ESP32 skips idle and CCOUNT busy-wait cycles,
//! and the skip changes nothing a run reports except bookkeeping.
//!
//! The fixture (`fixtures/esp32_ccount_blink`) drives GPIO2 50 times on a
//! 10 000-cycle CCOUNT busy-wait, then sits in WAITI: the two shapes of wait an
//! Arduino-ESP32 sketch spends its time in. Each case runs twice, with the
//! skip (the default) and with `LABWIRED_IDLE_FAST_FORWARD=0`, and the
//! result.json files must be equal except two counters that count retired work:
//! `metrics.pc_samples` (sampled per retired instruction) and
//! `metrics.peripheral_accesses` (skipped polls access nothing).
//!
//! On this path a step is a machine cycle, retired or skipped, so every advance
//! call, assertion check and settle window ends on the same cycle with the skip
//! on or off. Counting only retired instructions stopped an early-stop run
//! ~0.9 M cycles later with the skip on (four-servo lab, d26Wk2eXNiFS).

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

/// Run the fixture under `limits` (YAML lines below `limits:`, plus any
/// top-level blocks after them); returns (result.json, stderr).
fn run(name: &str, skip: bool, limits: &str) -> (serde_json::Value, String) {
    let root = workspace_root();
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-esp32-skip-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\n  system: \"{}\"\nlimits:\n{limits}",
            root.join("crates/cli/tests/fixtures/esp32_ccount_blink/blink.elf")
                .display(),
            root.join("configs/systems/esp32-wroom-32.yaml").display(),
        ),
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_labwired"));
    cmd.args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&dir)
        .args(["--no-uart-stdout", "--watch-gpio", "gpio:2"]);
    if skip {
        cmd.env_remove("LABWIRED_IDLE_FAST_FORWARD");
    } else {
        cmd.env("LABWIRED_IDLE_FAST_FORWARD", "0");
    }
    let output = cmd.output().expect("run labwired");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let result = std::fs::read_to_string(dir.join("result.json"))
        .unwrap_or_else(|_| panic!("no result.json; stderr:\n{stderr}"));
    (serde_json::from_str(&result).unwrap(), stderr)
}

/// Cycles the skip covered, from the run's own summary line.
fn skipped(stderr: &str) -> u64 {
    stderr
        .lines()
        .find_map(|l| l.split("idle_ff skipped ").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn without_bookkeeping(mut result: serde_json::Value) -> serde_json::Value {
    // Each run writes its script in its own temp dir.
    result["config"]
        .as_object_mut()
        .expect("config")
        .remove("script");
    let metrics = result["metrics"].as_object_mut().expect("metrics");
    metrics.remove("pc_samples");
    metrics.remove("peripheral_accesses");
    result
}

fn transitions(result: &serde_json::Value) -> usize {
    result["logic_edges"]["channels"][0]["transitions"]
        .as_array()
        .map_or(0, Vec::len)
}

fn check(name: &str, limits: &str) -> serde_json::Value {
    let (fast, fast_err) = run(name, true, limits);
    let (reference, _) = run(&format!("{name}-ref"), false, limits);
    assert!(skipped(&fast_err) > 0, "the skip did not fire:\n{fast_err}");
    assert_eq!(
        without_bookkeeping(fast.clone()),
        without_bookkeeping(reference),
        "the skip changed what the run reports"
    );
    fast
}

#[test]
fn a_skipped_busy_wait_and_idle_report_the_same_run() {
    let (_, err) = run(
        "count",
        true,
        "  max_steps: 10000000\n  max_cycles: 1000000\n",
    );
    // 50 periods of busy-wait (~500 k cycles) and the WAITI tail (~500 k).
    assert!(
        skipped(&err) > 900_000,
        "skipped {} of 1 M cycles:\n{err}",
        skipped(&err)
    );
    let fast = check("plain", "  max_steps: 10000000\n  max_cycles: 1000000\n");
    assert_eq!(transitions(&fast), 50);
    assert_eq!(fast["status"], "pass");
}

#[test]
fn an_early_stop_lands_on_the_same_cycle_with_the_skip() {
    let fast = check(
        "early-stop",
        "  max_steps: 10000000\n  max_cycles: 1000000\n  stop_when_assertions_pass: true\n  \
         stop_when_assertions_pass_settle_steps: 1000\n  stop_when_assertions_pass_min_steps: 1000\n\
         assertions:\n  - gpio_edges:\n      pin: \"gpio:2\"\n      min_edges: 30\n",
    );
    assert_eq!(fast["stop_reason"], "assertions_passed");
    assert!(transitions(&fast) >= 30 && transitions(&fast) < 50);
}
