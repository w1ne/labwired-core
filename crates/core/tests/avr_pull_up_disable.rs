// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ATmega `MCUCR.PUD` reaches the ports.
//!
//! `pinMode(pin, INPUT_PULLUP)` with nothing attached reads HIGH, because the
//! pull-up drives the pad. `MCUCR.PUD` switches every pull-up off, and the
//! same pad floats at its last level. The CPU owns `MCUCR`; the bus-side
//! ports see PUD only through `Bus::set_pull_ups_disabled`, so this runs the
//! real instructions on the real atmega328p chip yaml.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::pins::Pull;
use labwired_core::{Cpu, SimulationConfig};
use std::path::Path;

#[test]
fn mcucr_pud_disables_the_port_pull_ups() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/atmega328p.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        "name: pud\nchip: atmega328p.yaml\nexternal_devices: []\nboard_io: []\n",
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let portb = bus.find_peripheral_index_by_name("portb").unwrap();
    let pull = |bus: &SystemBus| {
        bus.peripherals[portb]
            .dev
            .pins()
            .and_then(|p| p.driver(2))
            .map(|d| d.pull)
    };

    let mut cpu = Avr::new();
    // LDI R16,0x04; OUT PORTB,R16; IN R17,PINB; LDI R18,0x10;
    // OUT MCUCR(io 0x35),R18; IN R19,PINB; OUT MCUCR,R1(=0); IN R20,PINB
    cpu.load_words(
        0,
        &[
            0xE004, 0xB905, 0xB113, 0xE120, 0xBF25, 0xB133, 0xBE15, 0xB143, 0xCFFF,
        ],
    );
    cpu.set_pc(0);
    let cfg = SimulationConfig::default();
    let step = |cpu: &mut Avr, bus: &mut SystemBus, n: usize| {
        for _ in 0..n {
            cpu.step(bus, &[], &cfg).unwrap();
        }
    };

    step(&mut cpu, &mut bus, 3);
    assert_eq!(cpu.r[17] & 0x04, 0x04, "pulled-up PB2 reads high");
    assert_eq!(pull(&bus), Some(Pull::Up));

    step(&mut cpu, &mut bus, 3);
    assert_eq!(cpu.io[0x35], 0x10, "MCUCR stored on the CPU");
    assert_eq!(cpu.r[19] & 0x04, 0, "PUD: no pull-up, PB2 floats at 0");
    assert_eq!(pull(&bus), Some(Pull::None), "a net no longer counts it");

    step(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.r[20] & 0x04, 0x04, "PUD cleared: the pull-up is back");
    assert_eq!(pull(&bus), Some(Pull::Up));
}
