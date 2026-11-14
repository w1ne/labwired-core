//! Machine / system YAML path for the Arduino Uno R3 golden ELF.
//! Same ATmega328P as the Nano; proves the Uno board manifest boots real
//! Arduino-core firmware and drives USART0 + PB5 (D13).
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::system::node::{build_node, NodeFirmware};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn core_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn machine_path_runs_uno_ok() {
    let root = core_root();
    let chip: ChipDescriptor = {
        let y = std::fs::read_to_string(root.join("configs/chips/atmega328p.yaml")).unwrap();
        serde_yaml::from_str(&y).expect("chip yaml")
    };
    let system: SystemManifest = {
        let y = std::fs::read_to_string(root.join("configs/systems/arduino-uno.yaml")).unwrap();
        serde_yaml::from_str(&y).expect("system yaml")
    };
    assert_eq!(
        system.cpu_hz,
        Some(16_000_000),
        "Uno R3 runs a 16 MHz crystal"
    );
    let elf = std::fs::read(root.join("tests/fixtures/avr/arduino-uno-blinky.elf"))
        .expect("missing golden ELF; build examples/arduino-uno-blinky");
    let mut machine =
        build_node("uno", &chip, &system, NodeFirmware::Elf(elf)).expect("build_node");
    let sink = Arc::new(Mutex::new(Vec::new()));
    machine
        .attach_uart_tx_sink(sink.clone(), false)
        .expect("uart sink");

    for step in 0..2_000_000u32 {
        if let Err(e) = machine.step() {
            panic!("step {step} pc={:#x}: {e:?}", machine.get_pc());
        }
        let serial = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
        if serial.contains("uno-ok") {
            return;
        }
    }
    let serial = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    panic!(
        "uno-ok never printed; serial={serial:?} pc={:#x}",
        machine.get_pc()
    );
}
