// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino `analogWrite` on an Uno drives the output-compare pads.
//!
//! The committed sketch (`tests/fixtures/avr/arduino-uno-pwm.cpp`) writes
//! 200 to D3 (OC2B), 50 to D11 (OC2A) and 128 to D5 (OC0B), then after 100 ms
//! writes 0 to D3, which the Arduino core turns into a plain LOW. Before the
//! COM bits drove the pads, all three stayed at their LOW latch, so an
//! H-bridge with its IN1 on D3 never ran forward. Here the pads carry the
//! duty, and a DRV8833-style plant with IN1 = D3 (PWM), IN2 = D4 (LOW) turns
//! forward until D3 drops.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::Machine;
use std::path::PathBuf;

const CPU_HZ: u64 = 16_000_000;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn boot() -> Machine<Avr> {
    let chip =
        ChipDescriptor::from_file(root().join("configs/chips/atmega328p.yaml")).expect("chip");
    let manifest = SystemManifest::from_yaml(
        r#"
name: "uno-pwm"
chip: "atmega328p.yaml"
external_devices:
  - id: "M1"
    type: "dc-motor"
    connection: "gpio"
    config:
      both_inputs_low: "coast"
      in1_pin: "PD3"
      in2_pin: "PD4"
      pwm_pin: "PD3"
      enable_pin: "5V"
      simulation_clock_hz: 16000000
      resistance_ohm: 1.2
      inductance_h: 0.002
      torque_constant_nm_per_a: 0.08
      back_emf_constant_v_per_rad_s: 0.08
      rotor_inertia_kg_m2: 0.00004
      viscous_friction_nm_per_rad_s: 0.00001
      supply_voltage_v: 12
      load_torque_nm: 0
      encoder_cpr: 1024
"#,
    )
    .expect("manifest");
    let bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/arduino-uno-pwm.elf"))
        .expect("missing tests/fixtures/avr/arduino-uno-pwm.elf");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::new();
    cpu.load_program_image(&image);
    Machine::new(cpu, bus)
}

fn pad(m: &Machine<Avr>, port: &str, pin: u8) -> bool {
    let idx = m.bus.find_peripheral_index_by_name(port).expect("port");
    m.bus.peripherals[idx].dev.read_gpio_output(pin).unwrap_or(false)
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
fn analog_write_duty_reaches_the_pads_and_turns_the_motor_forward() {
    let mut m = boot();
    let pads = [("portd", 3), ("portb", 3), ("portd", 5)];
    // 20 ms window well inside the 100 ms the sketch holds the duties.
    let got = measure(&mut m, &pads, CPU_HZ * 50 / 1000, CPU_HZ * 70 / 1000);
    let (d3, d11, d5) = (got[0], got[1], got[2]);
    assert!((d3.0 - 200.0 / 256.0).abs() < 0.02, "D3 duty {d3:?}");
    assert!((d11.0 - 50.0 / 256.0).abs() < 0.02, "D11 duty {d11:?}");
    assert!((d5.0 - 129.0 / 256.0).abs() < 0.02, "D5 duty {d5:?}");
    // Timer2 and Timer0 at clk/64 with an 8-bit sawtooth: ~976 Hz, ~19 periods.
    for (name, (_, rises)) in [("D3", d3), ("D11", d11), ("D5", d5)] {
        assert!((17..=21).contains(&rises), "{name} rising edges {rises}");
    }
    let rpm = m.bus.motor_speed_rpm("M1").expect("M1");
    assert!(rpm > 100.0, "IN1 PWM + IN2 LOW drives forward, got {rpm} rpm");

    // analogWrite(3, 0) releases OC2B to the LOW latch: the pad stays low.
    let after = measure(&mut m, &pads[..1], CPU_HZ * 110 / 1000, CPU_HZ * 130 / 1000);
    assert_eq!(after[0], (0.0, 0), "D3 after analogWrite(3, 0)");
}
