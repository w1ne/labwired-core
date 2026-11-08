// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Stock Arduino-ESP32 `Wire` on the ESP32-S2 reads an INA219's registers
//! through the I2C0 command list.
//!
//! Fixture: `tests/fixtures/esp32s2-arduino-l3.elf` is the Arduino matrix L3
//! sketch (`validation/arduino-matrix/sketches/L3_i2c_sensor`) built for
//! PlatformIO `esp32-s2-saola-1` (espressif32 8.0.0), debug info stripped. It
//! uses the L0 build's partition table (the two are byte-identical). The
//! sketch writes the INA219 register pointer, then reads CONFIG and BUS back
//! with `requestFrom` and prints `LW_L3_OK` only if they equal the INA219
//! model's 0x399F / 0x19CA. An ACK-only pass would add `LW_L3_PARTIAL_NO_RX`.

use labwired_core::system::builder::build_system_bus;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Boot the L3 fixture on the matrix S2 system and return UART0 once the
/// sketch has printed a verdict (or the step budget runs out).
fn run_l3(with_ina219: bool) -> String {
    let sys = root().join("validation/arduino-matrix/systems/esp32s2.yaml");
    let mut resolved = labwired_config::ResolvedSystem::from_manifest_file(&sys).expect("system");
    if !with_ina219 {
        resolved.manifest.external_devices.clear();
    }
    let mut bus = build_system_bus(Some(&resolved)).expect("bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let elf = std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l3.elf")).unwrap();
    let table =
        std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l0.partitions.bin")).unwrap();
    let flash = labwired_core::system::xtensa::attach_esp32s2_flash(&mut bus, 4 * 1024 * 1024);
    let mut cpu = labwired_core::cpu::XtensaLx7::new();
    cpu.reset(&mut bus).unwrap();
    labwired_core::boot::esp32s2::fast_boot(&elf, &mut bus, &mut cpu, &flash, Some(&table))
        .expect("fast_boot");
    let mut machine = Machine::new(cpu, bus);
    // Keep running past the verdict so a trailing `LW_L3_PARTIAL_NO_RX`
    // line, printed right after `LW_L3_OK`, is not cut off.
    let mut settle: Option<u32> = None;
    for _ in 0..12_000_000u64 {
        machine.step().expect("step");
        match settle.as_mut() {
            Some(0) => break,
            Some(n) => *n -= 1,
            None => {
                let out = uart.lock().unwrap();
                if out.windows(8).any(|w| w == b"LW_L3_OK")
                    || out.windows(10).any(|w| w == b"LW_L3_FAIL")
                {
                    settle = Some(500_000);
                }
            }
        }
    }
    let text = String::from_utf8_lossy(&uart.lock().unwrap()).into_owned();
    text
}

#[test]
fn wire_reads_ina219_config_and_bus_registers() {
    let text = run_l3(true);
    assert!(text.contains("LW_L3_OK"), "no LW_L3_OK; UART0: {text:?}");
    assert!(
        !text.contains("LW_L3_PARTIAL_NO_RX"),
        "ACK-only pass, master-receive did not return the registers: {text:?}"
    );
}

#[test]
fn without_the_ina219_the_address_is_nacked() {
    let text = run_l3(false);
    assert!(text.contains("LW_L3_FAIL nack"), "UART0: {text:?}");
    assert!(!text.contains("LW_L3_OK"), "UART0: {text:?}");
}
