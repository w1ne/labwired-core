// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The Arduino `Servo` library on an Uno moves a servo twin.
//!
//! The library runs Timer1 in normal mode at clk/8 and raises the pulse from
//! its TIMER1_COMPA ISR with `digitalWrite`, moving OCR1A along the 20 ms
//! frame. Before Timer1 existed the ISR never ran, so the pad stayed still.
//! Also, an AVR port has no GPIO observer list, so the servo never saw an
//! edge anyway. The committed sketch (`tests/fixtures/avr/arduino-uno-servo.cpp`)
//! attaches an SG90 (500..2400 us) on D6 and writes 150, 30 and then 90,
//! 300 ms apart.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::peripherals::components::Servo;
use labwired_core::Machine;
use std::path::PathBuf;

const CPU_HZ: u64 = 16_000_000;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn angle(m: &Machine<Avr>) -> f32 {
    let servo: &Servo = m.bus.observed_of::<Servo>().next().expect("servo");
    servo.angle_degrees()
}

#[test]
fn servo_library_on_d6_steers_the_servo_150_30_90() {
    let chip =
        ChipDescriptor::from_file(root().join("configs/chips/atmega328p.yaml")).expect("chip");
    let manifest = SystemManifest::from_yaml(
        r#"
name: "uno-servo"
chip: "atmega328p.yaml"
external_devices:
  - id: "M5"
    type: "servo"
    connection: "portd"
    config:
      signal_pin: "PD6"
      model: "sg90"
"#,
    )
    .expect("manifest");
    let bus = SystemBus::from_config(&chip, &manifest).expect("bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/arduino-uno-servo.elf"))
        .expect("missing tests/fixtures/avr/arduino-uno-servo.elf");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::new();
    cpu.load_program_image(&image);
    let mut m = Machine::new(cpu, bus);

    // The servo is listed once in inspect, its pad watcher is not.
    let ids: Vec<String> = m
        .bus
        .inspect_devices(None, &Default::default())
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(ids, ["M5"]);

    for (until_ms, want) in [(250u64, 150.0f32), (550, 30.0), (850, 90.0)] {
        while m.cpu.cycles < CPU_HZ * until_ms / 1000 {
            m.step().expect("step");
        }
        let got = angle(&m);
        assert!(
            (got - want).abs() < 2.0,
            "at {until_ms} ms the servo is at {got} deg, want {want}"
        );
    }
}
