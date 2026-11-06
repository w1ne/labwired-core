// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A plain Arduino-ESP32 3.x (ESP-IDF 5.x) sketch on a bare classic ESP32
//! prints from `setup()` and from `loop()` under the `arduino-esp32` profile.
//!
//! IDF 5.x builds `xSemaphoreCreateMutexWithCaps` as heap_caps_malloc of a
//! StaticQueue_t followed by `xQueueCreateMutexStatic`, and
//! `uart_driver_install` creates the UART `tx_mux` that way. The profile used
//! to thunk `xQueueCreateMutexStatic` to "return the buffer", so the queue was
//! never initialised and its `xQueueLock` owner word held heap garbage. The
//! first `Serial.println` then spun forever in `spinlock_acquire`: zero UART
//! bytes, zero GPIO edges, for every IDF 5 sketch on esp32dev.
//!
//! Fixtures: `fixtures/esp32_arduino_idf5_serial` and `_tasks` (source + platformio.ini
//! alongside; the ELF is `--strip-debug`, symbols kept). With the old thunk
//! this run reaches max_steps with an empty UART log.

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

/// Runs `fixtures/<fixture>/firmware.elf` under the arduino-esp32 profile on
/// the bare esp32-wroom-32 system and returns (success, uart, stderr).
fn run_fixture(fixture: &str, max_steps: u64, uart_contains: &[&str]) -> (bool, String, String) {
    let root = workspace_root();
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-{fixture}"));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    let assertions: String = uart_contains
        .iter()
        .map(|s| format!("  - uart_contains: {s:?}\n"))
        .collect();
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.2\"\n\
             inputs:\n  profile: \"arduino-esp32\"\n  firmware: \"{}\"\n  system: \"{}\"\n\
             limits:\n  max_steps: {max_steps}\n  stop_when_assertions_pass: true\n\
             assertions:\n{assertions}",
            root.join(format!("crates/cli/tests/fixtures/{fixture}/firmware.elf"))
                .display(),
            root.join("configs/systems/esp32-wroom-32.yaml").display(),
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .args(["test", "--script"])
        .arg(&script)
        .arg("--output-dir")
        .arg(&dir)
        .arg("--no-uart-stdout")
        .output()
        .expect("run labwired");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let uart = std::fs::read_to_string(dir.join("uart.log")).unwrap_or_default();
    (output.status.success(), uart, stderr)
}

#[test]
fn idf5_sketch_prints_from_setup_and_loop() {
    let (ok, uart, stderr) = run_fixture(
        "esp32_arduino_idf5_serial",
        60_000_000,
        &["idf5-boot", "tick"],
    );
    assert!(
        ok,
        "IDF 5 sketch did not print banner + loop line\nuart: {uart:?}\nstderr:\n{stderr}"
    );
    assert!(uart.contains("idf5-boot\r\n"), "uart: {uart:?}");
    assert!(uart.contains("tick\r\n"), "uart: {uart:?}");
}

/// Two FreeRTOS tasks (loopTask + a worker) each print and block in a real
/// `vTaskDelay(1)`, so every line is a real context switch and both contend
/// for the UART `tx_mux`.
///
/// IDF 5 dual-core names the running-task array `pxCurrentTCBs`. The profile
/// only looked up `pxCurrentTCB`, so the address stayed unset: the
/// `xTaskGetCurrentTaskHandle` thunk returned NULL and the CPU's hybrid window
/// preserve was keyed off a hardcoded address from another firmware — the
/// same bogus key for every task. The first switch resumed loopTask with the
/// wrong windows and `xTaskPriorityDisinherit` asserted after `m 0`.
#[test]
fn idf5_two_tasks_context_switch_and_share_serial() {
    let (ok, uart, stderr) = run_fixture(
        "esp32_arduino_idf5_tasks",
        120_000_000,
        &["tt-boot", "m 200\n", "w 200\n"],
    );
    assert!(
        ok,
        "two-task IDF 5 sketch did not reach 200 switches per task\n\
         uart tail: {:?}\nstderr:\n{stderr}",
        &uart[uart.len().saturating_sub(200)..]
    );
    assert!(!stderr.contains("[abort_halt]"), "stderr:\n{stderr}");
}
