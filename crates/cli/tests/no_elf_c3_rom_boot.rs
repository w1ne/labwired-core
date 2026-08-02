// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! HARD GATE for the ELF-less ESP32-C3 rom-boot path used by external agents via
//! the LabWired MCP (`labwired_run` / `labwired_verify`).
//!
//! For rom-boot chips the hosted compile deliberately ships the flash images but
//! NO firmware ELF (a multi-MB debug ELF overflows the D1 blob row →
//! SQLITE_TOOBIG). The builder therefore invokes `labwired test --rom-boot` with
//! `LABWIRED_ESP32C3_FLASH` set and NO `--firmware`/`inputs.firmware`. Before the
//! fix this 500'd: `run_test` unconditionally required firmware.
//!
//! This test drives the REAL `labwired` binary (the same one the builder runs) on
//! the curated `esp32c3-oled-demo` flash image with NO ELF and asserts the app
//! actually booted from flash and painted the OLED — proving "the flash image is
//! the program the real ROM loads" without any ELF present.

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

/// Step budget that reaches `OLED painted: LabWired` on the console.
///
/// This run is CONSOLE-BOUND, not compute-bound, and that is real silicon
/// behaviour. The curated image is a stock ESP-IDF v5.3.1 app: the mask ROM,
/// the 2nd-stage bootloader and `cpu_start`/`app_init`/`heap_init` all log at
/// INFO, so 3519 bytes have to shift out of UART0 before `app_main` even gets
/// to print the paint line. UART0 comes out of reset at CLKDIV=694 (~115200
/// baud off the 80 MHz APB source), i.e. 10 * 694 * 160 MHz / 80 MHz = 13_880
/// CPU cycles per byte — so the transcript alone costs ~48.8M of the run's
/// cycles. The ROM/IDF console does the flow control silicon requires (it polls
/// `STATUS.TXFIFO_CNT` before every byte), so nothing is dropped; the bytes
/// take wire time. `EspUart` (`crates/core/src/peripherals/esp_uart.rs`) models
/// that shift-out rate, which the STM32-shaped `Uart` the C3 used before
/// 15b96281 did not — it accepted every byte instantly, which is why the
/// original 8M budget looked sufficient.
///
/// MEASURED against the committed flash fixture on this tree: the marker leaves
/// the TX shift register at 24_439_616 steps / 61_675_548 cycles (~385 ms of
/// device time). 32M steps is that number plus ~31 % headroom, and the firmware
/// then spins in its 500 ms refresh loop, so the run still stops on `max_steps`
/// (verified: 32_000_000 steps / 82_487_648 cycles, `stop_reason: max_steps`).
const OLED_PAINT_MAX_STEPS: u64 = 32_000_000;

/// Write a `labwired test` script that names a system + limits + assertions but
/// deliberately sets an EMPTY firmware input (the schema requires the key; the
/// CLI filters empty and takes the ELF-less rom-boot path). Returns its path.
fn write_no_firmware_script(dir: &Path, system: &Path) -> PathBuf {
    let script = format!(
        "schema_version: \"1.0\"\n\
         inputs:\n  \
           firmware: \"\"\n  \
           system: \"{}\"\n\
         limits:\n  \
           max_steps: {OLED_PAINT_MAX_STEPS}\n\
         assertions:\n  \
           - expected_stop_reason: max_steps\n  \
           - uart_contains: \"OLED painted: LabWired\"\n",
        system.display(),
    );
    let path = dir.join("no_firmware_romboot.yaml");
    std::fs::write(&path, script).expect("write test script");
    path
}

#[test]
fn c3_rom_boot_runs_with_flash_and_no_elf() {
    let root = repo_root();
    // The curated OLED-demo flash image (bootloader + partition table + app) the
    // browser fast-start and the JIT differential gate both boot from.
    let flash = require(root.join("crates/wasm/tests/fixtures/esp32c3-oled-demo-flash.bin"));
    let system = require(root.join("configs/systems/esp32c3-oled-demo.yaml"));

    let tmp = std::env::temp_dir().join(format!("lw-no-elf-c3-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create tmp dir");
    let script = write_no_firmware_script(&tmp, &system);
    let out_dir = tmp.join("out");
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    // Invoke exactly as the builder does on the rom-boot path: `--rom-boot`, the
    // flash image via the env pin, and NO `--firmware`. The boot ROM
    // auto-provisions from the vendored images (crates/core/roms/esp32c3/)
    // resolved relative to the repo-root CWD.
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

    // The ELF-less branch must have been taken (not a silent firmware fallback).
    assert!(
        stderr.contains("ELF-less"),
        "expected the ELF-less C3 rom-boot branch to run; stderr:\n{stderr}"
    );

    // result.json must exist — proving the sim actually ran (a config error before
    // the sim starts writes it via write_config_error_outputs, so we ALSO assert
    // the run succeeded + the app painted below).
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
        "run ended in a config_error (firmware still required?):\n{result_json}\nstderr:\n{stderr}"
    );
    assert!(
        output.status.success(),
        "ELF-less rom-boot run failed (exit {:?}); the assertions (OLED paint) did not pass.\n\
         result.json:\n{result_json}\nstderr:\n{stderr}",
        output.status.code(),
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
