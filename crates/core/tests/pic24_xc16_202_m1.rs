// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! dsPIC33FJ32MC202 × real XC16 firmware: M1 28-pin package/GPIO.
//!
//! `examples/dspic33fj32mc202/firmware/dspic33fj32mc202-xc16-m1-gpio.elf`
//! is built by Microchip XC16 v1.70 from `xc16-m1-gpio.c` with the stock
//! `p33FJ32MC202.gld` linker script. The 28-pin package exposes RA0-RA4 and
//! RB0-RB15 (no PORTC); the firmware prints `G1`, makes both ports outputs
//! and toggles RA0/RB0 forever.
//!
//! The test checks the package masks, the direction registers, the toggling
//! latches and the bus-side pad levels. Scope and limitations are documented
//! in `examples/dspic33fj32mc202/VALIDATION.md`.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::console::ConsoleCapture;
use labwired_core::cpu::pic24::{SFR_LATA, SFR_LATB, SFR_TRISA, SFR_TRISB, SFR_TRISC};
use labwired_core::cpu::Pic24;
use labwired_core::peripherals::pic24_gpio::Pic24GpioPort;
use labwired_core::pins::PinPort;
use labwired_core::system::builder::pic24_port_masks;
use labwired_core::system::node::parse_elf_image;
use labwired_core::{Cpu, SimulationConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn boot(steps: usize) -> (Pic24, SystemBus, Vec<u8>) {
    let root = repo_root();
    let chip = ChipDescriptor::from_file(root.join("configs/chips/dspic33fj32mc202.yaml"))
        .expect("parse 202 chip descriptor");
    let system = SystemManifest::from_file(root.join("configs/systems/dspic33fj32mc202-demo.yaml"))
        .expect("parse 202 system manifest");
    let mut bus = SystemBus::from_config(&chip, &system).expect("build bus");

    let elf = std::fs::read(
        root.join("examples/dspic33fj32mc202/firmware/dspic33fj32mc202-xc16-m1-gpio.elf"),
    )
    .expect("read 202 M1 XC16 ELF");
    let image = parse_elf_image(&elf).expect("parse ELF");
    assert_eq!(image.arch, labwired_core::Arch::Pic24);

    let console = ConsoleCapture::for_manifest(&system);
    let sink: Arc<Mutex<Vec<u8>>> = console.heard_sink();

    let mut cpu = Pic24::new();
    // The same derivation the session builder uses for this descriptor.
    cpu.set_port_masks(pic24_port_masks(&chip));
    cpu.load_program_image(&image);
    cpu.set_serial_sink(sink.clone());
    cpu.reset(&mut bus).expect("reset");
    cpu.set_pc(0);

    for _ in 0..steps {
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .expect("202 M1 firmware instruction must decode and execute");
    }

    let bytes = sink.lock().unwrap().clone();
    (cpu, bus, bytes)
}

fn gpio<'a>(bus: &'a SystemBus, id: &str) -> &'a Pic24GpioPort {
    bus.peripherals
        .iter()
        .find(|p| p.name == id)
        .unwrap_or_else(|| panic!("no {id} peripheral"))
        .dev
        .as_any()
        .expect("native peripheral")
        .downcast_ref::<Pic24GpioPort>()
        .unwrap_or_else(|| panic!("{id} is not a Pic24GpioPort"))
}

#[test]
fn package_masks_are_the_28_pin_202() {
    let (mut cpu, bus, _) = boot(0);

    // Reset: every package pad is an input over its mask.
    assert_eq!(cpu.data_read(SFR_TRISA).unwrap(), 0x001F, "RA0-RA4");
    assert_eq!(cpu.data_read(SFR_TRISB).unwrap(), 0xFFFF, "RB0-RB15");
    assert_eq!(cpu.data_read(SFR_TRISC).unwrap(), 0x0000, "no PORTC");

    // Writes to pads the package does not have must not stick.
    cpu.data_write(SFR_TRISA, 0xFFFF).unwrap();
    cpu.data_write(SFR_TRISC, 0xFFFF).unwrap();
    assert_eq!(cpu.data_read(SFR_TRISA).unwrap(), 0x001F);
    assert_eq!(cpu.data_read(SFR_TRISC).unwrap(), 0x0000);

    assert_eq!(gpio(&bus, "gpioa").pin_count(), 5);
    assert_eq!(gpio(&bus, "gpiob").pin_count(), 16);
    assert_eq!(gpio(&bus, "gpioc").pin_count(), 0);
}

#[test]
fn firmware_toggles_ra0_and_rb0_through_the_pads() {
    let (mut cpu, mut bus, bytes) = boot(400);
    assert_eq!(
        bytes, b"G1",
        "the firmware reached main() and initialized UART1"
    );
    assert_eq!(cpu.data_read(SFR_TRISA).unwrap(), 0, "PORTA outputs");
    assert_eq!(cpu.data_read(SFR_TRISB).unwrap(), 0, "PORTB outputs");

    let mut last_a = cpu.data_read(SFR_LATA).unwrap();
    let mut last_b = cpu.data_read(SFR_LATB).unwrap();
    let mut lat_changed = false;
    let mut pad_changed = false;
    let mut last_pad = gpio(&bus, "gpioa").level(0);
    for _ in 0..40 {
        cpu.step(&mut bus, &[], &SimulationConfig::default())
            .expect("step the GPIO loop");
        let a = cpu.data_read(SFR_LATA).unwrap();
        let b = cpu.data_read(SFR_LATB).unwrap();
        if a != last_a || b != last_b {
            lat_changed = true;
        }
        last_a = a;
        last_b = b;
        let pad = gpio(&bus, "gpioa").level(0);
        if pad != last_pad {
            pad_changed = true;
            break;
        }
        last_pad = pad;
    }
    assert!(lat_changed, "LATA/LATB never changed in the toggle loop");
    assert!(pad_changed, "the RA0 pad level did not follow the latch");
}
