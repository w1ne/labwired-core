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

/// Runs the capacitive touch lab's own `test.yaml` through the real CLI. The
/// firmware ELF is a committed fixture (`tests/fixtures/avr/
/// capacitive-touch-lab.elf`), not built at test time, so this always runs —
/// no skip channel. `total=<n>` lines come from the real, unmodified
/// `CapacitiveSensorSketch` logic (adapted to one sensor); only a real RC/
/// capacitance simulation makes the pressed readings jump well clear of the
/// released ones, so the >= 3x median-ratio check below is the load-bearing
/// physics assertion, on top of `test.yaml`'s own pass/fail assertions.
#[test]
fn capacitive_touch_lab_test_yaml_passes() {
    let root = repo_root();

    let out_dir = temp_artifacts_dir("capacitive-touch-lab");
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .args([
            "test",
            "--script",
            "examples/capacitive-touch-lab/test.yaml",
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

    let counts: Vec<i64> = uart_log
        .lines()
        .filter_map(|line| line.split("total=").nth(1))
        .filter_map(|rest| {
            rest.chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect::<String>()
                .parse()
                .ok()
        })
        .collect();
    assert!(
        counts.len() >= 4,
        "expected multiple total= readings in uart output:\n{uart_log}"
    );

    // The stimulus presses the pad after 8,000,000 cycles (0.5 s at 16 MHz);
    // split readings at the half-way point of the run's step budget as a
    // rough released/pressed partition and compare medians.
    let mid = counts.len() / 2;
    let (released, pressed) = counts.split_at(mid);

    let median = |xs: &[i64]| -> f64 {
        let mut v = xs.to_vec();
        v.sort_unstable();
        v[v.len() / 2] as f64
    };
    let released_median = median(released);
    let pressed_median = median(pressed);

    assert!(
        released_median > 0.0,
        "released median must be positive: {released_median}"
    );
    assert!(
        pressed_median >= released_median * 3.0,
        "pressed median ({pressed_median}) must be at least 3x released median \
         ({released_median}); counts: {counts:?}"
    );
}
