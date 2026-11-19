// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino Nano Every (ATmega4809) + BME280 over `Wire`.
//!
//! The fixture is a stock ArduinoCore-megaavr build (PlatformIO
//! `atmelmegaavr` 1.10.0, ArduinoCore-megaavr 1.8.8, Adafruit BME280 Library
//! 2.3.0) of `examples/arduino-nano-every-bme280/firmware/src/main.cpp`;
//! rebuild with `examples/arduino-nano-every-bme280/firmware/build.sh` and
//! copy `.pio/build/nano_every/firmware.elf` to
//! `tests/fixtures/avr/arduino-nano-every-bme280.elf`.
//!
//! `Adafruit_BME280::begin()` reads the chip ID and the calibration blocks
//! with interrupt-driven multi-byte TWI0 transfers, and the three read calls
//! run the Bosch compensation on the raw ADC words. A wrong byte, a wrong
//! count or a lost repeated START moves the printed values.

use crate::common;
use labwired_core::machine::AdvanceRequest;
use labwired_core::system::builder::{
    build_machine, BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};

const FIXTURE: &str = "tests/fixtures/avr/arduino-nano-every-bme280.elf";

/// The serial output of the sketch on the Nano Every with `devices` on TWI0.
fn run_sketch(devices: &str) -> String {
    let chip_path = common::root("configs/chips/atmega4809.yaml");
    let yaml = format!(
        "name: \"nano-every-bme280\"\nchip: \"{}\"\ncpu_hz: 16000000\n{devices}",
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
    for _ in 0..80 {
        built
            .machine
            .advance(AdvanceRequest::run(Some(100_000)))
            .unwrap();
        let text = serial(&built);
        if text.contains("RH=") && text.ends_with('\n') || text.contains("not found") {
            break;
        }
    }
    serial(&built)
}

fn bme280(environment: &str) -> String {
    format!(
        "external_devices:\n\
         \x20 - id: \"bme280\"\n\
         \x20   type: \"bme280\"\n\
         \x20   connection: \"twi0\"\n\
         \x20   config:\n\
         \x20     i2c_address: 0x77\n{environment}"
    )
}

#[test]
fn nano_every_wire_reads_compensated_bme280_values() {
    let out = run_sketch(&bme280(""));
    assert_eq!(
        out, "NANO_EVERY_BME280 ID=0x60\r\nT=25.00 C\r\nP=1013.25 hPa\r\nRH=50.00 %\r\n",
        "serial:\n{out}"
    );
}

#[test]
fn nano_every_bme280_readings_follow_the_sensor_environment() {
    let out = run_sketch(&bme280(
        "\x20     temperature: 31.5\n\x20     humidity: 40.0\n\x20     pressure: 990.0\n",
    ));
    for want in ["T=31.50 C", "P=990.00 hPa", "RH=40.00 %"] {
        assert!(out.contains(want), "missing {want:?}; serial:\n{out}");
    }
}

#[test]
fn nano_every_wire_reports_an_absent_bme280() {
    let out = run_sketch("external_devices: []\n");
    assert_eq!(out, "BME280 not found at 0x77\r\n");
}
