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

use crate::common;
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

#[test]
fn esp32s3_builders_preserve_the_resolved_chip_io_levels() {
    let voltage_route: labwired_config::CosimModelConfig = serde_yaml::from_str(
        r#"
id: io_voltage_probe
adapter: analog
step_ns: 100000
inputs: {}
outputs: { v_pin: board.gpio_in.gpio8 }
config:
  vdd: 3.3
  netlist_text: "Vtest in 0 dc 3.3\nRpull in pad 4.7k\nRload pad 0 1e12\n"
  probes: { v_pin: "v(pad)" }
  sources: {}
"#,
    )
    .unwrap();
    let blobs = s3_rom_blobs();
    let elf = common::committed("tests/fixtures/tier1/esp32s3.elf");
    let flash = common::committed("tests/fixtures/tier1/esp32s3-flash.bin");
    for name in ["esp32s3", "esp32s3-zero"] {
        let resolved = labwired_config::ResolvedSystem::from_builtin_chip(name).unwrap();
        let mut chip = resolved.chip().unwrap();
        assert_eq!(chip.io_voltage_v, Some(3.3));
        let levels = chip.gpio_input_thresholds.expect("transcribed S3 limits");
        assert_eq!(levels.vil, 0.25);
        assert_eq!(levels.vih, 0.75);
        for present in [true, false] {
            if !present {
                chip.io_voltage_v = None;
                chip.gpio_input_thresholds = None;
            }
            for boot in [BootMode::FastBoot, BootMode::RomBoot] {
                let firmware = match boot {
                    BootMode::FastBoot => FirmwareSource::Elf(&elf),
                    BootMode::RomBoot => FirmwareSource::FlashImage {
                        image: &flash,
                        symbols: None,
                    },
                };
                let built = build_machine(BuildRequest {
                    chip: &chip,
                    system: &resolved.manifest,
                    firmware,
                    boot,
                    blobs: &blobs,
                    options: BuildOptions::default(),
                })
                .expect("construct an S3 without executing it");
                let bound = built.machine.bind_cosim(
                    std::slice::from_ref(&voltage_route),
                    std::path::Path::new("."),
                );
                let session = bound.expect("analog model constructs").expect("one model");
                if present {
                    assert!(
                        session.binding_errors().is_empty(),
                        "{:?}",
                        session.binding_errors()
                    );
                } else {
                    assert!(
                        session.binding_errors().iter().any(|e| matches!(
                            e,
                            labwired_core::cosim::RoutingError::NoInputThresholds { .. }
                        )),
                        "missing metadata must stay a binding error: {:?}",
                        session.binding_errors()
                    );
                }
            }
        }
    }
}

/// The production S3 constructor registers its peripheral bank
/// programmatically instead of through `SystemBus::from_config`, so it must
/// load the descriptor's `analog_pins:` table itself. Without that the generic
/// NTC divider deck binds with `NoAdcChannel` on the S3 fast-boot run even
/// though `esp32s3.yaml` publishes GPIO4 as ADC1_CH3 (the flash-image path
/// has its own test below). Run the committed Tier-1
/// ELF through `build_machine`, solve the deck across a model boundary, and
/// read the level back through the SENS SAR1 one-shot registers.
#[test]
fn esp32s3_build_machine_routes_the_generic_analog_deck_to_sens_adc1() {
    use labwired_config::SystemManifest;
    use labwired_core::machine::AdvanceRequest;
    use std::path::Path;

    const SAR_MEAS1_CTRL2: u64 = 0x6000_8800 + 0x00C;
    const SAR1_EN_PAD_CH3: u32 = 1 << (19 + 3);
    const MEAS_START_SAR: u32 = 1 << 17;
    const MEAS_DONE_SAR: u32 = 1 << 16;

    let (chip, mut manifest) = common::system("configs/systems/esp32s3-devkitc-1-v11.yaml");
    let deck = SystemManifest::from_file(common::root(
        "examples/esp32s3-devkitc-1/analog-ntc-diode-system.yaml",
    ))
    .expect("load the generic ESP32-S3 analog deck");
    manifest.cosim_models = deck.cosim_models;
    let fw = common::committed("tests/fixtures/tier1/esp32s3.elf");
    let blobs = BlobMap::new();
    let mut built = build_machine(BuildRequest {
        chip: &chip,
        system: &manifest,
        firmware: FirmwareSource::Elf(&fw),
        boot: BootMode::FastBoot,
        blobs: &blobs,
        options: BuildOptions::default(),
    })
    .expect("build the S3 machine");
    let mut cosim = built
        .machine
        .bind_cosim(&manifest.cosim_models, Path::new("."))
        .expect("build analog co-simulation")
        .expect("deck carries analog models");
    assert!(
        cosim.binding_errors().is_empty(),
        "the hosted S3 bus must resolve GPIO4/GPIO5 from esp32s3.yaml: {:?}",
        cosim.binding_errors()
    );
    cosim
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .expect("set generic NTC temperature");
    built
        .machine
        .advance_cosim(&mut cosim, AdvanceRequest::run(Some(20_000)))
        .expect("run firmware across an analog boundary");

    built
        .machine
        .bus_write_u32(SAR_MEAS1_CTRL2, SAR1_EN_PAD_CH3)
        .unwrap();
    built
        .machine
        .bus_write_u32(SAR_MEAS1_CTRL2, SAR1_EN_PAD_CH3 | MEAS_START_SAR)
        .unwrap();
    let word = built.machine.bus_read_u32(SAR_MEAS1_CTRL2).unwrap();
    assert_ne!(word & MEAS_DONE_SAR, 0, "MEAS1_DONE_SAR is raised");
    assert_eq!(
        word & 0xFFFF,
        u32::from(labwired_core::peripherals::esp_xtensa_common::esp32s3_adc1_mv_to_raw(1_650, 3)),
        "the 25 C NTC divider (1.65 V on GPIO4) reaches SENS ADC1 channel 3"
    );
}

/// The flash-image ROM-boot constructor is the path hosted S3 runs take, and
/// it also builds its bank programmatically. It must bind the same generic
/// deck without `NoAdcChannel` on GPIO4/GPIO5.
#[test]
fn esp32s3_flash_image_build_machine_binds_the_generic_analog_deck() {
    use labwired_config::SystemManifest;
    use std::path::Path;

    let (chip, mut manifest) = common::system("configs/systems/esp32s3-zero.yaml");
    let deck = SystemManifest::from_file(common::root(
        "examples/esp32s3-devkitc-1/analog-ntc-diode-system.yaml",
    ))
    .expect("load the generic ESP32-S3 analog deck");
    manifest.cosim_models = deck.cosim_models;
    let flash = common::committed("tests/fixtures/tier1/esp32s3-flash.bin");
    let blobs = s3_rom_blobs();
    let built = build_machine(BuildRequest {
        chip: &chip,
        system: &manifest,
        firmware: FirmwareSource::FlashImage {
            image: &flash,
            symbols: None,
        },
        boot: BootMode::RomBoot,
        blobs: &blobs,
        options: BuildOptions::default(),
    })
    .expect("build the S3 flash-boot machine");
    let cosim = built
        .machine
        .bind_cosim(&manifest.cosim_models, Path::new("."))
        .expect("build analog co-simulation")
        .expect("deck carries analog models");
    assert!(
        cosim.binding_errors().is_empty(),
        "the flash-boot S3 bus must resolve GPIO4/GPIO5 from esp32s3.yaml: {:?}",
        cosim.binding_errors()
    );
}
