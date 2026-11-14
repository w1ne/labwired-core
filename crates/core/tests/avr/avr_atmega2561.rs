// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ATmega2561: the 64-pin public SKU on the shared AVR mega2560 interpreter.
//!
//! Microchip DS40002211A Table 2-1 gives this part 256 KB flash, 4 KB EEPROM,
//! 8 KB SRAM, 54 GPIO, two USARTs and an 8-channel ADC. The public core routes
//! `atmega2561` to [`AvrModel::Atmega2560`] (the same alias public simavr uses
//! in `sim_mega2560.c`), so this file pins the things that make it the 2561 and
//! not the 2560: the exact memory geometry, the 54-pin A..G map, only USART0,
//! and exactly eight ADC inputs. The non-claims (EEPROM controller, USART1,
//! Timer3, ports H..L, ADC8..15) are asserted absent from the descriptor.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn chip() -> ChipDescriptor {
    ChipDescriptor::from_file(root().join("configs/chips/atmega2561.yaml"))
        .expect("load configs/chips/atmega2561.yaml")
}

fn boot() -> (Machine<Avr>, Arc<Mutex<Vec<u8>>>) {
    let yaml = root().join("examples/atmega2561-io/system.yaml");
    let manifest = SystemManifest::from_file(&yaml).expect("load system.yaml");
    let chip_path = yaml.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/atmega2561-io.elf"))
        .expect("missing fixture tests/fixtures/avr/atmega2561-io.elf");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::for_chip_name(&chip.name);
    assert_eq!(cpu.model, AvrModel::Atmega2560);
    cpu.load_program_image(&image);
    let serial = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(serial.clone());
    (Machine::new(cpu, bus), serial)
}

#[test]
fn descriptor_is_the_64_pin_2561_and_not_the_100_pin_2560() {
    let chip = chip();
    assert_eq!(chip.name, "atmega2561");
    assert_eq!(chip.arch, labwired_config::Arch::Avr);
    assert_eq!(chip.core.as_deref(), Some("avr8"));
    assert_eq!(chip.cpu_hz, 16_000_000);
    assert_eq!(chip.flash.base, 0);
    assert_eq!(chip.flash.size, 256 * 1024);
    assert_eq!(chip.ram.base, 0x0200);
    assert_eq!(chip.ram.size, 8 * 1024);

    // DS40002211A Figure 1-3: PA..PF are eight bits, PG is six (54 total).
    assert_eq!(chip.pins.len(), 54);
    for port in ["PA", "PB", "PC", "PD", "PE", "PF"] {
        for bit in 0..8 {
            assert!(
                chip.pins.contains_key(&format!("{port}{bit}")),
                "missing {port}{bit}"
            );
        }
    }
    for bit in 0..6 {
        assert!(
            chip.pins.contains_key(&format!("PG{bit}")),
            "missing PG{bit}"
        );
    }
    for absent in ["PG6", "PG7", "PH0", "PJ0", "PK0", "PL0"] {
        assert!(
            !chip.pins.contains_key(absent),
            "64-pin part has no {absent}"
        );
    }

    let ids: Vec<&str> = chip.peripherals.iter().map(|p| p.id.as_str()).collect();
    for expected in [
        "porta", "portb", "portc", "portd", "porte", "portf", "portg", "usart0", "adc", "spi",
        "i2c",
    ] {
        assert!(ids.contains(&expected), "missing peripheral {expected}");
    }
    for absent in [
        "usart1", "usart2", "usart3", "porth", "portj", "portk", "portl",
    ] {
        assert!(
            !ids.contains(&absent),
            "descriptor claims unmodelled/100-pin block {absent}"
        );
    }
    // DS40002211A gives the 2561 Timer3, but this slice does not model it.
    // Timer4/5 belong to the 100-pin parts; Table 2-1 lists six vs twelve
    // 16-bit-resolution PWM channels for the 2561 and 2560 respectively.
    for absent in ["timer3", "timer4", "timer5"] {
        assert!(
            !ids.contains(&absent),
            "timer block {absent} is not modelled in this slice"
        );
    }

    let adc = chip
        .peripherals
        .iter()
        .find(|p| p.id == "adc")
        .expect("adc");
    assert_eq!(
        adc.config.get("channels").and_then(|v| v.as_u64()),
        Some(8),
        "only ADC0..ADC7 are bonded on the 64-pin part"
    );
}

#[test]
fn chip_name_selects_the_public_mega_core_with_2561_geometry() {
    let cpu = Avr::for_chip_name("atmega2561");
    assert_eq!(cpu.model, AvrModel::Atmega2560);
    assert_eq!(cpu.flash.len(), 256 * 1024);
    assert_eq!(cpu.sram.len(), 8 * 1024);
    assert_eq!(cpu.sp, 0x21FF);
    assert!(cpu.model.three_byte_pc());
}

#[test]
fn atmega2561_boots_and_prints() {
    let (mut machine, serial) = boot();
    let portb = machine
        .bus
        .find_peripheral_index_by_name("portb")
        .expect("portb is declared");
    for _ in 0..10_000 {
        machine.step().expect("step");
        let printed = serial
            .lock()
            .unwrap()
            .windows(8)
            .any(|w| w == b"2561 OK\n".as_slice());
        if printed && machine.bus.peripherals[portb].dev.read_gpio_output(5) == Some(true) {
            break;
        }
    }
    assert_eq!(
        String::from_utf8_lossy(&serial.lock().unwrap()).trim_end(),
        "2561 OK"
    );
    assert_eq!(
        machine.bus.peripherals[portb].dev.read_gpio_output(5),
        Some(true),
        "firmware drove PB5 high through DDRB/PORTB"
    );
}

#[test]
fn only_eight_adc_inputs_are_mapped() {
    let (mut machine, _) = boot();
    let adc = machine
        .bus
        .find_peripheral_index_by_name("adc")
        .expect("adc is declared");
    let dev = &mut machine.bus.peripherals[adc].dev;
    assert_eq!(dev.adc_channel_count(), Some(8));
    assert!(dev.set_adc_channel_input(7, 5000), "ADC7 is bonded");
    assert!(
        !dev.set_adc_channel_input(8, 5000),
        "ADC8..15 are 100-pin pins and are not mapped here"
    );
}

#[test]
fn package_pin_map_is_authoritative_for_gpio_labels() {
    let (machine, _) = boot();
    assert_eq!(
        SystemBus::resolve_pin_odr_pub(&machine.bus, "PG5"),
        Some((0x10032 + 2, 5)),
        "PG5 is PORTG bit 5 through the datasheet pin map"
    );
    assert!(
        SystemBus::resolve_pin_odr_pub(&machine.bus, "PH0").is_none(),
        "PH0 is a 100-pin pin and must not fall back to a guessed port"
    );
}
