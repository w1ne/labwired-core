// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! VL53L0X: the declarative descriptor against the hand-written model it replaces.
//!
//! The deleted `components/vl53l0x.rs` named three engine gaps that had to close
//! before it could become data — byte-wise pointer auto-increment, write-1-to-
//! clear, and an honest conversion time. This file is the evidence that they
//! did, and it is deliberately NOT a blanket byte-parity assertion, because one
//! behaviour is *meant* to change.
//!
//! Held identical (the transcripts a real driver produces):
//!   * identification reads — Adafruit_VL53L0X and the ST API both refuse to
//!     initialize unless the model id reads 0xEE;
//!   * the 16-bit big-endian range at 0x1E/0x1F;
//!   * ST's 12-byte block read from RESULT_RANGE_STATUS (0x14), including the
//!     undecoded addresses in the middle of it, which is the case that could
//!     not be expressed at all before auto-increment.
//!
//! Deliberately DIFFERENT, and asserted as such so the change cannot be made
//! silently in either direction:
//!   * the old model set its ready flag on the first start and never cleared
//!     it, with no conversion time. The descriptor follows ST's default 33 ms
//!     timing budget and clears on acknowledge.
//!
//! The old model's 0x1C/0x1D range alias is not carried over. Its own comment
//! called it a guess ("some continuous reads"), not a datasheet register.
//!
//! ── A second deliberate change: the interrupt-status VALUE ─────────────────
//!
//! RESULT_INTERRUPT_STATUS used to read 0x07 (all three low bits) when a range
//! completed. Bits 2:0 are not three independent flags — they report WHICH
//! source asserted, and the source every polling driver configures through
//! SYSTEM_INTERRUPT_CONFIG_GPIO (0x0A) is 0x04, "new sample ready".
//! `VL53L0X_GetMeasurementDataReady` compares the masked status for EQUALITY
//! with that configured functionality, so 0x07 read as "not ready" forever and
//! every vendor measurement timed out. The tests below therefore expect 0x04.
//! Drivers that only test `status & 0x07 != 0` — the datasheet one-shot idiom,
//! including the shipped C3 example — are unaffected either way.
//!
//! The vendor-initialisation surface that grew out of this (register banks, the
//! factory NVM port, the reference-SPAD map) is locked in
//! `vl53l0x_vendor_init.rs`; this file stays the migration record.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;

const ADDR: u8 = 0x29;
const DISTANCE_MM: u16 = 350;

/// The descriptor under test, seeded like the old model's `set_distance_mm`.
///
/// Built from the EMBEDDED yaml rather than a local copy, so this fails if the
/// descriptor is ever dropped from `embedded_device_yaml` — the one way the
/// part could silently stop existing for wasm builds.
fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("vl53l0x")
        .expect("vl53l0x descriptor is not embedded — check embedded_device_yaml");
    let mut dev = GenericI2cDevice::from_yaml(yaml, ADDR).expect("vl53l0x.yaml does not build");
    dev.seed_input("distance", f64::from(DISTANCE_MM));
    dev
}

/// Point at `reg` and pull `n` bytes, the way a driver's read_multi does.
fn read_block(dev: &mut GenericI2cDevice, reg: u8, n: usize) -> Vec<u8> {
    dev.stop();
    dev.start();
    dev.write(reg);
    dev.start(); // repeated START — the read phase
    let out = (0..n).map(|_| dev.read()).collect();
    dev.stop();
    out
}

/// Establish that this bus has an honest microsecond source.
///
/// `data_ready` deliberately degrades to always-ready on families with no
/// absolute-µs counter, so a device that has never seen time advance reports
/// ready immediately — the same constant these parts had before the primitive
/// existed. A real bus drives `advance_time_us` every slice; a unit test has to
/// say so explicitly or it is measuring the holdout path, not the timed one.
fn with_clock(dev: &mut GenericI2cDevice) {
    dev.advance_time_us(1);
}

fn write_reg(dev: &mut GenericI2cDevice, reg: u8, value: u8) {
    dev.stop();
    dev.start();
    dev.write(reg);
    dev.write(value);
    dev.stop();
}

#[test]
fn identification_matches_the_model_it_replaces() {
    // Byte-for-byte what the hand-written `read_register` returned for these.
    let mut dev = declarative();
    assert_eq!(read_block(&mut dev, 0xC0, 1), vec![0xEE], "model id");
    assert_eq!(read_block(&mut dev, 0xC1, 1), vec![0xAA]);
    assert_eq!(read_block(&mut dev, 0xC2, 1), vec![0x10], "revision id");
}

#[test]
fn the_range_reads_big_endian_millimetres() {
    // 350 mm = 0x015E, high byte first — the old model's own assertion.
    let mut dev = declarative();
    assert_eq!(read_block(&mut dev, 0x1E, 2), vec![0x01, 0x5E]);
}

#[test]
fn identification_reads_walk_the_pointer_like_the_old_model() {
    // The hand model incremented `current_register` on EVERY byte read, so a
    // 3-byte read from 0xC0 returned the three identification bytes in order.
    // Before auto-increment the declarative engine returned 0xEE then 0xFF
    // forever. This is the regression that gap would cause.
    let mut dev = declarative();
    assert_eq!(read_block(&mut dev, 0xC0, 3), vec![0xEE, 0xAA, 0x10]);
}

#[test]
fn sts_twelve_byte_block_read_reaches_the_range_bytes() {
    // THE case the migration existed for. ST's API reads 12 bytes from
    // RESULT_RANGE_STATUS (0x14); the range lives at 0x1E/0x1F, which is
    // offsets 10 and 11 into that block. Addresses 0x15..0x1D are not decoded
    // by this model and read as the unmapped byte, exactly as the hand model's
    // `_ => 0` arm did.
    let mut dev = declarative();
    let block = read_block(&mut dev, 0x14, 12);

    assert_eq!(block.len(), 12);
    assert_eq!(block[0], 0x00, "RESULT_RANGE_STATUS: no error");
    assert_eq!(
        &block[1..10],
        &[0u8; 9],
        "undecoded addresses must read 0, as the model they replace did"
    );
    assert_eq!(
        (u16::from(block[10]) << 8) | u16::from(block[11]),
        DISTANCE_MM,
        "the range must land at offsets 10..11 of the block"
    );
}

#[test]
fn a_range_is_not_ready_until_the_timing_budget_elapses() {
    // THE deliberate fidelity change. The old model's flag was set from the
    // first start onward with no elapsed time at all.
    let mut dev = declarative();
    with_clock(&mut dev);
    write_reg(&mut dev, 0x00, 0x01); // SYSRANGE_START

    assert_eq!(
        read_block(&mut dev, 0x13, 1),
        vec![0x00],
        "interrupt status must be clear before the conversion completes"
    );

    dev.advance_time_us(33_000); // ST's default timing budget
    assert_eq!(
        read_block(&mut dev, 0x13, 1),
        vec![0x04],
        "bits 2:0 must report the asserted source once the range is done"
    );
}

#[test]
fn reading_the_range_does_not_acknowledge_the_interrupt() {
    // The VL53L0X clears on a WRITE to SYSTEM_INTERRUPT_CLEAR, not on a result
    // read. Modelling it as clear-on-read would let a driver that never
    // acknowledges appear to work — which is the whole point of a faithful twin.
    let mut dev = declarative();
    with_clock(&mut dev);
    write_reg(&mut dev, 0x00, 0x01);
    dev.advance_time_us(33_000);

    let _ = read_block(&mut dev, 0x1E, 2);
    assert_eq!(
        read_block(&mut dev, 0x13, 1),
        vec![0x04],
        "reading the range must NOT clear the interrupt"
    );

    write_reg(&mut dev, 0x0B, 0x01); // SYSTEM_INTERRUPT_CLEAR
    assert_eq!(
        read_block(&mut dev, 0x13, 1),
        vec![0x00],
        "writing SYSTEM_INTERRUPT_CLEAR must clear it"
    );
}

#[test]
fn a_second_range_can_be_started_after_acknowledging() {
    // The old model latched `ranging` forever, so this sequence was untestable.
    // A driver looping range → wait → read → acknowledge must keep working.
    let mut dev = declarative();
    with_clock(&mut dev);
    for round in 0..3 {
        write_reg(&mut dev, 0x00, 0x01);
        assert_eq!(
            read_block(&mut dev, 0x13, 1),
            vec![0x00],
            "round {round}: flag must start clear"
        );
        dev.advance_time_us(33_000);
        assert_eq!(read_block(&mut dev, 0x13, 1), vec![0x04], "round {round}");
        assert_eq!(read_block(&mut dev, 0x1E, 2), vec![0x01, 0x5E]);
        write_reg(&mut dev, 0x0B, 0x01);
    }
}

#[test]
fn the_shipped_c3_marketplace_example_still_reads_the_same_bytes() {
    // `examples/marketplace-arduino-c3/src/main.ino` is the one shipped sketch
    // that drives this part, and it does two things worth pinning.
    //
    // First it reads the range as TWO separately-pointed single-byte reads, at
    // 0x1E and then 0x1F. 0x1F is the second byte of the 2-byte
    // RESULT_RANGE_VAL — not a register start — so it only resolves because the
    // auto-increment path looks for the register COVERING an address. Without
    // that, a pointed read of 0x1F matches nothing and returns a zero word,
    // which would quietly halve the reported distance.
    //
    // Second, it writes SYSRANGE_START once in setup() and then never reads
    // RESULT_INTERRUPT_STATUS and never acknowledges. That is why the honest
    // 33 ms timing this migration introduces does NOT break it: the range
    // register is readable whenever it is asked for, and only the STATUS bit is
    // gated. A sketch that polls is now timed truthfully; a sketch that does
    // not poll is unaffected.
    let mut dev = declarative();
    with_clock(&mut dev);
    write_reg(&mut dev, 0x00, 0x01); // setup(): start ranging, once

    for _ in 0..3 {
        // loop(): id, then the two range bytes, each its own pointed read.
        assert_eq!(read_block(&mut dev, 0xC0, 1), vec![0xEE]);
        let hi = read_block(&mut dev, 0x1E, 1)[0];
        let lo = read_block(&mut dev, 0x1F, 1)[0];
        assert_eq!(
            (u16::from(hi) << 8) | u16::from(lo),
            DISTANCE_MM,
            "the sketch's two-pointed-reads pattern must still yield the range"
        );
    }
}

#[test]
fn the_distance_channel_drives_the_range() {
    // SimInput parity: the old model clamped to 0..2000 and rounded.
    use labwired_core::sim_input::SimInput;
    let mut dev = declarative();
    dev.set_input("distance", 1234.0).expect("distance channel");
    assert_eq!(read_block(&mut dev, 0x1E, 2), vec![0x04, 0xD2]);
}
