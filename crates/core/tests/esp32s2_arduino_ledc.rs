// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Stock Arduino-ESP32 `analogWrite` on the ESP32-S2 puts a PWM waveform on
//! the pad, through the LEDC and the GPIO matrix.
//!
//! Fixture: `tests/fixtures/esp32s2-arduino-l6.elf` is the Arduino matrix L6
//! sketch (`validation/arduino-matrix/sketches/L6_pwm`) built for PlatformIO
//! `esp32-s2-saola-1` (espressif32 8.0.0), debug info stripped. It uses the
//! L0 build's partition table (the two are byte-identical). The sketch calls
//! `analogWrite(2, 128)`: `ledcAttach` at the stock 1 kHz / 8-bit, so the pad
//! must show 500 µs high / 500 µs low.
//!
//! The probe is the logic analyzer on the GPIO2 pad, not a register: the test
//! fails if the LEDC computes the duty but the matrix does not carry it.

use labwired_core::logic_capture::LogicSource;
use labwired_core::system::builder::build_system_bus;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const CPU_HZ: u64 = 240_000_000;
const PWM_PAD: u8 = 2;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn analog_write_puts_1khz_half_duty_pwm_on_gpio2() {
    let sys = root().join("validation/arduino-matrix/systems/esp32s2.yaml");
    let resolved = labwired_config::ResolvedSystem::from_manifest_file(&sys).expect("system");
    let mut bus = build_system_bus(Some(&resolved)).expect("bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let elf = std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l6.elf")).unwrap();
    let table =
        std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l0.partitions.bin")).unwrap();
    let flash = labwired_core::system::xtensa::attach_esp32s2_flash(&mut bus, 4 * 1024 * 1024);
    let gpio = bus.find_peripheral_index_by_name("gpio").expect("gpio");
    let mut cpu = labwired_core::cpu::XtensaLx7::new();
    cpu.reset(&mut bus).unwrap();
    labwired_core::boot::esp32s2::fast_boot(&elf, &mut bus, &mut cpu, &flash, Some(&table))
        .expect("fast_boot");
    let mut machine = Machine::new(cpu, bus);
    machine.logic_watch(&[Some(LogicSource::pad(gpio, PWM_PAD))]);
    // The CLI run reaches LW_L6_OK at ~7.8M steps.
    for _ in 0..12_000_000u64 {
        machine.step().expect("step");
        if uart.lock().unwrap().windows(8).any(|w| w == b"LW_L6_OK") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&uart.lock().unwrap()).into_owned();
    assert!(text.contains("LW_L6_OK"), "no LW_L6_OK; UART0: {text:?}");

    let edges: Vec<(u64, bool)> = machine
        .logic_read_edges(0)
        .edges
        .iter()
        .map(|e| (e.cycle, e.value))
        .collect();
    // 30 ms of delay(1) at 1 kHz: ~60 edges. Demand a sustained waveform.
    assert!(edges.len() >= 40, "only {} edges on GPIO2", edges.len());
    let us = |cycles: u64| cycles as f64 * 1e6 / CPU_HZ as f64;
    for pair in edges.windows(2) {
        let [(a, level), (b, _)] = *pair else {
            continue;
        };
        let width = us(b - a);
        assert!(
            (width - 500.0).abs() < 1.0,
            "{} phase of {width} µs at cycle {a}; want 500 µs (1 kHz, duty 128/256)",
            if level { "high" } else { "low" }
        );
    }
}
