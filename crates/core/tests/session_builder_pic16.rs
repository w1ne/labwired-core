// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `build_machine` on PIC16 (PIC16F877A): the committed SDCC L1 smoke boots
//! through a `Session` and its CPU-internal USART reaches the console.
//!
//! The `Session`/DAP path builds the core in `system::builder::pic16`, not in
//! the CLI. The ELF says EM_MCHP_PIC, which the loader reports as PIC18, so
//! this also pins that the chip's `arch: pic16` retargets it to the PIC16
//! core. The ELF is the committed fixture `pic16f877a_smoke` runs, so this
//! needs no SDCC and cannot skip.

mod common;

use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use std::time::Duration;

const PIC16_ELF: &str = "examples/pic16f877a/firmware/pic16f877a-sdcc-uart-smoke.elf";

fn open(firmware: FirmwareSource<'_>, boot: BootMode) -> anyhow::Result<Session> {
    let (chip, manifest) = common::system("examples/pic16f877a/system.yaml");
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
fn pic16f877a_smoke_prints_through_build_machine() {
    let fw = common::committed(PIC16_ELF);
    let mut s = open(FirmwareSource::Elf(&fw), BootMode::FastBoot).unwrap();
    // Same banner `pic16f877a_smoke` and `examples/pic16f877a/io-smoke.yaml`
    // assert; the last line is printed only after the LED toggle round-trip.
    let r = s.expect("TOGGLE OK", Duration::from_secs(2));
    let transcript = s.uart_transcript();
    r.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{transcript}"));
    assert!(
        transcript.contains("PIC16F877A OK\r\nLED=1\r\nTOGGLE OK"),
        "unexpected console:\n{transcript}"
    );
}

#[test]
fn pic16_boots_only_from_an_elf() {
    let fw = common::committed(PIC16_ELF);
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
            .unwrap_or_else(|| panic!("PIC16 {boot:?} is not modelled"));
        assert!(
            format!("{err:#}").contains("not supported"),
            "{boot:?}: {err:#}"
        );
    }
}

#[test]
fn a_pic16_image_on_a_pic18_chip_is_not_run_as_pic16() {
    // Negative control for the retarget: the same bytes on the PIC18F4520
    // stay a PIC18 image and do not print the PIC16 banner.
    let fw = common::committed(PIC16_ELF);
    let (chip, manifest) = common::system("examples/pic18f4520/system.yaml");
    let blobs = BlobMap::new();
    let opened = Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    );
    if let Ok(mut s) = opened {
        assert!(s
            .expect("PIC16F877A OK", Duration::from_millis(300))
            .is_err());
    }
}
