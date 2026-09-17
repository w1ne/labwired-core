// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ADXL345 (I²C): the declarative descriptor against the hand-written model it
//! replaces.
//!
//! `components/adxl345.rs` is DELETED. The transcript that model produced is
//! pinned here as golden bytes, captured by running [`migration_script`]
//! against it on the commit that removed it.
//!
//! Held identical — every register the old model actually served, which is all
//! a driver could see of it:
//!   * DEVID 0xE5, BW_RATE 0x0A, POWER_CTL, DATA_FORMAT;
//!   * the six data bytes, including the rest sample of 1 g on Z;
//!   * the full-scale behaviour across DATA_FORMAT: ±512 counts in fixed
//!     10-bit mode at every range, and ±512/1024/2048/4096 in full resolution.
//!     That is the `clamp_from` primitive this port exists for, and the golden
//!     bytes are what the hand-written model produced for the same settings.
//!
//! DELIBERATELY DIFFERENT, asserted separately so neither can be lost:
//!   * the whole Table 16 map is served at its datasheet reset values. The old
//!     model answered six addresses and returned 0x00 for every other one, and
//!     discarded every write to the sixteen R/W configuration registers.
//!   * the conversion happens at READ time, not at `set_input` time. See
//!     `a_range_change_is_visible_to_the_next_read`.
//!   * the DATA_READY interrupt reaches a pad.
//!
//! NOT modelled, and named in `configs/devices/adxl345.yaml`: output data rates
//! other than the 100 Hz reset value, tap/activity/free-fall detection, the
//! FIFO, the offset trim and the JUSTIFY bit.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::sim_input::SimInput;

mod common;
use common::transcript::{read_reg, run_i2c, script, write_reg, Step};

const ADDR: u8 = 0x53;

fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("adxl345")
        .expect("adxl345 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("adxl345.yaml does not build")
}

/// The conversation the deleted model was driven through. The last three
/// blocks are the `clamp_from` matrix: the same 16 g of acceleration read
/// through three different DATA_FORMAT settings.
fn migration_script() -> Vec<Step<'static>> {
    script([
        read_reg(0x00, 1),        // DEVID
        read_reg(0x2C, 2),        // BW_RATE, POWER_CTL
        write_reg(0x31, &[0x0B]), // DATA_FORMAT: full-res, ±16 g
        read_reg(0x31, 1),
        write_reg(0x2D, &[0x08]), // POWER_CTL: measure
        read_reg(0x2D, 1),
        read_reg(0x32, 6), // the rest sample
        vec![
            Step::Input("x", 1.0),
            Step::Input("y", -0.5),
            Step::Input("z", 0.25),
        ],
        read_reg(0x32, 6),
        // Fixed 10-bit, ±2 g: 256 counts/g, saturating at ±512.
        write_reg(0x31, &[0x00]),
        vec![Step::Input("x", 1.0)],
        read_reg(0x32, 2),
        // Fixed 10-bit, ±16 g: 32 counts/g — still ±512 counts at full scale.
        write_reg(0x31, &[0x03]),
        vec![Step::Input("x", 16.0)],
        read_reg(0x32, 2),
        // Full resolution, ±16 g: 256 counts/g and ±4096 counts.
        write_reg(0x31, &[0x0B]),
        vec![Step::Input("x", 16.0)],
        read_reg(0x32, 2),
    ])
}

/// ⚠️ GOLDEN — captured from `components/adxl345.rs` on the commit that deleted
/// it, by running [`migration_script`] against it.
const ADXL345_GOLDEN: &[u8] = &[
    0xE5, // DEVID
    0x0A, 0x00, // BW_RATE, POWER_CTL
    0x0B, // DATA_FORMAT read back
    0x08, // POWER_CTL read back
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // rest: z = 256 counts
    0x00, 0x01, 0x80, 0xFF, 0x40, 0x00, // 1 g, -0.5 g, 0.25 g
    0x00, 0x01, // ±2 g fixed: 1 g = 256
    0x00, 0x02, // ±16 g fixed: 16 g = 512, the 10-bit full scale
    0x00, 0x10, // ±16 g full-res: 16 g = 4096
];

#[test]
fn the_wire_transcript_is_byte_identical_to_the_deleted_model() {
    let transcript = run_i2c(&mut declarative(), &migration_script());
    assert_eq!(
        transcript.bytes,
        ADXL345_GOLDEN,
        "the declarative ADXL345 moved a byte the hand-written one did not.\n\
         got:\n{}\nexpected:\n{}\n\
         If a change here is deliberate, say which datasheet line justifies it \
         and re-bless with:\n  {}",
        transcript.render(),
        common::transcript::Transcript {
            bytes: ADXL345_GOLDEN.to_vec()
        }
        .render(),
        transcript.as_literal()
    );
}

// ─── the clamp_from primitive itself ───────────────────────────────────────

/// The saturation point follows DATA_FORMAT, and it is the WHOLE eight-way
/// table, not the three combinations the transcript above happens to visit.
/// A constant `clamp_max` can satisfy at most one row of this.
#[test]
fn the_full_scale_count_follows_data_format() {
    // (DATA_FORMAT, counts per g, saturation count)
    for (fmt, per_g, full_scale) in [
        (0x00u8, 256i64, 512i64),
        (0x01, 128, 512),
        (0x02, 64, 512),
        (0x03, 32, 512),
        (0x08, 256, 512),
        (0x09, 256, 1024),
        (0x0A, 256, 2048),
        (0x0B, 256, 4096),
    ] {
        let mut dev = declarative();
        write(&mut dev, 0x31, fmt);
        // Inside the range: a plain linear conversion.
        dev.set_input("x", 1.0).expect("channel");
        assert_eq!(
            dev.register_word("DATAX0"),
            Some(per_g),
            "DATA_FORMAT 0x{fmt:02X}: 1 g must be {per_g} counts"
        );
        // Past it: saturation, positive and negative.
        dev.set_input("x", 16.0).expect("channel");
        assert_eq!(
            dev.register_word("DATAX0"),
            Some(full_scale),
            "DATA_FORMAT 0x{fmt:02X}: 16 g must saturate at {full_scale}"
        );
        dev.set_input("x", -16.0).expect("channel");
        assert_eq!(
            dev.register_word("DATAX0"),
            Some(-full_scale),
            "DATA_FORMAT 0x{fmt:02X}: -16 g must saturate at -{full_scale}"
        );
    }
}

/// Without `clamp_from` the only options were a constant window or none.
/// This is the failure a constant would produce, stated as a test: with the
/// ±2 g window baked in, the ±16 g driver below would read 512 instead of 4096.
#[test]
fn a_constant_clamp_could_not_satisfy_two_ranges_at_once() {
    let mut dev = declarative();
    dev.set_input("x", 4.0).expect("channel");
    write(&mut dev, 0x31, 0x08); // full-res, ±2 g
    assert_eq!(dev.register_word("DATAX0"), Some(512), "clipped at ±2 g");
    write(&mut dev, 0x31, 0x0B); // full-res, ±16 g
    assert_eq!(
        dev.register_word("DATAX0"),
        Some(1024),
        "4 g at 256 counts/g"
    );
}

// ─── the deliberately NEW half ─────────────────────────────────────────────

/// The conversion happens at READ time. The old model converted and clamped
/// inside `set_input`, so a driver that widened the range after posing a
/// sample kept reading the narrower range's clip forever.
#[test]
fn a_range_change_is_visible_to_the_next_read() {
    let mut dev = declarative();
    write(&mut dev, 0x31, 0x08); // full-res, ±2 g
    dev.set_input("x", 8.0).expect("channel");
    assert_eq!(dev.register_word("DATAX0"), Some(512), "clipped while ±2 g");
    write(&mut dev, 0x31, 0x0B); // widen to ±16 g — nothing else changes
    assert_eq!(
        dev.register_word("DATAX0"),
        Some(2048),
        "the silicon converts per sample, so widening the range un-clips the \
         SAME stimulus; the old model had already thrown those counts away"
    );
}

/// The whole Table 16 map is served at its datasheet reset values. The old
/// model answered six addresses and gave 0x00 for the rest.
#[test]
fn every_documented_register_reads_its_datasheet_reset() {
    let mut dev = declarative();
    for (addr, reset) in [
        (0x00u8, 0xE5u8),
        (0x2C, 0x0A),
        (0x30, 0x02), // INT_SOURCE: the power-on watermark bit
        (0x38, 0x00), // FIFO_CTL — past the old model's last register
        (0x39, 0x00), // FIFO_STATUS — the old model's unmapped read
    ] {
        assert_eq!(read_byte(&mut dev, addr), reset, "register 0x{addr:02X}");
    }
}

/// The sixteen R/W configuration registers store what a driver writes. The old
/// model discarded every one of them, so configure-then-verify could not pass.
#[test]
fn the_configuration_registers_store_what_is_written() {
    let mut dev = declarative();
    for (addr, value) in [(0x1Du8, 0x2Au8), (0x24, 0x10), (0x27, 0x77), (0x38, 0x9F)] {
        write(&mut dev, addr, value);
        assert_eq!(read_byte(&mut dev, addr), value, "register 0x{addr:02X}");
    }
}

/// DATA_READY reaches a pad, and INT_MAP routes it. The hand-written model had
/// no way to express any of this.
#[test]
fn data_ready_drives_the_mapped_interrupt_pad() {
    let mut dev = declarative();
    // Disabled at reset: the flag is raised, and both pads are driven to their
    // idle level. INT_INVERT is 0, so idle is LOW — the first firing is the
    // transition from "this pad has never been driven" to that idle level.
    dev.advance_time_us(10_000);
    assert_eq!(
        read_byte(&mut dev, 0x30) & 0x80,
        0x80,
        "INT_SOURCE.DATA_READY"
    );
    assert_eq!(
        dev.take_pin_drives(),
        vec![("INT1".to_string(), false), ("INT2".to_string(), false)],
        "an unenabled interrupt idles the pads low, it does not raise them"
    );
    dev.advance_time_us(10_000);
    assert!(
        dev.take_pin_drives().is_empty(),
        "a held level must not re-queue — the queue carries transitions"
    );

    // Enable it, mapped to INT1 (INT_MAP bit 7 = 0).
    write(&mut dev, 0x2E, 0x80);
    dev.advance_time_us(10_000);
    assert_eq!(dev.take_pin_drives(), vec![("INT1".to_string(), true)]);

    // Reading a data register clears the flag and drops the pad. INT2 is
    // already low, so only INT1 transitions. The read must COMPLETE the
    // register's word — DATAX0 is two bytes wide, and half a word is not a
    // read of it any more than half a byte is.
    dev.start();
    dev.write(0x32);
    dev.start();
    let (_lo, _hi) = (dev.read(), dev.read());
    dev.stop();
    assert_eq!(read_byte(&mut dev, 0x30) & 0x80, 0x00);
    assert_eq!(dev.take_pin_drives(), vec![("INT1".to_string(), false)]);

    // Re-map to INT2.
    write(&mut dev, 0x2F, 0x80);
    dev.advance_time_us(10_000);
    assert_eq!(dev.take_pin_drives(), vec![("INT2".to_string(), true)]);
}

// ─── helpers ───────────────────────────────────────────────────────────────

fn write(dev: &mut GenericI2cDevice, reg: u8, value: u8) {
    dev.start();
    dev.write(reg);
    dev.write(value);
    dev.stop();
}

fn read_byte(dev: &mut GenericI2cDevice, reg: u8) -> u8 {
    dev.start();
    dev.write(reg);
    dev.start();
    let b = dev.read();
    dev.stop();
    b
}
