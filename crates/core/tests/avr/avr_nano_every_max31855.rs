// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino Nano Every (ATmega4809) + MAX31855 over `SPI`.
//!
//! The fixture is a stock ArduinoCore-megaavr build (PlatformIO
//! `atmelmegaavr` 1.10.0, ArduinoCore-megaavr 1.8.8, Adafruit MAX31855
//! library 1.4.2, Adafruit BusIO 1.17.4) of
//! `examples/arduino-nano-every-max31855/firmware/src/main.cpp`; rebuild with
//! `examples/arduino-nano-every-max31855/firmware/build.sh`, which copies
//! `.pio/build/nano_every/firmware.elf` to
//! `tests/fixtures/avr/arduino-nano-every-max31855.elf`.
//!
//! `Adafruit_MAX31855::readCelsius()` and `readInternal()` each frame four
//! `SPI.transfer()` bytes with CS on D10 (PB1) and decode the 14-bit and
//! 12-bit fields. A wrong byte, a lost chip-select edge between the two
//! frames or a wrong bit order moves the printed values.

use crate::common;
use labwired_core::machine::AdvanceRequest;
use labwired_core::system::builder::{
    build_machine, BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};

const FIXTURE: &str = "tests/fixtures/avr/arduino-nano-every-max31855.elf";

/// The serial output of the sketch on the Nano Every with `devices` on SPI0.
fn run_sketch(devices: &str) -> String {
    let chip_path = common::root("configs/chips/atmega4809.yaml");
    let yaml = format!(
        "name: \"nano-every-max31855\"\nchip: \"{}\"\ncpu_hz: 16000000\n{devices}",
        chip_path.display()
    );
    let manifest: labwired_config::SystemManifest = serde_yaml::from_str(&yaml).unwrap();
    let chip = labwired_config::ChipDescriptor::from_file(&chip_path).unwrap();
    let firmware = common::committed(FIXTURE);
    let blobs = BlobMap::new();
    let mut built = build_machine(BuildRequest {
        chip: &chip,
        system: &manifest,
        firmware: FirmwareSource::Elf(&firmware),
        boot: BootMode::FastBoot,
        blobs: &blobs,
        options: BuildOptions::default(),
    })
    .unwrap();
    let serial = |built: &labwired_core::system::builder::BuiltMachine| {
        String::from_utf8_lossy(&built.uart.sink.lock().unwrap()).into_owned()
    };
    for _ in 0..20 {
        built
            .machine
            .advance(AdvanceRequest::run(Some(100_000)))
            .unwrap();
        let text = serial(&built);
        if (text.contains("CJ=") || text.contains("fault")) && text.ends_with('\n') {
            break;
        }
    }
    serial(&built)
}

fn max31855(cs_pin: &str, junctions: &str) -> String {
    format!(
        "external_devices:\n\
         \x20 - id: \"max31855\"\n\
         \x20   type: \"max31855\"\n\
         \x20   connection: \"spi0\"\n\
         \x20   config:\n\
         \x20     cs_pin: \"{cs_pin}\"\n{junctions}"
    )
}

#[test]
fn nano_every_spi_reads_both_max31855_junctions() {
    let out = run_sketch(&max31855("PB1", ""));
    assert_eq!(
        out, "NANO_EVERY_MAX31855\r\nTC=25.00 C\r\nCJ=22.0000 C\r\n",
        "serial:\n{out}"
    );
}

#[test]
fn nano_every_max31855_readings_follow_the_junction_temperatures() {
    let out = run_sketch(&max31855(
        "PB1",
        "\x20     temperature: 100.25\n\x20     internal: 30.5\n",
    ));
    assert_eq!(
        out, "NANO_EVERY_MAX31855\r\nTC=100.25 C\r\nCJ=30.5000 C\r\n",
        "serial:\n{out}"
    );
}

#[test]
fn nano_every_max31855_on_another_chip_select_reads_an_idle_bus() {
    // The sketch drives D10 (PB1); a converter on D9 (PB0) is never selected.
    let out = run_sketch(&max31855("PB0", ""));
    assert_eq!(out, "NANO_EVERY_MAX31855\r\nMAX31855 fault 0x7\r\n");
}

#[test]
fn nano_every_spi_reads_an_idle_bus_without_a_converter() {
    let out = run_sketch("external_devices: []\n");
    assert_eq!(out, "NANO_EVERY_MAX31855\r\nMAX31855 fault 0x7\r\n");
}
