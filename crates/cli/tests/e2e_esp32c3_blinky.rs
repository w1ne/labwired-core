// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// End-to-end coverage for examples/esp32c3-blinky: the canonical ESP32-C3
// Super Mini "hello world" (GPIO8 user LED blink + UART narration). This is
// the demo binary the Playground falls back to when a shared/agent C3 lab
// carries no firmware of its own, so a regression here means every shared
// bare C3 lab stops running ("Cannot run: no firmware" class of bug).
//
// Runs the committed firmware ELF through the committed scenario script via
// the `labwired test` CLI — no cross-compiler toolchain needed. It also
// pins rv32 C.JAL decoding: the blinky is small enough that the linker emits
// a compressed jal from _start to main, which a CJ-immediate decode bug once
// sent off into unmapped flash.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

#[test]
fn esp32c3_blinky_blinks_gpio8_and_narrates_over_uart() {
    let root = repo_root();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out_dir = std::env::temp_dir().join(format!("labwired-c3-blinky-{nonce}"));
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .args([
            "test",
            "--script",
            "examples/esp32c3-blinky/test-blink.yaml",
            "--no-uart-stdout",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute labwired");

    let result_json =
        std::fs::read_to_string(out_dir.join("result.json")).expect("read result.json");
    let result: serde_json::Value = serde_json::from_str(&result_json).expect("parse result.json");
    let uart = std::fs::read_to_string(out_dir.join("uart.log")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&out_dir);

    assert_eq!(
        result["status"].as_str(),
        Some("pass"),
        "blinky scenario failed (exit {:?})\n--- stderr ---\n{}\n--- uart ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
        uart,
    );

    // The loop must actually toggle, not print once and wedge.
    let ons = uart.matches("LED ON").count();
    let offs = uart.matches("LED OFF").count();
    assert!(
        ons >= 2 && offs >= 2,
        "expected several LED ON/OFF transitions, got {ons} on / {offs} off\n--- uart ---\n{uart}",
    );
}
