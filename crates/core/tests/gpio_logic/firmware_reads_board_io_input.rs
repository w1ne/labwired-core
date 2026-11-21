// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A part that drives an MCU input pin must be seen by the FIRMWARE, on every
//! family, through the load instruction the firmware itself executes.
//!
//! Each row boots the committed chip yaml with an active-low `board_io` button
//! on one pin (the shape the canvas compiler emits for a button, an encoder's
//! SW contact, ...), runs a poll loop on the CPU that copies the pin's input
//! register into RAM, and presses / releases the button from the stimulus API
//! while the loop runs. The RAM copy is what the firmware saw. A row fails
//! when the level reaches the pad model but not the register a load returns,
//! or reaches it only after the loop has stopped looking.
//!
//! The loop is Thumb on every row, as in `logic_wire_channels_cross_family`:
//! the pin path under test is the bus (board_io -> pad -> input register ->
//! CPU load), which no ISA changes. `setup` is the register writes a real
//! driver makes before it reads the pin (nRF52 connects the input buffer in
//! `PIN_CNF`); everything else is the chip's reset state.

use crate::common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::{Bus, Machine};

struct Row {
    chip: &'static str,
    /// `board_io` binding: GPIO peripheral id and pin number.
    port: &'static str,
    pin: u8,
    /// The register the firmware loads and the pin's bit in it (datasheet).
    input: (u64, u8),
    /// Driver writes made before the pin is read.
    setup: &'static [(u64, u32)],
}

#[rustfmt::skip]
const ROWS: &[Row] = &[
    // PIND (ATmega328P DS 14.4.10, data space 0x29).
    Row { chip: "atmega328p", port: "portd", pin: 2, input: (0x0001_0029, 2), setup: &[] },
    // GPIOA_IDR (RM0008 9.2.3 / RM0368 8.4.5 / RM0090 8.4.5).
    Row { chip: "stm32f103", port: "gpioa", pin: 0, input: (0x4001_0808, 0), setup: &[] },
    Row { chip: "stm32f401", port: "gpioa", pin: 0, input: (0x4002_0010, 0), setup: &[] },
    Row { chip: "stm32f407", port: "gpioa", pin: 0, input: (0x4002_0010, 0), setup: &[] },
    // SIO GPIO_IN (RP2040 DS 2.3.1.7, RP2350 DS 3.1.11).
    Row { chip: "rp2040", port: "sio", pin: 10, input: (0xD000_0004, 10), setup: &[] },
    Row { chip: "rp2350", port: "sio", pin: 10, input: (0xD000_0004, 10), setup: &[] },
    // P0.IN (nRF52840 PS 6.9.2); PIN_CNF[11] = 0 connects the input buffer.
    Row {
        chip: "nrf52840", port: "gpio0", pin: 11, input: (0x5000_0510, 11),
        setup: &[(0x5000_0700 + 4 * 11, 0)],
    },
    // GPIO_IN_REG (ESP32 TRM 4.13, ESP32-C3 TRM 5.14, ESP32-S3 TRM 6.14).
    Row { chip: "esp32", port: "gpio", pin: 4, input: (0x3FF4_403C, 4), setup: &[] },
    Row { chip: "esp32c3", port: "gpio", pin: 4, input: (0x6000_403C, 4), setup: &[] },
    Row { chip: "esp32s3", port: "gpio", pin: 4, input: (0x6000_403C, 4), setup: &[] },
];

fn machine(row: &Row) -> (Machine<CortexM>, u64, u8, u64) {
    let path = root(&format!("configs/chips/{}.yaml", row.chip));
    let chip = ChipDescriptor::from_file(&path).expect("load chip");
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        r#"
name: "firmware-reads-input"
chip: "{}"
external_devices: []
board_io:
  - id: "btn"
    kind: "button"
    peripheral: "{}"
    pin: {}
    signal: "input"
    active_high: false
"#,
        path.display(),
        row.port,
        row.pin
    ))
    .expect("parse manifest");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    // Clock enables are a driver's job and differ per family; the input path
    // is what this gate measures.
    bus.set_clock_gating_bypass(true);
    for &(addr, value) in row.setup {
        bus.write_u32(addr, value).expect("driver setup write");
    }
    let (addr, bit) = row.input;
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    let ram = chip.ram.base;
    let code = ram + 0x200;
    let out = ram + 0x600;
    let byte = addr + u64::from(bit / 8);
    // ldr r0,[pc,#8] ; ldr r2,[pc,#12] ; loop: ldrb r1,[r0] ; strb r1,[r2] ;
    // b loop ; nop ; .word byte ; .word out
    let halfwords: [u16; 6] = [0x4802, 0x4A03, 0x7801, 0x7011, 0xE7FC, 0xBF00];
    for (i, hw) in halfwords.iter().enumerate() {
        machine.bus.write_u16(code + 2 * i as u64, *hw).unwrap();
    }
    machine.bus.write_u32(code + 12, byte as u32).unwrap();
    machine.bus.write_u32(code + 16, out as u32).unwrap();
    machine.bus.write_u8(out, 0xA5).unwrap();
    machine.cpu.pc = code as u32;
    (machine, out, bit, addr)
}

fn seen(machine: &mut Machine<CortexM>, out: u64, bit: u8) -> bool {
    for _ in 0..64 {
        machine.step().expect("step");
    }
    machine.bus.read_u8(out).unwrap() >> (bit % 8) & 1 != 0
}

#[test]
fn firmware_reads_a_board_io_input_on_every_family() {
    let mut failures = Vec::new();
    for row in ROWS {
        let (mut m, out, bit, addr) = machine(row);
        let released = seen(&mut m, out, bit);
        m.bus.set_input(Some("btn"), "pressed", 1.0).expect("press");
        let pressed = seen(&mut m, out, bit);
        m.bus
            .set_input(Some("btn"), "pressed", 0.0)
            .expect("release");
        let again = seen(&mut m, out, bit);
        let ok = released && !pressed && again;
        println!(
            "{:<12} {:<6} pin {:<2} reg {addr:#010x} bit {:<2} released={} pressed={} released={} {}",
            row.chip,
            row.port,
            row.pin,
            bit,
            u8::from(released),
            u8::from(pressed),
            u8::from(again),
            if ok { "ok" } else { "FAIL" }
        );
        if !ok {
            failures.push(row.chip);
        }
    }
    assert!(
        failures.is_empty(),
        "firmware did not read the button level on: {failures:?}"
    );
}
