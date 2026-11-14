// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! PIC16F1829 × real SDCC firmware: L1 boot on the enhanced mid-range core.
//!
//! `examples/pic16f1829/firmware/pic16f1829-sdcc-uart-smoke.elf` is SDCC 4.4.0
//! (pic14 port, enhanced mid-range target) + gputils 1.4.0 output for
//! `sdcc-uart-smoke.c`, linked with the stock `16f1829_g.lkr` and wrapped
//! byte for byte from the Intel HEX by `hex2elf.py` (see `build.sh`). It runs
//! BSR bank switching (MOVLB), MOVLP paging, FSR/INDF and MOVIW pointer
//! reads, LSLF, RETLW string tables, the EUSART at 0x19A-0x19E, LATA/PORTA,
//! the OSCSTAT ready poll, and an ISR with no software context save that is
//! preempted into by TMR0 overflows, so main's results depend on the
//! hardware shadow registers.
//!
//! Scope and limitations: `examples/pic16f1829/VALIDATION.md`.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::Pic16;
use labwired_core::system::arch_policy::image_arch_for_chip;
use labwired_core::system::node::parse_elf_image;
use labwired_core::{Cpu, SimulationConfig};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const BANNER: &[u8] = b"PIC16F1829 OK\r\nLED=1\r\nTOGGLE OK\r\nSUM OK\r\nIRQ OK\r\n\
three many\r\nTMR0 IRQ OK\r\nMUL OK\r\n";
const STEPS: usize = 200_000;

const TRISA: u16 = 0x08C;
const LATA: u16 = 0x10C;
const TXSTA: u16 = 0x19E;
const OPTION_REG: u16 = 0x095;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn firmware(ext: &str) -> Vec<u8> {
    std::fs::read(repo_root().join(format!(
        "examples/pic16f1829/firmware/pic16f1829-sdcc-uart-smoke.{ext}"
    )))
    .expect("read PIC16F1829 firmware")
}

fn boot(steps: usize) -> (Pic16, Vec<u8>) {
    let root = repo_root();
    let chip = ChipDescriptor::from_file(root.join("configs/chips/pic16f1829.yaml"))
        .expect("parse PIC16F1829 chip descriptor");
    let system = SystemManifest::from_file(root.join("configs/systems/pic16f1829-curiosity.yaml"))
        .expect("parse PIC16F1829 system manifest");
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &system).expect("build bus");

    let image = parse_elf_image(&firmware("elf")).expect("parse ELF");
    assert_eq!(
        image_arch_for_chip(image.arch, chip.arch),
        labwired_core::Arch::Pic16,
        "EM_MCHP_PIC on an arch: pic16 chip is a PIC16 image"
    );

    let sink = Arc::new(Mutex::new(Vec::new()));
    let mut cpu = Pic16::for_chip_name(&chip.name);
    assert!(cpu.is_enhanced(), "pic16f1829 selects the enhanced core");
    cpu.load_program_image(&image).expect("image fits the part");
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
    let (cpu, bytes) = boot(STEPS);
    assert_eq!(
        String::from_utf8_lossy(&bytes),
        String::from_utf8_lossy(BANNER),
        "the SDCC startup must reach main() and every check must pass"
    );
    // The transmitter the firmware configured: TXEN | BRGH (+ TRMT).
    assert_eq!(cpu.read_data(TXSTA), 0x26);
    // TMR0 in timer mode, no prescaler.
    assert_eq!(cpu.read_data(OPTION_REG), 0x08);
}

#[test]
fn sdcc_firmware_drives_the_ra5_led_latch() {
    let (cpu, _) = boot(STEPS);
    // RA5 is an output; set once then toggled three times -> low.
    assert_eq!(cpu.read_data(TRISA) & 0x20, 0);
    assert_eq!(cpu.read_data(LATA) & 0x20, 0);
    assert_eq!(cpu.port_latch(0) & 0x20, 0);
    // The other PORTA pins stay inputs.
    assert_eq!(cpu.read_data(TRISA) & 0x1F, 0x1F);
}

#[test]
fn sdcc_firmware_programs_both_configuration_words() {
    let (cpu, _) = boot(1);
    // _FOSC_INTOSC & _WDTE_OFF & _PWRTE_ON & _MCLRE_ON (SDCC pic16f1829.h).
    assert_eq!(cpu.config_word(), 0x3FFC & 0x3FE7 & 0x3FDF & 0x3FFF);
    // _STVREN_ON & _LVP_OFF: stack faults reset the core.
    assert_eq!(cpu.config_word2(), 0x3FFF & 0x1FFF);
}

#[test]
fn sdcc_firmware_run_is_deterministic() {
    let (a, out_a) = boot(STEPS);
    let (b, out_b) = boot(STEPS);
    assert_eq!(out_a, out_b);
    assert_eq!(a.clock_cycles(), b.clock_cycles());
    assert_eq!(a.get_pc(), b.get_pc());
}

/// A classic mid-range image (CONFIG at word 0x2007) has no home on the
/// enhanced part: loading it is an error, not a panic or a silent drop.
#[test]
fn a_classic_image_is_rejected_by_the_enhanced_part() {
    let image = parse_elf_image(
        &std::fs::read(
            repo_root().join("examples/pic16f877a/firmware/pic16f877a-sdcc-uart-smoke.elf"),
        )
        .expect("read PIC16F877A firmware"),
    )
    .expect("parse ELF");
    let err = Pic16::for_chip_name("pic16f1829")
        .load_program_image(&image)
        .expect_err("word 0x2007 is not programmable on the PIC16F1829");
    assert!(err.contains("0x2007"), "{err}");
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
    // The image programs CONFIG1/CONFIG2 (words 0x8007/0x8008, bytes
    // 0x1000E-0x10011), past the 64 KiB HEX boundary.
    assert!(hex.contains_key(&0x1000E) && hex.contains_key(&0x10010));
}
