// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn write_temp_file(prefix: &str, contents: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push("labwired-tests");
    let _ = std::fs::create_dir_all(&dir);

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = dir.join(format!("{}-{}.yaml", prefix, nonce));
    std::fs::write(&path, contents).expect("Failed to write temp file");
    path
}

#[test]
fn test_cli_test_mode_breakpoint_halts_immediately() {
    let fw_abs = std::fs::canonicalize("../../tests/fixtures/uart-ok-thumbv7m.elf").unwrap();

    let program = labwired_loader::load_elf(Path::new(fw_abs.to_str().unwrap())).unwrap();
    let mut bus = labwired_core::bus::SystemBus::new();
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = labwired_core::Machine::new(cpu, bus);
    machine.load_firmware(&program).unwrap();
    let initial_pc = machine.cpu.pc;

    let script = write_temp_file(
        "script-breakpoint",
        &format!(
            r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
limits:
  max_steps: 1000000
assertions: []
"#,
            fw_abs.to_str().unwrap()
        ),
    );

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let output_dir = std::env::temp_dir().join(format!("labwired-tests-breakpoint-{}", nonce));
    let _ = std::fs::remove_dir_all(&output_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--no-uart-stdout",
            "--output-dir",
            output_dir.to_str().unwrap(),
            "--breakpoint",
            &format!("0x{initial_pc:x}"),
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());

    let result_path = output_dir.join("result.json");
    assert!(result_path.exists());
    let result_content = std::fs::read_to_string(&result_path).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result_content).unwrap();

    assert_eq!(result["status"], "pass");
    assert_eq!(result["stop_reason"], "halt");
    assert_eq!(result["steps_executed"], 0);
}
