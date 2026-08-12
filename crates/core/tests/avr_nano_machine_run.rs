//! Machine / system YAML path for Arduino Nano golden ELF.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::system::node::{build_node, NodeFirmware};
use labwired_core::world::MachineTrait as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn core_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn machine_path_runs_nano_ok() {
    let root = core_root();
    let chip: ChipDescriptor = {
        let y = std::fs::read_to_string(root.join("configs/chips/atmega328p.yaml")).unwrap();
        serde_yaml::from_str(&y).expect("chip yaml")
    };
    let system: SystemManifest = {
        let y = std::fs::read_to_string(root.join("configs/systems/arduino-nano.yaml")).unwrap();
        serde_yaml::from_str(&y).expect("system yaml")
    };
    let elf = std::fs::read(root.join("tests/fixtures/avr/arduino-nano-blinky.elf")).unwrap();
    let mut machine =
        build_node("nano", &chip, &system, NodeFirmware::Elf(elf)).expect("build_node");
    let sink = Arc::new(Mutex::new(Vec::new()));
    machine
        .attach_uart_tx_sink(sink.clone(), false)
        .expect("uart sink");

    for step in 0..2_000_000u32 {
        if let Err(e) = machine.step() {
            panic!("step {step} pc={:#x}: {e:?}", machine.get_pc());
        }
        let serial = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
        if serial.contains("nano-ok") {
            eprintln!("=== Machine path SUCCESS ===");
            eprintln!("steps={step} cycles={} pc={:#x}", machine.total_cycles(), machine.get_pc());
            eprintln!("serial={serial:?}");
            return;
        }
    }
    let serial = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    panic!("machine path failed serial={serial:?} pc={:#x}", machine.get_pc());
}
