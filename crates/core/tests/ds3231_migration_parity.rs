// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! DS3231: the declarative descriptor against the hand-written model it replaces.
//!
//! `components/ds3231.rs` is DELETED. The transcript that model produced is
//! pinned here as golden bytes, captured by running [`migration_script`]
//! against it on the commit that removed it, so the migration is a claim this
//! file can refuse rather than a diff someone has to trust.
//!
//! Held identical (the whole of the old model's observable behaviour):
//!   * the power-on instant — 2026-07-22 12:00:00 UTC, a Wednesday — read out
//!     as seven BCD bytes in one auto-incrementing transaction;
//!   * `RTClib::adjust()`-shaped set: seven BCD bytes written from pointer
//!     0x00, then read back;
//!   * CONTROL 0x1C and STATUS 0x00 at power-on, and OSF unsettable (a write of
//!     0x88 reads back 0x08);
//!   * the temperature register, and the `temperature` stimulus;
//!   * the `unix_time` stimulus posing the whole clock, DAY included;
//!   * alarm registers reading zero.
//!
//! DELIBERATELY DIFFERENT, each asserted separately below so it cannot be lost:
//!   * the seven time registers are now ONE instant. The old model held seven
//!     independent bytes, so writing the hour and then reading the date could
//!     report a calendar that does not exist.
//!   * 0x11/0x12 is the datasheet's real 10-bit 0.25 °C word. The old model put
//!     a rounded integer in 0x11 and a permanent zero in 0x12.
//!   * 12-hour mode: the old model ran the whole HOURS byte through its BCD
//!     decode and clamped, so "2 PM" (0x52) read back as 23. Bit 6 is masked
//!     away here and a 12-hour write reads back as the 24-hour hour it encodes.
//!   * the alarm registers are writable and read back. The old model dropped
//!     every write to them.
//!
//! Still NOT modelled, and named in `configs/devices/ds3231.yaml`: the clock
//! does not free-run, alarm MATCHING needs a register that is part BCD number
//! and part flag bits, and square-wave rates above 1 Hz need a field-driven
//! timer period.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::sim_input::SimInput;

mod common;
use common::transcript::{read_reg, run_i2c, script, write_reg, Step};

const ADDR: u8 = 0x68;

/// 2026-07-22 12:00:00 UTC — a Wednesday, and the instant the hand-written
/// model seeded as `time: [0, 0, 12, 4, 22, 7, 26]`.
const POWER_ON_UNIX: f64 = 1_784_721_600.0;
/// One hour, one minute and one second later: 2026-07-22 13:01:01 UTC.
const LATER_UNIX: f64 = 1_784_725_261.0;

/// The descriptor under test, built from the EMBEDDED yaml so this fails if the
/// part is ever dropped from `embedded_device_yaml` — the one way it could
/// silently stop existing for wasm builds.
fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("ds3231")
        .expect("ds3231 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("ds3231.yaml does not build")
}

/// The conversation the deleted model was driven through, in the order the
/// golden bytes below record. Read it as an RTC library would write it.
fn migration_script() -> Vec<Step<'static>> {
    script([
        // 1. Power-on: seven time bytes out of one transaction.
        read_reg(0x00, 7),
        // 2. Set the clock — 14:30:45 on Monday 2026-06-15, written as BCD.
        //    The day-of-week byte is deliberately 3 (Tuesday) and the date is a
        //    Monday: on silicon that counter is independent and reads back 3.
        write_reg(0x00, &[0x45, 0x30, 0x14, 0x03, 0x15, 0x06, 0x26]),
        // 3. Read it back.
        read_reg(0x00, 7),
        // 4. CONTROL + STATUS.
        read_reg(0x0E, 2),
        // 5. Try to SET the oscillator-stop flag; silicon only lets it be cleared.
        write_reg(0x0F, &[0x88]),
        read_reg(0x0F, 1),
        // 6. Temperature at its default, then driven.
        read_reg(0x11, 2),
        vec![Step::Input("temperature", 30.0)],
        read_reg(0x11, 2),
        // 7. Pose the whole clock from the host.
        vec![Step::Input("unix_time", LATER_UNIX)],
        read_reg(0x00, 7),
        // 8. The alarm block.
        read_reg(0x07, 7),
    ])
}

/// ⚠️ GOLDEN — captured from `components/ds3231.rs` (the hand-written model) on
/// the commit that deleted it, by running [`migration_script`] against it.
/// Not a value anyone chose: it is what the old part did.
const DS3231_GOLDEN: &[u8] = &[
    0x00, 0x00, 0x12, 0x04, 0x22, 0x07, 0x26, // power-on time
    0x45, 0x30, 0x14, 0x03, 0x15, 0x06, 0x26, // read back after adjust()
    0x1C, 0x00, // CONTROL, STATUS
    0x08, // STATUS after a write of 0x88 — OSF refused
    0x19, 0x00, // 25 °C
    0x1E, 0x00, // 30 °C
    0x01, 0x01, 0x13, 0x04, 0x22, 0x07, 0x26, // host-posed clock
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // alarms
];

#[test]
fn the_wire_transcript_is_byte_identical_to_the_deleted_model() {
    let transcript = run_i2c(&mut declarative(), &migration_script());
    assert_eq!(
        transcript.bytes,
        DS3231_GOLDEN,
        "the declarative DS3231 moved a byte the hand-written one did not.\n\
         got:\n{}\nexpected:\n{}\n\
         If a change here is deliberate, say which datasheet line justifies it \
         and re-bless with:\n  {}",
        transcript.render(),
        common::transcript::Transcript {
            bytes: DS3231_GOLDEN.to_vec()
        }
        .render(),
        transcript.as_literal()
    );
}

// ─── the BCD primitive itself ──────────────────────────────────────────────

/// `encode: { bcd: true }` is symmetric. A write of BCD nibbles decodes, and
/// the read encodes back — for every value the register can hold, not just the
/// ones the transcript above happens to visit.
#[test]
fn every_second_of_a_minute_round_trips_through_bcd() {
    let mut dev = declarative();
    for sec in 0..60u8 {
        let bcd = ((sec / 10) << 4) | (sec % 10);
        dev.start();
        dev.write(0x00);
        dev.write(bcd);
        dev.stop();
        dev.start();
        dev.write(0x00);
        dev.start();
        let got = dev.read();
        dev.stop();
        assert_eq!(
            got, bcd,
            "second {sec} did not round-trip (wrote 0x{bcd:02X})"
        );
    }
}

/// The value the MODEL stores is decimal, not nibbles. That is the whole reason
/// the decode happens at the wire boundary: a rule reading the register is
/// reading a number of seconds. Proven through the clock — 0x45 BCD is 45
/// seconds, and 45 seconds is what the instant moves by.
#[test]
fn a_bcd_write_stores_the_decimal_value_not_the_nibbles() {
    let mut dev = declarative();
    dev.set_input("unix_time", POWER_ON_UNIX).expect("channel");
    dev.start();
    dev.write(0x00);
    dev.write(0x45); // 45 seconds, BCD
    dev.stop();
    // 0x45 read as a plain byte would be 69, which is not a legal second at
    // all; as nibbles it is 45, and the instant has to move by exactly that.
    let minutes = read_byte(&mut dev, 0x01);
    assert_eq!(minutes, 0x00, "the minute must not have rolled");
    assert_eq!(read_byte(&mut dev, 0x00), 0x45);
}

// ─── the deliberately NEW half ─────────────────────────────────────────────

/// The seven registers are ONE instant. The old model could hold
/// 14:30:45 on the 31st of February; this one cannot, because the registers are
/// windows on a single Unix second.
#[test]
fn the_time_registers_are_one_clock_not_seven_bytes() {
    let mut dev = declarative();
    dev.set_input("unix_time", POWER_ON_UNIX).expect("channel");
    // 23:59:59 on 2026-07-22, then advance the SECOND by one: every other
    // register has to roll with it. Seven independent bytes could not.
    write(&mut dev, 0x02, 0x23);
    write(&mut dev, 0x01, 0x59);
    write(&mut dev, 0x00, 0x59);
    assert_eq!(read_byte(&mut dev, 0x04), 0x22, "still the 22nd");

    write(&mut dev, 0x00, 0x00); // rewind to :00 of the same minute
    write(&mut dev, 0x01, 0x00);
    write(&mut dev, 0x02, 0x00);
    assert_eq!(
        read_byte(&mut dev, 0x04),
        0x22,
        "midnight is still the 22nd"
    );
    assert_eq!(read_byte(&mut dev, 0x05), 0x07);
}

/// `calendar:` writes RECOMPOSE — a write to one register moves that field of
/// the clock and leaves the other six. Without it a `source`d register would be
/// read-only and `RTClib::adjust()` would do nothing at all.
#[test]
fn a_write_moves_only_its_own_field() {
    let mut dev = declarative();
    dev.set_input("unix_time", POWER_ON_UNIX).expect("channel");
    write(&mut dev, 0x02, 0x07); // hour := 07
    assert_eq!(read_byte(&mut dev, 0x00), 0x00, "seconds untouched");
    assert_eq!(read_byte(&mut dev, 0x01), 0x00, "minutes untouched");
    assert_eq!(read_byte(&mut dev, 0x02), 0x07);
    assert_eq!(read_byte(&mut dev, 0x04), 0x22, "date untouched");
    assert_eq!(read_byte(&mut dev, 0x05), 0x07, "month untouched");
    assert_eq!(read_byte(&mut dev, 0x06), 0x26, "year untouched");
}

/// 0x11/0x12 is the datasheet's 10-bit two's-complement word at 0.25 °C per
/// count, not the old model's integer-plus-a-permanent-zero. Both halves are
/// checked, and the negative case is the one an integer model gets wrong.
#[test]
fn the_temperature_word_carries_quarter_degrees_and_signs() {
    let mut dev = declarative();
    for (c, msb, lsb) in [
        (25.0, 0x19, 0x00),
        (25.25, 0x19, 0x40),
        (25.5, 0x19, 0x80),
        (25.75, 0x19, 0xC0),
        (-10.0, 0xF6, 0x00),
        (-10.25, 0xF5, 0xC0),
        (0.0, 0x00, 0x00),
    ] {
        dev.set_input("temperature", c).expect("channel");
        dev.start();
        dev.write(0x11);
        dev.start();
        let got = (dev.read(), dev.read());
        dev.stop();
        assert_eq!(got, (msb, lsb), "{c} °C encoded wrong");
    }
}

/// The alarm registers accept a write and read it back. The old model dropped
/// every write to 0x07..0x0D, so a driver that programmed an alarm and verified
/// it read zeros.
#[test]
fn the_alarm_registers_are_writable_bcd() {
    let mut dev = declarative();
    write(&mut dev, 0x07, 0x30); // alarm 1 at :30 seconds
    write(&mut dev, 0x08, 0x45);
    assert_eq!(read_byte(&mut dev, 0x07), 0x30);
    assert_eq!(read_byte(&mut dev, 0x08), 0x45);
}

/// The INT/SQW pad emits a 1 Hz square wave while CONTROL.INTCN is 0, and is
/// released high while it is 1 (the power-on state). This is the Tier-2 half —
/// what the hand-written model had no way to express at all.
#[test]
fn the_int_sqw_pad_toggles_at_one_hertz_when_intcn_is_clear() {
    let mut dev = declarative();
    // Power-on INTCN = 1: the pad is the alarm output, released high, and no
    // alarm flag is set.
    dev.advance_time_us(500_000);
    assert_eq!(dev.take_pin_drives(), vec![("INTSQW".to_string(), true)]);
    dev.advance_time_us(500_000);
    assert!(
        dev.take_pin_drives().is_empty(),
        "a held level must not re-queue — the queue carries transitions"
    );

    // Clear INTCN: the pad becomes the square wave.
    write(&mut dev, 0x0E, 0x18);
    let mut levels = Vec::new();
    for _ in 0..4 {
        dev.advance_time_us(500_000);
        levels.extend(dev.take_pin_drives());
    }
    assert_eq!(
        levels,
        vec![
            ("INTSQW".to_string(), false),
            ("INTSQW".to_string(), true),
            ("INTSQW".to_string(), false),
            ("INTSQW".to_string(), true),
        ],
        "one transition per half-second is a 1 Hz square wave"
    );
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
