// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! HARD GATE for the ELF-less ESP32-S3 rom-boot path used by external agents via
//! the LabWired MCP (`labwired_run` / `labwired_verify`).
//!
//! Same contract as `no_elf_c3_rom_boot.rs`: for rom-boot chips the hosted
//! compile deliberately ships the flash images but NO firmware ELF (a multi-MB
//! debug ELF overflows the D1 blob row → SQLITE_TOOBIG). The builder therefore
//! invokes `labwired test --rom-boot` with `LABWIRED_ESP32S3_FLASH` set and NO
//! `--firmware`/`inputs.firmware`. Before the fix the ELF-less gate was C3-only,
//! so every S3 device build died at configuration time with
//! `Missing firmware path`.
//!
//! This test drives the REAL `labwired` binary on the committed TIER1 S3 flash
//! image with NO ELF and asserts the app actually booted from flash and reached
//! its final protocol line — proving the S3 mask ROM loads the flash image as
//! the program without any ELF present.

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

/// Step budget that reaches `TIER1 done` on the console. The TIER1 matrix runs
/// this same flash image with a 30M budget (the full transcript lands between
/// 16M and 24M steps: ROM + bootloader + app bring-up dominate, and the UART
/// wire time is real). The firmware then spins, so the run stops on `max_steps`.
const TIER1_S3_MAX_STEPS: u64 = 30_000_000;

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
           max_steps: {TIER1_S3_MAX_STEPS}\n\
         assertions:\n  \
           - expected_stop_reason: max_steps\n  \
           - uart_contains: \"TIER1 done\"\n",
        system.display(),
    );
    let path = dir.join("no_firmware_romboot_s3.yaml");
    std::fs::write(&path, script).expect("write test script");
    path
}

#[test]
fn s3_rom_boot_runs_with_flash_and_no_elf() {
    let root = repo_root();
    // The committed TIER1 S3 flash image (bootloader + partition table + app) the
    // tier1 matrix boots with an ELF; here it must boot with NO ELF.
    let flash = require(root.join("tests/fixtures/tier1/esp32s3-flash.bin"));
    let system = require(root.join("configs/systems/esp32s3-zero.yaml"));

    let tmp = std::env::temp_dir().join(format!("lw-no-elf-s3-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create tmp dir");
    let script = write_no_firmware_script(&tmp, &system);
    let out_dir = tmp.join("out");
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    // Invoke exactly as the builder does on the rom-boot path: `--rom-boot`, the
    // flash image via the env pin, and NO `--firmware`. The boot ROM
    // auto-provisions from the vendored images (crates/core/roms/esp32s3/)
    // resolved relative to the core crate.
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
    // the run succeeded + the app reached its final line below).
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
        "ELF-less S3 rom-boot run failed (exit {:?}); the assertions (TIER1 done) did not pass.\n\
         result.json:\n{result_json}\nstderr:\n{stderr}",
        output.status.code(),
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
