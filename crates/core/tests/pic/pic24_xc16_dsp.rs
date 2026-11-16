// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! dsPIC33FJ32MC204 × real XC16 DSP firmware: boots a compiler-built ELF
//! whose body is a fixed DSP MAC-class sequence.
//!
//! `examples/dspic33fj32mc204/firmware/dspic33fj32mc204-xc16-dsp-mac.elf` is
//! built by Microchip XC16 v1.70 from `xc16-dsp-mac.c` (recipe in the
//! example README). The inline assembly in that C file is the DS70000157G
//! worked examples for LAC/MAC/MPY/MSC/SAC/SAC.R plus a normal-saturation
//! clamp; the program writes a completion marker to 0x0E00 and loops.
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

/// SFR addresses used to reconstruct the 40-bit accumulators (DS70283H).
const SFR_ACCAL: u16 = 0x0022;
const SFR_ACCAH: u16 = 0x0024;
const SFR_ACCAU: u16 = 0x0026;
const SFR_ACCBL: u16 = 0x0028;
const SFR_ACCBH: u16 = 0x002A;
const SFR_ACCBU: u16 = 0x002C;
const SFR_SR: u16 = 0x0042;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The 40-bit accumulator as a signed value.
fn read_acc(cpu: &mut Pic24, l: u16, h: u16, u: u16) -> i64 {
    let lo = i64::from(cpu.data_read(l).unwrap());
    let hi = i64::from(cpu.data_read(h).unwrap());
    let up = i64::from(cpu.data_read(u).unwrap() & 0x00FF);
    let raw = (up << 32) | (hi << 16) | lo;
    if raw >= (1 << 39) {
        raw - (1 << 40)
    } else {
        raw
    }
}

fn run_until_marker() -> Pic24 {
    let root = repo_root();
    let chip = ChipDescriptor::from_file(root.join("configs/chips/dspic33fj32mc204.yaml"))
        .expect("parse chip descriptor");
    let system = SystemManifest::from_file(root.join("configs/systems/dspic33fj32mc204-demo.yaml"))
        .expect("parse system manifest");
    let mut bus: labwired_core::bus::SystemBus =
        labwired_core::bus::SystemBus::from_config(&chip, &system).expect("build bus");

    let elf = std::fs::read(
        root.join("examples/dspic33fj32mc204/firmware/dspic33fj32mc204-xc16-dsp-mac.elf"),
    )
    .expect("read XC16 DSP ELF");
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
    cpu.set_serial_sink(sink);
    cpu.reset(&mut bus).expect("reset");
    cpu.set_pc(0);

    for _ in 0..2000 {
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .expect("DSP firmware instruction must decode and execute");
        if cpu.data_read(0x0E00).unwrap() == 0xBEEF {
            return cpu;
        }
    }
    panic!("the DSP firmware never reached its completion marker");
}

#[test]
fn xc16_dsp_firmware_runs_the_mac_class_and_saturates() {
    let mut cpu = run_until_marker();

    // SAC A, #4, W0 stored ACCxH of 0x472D_2400 >> 4.
    assert_eq!(
        cpu.data_read(0x0E02).unwrap(),
        0x0472,
        "SAC stores ACCxH after the shift"
    );
    // SAC.R A, #4, W1 rounded 0x120F_FF00 >> 4 to 0x0121 (PRM example 1).
    assert_eq!(
        cpu.data_read(0x0E04).unwrap(),
        0x0121,
        "SAC.R rounds the shifted accumulator"
    );

    // MSC's write-back stored the rounded *other* accumulator (ACCA).
    assert_eq!(
        cpu.get_register(13),
        0x472D,
        "W13 = rounded ACCA from the MSC write-back"
    );

    // MAC prefetches: X at 0x0A00 -> W4, Y at 0x0C00 -> W5, both +2.
    assert_eq!(
        cpu.get_register(4),
        0x8000,
        "W4 reloaded for the clamp test"
    );
    assert_eq!(cpu.get_register(5), 0x909C, "MAC Y prefetch destination");
    assert_eq!(cpu.get_register(8), 0x0A02, "MAC X post-increment");
    assert_eq!(cpu.get_register(10), 0x0C02, "MAC Y post-increment");
    assert_eq!(cpu.get_register(6), 0x1234, "W6 untouched by the MAC");
    assert_eq!(cpu.get_register(7), 0x5678, "W7 untouched by the MAC");

    // MPY W6*W7, B then MSC W6*W7, B cancel exactly.
    assert_eq!(
        read_acc(&mut cpu, SFR_ACCBL, SFR_ACCBH, SFR_ACCBU),
        0,
        "MPY then MSC cancel"
    );

    // Normal saturation: MPY W4*W4, A with W4 = 0x8000 is +2^31, clamped.
    assert_eq!(
        read_acc(&mut cpu, SFR_ACCAL, SFR_ACCAH, SFR_ACCAU),
        0x7FFF_FFFF,
        "normal saturation clamps to 1.31"
    );
    let sr = cpu.data_read(SFR_SR).unwrap();
    assert_ne!(sr & 0x2000, 0, "SA is set by the clamp");
    assert_eq!(sr & 0x0400, 0x0400, "SAB follows SA");
}
