// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The CLI surfaces of fault injection (`labwired fault-inject`) and firmware
//! coverage (`labwired test --coverage`), on the committed nRF54L15
//! smart-ring probe.

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fault_inject(faults: &str) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(root())
        .args([
            "fault-inject",
            "-f",
            "tests/fixtures/nrf54l15-smart-ring.elf",
            "-s",
            "examples/nrf54l15-smart-ring/system.yaml",
            "--until-cycle",
            "20000",
            "--faults-json",
            faults,
        ])
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn fault_inject_prints_the_lockstep_verdict_deterministically() {
    let faults = r#"[{"at_cycle":500,"kind":"register_bit_flip","register":"R0","bit":3}]"#;
    let (code, out, err) = fault_inject(faults);
    assert_eq!(code, 0, "{err}");
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["verdict"], "output_changed");
    assert_eq!(report["injected"][0]["applied_cycle"], 500);
    assert_eq!(report["first_divergence"]["registers"][0]["register"], "R0");
    let (_, again, _) = fault_inject(faults);
    assert_eq!(out, again, "same inputs, same report");

    let (code, _, err) = fault_inject(r#"[{"at_cycle":500,"kind":"bus_nack"}]"#);
    assert_eq!(code, 2);
    assert!(err.contains("unknown variant `bus_nack`"), "{err}");
}

#[test]
fn test_coverage_writes_function_level_lcov() {
    let out_dir = std::env::temp_dir().join(format!("lw-cov-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out_dir);
    let out = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(root())
        .args([
            "test",
            "--script",
            "examples/nrf54l15-smart-ring/io-smoke.yaml",
            "--no-uart-stdout",
            "--coverage",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lcov = std::fs::read_to_string(out_dir.join("coverage.info")).unwrap();
    assert!(lcov.contains("SF:main.c\n"), "{lcov}");
    assert!(lcov.contains("FNDA:1,main\n"), "{lcov}");
    assert!(lcov.contains("FNDA:0,HardFault_Handler\n"), "{lcov}");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out_dir.join("coverage.json")).unwrap())
            .unwrap();
    assert!(json["total_functions"].as_u64().unwrap() > 0);
    assert!(
        json["covered_functions"].as_u64().unwrap() < json["total_functions"].as_u64().unwrap()
    );
    let _ = std::fs::remove_dir_all(&out_dir);
}
