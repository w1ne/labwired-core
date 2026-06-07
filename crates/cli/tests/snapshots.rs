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

    // Phase 1: Run for N steps and save snapshot.
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

    assert!(output.status.success(), "phase 1 failed: {:?}", output);
    assert!(
        snapshot_path.exists(),
        "Snapshot file should have been created"
    );

    // Parse the snapshot JSON and assert PC / SP are in plausible ranges.
    // uart-ok-thumbv7m loads into the default CI fixture address space:
    //   Flash: 0x0000_0000..0x0001_0000 (linked at 0x400 for ci-fixture)
    //   RAM:   0x2000_0000..0x2100_0000
    let snap_raw = std::fs::read_to_string(&snapshot_path).expect("read snapshot");
    let snap: serde_json::Value = serde_json::from_str(&snap_raw).expect("parse snapshot JSON");

    let entry_pc: u64 = 0x401; // ELF entry as logged by the loader
    let saved_pc = snap["cpu"]["pc"]
        .as_u64()
        .expect("snapshot cpu.pc should be a number");
    eprintln!("[snapshot] entry_pc=0x{entry_pc:08x}  saved_pc=0x{saved_pc:08x}");

    // After 100 steps the PC should have advanced past the entry point.
    assert!(
        saved_pc > entry_pc,
        "saved PC 0x{saved_pc:08x} should have advanced past entry 0x{entry_pc:08x}"
    );
    // PC should be in flash range (ci-fixture maps code near 0x400).
    assert!(
        saved_pc < 0x0001_0000,
        "saved PC 0x{saved_pc:08x} is unexpectedly outside flash range"
    );

    // SP is in registers[13] for ARM.
    let regs = snap["cpu"]["registers"]
        .as_array()
        .expect("cpu.registers array");
    let sp = regs
        .get(13)
        .and_then(|v| v.as_u64())
        .expect("SP at registers[13]");
    eprintln!("[snapshot] saved SP=0x{sp:08x}");
    // SP should be in RAM range (0x2000_0000 .. 0x2100_0000).
    assert!(
        (0x2000_0000..0x2100_0000).contains(&sp),
        "saved SP 0x{sp:08x} is not in the expected RAM range 0x20000000..0x21000000"
    );

    // Phase 2: Load the snapshot and run for more steps.
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

    assert!(
        output_load.status.success(),
        "phase 2 failed; stderr: {}",
        String::from_utf8_lossy(&output_load.stderr)
    );

    // Phase 2 must print the resume banner so we know state was actually loaded.
    let stdout = String::from_utf8_lossy(&output_load.stdout);
    let stderr = String::from_utf8_lossy(&output_load.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("Resuming simulation"),
        "Phase 2 should indicate resumption; combined output: {combined}"
    );

    let _ = std::fs::remove_file(&snapshot_path);
}
