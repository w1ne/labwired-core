//! Push-vs-poll differential oracle for the AVR GPIO port on the real Uno
//! blinky firmware: the edge stream captured through the event-driven tap must
//! be byte-identical to the forced per-cycle poll reference.
//! (Synthetic DDR/PORT/PIN coverage lives in
//! `src/tests/logic_capture_differential.rs`.)
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::logic_capture::LogicSource;
use labwired_core::{DebugControl, Machine};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn uno_blinky() -> Machine<Avr> {
    let root = root();
    let chip: ChipDescriptor = serde_yaml::from_str(
        &std::fs::read_to_string(root.join("configs/chips/atmega328p.yaml")).unwrap(),
    )
    .unwrap();
    let system: SystemManifest = serde_yaml::from_str(
        &std::fs::read_to_string(root.join("configs/systems/arduino-uno.yaml")).unwrap(),
    )
    .unwrap();
    let elf = std::fs::read(root.join("tests/fixtures/avr/arduino-uno-blinky.elf")).unwrap();
    let bus = SystemBus::from_config(&chip, &system).expect("build bus");
    let image = labwired_loader::load_elf_bytes(&elf).expect("parse ELF");
    let mut cpu = Avr::new();
    cpu.load_program_image(&image);
    Machine::new(cpu, bus)
}

#[test]
fn avr_uno_blinky_push_stream_is_byte_identical_to_poll() {
    let run = |force_poll: bool| {
        let mut machine = uno_blinky();
        machine.logic_force_poll_capture(force_poll);
        let idx = machine.bus.find_peripheral_index_by_name("portb").unwrap();
        machine.logic_watch(&[Some(LogicSource::pad(idx, 5))]);
        machine.run(Some(3_000_000)).unwrap();
        let b = machine.logic_read_edges(0);
        (b.edges, b.dropped, machine.total_cycles)
    };
    let (poll, pd, pc) = run(true);
    let (push, qd, qc) = run(false);
    assert!(
        poll.len() >= 2,
        "blinky must toggle PB5, got {}",
        poll.len()
    );
    assert_eq!(poll, push, "AVR push edges must be byte-identical to poll");
    assert_eq!(pd, qd);
    assert_eq!(pc, qc, "identical simulated time");
}
