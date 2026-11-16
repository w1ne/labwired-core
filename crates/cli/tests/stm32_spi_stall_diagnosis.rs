// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A step-budget run that ends in an SPI status wait names what it waits on
//! (issue #1950).
//!
//! Both fixtures are STM32duino sketches for the F411 Black Pill that spin in
//! `spi_transfer()` on `SR.TXE`; see the `.ino` next to each ELF.
//!
//! - `begin_noop`: Arduino core 3.0.0 makes `SPI.begin()` a no-op, so the
//!   handle's `Instance` is NULL and the loop reads `0x00000008` (the vector
//!   table), not SPI1. No MMIO happens, so `stalled_poll` cannot see it;
//!   `stalled_null_read` must.
//! - `clock_gated`: SPI1 is set up, then its RCC clock is gated off. Gated
//!   reads return 0 without reaching the model, and they used to bypass the
//!   poll-streak bookkeeping too, so the run reported no stall at all.

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

fn run(fixture: &str) -> serde_json::Value {
    let root = workspace_root();
    let dir = labwired_cli::test_support::unique_temp_dir(&format!("labwired-spi-stall-{fixture}"));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("script.yaml");
    std::fs::write(
        &script,
        format!(
            "schema_version: \"1.0\"\ninputs:\n  firmware: \"{}\"\n  system: \"{}\"\nlimits:\n  max_steps: 2000000\n",
            root.join(format!(
                "crates/cli/tests/fixtures/stm32f411_spi_stall/{fixture}.elf"
            ))
            .display(),
            root.join("configs/systems/stm32f411ceu6-blackpill.yaml")
                .display(),
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
    let result = std::fs::read_to_string(dir.join("result.json"))
        .unwrap_or_else(|_| panic!("no result.json; stderr:\n{stderr}"));
    let _ = std::fs::remove_dir_all(&dir);
    serde_json::from_str(&result).unwrap()
}

#[test]
fn a_null_spi_handle_wait_is_named_as_a_null_read() {
    let r = run("begin_noop");
    let m = &r["metrics"];
    assert_eq!(
        m["stalled_poll"],
        serde_json::Value::Null,
        "the loop does no MMIO"
    );
    let null = &m["stalled_null_read"];
    assert_eq!(null["address"], "0x00000008", "NULL + SPI_SR offset: {m}");
    let detail = null["detail"].as_str().unwrap_or_default();
    assert!(detail.contains("NULL + 0x08"), "{detail}");
}

#[test]
fn a_clock_gated_spi_status_wait_names_the_register() {
    let r = run("clock_gated");
    let m = &r["metrics"];
    let stall = &m["stalled_poll"];
    assert_eq!(stall["peripheral"], "spi1", "{m}");
    assert_eq!(stall["offset"], 8);
    assert_eq!(stall["register"], "SR");
    assert_eq!(stall["clock_gated"], true);
    let detail = stall["detail"].as_str().unwrap_or_default();
    assert!(detail.contains("spi1 SR (0x08)"), "{detail}");
    assert!(detail.contains("clock-gated"), "{detail}");
    assert_eq!(
        m["stalled_null_read"],
        serde_json::Value::Null,
        "SPI1 is a real peripheral pointer"
    );
}
