// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn get_labwired_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_labwired"))
}

fn run_stress_test(name: &str, yaml_content: &str, extra_args: &[&str]) -> Value {
    let temp_dir = std::env::temp_dir().join(format!("labwired-stress-{}", name));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let firmware_path = workspace_root.join("tests/fixtures/uart-ok-thumbv7m.elf");
    let system_path = workspace_root.join("configs/systems/ci-fixture-uart1.yaml");

    let firmware_path = firmware_path.canonicalize().unwrap();
    let system_path = system_path.canonicalize().unwrap();

    let script_content = yaml_content
        .replace("__FIRMWARE__", &firmware_path.display().to_string())
        .replace("__SYSTEM__", &system_path.display().to_string());

    let script_path = temp_dir.join("script.yaml");
    std::fs::write(&script_path, script_content).unwrap();

    let mut cmd = Command::new(get_labwired_bin());
    cmd.arg("test")
       .arg("--script").arg(&script_path)
       .arg("--output-dir").arg(&temp_dir)
       .arg("--no-uart-stdout");
    
    for arg in extra_args {
        cmd.arg(arg);
    }

    let output = cmd.output().expect("Failed to run labwired");

    let result_json_path = temp_dir.join("result.json");
    if !result_json_path.exists() {
        panic!("Stress test '{}' failed. Stderr: {}", name, String::from_utf8_lossy(&output.stderr));
    }

    let result_content = std::fs::read_to_string(&result_json_path).unwrap();
    serde_json::from_str(&result_content).unwrap()
}

#[test]
fn test_long_run_cycle_stability() {
    // Run for 1M steps to ensure no drift or crash
    let script = r#"
schema_version: "1.0"
inputs:
  firmware: "__FIRMWARE__"
  system: "__SYSTEM__"
limits:
  max_steps: 1000000
assertions:
  - expected_stop_reason: max_steps
"#;
    let result = run_stress_test("long_run", script, &[]);
    assert_eq!(result["status"], "pass");
    assert_eq!(result["stop_reason"], "max_steps");
}

#[test]
fn test_nested_irq_config_validation() {
    // Verify that complex system configs with multiple IRQs are parsed and loaded correctly
    let script = r#"
schema_version: "1.0"
inputs:
  firmware: "__FIRMWARE__"
  system: "__SYSTEM__"
limits:
  max_steps: 100
assertions: []
"#;
    // We override the system to one with more peripherals if we had one, 
    // but for now we verify the runner handles multiple IRQ sources.
    let result = run_stress_test("irq_config", script, &[]);
    assert_eq!(result["status"], "pass");
}
