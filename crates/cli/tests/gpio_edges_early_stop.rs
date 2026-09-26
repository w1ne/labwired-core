// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! HARD GATE for the `gpio_edges` assertion early-stop — the acceptance shape a
//! hosted ESP32-S3 prove lowers to.
//!
//! On an S3 the serial is not captured, so `gpio_edges` is the only observable
//! channel and it IS the whole oracle. Without a stop condition the run is
//! decided only after the full step budget: the hosted dual-LED prove satisfied
//! its oracle at 86M cycles of a 200M budget and still executed the remaining
//! 114M (~2.3x wall clock) for a verdict already known.
//!
//! This drives the REAL `labwired` binary on the committed TIER1 S3 flash image
//! with a `gpio_edges` assertion on `gpio:4` (the pad the TIER1 gpio test
//! toggles) and asserts:
//!   1. the run stops with `assertions_passed` — NOT `max_steps`;
//!   2. far short of the budget (the edges land around 17M of 200M);
//!   3. the pin is auto-armed: no `--watch-gpio` flag is passed, yet
//!      `logic_edges` carries the channel the assertion named.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Repo root = crates/cli/../.. (matches the other CLI integration tests).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn require(path: PathBuf) -> PathBuf {
    assert!(path.exists(), "missing fixture: {}", path.display());
    path
}

/// Budget far above where the evidence lands: the whole point is that the run
/// must NOT need it. 200M mirrors the hosted S3 budget.
const S3_MAX_STEPS: u64 = 200_000_000;

#[test]
fn gpio_edges_assertion_stops_the_run_on_observed_edges() {
    let root = repo_root();
    let flash = require(root.join("tests/fixtures/tier1/esp32s3-flash.bin"));
    let system = require(root.join("configs/systems/esp32s3-zero.yaml"));

    let tmp = std::env::temp_dir().join(format!("lw-gpio-stop-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create tmp dir");
    let script = tmp.join("gpio_edges_stop.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\n\
             inputs:\n  \
               firmware: \"\"\n  \
               system: \"{}\"\n\
             limits:\n  \
               max_steps: {S3_MAX_STEPS}\n  \
               stop_when_assertions_pass: true\n  \
               stop_when_assertions_pass_settle_steps: 100000\n  \
               stop_when_assertions_pass_min_steps: 0\n\
             assertions:\n  \
               - gpio_edges:\n      \
                   pin: \"gpio:4\"\n      \
                   min_edges: 2\n",
            system.display(),
        ),
    )
    .expect("write test script");
    let out_dir = tmp.join("out");
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    // NO --watch-gpio: the assertion must arm its own channel.
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .env("LABWIRED_ESP32S3_FLASH", &flash)
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--rom-boot",
            "--no-uart-stdout",
            "--no-key",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn labwired");

    assert!(
        output.status.success(),
        "run failed (exit {:?}); stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );

    let result_path = out_dir.join("result.json");
    let result_json = std::fs::read_to_string(&result_path)
        .unwrap_or_else(|e| panic!("no result.json at {}: {e}", result_path.display()));
    let parsed: serde_json::Value = serde_json::from_str(&result_json).expect("result.json parses");

    assert_eq!(
        parsed["stop_reason"], "assertions_passed",
        "expected the gpio assertion to stop the run, got:\n{result_json}",
    );
    let steps = parsed["steps_executed"].as_u64().unwrap_or(u64::MAX);
    assert!(
        steps < 30_000_000,
        "expected the early stop far below the {S3_MAX_STEPS} budget, ran {steps} steps",
    );
    // The settle window is 100k steps past the last needed edge; a run that
    // stopped at the very first batch would mean the assertion was satisfied
    // by something other than a real transition.
    assert!(
        steps > 100_000,
        "stopped at {steps} steps — the settle window was skipped",
    );

    // The assertion named `gpio:4`, so that channel must have been armed and
    // captured without any --watch-gpio flag.
    let channels = parsed["logic_edges"]["channels"]
        .as_array()
        .unwrap_or_else(|| panic!("no logic_edges channels in:\n{result_json}"));
    let edges: usize = channels
        .iter()
        .map(|c| c["transitions"].as_array().map_or(0, Vec::len))
        .sum();
    assert!(
        edges >= 2,
        "expected >= 2 captured transitions on the asserted pin, got {edges}",
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
