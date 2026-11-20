// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The Arduino matrix L8 sketch, on stock Arduino-ESP32 on the ESP32-S2, sends
//! a CAN frame to itself through the TWAI controller at 0x3F42_B000.
//!
//! Fixture: `tests/fixtures/esp32s2-arduino-l8.elf` is the Arduino matrix L8
//! sketch (`validation/arduino-matrix/sketches/L8_can`) built for PlatformIO
//! `esp32-s2-saola-1` (espressif32 8.0.0), debug info stripped. It uses the
//! L0 build's partition table (the two are byte-identical). The sketch leaves
//! reset mode, loads a standard frame (ID 0x123, DLC 1, data 0xA5), issues
//! `CMD.self_rx_req`, and checks STATUS.rx_buf_st, RX_MSG_CNT and the frame.
//!
//! There is no CAN bus: this is register-level self-reception only.

use labwired_core::system::builder::build_system_bus;
use labwired_core::{Bus, Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const TWAI: u64 = 0x3F42_B000;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn twai_self_reception_returns_the_frame() {
    let sys = root().join("validation/arduino-matrix/systems/esp32s2.yaml");
    let resolved = labwired_config::ResolvedSystem::from_manifest_file(&sys).expect("system");
    let mut bus = build_system_bus(Some(&resolved)).expect("bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let elf = std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l8.elf")).unwrap();
    let table =
        std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l0.partitions.bin")).unwrap();
    let flash = labwired_core::system::xtensa::attach_esp32s2_flash(&mut bus, 4 * 1024 * 1024);
    let mut cpu = labwired_core::cpu::XtensaLx7::new();
    cpu.reset(&mut bus).unwrap();
    labwired_core::boot::esp32s2::fast_boot(&elf, &mut bus, &mut cpu, &flash, Some(&table))
        .expect("fast_boot");
    let mut machine = Machine::new(cpu, bus);
    for _ in 0..12_000_000u64 {
        machine.step().expect("step");
        let out = uart.lock().unwrap();
        if out.windows(8).any(|w| w == b"LW_L8_OK") || out.windows(10).any(|w| w == b"LW_L8_FAIL") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&uart.lock().unwrap()).into_owned();
    assert!(text.contains("LW_L8_OK"), "no LW_L8_OK; UART0: {text:?}");

    // The controller state the sketch left behind: operating mode, one
    // received frame held in the RX buffer, and the frame itself.
    let r = |off: u64| machine.bus.read_u32(TWAI + off).unwrap() & 0xFF;
    assert_eq!(r(0x00) & 1, 0, "MODE.reset_mode still set");
    assert_eq!(r(0x08) & 1, 1, "STATUS.rx_buf_st clear");
    assert_eq!(r(0x74), 1, "RX_MSG_CNT");
    assert_eq!(
        [r(0x40), r(0x44), r(0x48), r(0x4C)],
        [0x01, 0x24, 0x60, 0xA5]
    );
}
