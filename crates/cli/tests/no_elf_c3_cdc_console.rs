// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! HARD GATE: ELF-less C3 rom-boot captures Arduino USB-CDC `Serial` even when
//! the system yaml does NOT declare `debug_uart: usb_serial_jtag`.
//!
//! That is the hosted playground shape. `compile()` historically omitted the
//! key; the builder invokes `labwired test --rom-boot` with `firmware: ""` and
//! that yaml. Arduino on a native-USB C3 is built with
//! `-DARDUINO_USB_CDC_ON_BOOT=1`, so `Serial` is HWCDC (USB-Serial-JTAG), not
//! UART0. Tapping only UART0 yields the ROM/bootloader banner and an empty
//! app console — GPIO still toggles, `uart_contains` never sees the sketch.
//!
//! The ELF-bearing `test` path always mirrors CDC into `uart_tx`. This gate
//! locks the ELF-less arm to the same rule when `debug_uart` is undeclared.
//! (Wasm C3 ROM-boot does NOT mix taps: undeclared means UART0 heard, CDC
//! unheard. Do not cargo-cult this mix onto `attach_c3_flash_console`.)
//!
//! The flash image is `platformio/esp32c3-usb-cdc-console` (Serial.begin +
//! `LW_CDC_SETUP` / `LW_CDC_LOOP N`).

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Cold rom-boot of the CDC console fixture prints `LW_CDC_LOOP 1` by 18.8M
/// steps / 50M cycles (measured). 32M steps is that number plus headroom;
/// `stop_when_assertions_pass` should halt far earlier once the sink is tapped.
const CDC_MAX_STEPS: u64 = 32_000_000;

fn write_hosted_shape_script(dir: &Path, system: &Path) -> PathBuf {
    let script = format!(
        "schema_version: \"1.0\"\n\
         inputs:\n  \
           firmware: \"\"\n  \
           system: \"{}\"\n\
         limits:\n  \
           max_steps: {CDC_MAX_STEPS}\n  \
           stop_when_assertions_pass: true\n\
         assertions:\n  \
           - uart_contains: \"LW_CDC_SETUP\"\n  \
           - uart_contains: \"LW_CDC_LOOP\"\n  \
           - expected_stop_reason: assertions_passed\n",
        system.display(),
    );
    let path = dir.join("no_elf_c3_cdc.yaml");
    std::fs::write(&path, script).expect("write test script");
    path
}

#[test]
fn c3_elf_less_rom_boot_captures_cdc_without_debug_uart() {
    let root = repo_root();
    let flash = require(root.join("crates/core/tests/fixtures/esp32c3-usb-cdc-console-flash.bin"));
    // Hosted-like: cpu_hz, board_io, NO debug_uart. Same shape the playground
    // emitter produced before it started declaring the USB console.
    let system = require(root.join("configs/systems/esp32c3-devkit.yaml"));
    assert!(
        !yaml_declares_debug_uart(&std::fs::read_to_string(&system).expect("read system yaml")),
        "esp32c3-devkit.yaml grew a debug_uart: key; this gate needs an undeclared console"
    );

    let tmp = std::env::temp_dir().join(format!("lw-no-elf-c3-cdc-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create tmp dir");
    let script = write_hosted_shape_script(&tmp, &system);
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
    assert!(
        stderr.contains("ELF-less"),
        "expected the ELF-less C3 rom-boot branch to run; stderr:\n{stderr}"
    );

    let result_path = out_dir.join("result.json");
    let result_json = std::fs::read_to_string(&result_path).unwrap_or_else(|e| {
        panic!(
            "no result.json at {} (exit {:?}): {e}\nstderr:\n{stderr}",
            result_path.display(),
            output.status.code(),
        )
    });

    let uart = std::fs::read_to_string(out_dir.join("uart.log")).unwrap_or_default();
    assert!(
        output.status.success(),
        "ELF-less C3 CDC run failed (exit {:?}); USB-Serial-JTAG was not tapped \
         on an undeclared console.\nresult.json:\n{result_json}\nuart.log:\n{uart}\nstderr:\n{stderr}",
        output.status.code(),
    );
    assert!(
        uart.contains("LW_CDC_SETUP") && uart.contains("LW_CDC_LOOP"),
        "assertions passed but uart.log is missing the CDC markers:\n{uart}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

fn yaml_declares_debug_uart(yaml: &str) -> bool {
    yaml.lines()
        .any(|l| l.trim_start().starts_with("debug_uart:"))
}

fn write_uart0_system(dir: &Path, chip: &Path) -> PathBuf {
    let yaml = format!(
        "name: \"playground-board\"\n\
         chip: \"{}\"\n\
         cpu_hz: 160_000_000\n\
         debug_uart: \"uart0\"\n\
         external_devices: []\n\
         board_io: []\n",
        chip.display(),
    );
    let path = dir.join("uart0.yaml");
    std::fs::write(&path, yaml).expect("write uart0 system yaml");
    path
}

/// Long enough that a CDC tap would have printed `LW_CDC_SETUP` (the undeclared
/// gate stops around 3.8M interpreted steps). Not the full 32M budget.
const UART0_MAX_STEPS: u64 = 8_000_000;

fn write_max_steps_script(dir: &Path, system: &Path) -> PathBuf {
    let script = format!(
        "schema_version: \"1.0\"\n\
         inputs:\n  \
           firmware: \"\"\n  \
           system: \"{}\"\n\
         limits:\n  \
           max_steps: {UART0_MAX_STEPS}\n\
         assertions:\n  \
           - expected_stop_reason: max_steps\n",
        system.display(),
    );
    let path = dir.join("uart0_cdc.yaml");
    std::fs::write(&path, script).expect("write uart0 script");
    path
}

/// Bridge-chip boards declare UART0. Mixing CDC into `uart_tx` would hide a
/// real wiring fact (Serial on the wrong console). The same CDC-on-boot image
/// must then leave `LW_CDC_SETUP` out of uart.log.
#[test]
fn c3_elf_less_rom_boot_does_not_mix_cdc_when_debug_uart_is_uart0() {
    let root = repo_root();
    let flash = require(root.join("crates/core/tests/fixtures/esp32c3-usb-cdc-console-flash.bin"));
    let chip = require(root.join("configs/chips/esp32c3.yaml"));

    let tmp = std::env::temp_dir().join(format!("lw-no-elf-c3-uart0-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create tmp dir");
    let system = write_uart0_system(&tmp, &chip);
    let script = write_max_steps_script(&tmp, &system);
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
    assert!(
        stderr.contains("ELF-less"),
        "expected the ELF-less C3 rom-boot branch to run; stderr:\n{stderr}"
    );
    assert!(
        output.status.success(),
        "uart0 run itself failed (exit {:?}); stderr:\n{stderr}",
        output.status.code(),
    );

    let uart = std::fs::read_to_string(out_dir.join("uart.log")).unwrap_or_default();
    assert!(
        uart.contains("ESP-ROM:esp32c3"),
        "uart0 run captured no UART0 ROM banner; the sink was not UART0:\n{uart}"
    );
    assert!(
        !uart.contains("LW_CDC_SETUP") && !uart.contains("LW_CDC_LOOP"),
        "explicit debug_uart: uart0 mixed USB-CDC into uart.log; bridge-chip \
         boards must not see HWCDC on the assertion buffer:\n{uart}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
