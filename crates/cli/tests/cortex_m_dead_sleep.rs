// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A Cortex-M core parked in WFI with no wake source ends at `max_steps`.
//!
//! `fixtures/cortex_m_dead_sleep` drives PA5 high and then sits in `wfi; b`
//! with no interrupt enabled. With idle fast-forward off every WFI cycle is a
//! step, so the run stops at `max_steps`. With it on, `step` counts retired
//! instructions only, and the loop used to skip idle time forever without
//! moving it: `labwired test` never returned. Both arms must now report the
//! same status, stop reason and step count, and neither may hang (a watchdog
//! kills a hung child). Total cycles are expected to differ: the early end
//! moves no device time, so the skip-on arm reports fewer cycles.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run(name: &str, skip: bool) -> serde_json::Value {
    let root = workspace_root();
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-cm-dead-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\n  system: \"{}\"\nlimits:\n  max_steps: 200000\nassertions:\n  - expected_stop_reason: max_steps\n",
            root.join("crates/cli/tests/fixtures/cortex_m_dead_sleep/dead_sleep.elf").display(),
            root.join("configs/systems/nucleo-l476rg.yaml").display(),
        ),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&dir)
        .arg("--no-uart-stdout")
        .env("LABWIRED_IDLE_FAST_FORWARD", if skip { "1" } else { "0" })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("run labwired");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("labwired test hung on a dead WFI sleep (idle_ff={skip})");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let result = std::fs::read_to_string(dir.join("result.json")).expect("result.json");
    serde_json::from_str(&result).unwrap()
}

#[test]
fn a_dead_wfi_sleep_ends_at_max_steps_with_and_without_the_skip() {
    let fast = run("skip", true);
    let reference = run("ref", false);
    for (arm, r) in [("idle_ff on", &fast), ("idle_ff off", &reference)] {
        assert_eq!(r["status"], "pass", "{arm}: {r}");
        assert_eq!(r["stop_reason"], "max_steps", "{arm}: {r}");
        assert_eq!(r["steps_executed"], 200_000, "{arm}: {r}");
    }
}
