// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `labwired test` must boot an ESP32-S3 image — every variant, both layouts.
//!
//! `labwired run --chip configs/chips/esp32s3-zero.yaml` printed the whole
//! TIER1 transcript while `labwired test` on the same ELF and the same chip
//! died with a memory violation, so no ESP32-S3 descriptor could be covered by
//! any gate: every gate in this repo runs `labwired test`. Two defects, both
//! invisible to the Arduino matrix's `esp32s3` cell:
//!
//!   1. the S3 machine was selected by an exact chip name (`== "esp32s3"`), so
//!      `esp32s3-zero` — a shipped board variant — took the generic builder,
//!      which loads none of an S3 image's segments. The browser had the right
//!      test (`starts_with("esp32s3")`) in its own copy; the two have been one
//!      predicate, `ChipDescriptor::is_esp32s3`, since.
//!   2. every S3 image then booted on the factory-partition flash layout —
//!      segments stored at `0x10000 + offset` with the flash MMU seeded to
//!      match. That is right for an Arduino/ESP-IDF app booting from `app0`,
//!      and wrong for a bare-metal image linked for the identity XIP windows:
//!      its `.rodata` jump table read back as zero and the firmware jumped to
//!      0x0 at step 48. The Arduino matrix never saw it because its images DO
//!      have a partition table.
//!
//! Both layouts are asserted here. Fixing one while breaking the other is the
//! obvious way to regress this, and a test that only covers the bare-metal case
//! would call that a pass.

use std::path::PathBuf;
use std::process::Command;

fn labwired_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_labwired"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Run `labwired test` on a script written into `dir`, and return its UART.
fn run_script(dir: &std::path::Path, script: &str) -> (bool, String) {
    let script_path = dir.join("script.yaml");
    std::fs::write(&script_path, script).expect("write script");
    let out_dir = dir.join("out");
    let output = Command::new(labwired_bin())
        .arg("test")
        .arg("--script")
        .arg(&script_path)
        .arg("--output-dir")
        .arg(&out_dir)
        .arg("--no-uart-stdout")
        .current_dir(workspace_root())
        .output()
        .expect("spawn labwired test");
    let uart = std::fs::read_to_string(out_dir.join("uart.log")).unwrap_or_default();
    (output.status.success(), uart)
}

/// The S3 TIER1 fixture is a committed blob; a fresh clone without the blobs
/// skips rather than fails, like the rest of the TIER1 harness.
fn fixture() -> Option<PathBuf> {
    let elf = workspace_root().join("tests/fixtures/tier1/esp32s3.elf");
    elf.is_file().then_some(elf)
}

fn tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("labwired-s3-boot-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// A bare chip name, no manifest file. This is the shape a user writes first,
/// and the shape that used to skip the ESP32 path entirely: it required a
/// `system:` path, so `inputs.chip:` fell through to the generic builder.
#[test]
fn test_command_boots_an_s3_board_variant_named_by_chip() {
    let Some(elf) = fixture() else {
        eprintln!("skip: tests/fixtures/tier1/esp32s3.elf not present");
        return;
    };
    let dir = tmp_dir("chip");
    let (ok, uart) = run_script(
        &dir,
        &format!(
            r#"schema_version: "1.0"
inputs:
  firmware: "{}"
  chip: "esp32s3-zero"
limits:
  max_steps: 8000000
assertions:
  - uart_contains: "TIER1 done"
"#,
            elf.display()
        ),
    );
    assert!(
        ok && uart.contains("TIER1 done"),
        "esp32s3-zero named by `inputs.chip` did not reach TIER1 done. UART:\n{uart}"
    );
    // The chip is the variant, not the family: a fix that resolved every S3
    // name to the parent descriptor would pass the line above and still be
    // wrong about which board ran.
    assert!(
        uart.contains("TIER1 gpio PASS"),
        "S3 GPIO never passed on the board variant. UART:\n{uart}"
    );
}

/// The same part through a manifest, which is what the coverage-matrix cell
/// runs. Both entry points build the same machine or one of them is a lie.
#[test]
fn test_command_boots_an_s3_board_variant_through_a_manifest() {
    let Some(elf) = fixture() else {
        eprintln!("skip: tests/fixtures/tier1/esp32s3.elf not present");
        return;
    };
    let dir = tmp_dir("manifest");
    // Script-relative paths resolve against the SCRIPT, which lives in a temp
    // directory here — so name both inputs absolutely.
    let (ok, uart) = run_script(
        &dir,
        &format!(
            r#"schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
limits:
  max_steps: 8000000
assertions:
  - uart_contains: "TIER1 done"
"#,
            elf.display(),
            workspace_root()
                .join("examples/esp32s3-zero/system.yaml")
                .display()
        ),
    );
    assert!(
        ok && uart.contains("TIER1 done"),
        "the committed esp32s3-zero manifest did not reach TIER1 done. UART:\n{uart}"
    );
}

/// The factory-partition layout must survive the bare-metal fix. An S3 image
/// with a partition table beside it is an Arduino/ESP-IDF app booting from
/// `app0`: its XIP segments belong at factory offsets with the MMU seeded, and
/// putting it on identity XIP breaks it exactly as symmetrically as the reverse
/// broke the fixture.
#[test]
fn an_image_with_a_partition_table_keeps_the_factory_layout() {
    let Some(elf) = fixture() else {
        eprintln!("skip: tests/fixtures/tier1/esp32s3.elf not present");
        return;
    };
    let dir = tmp_dir("factory");
    // A partition table beside the firmware is what selects the layout, so
    // copy the fixture next to one. The table's contents do not matter here —
    // only that the CLI sees an app that boots from a partition and lays the
    // flash out for it. The fixture is bare-metal, so it must NOT complete.
    let staged = dir.join("firmware.elf");
    std::fs::copy(&elf, &staged).expect("stage firmware");
    std::fs::write(dir.join("partitions.bin"), [0u8; 0xC00]).expect("stage partition table");
    let (_ok, uart) = run_script(
        &dir,
        &format!(
            r#"schema_version: "1.0"
inputs:
  firmware: "{}"
  chip: "esp32s3-zero"
limits:
  max_steps: 200000
assertions:
  - uart_contains: "TIER1 done"
"#,
            staged.display()
        ),
    );
    assert!(
        !uart.contains("TIER1 done"),
        "a bare-metal image ran to completion on the FACTORY layout, so the \
         partition table no longer selects a layout at all — the Arduino S3 \
         path this protects is unprotected. UART:\n{uart}"
    );
}
