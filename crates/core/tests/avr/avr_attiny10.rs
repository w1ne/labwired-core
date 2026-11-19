//! ATtiny10 (AVRrc reduced core) running avr-gcc output: flash-mapped
//! constants at 0x4000, `.data` copied from flash, one-word LDS/STS, RCALL on
//! the 32-byte stack, and PORTB. Source: tests/fixtures/avr/attiny10/.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::{Bus, Machine};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/avr/attiny10")
}

#[test]
fn attiny10_reduced_core_runs_avr_gcc_image() {
    let system_path = fixture().join("system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load system.yaml");
    let chip = ChipDescriptor::from_file(fixture().join(&manifest.chip)).expect("load chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let elf = std::fs::read(fixture().join("attiny10-rc.elf")).expect("fixture ELF");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse AVR-GCC ELF");
    let mut cpu = Avr::for_chip_name(&chip.name);
    assert_eq!(cpu.model, AvrModel::Attiny10);
    assert!(cpu.model.is_reduced_core());
    cpu.load_program_image(&image);
    let mut machine = Machine::new(cpu, bus);

    // The image ends in `rjmp .-2` at 0x8A; 200 steps is well past it.
    for _ in 0..200 {
        machine
            .step()
            .expect("AVRrc image runs without a decode fault");
    }

    // sum = 0x11 + 0x22 + 0x33 + 0x44, read through the 0x4000 flash map.
    assert_eq!(machine.bus.read_u8(0x42).unwrap(), 0xAA, "sum in SRAM");
    // counter = 5 from .data, incremented once.
    assert_eq!(machine.bus.read_u8(0x40).unwrap(), 6, "counter in SRAM");
    // DDRB = 0x0F; PORTB = (0xAA ^ 6) & 0x0F.
    assert_eq!(machine.bus.read_u8(0x10001).unwrap(), 0x0F, "DDRB");
    assert_eq!(machine.bus.read_u8(0x10002).unwrap(), 0x0C, "PORTB");
    let portb = machine
        .bus
        .find_peripheral_index_by_name("portb")
        .expect("portb");
    let dev = &machine.bus.peripherals[portb].dev;
    assert_eq!(dev.read_gpio_output(2), Some(true), "PB2 driven high");
    assert_eq!(dev.read_gpio_output(0), Some(false), "PB0 driven low");
}

#[test]
fn attiny10_refuses_a_classic_only_instruction() {
    let mut cpu = Avr::for_chip_name("attiny10");
    // ADIW r24, 1 does not exist on AVRrc.
    cpu.load_words(0, &[0x9601]);
    let chip = ChipDescriptor::from_file(fixture().join("chip.yaml")).expect("load chip");
    let manifest = SystemManifest::from_file(fixture().join("system.yaml")).expect("manifest");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let mut machine = Machine::new(cpu, bus);
    assert!(machine.step().is_err(), "ADIW must not execute on ATtiny10");
}
