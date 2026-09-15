// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `build_machine` on RISC-V: every boot path the browser constructor has, each
//! driven through a `Session` and judged by what the firmware prints.
//!
//! * `Elf` + `FastBoot` — a bare ELF, optionally with the ESP32-C3 mask ROM
//!   blobs injected (the C3 then runs esp-hal's ROM calls for real);
//! * `FlashImage` + `RomBoot` — the C3 mask ROM runs from its reset vector and
//!   loads the 2nd-stage bootloader out of the merged flash image;
//! * `FlashImage` + `FastBoot` — the ROM replay is skipped and the bootloader
//!   is entered directly.
//!
//! Every input is a committed file (fixture ELFs, flash images, the vendored
//! C3 ROM), so none of these tests can skip.

mod common;

use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use std::time::Duration;

/// The vendored ESP32-C3 mask ROM under the blob names the builder reads.
fn c3_rom_blobs() -> BlobMap {
    let mut blobs = BlobMap::new();
    blobs.insert(
        "esp32c3_irom".into(),
        common::committed("crates/core/roms/esp32c3/esp32c3_rom.bin"),
    );
    blobs.insert(
        "esp32c3_drom".into(),
        common::committed("crates/core/roms/esp32c3/esp32c3_drom.bin"),
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

#[test]
fn riscv_fixture_prints_expected_uart_via_build_machine() {
    // Board and expected text come from the script `ci-fixture-riscv/ci/test.sh`
    // runs. Its `inputs.firmware` is a riscv32i build that no test lane in this
    // repo produces (core-ci builds only the ARM fixtures), so the firmware is
    // the committed build of the same crate that `firmware_survival` and
    // `examples/ci-multiarch` boot — a test bound to the unbuilt path would skip
    // on every lane.
    let (chip, manifest, expected) = common::script_board("examples/ci/riscv-uart-ok.yaml");
    let fw = common::committed("tests/fixtures/riscv-ci-fixture.elf");
    let blobs = BlobMap::new();
    let mut s = open(
        &chip,
        &manifest,
        FirmwareSource::Elf(&fw),
        BootMode::FastBoot,
        &blobs,
    )
    .unwrap();
    for text in &expected {
        s.expect(&regex::escape(text), Duration::from_secs(5))
            .unwrap();
    }
}

#[test]
fn esp32c3_elf_with_rom_blobs_runs_the_tier1_fixture() {
    // The C3 Tier-1 fixture on the devkit board, with the real mask ROM
    // injected: the faithful-ROM branch of the plain constructor (ROM `.data`
    // replay, analog I2C master, USB-Serial-JTAG model).
    let (chip, manifest) = common::system("configs/systems/esp32c3-devkit.yaml");
    let fw = common::committed("tests/fixtures/tier1/esp32c3.elf");
    let blobs = c3_rom_blobs();
    let mut s = open(
        &chip,
        &manifest,
        FirmwareSource::Elf(&fw),
        BootMode::FastBoot,
        &blobs,
    )
    .unwrap();
    let done = s.expect("TIER1 done", Duration::from_millis(200));
    let transcript = s.uart_transcript();
    done.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{transcript}"));
    assert!(
        !transcript.contains("FAIL"),
        "a Tier-1 class failed:\n{transcript}"
    );
}

#[test]
fn esp32c3_flash_image_rom_boot_runs_the_mask_rom_into_the_bootloader() {
    let (chip, manifest) = common::system("configs/systems/esp32c3-devkit.yaml");
    let flash = common::committed("crates/wasm/tests/fixtures/esp32c3-hello-world-flash.bin");
    let blobs = c3_rom_blobs();
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
    // The ROM banner proves the reset vector ran the injected mask ROM; the
    // bootloader banner proves the ROM read the 2nd stage out of the flash
    // image this request carried.
    for marker in ["ESP-ROM:esp32c3", "2nd stage bootloader"] {
        let r = s.expect(marker, Duration::from_millis(500));
        let transcript = s.uart_transcript();
        r.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{transcript}"));
    }
}

#[test]
fn esp32c3_flash_image_fast_boot_enters_the_bootloader_without_the_rom() {
    let (chip, manifest) = common::system("configs/systems/esp32c3-devkit.yaml");
    let flash = common::committed("crates/wasm/tests/fixtures/esp32c3-hello-world-flash.bin");
    let blobs = c3_rom_blobs();
    let mut s = open(
        &chip,
        &manifest,
        FirmwareSource::FlashImage {
            image: &flash,
            symbols: None,
        },
        BootMode::FastBoot,
        &blobs,
    )
    .unwrap();
    let r = s.expect("2nd stage bootloader", Duration::from_millis(500));
    let transcript = s.uart_transcript();
    r.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{transcript}"));
    assert!(
        !transcript.contains("ESP-ROM:"),
        "fast boot must skip the mask ROM replay, but the ROM banner printed:\n{transcript}"
    );
}

#[test]
fn rom_boot_from_an_elf_is_refused() {
    let (chip, manifest) = common::system("configs/systems/esp32c3-devkit.yaml");
    let fw = common::committed("tests/fixtures/tier1/esp32c3.elf");
    let blobs = c3_rom_blobs();
    let err = open(
        &chip,
        &manifest,
        FirmwareSource::Elf(&fw),
        BootMode::RomBoot,
        &blobs,
    )
    .err()
    .expect("ROM boot needs a flash image");
    assert!(
        format!("{err:#}").contains("flash image"),
        "error should say what ROM boot needs: {err:#}"
    );
}

#[test]
fn flash_fast_boot_refuses_an_image_with_no_esp_bootloader_header() {
    let (chip, manifest) = common::system("configs/systems/esp32c3-devkit.yaml");
    let blobs = c3_rom_blobs();
    // Erased flash: no 0xE9 image magic at offset 0, so there is no 2nd-stage
    // bootloader to enter.
    let flash = vec![0xFF; 4096];
    let err = open(
        &chip,
        &manifest,
        FirmwareSource::FlashImage {
            image: &flash,
            symbols: None,
        },
        BootMode::FastBoot,
        &blobs,
    )
    .err()
    .expect("an erased flash image has no bootloader");
    assert!(
        format!("{err:#}").contains("bad magic"),
        "error should say the bootloader header is wrong: {err:#}"
    );
}

#[test]
fn flash_image_boot_without_rom_blobs_names_the_missing_blobs() {
    let (chip, manifest) = common::system("configs/systems/esp32c3-devkit.yaml");
    let flash = common::committed("crates/wasm/tests/fixtures/esp32c3-hello-world-flash.bin");
    let blobs = BlobMap::new();
    for boot in [BootMode::RomBoot, BootMode::FastBoot] {
        let err = open(
            &chip,
            &manifest,
            FirmwareSource::FlashImage {
                image: &flash,
                symbols: None,
            },
            boot,
            &blobs,
        )
        .err()
        .unwrap_or_else(|| panic!("{boot:?} from flash needs the mask ROM"));
        let msg = format!("{err:#}");
        assert!(
            msg.contains("esp32c3_irom") && msg.contains("esp32c3_drom"),
            "{boot:?}: error should name the blobs: {msg}"
        );
    }
}
