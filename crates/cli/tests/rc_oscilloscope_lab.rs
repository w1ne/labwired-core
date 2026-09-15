// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("Failed to canonicalize repo root")
}

fn temp_artifacts_dir(prefix: &str) -> PathBuf {
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-golden-{prefix}"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

// The example's own test.yaml expects
// `../../target/thumbv7em-none-eabi/release/rc-oscilloscope-lab`, built by
// `cargo build -p rc-oscilloscope-lab --release --target thumbv7em-none-eabi`.
const FIRMWARE_REL: &str = "target/thumbv7em-none-eabi/release/rc-oscilloscope-lab";

// Runs the RC oscilloscope lab's own test.yaml through the real CLI, the
// same way the golden examples do. That script prints `tau_us=<n>` after
// integrating the RC charge curve in the in-core analog engine and asserts
// it, plus `rc_shape=ok`, itself — but nothing in CI built the firmware it
// needs, so `scripts/example_smokes.sh` (nightly only) reported the missing
// ELF as UNCOVERED rather than a failure. A demo test that never runs is a
// false pass; this makes core-full build the firmware and run it for real.
#[test]
fn rc_oscilloscope_lab_test_yaml_passes() {
    let root = repo_root();
    let firmware = root.join(FIRMWARE_REL);
    if !firmware.exists() {
        labwired_core::test_support::skip_or_fail_missing_firmware(
            "rc-oscilloscope-lab",
            "rc-oscilloscope-lab firmware ELF",
            "cargo build -p rc-oscilloscope-lab --release --target thumbv7em-none-eabi",
        );
        return;
    }

    let out_dir = temp_artifacts_dir("rc-oscilloscope-lab");
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .args([
            "test",
            "--script",
            "examples/rc-oscilloscope-lab/test.yaml",
            "--no-uart-stdout",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute labwired");

    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let result_path = out_dir.join("result.json");
    assert!(result_path.exists(), "result.json not produced");
    let result_json = std::fs::read_to_string(&result_path).expect("read result.json");
    let result: serde_json::Value = serde_json::from_str(&result_json).expect("parse result.json");
    assert_eq!(
        result["status"], "pass",
        "expected status=pass in result.json: {result}"
    );

    let uart_log = std::fs::read_to_string(out_dir.join("uart.log")).expect("read uart.log");
    let _ = std::fs::remove_dir_all(&out_dir);

    assert!(
        uart_log.contains("rc_shape=ok"),
        "uart output missing rc_shape=ok:\n{uart_log}"
    );

    // Only a real RC charge curve produces tau_us in 800..1200 (R1=10k,
    // C1=100n -> tau=1ms); a flat ADC never crosses and a placeholder ramp
    // crosses far too late, so this is the load-bearing physics assertion.
    let tau_us: u32 = uart_log
        .lines()
        .find_map(|line| line.split("tau_us=").nth(1))
        .and_then(|rest| {
            rest.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or_else(|| panic!("no tau_us=<n> found in uart output:\n{uart_log}"));
    assert!(
        (800..1200).contains(&tau_us),
        "tau_us={tau_us} out of expected range 800..1200"
    );
}
