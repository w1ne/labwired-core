// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The hosted ESP32-C3 run (ELF-less `labwired test --rom-boot`, no
//! `debug_uart`) shows UART0 and USB-Serial-JTAG in ONE stream, and each byte
//! the mask ROM mirrors onto both consoles must appear in it once.
//!
//! The C3 ROM prints its banner and segment-load log to UART0 AND to the USB
//! CDC port (`usb_uart_tx_one_char`). With both consoles tapped into one plain
//! buffer, the MCP audit (2026-10-09) got every ROM line twice, interleaved
//! mid-word at the two channels' different paces:
//!
//! ```text
//! ESP-ROM:esp32c3-api1-20210207
//! ESP-ROBuild:Feb  7 2021
//! M:esp3rst:0x1 (POWERON),boot:0x8 (SPI_FAST_FLASH_BOOT)
//! ```
//!
//! This drives the real binary exactly as the builder does, on the committed
//! UART0-console Arduino image, and asserts the stream reads as one console.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn write(dir: &Path, name: &str, body: String) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write fixture file");
    path
}

#[test]
fn c3_rom_banner_appears_once_in_the_merged_console() {
    let root = repo_root();
    let flash = root.join("crates/core/tests/fixtures/esp32c3-uart0-console-control-flash.bin");
    assert!(flash.exists(), "missing fixture: {}", flash.display());

    let tmp = std::env::temp_dir().join(format!("lw-c3-merged-console-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create tmp dir");
    // No `debug_uart`: the hosted shape that taps both consoles.
    let system = write(
        &tmp,
        "system.yaml",
        format!(
            "name: \"c3-merged-console\"\nchip: \"{}\"\n",
            root.join("configs/chips/esp32c3.yaml").display()
        ),
    );
    let script = write(
        &tmp,
        "script.yaml",
        format!(
            "schema_version: \"1.0\"\n\
             inputs:\n  firmware: \"\"\n  system: \"{}\"\n\
             limits:\n  max_steps: 30000000\n\
             assertions:\n  - uart_contains: \"LW_CDC_LOOP 1\"\n",
            system.display()
        ),
    );
    let out_dir = tmp.join("out");
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .env("LABWIRED_ESP32C3_FLASH", &flash)
        .args([
            "test",
            "--script",
            script.to_str().unwrap(),
            "--rom-boot",
            "--no-uart-stdout",
            "--no-key",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("spawn labwired");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let uart = std::fs::read_to_string(out_dir.join("uart.log"))
        .unwrap_or_else(|e| panic!("no uart.log ({e}); stderr:\n{stderr}"));
    assert!(
        output.status.success(),
        "run failed (exit {:?}); uart.log:\n{uart}\nstderr:\n{stderr}",
        output.status.code()
    );

    // The ROM log, once and in order, then the sketch.
    let expected_start = "ESP-ROM:esp32c3-api1-20210207\r\n\
                          Build:Feb  7 2021\r\n\
                          rst:0x1 (POWERON),boot:0x8 (SPI_FAST_FLASH_BOOT)\r\n\
                          SPIWP:0xee\r\n\
                          mode:DIO, clock div:1\r\n";
    assert!(
        uart.starts_with(expected_start),
        "the ROM banner must read as one console:\n{uart}"
    );
    assert_eq!(uart.matches("ESP-ROM:").count(), 1, "{uart}");
    assert_eq!(uart.matches("entry 0x").count(), 1, "{uart}");
    assert!(
        uart.contains("entry 0x403cc710\r\nLW_CDC_SETUP\r\n"),
        "{uart}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
