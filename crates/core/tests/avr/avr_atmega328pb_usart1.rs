use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::{Cpu, Machine};
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

/// Run a stock Arduino AVR core ELF on the ATmega328PB model. Returns the
/// captured console text and the CPU cycle count at each PB5 rising edge.
fn run_arduino_sketch(elf: &str) -> (String, Vec<u64>) {
    let chip = ChipDescriptor::from_file(root().join("configs/chips/atmega328pb.yaml"))
        .expect("load ATmega328PB chip descriptor");
    let bus =
        SystemBus::from_config(&chip, &SystemManifest::default()).expect("build ATmega328PB bus");
    let elf_bytes = std::fs::read(root().join("tests/fixtures/avr").join(elf))
        .unwrap_or_else(|err| panic!("missing compiled Arduino ELF {elf}: {err}"));
    let image = labwired_loader::load_elf_bytes(&elf_bytes).expect("parse AVR-GCC ELF");
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

    let mut rising_edges = Vec::new();
    let mut previous_high = false;
    let done = b"ATMEGA328PB_ARDUINO_DONE";
    // setup() plus three 100 ms loop turns is under 6 M cycles; the step budget
    // is generous, and the run stops as soon as the sketch prints DONE.
    for _ in 0..12_000_000 {
        machine.step().expect("step Arduino ATmega328PB firmware");
        let high = machine.bus.peripherals[portb].dev.read_gpio_output(5) == Some(true);
        if high && !previous_high {
            rising_edges.push(machine.cpu.clock_cycles());
        }
        previous_high = high;
        let printed = {
            let console = serial.lock().unwrap();
            console.windows(done.len()).any(|window| window == done)
        };
        if printed {
            break;
        }
    }

    let console = String::from_utf8_lossy(&serial.lock().unwrap()).to_string();
    (console, rising_edges)
}

#[test]
fn atmega328pb_runs_stock_arduino_core_blink_and_serial() {
    let (console, rising_edges) = run_arduino_sketch("atmega328pb-arduino-blink.elf");
    assert!(
        console.contains("ATMEGA328PB_ARDUINO_BOOT"),
        "console: {console}"
    );
    assert!(
        console.contains("ATMEGA328PB_ARDUINO_DONE"),
        "console: {console}"
    );
    assert!(
        rising_edges.len() >= 3,
        "stock Arduino core produced {} PB5 rising edges",
        rising_edges.len()
    );
    // loop() drives PB5 high, delay(50), low, delay(50): one rising edge per
    // 100 ms = 1_600_000 cycles at 16 MHz. The stock core's millis() baseline
    // is the Timer0 overflow interrupt, so this also measures that cadence.
    for pair in rising_edges.windows(2) {
        let interval = pair[1] - pair[0];
        assert!(
            (1_560_000..=1_640_000).contains(&interval),
            "PB5 rising-edge interval {interval} cycles is outside the 100 ms band"
        );
    }
}

#[test]
fn atmega328pb_arduino_control_without_digitalwrite_keeps_pb5_flat() {
    let (console, rising_edges) = run_arduino_sketch("atmega328pb-arduino-control.elf");
    assert!(
        console.contains("ATMEGA328PB_ARDUINO_BOOT"),
        "console: {console}"
    );
    assert!(
        console.contains("ATMEGA328PB_ARDUINO_DONE"),
        "console: {console}"
    );
    assert!(
        rising_edges.is_empty(),
        "negative control raised PB5 {} times",
        rising_edges.len()
    );
}
