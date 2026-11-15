// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino `analogWrite` on a Mega 2560 drives the mega's output-compare pads.
//!
//! The committed sketch (`tests/fixtures/avr/arduino-mega-2560-pwm.cpp`)
//! writes a duty to every Timer0/1/2 OC pin: D13 = PB7 = OC0A (128),
//! D4 = PG5 = OC0B (64), D11 = PB5 = OC1A (200), D12 = PB6 = OC1B (50),
//! D10 = PB4 = OC2A (100), D9 = PH6 = OC2B (180), then after 100 ms writes 0
//! to D9. Before the mega pin map, `sync_oc_pins` skipped the ATmega2560 and
//! every pad followed its LOW `PORTx` latch, so `analogWrite(13, 128)`
//! on the Mega's LED never toggled.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::Machine;
use std::path::PathBuf;

const CPU_HZ: u64 = 16_000_000;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn boot() -> Machine<Avr> {
    let chip =
        ChipDescriptor::from_file(root().join("configs/chips/atmega2560.yaml")).expect("chip");
    let manifest = SystemManifest::from_yaml(
        r#"
name: "mega-pwm"
chip: "atmega2560.yaml"
external_devices: []
"#,
    )
    .expect("manifest");
    let bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/arduino-mega-2560-pwm.elf"))
        .expect("missing tests/fixtures/avr/arduino-mega-2560-pwm.elf");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::new_model(AvrModel::Atmega2560);
    cpu.load_program_image(&image);
    Machine::new(cpu, bus)
}

fn pad(m: &Machine<Avr>, port: &str, pin: u8) -> bool {
    let idx = m.bus.find_peripheral_index_by_name(port).expect("port");
    m.bus.peripherals[idx]
        .dev
        .read_gpio_output(pin)
        .unwrap_or(false)
}

/// High-time fraction and rising-edge count of each pad over `[from, to)`.
fn measure(m: &mut Machine<Avr>, pads: &[(&str, u8)], from: u64, to: u64) -> Vec<(f64, u32)> {
    while m.cpu.cycles < from {
        m.step().expect("step");
    }
    let mut high = vec![0u64; pads.len()];
    let mut rises = vec![0u32; pads.len()];
    let mut last: Vec<bool> = pads.iter().map(|(p, b)| pad(m, p, *b)).collect();
    let mut t = m.cpu.cycles;
    while m.cpu.cycles < to {
        m.step().expect("step");
        let dt = m.cpu.cycles - t;
        t = m.cpu.cycles;
        for (i, (p, b)) in pads.iter().enumerate() {
            let now = pad(m, p, *b);
            if last[i] {
                high[i] += dt;
            }
            if now && !last[i] {
                rises[i] += 1;
            }
            last[i] = now;
        }
    }
    let span = (t - from) as f64;
    high.iter()
        .zip(rises)
        .map(|(h, r)| (*h as f64 / span, r))
        .collect()
}

#[test]
fn analog_write_reaches_every_mega_timer0_1_2_pad() {
    let mut m = boot();
    // (pad, expected duty, rising edges in 20 ms)
    let fast = 17..=21; // Timer0 fast PWM, clk/64 / 256 = 976 Hz
    let phase = 9..=11; // Timer1/2 phase-correct, clk/64 / 510 = 490 Hz
    let cases = [
        ("D13 OC0A", ("portb", 7), 129.0 / 256.0, fast.clone()),
        ("D4 OC0B", ("portg", 5), 65.0 / 256.0, fast),
        ("D11 OC1A", ("portb", 5), 200.0 / 255.0, phase.clone()),
        ("D12 OC1B", ("portb", 6), 50.0 / 255.0, phase.clone()),
        ("D10 OC2A", ("portb", 4), 100.0 / 255.0, phase.clone()),
        ("D9 OC2B", ("porth", 6), 180.0 / 255.0, phase),
    ];
    let pads: Vec<(&str, u8)> = cases.iter().map(|c| c.1).collect();
    // 20 ms window well inside the 100 ms the sketch holds the duties.
    let got = measure(&mut m, &pads, CPU_HZ * 50 / 1000, CPU_HZ * 70 / 1000);
    for ((name, _, duty, edges), (high, rises)) in cases.iter().zip(&got) {
        assert!((high - duty).abs() < 0.02, "{name} duty {high} want {duty}");
        assert!(edges.contains(rises), "{name} rising edges {rises}");
    }

    // analogWrite(9, 0) releases OC2B to the LOW latch: PH6 stays low.
    let after = measure(&mut m, &pads[5..], CPU_HZ * 110 / 1000, CPU_HZ * 130 / 1000);
    assert_eq!(after[0], (0.0, 0), "D9 after analogWrite(9, 0)");
}
