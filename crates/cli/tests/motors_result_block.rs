// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `result.json` must carry the motor evidence block the hosted oracle's
//! `motors` clause reads. The run here is deliberately inert (no firmware
//! drives the PWM pin) — this is the serialization contract, not a dynamics
//! test; dynamics are covered by the bus tests.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn result_json_carries_the_motors_block() {
    let tmp = labwired_cli::test_support::unique_temp_dir("lw-motors-block");
    std::fs::create_dir_all(&tmp).unwrap();
    let chip = tmp.join("chip.yaml");
    let system = tmp.join("system.yaml");
    let script = tmp.join("script.yaml");
    std::fs::write(
        &chip,
        r#"
name: motor-test
arch: arm
core: cortex-m4
flash: { base: 0x08000000, size: "1MB" }
ram: { base: 0x20000000, size: "192KB" }
peripherals:
  - id: gpioa
    type: gpio
    base_address: 0x48000000
    size: "1KB"
    config: { profile: stm32v2 }
"#,
    )
    .unwrap();
    // The DC config intentionally omits encoder pins: this exercises the
    // no-feedback path.
    std::fs::write(
        &system,
        r#"
name: motors-block
chip: chip.yaml
motor_models:
  - kind: dc
    id: wheel
    resistance_ohm: 1.0
    inductance_h: 0.001
    torque_constant_nm_per_a: 0.1
    back_emf_constant_v_per_rad_s: 0.1
    rotor_inertia_kg_m2: 0.01
    viscous_friction_nm_per_rad_s: 0.001
    supply_voltage_v: 12.0
    load_torque_nm: 0.0
    encoder_cpr: 16
    pwm_pin: PA0
    direction_pin: PA1
"#,
    )
    .unwrap();
    std::fs::write(&script, format!(
        "schema_version: \"1.0\"\ninputs:\n  firmware: \"\"\n  system: \"{}\"\nlimits:\n  max_steps: 1000\nassertions:\n  - expected_stop_reason: max_steps\n",
        system.display()
    )).unwrap();
    let out = tmp.join("out");
    // An STM32F4 ELF for the STM32 chip above: the run may fault on an unmapped
    // peripheral, but a mid-run fault still reaches the tail that writes
    // result.json (only a load/reset error takes the early path).
    let firmware = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/nucleo-f407-smoke.elf");
    assert!(firmware.exists(), "missing fixture: {}", firmware.display());

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--firmware",
            firmware.to_str().unwrap(),
            "--no-uart-stdout",
            "--no-key",
            "--output-dir",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("spawn labwired");
    let result = std::fs::read_to_string(out.join("result.json")).unwrap_or_else(|e| {
        panic!(
            "no result.json ({e}); stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let motors = parsed["motors"].as_array().expect("motors block");
    assert_eq!(motors.len(), 1);
    assert_eq!(motors[0]["id"], "wheel");
    assert!(motors[0]["speed_rpm"].is_number());
    assert!(motors[0]["speed_rpm_peak_abs"].is_number());

    // A manifest with no motors omits the block entirely, so motor-less
    // result.json stays byte-identical to the pre-motors contract.
    let no_motor_system = tmp.join("system-no-motors.yaml");
    std::fs::write(&no_motor_system, "name: no-motors\nchip: chip.yaml\n").unwrap();
    let no_motor_script = tmp.join("script-no-motors.yaml");
    std::fs::write(
        &no_motor_script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"\"\n  system: \"{}\"\nlimits:\n  max_steps: 1000\nassertions:\n  - expected_stop_reason: max_steps\n",
            no_motor_system.display()
        ),
    )
    .unwrap();
    let no_motor_out = tmp.join("out-no-motors");
    Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args([
            "test",
            "--script",
            no_motor_script.to_str().unwrap(),
            "--firmware",
            firmware.to_str().unwrap(),
            "--no-uart-stdout",
            "--no-key",
            "--output-dir",
            no_motor_out.to_str().unwrap(),
        ])
        .output()
        .expect("spawn labwired");
    let no_motor: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(no_motor_out.join("result.json")).expect("no-motor result.json"),
    )
    .unwrap();
    assert!(
        no_motor.get("motors").is_none(),
        "an empty motors block must be omitted"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
