// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `build_machine` on PIC18 (PIC18F4520): the committed SDCC L1 smoke boots
//! through a `Session` and its CPU-internal EUSART reaches the console.
//!
//! The `Session`/DAP path builds the core in `system::builder::pic18`, not in
//! the CLI, so this pins that it uses the chip's geometry (32 KB flash, Access
//! Bank split at 0x80) and wires the EUSART into the session UART. The ELF is
//! the committed fixture `pic18f4520_smoke` runs, so this needs no SDCC and
//! cannot skip.

mod common;

use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use std::time::Duration;

const PIC18_ELF: &str = "examples/pic18f4520/firmware/pic18f4520-sdcc-uart-smoke.elf";

fn open(firmware: FirmwareSource<'_>, boot: BootMode) -> anyhow::Result<Session> {
    let (chip, manifest) = common::system("examples/pic18f4520/system.yaml");
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
fn pic18f4520_smoke_prints_through_build_machine() {
    let fw = common::committed(PIC18_ELF);
    let mut s = open(FirmwareSource::Elf(&fw), BootMode::FastBoot).unwrap();
    // Same banner `pic18f4520_smoke` and `examples/pic18f4520/io-smoke.yaml`
    // assert; the last line is printed only after the LED toggle round-trip.
    let r = s.expect("TOGGLE OK", Duration::from_secs(2));
    let transcript = s.uart_transcript();
    r.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{transcript}"));
    assert!(
        transcript.contains("PIC18F4520 OK\r\nLED=1\r\nTOGGLE OK"),
        "unexpected console:\n{transcript}"
    );
}

#[test]
fn pic18_boots_only_from_an_elf() {
    let fw = common::committed(PIC18_ELF);
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
            .unwrap_or_else(|| panic!("PIC18 {boot:?} is not modelled"));
        assert!(
            format!("{err:#}").contains("not supported"),
            "{boot:?}: {err:#}"
        );
    }
}
