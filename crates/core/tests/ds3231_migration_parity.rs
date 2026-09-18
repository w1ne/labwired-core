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

/// The INT/SQW pad emits a square wave while CONTROL.INTCN is 0, and is
/// released high while it is 1 (the power-on state). This is the Tier-2 half —
/// what the hand-written model had no way to express at all.
///
/// ⚠️ The rate is **RS2:RS1**, and this test asks for 1 Hz by WRITING 00 into
/// it. An earlier version of this test wrote `0x18` — INTCN clear but RS left
/// at its power-on `11` — and called the result "one hertz", which it only was
/// because nothing read the field. `timers[].period_from` reads it now, and
/// [`the_square_wave_rate_follows_the_rs_bits`] is the other half.
#[test]
fn the_int_sqw_pad_toggles_at_one_hertz_when_rs_selects_one_hertz() {
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

    // Clear INTCN and select RS = 00: the pad becomes a 1 Hz square wave.
    write(&mut dev, 0x0E, 0x00);
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

/// **`timers[].period_from` on the wire.** Datasheet Table 2: RS2:RS1 selects
/// 1 Hz / 1.024 kHz / 4.096 kHz / 8.192 kHz, and the pad TOGGLES, so each
/// setting's half period is what the timer runs at.
///
/// The counts below are transitions observed over exactly one simulated
/// second. A model with a constant `period_us` reports 2 for every row, which
/// is the failure this key removes.
#[test]
fn the_square_wave_rate_follows_the_rs_bits() {
    // (CONTROL byte with INTCN clear, the datasheet half period in µs)
    const RATES: &[(u8, u64)] = &[
        (0x00, 500_000), // RS = 00 → 1 Hz
        (0x08, 488),     // RS = 01 → 1.024 kHz
        (0x10, 122),     // RS = 10 → 4.096 kHz
        (0x18, 61),      // RS = 11 → 8.192 kHz
    ];
    for (control, half_period_us) in RATES {
        let mut dev = declarative();
        write(&mut dev, 0x0E, *control);
        assert_eq!(
            dev.timer_period_us("sqw"),
            Some(*half_period_us),
            "CONTROL {control:#04x} (RS = {}) did not reach the timer",
            (control >> 3) & 0b11
        );
        // …and the resolved number is what the pad actually runs at, not just
        // a value cached somewhere. One simulated second, walked in half-period
        // steps so no firing is lost to the bank's catch-up cap.
        let _ = dev.take_pin_drives();
        let steps = 1_000_000 / half_period_us;
        let mut transitions = 0usize;
        for _ in 0..steps {
            dev.advance_time_us(*half_period_us);
            transitions += dev.take_pin_drives().len();
        }
        assert_eq!(
            transitions, steps as usize,
            "CONTROL {control:#04x}: one pad transition per half period",
        );
    }
}

/// ⚠️ The power-on CONTROL is `0x1C`, whose RS bits are `11` — the square-wave
/// divider powers up at **8.192 kHz**, not 1 Hz. A constant `period_us` was not
/// just inflexible here, it was wrong at reset, and this asserts the reset
/// value directly rather than inferring it from a firing count.
#[test]
fn the_square_wave_divider_powers_up_at_8192_hz() {
    let dev = declarative();
    assert_eq!(dev.timer_period_us("sqw"), Some(61));
}

// ─── the clock free-runs ───────────────────────────────────────────────────

/// ⚠️ **Deliberate difference — the clock now ticks.**
///
/// The hand-written model's seven time registers were seven independent
/// storage bytes that nothing ever advanced: a sketch that read the seconds a
/// hundred times got the same number a hundred times. Phase C2 made them seven
/// windows on one instant but still could not MOVE it, because no rule action
/// could write a stimulus channel.
///
/// `set_input:` is that action, and a 1 Hz timer carries the counter chain —
/// which is what a 32.768 kHz crystal does.
#[test]
fn one_second_of_simulated_time_is_one_second_on_the_clock() {
    let mut dev = declarative();
    let seconds = |d: &mut GenericI2cDevice| read_byte(d, 0x00);
    let start = seconds(&mut dev);
    assert_eq!(start, 0x00, "the seeded instant is 12:00:00 exactly");
    for expect in 1..=5u8 {
        dev.advance_time_us(1_000_000);
        // BCD, so 1..5 read back as 0x01..0x05.
        assert_eq!(seconds(&mut dev), expect, "after {expect} s");
    }
    // …and the minute rolls when the seconds do, because it is ONE instant and
    // not seven bytes.
    dev.advance_time_us(55 * 1_000_000);
    assert_eq!(seconds(&mut dev), 0x00);
    assert_eq!(read_byte(&mut dev, 0x01), 0x01, "12:01:00");
    assert_eq!(read_byte(&mut dev, 0x02), 0x12, "the hour did not move");
}

/// `set_input:` is the exact inverse of `input()`: a rule that writes back what
/// it read changes nothing. Here the round trip runs 3600 times and the clock
/// lands exactly one hour on — a drift of one count per tick would be an hour
/// out by the end.
#[test]
fn the_clock_does_not_drift_over_an_hour() {
    let mut dev = declarative();
    for _ in 0..3600 {
        dev.advance_time_us(1_000_000);
    }
    assert_eq!(read_byte(&mut dev, 0x00), 0x00, "seconds");
    assert_eq!(read_byte(&mut dev, 0x01), 0x00, "minutes");
    assert_eq!(read_byte(&mut dev, 0x02), 0x13, "13:00:00 BCD");
}

/// ⚠️ EOSC (CONTROL bit 7) is the datasheet's "enable oscillator", active LOW.
/// Setting it stops the crystal, and a stopped crystal is a stopped clock —
/// otherwise the bit would parse, store, read back, and change nothing.
#[test]
fn setting_eosc_stops_the_clock() {
    let mut dev = declarative();
    dev.advance_time_us(2_000_000);
    assert_eq!(read_byte(&mut dev, 0x00), 0x02);
    write(&mut dev, 0x0E, 0x9C); // EOSC set, everything else at reset
    dev.advance_time_us(10_000_000);
    assert_eq!(read_byte(&mut dev, 0x00), 0x02, "the oscillator is stopped");
    write(&mut dev, 0x0E, 0x1C); // …and clearing it starts the clock again
    dev.advance_time_us(3_000_000);
    assert_eq!(read_byte(&mut dev, 0x00), 0x05);
}

/// A host or a driver posing the clock still wins: `RTClib::adjust()` writes
/// the registers, and the free-running tick carries on from THERE rather than
/// from where it would have been.
#[test]
fn a_driver_setting_the_time_reanchors_the_free_running_clock() {
    let mut dev = declarative();
    dev.advance_time_us(30_000_000);
    assert_eq!(read_byte(&mut dev, 0x00), 0x30);
    write(&mut dev, 0x00, 0x00); // "set the seconds to 0"
    assert_eq!(read_byte(&mut dev, 0x00), 0x00);
    dev.advance_time_us(4_000_000);
    assert_eq!(
        read_byte(&mut dev, 0x00),
        0x04,
        "counting on from the write"
    );
}

/// The day-of-week counter follows the date across a midnight the clock
/// reached by TICKING, not by being posed. 2026-07-22 12:00 UTC is a
/// Wednesday (4); twelve hours on is Thursday (5).
#[test]
fn the_day_of_week_counter_advances_across_a_ticked_midnight() {
    let mut dev = declarative();
    assert_eq!(read_byte(&mut dev, 0x03), 0x04, "Wednesday");
    for _ in 0..(12 * 3600) {
        dev.advance_time_us(1_000_000);
    }
    assert_eq!(read_byte(&mut dev, 0x03), 0x05, "Thursday");
    assert_eq!(read_byte(&mut dev, 0x04), 0x23, "the 23rd");
}

// ─── alarm matching ────────────────────────────────────────────────────────

/// ⚠️ **Deliberate difference — the alarms match.**
///
/// The hand-written model DROPPED every write to these registers. Phase C2
/// made them writable BCD storage but masked the A1Mx bits away, so the bit
/// that decides the alarm RATE was gone by the time anything could read it.
///
/// `encode: { value_mask: 0x7F }` splits the byte: the low seven bits go
/// through the nibble encode and bit 7 is a plain flag, stored and served
/// verbatim. A write of `0x89` — "mask set, 9 seconds" — is both halves.
#[test]
fn an_alarm_byte_carries_a_number_and_a_flag_at_once() {
    let mut dev = declarative();
    write(&mut dev, 0x07, 0x89);
    assert_eq!(
        read_byte(&mut dev, 0x07),
        0x89,
        "both halves survive the round trip"
    );
    // The whole two-digit BCD range still fits inside the mask, so the tens
    // digit is not truncated by it.
    write(&mut dev, 0x07, 0x59);
    assert_eq!(read_byte(&mut dev, 0x07), 0x59, "mask clear, 59 seconds");

    // The two halves are decoded DIFFERENTLY, which is the whole claim. A
    // nibble above 9 is not a decimal digit: the counter chain reads `0x5A` as
    // 5*10 + 10 = 60 and puts 60 back on the wire as `0x60`, while bit 7 is
    // carried through untouched. A byte stored verbatim would read back `0xDA`
    // and a byte run WHOLE through the nibble decode would lose the flag.
    write(&mut dev, 0x07, 0xDA);
    assert_eq!(
        read_byte(&mut dev, 0x07),
        0xE0,
        "the number half is decoded (0x5A ⇒ 60 ⇒ 0x60), the flag half is not"
    );
}

/// Datasheet Table 2, row 4: A1M4:A1M2 set, A1M1 clear ⇒ "alarm when seconds
/// match". Programme second 5 and the flag comes up at 12:00:05 and at no
/// other second of that minute.
#[test]
fn alarm_1_fires_when_the_seconds_match() {
    let mut dev = declarative();
    let a1f = |d: &mut GenericI2cDevice| read_byte(d, 0x0F) & 0x01 != 0;
    write(&mut dev, 0x07, 0x05); // seconds = 5, A1M1 clear
    write(&mut dev, 0x08, 0x80); // A1M2 set
    write(&mut dev, 0x09, 0x80); // A1M3 set
    write(&mut dev, 0x0A, 0x80); // A1M4 set
    for _ in 0..4 {
        dev.advance_time_us(1_000_000);
        assert!(!a1f(&mut dev), "seconds 1..4 do not match");
    }
    dev.advance_time_us(1_000_000);
    assert!(a1f(&mut dev), "12:00:05");
    // The flag is firmware-clearable and stays clear for the rest of the minute.
    write(&mut dev, 0x0F, 0x00);
    for _ in 0..10 {
        dev.advance_time_us(1_000_000);
        assert!(!a1f(&mut dev), "no second but 5 matches");
    }
}

/// Table 2, row 1: every mask bit SET ⇒ "alarm once per second".
#[test]
fn alarm_1_with_every_mask_bit_set_fires_every_second() {
    let mut dev = declarative();
    for reg in [0x07u8, 0x08, 0x09, 0x0A] {
        write(&mut dev, reg, 0x80);
    }
    for _ in 0..3 {
        write(&mut dev, 0x0F, 0x00);
        dev.advance_time_us(1_000_000);
        assert!(read_byte(&mut dev, 0x0F) & 0x01 != 0, "every second");
    }
}

/// Table 2, row 4 for alarm 1: hours, minutes AND seconds. Nothing fires until
/// all three line up.
#[test]
fn alarm_1_can_require_the_whole_time_of_day() {
    let mut dev = declarative();
    // 12:00:10, every field significant, day/date don't care.
    write(&mut dev, 0x07, 0x10);
    write(&mut dev, 0x08, 0x00);
    write(&mut dev, 0x09, 0x12);
    write(&mut dev, 0x0A, 0x80);
    let a1f = |d: &mut GenericI2cDevice| read_byte(d, 0x0F) & 0x01 != 0;
    for _ in 0..9 {
        dev.advance_time_us(1_000_000);
        assert!(!a1f(&mut dev));
    }
    dev.advance_time_us(1_000_000);
    assert!(a1f(&mut dev), "12:00:10");
    // An hour later the seconds match again but the HOUR does not.
    write(&mut dev, 0x0F, 0x00);
    tick_seconds(&mut dev, 3600);
    assert_eq!(
        read_byte(&mut dev, 0x02),
        0x13,
        "the clock really reached 13h"
    );
    assert!(!a1f(&mut dev), "13:00:10 is not 12:00:10");
}

/// ⚠️ **The DATE rate is what `reported()` unblocked.** The clock's day of
/// month is not STORED anywhere — it is computed at read time from
/// `unix_time` — so `reg(DATE)` is its reset value forever and a comparison
/// against it could never match. `reported(DATE)` is the byte the register
/// would put on the wire, which is the day the clock is really showing.
#[test]
fn alarm_1_can_match_the_day_of_month() {
    let mut dev = declarative();
    // The seeded instant is 2026-07-22 12:00:00 UTC. Ask for the 23rd at
    // 00:00:00, every field significant, DY/DT = 0 (a date).
    write(&mut dev, 0x07, 0x00);
    write(&mut dev, 0x08, 0x00);
    write(&mut dev, 0x09, 0x00);
    write(&mut dev, 0x0A, 0x23);
    let a1f = |d: &mut GenericI2cDevice| read_byte(d, 0x0F) & 0x01 != 0;
    tick_seconds(&mut dev, 11 * 3600);
    assert_eq!(read_byte(&mut dev, 0x02), 0x23, "23:00:00");
    assert_eq!(read_byte(&mut dev, 0x04), 0x22, "…still the 22nd");
    assert!(!a1f(&mut dev), "23:00:00 on the 22nd");
    tick_seconds(&mut dev, 3600);
    assert_eq!(
        read_byte(&mut dev, 0x04),
        0x23,
        "the clock rolled into the 23rd"
    );
    assert!(a1f(&mut dev), "midnight into the 23rd");
}

/// DY/DT = 1 makes the same byte a day of WEEK instead. 2026-07-23 is a
/// Thursday, which the DS3231 numbers 5 with Sunday = 1.
#[test]
fn alarm_1_can_match_the_day_of_week_instead() {
    let mut dev = declarative();
    write(&mut dev, 0x07, 0x00);
    write(&mut dev, 0x08, 0x00);
    write(&mut dev, 0x09, 0x00);
    write(&mut dev, 0x0A, 0x45); // DY/DT set, day 5
    let a1f = |d: &mut GenericI2cDevice| read_byte(d, 0x0F) & 0x01 != 0;
    tick_seconds(&mut dev, 11 * 3600);
    assert!(!a1f(&mut dev));
    tick_seconds(&mut dev, 3600);
    assert_eq!(
        read_byte(&mut dev, 0x03),
        0x05,
        "the day-of-week counter says Thursday"
    );
    assert!(a1f(&mut dev), "Thursday 00:00:00");
}

/// Alarm 2 has no seconds register: it fires at the top of a minute.
#[test]
fn alarm_2_fires_on_the_minute() {
    let mut dev = declarative();
    write(&mut dev, 0x0B, 0x01); // minute = 1, A2M2 clear
    write(&mut dev, 0x0C, 0x80); // A2M3 set
    write(&mut dev, 0x0D, 0x80); // A2M4 set
    let a2f = |d: &mut GenericI2cDevice| read_byte(d, 0x0F) & 0x02 != 0;
    dev.advance_time_us(59 * 1_000_000);
    assert!(!a2f(&mut dev), "12:00:59");
    dev.advance_time_us(1_000_000);
    assert!(a2f(&mut dev), "12:01:00");
}

/// The whole point of an alarm: it reaches the INT/SQW pad, but only through
/// the enable bit — which is what a driver sets to arm it.
#[test]
fn an_enabled_alarm_pulls_the_int_pad_low() {
    let mut dev = declarative();
    // INTCN stays set (the reset state), so the pad is the alarm output.
    for reg in [0x07u8, 0x08, 0x09, 0x0A] {
        write(&mut dev, reg, 0x80); // once per second
    }
    dev.advance_time_us(1_000_000);
    let _ = dev.take_pin_drives();
    assert!(read_byte(&mut dev, 0x0F) & 0x01 != 0, "A1F is up");
    assert_eq!(
        dev.take_pin_drives(),
        Vec::new(),
        "…but the pad does not move while A1IE is clear"
    );

    write(&mut dev, 0x0E, 0x1D); // A1IE set
    dev.advance_time_us(1_000_000);
    assert_eq!(
        dev.take_pin_drives(),
        vec![("INTSQW".to_string(), false)],
        "an enabled alarm pulls the open-drain pad low"
    );
    // Clearing the flag RELEASES the pad, and the next match — one second
    // later, because every mask bit is set — pulls it low again. Both edges
    // are real: a once-per-second alarm's pad is a pulse train, not a level,
    // and the release is visible because the `intpad` timer re-evaluates the
    // level twice a second rather than only when the flags move.
    write(&mut dev, 0x0F, 0x00);
    dev.advance_time_us(1_000_000);
    assert_eq!(
        dev.take_pin_drives(),
        vec![("INTSQW".to_string(), true), ("INTSQW".to_string(), false)],
        "released on the clear, pulled low again by the next match"
    );
}

// ─── helpers ───────────────────────────────────────────────────────────────

/// Advance the device clock ONE SECOND AT A TIME.
///
/// ⚠️ A single `advance_time_us` of many hours does NOT move this clock by many
/// hours. The timer bank replays at most `MAX_TIMER_CATCHUP` (4096) firings per
/// advance and then re-anchors every overdue deadline to `now` — an engine hang
/// guard, not DS3231 behaviour — so one 3600-second jump moves the 1 Hz counter
/// chain by about 1365 seconds, the share of that budget left after the 2 Hz
/// `intpad` timer has taken its own. A test that wants N seconds of CLOCK has
/// to give the bank N chances to tick, which is also what a real board's bus
/// traffic does.
///
/// Every assertion below that depends on the clock having reached a particular
/// hour or date checks the time registers as well, so a future change to that
/// budget fails loudly instead of quietly making an alarm test vacuous.
fn tick_seconds(dev: &mut GenericI2cDevice, seconds: u64) {
    for _ in 0..seconds {
        dev.advance_time_us(1_000_000);
    }
}

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
