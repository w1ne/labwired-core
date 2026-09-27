// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 USB-CDC console gate: an Arduino sketch whose `Serial` is an
//! interrupt-driven `HWCDC` must emit its output through the USB_SERIAL_JTAG
//! sink on the REAL ELF-less rom-boot path the hosted builder drives.
//!
//! # Why this gate exists
//!
//! An ESP32-S3-Zero wires its USB-C socket to the chip's own USB-Serial-JTAG
//! block, not to UART0. Firmware for such a board is built with
//! `-DARDUINO_USB_CDC_ON_BOOT=1 -DARDUINO_USB_MODE=1`, which makes Arduino's
//! `Serial` an `HWCDC` rather than a `HardwareSerial` — `HardwareSerial` is not
//! even linked into the image, so a silent USB_SERIAL_JTAG block means the
//! sketch's console is invisible in the twin.
//!
//! `HWCDC` is entirely interrupt-driven: it decides the cable is plugged in
//! from `USB_SERIAL_JTAG.int_raw`'s SOF bit (polled by a 1 kHz FreeRTOS tick
//! hook) and its TX ISR is delivered through the interrupt matrix on
//! `SERIAL_IN_EMPTY` (S3 matrix source 96). A model that only appends EP1
//! writes captures NOTHING from such a build. The model
//! (`crates/core/src/peripherals/esp32s3/usb_serial_jtag.rs`) therefore
//! models SOF, INT_ENA/INT_ST/INT_CLR, the SERIAL_IN_EMPTY level and the
//! matrix source; this test asserts the observable end of that chain.
//!
//! # What the fixture is
//!
//! `tests/fixtures/esp32s3-usb-cdc-console-flash.bin` is a merged flash image
//! (bootloader@0x0 + partition table@0x8000 + boot_app0@0xe000 + app@0x10000)
//! for `board = esp32-s3-devkitc-1`, built with PlatformIO
//! `platform = espressif32@7.0.1` and
//! `build_flags = -DARDUINO_USB_CDC_ON_BOOT=1 -DARDUINO_USB_MODE=1`. The
//! sketch is:
//!
//! ```cpp
//! #include <Arduino.h>
//! void setup() { Serial.begin(115200); Serial.println("LW_CDC_SETUP"); }
//! void loop() {
//!   static uint32_t n = 0;
//!   Serial.print("LW_CDC_LOOP "); Serial.println(n++);
//!   delay(50);
//! }
//! ```
//!
//! It prints `LW_CDC_SETUP` once from `setup()` and then `LW_CDC_LOOP <n>`
//! (CRLF-terminated, as Arduino `println` emits) from `loop()` every 50 ms.
//! The gate asserts on a LOOP line past the first iteration: a sketch whose
//! `loop()` is dead can still print from `setup()`.
//!
//! MEASURED: `LW_CDC_LOOP 1` leaves the chip at 9.7M steps on the release CLI
//! (2.7 s wall clock), so the 30M budget below stops on `assertions_passed`
//! with ~3x headroom rather than running the budget out.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Repo root = crates/cli/../.. (matches the other CLI integration tests).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn require(path: PathBuf) -> PathBuf {
    assert!(path.exists(), "missing fixture: {}", path.display());
    path
}

const CDC_MAX_STEPS: u64 = 30_000_000;

/// Write a `labwired test` script that names a system + limits + assertions but
/// deliberately sets an EMPTY firmware input (the schema requires the key; the
/// CLI filters empty and takes the ELF-less rom-boot path). Returns its path.
///
/// The stop-on-evidence rails mirror what the hosted builder emits
/// (`STOP_SETTLE_STEPS`/`STOP_MIN_STEPS` in services/labwired-builder): a run
/// that satisfies the assertion stops with `assertions_passed`, so this gate
/// proves the console bytes arrived, not merely that the budget elapsed.
fn write_cdc_script(dir: &Path, system: &Path) -> PathBuf {
    let script = format!(
        "schema_version: \"1.0\"\n\
         inputs:\n  \
           firmware: \"\"\n  \
           system: \"{}\"\n\
         limits:\n  \
           max_steps: {CDC_MAX_STEPS}\n  \
           stop_when_assertions_pass: true\n  \
           stop_when_assertions_pass_settle_steps: 100000\n  \
           stop_when_assertions_pass_min_steps: 1000\n\
         assertions:\n  \
           - uart_contains: \"LW_CDC_LOOP 1\"\n",
        system.display(),
    );
    let path = dir.join("s3_usb_cdc_console.yaml");
    std::fs::write(&path, script).expect("write test script");
    path
}

#[test]
fn s3_arduino_usb_cdc_serial_is_captured() {
    let root = repo_root();
    let flash = require(root.join("tests/fixtures/esp32s3-usb-cdc-console-flash.bin"));
    let system = require(root.join("configs/systems/esp32s3-zero.yaml"));

    let tmp = std::env::temp_dir().join(format!("lw-s3-usb-cdc-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create tmp dir");
    let script = write_cdc_script(&tmp, &system);
    let out_dir = tmp.join("out");
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    // Invoke exactly as the builder does on the rom-boot path: `--rom-boot`, the
    // flash image via the env pin, and NO `--firmware`. The boot ROM
    // auto-provisions from the vendored images (crates/core/roms/esp32s3/)
    // resolved relative to the repo-root CWD. The CLI routes the USB_SERIAL_JTAG
    // sink into the same capture buffer as UART0 for this path.
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .env("LABWIRED_ESP32S3_FLASH", &flash)
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--rom-boot",
            "--no-uart-stdout",
            "--no-key",
            "--output-dir",
            out_dir.to_str().unwrap(),
            "--max-steps",
            &CDC_MAX_STEPS.to_string(),
        ])
        .output()
        .expect("spawn labwired");

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    // The ELF-less branch must have been taken (not a silent firmware fallback).
    assert!(
        stderr.contains("ELF-less"),
        "expected the ELF-less S3 rom-boot branch to run; stderr:\n{stderr}"
    );

    let result_path = out_dir.join("result.json");
    let result_json = std::fs::read_to_string(&result_path).unwrap_or_else(|e| {
        panic!(
            "no result.json at {} (exit {:?}): {e}\nstderr:\n{stderr}",
            result_path.display(),
            output.status.code(),
        )
    });

    assert!(
        !result_json.contains("\"config_error\""),
        "run ended in a config_error:\n{result_json}\nstderr:\n{stderr}"
    );
    assert!(
        output.status.success(),
        "ELF-less S3 rom-boot run failed (exit {:?}); the CDC serial assertion did not pass.\n\
         result.json:\n{result_json}\nstderr:\n{stderr}",
        output.status.code(),
    );
    assert!(
        result_json.contains("\"assertions_passed\""),
        "the run must stop on the CDC evidence, not the step budget:\n{result_json}"
    );

    // The captured console: ROM banner + the sketch's own output. The image has
    // NO HardwareSerial linked (CDC-on-boot), so these bytes can only have come
    // through the USB_SERIAL_JTAG sink.
    let uart_log = std::fs::read_to_string(out_dir.join("uart.log")).unwrap_or_default();
    for needle in ["LW_CDC_SETUP", "LW_CDC_LOOP 0", "LW_CDC_LOOP 1"] {
        assert!(
            uart_log.contains(needle),
            "captured console is missing {needle:?} — the CDC console did not reach the twin.\n\
             uart.log:\n{uart_log}\nstderr:\n{stderr}"
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}
