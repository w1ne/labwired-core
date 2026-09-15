// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `build_machine` on AVR (ATmega328P / Arduino Nano): the committed golden
//! sketch boots through a `Session` and its USART reaches the console.
//!
//! The firmware is `tests/fixtures/avr/arduino-nano-blinky.elf`, the same
//! committed ELF `avr_nano_golden_survival` (a core-ci step) runs, so this needs
//! no AVR toolchain and cannot skip.

mod common;

use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use std::time::Duration;

const NANO_ELF: &str = "tests/fixtures/avr/arduino-nano-blinky.elf";

fn open(firmware: FirmwareSource<'_>, boot: BootMode) -> anyhow::Result<Session> {
    let (chip, manifest) = common::system("configs/systems/arduino-nano.yaml");
    let blobs = BlobMap::new();
    Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware,
            boot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
}

#[test]
fn arduino_nano_sketch_prints_through_build_machine() {
    let fw = common::committed(NANO_ELF);
    let mut s = open(FirmwareSource::Elf(&fw), BootMode::FastBoot).unwrap();
    // The sketch's Serial banner, the marker `avr_nano_golden_survival` and
    // `avr_nano_machine_run` assert.
    let r = s.expect("nano-ok", Duration::from_secs(2));
    let transcript = s.uart_transcript();
    r.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{transcript}"));
}

#[test]
fn avr_elf_image_is_the_image_the_loader_produces() {
    // The builder cannot call `labwired_loader` (the loader depends on core), so
    // core carries the loader's AVR segment mapping: flash LMA segments plus the
    // biased data-space VMA copies that preload SRAM. Both must agree.
    let fw = common::committed(NANO_ELF);
    let core = labwired_core::system::node::parse_avr_elf_image(&fw).unwrap();
    let loader = labwired_loader::load_elf_bytes(&fw).unwrap();
    assert_eq!(core.entry_point, loader.entry_point);
    assert_eq!(core.arch, loader.arch);
    let seg = |img: &labwired_core::memory::ProgramImage| {
        img.segments
            .iter()
            .map(|s| (s.start_addr, s.data.clone()))
            .collect::<Vec<_>>()
    };
    assert!(
        core.segments.len() > 1,
        "the sketch has code and a data section"
    );
    assert_eq!(seg(&core), seg(&loader));
}

#[test]
fn avr_refuses_firmware_that_is_not_an_avr_elf() {
    let arm = common::committed("tests/fixtures/uart-ok-thumbv7m.elf");
    let err = open(FirmwareSource::Elf(&arm), BootMode::FastBoot)
        .err()
        .expect("an ARM ELF cannot run on an ATmega328P");
    assert!(
        format!("{err:#}").contains("AVR"),
        "error should say the ELF is not AVR: {err:#}"
    );
}

#[test]
fn avr_boots_only_from_an_elf() {
    let fw = common::committed(NANO_ELF);
    for (firmware, boot) in [
        (
            FirmwareSource::FlashImage {
                image: &fw,
                symbols: None,
            },
            BootMode::FastBoot,
        ),
        (FirmwareSource::Elf(&fw), BootMode::RomBoot),
    ] {
        let err = open(firmware, boot)
            .err()
            .unwrap_or_else(|| panic!("AVR {boot:?} is not modelled"));
        assert!(
            format!("{err:#}").contains("not supported"),
            "{boot:?}: {err:#}"
        );
    }
}
