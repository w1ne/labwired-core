// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `build_machine` on Xtensa: the three boot paths the browser constructor has,
//! each driven through a `Session` and judged by what the firmware prints.
//!
//! * ESP32-S3, `Elf` + `FastBoot` — the Tier-1 S3 fixture script;
//! * ESP32-S3, `FlashImage` + `RomBoot` — the mask ROM boots the merged flash
//!   image with no ELF, the path every hosted S3 run takes;
//! * ESP32 (classic), `Elf` + `FastBoot` — dual-core, both stacks seeded.
//!
//! Every input is a committed file (Tier-1 ELFs and flash image, the vendored
//! S3 ROM), so none of these tests can skip. Deliberately NOT behind
//! `esp32s3-fixtures`: that feature is for tests that build firmware with the
//! +esp toolchain, it is in no CI feature set, and nothing here needs it.

mod common;

use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use std::time::Duration;

/// The vendored ESP32-S3 mask ROM under the blob names the builder reads.
fn s3_rom_blobs() -> BlobMap {
    let mut blobs = BlobMap::new();
    blobs.insert(
        "esp32s3_irom".into(),
        common::committed("crates/core/roms/esp32s3/esp32s3_rom.bin"),
    );
    blobs.insert(
        "esp32s3_drom".into(),
        common::committed("crates/core/roms/esp32s3/esp32s3_drom.bin"),
    );
    blobs
}

fn open(
    chip: &labwired_config::ChipDescriptor,
    manifest: &labwired_config::SystemManifest,
    firmware: FirmwareSource<'_>,
    boot: BootMode,
    blobs: &BlobMap,
) -> anyhow::Result<Session> {
    Session::open(
        BuildRequest {
            chip,
            system: manifest,
            firmware,
            boot,
            blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
}

/// `expect` each marker in order, printing the console on a miss.
fn expect_all(s: &mut Session, markers: &[&str], each: Duration) {
    for marker in markers {
        let r = s.expect(&regex::escape(marker), each);
        let transcript = s.uart_transcript();
        r.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{transcript}"));
    }
}

fn assert_not_supported(err: anyhow::Error) {
    let msg = format!("{err:#}");
    assert!(
        msg.contains("not supported"),
        "expected a not-supported error, got: {msg}"
    );
}

#[test]
fn esp32s3_fixture_prints_expected_uart_via_build_machine() {
    // `examples/esp32s3-zero/tier1-smoke.yaml`: the CLI gate on the S3 family.
    // Its firmware is committed, so `script_fixture` cannot return `None`.
    let f = common::script_fixture(
        "examples/esp32s3-zero/tier1-smoke.yaml",
        "tier1-esp32s3",
        "committed fixture; restore tests/fixtures/tier1/esp32s3.elf",
    )
    .expect("the S3 Tier-1 fixture is committed");
    let blobs = BlobMap::new();
    let mut s = open(
        &f.chip,
        &f.manifest,
        FirmwareSource::Elf(&f.firmware),
        BootMode::FastBoot,
        &blobs,
    )
    .unwrap();
    let markers: Vec<&str> = f.uart_contains.iter().map(String::as_str).collect();
    expect_all(&mut s, &markers, Duration::from_millis(200));
}

#[test]
fn esp32s3_flash_image_rom_boot_reaches_the_partition_table_without_an_elf() {
    let (chip, manifest) = common::system("configs/systems/esp32s3-zero.yaml");
    let flash = common::committed("tests/fixtures/tier1/esp32s3-flash.bin");
    let blobs = s3_rom_blobs();
    let mut s = open(
        &chip,
        &manifest,
        FirmwareSource::FlashImage {
            image: &flash,
            symbols: None,
        },
        BootMode::RomBoot,
        &blobs,
    )
    .unwrap();
    // The ROM banner proves the reset vector ran the injected mask ROM. The
    // bootloader banner proves the ROM read the 2nd stage out of the flash image
    // this request carried. "Partition Table:" is printed only after the
    // bootloader `bootloader_mmap`s the table at 0x8000 and verifies it, which
    // reads through the MMU XIP model `real_reset_boot` selects; identity XIP
    // serves the wrong page there.
    //
    // It stops short of "Loaded app from partition" on cost: that line is ~14.5M
    // cycles in, where "Partition Table:" is ~5.8M, and this path steps slowly
    // in the debug test profile.
    expect_all(
        &mut s,
        &[
            "ESP-ROM:esp32s3",
            "2nd stage bootloader",
            "Partition Table:",
        ],
        Duration::from_millis(100),
    );
}

#[test]
fn esp32_classic_fixture_runs_the_tier1_classes_on_two_cores() {
    let (chip, manifest) = common::system("configs/systems/esp32-wroom-32.yaml");
    let fw = common::committed("tests/fixtures/tier1/esp32.elf");
    let blobs = BlobMap::new();
    let mut s = open(
        &chip,
        &manifest,
        FirmwareSource::Elf(&fw),
        BootMode::FastBoot,
        &blobs,
    )
    .unwrap();
    expect_all(&mut s, &["TIER1 done"], Duration::from_millis(100));
    // Every class passes except `dma`, which the fixture reports as an honest
    // gap on this silicon (no general-purpose mem-to-mem DMA on the classic
    // ESP32; the Tier-1 matrix renders that cell `na`).
    const DMA_GAP: &str = "TIER1 dma FAIL code=esp32-no-mem2mem-dma";
    let transcript = s.uart_transcript();
    let classes: Vec<&str> = transcript
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("TIER1 ") && *l != "TIER1 done")
        .collect();
    assert!(
        classes.len() > 1 && classes.contains(&DMA_GAP),
        "expected the Tier-1 class lines including the dma gap:\n{transcript}"
    );
    for line in classes {
        assert!(
            line == DMA_GAP || line.ends_with(" PASS"),
            "Tier-1 class did not pass: {line}\n--- console ---\n{transcript}"
        );
    }
}

#[test]
fn esp32s3_rom_boot_without_rom_blobs_names_the_missing_blobs() {
    let (chip, manifest) = common::system("configs/systems/esp32s3-zero.yaml");
    let flash = common::committed("tests/fixtures/tier1/esp32s3-flash.bin");
    let err = open(
        &chip,
        &manifest,
        FirmwareSource::FlashImage {
            image: &flash,
            symbols: None,
        },
        BootMode::RomBoot,
        &BlobMap::new(),
    )
    .err()
    .expect("S3 ROM boot needs the mask ROM");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("esp32s3_irom") && msg.contains("esp32s3_drom"),
        "error should name the blobs: {msg}"
    );
}

#[test]
fn esp32s3_rom_boot_from_an_elf_is_refused() {
    let (chip, manifest) = common::system("configs/systems/esp32s3-zero.yaml");
    let fw = common::committed("tests/fixtures/tier1/esp32s3.elf");
    let err = open(
        &chip,
        &manifest,
        FirmwareSource::Elf(&fw),
        BootMode::RomBoot,
        &s3_rom_blobs(),
    )
    .err()
    .expect("ROM boot needs a flash image");
    assert!(
        format!("{err:#}").contains("flash image"),
        "error should say what ROM boot needs: {err:#}"
    );
}

#[test]
fn unmodelled_xtensa_boot_paths_are_not_supported() {
    let (s3_chip, s3_manifest) = common::system("configs/systems/esp32s3-zero.yaml");
    let flash = common::committed("tests/fixtures/tier1/esp32s3-flash.bin");
    let image = FirmwareSource::FlashImage {
        image: &flash,
        symbols: None,
    };
    // The S3 has no fast boot from a merged flash image: the image boots
    // through the mask ROM or not at all.
    assert_not_supported(
        open(
            &s3_chip,
            &s3_manifest,
            image,
            BootMode::FastBoot,
            &s3_rom_blobs(),
        )
        .err()
        .expect("S3 flash fast boot"),
    );

    let (chip, manifest) = common::system("configs/systems/esp32-wroom-32.yaml");
    let elf = common::committed("tests/fixtures/tier1/esp32.elf");
    for (firmware, boot) in [
        (
            FirmwareSource::FlashImage {
                image: &flash,
                symbols: None,
            },
            BootMode::FastBoot,
        ),
        (
            FirmwareSource::FlashImage {
                image: &flash,
                symbols: None,
            },
            BootMode::RomBoot,
        ),
        (FirmwareSource::Elf(&elf), BootMode::RomBoot),
    ] {
        assert_not_supported(
            open(&chip, &manifest, firmware, boot, &BlobMap::new())
                .err()
                .unwrap_or_else(|| panic!("classic ESP32 {boot:?} is not modelled")),
        );
    }
}
