// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **ADXL345 register map against the datasheet, not against the model.**
//!
//! The expectations below are transcribed from Table 16, "Register Map", on
//! page 14 of the Analog Devices ADXL345 datasheet Rev. 0 — the same document
//! the Part Knowledge corpus cites, sha256 `2c0d2a0e…`, which states reset
//! values in binary. They are written from the document so that a model which
//! drifts from it fails here rather than agreeing with itself.
//!
//! What this caught: before the full map was declared, the descriptor held six
//! register entries and every other documented address fell through to the
//! declarative engine's unmapped-read default of `0xFF`. Silicon answers `0x00`
//! for those (and `0x0A` for BW_RATE, `0x02` for INT_SOURCE), so a driver doing
//! read-modify-write on ACT_INACT_CTL, or polling INT_SOURCE for a tap, read
//! values the part never produces — and writes to them were discarded, so a
//! configure-then-verify sequence could not pass.
//!
//! Scope, stated plainly: this asserts the *register interface* — reset values,
//! access, and that R/W registers store what a driver writes. It does not
//! assert behaviour, because the detectors are not modelled. ACT_TAP_STATUS and
//! INT_SOURCE never change on their own, and nothing here pretends otherwise.

use labwired_core::peripherals::components::declarative_spi::GenericSpiDevice;
use labwired_core::peripherals::spi::SpiDevice;
use labwired_core::sim_input::SimInput;

fn adxl345() -> GenericSpiDevice {
    let yaml = labwired_config::embedded_device_yaml("adxl345_spi")
        .expect("adxl345_spi descriptor is embedded");
    GenericSpiDevice::from_yaml(yaml, "CS").expect("adxl345_spi.yaml must parse")
}

/// 4-wire SPI command byte: bit 7 = read, bit 6 = multi-byte, bits 5:0 = addr.
fn read_u8(d: &mut GenericSpiDevice, addr: u8) -> u8 {
    d.cs_select();
    d.transfer(0x80 | (addr & 0x3F));
    let value = d.transfer(0x00);
    d.cs_release();
    value
}

fn write_u8(d: &mut GenericSpiDevice, addr: u8, value: u8) {
    d.cs_select();
    d.transfer(addr & 0x3F);
    d.transfer(value);
    d.cs_release();
}

/// Table 16, page 14. `(address, name, access as written, reset value)`.
/// Reset values are the table's binary column converted to hex.
/// DATAX0..DATAZ1 (0x32-0x37) are omitted: the model spans them as three
/// two-byte registers driven by the accel inputs, so their read value is a
/// function of stimulus rather than a fixed reset, and byte parity for that
/// path is already pinned in declarative_device_byte_parity.rs.
const TABLE_16: &[(u8, &str, &str, u8)] = &[
    (0x00, "DEVID", "R", 0xE5),
    (0x1D, "THRESH_TAP", "R/W", 0x00),
    (0x1E, "OFSX", "R/W", 0x00),
    (0x1F, "OFSY", "R/W", 0x00),
    (0x20, "OFSZ", "R/W", 0x00),
    (0x21, "DUR", "R/W", 0x00),
    (0x22, "Latent", "R/W", 0x00),
    (0x23, "Window", "R/W", 0x00),
    (0x24, "THRESH_ACT", "R/W", 0x00),
    (0x25, "THRESH_INACT", "R/W", 0x00),
    (0x26, "TIME_INACT", "R/W", 0x00),
    (0x27, "ACT_INACT_CTL", "R/W", 0x00),
    (0x28, "THRESH_FF", "R/W", 0x00),
    (0x29, "TIME_FF", "R/W", 0x00),
    (0x2A, "TAP_AXES", "R/W", 0x00),
    (0x2B, "ACT_TAP_STATUS", "R", 0x00),
    (0x2C, "BW_RATE", "R/W", 0x0A),
    (0x2D, "POWER_CTL", "R/W", 0x00),
    (0x2E, "INT_ENABLE", "R/W", 0x00),
    (0x2F, "INT_MAP", "R/W", 0x00),
    (0x30, "INT_SOURCE", "R", 0x02),
    (0x31, "DATA_FORMAT", "R/W", 0x00),
    (0x38, "FIFO_CTL", "R/W", 0x00),
    (0x39, "FIFO_STATUS", "R", 0x00),
];

#[test]
fn every_documented_register_reads_its_datasheet_reset_value() {
    let mut d = adxl345();
    let mut wrong = Vec::new();
    for &(addr, name, _, reset) in TABLE_16 {
        let got = read_u8(&mut d, addr);
        if got != reset {
            wrong.push(format!(
                "{name} (0x{addr:02X}): got 0x{got:02X}, datasheet 0x{reset:02X}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "registers disagree with Table 16:\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
fn writable_registers_store_what_a_driver_writes() {
    // The configure-then-verify sequence a real driver performs. 0x5A is chosen
    // to differ from every reset value in the table, so a register that silently
    // discards the write fails rather than coincidentally matching.
    let mut d = adxl345();
    let mut lost = Vec::new();
    for &(addr, name, access, _) in TABLE_16 {
        if access != "R/W" {
            continue;
        }
        write_u8(&mut d, addr, 0x5A);
        let got = read_u8(&mut d, addr);
        if got != 0x5A {
            lost.push(format!(
                "{name} (0x{addr:02X}): wrote 0x5A, read 0x{got:02X}"
            ));
        }
    }
    assert!(
        lost.is_empty(),
        "writes did not stick:\n  {}",
        lost.join("\n  ")
    );
}

#[test]
fn read_only_registers_ignore_writes() {
    let mut d = adxl345();
    for &(addr, name, access, reset) in TABLE_16 {
        if access != "R" {
            continue;
        }
        write_u8(&mut d, addr, 0x5A);
        assert_eq!(
            read_u8(&mut d, addr),
            reset,
            "{name} (0x{addr:02X}) is read-only in Table 16 but accepted a write"
        );
    }
}

#[test]
fn the_table_covers_every_address_the_datasheet_documents() {
    // Guards the failure where this file quietly shrinks: Table 16 names thirty
    // registers, six of which are the DATAX0..DATAZ1 pair halves excluded above.
    assert_eq!(
        TABLE_16.len(),
        24,
        "Table 16 has 30 registers, 6 of them data-pair halves"
    );
}

#[test]
fn an_undocumented_address_still_reads_as_unmapped() {
    // 0x3A is past FIFO_STATUS and is not a register. The engine's 0xFF for an
    // unmapped address is correct here; the defect was that documented
    // registers were reaching it, not the default itself.
    let mut d = adxl345();
    assert_eq!(read_u8(&mut d, 0x3A), 0xFF);
}

#[test]
fn each_data_byte_is_addressable_on_its_own() {
    // Table 16 numbers all six data bytes separately (0x32-0x37), and page 18
    // says "the least significant byte does not have to be read if that
    // information is not needed" -- so pointing at the high byte alone is a
    // supported access, not an edge case.
    //
    // The model declares them as three two-byte registers, which is right for a
    // burst. What was wrong was a single-byte read: selecting registers by
    // `addr >= start` skipped the register containing the address and answered
    // from the next one, so 0x37 returned FIFO_CTL rather than DATAZ1 -- a
    // different register's value, indistinguishable from real sensor data.
    let mut d = adxl345();
    d.set_input("accel_z", 1.0).unwrap();

    // 1 g x 256 LSB/g = 0x0100, little-endian across 0x36 (lo) and 0x37 (hi).
    d.cs_select();
    d.transfer(0xC0 | 0x36);
    let lo = d.transfer(0x00);
    let hi = d.transfer(0x00);
    d.cs_release();
    assert_eq!((lo, hi), (0x00, 0x01), "burst read of DATAZ");

    // The same two bytes, addressed one at a time.
    assert_eq!(read_u8(&mut d, 0x36), lo, "DATAZ0 read on its own");
    assert_eq!(read_u8(&mut d, 0x37), hi, "DATAZ1 read on its own");
}

#[test]
fn a_read_inside_a_register_still_walks_on_to_the_next() {
    // Starting mid-register must not break auto-increment: after the high byte
    // of DATAZ the next byte is FIFO_CTL, exactly as a burst from 0x36 would
    // reach it.
    let mut d = adxl345();
    d.set_input("accel_z", 1.0).unwrap();
    d.cs_select();
    d.transfer(0xC0 | 0x37);
    let hi = d.transfer(0x00);
    let next = d.transfer(0x00);
    d.cs_release();
    assert_eq!(hi, 0x01, "DATAZ1");
    assert_eq!(next, 0x00, "FIFO_CTL follows");
}
