// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! PIC24F KA family × real XC16 firmware: geometry + UART1 boot.
//!
//! The six descriptors are transcribed from Microchip DS30009937C
//! (PIC24F04KA201 family) and DS39927C (PIC24F16KA102 family). Each example
//! ELF is built by XC16 v1.70 from its own `xc16-uart-smoke.c` with the stock
//! part GLD; it transmits `HI` on UART1 and then toggles RA0 forever.
//!
//! Scope is deliberately the modelled slice: package masks/GPIO and UART1.
//! Data EEPROM, UART2, timers, ADC, SPI and I2C are not exercised here; see
//! `examples/<part>/VALIDATION.md` for the limits.

use labwired_config::{Arch as ConfigArch, ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::console::ConsoleCapture;
use labwired_core::cpu::pic24::{
    PIC24_IO_MIRROR_BASE, SFR_TRISA, SFR_TRISB, SFR_TRISC, SFR_U1MODE, SFR_U1STA, SFR_U1TXREG,
};
use labwired_core::cpu::Pic24;
use labwired_core::system::builder::pic24_port_masks;
use labwired_core::system::node::parse_elf_image;
use labwired_core::{Cpu, SimulationConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct KaPart {
    name: &'static str,
    flash: u64,
    ram: u64,
    port_a: u16,
    port_b: u16,
    pin_count: usize,
}

const PARTS: &[KaPart] = &[
    KaPart {
        name: "pic24f04ka200",
        flash: 4 * 1024,
        ram: 512,
        port_a: 0x007F,
        port_b: 0xC310,
        pin_count: 12,
    },
    KaPart {
        name: "pic24f04ka201",
        flash: 4 * 1024,
        ram: 512,
        port_a: 0x007F,
        port_b: 0xF397,
        pin_count: 18,
    },
    KaPart {
        name: "pic24f08ka101",
        flash: 8 * 1024,
        ram: 1536,
        port_a: 0x007F,
        port_b: 0xF397,
        pin_count: 18,
    },
    KaPart {
        name: "pic24f08ka102",
        flash: 8 * 1024,
        ram: 1536,
        port_a: 0x00FF,
        port_b: 0xFFFF,
        pin_count: 24,
    },
    KaPart {
        name: "pic24f16ka101",
        flash: 16 * 1024,
        ram: 1536,
        port_a: 0x007F,
        port_b: 0xF397,
        pin_count: 18,
    },
    KaPart {
        name: "pic24f16ka102",
        flash: 16 * 1024,
        ram: 1536,
        port_a: 0x00FF,
        port_b: 0xFFFF,
        pin_count: 24,
    },
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(part: &str) -> (ChipDescriptor, SystemManifest) {
    let root = repo_root();
    let chip = ChipDescriptor::from_file(root.join(format!("configs/chips/{part}.yaml")))
        .unwrap_or_else(|e| panic!("parse {part} chip descriptor: {e}"));
    let system = SystemManifest::from_file(root.join(format!("examples/{part}/system.yaml")))
        .unwrap_or_else(|e| panic!("parse {part} system manifest: {e}"));
    (chip, system)
}

fn boot(part: &str, steps: usize) -> (Pic24, Vec<u8>) {
    let root = repo_root();
    let (chip, system) = load(part);
    let mut bus = SystemBus::from_config(&chip, &system).expect("build bus");

    let elf = std::fs::read(root.join(format!(
        "examples/{part}/firmware/{part}-xc16-uart-smoke.elf"
    )))
    .unwrap_or_else(|e| panic!("read {part} XC16 ELF: {e}"));
    let image = parse_elf_image(&elf).expect("parse ELF");
    assert_eq!(
        image.arch,
        labwired_core::Arch::Pic24,
        "e_machine is EM_DSPIC30F"
    );

    let console = ConsoleCapture::for_manifest(&system);
    let sink: Arc<Mutex<Vec<u8>>> = console.heard_sink();

    let mut cpu = Pic24::new();
    cpu.set_port_masks(pic24_port_masks(&chip));
    cpu.load_program_image(&image);
    cpu.set_serial_sink(sink.clone());
    cpu.reset(&mut bus).expect("reset");
    cpu.set_pc(0);

    for _ in 0..steps {
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .unwrap_or_else(|e| panic!("{part} XC16 firmware must decode and execute: {e}"));
    }

    let bytes = sink.lock().unwrap().clone();
    (cpu, bytes)
}

#[test]
fn ka_descriptors_match_datasheet_geometry_masks_and_pins() {
    for part in PARTS {
        let (chip, _) = load(part.name);
        assert_eq!(chip.name, part.name);
        assert_eq!(chip.arch, ConfigArch::Pic24);
        assert_eq!(chip.core.as_deref(), Some("pic24f"));
        assert_eq!(chip.cpu_hz, 32_000_000);
        assert_eq!((chip.flash.base, chip.flash.size), (0, part.flash));
        assert_eq!((chip.ram.base, chip.ram.size), (0x0800, part.ram));

        assert_eq!(
            pic24_port_masks(&chip),
            [part.port_a, part.port_b, 0x0000],
            "{}: package masks",
            part.name
        );

        let gpio_base = |port: &str| {
            chip.peripherals
                .iter()
                .find(|p| p.id == format!("gpio{port}"))
                .unwrap_or_else(|| panic!("{}: missing gpio{port}", part.name))
        };
        for (port, base, mask) in [
            ("a", 0x0001_02C0, part.port_a),
            ("b", 0x0001_02D0, part.port_b),
            ("c", 0x0001_02E0, 0x0000),
        ] {
            let p = gpio_base(port);
            assert_eq!(p.r#type, "pic24_gpio", "{}: gpio{port} type", part.name);
            assert_eq!(
                p.base_address, base,
                "{}: gpio{port} mirror base",
                part.name
            );
            assert_eq!(
                p.config.get("mask").and_then(|v| v.as_u64()),
                Some(u64::from(mask)),
                "{}: gpio{port} mask",
                part.name
            );
        }

        // The pin map and the mask must describe the same package: every
        // declared label is on the matching GPIO port/bit, and the union of
        // labels is exactly the datasheet mask.
        assert_eq!(chip.pins.len(), part.pin_count, "{}: pin count", part.name);
        let mut labeled_a = 0u16;
        let mut labeled_b = 0u16;
        for (label, loc) in &chip.pins {
            let rest = label
                .strip_prefix('R')
                .unwrap_or_else(|| panic!("{}: bad PIC24 pin label {label}", part.name));
            let (port, bit) = rest.split_at(1);
            let bit: u8 = bit.parse().unwrap_or_else(|_| panic!("{label}: bit"));
            let expected_port = match port {
                "A" => "gpioa",
                "B" => "gpiob",
                other => panic!("{}: unexpected port {other} on {label}", part.name),
            };
            assert_eq!(&loc.gpio, expected_port, "{}: {label} gpio", part.name);
            assert_eq!(loc.bit, bit, "{}: {label} bit", part.name);
            match port {
                "A" => labeled_a |= 1 << bit,
                "B" => labeled_b |= 1 << bit,
                _ => unreachable!(),
            }
        }
        assert_eq!(labeled_a, part.port_a, "{}: PORTA pin labels", part.name);
        assert_eq!(labeled_b, part.port_b, "{}: PORTB pin labels", part.name);
    }

    // The public core's KA-relevant SFR map is the same one the datasheets
    // give: ports at 0x02C0.. and UART1 at 0x0220... (DS30009937C Tables
    // 4-10/4-12/4-13; DS39927C Tables 4-10/4-12/4-13).
    assert_eq!(
        [SFR_TRISA, SFR_TRISA + 2, SFR_TRISA + 4, SFR_TRISA + 6],
        [0x02C0, 0x02C2, 0x02C4, 0x02C6]
    );
    assert_eq!(
        [SFR_TRISB, SFR_TRISB + 2, SFR_TRISB + 4, SFR_TRISB + 6],
        [0x02C8, 0x02CA, 0x02CC, 0x02CE]
    );
    assert_eq!(SFR_TRISC, 0x02D0);
    // U1RXREG 0x0226 and U1BRG 0x0228 are not symbolic constants in the
    // public core (only TX is modelled), but the datasheet address window is
    // still pinned here for the descriptor's source.
    assert_eq!(
        [SFR_U1MODE, SFR_U1STA, SFR_U1TXREG],
        [0x0220, 0x0222, 0x0224]
    );
    assert_eq!(PIC24_IO_MIRROR_BASE, 0x0001_0000);
}

fn assert_boot(part: &str) {
    let (mut cpu, bytes) = boot(part, 400);
    assert_eq!(
        bytes, b"HI",
        "{part}: the XC16 startup must reach main() and transmit both bytes"
    );
    let (chip, _) = load(part);
    let mask = pic24_port_masks(&chip);
    assert_eq!(
        cpu.data_read(SFR_TRISB).unwrap(),
        mask[1],
        "{part}: PORTB reset direction"
    );
    assert_eq!(
        cpu.data_read(SFR_TRISA).unwrap(),
        mask[0] & !1,
        "{part}: firmware makes RA0 an output"
    );
}

#[test]
fn pic24f04ka200_uart_smoke() {
    assert_boot("pic24f04ka200");
}

#[test]
fn pic24f04ka201_uart_smoke() {
    assert_boot("pic24f04ka201");
}

#[test]
fn pic24f08ka101_uart_smoke() {
    assert_boot("pic24f08ka101");
}

#[test]
fn pic24f08ka102_uart_smoke() {
    assert_boot("pic24f08ka102");
}

#[test]
fn pic24f16ka101_uart_smoke() {
    assert_boot("pic24f16ka101");
}

#[test]
fn pic24f16ka102_uart_smoke() {
    assert_boot("pic24f16ka102");
}
