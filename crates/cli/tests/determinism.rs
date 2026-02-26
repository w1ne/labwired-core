// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::path::PathBuf;
use std::process::Command;

fn get_labwired_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_labwired"))
}

#[test]
fn test_determinism_smoke() {
    let runs = 5;
    let temp_dir = std::env::temp_dir().join("labwired-determinism-smoke");
    let _ = std::fs::remove_dir_all(&temp_dir); // clean start
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let firmware_path = workspace_root.join("tests/fixtures/uart-ok-thumbv7m.elf");
    let system_path = workspace_root.join("configs/systems/ci-fixture-uart1.yaml");

    if !firmware_path.exists() {
        panic!("Firmware fixture not found at {:?}", firmware_path);
    }
    if !system_path.exists() {
        panic!("System fixture not found at {:?}", system_path);
    }
    let firmware_path = firmware_path.canonicalize().unwrap();
    let system_path = system_path.canonicalize().unwrap();

    let script_content = format!(
        r#"
schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
limits:
  max_steps: 1000
assertions: []
"#,
        firmware_path.display(),
        system_path.display()
    );

    let script_path = temp_dir.join("script.yaml");
    std::fs::write(&script_path, script_content).unwrap();

    let mut results: Vec<serde_json::Value> = Vec::new();

    for i in 0..runs {
        let output_dir = temp_dir.join(format!("run_{}", i));
        let output = Command::new(get_labwired_bin())
            .arg("test")
            .arg("--script")
            .arg(&script_path)
            .arg("--output-dir")
            .arg(&output_dir)
            .arg("--no-uart-stdout")
            .output()
            .expect("Failed to run labwired");

        let result_json_path = output_dir.join("result.json");
        if !result_json_path.exists() {
            panic!(
                "Run {} failed to produce result.json. Exit: {:?}\nStdout: {}\nStderr: {}",
                i,
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Assert passed status because firmware-armv6m-ci-fixture is valid and shouldn't crash
        assert!(output.status.success(), "Run {} failed exit code", i);

        let result_json_path = output_dir.join("result.json");
        let result_content =
            std::fs::read_to_string(&result_json_path).expect("Failed to read result.json");
        let json: serde_json::Value =
            serde_json::from_str(&result_content).expect("Failed to parse result.json");
        results.push(json);
    }

    let first = &results[0];
    for (i, current) in results.iter().enumerate().skip(1) {
        assert_eq!(first, current, "Run {} result differs from Run 0", i);
    }
}
