// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **TMP102 register access derived from the datasheet, not from the YAML.**
//!
//! Source: Texas Instruments TMP102 datasheet SBOS397I.
//!
//! Table 6-7 "Pointer Addresses" is the whole claim this file gates:
//!
//! ```text
//!   P1 P0  REGISTER
//!   0  0   Temperature Register (Read Only)
//!   0  1   Configuration Register (Read/Write)
//!   1  0   TLOW Register (Read/Write)
//!   1  1   THIGH Register (Read/Write)
//! ```
//!
//! Three of the four pointer slots are READ/WRITE on silicon. Section 6.5.3
//! repeats it for the one that matters most — "The Configuration Register is a
//! 16-bit read/write register" — and every real driver uses it: shutdown mode
//! (SD), one-shot (OS), conversion rate (CR1:CR0), extended mode (EM), fault
//! queue (F1:F0), and the TLOW/THIGH alert limits. A driver that programs the
//! part and reads the register back to confirm must see what it wrote.
//!
//! Not every bit is firmware's. Inside the writable Configuration Register the
//! datasheet documents bits that belong to silicon, and those must NOT take a
//! firmware write:
//!
//!   * **R1:R0** (byte 1, bits 6:5 — D14:D13 of the word) — "Converter
//!     Resolution. Read-only bits. The TMP102 converter resolution is set on
//!     start-up to 11, which sets the temperature register to a 12-bit
//!     resolution." They always read 11.
//!   * **AL** (byte 2, bit 5 — D5 of the word) — "The AL bit is a read-only
//!     function. Reading the AL bit provides information about the comparator
//!     mode status." With POL = 0 it "reads as 1 until the temperature equals
//!     or exceeds T(HIGH)", so it powers up 1 and, with no alert condition
//!     modelled, stays there.
//!   * **byte 2, bits 3:0** (D3:D0) — unused, always read 0
//!     (Tables 6-10 / 6-11 show them as `0`).
//!
//! The reset word is 0x60A0 (Tables 6-10 / 6-11: byte 1 = 0110_0000,
//! byte 2 = 1010_0000), which is exactly R1:R0 = 11, CR1:CR0 = 10 (4 Hz),
//! AL = 1, everything else 0.
//!
//! TLOW/THIGH are left-justified limits: 12-bit (D15:D4) in normal mode,
//! 13-bit (D15:D3) in extended mode. D2:D0 are unused in BOTH modes and read 0.
//!
//! These expectations were written from the tables above before the descriptor
//! was touched, and the test was watched failing against the shipping YAML
//! (which declared all three registers `access: r`): CONFIG read back
//! `(0x60, 0xA0)` instead of `(0xE1, 0x70)`, TLOW `0x4B00` instead of `0x1900`.
//!
//! Deliberately NOT asserted, because the model does not claim them: the AL bit
//! tracking the comparator against the programmed limits, and the single-byte
//! ("MS byte only") update format. Both are named in `configs/devices/tmp102.yaml`.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;

fn tmp102() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("tmp102").expect("tmp102 descriptor embedded");
    GenericI2cDevice::from_yaml(yaml, 0).expect("tmp102.yaml must parse")
}

/// What a driver does to program a register: START, pointer byte, the 16-bit
/// big-endian word (MSB first), STOP — the datasheet's Write Word Format.
/// "Register bytes are sent with the most significant byte first, followed by
/// the least significant byte."
fn write_word(d: &mut dyn I2cDevice, pointer: u8, msb: u8, lsb: u8) {
    d.start();
    d.write(pointer);
    d.write(msb);
    d.write(lsb);
    d.stop();
}

/// What a driver does to read it back: START, pointer byte, repeated START,
/// two bytes MSB-first, STOP — the datasheet's Read Word Format.
fn read_word(d: &mut dyn I2cDevice, pointer: u8) -> (u8, u8) {
    d.start();
    d.write(pointer);
    d.start();
    let msb = d.read();
    let lsb = d.read();
    d.stop();
    (msb, lsb)
}

const PTR_TEMP: u8 = 0x00;
const PTR_CONFIG: u8 = 0x01;
const PTR_TLOW: u8 = 0x02;
const PTR_THIGH: u8 = 0x03;

#[test]
fn config_register_is_read_write_and_reads_back_what_the_driver_wrote() {
    let mut d = tmp102();

    // Power-on value first (Tables 6-10 / 6-11).
    assert_eq!(
        read_word(&mut d, PTR_CONFIG),
        (0x60, 0xA0),
        "CONFIG must power up at 0x60A0"
    );

    // A realistic driver configuration: one-shot in shutdown mode, 1 Hz
    // conversion rate, extended (13-bit) mode.
    //
    //   byte 1 = OS R1 R0 F1 F0 POL TM SD = 1 0 0 0 0 0 0 1 = 0x81
    //   byte 2 = CR1 CR0 AL EM  0 0 0 0   = 0 1  0  1 0000  = 0x50
    //
    // Note the driver writes R1:R0 = 00 and AL = 0. Silicon ignores both.
    write_word(&mut d, PTR_CONFIG, 0x81, 0x50);

    // Expected read-back, bit by bit:
    //   byte 1: OS = 1 (written), R1:R0 = 11 (read-only, always 11),
    //           F1:F0 = 00, POL = 0, TM = 0, SD = 1  → 1 11 00 0 0 1 = 0xE1
    //   byte 2: CR1:CR0 = 01 (written), AL = 1 (read-only, unchanged from
    //           power-up), EM = 1 (written), bits 3:0 = 0 → 0 1 1 1 0000 = 0x70
    assert_eq!(
        read_word(&mut d, PTR_CONFIG),
        (0xE1, 0x70),
        "CONFIG must read back the written SD/OS/CR/EM bits, with R1:R0 and AL \
         held by silicon"
    );

    // Writing zeros clears every firmware-owned bit and still leaves the
    // silicon-owned ones: R1:R0 = 11 (0x60) and AL = 1 (0x20).
    write_word(&mut d, PTR_CONFIG, 0x00, 0x00);
    assert_eq!(
        read_word(&mut d, PTR_CONFIG),
        (0x60, 0x20),
        "an all-zero CONFIG write must leave R1:R0 = 11 and AL = 1"
    );

    // The datasheet's unused byte-2 bits 3:0 read 0 no matter what is written.
    write_word(&mut d, PTR_CONFIG, 0x00, 0x0F);
    assert_eq!(
        read_word(&mut d, PTR_CONFIG).1 & 0x0F,
        0x00,
        "CONFIG byte 2 bits 3:0 are unused and always read 0"
    );
}

#[test]
fn tlow_and_thigh_are_read_write_limit_registers() {
    let mut d = tmp102();

    // Power-on limits: TLOW = 75 °C (0x4B00), THIGH = 80 °C (0x5000).
    assert_eq!(read_word(&mut d, PTR_TLOW), (0x4B, 0x00));
    assert_eq!(read_word(&mut d, PTR_THIGH), (0x50, 0x00));

    // A thermostat driver narrows the window: TLOW = 25 °C, THIGH = 30 °C.
    // Normal mode is 12-bit left-justified, 1 LSB = 0.0625 °C:
    //   25.0 / 0.0625 = 400 = 0x190 → 0x1900
    //   30.0 / 0.0625 = 480 = 0x1E0 → 0x1E00
    write_word(&mut d, PTR_TLOW, 0x19, 0x00);
    write_word(&mut d, PTR_THIGH, 0x1E, 0x00);
    assert_eq!(
        read_word(&mut d, PTR_TLOW),
        (0x19, 0x00),
        "TLOW is Read/Write (Table 6-7)"
    );
    assert_eq!(
        read_word(&mut d, PTR_THIGH),
        (0x1E, 0x00),
        "THIGH is Read/Write (Table 6-7)"
    );

    // The limits are left-justified: D2:D0 are unused in normal AND extended
    // mode and read back 0.
    write_word(&mut d, PTR_THIGH, 0x1E, 0x07);
    assert_eq!(
        read_word(&mut d, PTR_THIGH),
        (0x1E, 0x00),
        "THIGH bits 2:0 are unused and always read 0"
    );
}

#[test]
fn temperature_register_stays_read_only() {
    let mut d = tmp102();
    // Table 6-7: pointer 00 is "Temperature Register (Read Only)". A write to
    // it must be absorbed, leaving the conversion result intact.
    write_word(&mut d, PTR_TEMP, 0xDE, 0xAD);
    assert_eq!(
        read_word(&mut d, PTR_TEMP),
        (0x19, 0x00),
        "TEMP is Read Only — a write must not land"
    );
}

#[test]
fn configuring_the_part_does_not_disturb_the_temperature_reading() {
    // The whole point of the fix: a driver may configure the part and still get
    // a real conversion result out of pointer 0x00. (The model's TEMP register
    // self-drifts +0.5 °C per full read, so the first read after configuration
    // is still the power-on 25.0 °C.)
    let mut d = tmp102();
    write_word(&mut d, PTR_CONFIG, 0x81, 0x50);
    write_word(&mut d, PTR_TLOW, 0x19, 0x00);
    write_word(&mut d, PTR_THIGH, 0x1E, 0x00);
    assert_eq!(read_word(&mut d, PTR_TEMP), (0x19, 0x00));
    assert_eq!(read_word(&mut d, PTR_TEMP), (0x19, 0x80));
}
