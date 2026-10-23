// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! dsPIC33FJ32MC202 × real XC16 firmware: M0 boot + UART1.
//!
//! `examples/dspic33fj32mc202/firmware/dspic33fj32mc202-xc16-uart-smoke.elf`
//! is built by Microchip XC16 v1.70 from `xc16-uart-smoke.c` with the stock
//! `p33FJ32MC202.gld` linker script. The 202 is the 28-pin sibling of the
//! onboarded dsPIC33FJ32MC204 and shares its core, memory, UART1 register map
//! and instruction set, so the same PIC24 CPU model boots it.
//!
//! The program transmits `HI` on UART1 and then toggles LATB0 forever.
//!
//! Scope and limitations are documented in
//! `examples/dspic33fj32mc202/VALIDATION.md`.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::console::ConsoleCapture;
use labwired_core::cpu::pic24::SFR_LATB;
use labwired_core::cpu::Pic24;
use labwired_core::system::node::parse_elf_image;
use labwired_core::{Cpu, SimulationConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn boot(steps: usize) -> (Pic24, Vec<u8>) {
    let root = repo_root();
    let chip = ChipDescriptor::from_file(root.join("configs/chips/dspic33fj32mc202.yaml"))
        .expect("parse 202 chip descriptor");
    let system = SystemManifest::from_file(root.join("configs/systems/dspic33fj32mc202-demo.yaml"))
        .expect("parse 202 system manifest");
    let mut bus: labwired_core::bus::SystemBus =
        labwired_core::bus::SystemBus::from_config(&chip, &system).expect("build bus");

    let elf = std::fs::read(
        root.join("examples/dspic33fj32mc202/firmware/dspic33fj32mc202-xc16-uart-smoke.elf"),
    )
    .expect("read 202 XC16 ELF");
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
    cpu.set_pc(0);

    for _ in 0..steps {
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .expect("202 XC16 firmware instruction must decode and execute");
    }

    let bytes = sink.lock().unwrap().clone();
    (cpu, bytes)
}

#[test]
fn xc16_202_firmware_boots_and_writes_hi_to_uart() {
    let (mut cpu, bytes) = boot(300);
    assert_eq!(
        bytes, b"HI",
        "the XC16 startup must reach main() and transmit both bytes"
    );
}

#[test]
fn xc16_202_firmware_toggles_latb0() {
    let (mut cpu, bytes) = boot(300);
    assert_eq!(bytes, b"HI");
    let mut bus = labwired_core::bus::SystemBus::new();
    let mut last = cpu.data_read(SFR_LATB).expect("LATB");
    let mut toggled = false;
    for _ in 0..40 {
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .expect("step the toggle loop");
        let now = cpu.data_read(SFR_LATB).expect("LATB");
        if now != last {
            toggled = true;
            break;
        }
        last = now;
    }
    assert!(toggled, "LATB0 never toggled after the UART output");
}
