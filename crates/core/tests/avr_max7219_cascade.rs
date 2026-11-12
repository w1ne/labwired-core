// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Booted ATmega328P instruction fixtures exercise GPIO, SPDR, the parking
//! dispatcher and MAX7219 artifacts together. These are explicit AVR opcodes,
//! not a compiled Arduino/LedControl library qualification.
//! Encodings follow Microchip's AVR Instruction Set Manual, LDI, IN and OUT:
//! https://www.microchip.com/DS40002198
//!
//! MAX7219/MAX7221 data sheet, Serial-Addressing Modes (page 6): MAX7219 shifts
//! regardless of LOAD; its registers latch on LOAD rising. MAX7221 differs.
//! https://www.analog.com/media/en/technical-documentation/data-sheets/MAX7219-MAX7221.pdf

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::inspect::{Artifact, InspectOpts};
use labwired_core::peripherals::components::declarative_display::DeclarativeDisplayKit;
use labwired_core::peripherals::kit::{AttachCtx, PeripheralKit};
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
const PEER_CS: u8 = 1 << 1;
const DC: u8 = 1;
const DIN: u8 = 1 << 3;
const CLK: u8 = 1 << 5;

fn rig_with_devices(devices: &str) -> (Avr, SystemBus) {
    let chip_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/atmega328p.yaml");
    let chip = ChipDescriptor::from_file(chip_path).unwrap();
    let system = SystemManifest::from_yaml(&format!(
        "name: avr-max7219\nchip: atmega328p\nexternal_devices:\n{devices}"
    ))
    .unwrap();
    let bus = SystemBus::from_config(&chip, &system).unwrap();
    (Avr::for_chip_name("atmega328p"), bus)
}

fn rig(load_pin: &str, cascade: bool) -> (Avr, SystemBus) {
    // Deliberately attach the far device first: insertion order must not
    // substitute for the explicit DOUT -> DIN chain order.
    let far = if cascade {
        format!(
            "  - id: far\n    type: led-matrix\n    connection: spi\n    config: {{cs_pin: {load_pin}, chain_index: 1}}\n"
        )
    } else {
        String::new()
    };
    rig_with_devices(&format!(
        "{far}  - id: near\n    type: led-matrix\n    connection: spi\n    config: {{cs_pin: {load_pin}, chain_index: 0}}\n"
    ))
}

fn mixed_rig(peer_type: &str, config: &str, with_max7219: bool) -> (Avr, SystemBus) {
    let max = if with_max7219 {
        "  - id: near\n    type: led-matrix\n    connection: spi\n    config: {cs_pin: PB2, chain_index: 0}\n"
    } else {
        ""
    };
    rig_with_devices(&format!(
        "{max}  - id: peer\n    type: {peer_type}\n    connection: spi\n    config: {{{config}}}\n"
    ))
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

    fn read(&mut self, data_address: u16, register: u8) {
        let io = data_address.checked_sub(0x20).unwrap();
        assert!(io < 64 && register < 32);
        // IN Rd,A = 1011 0AAd dddd AAAA; IN r17,SPDR is 0xB51E.
        self.0
            .push(0xB000 | ((io & 0x30) << 5) | (u16::from(register) << 4) | (io & 0xF));
    }

    fn hardware_init(&mut self) {
        self.write(PORTB, LOAD | PEER_CS);
        self.write(DDRB, LOAD | PEER_CS | DC | DIN | CLK);
        self.write(SPCR, 0x50); // SPE | MSTR
    }

    fn peer_transaction(&mut self, bytes: &[u8], response_register: u8) {
        self.write(PORTB, LOAD); // peer selected; MAX7219 LOAD remains high
        self.hardware_bytes(bytes);
        self.read(SPDR, response_register);
        self.write(PORTB, LOAD | PEER_CS);
    }

    fn panel_command(&mut self, command: u8, data: &[u8], cs_high: bool) {
        let port = LOAD | if cs_high { PEER_CS } else { 0 };
        self.write(PORTB, port);
        self.write(SPDR, command);
        self.write(PORTB, port | DC);
        self.hardware_bytes(data);
    }

    fn white_block(&mut self) {
        self.panel_command(0x01, &[], false); // SWRESET
        self.panel_command(0x11, &[], false); // SLPOUT
        self.panel_command(0x3A, &[0x55], false); // RGB565
        self.panel_command(0x29, &[], false); // DISPON
        self.panel_command(0x2A, &[0, 0, 0, 1], false); // x0..1
        self.panel_command(0x2B, &[0, 0, 0, 1], false); // y0..1
        self.panel_command(0x2C, &[0xFF; 8], false); // four white pixels
        self.write(PORTB, LOAD | PEER_CS);
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

#[test]
fn avr_guest_mixed_bus_keeps_real_slave_commands_intact_and_ignores_cs_high() {
    // Each read requires one uninterrupted command across several SPDR writes.
    // The deselected command would change the subsequent read if it leaked.
    type CommandCase = (
        &'static str,
        &'static [u8],
        &'static [u8],
        &'static [u8],
        u8,
    );
    let cases: &[CommandCase] = &[
        (
            "mcp2515",
            &[0x02, 0x2A, 0x55],
            &[0x03, 0x2A, 0],
            &[0x02, 0x2A, 0xAA],
            0x55,
        ),
        ("rc522", &[0x22, 0x5A], &[0xA2, 0], &[0x22, 0xAA], 0x5A),
        (
            "microsd",
            &[0x40, 0, 0, 0, 0, 0x95],
            &[0x77, 0, 0, 0, 0, 0],
            &[0x69, 0, 0, 0, 0, 0],
            1,
        ),
    ];
    for &(kind, write, read, deselected_write, expected) in cases {
        for with_max in [false, true] {
            for cpu_owned in [false, true] {
                let (mut cpu, mut bus) = mixed_rig(kind, "cs_pin: PB1", with_max);
                if cpu_owned {
                    for device in bus.take_spi_devices("spi") {
                        cpu.push_spi_device(device);
                    }
                }
                let mut guest = Guest::default();
                guest.hardware_init();
                guest.peer_transaction(write, 19);
                guest.peer_transaction(read, 17);
                guest.hardware_bytes(deselected_write); // all configured CS high
                guest.peer_transaction(read, 18);
                guest.run(&mut cpu, &mut bus);
                assert_eq!(
                    cpu.r[17], expected,
                    "{kind}, MAX={with_max}, CPU-owned={cpu_owned}"
                );
                assert_eq!(
                    cpu.r[18], expected,
                    "{kind}: CS-high write leaked, MAX={with_max}, CPU-owned={cpu_owned}"
                );
            }
        }
    }
}

#[test]
fn avr_guest_mixed_st7789_paints_with_cs_held_and_ignores_deselected_reset() {
    for with_max in [false, true] {
        let (mut cpu, mut bus) = mixed_rig("st7789-170x320", "cs_pin: PB1, dc_pin: PB0", with_max);
        let mut guest = Guest::default();
        guest.hardware_init();
        guest.white_block();
        guest.panel_command(0x01, &[], true); // CS-high SWRESET must be ignored
        guest.run(&mut cpu, &mut bus);
        let art = artifact(&bus, "peer");
        assert_eq!(art.meta["awake"], true);
        assert_eq!(art.meta["display_on"], true);
        assert_eq!(art.meta["painted_bytes"], 8);
        assert_eq!(
            art.bytes.unwrap().iter().filter(|&&byte| byte != 0).count(),
            8
        );
    }
}

#[test]
fn avr_guest_cs_less_displays_receive_mosi_in_parallel_with_a_max7219() {
    let (mut cpu, mut bus) = rig_with_devices(
        "  - id: near\n    type: led-matrix\n    connection: spi\n    config: {cs_pin: PB2}\n  - id: first\n    type: st7789-170x320\n    connection: spi\n    config: {cs_pin: '', dc_pin: PB0}\n  - id: second\n    type: st7789-170x320\n    connection: spi\n    config: {cs_pin: '', dc_pin: PB0}\n"
    );
    let mut guest = Guest::default();
    guest.hardware_init();
    guest.white_block();
    guest.run(&mut cpu, &mut bus);
    for id in ["first", "second"] {
        let art = artifact(&bus, id);
        assert_eq!(art.meta["display_on"], true, "{id}");
        assert_eq!(art.meta["painted_bytes"], 8, "{id}");
    }
}

#[test]
fn avr_guest_clocked_strip_receives_frames_with_or_without_a_cs_label() {
    for cs_pin in ["PB1", ""] {
        let (mut cpu, mut bus) = mixed_rig(
            "apa102",
            &format!("cs_pin: '{cs_pin}', num_pixels: 1"),
            true,
        );
        let mut guest = Guest::default();
        guest.hardware_init();
        guest.peer_transaction(&[0, 0, 0, 0, 0xFF, 0x12, 0x34, 0x56, 0, 0, 0, 0], 17);
        guest.run(&mut cpu, &mut bus);
        if cs_pin.is_empty() {
            // The existing spi_frames descriptor uses cs_release as its model
            // transaction boundary. This assertion proves no-CS byte delivery;
            // it does not claim a physical APA102 has a chip-select pin.
            let index = bus.find_peripheral_index_by_name("spi").unwrap();
            for device in bus.peripherals[index]
                .dev
                .spi_attached_devices_mut()
                .unwrap()
            {
                if device.cs_pin().is_empty() {
                    device.cs_release();
                }
            }
        }
        assert_eq!(artifact(&bus, "peer").bytes, Some(vec![0x56, 0x34, 0x12]));
    }
}

#[test]
fn avr_guest_display_gpio_handle_forwards_selection_without_snapshotting_cs() {
    let (mut cpu, mut bus) = rig("PB2", false);
    let yaml = include_str!("fixtures/st7735r-memory.yaml")
        .replace("    width: 132", "    glass_crop: true\n    width: 132");
    let kit = DeclarativeDisplayKit::from_yaml(&yaml).unwrap();
    let ext = serde_yaml::from_str(
        "id: peer\ntype: st7735r-memory-prototype\nconnection: spi\nconfig: {cs_pin: PB1, dc_pin: PB0, reset_pin: PD2, backlight_pin: PD3, col_offset: 0, row_offset: 0, cols: 1, rows: 1}",
    )
    .unwrap();
    kit.attach(&mut AttachCtx::new(&mut bus, &ext)).unwrap();

    let mut guest = Guest::default();
    guest.hardware_init();
    guest.write(DDRD, (1 << 2) | (1 << 3));
    guest.write(PORTD, (1 << 2) | (1 << 3)); // release reset, light on
    guest.panel_command(0x01, &[], false);
    guest.panel_command(0x11, &[], false);
    guest.panel_command(0x29, &[], false);
    guest.panel_command(0x2A, &[0, 0, 0, 0], false);
    guest.panel_command(0x2B, &[0, 0, 0, 0], false);
    guest.panel_command(0x2C, &[0xFC, 0, 0], false); // RGB666 red; CS stays low
    guest.run(&mut cpu, &mut bus);
    assert_eq!(artifact(&bus, "peer").bytes, Some(vec![255, 0, 0]));

    let index = bus.find_peripheral_index_by_name("spi").unwrap();
    let snapshot = {
        let devices = bus.peripherals[index].dev.spi_attached_devices().unwrap();
        let peer = devices
            .iter()
            .find(|device| device.cs_pin() == "PB1")
            .unwrap();
        assert!(peer.is_selected());
        peer.runtime_snapshot()
    };
    let mut deselect = Guest::default();
    deselect.write(PORTB, LOAD | PEER_CS);
    deselect.run(&mut cpu, &mut bus);
    let devices = bus.peripherals[index]
        .dev
        .spi_attached_devices_mut()
        .unwrap();
    let peer = devices
        .iter_mut()
        .find(|device| device.cs_pin() == "PB1")
        .unwrap();
    assert!(!peer.is_selected());
    peer.restore_runtime_snapshot(&snapshot).unwrap();
    assert!(
        !peer.is_selected(),
        "picture-only restore must not change the CS wire"
    );
}
