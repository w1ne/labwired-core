// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn test_cli_interactive_writes_snapshot() {
    let firmware = std::fs::canonicalize("../../tests/fixtures/uart-ok-thumbv7m.elf").unwrap();

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let snapshot_path =
        std::env::temp_dir().join(format!("labwired-interactive-snapshot-{}.json", nonce));
    let _ = std::fs::remove_file(&snapshot_path);

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "--firmware",
            firmware.to_str().unwrap(),
            "--max-steps",
            "1",
            "--snapshot",
            snapshot_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute labwired");

    assert!(output.status.success());
    assert!(snapshot_path.exists());

    let snapshot_content = std::fs::read_to_string(&snapshot_path).unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot_content).unwrap();
    assert_eq!(snapshot["type"], "interactive");

    let regs = snapshot["cpu"]["registers"].as_array().unwrap();
    assert_eq!(regs.len(), 16);

    // PC must be in the CI-fixture flash range (linked near 0x400).
    let pc = snapshot["cpu"]["pc"]
        .as_u64()
        .expect("cpu.pc must be numeric");
    assert!(
        (0x400..0x1_0000).contains(&pc),
        "PC 0x{pc:08x} is not in the expected flash range [0x400, 0x10000)"
    );

    // SP is registers[13] and must be in the RAM region.
    let sp = regs[13]
        .as_u64()
        .expect("SP (registers[13]) must be numeric");
    assert!(
        (0x2000_0000..0x2100_0000).contains(&sp),
        "SP 0x{sp:08x} is not in expected RAM range [0x20000000, 0x21000000)"
    );

    eprintln!("[interactive_snapshot] pc=0x{pc:08x}  sp=0x{sp:08x}");
    let _ = std::fs::remove_file(&snapshot_path);
}
