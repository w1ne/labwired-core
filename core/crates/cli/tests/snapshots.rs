// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn get_temp_path(prefix: &str, extension: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push("labwired-tests");
    let _ = std::fs::create_dir_all(&dir);

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    dir.join(format!("{}-{}.{}", prefix, nonce, extension))
}

#[test]
fn test_cli_snapshot_save_and_load() {
    let fw_abs = std::fs::canonicalize("../../tests/fixtures/uart-ok-thumbv7m.elf").unwrap();
    let snapshot_path = get_temp_path("snapshot", "json");

    // Phase 1: Run until a breakpoint and save snapshot
    // We'll use a breakpoint at some point in the UART output loop.
    // For uart-ok-thumbv7m, entry is usually around 0x8000000 (standard STM32) or similar if mapped.
    // Actually, let's just run for N steps and save.

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "--firmware",
            fw_abs.to_str().unwrap(),
            "--max-steps",
            "100",
            "--snapshot",
            snapshot_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute phase 1");

    assert!(output.status.success());
    assert!(
        snapshot_path.exists(),
        "Snapshot file should have been created"
    );

    // Phase 2: Load the snapshot and run for more steps
    // We expect it to resume and potentially output more characters if it wasn't done.
    let output_load = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "machine",
            "load",
            "--snapshot",
            snapshot_path.to_str().unwrap(),
            "--max-steps",
            "100",
        ])
        .output()
        .expect("Failed to execute phase 2");

    assert!(output_load.status.success());

    // Basic verification: it didn't crash and finished successfully.
    let stdout = String::from_utf8_lossy(&output_load.stdout);
    assert!(
        stdout.contains("Resuming simulation"),
        "Should indicate resumption"
    );
}
