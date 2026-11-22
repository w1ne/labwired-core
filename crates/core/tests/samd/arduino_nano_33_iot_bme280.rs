// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino Nano 33 IoT (ATSAMD21G18A) + BME280 over `Wire`, console on the
//! native USB port.
//!
//! The fixture is a stock ArduinoCore-samd build (PlatformIO `atmelsam`
//! 9.0.0, ArduinoCore-samd 1.8.14, Adafruit BME280 Library 2.3.0) of
//! `examples/arduino-nano-33-iot-bme280/firmware/src/main.cpp`; rebuild with
//! `examples/arduino-nano-33-iot-bme280/firmware/build.sh`.
//!
//! `Serial` on this board is `SerialUSB`. The sketch prints nothing until
//! the scripted host has reset the bus, read the descriptors, set the
//! address and configuration and opened the CDC port (`while (!Serial)`),
//! so every printed byte crossed the `samd21_usb` model's bulk IN endpoint.
//! What the host read (IDs, interfaces, strings) is asserted from its log in
//! `examples/arduino-nano-33-iot-bme280/bme280-smoke.yaml`.
//! The sensor is read on SERCOM4, which the board manifest turns into the
//! I2C controller (the chip descriptor carries the Arduino Zero roles).

use crate::common;
use labwired_core::machine::AdvanceRequest;
use labwired_core::system::builder::{
    build_machine, BlobMap, BootMode, BuildOptions, BuildRequest, BuiltMachine, FirmwareSource,
};

const FIXTURE: &str = "tests/fixtures/samd/arduino-nano-33-iot-bme280.elf";

const USB_HOST: &str = "\x20 - id: \"usb\"\n\
     \x20   type: \"samd21_usb\"\n\
     \x20   base_address: 0x41005000\n\
     \x20   config:\n\
     \x20     host: true\n";

const BME280: &str = "external_devices:\n\
     \x20 - id: \"bme280\"\n\
     \x20   type: \"bme280\"\n\
     \x20   connection: \"sercom4\"\n\
     \x20   config:\n\
     \x20     i2c_address: 0x77\n";

/// The Nano 33 IoT with `devices` on Wire and `usb` in the manifest's
/// peripheral list, run for up to `millis` of simulated time or until the
/// sketch has printed its last line.
fn run_sketch(devices: &str, usb: &str, millis: u64) -> BuiltMachine {
    let chip_path = common::root("configs/chips/atsamd21g18a.yaml");
    let yaml = format!(
        "name: \"nano-33-iot-bme280\"\nchip: \"{}\"\n{devices}peripherals:\n{usb}\
         \x20 - id: \"sercom4\"\n\
         \x20   type: \"samd21_sercom_i2cm\"\n\
         \x20   base_address: 0x42001800\n",
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
    // 48 MHz: one millisecond is 48_000 cycles.
    for _ in 0..millis {
        built
            .machine
            .advance(AdvanceRequest::run(Some(48_000)))
            .unwrap();
        let text = serial(&built);
        if text.contains("RH=") && text.ends_with('\n') || text.contains("not found") {
            break;
        }
    }
    built
}

fn serial(built: &BuiltMachine) -> String {
    String::from_utf8_lossy(&built.uart.sink.lock().unwrap()).into_owned()
}

#[test]
fn nano_33_iot_prints_compensated_bme280_values_on_the_usb_serial_port() {
    let built = run_sketch(BME280, USB_HOST, 700);
    let out = serial(&built);
    assert_eq!(
        out, "NANO_33_IOT_BME280 ID=0x60\r\nT=25.00 C\r\nP=1013.25 hPa\r\nRH=50.00 %\r\n",
        "serial:\n{out}"
    );
}

#[test]
fn nano_33_iot_bme280_readings_follow_the_sensor_environment() {
    let devices = format!(
        "{BME280}\x20     temperature: 31.5\n\x20     humidity: 40.0\n\x20     pressure: 990.0\n"
    );
    let out = serial(&run_sketch(&devices, USB_HOST, 700));
    for want in ["T=31.50 C", "P=990.00 hPa", "RH=40.00 %"] {
        assert!(out.contains(want), "missing {want:?}; serial:\n{out}");
    }
}

#[test]
fn nano_33_iot_reports_an_absent_bme280() {
    let out = serial(&run_sketch("external_devices: []\n", USB_HOST, 700));
    assert_eq!(out, "BME280 not found at 0x77\r\n");
}

/// The chip default: no cable in the native port. Nothing enumerates, the
/// port never opens and `while (!Serial)` holds the sketch.
#[test]
fn without_a_host_the_sketch_waits_for_the_port_and_prints_nothing() {
    let built = run_sketch(BME280, "", 700);
    assert_eq!(serial(&built), "");
}
