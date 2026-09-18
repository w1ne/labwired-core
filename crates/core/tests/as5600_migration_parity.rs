// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! AS5600: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! The deleted `components/as5600.rs` is reproduced below as [`legacy`] — the
//! wire behaviour only — and both models are driven through the SAME I²C
//! script. Transcripts that must be identical are asserted equal byte for
//! byte; the one that must differ is asserted as a difference, by name.
//!
//! Held identical:
//!   * the 12-bit angle at RAW_ANGLE (0x0C) and ANGLE (0x0E), right-aligned in
//!     a big-endian word, across the whole 0..360° stimulus range;
//!   * the byte-walking pointer, which is how every driver reads the angle as
//!     one two-byte transaction;
//!   * STATUS (magnet detected), AGC and MAGNITUDE, and the 0x00 an undecoded
//!     address answers.
//!
//! Deliberately DIFFERENT:
//!   * ZPOS / MPOS / MANG / CONF are real storage. The old model discarded
//!     every configuration write and read back 0 forever, so a driver that
//!     programmed a zero position and verified it saw its write vanish.
//!
//! No longer different: exactly 360.0°. The first version of this descriptor
//! had to clamp there because `encode:` could not wrap; `encode.wrap: 4096`
//! landed with this file's second revision and a full turn reads 0 again.
//!
//! Scripts are driven through Phase A's shared harness
//! (`tests/common/transcript.rs`), the same one the byte-parity ratchet uses.

mod common;

use common::transcript::{read_reg, run_i2c, script, write_reg, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::sim_input::SimInput;

const ADDR: u8 = 0x36;

// ─── the model this descriptor replaces ────────────────────────────────────

/// `crates/core/src/peripherals/components/as5600.rs` at `origin/main`, wire
/// behaviour only. This is the ONLY place the old behaviour survives.
mod legacy {
    use labwired_core::peripherals::i2c::I2cDevice;

    const REG_STATUS: u8 = 0x0B;
    const REG_RAW_ANGLE_H: u8 = 0x0C;
    const REG_RAW_ANGLE_L: u8 = 0x0D;
    const REG_ANGLE_H: u8 = 0x0E;
    const REG_ANGLE_L: u8 = 0x0F;
    const REG_AGC: u8 = 0x1A;
    const REG_MAG_H: u8 = 0x1B;
    const REG_MAG_L: u8 = 0x1C;
    const STATUS_MD: u8 = 1 << 5;

    pub struct As5600 {
        address: u8,
        current_register: u8,
        register_address_written: bool,
        angle_deg: f64,
    }

    impl As5600 {
        pub fn new(address: u8) -> Self {
            Self {
                address,
                current_register: 0,
                register_address_written: false,
                angle_deg: 0.0,
            }
        }

        pub fn set_angle_deg(&mut self, deg: f64) {
            let mut d = deg % 360.0;
            if d < 0.0 {
                d += 360.0;
            }
            self.angle_deg = d;
        }

        fn raw12(&self) -> u16 {
            ((self.angle_deg / 360.0) * 4096.0)
                .round()
                .clamp(0.0, 4095.0) as u16
        }

        fn read_register(&self, reg: u8) -> u8 {
            let raw = self.raw12();
            match reg {
                REG_STATUS => STATUS_MD,
                REG_RAW_ANGLE_H | REG_ANGLE_H => ((raw >> 8) & 0x0F) as u8,
                REG_RAW_ANGLE_L | REG_ANGLE_L => (raw & 0xFF) as u8,
                REG_AGC => 128,
                REG_MAG_H => 0x0C,
                REG_MAG_L => 0x80,
                _ => 0,
            }
        }
    }

    impl I2cDevice for As5600 {
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
                self.current_register = self.current_register.wrapping_add(1);
            }
        }

        fn stop(&mut self) {
            self.register_address_written = false;
        }
    }
}

// ─── the script vocabulary ─────────────────────────────────────────────────
//
// Every step is framed STOP … STOP: the model this replaces clears its
// pointer-written latch on STOP only, so a script that did not close the
// previous transaction would feed the next register address in as DATA.

/// Point at `reg`, repeated START, clock out `n` bytes.
fn read_at(reg: u8, n: usize) -> Vec<Step<'static>> {
    script([vec![Step::Stop], read_reg(reg, n)])
}

/// Point at `reg` and write `data` — a configuration write.
fn write_at(reg: u8, data: &[u8]) -> Vec<Step<'static>> {
    script([vec![Step::Stop], write_reg(reg, data)])
}

fn declarative(angle: f64) -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("as5600")
        .expect("as5600 descriptor is not embedded — check embedded_device_yaml");
    let mut dev = GenericI2cDevice::from_yaml(yaml, ADDR).expect("as5600.yaml does not build");
    dev.set_input("angle", angle).expect("angle channel");
    dev
}

fn both(angle: f64, steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::As5600::new(ADDR);
    old.set_angle_deg(angle);
    let mut new = declarative(angle);
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

fn assert_parity(name: &str, angle: f64, steps: &[Step<'_>]) -> Vec<u8> {
    let (old, new) = both(angle, steps);
    assert!(!old.is_empty(), "{name}: the script read no bytes at all");
    assert_eq!(old, new, "{name}: the YAML model changed the transcript");
    new
}

// ─── held identical ────────────────────────────────────────────────────────

#[test]
fn the_whole_register_map_reads_identically() {
    // Every address the old model decoded, plus the gaps between them, in one
    // walk from 0x00. The gaps matter: the old model answered 0 for them, and
    // the descriptor has to answer the same `unmapped_byte`.
    let steps = script((0x00u8..=0x20).map(|r| read_at(r, 1)));
    let bytes = assert_parity("full map", 123.0, &steps);
    assert_eq!(bytes.len(), 0x21);
    assert_eq!(bytes[0x0B], 0x20, "STATUS: magnet detected");
    assert_eq!(bytes[0x1A], 128, "AGC mid-range");
    assert_eq!((bytes[0x1B], bytes[0x1C]), (0x0C, 0x80), "MAGNITUDE");
}

#[test]
fn a_two_byte_angle_read_walks_the_pointer() {
    // THE transaction every driver issues: point at 0x0C, read two bytes. It
    // only works because the pointer walks one BYTE per byte read.
    for angle in [0.0, 0.5, 45.0, 90.0, 123.4, 180.0, 270.0, 359.9] {
        let bytes = assert_parity("angle read", angle, &read_at(0x0C, 2));
        let raw = (u16::from(bytes[0]) << 8) | u16::from(bytes[1]);
        assert!(raw <= 4095, "angle {angle}: raw {raw} is not 12-bit");
    }
}

#[test]
fn the_encoded_angle_matches_over_the_whole_range() {
    // The encode is a multiply where the old model divided, so the two could
    // round apart at a boundary. Swept in 0.05° steps — 7201 comparisons —
    // rather than asserted at a handful of convenient angles.
    // 0.00 .. 360.00 in 0.05° steps, the full turn included: with
    // `encode.wrap` there is no longer a value in the channel range where the
    // two models disagree.
    for step in 0..=7200u32 {
        let angle = f64::from(step) * 0.05;
        let (old, new) = both(angle, &script([read_at(0x0C, 2), read_at(0x0E, 2)]));
        assert_eq!(old, new, "angle {angle}: the encoded word differs");
    }
}

#[test]
fn angle_and_raw_angle_report_the_same_word() {
    // Stated approximation, pinned: the ZPOS/MPOS/MANG output scaling is not
    // applied, so ANGLE equals RAW_ANGLE — exactly as in the model replaced.
    let bytes = assert_parity("angle vs raw", 200.0, &read_at(0x0C, 4));
    assert_eq!(bytes[0..2], bytes[2..4]);
}

#[test]
fn a_configuration_write_does_not_disturb_the_angle() {
    // A driver that programs CONF must still read the same angle afterwards —
    // the regression that making those registers real storage could cause.
    assert_parity(
        "config then angle",
        90.0,
        &script([write_at(0x07, &[0x00, 0x20]), read_at(0x0C, 2)]),
    );
}

// ─── deliberately different ────────────────────────────────────────────────

#[test]
fn a_configuration_write_now_sticks() {
    // THE deliberate change. The old model discarded configuration writes and
    // answered 0 forever ("Config registers ignored for wave-1 readback
    // fidelity"), so a driver that set a zero position and verified it failed.
    let steps = script([write_at(0x01, &[0x0A, 0xBC]), read_at(0x01, 2)]);
    let (old, new) = both(0.0, &steps);

    assert_eq!(old, vec![0x00, 0x00], "the model this replaces read back 0");
    assert_eq!(
        new,
        vec![0x0A, 0xBC],
        "ZPOS must read back what was written"
    );
    assert_ne!(old, new, "this difference is the point of the migration");
}

#[test]
fn exactly_360_degrees_wraps_to_zero_like_the_model_it_replaces() {
    // This was the ONE deliberate difference the first AS5600 descriptor had
    // to declare: `encode:` was linear-and-clamp only, so a full turn answered
    // a clamped 4095 (0.088° short of where the magnet is) where the hand
    // model's `deg % 360.0` answered 0. `encode.wrap: 4096` is that missing
    // primitive, so the difference is GONE — a full turn is the same shaft
    // position as 0 and now reads the same count.
    let (old, new) = both(360.0, &read_at(0x0C, 2));

    assert_eq!(
        old,
        vec![0x00, 0x00],
        "the model this replaces wrapped to 0"
    );
    assert_eq!(new, old, "encode.wrap must reproduce the wrap, not clamp");
    assert_ne!(
        new,
        vec![0x0F, 0xFF],
        "0x0FFF is the CLAMPED answer this primitive exists to replace"
    );
}

#[test]
fn the_full_turn_reads_the_same_count_as_the_start_of_one() {
    // `wrap` is a modular counter, not a special case for the one value 360.0.
    // The declared channel range is 0..360°, so 360.0 and 0.0 are the two
    // stimuli in range that are the SAME shaft position; they must produce the
    // same two bytes, and the count either side of the roll-over must not move.
    let full = assert_parity("360°", 360.0, &read_at(0x0C, 2));
    let zero = assert_parity("0°", 0.0, &read_at(0x0C, 2));
    assert_eq!(full, zero, "a full turn is the same shaft position as 0");

    let just_under = assert_parity("359.95°", 359.95, &read_at(0x0C, 2));
    assert_eq!(
        just_under,
        vec![0x0F, 0xFF],
        "the last count before the roll"
    );
    let just_over = assert_parity("0.05°", 0.05, &read_at(0x0C, 2));
    assert_eq!(just_over, vec![0x00, 0x01], "the first count after it");
}

#[test]
fn a_config_register_still_reads_zero_until_it_is_written() {
    // Which is why every transcript that does not write configuration is
    // byte-identical: the change is that a write sticks, not that the power-on
    // value moved.
    let steps = script((0x00u8..=0x0A).map(|r| read_at(r, 1)));
    let bytes = assert_parity("unwritten config", 10.0, &steps);
    assert_eq!(bytes, vec![0u8; 11]);
}
