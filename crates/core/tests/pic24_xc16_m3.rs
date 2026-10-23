// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! dsPIC33FJ32MC204 × real XC16 M3 firmware: ADC1 + SPI1 loopback + TMP102.
//!
//! `examples/dspic33fj32mc204/firmware/dspic33fj32mc204-xc16-m3-adc-spi-i2c.elf`
//! is built by Microchip XC16 v1.70 from `xc16-m3-adc-spi-i2c.c` (recipe in
//! the example README). The program samples AN4, shifts 0xA5 through SPI1
//! (SDO-SDI loopback), reads the TMP102 temperature register over I2C1 and
//! writes the results plus a completion marker to RAM.
//!
//! The devices are attached from the example system manifest
//! (`examples/dspic33fj32mc204/system.yaml`), using the same factory call the
//! PIC24 session builder uses, so the manifest's `tmp102` and `loopback`
//! entries are exercised here too.
//!
//! Scope and limitations are documented in
//! `examples/dspic33fj32mc204/VALIDATION.md` and `docs/pic24-m0-reference.md`
//! §3.4.

use labwired_config::SystemManifest;
use labwired_core::cpu::pic24::{
    SFR_AD1CHS0, SFR_AD1CON1, SFR_AD1PCFGL, SFR_SPI1CON1, SFR_SPI1STAT,
};
use labwired_core::cpu::Pic24;
use labwired_core::peripherals::components::i2c_factory::build_external_i2c_device;
use labwired_core::peripherals::spi_loopback::SpiLoopback;
use labwired_core::system::node::parse_elf_image;
use labwired_core::{Cpu, SimulationConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const MARKER: u16 = 0x0E00;
const ADC_RESULT: u16 = 0x0E02;
const SPI_RESULT: u16 = 0x0E04;
const I2C_MSB: u16 = 0x0E06;
const I2C_LSB: u16 = 0x0E07;
const FAIL_CODE: u16 = 0x0E08;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_firmware() -> (Pic24, Vec<u8>) {
    let root = repo_root();
    let system = SystemManifest::from_file(root.join("examples/dspic33fj32mc204/system-m3.yaml"))
        .expect("parse the M3 example system manifest");

    let elf = std::fs::read(
        root.join("examples/dspic33fj32mc204/firmware/dspic33fj32mc204-xc16-m3-adc-spi-i2c.elf"),
    )
    .expect("read XC16 M3 ELF");
    let image = parse_elf_image(&elf).expect("parse ELF");
    assert_eq!(image.arch, labwired_core::Arch::Pic24);

    let mut bus = labwired_core::bus::SystemBus::new();
    let mut cpu = Pic24::new();
    cpu.load_program_image(&image);

    // Same attach policy as `system::builder::pic24`: I2C1 devices come from
    // the generic external-device factory, SPI1 accepts the wire loopback.
    for device in &system.external_devices {
        match device.connection.as_str() {
            "i2c1" => {
                let model = build_external_i2c_device(&device.r#type, &device.id, &device.config)
                    .expect("build the manifest's I2C device");
                cpu.attach_i2c_device(model);
            }
            "spi1" => {
                assert!(matches!(
                    device.r#type.as_str(),
                    "loopback" | "spi_loopback"
                ));
                cpu.attach_spi_device(Box::new(SpiLoopback::new("")));
            }
            _ => {}
        }
    }

    let sink: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(sink.clone());
    cpu.reset(&mut bus).expect("reset");
    cpu.set_pc(0);
    // The harness's analog source: AN4 (RB2) at 0x2AA (10-bit).
    cpu.set_adc_analog(4, 0x2AA);

    for _ in 0..200_000 {
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .expect("M3 firmware instruction must decode and execute");
        if cpu.data_read(MARKER).unwrap() == 0xBEEF {
            let bytes = sink.lock().unwrap().clone();
            return (cpu, bytes);
        }
    }
    panic!("the M3 firmware never reached its completion marker");
}

#[test]
fn xc16_m3_firmware_samples_adc_loops_spi_and_reads_tmp102() {
    let (mut cpu, uart) = run_firmware();

    assert_eq!(
        cpu.data_read(ADC_RESULT).unwrap(),
        0x02AA,
        "ADC1BUF0: AN4 unsigned integer"
    );
    assert_eq!(
        cpu.data_read(SPI_RESULT).unwrap(),
        0x00A5,
        "SPI1 loopback byte"
    );
    assert_eq!(
        cpu.data_read(I2C_MSB).unwrap(),
        0x0019,
        "TMP102 temperature MSB: 25.0 C"
    );
    assert_eq!(cpu.data_read(I2C_LSB).unwrap(), 0x0000, "TMP102 LSB");
    assert_eq!(cpu.data_read(FAIL_CODE).unwrap(), 0, "no I2C failure");
    assert_eq!(uart, b"M3 OK\n", "UART1 completion message");

    // The register image the firmware programmed.
    assert_eq!(cpu.data_read(SFR_AD1CON1).unwrap() & 0x8000, 0x8000, "ADON");
    assert_eq!(cpu.data_read(SFR_AD1PCFGL).unwrap(), 0x01EF);
    assert_eq!(cpu.data_read(SFR_AD1CHS0).unwrap(), 4, "CH0SA = AN4");
    assert_eq!(cpu.data_read(SFR_SPI1CON1).unwrap(), 0x0137);
    assert_eq!(
        cpu.data_read(SFR_SPI1STAT).unwrap() & 0x8000,
        0x8000,
        "SPIEN"
    );
}

/// Build and run the M3 machine through the full session path and return
/// the UART bytes. `system` must carry an absolute `chip` path.
fn run_builder(system: &SystemManifest, elf: &[u8]) -> String {
    use labwired_config::ChipDescriptor;
    use labwired_core::system::builder::{
        build_machine, BootMode, BuildOptions, BuildRequest, FirmwareSource,
    };
    use labwired_core::AdvanceRequest;

    let root = repo_root();
    let chip = ChipDescriptor::from_file(root.join("configs/chips/dspic33fj32mc204.yaml"))
        .expect("parse chip descriptor");
    let blobs = std::collections::HashMap::new();
    let built = build_machine(BuildRequest {
        chip: &chip,
        system,
        firmware: FirmwareSource::Elf(elf),
        boot: BootMode::FastBoot,
        blobs: &blobs,
        options: BuildOptions {
            echo_uart_stdout: false,
            uart_rx: None,
        },
    })
    .expect("build the PIC24 M3 machine");
    let mut machine = built.machine;
    machine
        .advance(AdvanceRequest::run(Some(200_000)))
        .expect("run the M3 firmware");
    let out = built.uart.sink.lock().unwrap().clone();
    String::from_utf8_lossy(&out).into_owned()
}

/// The full session path: `build_machine` reads the M3 system, attaches the
/// TMP102 to the CPU-internal I2C1 and the loopback to SPI1, and the firmware
/// only prints `M3 OK` after it has read the sensor and matched the SPI
/// loopback byte. A second machine built with the devices removed must print
/// `M3 FAIL`, so this test cannot pass without the attach path.
#[test]
fn example_system_builder_attaches_the_m3_devices() {
    let root = repo_root();
    let mut system =
        SystemManifest::from_file(root.join("examples/dspic33fj32mc204/system-m3.yaml"))
            .expect("parse the M3 example system");
    system.chip = root
        .join("configs/chips/dspic33fj32mc204.yaml")
        .display()
        .to_string();
    let elf = std::fs::read(
        root.join("examples/dspic33fj32mc204/firmware/dspic33fj32mc204-xc16-m3-adc-spi-i2c.elf"),
    )
    .expect("read XC16 M3 ELF");

    let out = run_builder(&system, &elf);
    assert!(
        out.ends_with("M3 OK\n"),
        "builder-attached TMP102/loopback did not complete: {out:?}"
    );

    // Negative control: without the external devices the firmware must take
    // its failure path, not print the success message.
    let mut no_devices = system.clone();
    no_devices.external_devices.clear();
    let failed = run_builder(&no_devices, &elf);
    assert!(
        failed.ends_with("M3 FAIL\n"),
        "the end-to-end test cannot detect a missing device: {failed:?}"
    );
}
