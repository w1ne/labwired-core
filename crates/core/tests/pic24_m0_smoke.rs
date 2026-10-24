// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! dsPIC33FJ32MC204 M0 smoke: the committed fixture ELF boots on the PIC24
//! core and transmits `H` through UART1 (`U1TXREG 0x0224`), deterministically.
//!
//! Scope and limitations are documented in
//! `examples/dspic33fj32mc204/VALIDATION.md`.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::console::ConsoleCapture;
use labwired_core::cpu::Pic24;
use labwired_core::system::node::parse_elf_image;
use labwired_core::{Cpu, SimulationConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_fixture() -> (Vec<u8>, u32) {
    let root = repo_root();
    let chip = ChipDescriptor::from_file(root.join("configs/chips/dspic33fj32mc204.yaml"))
        .expect("parse chip descriptor");
    let system = SystemManifest::from_file(root.join("configs/systems/dspic33fj32mc204-demo.yaml"))
        .expect("parse system manifest");
    let mut bus: labwired_core::bus::SystemBus =
        labwired_core::bus::SystemBus::from_config(&chip, &system).expect("build bus");

    let elf = std::fs::read(
        root.join("examples/dspic33fj32mc204/firmware/dspic33fj32mc204-uart-smoke.elf"),
    )
    .expect("read fixture ELF");
    let image = parse_elf_image(&elf).expect("parse ELF");
    assert_eq!(
        image.arch,
        labwired_core::Arch::Pic24,
        "e_machine is EM_DSPIC30F"
    );

    let console = ConsoleCapture::for_manifest(&system);
    let sink: Arc<Mutex<Vec<u8>>> = console.heard_sink();

    let mut cpu = Pic24::new();
    cpu.load_program_image(&image);
    cpu.set_serial_sink(sink.clone());
    cpu.reset(&mut bus).expect("reset");
    // dsPIC33F resets to PC = 0 and vectors through the GOTO at address 0
    // (DS70283H Figure 7-1); the fixture carries that reset vector.
    cpu.set_pc(0);

    for _ in 0..33 {
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .expect("fixture instruction must decode and execute");
    }

    let bytes = sink.lock().unwrap().clone();
    (bytes, cpu.get_pc())
}

#[test]
fn pic24_fixture_boots_and_writes_h_to_uart() {
    let (bytes, pc) = run_fixture();
    assert_eq!(bytes, b"H", "U1TXREG must deliver the fixture byte");
    // MOV, MOV, 6×NOP, BRA self-loop → PC sits at the BRA (0x210).
    assert_eq!(pc, 0x210, "self-loop must hold the PC");
}

#[test]
fn pic24_fixture_is_deterministic() {
    let a = run_fixture();
    let b = run_fixture();
    assert_eq!(
        a, b,
        "same ELF + same manifest must produce the same result"
    );
}
