// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Booted ATmega328P instruction fixtures exercise GPIO, SPDR, the parking
//! dispatcher and MAX7219 artifacts together. These are explicit AVR opcodes,
//! not a compiled Arduino/LedControl library qualification.
//! Encodings follow Microchip's AVR Instruction Set Manual, LDI and OUT:
//! https://www.microchip.com/DS40002198
//!
//! MAX7219/MAX7221 data sheet, Serial-Addressing Modes (page 6): MAX7219 shifts
//! regardless of LOAD; its registers latch on LOAD rising. MAX7221 differs.
//! https://www.analog.com/media/en/technical-documentation/data-sheets/MAX7219-MAX7221.pdf

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::inspect::{Artifact, InspectOpts};
use labwired_core::{Cpu, SimulationConfig};
use std::path::PathBuf;

const DDRB: u16 = 0x24;
const PORTB: u16 = 0x25;
const DDRD: u16 = 0x2A;
const PORTD: u16 = 0x2B;
const SPCR: u16 = 0x4C;
const SPDR: u16 = 0x4E;
const MCUCR: u16 = 0x55;
const LOAD: u8 = 1 << 2;
const DIN: u8 = 1 << 3;
const CLK: u8 = 1 << 5;

fn rig(load_pin: &str, cascade: bool) -> (Avr, SystemBus) {
    let chip_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/atmega328p.yaml");
    let chip = ChipDescriptor::from_file(chip_path).unwrap();
    // Deliberately attach the far device first: insertion order must not
    // substitute for the explicit DOUT -> DIN chain order.
    let far = if cascade {
        format!(
            "  - id: far\n    type: led-matrix\n    connection: spi\n    config: {{cs_pin: {load_pin}, chain_index: 1}}\n"
        )
    } else {
        String::new()
    };
    let system = SystemManifest::from_yaml(&format!(
        "name: avr-max7219\nchip: atmega328p\nexternal_devices:\n{far}  - id: near\n    type: led-matrix\n    connection: spi\n    config: {{cs_pin: {load_pin}, chain_index: 0}}\n"
    ))
    .unwrap();
    let bus = SystemBus::from_config(&chip, &system).unwrap();
    (Avr::for_chip_name("atmega328p"), bus)
}

#[derive(Default)]
struct Guest(Vec<u16>);

impl Guest {
    fn write(&mut self, data_address: u16, value: u8) {
        let io = data_address.checked_sub(0x20).unwrap();
        assert!(io < 64);
        // AVR instruction set: LDI r16,K = 1110 KKKK 0000 KKKK;
        // OUT A,r16 = 1011 1AA1 0000 AAAA. For example, LDI r16,0xA5
        // is 0xEA05 and OUT PORTB,r16 (A=5) is 0xB905.
        self.0
            .push(0xE000 | ((u16::from(value) & 0xF0) << 4) | u16::from(value & 0xF));
        self.0.push(0xB900 | ((io & 0x30) << 5) | (io & 0xF));
    }

    fn bits(&mut self, bytes: &[u8], load_high: bool) {
        for &byte in bytes {
            for bit in (0..8).rev() {
                let port =
                    if load_high { LOAD } else { 0 } | if byte & (1 << bit) != 0 { DIN } else { 0 };
                self.write(PORTB, port);
                self.write(PORTB, port | CLK);
                self.write(PORTB, port);
            }
        }
    }

    fn hardware_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write(SPDR, byte);
        }
    }

    fn run(self, cpu: &mut Avr, bus: &mut SystemBus) {
        let start = cpu.get_pc();
        cpu.load_words(start, &self.0);
        let config = SimulationConfig::default();
        for _ in &self.0 {
            cpu.step(bus, &[], &config).expect("AVR guest must execute");
        }
        assert_eq!(cpu.get_pc(), start + 2 * self.0.len() as u32);
    }
}

fn artifact(bus: &SystemBus, id: &str) -> Artifact {
    bus.display_artifact(
        id,
        &InspectOpts {
            include_bytes: true,
            ..Default::default()
        },
    )
    .unwrap_or_else(|| panic!("missing display artifact for {id}"))
}

fn assert_row(bus: &SystemBus, id: &str, row: usize, value: u8) {
    let art = artifact(bus, id);
    assert_eq!(art.meta["shutdown"], false, "{id} must be awake");
    assert_eq!(art.bytes.unwrap()[row], value, "{id} row {row}");
}

#[test]
fn avr_guest_soft_spi_cascade_uses_configured_order_and_load_edges() {
    let (mut cpu, mut bus) = rig("PB2", true);
    let mut init = Guest::default();
    init.write(DDRB, LOAD | DIN | CLK);
    init.write(PORTB, LOAD);
    init.write(PORTB, 0);
    init.bits(&[0x0C, 1, 0x0C, 1], false);
    init.write(PORTB, LOAD);
    init.run(&mut cpu, &mut bus);
    assert_eq!(cpu.spcr & (1 << 6), 0, "guest uses GPIO, not SPDR");

    let mut paint = Guest::default();
    paint.write(PORTB, 0);
    paint.bits(&[0x01, 0xA5, 0x01, 0x3C], false);
    paint.run(&mut cpu, &mut bus);
    assert_row(&bus, "far", 0, 0);
    assert_row(&bus, "near", 0, 0);

    let mut latch = Guest::default();
    latch.write(PORTB, LOAD);
    latch.run(&mut cpu, &mut bus);
    assert_row(&bus, "far", 0, 0xA5);
    assert_row(&bus, "near", 0, 0x3C);
}

#[test]
fn avr_guest_hardware_spi_load_high_shifts_but_only_a_rising_edge_latches() {
    let (mut cpu, mut bus) = rig("PB2", true);
    let mut init = Guest::default();
    init.write(DDRB, LOAD | DIN | CLK);
    init.write(SPCR, 0x50); // SPE | MSTR
    init.write(PORTB, LOAD);
    init.write(PORTB, 0);
    init.hardware_bytes(&[0x0C, 1, 0x0C, 1]);
    init.write(PORTB, LOAD); // must latch even while SPE=1
    init.run(&mut cpu, &mut bus);
    assert_row(&bus, "near", 0, 0);
    assert_row(&bus, "far", 0, 0);

    let mut shift_high = Guest::default();
    shift_high.hardware_bytes(&[0x01, 0xA5, 0x01, 0x3C]);
    shift_high.write(PORTB, LOAD);
    shift_high.write(PORTB, LOAD | DIN);
    shift_high.run(&mut cpu, &mut bus);
    assert_row(&bus, "near", 0, 0);
    assert_row(&bus, "far", 0, 0);

    let mut falling = Guest::default();
    falling.write(PORTB, 0);
    falling.run(&mut cpu, &mut bus);
    assert_row(&bus, "near", 0, 0);
    let mut rising = Guest::default();
    rising.write(PORTB, LOAD);
    rising.run(&mut cpu, &mut bus);
    assert_row(&bus, "far", 0, 0xA5);
    assert_row(&bus, "near", 0, 0x3C);
}

#[test]
fn avr_guest_software_spi_clocks_with_load_high() {
    let (mut cpu, mut bus) = rig("PB2", false);
    let mut guest = Guest::default();
    guest.write(DDRB, LOAD | DIN | CLK);
    guest.write(PORTB, LOAD);
    guest.bits(&[0x0C, 1], true);
    guest.run(&mut cpu, &mut bus);
    assert_eq!(artifact(&bus, "near").meta["shutdown"], true);

    let mut latch = Guest::default();
    latch.write(PORTB, 0);
    latch.write(PORTB, LOAD);
    latch.run(&mut cpu, &mut bus);
    assert_row(&bus, "near", 0, 0);
}

#[test]
fn avr_guest_cross_port_load_release_resets_an_incomplete_software_byte() {
    let (mut cpu, mut bus) = rig("PD2", false);
    let mut init = Guest::default();
    init.write(DDRB, DIN | CLK);
    init.write(DDRD, LOAD);
    init.write(PORTD, LOAD);
    init.write(PORTD, 0);
    init.bits(&[0x0C, 1], false);
    init.write(PORTD, LOAD);
    init.run(&mut cpu, &mut bus);

    let mut guest = Guest::default();
    guest.write(PORTD, 0);
    for _ in 0..3 {
        guest.write(PORTB, DIN);
        guest.write(PORTB, DIN | CLK);
        guest.write(PORTB, DIN);
    }
    guest.write(PORTD, LOAD); // partial byte must not survive this edge
    guest.write(PORTD, 0);
    guest.bits(&[0x01, 0xA5], false);
    guest.write(PORTD, LOAD);
    guest.run(&mut cpu, &mut bus);
    assert_row(&bus, "near", 0, 0xA5);
}

#[test]
fn avr_guest_wrong_din_clock_wiring_does_not_report_a_working_display() {
    let (mut cpu, mut bus) = rig("PB2", false);
    let mut guest = Guest::default();
    guest.write(DDRB, LOAD | (1 << 4) | (1 << 3));
    guest.write(PORTB, LOAD);
    guest.write(PORTB, 0);
    // LedControl(12, 11, 10, 1) toggles PB4/PB3, never the supported PB5 CLK.
    for byte in [0x0Cu8, 1] {
        for bit in (0..8).rev() {
            let port = if byte & (1 << bit) != 0 { 1 << 4 } else { 0 };
            guest.write(PORTB, port);
            guest.write(PORTB, port | (1 << 3));
            guest.write(PORTB, port);
        }
    }
    guest.write(PORTB, LOAD);
    guest.run(&mut cpu, &mut bus);
    assert_eq!(artifact(&bus, "near").meta["shutdown"], true);
}

#[test]
fn avr_guest_input_only_din_or_clock_cannot_write_a_frame() {
    for ddr in [LOAD, LOAD | DIN, LOAD | CLK] {
        let (mut cpu, mut bus) = rig("PB2", false);
        let mut guest = Guest::default();
        guest.write(MCUCR, 1 << 4); // PUD: PORT writes cannot enable pull-ups.
        guest.write(DDRB, ddr); // LOAD is driven; DIN or CLK is still an input.
        guest.write(PORTB, LOAD);
        guest.write(PORTB, 0);
        guest.bits(&[0x0C, 1], false);
        guest.write(PORTB, LOAD);
        guest.run(&mut cpu, &mut bus);
        assert_eq!(
            artifact(&bus, "near").meta["shutdown"],
            true,
            "DDRB={ddr:#04x}: input latch writes must not clock a command"
        );
    }
}

#[test]
fn avr_guest_direction_loss_discards_partial_bits_and_reacquires_clock_level() {
    for reacquire_high in [false, true] {
        let (mut cpu, mut bus) = rig("PB2", false);
        let mut guest = Guest::default();
        guest.write(MCUCR, 1 << 4); // PUD
        guest.write(DDRB, LOAD | DIN | CLK);
        guest.write(PORTB, LOAD);
        guest.write(PORTB, 0);
        guest.bits(&[0x0C, 1], false);
        guest.write(PORTB, LOAD);

        guest.write(PORTB, 0);
        for _ in 0..3 {
            guest.write(PORTB, DIN);
            guest.write(PORTB, DIN | CLK);
            guest.write(PORTB, DIN);
        }
        // No LOAD edge can reset this partial byte. Losing DDR ownership must.
        guest.write(DDRB, LOAD);
        if reacquire_high {
            guest.write(PORTB, DIN | CLK);
        }
        guest.write(DDRB, LOAD | DIN | CLK);
        if reacquire_high {
            // Repeating the acquired high level must not count as a new edge.
            guest.write(PORTB, DIN | CLK);
        }
        guest.bits(&[0x01, 0xA5], false);
        guest.write(PORTB, LOAD);
        guest.run(&mut cpu, &mut bus);
        assert_row(&bus, "near", 0, 0xA5);
    }
}
