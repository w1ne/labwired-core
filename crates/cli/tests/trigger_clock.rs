// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Every `labwired test` trigger runs on the machine clock.
//!
//! `after_cycles` stimuli, `max_cycles` and the reported `at_cycle` all mean
//! `Machine::total_cycles`, the clock co-simulation and traces convert to time.
//! They used to be compared against the `PerformanceMetrics` counter, which on
//! an AVR counts datasheet cycles and on Cortex-M counts 2 per 32-bit Thumb
//! instruction, so a press meant for cycle 8,000,000 on a Nano landed at
//! 5,121,440 while `at_cycle` reported that as if it were on time.
//!
//! A trigger must land on its cycle, or at the first instruction boundary
//! after it: never more than one instruction's cycles late (4 on an AVR, 1 on
//! Cortex-M).

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Run `labwired test` over a temp-dir system + script and return `result.json`.
fn run(name: &str, system_yaml: &str, script_yaml: &str) -> serde_json::Value {
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-clock-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("system.yaml"), system_yaml).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(&script, script_yaml).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&dir)
        .arg("--no-uart-stdout")
        .output()
        .expect("run labwired");
    let result = std::fs::read_to_string(dir.join("result.json")).unwrap_or_else(|_| {
        panic!(
            "no result.json; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let _ = std::fs::remove_dir_all(&dir);
    serde_json::from_str(&result).expect("parse result.json")
}

/// A system whose one model reads `ui.clock.probe`, so a `cosim_signal`
/// stimulus has something to reach. The model steps every 10 ms: its
/// boundaries are far from the thresholds below, so only the trigger's own
/// cycle cap can land the machine on them.
fn system(chip: &str) -> String {
    format!(
        r#"
name: "trigger-clock"
chip: "{chip}"
cosim_models:
  - id: "probe"
    adapter: "mock"
    step_ns: 10000000
    inputs:
      level: "ui.clock.probe"
    outputs: {{}}
    config:
      outputs: {{}}
external_devices: []
"#,
        chip = workspace_root().join(chip).display()
    )
}

fn script(firmware: &str, max_steps: u64, thresholds: &[u64]) -> String {
    let stimuli: String = thresholds
        .iter()
        .enumerate()
        .map(|(i, cycles)| {
            format!(
                "  - cosim_signal: {{ path: ui.clock.probe, value: {i} }}\n    trigger: !after_cycles {{ cycles: {cycles} }}\n"
            )
        })
        .collect();
    format!(
        "schema_version: \"1.2\"\ninputs:\n  firmware: \"{}\"\n  system: \"./system.yaml\"\n\
         limits:\n  max_steps: {max_steps}\nstimuli:\n{stimuli}assertions:\n  \
         - expected_stop_reason: max_steps\n",
        workspace_root().join(firmware).display()
    )
}

fn assert_applied_on_time(result: &serde_json::Value, thresholds: &[u64], late_by_at_most: u64) {
    assert_eq!(result["status"], "pass", "{result}");
    let stimuli = result["stimuli"].as_array().expect("stimuli block");
    assert_eq!(stimuli.len(), thresholds.len(), "{result}");
    for (stimulus, threshold) in stimuli.iter().zip(thresholds) {
        assert_eq!(stimulus["outcome"], "applied", "{stimulus}");
        let at = stimulus["at_cycle"].as_u64().expect("at_cycle");
        assert!(
            (*threshold..=threshold + late_by_at_most).contains(&at),
            "after_cycles {threshold} applied at machine cycle {at} (allowed {threshold}..={})",
            threshold + late_by_at_most
        );
    }
}

/// ATmega328P: the Nano blink sketch, whose instructions take 1–4 cycles.
#[test]
fn an_avr_stimulus_lands_on_its_machine_cycle() {
    let thresholds = [123_457, 1_000_003, 4_567_891];
    let result = run(
        "avr-stimulus",
        &system("configs/chips/atmega328p.yaml"),
        &script(
            "tests/fixtures/avr/arduino-nano-blinky.elf",
            4_000_000,
            &thresholds,
        ),
    );
    // One AVR step is at most 4 cycles, so the first boundary at or past the
    // threshold is at most 3 cycles after it.
    assert_applied_on_time(&result, &thresholds, 3);
}

/// STM32F401: the NUCLEO blink image, a mix of 16- and 32-bit Thumb.
#[test]
fn an_arm_stimulus_lands_on_its_machine_cycle() {
    let thresholds = [123_457, 1_000_003, 1_499_999];
    let result = run(
        "arm-stimulus",
        &system("configs/chips/stm32f401.yaml"),
        &script(
            "tests/fixtures/stm32f401-blinky.elf",
            1_600_000,
            &thresholds,
        ),
    );
    assert_applied_on_time(&result, &thresholds, 0);
}

/// `max_cycles` on an AVR stops at the requested machine cycle, and the stop
/// details report the observation on that clock.
#[test]
fn an_avr_max_cycles_limit_stops_on_its_machine_cycle() {
    for limit in [10_001_u64, 777_777] {
        let result = run(
            "avr-max-cycles",
            &format!(
                "name: \"avr-max-cycles\"\nchip: \"{}\"\nexternal_devices: []\n",
                workspace_root()
                    .join("configs/chips/atmega328p.yaml")
                    .display()
            ),
            &format!(
                "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\n  system: \"./system.yaml\"\n\
                 limits:\n  max_steps: 5000000\n  max_cycles: {limit}\nassertions:\n  \
                 - expected_stop_reason: max_cycles\n",
                workspace_root()
                    .join("tests/fixtures/avr/arduino-nano-blinky.elf")
                    .display()
            ),
        );
        assert_eq!(result["status"], "pass", "{result}");
        assert_eq!(result["stop_reason"], "max_cycles", "{result}");
        let details = &result["stop_reason_details"];
        assert_eq!(details["triggered_limit"]["value"], limit, "{details}");
        let observed = details["observed"]["value"].as_u64().expect("observed");
        assert!(
            (limit..=limit + 3).contains(&observed),
            "max_cycles {limit} stopped at machine cycle {observed}"
        );
        assert!(
            result["steps_executed"].as_u64().unwrap() < limit,
            "AVR instructions take more than one cycle on average, so fewer steps than \
             cycles: {result}"
        );
    }
}
