// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Stock Arduino-ESP32 boots on the ESP32-S2 profile and prints over UART0.
//!
//! Fixture: `tests/fixtures/esp32s2-arduino-l0.elf` is the Arduino matrix L0
//! sketch (`validation/arduino-matrix/sketches/L0_serial_boot`) built for
//! PlatformIO `esp32-s2-saola-1` (espressif32 8.0.0), debug info stripped;
//! `.partitions.bin` is that build's partition table. The run goes through the
//! genuine ESP32-S2 mask ROM (`crates/core/roms/esp32s2`), ESP-IDF's startup,
//! FreeRTOS and the Arduino core to `Serial.println("LW_L0_OK")`.

use labwired_core::system::builder::build_system_bus;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Boot the fixture; return the UART0 bytes after at most `max_steps`, and
/// whether the run stopped on an error.
fn boot(max_steps: u64, partitions: bool) -> (Vec<u8>, Option<String>) {
    let sys = root().join("validation/arduino-matrix/systems/esp32s2.yaml");
    let resolved = labwired_config::ResolvedSystem::from_manifest_file(&sys).expect("system");
    let mut bus = build_system_bus(Some(&resolved)).expect("bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let elf = std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l0.elf")).unwrap();
    let table =
        std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l0.partitions.bin")).unwrap();
    let flash = labwired_core::system::xtensa::attach_esp32s2_flash(&mut bus, 4 * 1024 * 1024);
    let mut cpu = labwired_core::cpu::XtensaLx7::new();
    cpu.reset(&mut bus).unwrap();
    labwired_core::boot::esp32s2::fast_boot(
        &elf,
        &mut bus,
        &mut cpu,
        &flash,
        partitions.then_some(table.as_slice()),
    )
    .expect("fast_boot");
    let mut machine = Machine::new(cpu, bus);
    let mut err = None;
    for _ in 0..max_steps {
        if let Err(e) = machine.step() {
            err = Some(format!("{e}"));
            break;
        }
        if uart.lock().unwrap().windows(8).any(|w| w == b"LW_L0_OK") {
            break;
        }
    }
    let out = uart.lock().unwrap().clone();
    (out, err)
}

#[test]
fn stock_arduino_l0_prints_over_uart0() {
    // The CLI run takes ~606k steps; the budget leaves room, not a hiding place.
    let (out, err) = boot(1_500_000, true);
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("LW_L0_OK"),
        "no LW_L0_OK (stopped: {err:?}); UART0: {text:?}"
    );
}

/// Negative control: without the partition table the app's `esp_partition`
/// finds no table and aborts before `setup()`, so the marker cannot appear.
/// This is what makes the pass above depend on the flash model (partition
/// table read through SPI1/MMU), not just on the CPU reaching `setup()`.
#[test]
fn without_partition_table_the_app_never_reaches_setup() {
    let (out, _err) = boot(1_500_000, false);
    assert!(
        !String::from_utf8_lossy(&out).contains("LW_L0_OK"),
        "L0 printed with an erased partition table"
    );
}
