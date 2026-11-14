// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A keypad key joins its row and column both ways, so the classic row-drive
//! scan and the Arduino `Keypad.h` column-drive scan read the same key.
//!
//! `Keypad.h` sets the rows `INPUT_PULLUP`, pulses each column `OUTPUT LOW`,
//! reads the rows, then ends the pulse with `pinMode(INPUT)`, which on AVR
//! also clears PORT. A column left like that is released, not driven low.
//! Uno wiring: rows D2-D5 (PD2-PD5), columns D6-D9 (PD6, PD7, PB0, PB1).

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::Path;

const PORTB: u64 = 0x0001_0023; // PIN, DDR, PORT
const PORTD: u64 = 0x0001_0029;
const ROWS: [(u64, u8); 4] = [(PORTD, 2), (PORTD, 3), (PORTD, 4), (PORTD, 5)];
const COLS: [(u64, u8); 4] = [(PORTD, 6), (PORTD, 7), (PORTB, 0), (PORTB, 1)];

fn uno_with_keypad() -> SystemBus {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/atmega328p.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: keypad-scan
chip: atmega328p.yaml
external_devices:
  - id: kp
    type: keypad
    connection: gpio
    config:
      row_pins: [PD2, PD3, PD4, PD5]
      col_pins: [PD6, PD7, PB0, PB1]
board_io: []
"#,
    )
    .unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}

fn set(bus: &mut SystemBus, (port, bit): (u64, u8), reg: u64, high: bool) {
    let v = bus.read_u8(port + reg).unwrap();
    let v = if high { v | 1 << bit } else { v & !(1 << bit) };
    bus.write_u8(port + reg, v).unwrap();
    let _ = bus.tick_peripherals_fully();
}

fn read(bus: &SystemBus, (port, bit): (u64, u8)) -> bool {
    bus.read_u8(port).unwrap() & 1 << bit != 0
}

const DDR: u64 = 1;
const PORT: u64 = 2;

/// `Keypad.h` `scanKeys()`, register for register. Returns the pressed keys.
fn column_drive_scan(bus: &mut SystemBus) -> Vec<usize> {
    for row in ROWS {
        set(bus, row, DDR, false);
        set(bus, row, PORT, true); // INPUT_PULLUP
    }
    let mut keys = Vec::new();
    for (c, col) in COLS.into_iter().enumerate() {
        set(bus, col, DDR, true); // pinMode(OUTPUT)
        set(bus, col, PORT, false); // digitalWrite(LOW)
        for (r, row) in ROWS.into_iter().enumerate() {
            if !read(bus, row) {
                keys.push(r * 4 + c);
            }
        }
        set(bus, col, PORT, true); // digitalWrite(HIGH)
        set(bus, col, DDR, false); // pinMode(INPUT): DDR, then PORT cleared
        set(bus, col, PORT, false);
    }
    keys
}

fn row_drive_scan(bus: &mut SystemBus) -> Vec<usize> {
    for col in COLS {
        set(bus, col, DDR, false);
        set(bus, col, PORT, true);
    }
    for row in ROWS {
        set(bus, row, DDR, true);
        set(bus, row, PORT, true);
    }
    let mut keys = Vec::new();
    for (r, row) in ROWS.into_iter().enumerate() {
        set(bus, row, PORT, false);
        for (c, col) in COLS.into_iter().enumerate() {
            if !read(bus, col) {
                keys.push(r * 4 + c);
            }
        }
        set(bus, row, PORT, true);
    }
    keys
}

#[test]
fn keypad_h_column_drive_scan_reads_every_key() {
    let mut bus = uno_with_keypad();
    assert_eq!(column_drive_scan(&mut bus), Vec::<usize>::new());
    for key in 0..16 {
        bus.set_input(Some("kp"), "key", key as f64).unwrap();
        // Twice: the second scan starts from columns released by the first.
        assert_eq!(column_drive_scan(&mut bus), vec![key], "key {key}");
        assert_eq!(column_drive_scan(&mut bus), vec![key], "key {key} rescan");
    }
    bus.set_input(Some("kp"), "key", -1.0).unwrap();
    assert_eq!(column_drive_scan(&mut bus), Vec::<usize>::new());
}

#[test]
fn row_drive_scan_still_reads_every_key() {
    let mut bus = uno_with_keypad();
    for key in 0..16 {
        bus.set_input(Some("kp"), "key", key as f64).unwrap();
        assert_eq!(row_drive_scan(&mut bus), vec![key], "key {key}");
    }
    bus.set_input(Some("kp"), "key", -1.0).unwrap();
    assert_eq!(row_drive_scan(&mut bus), Vec::<usize>::new());
}
