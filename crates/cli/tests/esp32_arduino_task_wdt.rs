// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A stock Arduino-ESP32 3.x (ESP-IDF 5.x) task watchdog on a classic ESP32
//! fires on a stuck `loopTask`, and core 1 survives the TWDT's cross-core
//! "print backtrace" request.
//!
//! The sketch arms the TWDT with a 100 ms timeout, feeds it for 300 ms, then
//! spins. The TWDT ISR on core 0 logs the starving task and asks core 1 for a
//! backtrace; core 1 walks its stack from inside the cross-core interrupt.
//! Under shadow-window mode that walk read frames that were never spilled,
//! core 1 raised an exception and halted, and `loopTask` never printed again.
//! The classic `labwired test` path now runs faithful windows on both cores.
//!
//! Fixture: `fixtures/esp32_arduino_task_wdt` (source + platformio.ini
//! alongside; the ELF is `--strip-debug`). Plain path, no `arduino-esp32`
//! profile.

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

#[test]
fn task_watchdog_fires_on_stuck_loop_task_and_core1_resumes() {
    let root = workspace_root();
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-esp32-task-wdt");
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\n\
             inputs:\n  firmware: \"{}\"\n  system: \"{}\"\n\
             limits:\n  max_steps: 150000000\n  wall_time_ms: 600000\n  \
             stop_when_assertions_pass: true\n\
             assertions:\n\
             \x20 - uart_contains: \"LW_TWDT_FED_OK\"\n\
             \x20 - uart_contains: \"task_wdt: Task watchdog got triggered\"\n\
             \x20 - uart_contains: \"Backtrace: 0x\"\n\
             \x20 - uart_contains: \"LW_TWDT_FIRED\"\n",
            root.join("crates/cli/tests/fixtures/esp32_arduino_task_wdt/firmware.elf")
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
    let stderr = String::from_utf8_lossy(&output.stderr);
    let uart = std::fs::read_to_string(dir.join("uart.log")).unwrap_or_default();
    assert!(
        output.status.success(),
        "task watchdog sketch failed\nuart: {uart:?}\nstderr:\n{stderr}"
    );
    assert!(!uart.contains("LW_TWDT_FALSE_FIRE"), "uart: {uart:?}");
    // The fed phase must end before the first timeout report.
    let fed = uart.find("LW_TWDT_FED_OK").unwrap();
    let fired = uart.find("task_wdt: Task watchdog got triggered").unwrap();
    assert!(fed < fired, "watchdog fired while fed\nuart: {uart:?}");
}
