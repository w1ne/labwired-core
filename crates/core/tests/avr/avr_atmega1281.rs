// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ATmega1281: the 64-pin 128 KB mega core with a two-byte PC.

use labwired_config::{Arch, ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::Machine;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn chip() -> ChipDescriptor {
    ChipDescriptor::from_file(root().join("configs/chips/atmega1281.yaml"))
        .expect("load configs/chips/atmega1281.yaml")
}

fn machine() -> Machine<Avr> {
    let yaml = root().join("configs/systems/atmega1281.yaml");
    let manifest = SystemManifest::from_file(&yaml).expect("load system.yaml");
    let chip_path = yaml.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let cpu = Avr::for_chip_name(&chip.name);
    assert_eq!(cpu.model, AvrModel::Atmega1281);
    Machine::new(cpu, bus)
}

#[test]
fn descriptor_is_the_64_pin_1281_with_128_kb_flash() {
    let chip = chip();
    assert_eq!(chip.name, "atmega1281");
    assert_eq!(chip.arch, Arch::Avr);
    assert_eq!(chip.core.as_deref(), Some("avr8"));
    assert_eq!(chip.cpu_hz, 16_000_000);
    assert_eq!(chip.flash.base, 0);
    assert_eq!(chip.flash.size, 128 * 1024);
    assert_eq!(chip.ram.base, 0x0200);
    assert_eq!(chip.ram.size, 8 * 1024);
    assert_eq!(chip.pins.len(), 54);
    for absent in ["PG6", "PG7", "PH0", "PJ0", "PK0", "PL0"] {
        assert!(
            !chip.pins.contains_key(absent),
            "64-pin part has no {absent}"
        );
    }
    let adc = chip
        .peripherals
        .iter()
        .find(|p| p.id == "adc")
        .expect("adc");
    assert_eq!(adc.config.get("channels").and_then(|v| v.as_u64()), Some(8));
    let ids: Vec<&str> = chip.peripherals.iter().map(|p| p.id.as_str()).collect();
    for absent in [
        "usart1", "usart2", "usart3", "porth", "portj", "portk", "portl", "timer3", "timer4",
        "timer5",
    ] {
        assert!(!ids.contains(&absent), "descriptor does not claim {absent}");
    }
}

#[test]
fn chip_name_selects_128_kb_two_byte_pc_model() {
    let cpu = Avr::for_chip_name("atmega1281");
    assert_eq!(cpu.model, AvrModel::Atmega1281);
    assert_eq!(cpu.flash.len(), 128 * 1024);
    assert_eq!(cpu.sram.len(), 8 * 1024);
    assert_eq!(cpu.sp, 0x21FF);
    assert!(!cpu.model.three_byte_pc());
    assert!(!cpu.model.supports_eind());
}

#[test]
fn instruction_smoke_sets_status_led_on_pb5() {
    let mut machine = machine();
    let portb = machine
        .bus
        .find_peripheral_index_by_name("portb")
        .expect("portb");
    // SBI DDRB,5; SBI PORTB,5; RJMP .  The small hand-encoded program exercises
    // the 1281 core through the exact package descriptor and bus bindings.
    machine.cpu.load_words(0, &[0x9A25, 0x9A2D, 0xCFFF]);
    machine.step().expect("set PB5 output");
    machine.step().expect("drive PB5 high");
    assert_eq!(
        machine.bus.peripherals[portb].dev.read_gpio_output(5),
        Some(true)
    );
}
