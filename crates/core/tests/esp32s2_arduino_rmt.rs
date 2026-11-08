// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Stock Arduino-ESP32 drives the ESP32-S2's RGB pixel through the RMT, and
//! the WS2812 waveform is on the pad.
//!
//! Fixture: `tests/fixtures/esp32s2-arduino-l2.elf` is the Arduino matrix L2
//! sketch (`validation/arduino-matrix/sketches/L2_blink_serial`) built for
//! PlatformIO `esp32-s2-saola-1` (espressif32 8.0.0), debug info stripped. It
//! uses the L0 build's partition table (the two are byte-identical). On this
//! board `LED_BUILTIN` is the WS2812 on GPIO18, so `digitalWrite` becomes
//! `rgbLedWrite`: ESP-IDF's RMT TX driver encodes 24 bits into RMTMEM, starts
//! channel 0, and waits for the TX_END interrupt.
//!
//! The probe is the logic analyzer on the GPIO18 pad, not a register: the
//! test fails if the RMT plays its symbols but the GPIO matrix does not carry
//! them to the pad.

use labwired_core::logic_capture::LogicSource;
use labwired_core::system::builder::build_system_bus;
use labwired_core::{Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The S2 core clock the profile runs at; RMT ticks are APB (80 MHz) / 8.
const CPU_HZ: u64 = 240_000_000;
const PIXEL_PAD: u8 = 18;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// (high, low) cycle pairs on the pixel pad until `LW_L2_OK`, and the UART.
fn run_l2() -> (Vec<(u64, bool)>, String) {
    let sys = root().join("validation/arduino-matrix/systems/esp32s2.yaml");
    let resolved = labwired_config::ResolvedSystem::from_manifest_file(&sys).expect("system");
    let mut bus = build_system_bus(Some(&resolved)).expect("bus");
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let elf = std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l2.elf")).unwrap();
    let table =
        std::fs::read(root().join("tests/fixtures/esp32s2-arduino-l0.partitions.bin")).unwrap();
    let flash = labwired_core::system::xtensa::attach_esp32s2_flash(&mut bus, 4 * 1024 * 1024);
    let gpio = bus.find_peripheral_index_by_name("gpio").expect("gpio");
    let mut cpu = labwired_core::cpu::XtensaLx7::new();
    cpu.reset(&mut bus).unwrap();
    labwired_core::boot::esp32s2::fast_boot(&elf, &mut bus, &mut cpu, &flash, Some(&table))
        .expect("fast_boot");
    let mut machine = Machine::new(cpu, bus);
    machine.logic_watch(&[Some(LogicSource::pad(gpio, PIXEL_PAD))]);
    // The CLI run reaches LW_L2_OK at ~1.33M steps.
    for _ in 0..3_000_000u64 {
        machine.step().expect("step");
        if uart.lock().unwrap().windows(8).any(|w| w == b"LW_L2_OK") {
            break;
        }
    }
    let edges = machine
        .logic_read_edges(0)
        .edges
        .iter()
        .map(|e| (e.cycle, e.value))
        .collect();
    let text = String::from_utf8_lossy(&uart.lock().unwrap()).into_owned();
    (edges, text)
}

/// Split a pad's edges into WS2812 frames (a reset gap > 50 µs separates
/// them) and decode each bit from its high time: T0H 0.4 µs, T1H 0.8 µs.
fn decode(edges: &[(u64, bool)]) -> Vec<(u32, usize)> {
    let ns = |cycles: u64| cycles * 1_000_000_000 / CPU_HZ;
    let mut frames = Vec::new();
    let (mut value, mut bits, mut last_fall) = (0u32, 0usize, None::<u64>);
    for pair in edges.windows(2) {
        let [(rise, true), (fall, false)] = *pair else {
            continue;
        };
        if last_fall.is_some_and(|f| ns(rise - f) > 50_000) && bits > 0 {
            frames.push((value, bits));
            (value, bits) = (0, 0);
        }
        let high = ns(fall - rise);
        assert!(
            (350..=450).contains(&high) || (750..=850).contains(&high),
            "WS2812 high time {high} ns is neither T0H nor T1H"
        );
        value = (value << 1) | u32::from(high > 600);
        bits += 1;
        last_fall = Some(fall);
    }
    if bits > 0 {
        frames.push((value, bits));
    }
    frames
}

#[test]
fn digital_write_on_led_builtin_puts_ws2812_frames_on_gpio18() {
    let (edges, uart) = run_l2();
    assert!(uart.contains("LW_L2_OK"), "no LW_L2_OK; UART0: {uart:?}");
    let frames = decode(&edges);
    // digitalWrite(HIGH) = rgbLedWrite(RGB_BRIGHTNESS = 64, 64, 64) in GRB
    // order; digitalWrite(LOW) = all off. One of each before LW_L2_OK.
    assert_eq!(
        frames,
        vec![(0x40_40_40, 24), (0x00_00_00, 24)],
        "decoded frames (GRB, bit count) from {} edges",
        edges.len()
    );
}
