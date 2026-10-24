// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! dsPIC33FJ32MC204 × real XC16 firmware: boots a compiler-built ELF.
//!
//! `examples/dspic33fj32mc204/firmware/dspic33fj32mc204-xc16-uart-smoke.elf`
//! is produced by Microchip XC16 v1.70 from `xc16-uart-smoke.c` with the
//! stock `p33FJ32MC204.gld` linker script (build recipe in the example
//! README). Unlike the hand-written M0 fixture, this exercises the real
//! toolchain's reset vector, IVT entries, section packing, startup code
//! (`__reset`/`__psv_init`/`__crt_start_mode`) and call/return sequences.
//!
//! The program transmits `HI` on UART1 and then toggles LATB0 forever.
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

fn run_firmware(steps: usize) -> (Vec<u8>, u32) {
    let root = repo_root();
    let chip = ChipDescriptor::from_file(root.join("configs/chips/dspic33fj32mc204.yaml"))
        .expect("parse chip descriptor");
    let system = SystemManifest::from_file(root.join("configs/systems/dspic33fj32mc204-demo.yaml"))
        .expect("parse system manifest");
    let mut bus: labwired_core::bus::SystemBus =
        labwired_core::bus::SystemBus::from_config(&chip, &system).expect("build bus");

    let elf = std::fs::read(
        root.join("examples/dspic33fj32mc204/firmware/dspic33fj32mc204-xc16-uart-smoke.elf"),
    )
    .expect("read XC16 ELF");
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
    // (DS70283H Figure 7-1).
    cpu.set_pc(0);

    for _ in 0..steps {
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .expect("XC16 firmware instruction must decode and execute");
    }

    let bytes = sink.lock().unwrap().clone();
    (bytes, cpu.get_pc())
}

/// The ELF's section layout is the thing #1447 tried to describe: verify it
/// against the compiler's own output before running a single instruction.
#[test]
fn xc16_elf_sections_land_at_their_pc_unit_addresses() {
    let root = repo_root();
    let elf = std::fs::read(
        root.join("examples/dspic33fj32mc204/firmware/dspic33fj32mc204-xc16-uart-smoke.elf"),
    )
    .expect("read XC16 ELF");
    let image = parse_elf_image(&elf).expect("parse ELF");

    let seg = |addr: u64| {
        image
            .segments
            .iter()
            .find(|s| s.start_addr == addr)
            .unwrap_or_else(|| panic!("no segment at {addr:#x}"))
    };
    // .reset — the 2-word GOTO at PC 0 (8 bytes = 2 four-byte slots).
    let reset = seg(0x0);
    assert_eq!(reset.data.len(), 8, "GOTO 0x200 is two instruction words");
    assert_eq!(
        u32::from_le_bytes([reset.data[0], reset.data[1], reset.data[2], 0]),
        0x040200,
        "GOTO 0x200"
    );
    // .text — the compiler's code, already in 4-byte slots.
    assert_eq!(seg(0x200).data.len(), 0x19c);
    assert_eq!(seg(0x2ce).data.len(), 0x74);
    // The T1 vector (number 11 -> PC 0x1A) holds the default handler's
    // 24-bit address as data (DS70283H §7.1).
    let t1_vector = seg(0x1A);
    assert_eq!(t1_vector.data.len(), 4, "one 4-byte vector slot");
    assert_eq!(
        u32::from_le_bytes([t1_vector.data[0], t1_vector.data[1], t1_vector.data[2], 0]),
        0x2c8,
        "__DefaultInterrupt"
    );
}

#[test]
fn xc16_firmware_boots_and_writes_hi_to_uart() {
    let (bytes, pc) = run_firmware(300);
    assert_eq!(
        bytes, b"HI",
        "the XC16 startup must reach main() and transmit both bytes"
    );
    // main() ends in the LATB0 toggle loop at 0x300..0x306.
    assert!(
        (0x300..=0x306).contains(&pc),
        "PC {pc:#x} should sit in the LATB0 toggle loop"
    );

    // Running further only toggles the pin: no extra UART bytes, PC stays in
    // the loop.
    let (bytes2, pc2) = run_firmware(400);
    assert_eq!(bytes2, b"HI", "no extra bytes after the two transmits");
    assert!(
        (0x300..=0x306).contains(&pc2),
        "PC {pc2:#x} should stay in the loop"
    );
}

#[test]
fn xc16_firmware_is_deterministic() {
    let a = run_firmware(300);
    let b = run_firmware(300);
    assert_eq!(
        a, b,
        "same ELF + same manifest must produce the same result"
    );
}
