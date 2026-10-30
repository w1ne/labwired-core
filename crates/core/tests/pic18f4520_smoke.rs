// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! PIC18F4520 × real SDCC firmware: L1 boot + EUSART TX + PORTB latch.
//!
//! `examples/pic18f4520/firmware/pic18f4520-sdcc-uart-smoke.elf` is SDCC 4.4.0
//! (pic16 port) + gputils 1.4.0 output for `sdcc-uart-smoke.c`, linked with
//! the stock `18f4520_g.lkr` and wrapped byte for byte from the Intel HEX by
//! `hex2elf.py` (see `build.sh`). The C runtime's software stack, the
//! generic-pointer string reads (`TBLRD*+`) and the `MULLW`-based pointer
//! arithmetic all run on the PIC18 core.
//!
//! Scope and limitations: `examples/pic18f4520/VALIDATION.md`.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::pic18::{SFR_LATA, SFR_TRISA, SFR_TXSTA};
use labwired_core::cpu::Pic18;
use labwired_core::system::node::parse_elf_image;
use labwired_core::{Cpu, SimulationConfig};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const BANNER: &[u8] = b"PIC18F4520 OK\r\nLED=1\r\nTOGGLE OK\r\n";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn firmware(ext: &str) -> Vec<u8> {
    std::fs::read(repo_root().join(format!(
        "examples/pic18f4520/firmware/pic18f4520-sdcc-uart-smoke.{ext}"
    )))
    .expect("read PIC18F4520 firmware")
}

fn boot(steps: usize) -> (Pic18, Vec<u8>) {
    let root = repo_root();
    let chip = ChipDescriptor::from_file(root.join("configs/chips/pic18f4520.yaml"))
        .expect("parse PIC18F4520 chip descriptor");
    let system = SystemManifest::from_file(root.join("configs/systems/pic18f4520-demo.yaml"))
        .expect("parse PIC18F4520 system manifest");
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &system).expect("build bus");

    let image = parse_elf_image(&firmware("elf")).expect("parse ELF");
    assert_eq!(
        image.arch,
        labwired_core::Arch::Pic18,
        "e_machine is EM_MCHP_PIC"
    );

    let sink = Arc::new(Mutex::new(Vec::new()));
    let mut cpu = Pic18::for_chip_name(&chip.name);
    cpu.load_program_image(&image);
    cpu.set_serial_sink(sink.clone());
    cpu.reset(&mut bus).expect("reset");

    for _ in 0..steps {
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .expect("SDCC firmware instruction must decode and execute");
    }
    let bytes = sink.lock().unwrap().clone();
    (cpu, bytes)
}

#[test]
fn sdcc_firmware_boots_and_prints_the_banner() {
    let (cpu, bytes) = boot(20_000);
    assert_eq!(
        String::from_utf8_lossy(&bytes),
        String::from_utf8_lossy(BANNER),
        "the SDCC startup must reach main() and print all three lines"
    );
    // The transmitter the firmware configured: TXEN | BRGH (+ TRMT).
    assert_eq!(cpu.read_data(SFR_TXSTA), 0x26);
}

#[test]
fn sdcc_firmware_drives_the_rb0_led_latch() {
    let (cpu, _) = boot(20_000);
    let (latb, trisb) = (SFR_LATA + 1, SFR_TRISA + 1);
    // RB0 is an output; set once then toggled three times -> low.
    assert_eq!(cpu.read_data(trisb) & 0x01, 0);
    assert_eq!(cpu.read_data(latb) & 0x01, 0);
    // The other PORTB pins stay inputs.
    assert_eq!(cpu.read_data(trisb) & 0xFE, 0xFE);
}

#[test]
fn sdcc_firmware_run_is_deterministic() {
    let (a, out_a) = boot(20_000);
    let (b, out_b) = boot(20_000);
    assert_eq!(out_a, out_b);
    assert_eq!(a.clock_cycles(), b.clock_cycles());
    assert_eq!(a.get_pc(), b.get_pc());
}

/// The committed ELF is only a container: its PT_LOAD payload must equal the
/// SDCC/gputils Intel HEX it was wrapped from, byte for byte.
#[test]
fn elf_payload_equals_the_sdcc_hex() {
    let mut hex = BTreeMap::new();
    let mut upper = 0u32;
    for line in String::from_utf8(firmware("hex")).unwrap().lines() {
        let raw: Vec<u8> = (1..line.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&line[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(
            raw.iter().fold(0u8, |s, b| s.wrapping_add(*b)),
            0,
            "checksum"
        );
        let (count, addr, kind) = (
            raw[0] as usize,
            u32::from(raw[1]) << 8 | u32::from(raw[2]),
            raw[3],
        );
        let data = &raw[4..4 + count];
        match kind {
            0 => {
                for (i, b) in data.iter().enumerate() {
                    hex.insert(upper + addr + i as u32, *b);
                }
            }
            1 => break,
            4 => upper = (u32::from(data[0]) << 8 | u32::from(data[1])) << 16,
            other => panic!("unexpected record type {other}"),
        }
    }

    let image = parse_elf_image(&firmware("elf")).expect("parse ELF");
    let mut elf = BTreeMap::new();
    for seg in &image.segments {
        for (i, b) in seg.data.iter().enumerate() {
            elf.insert(seg.start_addr as u32 + i as u32, *b);
        }
    }
    assert_eq!(image.entry_point, 0);
    assert_eq!(elf, hex);
    // The image programs the configuration words (CONFIG1H HS oscillator).
    assert!(hex.contains_key(&0x30_0001));
}
