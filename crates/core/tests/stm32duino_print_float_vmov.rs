// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `Serial.print(float)` on STM32duino 3 (Cortex-M4F) must not BusFault.
//!
//! Production run 2026-10-10: the stock Adafruit_BME280 sketch on
//! NUCLEO-L476RG printed `T=` and then HardFaulted (precise BusFault at
//! 0x403F_7FC0) inside `arduino::Print::print(float, int)`:
//!
//! ```text
//! bl   __aeabi_f2d
//! vmov d0, r0, r1      ; ec41 0b10
//! ```
//!
//! `vmov d0, r0, r1` is the 64-bit core-to-FP transfer (ARMv7-M ARM A7.9,
//! opcode 0b0010x). The decoder read it as `vstmdb r1, {16 words}` and stored
//! to `r1 - 0x40`, where r1 held the high word of the double (0x403F_8000 for
//! 31.5 °C). F103 (Cortex-M3, soft-float) never emits the instruction.
//!
//! The fixtures are PlatformIO `ststm32@20.0.0` builds
//! (`framework-arduinoststm32` 4.30000.0, Adafruit BME280 Library) of
//! `tests/fixtures/stm32duino-print-float/main.ino`, the exact sketch of the
//! production request; rebuild with `pio run` in that directory and
//! `arm-none-eabi-strip` the three `firmware.elf`s.

mod common;

use labwired_core::machine::AdvanceRequest;
use labwired_core::system::builder::{
    build_machine, BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};

fn run_sketch(chip: &str, fixture: &str) -> String {
    let chip_path = common::root(&format!("configs/chips/{chip}.yaml"));
    let yaml = format!(
        "name: \"print-float-{chip}\"\n\
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
    // The production request's stimulus.
    built.machine.set_input("temperature", 31.5).unwrap();
    for _ in 0..400 {
        built
            .machine
            .advance(AdvanceRequest::run(Some(100_000)))
            .unwrap();
        let text = String::from_utf8_lossy(&built.uart.sink.lock().unwrap()).into_owned();
        if text.contains(" C\r\n") {
            return text;
        }
    }
    let text = String::from_utf8_lossy(&built.uart.sink.lock().unwrap()).into_owned();
    text
}

fn assert_prints_temperature(chip: &str, fixture: &str) {
    let out = run_sketch(chip, fixture);
    assert!(
        out.contains("BME280 ok\r\nT=31.50 C\r\n"),
        "{chip}: expected the BME280 model's 31.50 C; serial:\n{out}"
    );
}

#[test]
fn l476_print_float_reaches_serial() {
    assert_prints_temperature("stm32l476", "stm32duino-print-float-l476.elf");
}

#[test]
fn f401_print_float_reaches_serial() {
    assert_prints_temperature("stm32f401", "stm32duino-print-float-f401.elf");
}

#[test]
fn f103_print_float_reaches_serial() {
    assert_prints_temperature("stm32f103", "stm32duino-print-float-f103.elf");
}
