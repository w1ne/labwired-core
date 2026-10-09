// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32duino `Wire` multi-byte reads against a BME280, on both STM32 I²C
//! generations.
//!
//! The hosted MCP audit (2026-10-09) found `Adafruit_BME280::begin()` never
//! succeeding on NUCLEO-L476RG, -F401RE and -F103RB: a register-pointer write
//! followed by a 2-byte `requestFrom` returned 0 bytes, while 1-byte reads
//! worked. The library's calibration read is multi-byte, so `begin()` failed.
//!
//! * Legacy controller (F1/F4, RM0008 §26.3.3 / RM0090 §27.3.3): the HAL
//!   receives the last two bytes of every transfer through BTF ("data N-1 in
//!   DR, data N in the shift register"). The model never raised BTF on a
//!   receive, so the interrupt-driven `HAL_I2C_Master_Seq_Receive_IT` that
//!   STM32duino uses waited for an event that never came.
//! * Modern controller (L4, RM0351 §37.4.8): CR2.NBYTES bytes are transferred
//!   per START, each with its own TXIS / RXNE. The model moved one byte and
//!   then declared the transfer complete.
//!
//! The fixtures are real STM32duino builds (PlatformIO `ststm32`,
//! `framework-arduinoststm32` 4.21200.0, Adafruit BME280 Library 2.3.0) of
//! `tests/fixtures/stm32duino-bme280/main.ino`; rebuild with `pio run` in that
//! directory and `arm-none-eabi-strip` the three `firmware.elf`s.

mod common;

use labwired_core::machine::AdvanceRequest;
use labwired_core::system::builder::{
    build_machine, BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};

fn run_bme280_sketch(chip: &str, fixture: &str) -> String {
    let chip_path = common::root(&format!("configs/chips/{chip}.yaml"));
    let yaml = format!(
        "name: \"bme280-{chip}\"\n\
         chip: \"{}\"\n\
         external_devices:\n\
         \x20 - id: \"bme\"\n\
         \x20   type: \"bme280\"\n\
         \x20   connection: \"i2c1\"\n\
         \x20   config:\n\
         \x20     i2c_address: 0x76\n",
        chip_path.display()
    );
    let manifest: labwired_config::SystemManifest = serde_yaml::from_str(&yaml).unwrap();
    let chip_desc = labwired_config::ChipDescriptor::from_file(&chip_path).unwrap();

    let fw = common::committed(&format!("tests/fixtures/{fixture}"));
    let blobs = BlobMap::new();
    let mut built = build_machine(BuildRequest {
        chip: &chip_desc,
        system: &manifest,
        firmware: FirmwareSource::Elf(&fw),
        boot: BootMode::FastBoot,
        blobs: &blobs,
        options: BuildOptions::default(),
    })
    .unwrap();
    for _ in 0..400 {
        built
            .machine
            .advance(AdvanceRequest::run(Some(100_000)))
            .unwrap();
        let text = String::from_utf8_lossy(&built.uart.sink.lock().unwrap()).into_owned();
        if text.contains("DONE") {
            return text;
        }
    }
    let text = String::from_utf8_lossy(&built.uart.sink.lock().unwrap()).into_owned();
    text
}

fn assert_multibyte_reads(chip: &str, fixture: &str) {
    let out = run_bme280_sketch(chip, fixture);
    eprintln!("{chip} serial:\n{out}");
    for want in [
        "reg 0xD0 len 1 end=0 got=1 : 60",
        "reg 0x88 len 2 end=0 got=2 :",
        "reg 0x88 len 3 end=0 got=3 :",
        "reg 0x88 len 6 end=0 got=6 :",
        "BME280 ok",
        "DONE",
    ] {
        assert!(
            out.contains(want),
            "{chip}: serial is missing {want:?}; full serial:\n{out}"
        );
    }
    // A multi-byte read is the same register stream as the single-byte reads
    // it extends: the 6-byte calibration read starts with the 2- and 3-byte
    // reads' bytes, so a model that returns the right COUNT of wrong bytes
    // (repeats, zeros, shifted by one) fails here.
    let line = |len: u8| -> Vec<String> {
        let tag = format!("reg 0x88 len {len} end=0 got={len} :");
        let l = out.lines().find(|l| l.starts_with(&tag)).unwrap();
        l[tag.len()..]
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    };
    let (two, three, six) = (line(2), line(3), line(6));
    assert_eq!(two.len(), 2, "{chip}: {out}");
    assert_eq!(three.len(), 3, "{chip}: {out}");
    assert_eq!(six.len(), 6, "{chip}: {out}");
    assert_eq!(&six[..2], &two[..], "{chip}: {out}");
    assert_eq!(&six[..3], &three[..], "{chip}: {out}");
    assert!(
        out.lines()
            .any(|l| l.starts_with("T=") && l.ends_with(" C")),
        "{chip}: no temperature line; serial:\n{out}"
    );
}

#[test]
fn l476_wire_multibyte_reads_and_bme280_begin() {
    assert_multibyte_reads("stm32l476", "stm32duino-bme280-l476.elf");
}

#[test]
fn f401_wire_multibyte_reads_and_bme280_begin() {
    assert_multibyte_reads("stm32f401", "stm32duino-bme280-f401.elf");
}

#[test]
fn f103_wire_multibyte_reads_and_bme280_begin() {
    assert_multibyte_reads("stm32f103", "stm32duino-bme280-f103.elf");
}
