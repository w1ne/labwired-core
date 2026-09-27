// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The COMPOSED motors acceptance path, end to end through the real CLI.
//!
//! The oracle's `motors` clause is lowered by `motorClausesToAcceptance`
//! (`packages/board-config/src/oracle.ts`) into a script assertion
//! `motor_speed_reached: { id, min_abs_rpm }`. The CLI parses that assertion,
//! the run stops early on `assertions_passed`, and `result.json` carries the
//! motor evidence (`speed_rpm_peak_abs`) the oracle then reads back.
//!
//! Each seam is unit-tested on its own; this test drives the whole wire:
//! lowered YAML → CLI parse → evaluator → bus latch → early stop →
//! `result.json` motors block. The firmware is the STM32L476 BLDC six-step
//! demo — a real `thumbv7em-none-eabihf` ELF whose loop spins the motor to its
//! target speed, so the peak evidence is dynamics, not a static field.
//!
//! The assertion YAML below is the verbatim shape `motorClausesToAcceptance` +
//! `buildScriptYaml` emit (see `oracle-core-contract.test.ts`, which gates the
//! key names against core's deserializer).

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Repo root = crates/cli/../.. (matches the other CLI integration tests).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

/// The BLDC demo firmware, built once per test process into a dedicated target
/// dir (same helper as `crates/core/tests/nucleo_l476rg.rs`, which owns the
/// dynamics assertions; this test owns the CLI wire).
fn ensure_bldc_firmware_built() -> PathBuf {
    static ELF: OnceLock<PathBuf> = OnceLock::new();
    ELF.get_or_init(|| {
        let root = repo_root();
        let target_dir = std::env::temp_dir()
            .join("labwired-fixtures")
            .join("stm32l476-bldc");
        let status = Command::new("cargo")
            .args([
                "build",
                "--release",
                "-p",
                "firmware-l476-demo",
                "--bin",
                "firmware-l476-bldc-six-step",
                "--target",
                "thumbv7em-none-eabihf",
                "--target-dir",
            ])
            .arg(&target_dir)
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTFLAGS")
            .current_dir(&root)
            .status()
            .expect("invoke cargo build for STM32L476 BLDC fixture");
        assert!(status.success(), "STM32L476 BLDC firmware build failed");
        let elf = target_dir.join("thumbv7em-none-eabihf/release/firmware-l476-bldc-six-step");
        assert!(elf.is_file(), "firmware ELF was not produced at {elf:?}");
        elf
    })
    .clone()
}

/// Deliberately far below the demo's target speed: the point is that the run
/// stops on the LOWERED clause, not that it reaches the firmware's own band.
const LOWERED_MIN_RPM: f64 = 10.0;

#[test]
fn lowered_motor_clause_stops_the_run_and_leaves_peak_evidence() {
    let root = repo_root();
    let elf = ensure_bldc_firmware_built();
    let system = root.join("examples/nucleo-l476rg-bldc/system.yaml");
    assert!(system.is_file(), "missing system: {}", system.display());

    let tmp = labwired_cli::test_support::unique_temp_dir("lw-motors-composed");
    std::fs::create_dir_all(&tmp).expect("create tmp dir");
    let script = tmp.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\n\
             inputs:\n  \
               firmware: \"\"\n  \
               system: \"{}\"\n\
             limits:\n  \
               max_steps: 5000000\n  \
               stop_when_assertions_pass: true\n  \
               stop_when_assertions_pass_settle_steps: 100000\n  \
               stop_when_assertions_pass_min_steps: 1000\n\
             assertions:\n  \
               - motor_speed_reached:\n      \
                   id: drive_motor\n      \
                   min_abs_rpm: {LOWERED_MIN_RPM}\n",
            system.display(),
        ),
    )
    .expect("write script");

    let out_dir = tmp.join("out");
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--firmware",
            elf.to_str().unwrap(),
            "--no-uart-stdout",
            "--no-key",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn labwired");

    let result = std::fs::read_to_string(out_dir.join("result.json")).unwrap_or_else(|e| {
        panic!(
            "no result.json ({e}); stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(
        output.status.success(),
        "run failed (exit {:?}); result.json:\n{result}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        parsed["stop_reason"], "assertions_passed",
        "the lowered motor clause must stop the run on evidence:\n{result}"
    );
    assert_eq!(parsed["assertions"][0]["passed"], true, "got:\n{result}");

    let motors = parsed["motors"].as_array().expect("motors block");
    let motor = motors
        .iter()
        .find(|m| m["id"] == "drive_motor")
        .unwrap_or_else(|| panic!("no drive_motor snapshot in:\n{result}"));
    let peak = motor["speed_rpm_peak_abs"]
        .as_f64()
        .expect("speed_rpm_peak_abs is a number");
    assert!(
        peak >= LOWERED_MIN_RPM,
        "peak {peak} rpm is below the clause's {LOWERED_MIN_RPM} rpm:\n{result}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
