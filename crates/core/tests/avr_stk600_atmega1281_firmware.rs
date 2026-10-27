use labwired_config::{BoardIoKind, BoardIoSignal, ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn assert_stk600_harness(manifest: &SystemManifest) {
    assert_eq!(manifest.name, "stk600-atmega1281");
    assert_eq!(manifest.cpu_hz, Some(16_000_000));
    assert!(manifest.board_io.iter().any(|pin| {
        pin.id == "led0"
            && pin.kind == BoardIoKind::Led
            && pin.peripheral == "portb"
            && pin.pin == 5
            && pin.signal == BoardIoSignal::Output
            && !pin.active_high
    }));
    assert!(manifest.board_io.iter().any(|pin| {
        pin.id == "sw0"
            && pin.kind == BoardIoKind::Button
            && pin.peripheral == "portb"
            && pin.pin == 0
            && pin.signal == BoardIoSignal::Input
            && !pin.active_high
    }));
}

#[test]
fn stk600_atmega1281_routes_active_low_switch_to_led_and_captures_usart0() {
    let system_path = root().join("examples/stk600-atmega1281/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load STK600 system.yaml");
    assert_stk600_harness(&manifest);

    let core_system_path = root().join("configs/systems/stk600-atmega1281.yaml");
    let core_manifest =
        SystemManifest::from_file(&core_system_path).expect("load canonical STK600 system");
    assert_stk600_harness(&core_manifest);
    let core_chip_path = core_system_path.parent().unwrap().join(&core_manifest.chip);
    let core_chip = ChipDescriptor::from_file(&core_chip_path).expect("load canonical chip");
    let _core_bus =
        SystemBus::from_config(&core_chip, &core_manifest).expect("build canonical STK600 bus");

    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load ATmega1281 chip");
    assert_eq!(core_chip.name, chip.name);
    let bus = SystemBus::from_config(&chip, &manifest).expect("build STK600 bus");
    let elf = std::fs::read(root().join("tests/fixtures/avr/stk600-atmega1281.elf"))
        .expect("missing compiled STK600 firmware fixture");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse AVR-GCC ELF");
    let mut cpu = Avr::for_chip_name(&chip.name);
    assert_eq!(cpu.model, AvrModel::Atmega1281);
    cpu.load_program_image(&image);
    let serial = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(serial.clone());
    let mut machine = Machine::new(cpu, bus);
    let portb = machine
        .bus
        .find_peripheral_index_by_name("portb")
        .expect("portb");

    for _ in 0..100_000 {
        machine.step().expect("step STK600 AVR-GCC image");
        if serial
            .lock()
            .unwrap()
            .windows(b"STK600 1281 READY\n".len())
            .any(|window| window == b"STK600 1281 READY\n")
        {
            break;
        }
    }
    assert_eq!(
        String::from_utf8_lossy(&serial.lock().unwrap()).trim_end(),
        "STK600 1281 READY"
    );
    assert_eq!(
        machine.bus.peripherals[portb].dev.read_gpio_output(5),
        Some(true)
    );

    machine
        .bus
        .set_input(Some("sw0"), "pressed", 1.0)
        .expect("press active-low SW0");
    for _ in 0..2_000 {
        machine.step().expect("step while SW0 pressed");
    }
    assert_eq!(
        machine.bus.peripherals[portb].dev.read_gpio_output(5),
        Some(false),
        "pressing active-low SW0 lights active-low LED0"
    );

    machine
        .bus
        .set_input(Some("sw0"), "pressed", 0.0)
        .expect("release active-low SW0");
    for _ in 0..2_000 {
        machine.step().expect("step while SW0 released");
    }
    assert_eq!(
        machine.bus.peripherals[portb].dev.read_gpio_output(5),
        Some(true),
        "releasing SW0 turns LED0 off"
    );
}
