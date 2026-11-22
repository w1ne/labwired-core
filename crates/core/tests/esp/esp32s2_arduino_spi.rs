// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Stock Arduino-ESP32 `SPI` on the ESP32-S2 reads a MAX31855 frame through
//! GP-SPI2 (FSPI).
//!
//! Fixture: `tests/fixtures/esp32s2-arduino-l4.elf` is the Arduino matrix L4
//! sketch (`validation/arduino-matrix/sketches/L4_spi_sensor`) built for
//! PlatformIO `esp32-s2-saola-1` (espressif32 8.0.0), debug info stripped. It
//! uses the L0 build's partition table (the two are byte-identical). The
//! sketch clocks four bytes with `SPI.transfer(0)` and prints `LW_L4_OK` only
//! for the MAX31855 model's default frame 0x01901600 (25.0 C, no fault).
//! With no device on the bus MISO idles high, so the frame is 0xFFFFFFFF.

use labwired_core::system::builder::build_system_bus;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Boot the L4 fixture on the matrix S2 system and return UART0 once the
/// sketch has printed a verdict (or the step budget runs out).
fn run_l4(with_max31855: bool) -> String {
    let sys = root().join("validation/arduino-matrix/systems/esp32s2.yaml");
    let mut resolved = labwired_config::ResolvedSystem::from_manifest_file(&sys).expect("system");
    if !with_max31855 {
        resolved
            .manifest
            .external_devices
            .retain(|d| d.id != "max31855");
    }
    let mut bus = build_system_bus(Some(&resolved)).expect("bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let elf = std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l4.elf")).unwrap();
    let table =
        std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l0.partitions.bin")).unwrap();
    let flash = labwired_core::system::xtensa::attach_esp32s2_flash(&mut bus, 4 * 1024 * 1024);
    let mut cpu = labwired_core::cpu::XtensaLx7::new();
    cpu.reset(&mut bus).unwrap();
    labwired_core::boot::esp32s2::fast_boot(&elf, &mut bus, &mut cpu, &flash, Some(&table))
        .expect("fast_boot");
    let mut machine = Machine::new(cpu, bus);
    // Keep running a little past the verdict so the FAIL line's frame value
    // is not cut off.
    let mut settle: Option<u32> = None;
    for _ in 0..12_000_000u64 {
        machine.step().expect("step");
        match settle.as_mut() {
            Some(0) => break,
            Some(n) => *n -= 1,
            None => {
                let out = uart.lock().unwrap();
                if out.windows(8).any(|w| w == b"LW_L4_OK")
                    || out.windows(10).any(|w| w == b"LW_L4_FAIL")
                {
                    settle = Some(200_000);
                }
            }
        }
    }
    let text = String::from_utf8_lossy(&uart.lock().unwrap()).into_owned();
    text
}

#[test]
fn spi_reads_the_max31855_frame() {
    let text = run_l4(true);
    assert!(text.contains("LW_L4_OK"), "no LW_L4_OK; UART0: {text:?}");
}

#[test]
fn without_the_max31855_miso_idles_high() {
    let text = run_l4(false);
    assert!(
        text.contains("LW_L4_FAIL frame=0xffffffff"),
        "UART0: {text:?}"
    );
    assert!(!text.contains("LW_L4_OK"), "UART0: {text:?}");
}
