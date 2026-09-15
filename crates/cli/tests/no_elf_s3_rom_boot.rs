// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! HARD GATE for the ELF-less ESP32-S3 rom-boot path — the S3 twin of
//! `no_elf_c3_rom_boot.rs`.
//!
//! For rom-boot chips the hosted compile deliberately ships the flash image but
//! NO firmware ELF (a multi-MB debug ELF overflows the D1 blob row →
//! SQLITE_TOOBIG). The builder therefore invokes `labwired test --rom-boot` with
//! `LABWIRED_ESP32S3_FLASH` set and NO `--firmware`/`inputs.firmware`.
//!
//! The C3 accepted that request; the S3 refused it with
//! `Missing firmware path (provide --firmware or set inputs.firmware in script)`
//! and `status: config_error` at 0 steps, because the ELF-less dispatch was
//! gated on `LABWIRED_ESP32C3_FLASH` + an `esp32c3` chip name. An ELF-less
//! hosted S3 rom-boot was therefore dead in production.
//!
//! Nothing about the S3 machine needed the ELF: the ELF-bearing `--rom-boot`
//! arm in `commands/test.rs` builds it from `configure_xtensa_esp32s3` +
//! `esp32s3_rom::provision_rom_images()` (mask ROM from env pins / the
//! toolchain's ROM ELF / the vendored images) and the flash image, and uses the
//! app ELF only for symbol diagnostics.
//!
//! ── Why this test stops where it stops ──────────────────────────────────
//!
//! It asserts the EARLIEST line that proves the ELF-less request was accepted
//! AND that code fetched from the flash image is executing: the 2nd-stage
//! bootloader's own `ESP-IDF ... 2nd stage bootloader` banner. Everything
//! before it (`ESP-ROM:esp32s3-20210327`, `load:0x...`, `entry 0x403c8924`) is
//! printed by the mask ROM; the banner is the first byte emitted by code the
//! mask ROM read out of `LABWIRED_ESP32S3_FLASH` and jumped to. That is the
//! whole claim — "the flash image is the program", with no ELF present.
//!
//! It deliberately does NOT run the app to completion. Like the C3, this boot
//! is CONSOLE-BOUND, not compute-bound: `EspUart` models the real shift-out
//! rate, so the transcript costs cycles by the byte. The sibling
//! `no_elf_c3_rom_boot` runs 32M steps for its app-level marker and takes ~423 s
//! of `pr-workspace-tests` wall-clock — the single largest item in that lane. A
//! second test of that size is not acceptable, and is not needed: the app-level
//! S3 boot is already covered by the TIER1 matrix (`tier1.rs`, 30M steps,
//! ELF-bearing). What is uncovered, and what this gates, is the ELF-LESS
//! request shape.
//!
//! MEASURED against the committed tier1 flash fixture on this tree: the banner
//! lands between 3M and 4M steps; 5M is that plus ~38 % headroom and the run
//! still stops on `max_steps` (476 UART bytes, ~3 s wall-clock).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Step budget that reaches the 2nd-stage bootloader banner. See the module
/// comment for the measurement.
const BOOTLOADER_BANNER_MAX_STEPS: u64 = 5_000_000;

/// The first console bytes emitted by code loaded from the flash image.
const FLASH_BOOT_MARKER: &str = "2nd stage bootloader";

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

/// Write a `labwired test` script that names a chip + limits + assertions but
/// deliberately sets an EMPTY firmware input (the schema requires the key; the
/// CLI filters empty and takes the ELF-less rom-boot path). Returns its path.
///
/// `inputs.chip` rather than `inputs.system` on purpose: it is the shape with
/// no manifest file behind it, so it also covers the `sys_anchor` fallback the
/// S3 machine build needs.
fn write_no_firmware_script(dir: &Path) -> PathBuf {
    let script = format!(
        "schema_version: \"1.0\"\n\
         inputs:\n  \
           firmware: \"\"\n  \
           chip: \"esp32s3\"\n\
         limits:\n  \
           max_steps: {BOOTLOADER_BANNER_MAX_STEPS}\n\
         assertions:\n  \
           - expected_stop_reason: max_steps\n  \
           - uart_contains: \"{FLASH_BOOT_MARKER}\"\n",
    );
    let path = dir.join("no_firmware_romboot_s3.yaml");
    std::fs::write(&path, script).expect("write test script");
    path
}

#[test]
fn s3_rom_boot_runs_with_flash_and_no_elf() {
    let root = repo_root();
    // The committed TIER1 rom-boot fixture (bootloader + partition table + app),
    // the same flash image `tier1::run_target` boots the S3 row from.
    let flash = require(root.join("tests/fixtures/tier1/esp32s3-flash.bin"));

    let tmp = std::env::temp_dir().join(format!("lw-no-elf-s3-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create tmp dir");
    let script = write_no_firmware_script(&tmp);
    let out_dir = tmp.join("out");
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    // Invoke exactly as the builder does on the rom-boot path: `--rom-boot`, the
    // flash image via the env pin, and NO `--firmware`. The boot ROM
    // auto-provisions from the vendored images (crates/core/roms/esp32s3/).
    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(&root)
        .env("LABWIRED_ESP32S3_FLASH", &flash)
        .env_remove("LABWIRED_ESP32C3_FLASH")
        .env_remove("LABWIRED_ESP32S3_FASTBOOT")
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
        "expected the ELF-less S3 rom-boot branch to run; stderr:\n{stderr}"
    );

    // result.json must exist — proving the sim actually ran (a config error before
    // the sim starts writes it via write_config_error_outputs, so we ALSO assert
    // the run succeeded + the flash-loaded bootloader spoke below).
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
        "ELF-less S3 rom-boot run failed (exit {:?}); the flash-boot assertion did not pass.\n\
         result.json:\n{result_json}\nstderr:\n{stderr}",
        output.status.code(),
    );

    // Belt and braces: the marker must be in the captured UART, not merely in a
    // passing-assertion summary.
    let uart = std::fs::read_to_string(out_dir.join("uart.log")).expect("read uart.log");
    assert!(
        uart.contains(FLASH_BOOT_MARKER),
        "no `{FLASH_BOOT_MARKER}` in the console — the mask ROM did not hand off to \
         code loaded from the flash image.\nuart.log:\n{uart}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
