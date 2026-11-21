// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Classic ESP32 console tap: the dev-board USB-UART bridge (CP210x / CH340 /
//! CH9102) carries UART0 only, and arduino-esp32 `Serial` is UART0. Each
//! classic ESP32 manifest therefore names `debug_uart: "uart0"`. Without that
//! line the run paths fell back to one capture on every UART, and firmware
//! that also drove UART2 (Arduino `Serial2`) spliced that traffic into the
//! Serial pane.
//!
//! Firmware: tests/fixtures/esp32-classic-console/console.S (register level;
//! UART0 prints `VCP_BOOT` / `VCP_DONE`, UART2 sends `AUX_UART2_DATA` one
//! byte at a time between the console bytes and waits for each to drain).

use crate::common;
use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use std::time::Duration;

const CONSOLE: &str = "VCP_BOOT\r\nVCP_DONE\r\n";
const FIXTURE: &str = "crates/core/tests/fixtures/esp32-classic-console/console.elf";

/// Every manifest that selects the classic `configs/chips/esp32.yaml`.
const MANIFESTS: &[&str] = &[
    "configs/systems/adafruit-feather-esp32-v2.yaml",
    "configs/systems/adafruit-feather-esp32-v2-bme280.yaml",
    "configs/systems/esp32-devkitm-1.yaml",
    "configs/systems/esp32-devkitm-1-bme280.yaml",
    "configs/systems/esp32-wroom-32.yaml",
    "configs/systems/esp32-wroom-epaper.yaml",
    "examples/adafruit-feather-esp32-v2/analog-diode-system.yaml",
    "examples/esp32-bay-occupancy/system.yaml",
    "examples/esp32-bay-occupancy/system-sensor-missing.yaml",
    "examples/esp32-epaper-lab/system.yaml",
    "examples/ili9341-16bit-lab/system.yaml",
    "validation/arduino-matrix/systems/esp32.yaml",
];

/// Run the fixture and return the console capture once `VCP_DONE` is out.
fn console(
    manifest: &labwired_config::SystemManifest,
    chip: &labwired_config::ChipDescriptor,
) -> String {
    let fw = common::committed(FIXTURE);
    let mut s = Session::open(
        BuildRequest {
            chip,
            system: manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &BlobMap::new(),
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .expect("open session");
    let r = s.expect("VCP_DONE\r\n", Duration::from_secs(5));
    let text = s.uart_transcript();
    r.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{text}"));
    text
}

#[test]
fn every_classic_esp32_manifest_names_uart0() {
    for rel in MANIFESTS {
        let (chip, manifest) = common::system(rel);
        assert_eq!(chip.name, "esp32", "{rel}: not the classic chip");
        assert_eq!(manifest.debug_uart.as_deref(), Some("uart0"), "{rel}");
    }
}

/// The run path: the declared console holds UART0 only.
#[test]
fn console_capture_holds_uart0_only() {
    for rel in MANIFESTS {
        let (chip, manifest) = common::system(rel);
        assert_eq!(console(&manifest, &chip), CONSOLE, "{rel}");
    }
}

/// Control: the all-UART fallback these boards used before `debug_uart`
/// splices UART2 into the console.
#[test]
fn all_uart_fallback_interleaves_uart2_into_the_console() {
    let (chip, mut manifest) = common::system("configs/systems/esp32-wroom-32.yaml");
    manifest.debug_uart = None;
    let text = console(&manifest, &chip);
    assert!(
        text.contains('X') && !text.starts_with("VCP_BOOT\r\n"),
        "fallback no longer interleaves UART2: {text:?}"
    );
}
