// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#![cfg(feature = "event-scheduler")]

//! `--capture-app-entry` must cost the run nothing once the snapshot is written.
//!
//! The hosted builder runs every ESP32-C3 rom-boot with a snapshot-cache miss
//! (the first run of every freshly compiled sketch) as `--rom-boot
//! --capture-app-entry`. The capture is taken at the first app-entry PC, a few
//! million steps in. The run loop used to treat the flag as a standing
//! condition and kept idle fast-forward at one cycle of fuel per call for the
//! whole run, so every idle FreeRTOS tick was skipped one cycle at a time.
//! Measured on a stock Arduino ESP32-C3 sketch (SSD1306 + analogRead + Serial,
//! acceptance clause, 1 s of device time): 2.6 s without the flag, 27.8 s with
//! it, identical steps, cycles and UART. On the hosted wall-clock budget that
//! was the difference between a pass and a timeout.
//!
//! This drives the real binary twice on the committed `esp32c3-oled-demo`
//! flash image, with the hosted acceptance shape (an early-stop clause that
//! never matches, so the run goes to `max_cycles`), and asserts:
//! * the capture is written,
//! * the capture run is observably identical to the run without the flag,
//! * it is not several times slower in wall clock.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// 1 s of device time at 160 MHz: the image boots, logs, paints the OLED and
/// then idles in its 500 ms refresh loop, which is where the fuel matters.
const MAX_CYCLES: u64 = 160_000_000;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn write_script(dir: &Path, system: &Path) -> PathBuf {
    let script = format!(
        "schema_version: \"1.0\"\n\
         inputs:\n  \
           firmware: \"\"\n  \
           system: \"{}\"\n\
         limits:\n  \
           max_steps: 4000000000\n  \
           max_cycles: {MAX_CYCLES}\n  \
           wall_time_ms: 600000\n  \
           stop_when_assertions_pass: true\n  \
           stop_when_assertions_pass_settle_steps: 100000\n  \
           stop_when_assertions_pass_min_steps: 1000\n\
         assertions:\n  \
           - uart_contains: \"this text is never printed\"\n",
        system.display(),
    );
    let path = dir.join("script.yaml");
    std::fs::write(&path, script).expect("write script");
    path
}

struct Run {
    wall: Duration,
    result: serde_json::Value,
    stderr: String,
}

fn run(root: &Path, flash: &Path, script: &Path, out: &Path, capture: Option<&Path>) -> Run {
    std::fs::create_dir_all(out).expect("create out dir");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_labwired"));
    cmd.current_dir(root)
        .env("LABWIRED_ESP32C3_FLASH", flash)
        .env_remove("LABWIRED_IDLE_FAST_FORWARD")
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--rom-boot",
            "--no-uart-stdout",
            "--no-key",
            "--output-dir",
            out.to_str().unwrap(),
        ]);
    if let Some(path) = capture {
        cmd.arg("--capture-app-entry").arg(path);
    }
    let start = Instant::now();
    let output = cmd.output().expect("spawn labwired");
    let wall = start.elapsed();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let text = std::fs::read_to_string(out.join("result.json")).unwrap_or_else(|e| {
        panic!(
            "no result.json in {}: {e}\nstderr:\n{stderr}",
            out.display()
        )
    });
    Run {
        wall,
        result: serde_json::from_str(&text).expect("parse result.json"),
        stderr,
    }
}

#[test]
fn capture_app_entry_run_matches_and_keeps_pace_with_plain_run() {
    let root = repo_root();
    let flash = root.join("crates/wasm/tests/fixtures/esp32c3-oled-demo-flash.bin");
    let system = root.join("configs/systems/esp32c3-oled-demo.yaml");
    assert!(flash.exists(), "missing fixture {}", flash.display());
    assert!(system.exists(), "missing fixture {}", system.display());

    let tmp = std::env::temp_dir().join(format!("lw-c3-capture-speed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tmp dir");
    let script = write_script(&tmp, &system);
    let snapshot = tmp.join("app-entry.lwrs");

    let plain = run(&root, &flash, &script, &tmp.join("plain"), None);
    let captured = run(
        &root,
        &flash,
        &script,
        &tmp.join("capture"),
        Some(&snapshot),
    );

    assert!(
        snapshot.exists(),
        "the app-entry snapshot was not written; stderr:\n{}",
        captured.stderr
    );

    for key in ["stop_reason", "steps_executed", "cycles", "uart_lines"] {
        assert_eq!(
            plain.result[key], captured.result[key],
            "`{key}` differs between the run with --capture-app-entry and the run without it"
        );
    }
    assert_eq!(plain.result["stop_reason"], "max_cycles");

    // Measured on this test before the fix (release, x86_64): 12.8 s plain,
    // 29.2 s with the flag (2.3x; this image spends more of its second
    // interpreted than the Arduino sketch above does). After: the same wall
    // time either way. The bound leaves room for a noisy shared runner and
    // still fails on the one-cycle fuel.
    eprintln!(
        "plain run {:?}, --capture-app-entry run {:?}",
        plain.wall, captured.wall
    );
    let bound = plain.wall.mul_f64(1.5) + Duration::from_secs(1);
    assert!(
        captured.wall <= bound,
        "--capture-app-entry run took {:?}, the identical run without it {:?} (bound {:?})",
        captured.wall,
        plain.wall,
        bound
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
