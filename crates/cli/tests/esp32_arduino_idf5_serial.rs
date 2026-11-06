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
//! Fixture: `fixtures/esp32_arduino_idf5_serial` (source + platformio.ini
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

#[test]
fn idf5_sketch_prints_from_setup_and_loop() {
    let root = workspace_root();
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-esp32-idf5-serial");
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.2\"\n\
             inputs:\n  profile: \"arduino-esp32\"\n  firmware: \"{}\"\n  system: \"{}\"\n\
             limits:\n  max_steps: 60000000\n  stop_when_assertions_pass: true\n\
             assertions:\n  - uart_contains: \"idf5-boot\"\n  - uart_contains: \"tick\"\n",
            root.join("crates/cli/tests/fixtures/esp32_arduino_idf5_serial/firmware.elf")
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
        "IDF 5 sketch did not print banner + loop line\nuart: {uart:?}\nstderr:\n{stderr}"
    );
    assert!(uart.contains("idf5-boot\r\n"), "uart: {uart:?}");
    assert!(uart.contains("tick\r\n"), "uart: {uart:?}");
}
