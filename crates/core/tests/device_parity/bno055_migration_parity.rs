// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! BNO055: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! The deleted `components/bno055.rs` is reproduced below as [`legacy`] — the
//! wire behaviour only — and both models are driven through the SAME I²C
//! script. Transcripts that must be identical are asserted equal byte for
//! byte; the one that must differ is asserted as a difference, by name.
//!
//! Held identical:
//!   * the four identity bytes and the three status constants;
//!   * the six Euler bytes as one auto-incrementing burst from 0x1A —
//!     16-bit signed little-endian, 16 counts per degree — swept over every
//!     tenth of a degree of all three declared channel ranges
//!     (`euler_words_match_across_the_whole_channel_range`), including the
//!     i16 saturation at the ends;
//!   * every undeclared address answering 0x00, and the pointer wrapping
//!     0xFF → 0x00 rather than saturating;
//!   * writes: UNIT_SEL / OPR_MODE / PWR_MODE store and read back, the
//!     read-only identity and status registers drop the write, SYS_TRIGGER
//!     drops it (the soft-reset bit included), and a burst write walks the
//!     pointer byte by byte;
//!   * PAGE_ID = 1 blanking the WHOLE map, PAGE_ID itself included, and
//!     PAGE_ID = 0 bringing it back untouched.
//!
//! Deliberately DIFFERENT (its own test below):
//!   * the bank select's MASK. The hand model stored `PAGE_ID & 0x01`, so a
//!     write of 0x02 selected page 0; the engine's bank select takes the byte
//!     as written, so 0x02 selects bank 2 — which has no registers and reads
//!     0x00 everywhere. The datasheet defines PAGE_ID as a 1-bit field with
//!     values 0 and 1 only (§4.2.1), so the two can only disagree about a
//!     value silicon does not define and no driver sends.
//!
//! Scripts are driven through the shared harness (`tests/common/transcript.rs`),
//! the same one the byte-parity ratchet uses.

use crate::common;
use common::transcript::{read_reg, run_i2c, script, write_reg, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;

const ADDR: u8 = 0x28;

const REG_CHIP_ID: u8 = 0x00;
const REG_PAGE_ID: u8 = 0x07;
const REG_EUL_H_LSB: u8 = 0x1A;
const REG_CALIB_STAT: u8 = 0x35;
const REG_UNIT_SEL: u8 = 0x3B;
const REG_OPR_MODE: u8 = 0x3D;
const REG_PWR_MODE: u8 = 0x3E;
const REG_SYS_TRIGGER: u8 = 0x3F;

// ─── the model this descriptor replaces ────────────────────────────────────

/// `crates/core/src/peripherals/components/bno055.rs` at `origin/main`, wire
/// behaviour only. This is the ONLY place the old behaviour survives.
mod legacy {
    use labwired_core::peripherals::i2c::I2cDevice;

    const REG_CHIP_ID: u8 = 0x00;
    const REG_ACC_ID: u8 = 0x01;
    const REG_MAG_ID: u8 = 0x02;
    const REG_GYR_ID: u8 = 0x03;
    const REG_PAGE_ID: u8 = 0x07;
    const REG_EUL_H_LSB: u8 = 0x1A;
    const REG_EUL_H_MSB: u8 = 0x1B;
    const REG_EUL_R_LSB: u8 = 0x1C;
    const REG_EUL_R_MSB: u8 = 0x1D;
    const REG_EUL_P_LSB: u8 = 0x1E;
    const REG_EUL_P_MSB: u8 = 0x1F;
    const REG_UNIT_SEL: u8 = 0x3B;
    const REG_OPR_MODE: u8 = 0x3D;
    const REG_PWR_MODE: u8 = 0x3E;
    const REG_SYS_TRIGGER: u8 = 0x3F;
    const REG_SYS_STATUS: u8 = 0x39;
    const REG_SYS_ERR: u8 = 0x3A;
    const REG_CALIB_STAT: u8 = 0x35;

    const CHIP_ID: u8 = 0xA0;
    const ACC_ID: u8 = 0xFB;
    const MAG_ID: u8 = 0x32;
    const GYR_ID: u8 = 0x0F;

    pub struct Bno055 {
        address: u8,
        current_register: u8,
        register_address_written: bool,
        page: u8,
        opr_mode: u8,
        unit_sel: u8,
        pwr_mode: u8,
        heading: f64,
        roll: f64,
        pitch: f64,
    }

    impl Bno055 {
        pub fn new(address: u8) -> Self {
            Self {
                address,
                current_register: 0,
                register_address_written: false,
                page: 0,
                opr_mode: 0x00,
                unit_sel: 0x00,
                pwr_mode: 0x00,
                heading: 0.0,
                roll: 0.0,
                pitch: 0.0,
            }
        }

        /// The old `SimInput::set_input`, verbatim.
        pub fn set_channel(&mut self, key: &str, value: f64) {
            match key {
                "heading" => self.heading = value,
                "roll" => self.roll = value,
                "pitch" => self.pitch = value,
                other => panic!("no such channel: {other}"),
            }
        }

        fn eul_i16(deg: f64) -> i16 {
            (deg * 16.0).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
        }

        fn read_register(&self, reg: u8) -> u8 {
            if self.page != 0 {
                return 0;
            }
            match reg {
                REG_CHIP_ID => CHIP_ID,
                REG_ACC_ID => ACC_ID,
                REG_MAG_ID => MAG_ID,
                REG_GYR_ID => GYR_ID,
                REG_PAGE_ID => self.page,
                REG_EUL_H_LSB => (Self::eul_i16(self.heading) as u16 & 0xFF) as u8,
                REG_EUL_H_MSB => ((Self::eul_i16(self.heading) as u16) >> 8) as u8,
                REG_EUL_R_LSB => (Self::eul_i16(self.roll) as u16 & 0xFF) as u8,
                REG_EUL_R_MSB => ((Self::eul_i16(self.roll) as u16) >> 8) as u8,
                REG_EUL_P_LSB => (Self::eul_i16(self.pitch) as u16 & 0xFF) as u8,
                REG_EUL_P_MSB => ((Self::eul_i16(self.pitch) as u16) >> 8) as u8,
                REG_UNIT_SEL => self.unit_sel,
                REG_OPR_MODE => self.opr_mode,
                REG_PWR_MODE => self.pwr_mode,
                REG_SYS_STATUS => 0x05,
                REG_SYS_ERR => 0x00,
                REG_CALIB_STAT => 0xFF,
                _ => 0,
            }
        }

        fn write_register(&mut self, reg: u8, value: u8) {
            match reg {
                REG_PAGE_ID => self.page = value & 0x01,
                REG_OPR_MODE => self.opr_mode = value,
                REG_UNIT_SEL => self.unit_sel = value,
                REG_PWR_MODE => self.pwr_mode = value,
                REG_SYS_TRIGGER => {}
                _ => {}
            }
        }
    }

    impl I2cDevice for Bno055 {
        fn address(&self) -> u8 {
            self.address
        }

        fn read(&mut self) -> u8 {
            let value = self.read_register(self.current_register);
            self.current_register = self.current_register.wrapping_add(1);
            value
        }

        fn write(&mut self, data: u8) {
            if !self.register_address_written {
                self.current_register = data;
                self.register_address_written = true;
            } else {
                self.write_register(self.current_register, data);
                self.current_register = self.current_register.wrapping_add(1);
            }
        }

        fn stop(&mut self) {
            self.register_address_written = false;
        }
    }
}

// ─── the two models under test ─────────────────────────────────────────────

fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("bno055")
        .expect("bno055 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("bno055.yaml does not build")
}

/// Run one script against BOTH models and return `(legacy, declarative)`.
///
/// ⚠️ The same script object, not two scripts that look alike: a divergence
/// that lived in the script rather than in the models would compare equal and
/// prove nothing.
fn both(steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    both_at(&[], steps)
}

/// The same, with the three Euler channels driven first. The legacy model's
/// `SimInput` impl went with the file, so its channels are driven through the
/// setter that impl called.
fn both_at(channels: &[(&str, f64)], steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::Bno055::new(ADDR);
    let mut new = declarative();
    for (key, value) in channels {
        old.set_channel(key, *value);
        new.seed_input(key, *value);
    }
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

// ─── identical: identity and the status constants ──────────────────────────

#[test]
fn identity_and_status_bytes_are_identical() {
    // Adafruit_BNO055::begin() polls CHIP_ID and gives up after 850 ms if it
    // never reads 0xA0, so the first four bytes are the whole of "does this
    // sketch start at all".
    let s = script([
        read_reg(REG_CHIP_ID, 4),
        read_reg(REG_CALIB_STAT, 1),
        read_reg(0x39, 2),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "identity/status transcript diverged");
    assert_eq!(
        old,
        vec![0xA0, 0xFB, 0x32, 0x0F, 0xFF, 0x05, 0x00],
        "CHIP_ID/ACC_ID/MAG_ID/GYR_ID, CALIB_STAT, SYS_STATUS, SYS_ERR"
    );
}

// ─── identical: the Euler burst ────────────────────────────────────────────

/// Decode the six-byte Euler burst the way `Adafruit_BNO055::getVector()` does.
fn decode_euler(b: &[u8]) -> (i16, i16, i16) {
    (
        i16::from_le_bytes([b[0], b[1]]),
        i16::from_le_bytes([b[2], b[3]]),
        i16::from_le_bytes([b[4], b[5]]),
    )
}

#[test]
fn euler_burst_is_identical_and_carries_the_stimulus() {
    let s = read_reg(REG_EUL_H_LSB, 6);
    let (old, new) = both_at(&[("heading", 90.0), ("roll", -10.5), ("pitch", 30.25)], &s);
    assert_eq!(old, new, "Euler burst diverged");
    let (h, r, p) = decode_euler(&old);
    assert_eq!(h, 90 * 16, "heading is degrees x 16");
    assert_eq!(r, -168, "-10.5 deg x 16");
    assert_eq!(p, 484, "30.25 deg x 16");
}

/// The sweep. `encode: { scale: 16.0 }` on a `signed: true` two-byte register
/// has to reproduce `(deg * 16).round().clamp(i16)` exactly — a different
/// rounding rule or a missing saturation would show at a tenth of a degree, not
/// at the whole degrees a hand-written example uses.
#[test]
fn euler_words_match_across_the_whole_channel_range() {
    let s = read_reg(REG_EUL_H_LSB, 6);
    let mut checked = 0usize;
    // heading 0..360, roll and pitch -180..180 — the declared ranges. Stepped a
    // tenth of a degree, which is where a half-count lands (0.1 x 16 = 1.6).
    for tenth in 0..=3600i32 {
        let heading = f64::from(tenth) / 10.0;
        let roll = f64::from(tenth) / 10.0 - 180.0;
        let pitch = 180.0 - f64::from(tenth) / 10.0;
        let (old, new) = both_at(
            &[("heading", heading), ("roll", roll), ("pitch", pitch)],
            &s,
        );
        assert_eq!(
            old, new,
            "Euler words diverged at heading {heading}, roll {roll}, pitch {pitch}: \
             legacy {old:02X?}, descriptor {new:02X?}"
        );
        checked += 1;
    }
    assert_eq!(checked, 3601);
}

/// The i16 SATURATION the legacy `clamp` performs. 360 deg x 16 = 5760 counts
/// is comfortably inside i16, so the declared ranges never reach it — this
/// drives the channel past its declared maximum, which `seed_input` (unlike
/// `set_input`) allows, precisely so the clamp is exercised rather than assumed.
#[test]
fn euler_saturates_identically_past_the_i16_ceiling() {
    let s = read_reg(REG_EUL_H_LSB, 2);
    for deg in [2047.9, 2048.0, 5000.0, -2048.1, -5000.0] {
        let (old, new) = both_at(&[("heading", deg)], &s);
        assert_eq!(old, new, "saturation diverged at {deg} deg");
    }
    let (old, _) = both_at(&[("heading", 5000.0)], &s);
    assert_eq!(
        i16::from_le_bytes([old[0], old[1]]),
        i16::MAX,
        "5000 deg x 16 saturates at i16::MAX"
    );
}

// ─── identical: what stores, what drops ────────────────────────────────────

#[test]
fn configuration_registers_store_and_read_back_identically() {
    let s = script([
        write_reg(REG_OPR_MODE, &[0x0C]), // NDOF
        write_reg(REG_UNIT_SEL, &[0x83]),
        write_reg(REG_PWR_MODE, &[0x02]),
        read_reg(REG_OPR_MODE, 1),
        read_reg(REG_UNIT_SEL, 1),
        read_reg(REG_PWR_MODE, 1),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "configuration read-back diverged");
    assert_eq!(old, vec![0x0C, 0x83, 0x02]);
}

#[test]
fn read_only_registers_drop_the_write_identically() {
    let s = script([
        // The identity bytes and the status constants are silicon's.
        write_reg(REG_CHIP_ID, &[0x00, 0x00, 0x00, 0x00]),
        write_reg(REG_CALIB_STAT, &[0x00]),
        // SYS_TRIGGER bit 5 is the soft reset; the part stays powered and the
        // byte is not stored, so the register reads 0 afterwards.
        write_reg(REG_SYS_TRIGGER, &[0x20]),
        read_reg(REG_CHIP_ID, 4),
        read_reg(REG_CALIB_STAT, 1),
        read_reg(REG_SYS_TRIGGER, 1),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "read-only drop transcript diverged");
    assert_eq!(old, vec![0xA0, 0xFB, 0x32, 0x0F, 0xFF, 0x00]);
}

/// The NEGATIVE control for the test above: if the descriptor had made those
/// registers writable, the burst write would have landed and this read would
/// show it. Writing a DISTINCT non-zero pattern is what tells "the write was
/// dropped" apart from "the write stored the reset value again".
#[test]
fn a_burst_write_over_the_identity_block_changes_nothing() {
    let s = script([
        write_reg(REG_CHIP_ID, &[0xDE, 0xAD, 0xBE, 0xEF, 0x11, 0x22, 0x33]),
        read_reg(REG_CHIP_ID, 8),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "identity burst-write transcript diverged");
    assert_eq!(
        old,
        vec![0xA0, 0xFB, 0x32, 0x0F, 0x00, 0x00, 0x00, 0x00],
        "the four identity bytes, then the undeclared 0x04..0x07 gap — PAGE_ID \
         at 0x07 reads its stored 0"
    );
}

#[test]
fn a_burst_write_walks_the_pointer_identically() {
    // One transaction: pointer 0x3B, then three data bytes into UNIT_SEL,
    // 0x3C (undeclared) and OPR_MODE.
    let s = script([
        write_reg(REG_UNIT_SEL, &[0x81, 0xFF, 0x0C]),
        read_reg(REG_UNIT_SEL, 3),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "burst write transcript diverged");
    assert_eq!(
        old,
        vec![0x81, 0x00, 0x0C],
        "UNIT_SEL stored, the gap swallowed its byte, OPR_MODE stored"
    );
}

// ─── identical: undeclared addresses and the pointer wrap ──────────────────

#[test]
fn undeclared_addresses_read_zero_and_the_pointer_wraps() {
    let s = script([
        read_reg(0x08, 8),
        read_reg(0x20, 4),
        // 0xFE, 0xFF, then 0x00 and 0x01 — the wrap, not a saturation.
        read_reg(0xFE, 4),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new, "unmapped/wrap transcript diverged");
    assert_eq!(
        &old[12..],
        &[0x00, 0x00, 0xA0, 0xFB],
        "the pointer wrapped 0xFF -> 0x00 and landed on CHIP_ID"
    );
}

// ─── identical: the bank select at its two defined values ──────────────────

#[test]
fn page_one_blanks_the_whole_map_identically() {
    let s = script([
        write_reg(REG_PAGE_ID, &[0x01]),
        // EVERYTHING reads 0 on page 1 — PAGE_ID itself included.
        read_reg(REG_CHIP_ID, 8),
        read_reg(REG_EUL_H_LSB, 6),
        read_reg(REG_CALIB_STAT, 1),
        // …and page 0 brings the map back untouched.
        write_reg(REG_PAGE_ID, &[0x00]),
        read_reg(REG_CHIP_ID, 8),
        read_reg(REG_EUL_H_LSB, 6),
        read_reg(REG_CALIB_STAT, 1),
    ]);
    let (old, new) = both_at(&[("heading", 45.0)], &s);
    assert_eq!(old, new, "bank-select transcript diverged");
    assert_eq!(&old[..15], &[0x00; 15], "page 1 is blank");
    assert_eq!(
        &old[15..19],
        &[0xA0, 0xFB, 0x32, 0x0F],
        "page 0 restores the identity bytes"
    );
    let euler = &old[23..29];
    assert_eq!(
        i16::from_le_bytes([euler[0], euler[1]]),
        45 * 16,
        "the heading survived the bank round trip"
    );
}

// ─── the one deliberate difference ─────────────────────────────────────────

/// ⚠️ THE NAMED DIFFERENCE. The hand model stored `PAGE_ID & 0x01`, so writing
/// 0x02 selected page 0 and left the map live; the engine's bank select takes
/// the byte as written, so 0x02 selects bank 2 — a bank this part declares no
/// registers on, which therefore reads 0x00 everywhere.
///
/// The BNO055 datasheet defines PAGE_ID as a one-bit field holding 0 or 1
/// (§4.2.1, Table 4-2), so the two models can only disagree about a value
/// silicon does not define and no driver sends. Asserted as a difference so it
/// is a stated fact rather than a surprise, with
/// `page_one_blanks_the_whole_map_identically` immediately above pinning both
/// DEFINED values.
#[test]
fn an_undefined_page_value_masks_on_the_model_and_selects_on_the_descriptor() {
    let s = script([write_reg(REG_PAGE_ID, &[0x02]), read_reg(REG_CHIP_ID, 1)]);
    let (old, new) = both(&s);
    assert_eq!(
        old,
        vec![0xA0],
        "the deleted model masked 0x02 to bank 0 and answered CHIP_ID"
    );
    assert_eq!(
        new,
        vec![0x00],
        "the descriptor selects bank 2, which has no registers"
    );
}

/// The other half: getting BACK from an undefined bank still works, so an
/// unlucky write cannot strand a part. The engine answers the bank select
/// before it decodes a register, which is what makes PAGE_ID reachable from a
/// bank that declares nothing.
#[test]
fn an_undefined_page_is_escapable_on_both_models() {
    let s = script([
        write_reg(REG_PAGE_ID, &[0x02]),
        write_reg(REG_PAGE_ID, &[0x00]),
        read_reg(REG_CHIP_ID, 1),
    ]);
    let (old, new) = both(&s);
    assert_eq!(old, new);
    assert_eq!(old, vec![0xA0]);
}
