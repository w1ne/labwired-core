use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn atmega328pb_firmware_uses_usart1_and_drives_pb5() {
    let system_path = root().join("examples/atmega328pb-usart1/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load system.yaml");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/atmega328pb-usart1.elf"))
        .expect("missing compiled ATmega328PB USART1 ELF");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse AVR-GCC ELF");
    let mut cpu = Avr::for_chip_name(&chip.name);
    assert_eq!(cpu.model, AvrModel::Atmega328pb);
    cpu.load_program_image(&image);
    let serial = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(Arc::clone(&serial));
    let mut machine = Machine::new(cpu, bus);
    let portb = machine
        .bus
        .find_peripheral_index_by_name("portb")
        .expect("portb");
    let expected = b"PB_USART1_OK\n";

    for _ in 0..100_000 {
        machine.step().expect("step ATmega328PB ELF");
        let printed = serial
            .lock()
            .unwrap()
            .windows(expected.len())
            .any(|window| window == expected);
        if printed && machine.bus.peripherals[portb].dev.read_gpio_output(5) == Some(true) {
            break;
        }
    }

    assert_eq!(
        String::from_utf8_lossy(&serial.lock().unwrap()).trim_end(),
        "PB_USART1_OK"
    );
    assert_eq!(
        machine.bus.peripherals[portb].dev.read_gpio_output(5),
        Some(true),
        "firmware drives PB5 high"
    );
}
