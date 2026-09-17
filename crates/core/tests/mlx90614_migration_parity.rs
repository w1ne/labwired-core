// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! MLX90614: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! The deleted `components/mlx90614.rs` is reproduced below as [`legacy`] — the
//! wire behaviour only — and both models are driven through the SAME I²C
//! script. Transcripts that must be identical are asserted equal byte for
//! byte; the ones that must differ are asserted as differences, by name.
//!
//! Held identical:
//!   * the three-byte SMBus read-word frame — LSB, MSB, PEC — for all three
//!     RAM commands, over the ENTIRE declared channel range of each (−70..380 °C
//!     object, −40..125 °C ambient), every 0.01 °C;
//!   * the PEC itself, which is a CRC-8 over `[addr·W, cmd, addr·R, LSB, MSB]`
//!     and therefore over bytes the RESPONSE does not contain;
//!   * a read that runs past the three-byte frame answering 0xFF.
//!
//! Deliberately DIFFERENT (each with its own test below):
//!   * an undecoded command answers 0xFF. The hand model answered EVERY command
//!     byte with an object-temperature frame, so a driver reading its own SMBus
//!     address out of EEPROM at 0x2E was handed a temperature and believed it;
//!   * a bare read with no preceding command answers 0xFF, where the model
//!     powered up with its RAM pointer already parked on TOBJ1.
//!
//! Two primitives were needed: `crc8.covers: transaction` (SMBus 3.1 §6.4.1 —
//! the PEC covers the addressed frame, not the response word) and a
//! little-endian `ResponseWord` (SMBus 3.1 §6.5.5 sends data low byte first).
//! `the_pec_covers_the_address_and_the_command` is the test that a
//! response-only checksum could not pass.

mod common;

use common::transcript::{read_reg, run_i2c, script, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::sim_input::SimInput;

const ADDR: u8 = 0x5A;
const CMD_TA: u8 = 0x06;
const CMD_TOBJ1: u8 = 0x07;
const CMD_TOBJ2: u8 = 0x08;

// ─── the model this descriptor replaces ────────────────────────────────────

/// `crates/core/src/peripherals/components/mlx90614.rs` at `origin/main`, wire
/// behaviour only. This is the ONLY place the old behaviour survives.
mod legacy {
    use labwired_core::peripherals::i2c::I2cDevice;

    fn celsius_to_raw(t_c: f64) -> u16 {
        (((t_c + 273.15) * 50.0).round()).clamp(0.0, 0x7FFF as f64) as u16
    }

    pub fn smbus_pec(bytes: &[u8]) -> u8 {
        let mut crc: u8 = 0;
        for &b in bytes {
            crc ^= b;
            for _ in 0..8 {
                crc = if crc & 0x80 != 0 {
                    (crc << 1) ^ 0x07
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    pub struct Mlx90614 {
        address: u8,
        surface: f64,
        ambient: f64,
        pointer: u8,
        write_buf: Vec<u8>,
        response: [u8; 3],
        read_byte_idx: usize,
        latched: bool,
    }

    impl Mlx90614 {
        pub fn new(address: u8, surface: f64, ambient: f64) -> Self {
            Self {
                address,
                surface,
                ambient,
                pointer: 0x07,
                write_buf: Vec::with_capacity(2),
                response: [0; 3],
                read_byte_idx: 0,
                latched: false,
            }
        }

        fn latch_response(&mut self) {
            // ⚠️ The `_` arm is the approximation this migration replaces: ANY
            // command byte answered with the surface temperature.
            let raw = match self.pointer {
                0x07 | 0x08 => celsius_to_raw(self.surface),
                0x06 => celsius_to_raw(self.ambient),
                _ => celsius_to_raw(self.surface),
            };
            let lsb = (raw & 0xFF) as u8;
            let msb = (raw >> 8) as u8;
            let addr_w = self.address << 1;
            let addr_r = (self.address << 1) | 1;
            let pec = smbus_pec(&[addr_w, self.pointer, addr_r, lsb, msb]);
            self.response = [lsb, msb, pec];
            self.read_byte_idx = 0;
            self.latched = true;
        }
    }

    impl I2cDevice for Mlx90614 {
        fn address(&self) -> u8 {
            self.address
        }

        fn start(&mut self) {
            self.write_buf.clear();
            self.read_byte_idx = 0;
            self.latched = false;
        }

        fn stop(&mut self) {
            self.write_buf.clear();
        }

        fn write(&mut self, data: u8) {
            self.write_buf.push(data);
            if self.write_buf.len() == 1 {
                self.pointer = data;
            }
        }

        fn read(&mut self) -> u8 {
            if !self.latched {
                self.latch_response();
            }
            let byte = self
                .response
                .get(self.read_byte_idx)
                .copied()
                .unwrap_or(0xFF);
            self.read_byte_idx += 1;
            byte
        }
    }
}

// ─── harness ───────────────────────────────────────────────────────────────

fn declarative(address: u8) -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("mlx90614")
        .expect("mlx90614 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, address).expect("mlx90614.yaml does not build")
}

/// Both models at one address, with both temperatures driven.
fn both_at(address: u8, surface: f64, ambient: f64, steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::Mlx90614::new(address, surface, ambient);
    let mut new = declarative(address);
    new.set_input("surface_temp", surface).expect("surface");
    new.set_input("ambient_temp", ambient).expect("ambient");
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

fn both(surface: f64, ambient: f64, steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    both_at(ADDR, surface, ambient, steps)
}

fn assert_parity(name: &str, surface: f64, ambient: f64, steps: &[Step<'_>]) -> Vec<u8> {
    let (old, new) = both(surface, ambient, steps);
    assert!(!old.is_empty(), "{name}: the script read no bytes at all");
    assert_eq!(old, new, "{name}: the YAML model changed the transcript");
    new
}

/// The little-endian word of a 3-byte SMBus frame.
fn raw(frame: &[u8]) -> u16 {
    assert_eq!(frame.len(), 3, "an SMBus read-word frame is LSB, MSB, PEC");
    (u16::from(frame[1]) << 8) | u16::from(frame[0])
}

fn raw_to_celsius(raw: u16) -> f64 {
    f64::from(raw) * 0.02 - 273.15
}

// ─── held identical ────────────────────────────────────────────────────────

#[test]
fn the_object_frame_matches_over_the_whole_channel_range() {
    // §8.4.4 encodes T[K] × 50, and the descriptor reaches that through a
    // DERIVED channel (`surface_temp + 273.15`) with `encode: { scale: 50.0 }`
    // — the same order the datasheet states, which matters: multiplying before
    // adding would land exactly halfway between two counts at every 0.02 °C and
    // round apart there. Swept at 0.01 °C over the entire −70..380 °C range,
    // 45001 points, PEC included.
    for milli in (-70_000i32..=380_000).step_by(10) {
        let t = f64::from(milli) / 1000.0;
        let (old, new) = both(t, 22.0, &read_reg(CMD_TOBJ1, 3));
        assert_eq!(old, new, "{t} °C: the SMBus frame differs");
    }
}

#[test]
fn the_ambient_frame_matches_over_the_whole_channel_range() {
    for milli in (-40_000i32..=125_000).step_by(10) {
        let t = f64::from(milli) / 1000.0;
        let (old, new) = both(18.0, t, &read_reg(CMD_TA, 3));
        assert_eq!(old, new, "{t} °C: the Ta frame differs");
    }
}

#[test]
fn the_frame_is_lsb_then_msb_then_pec() {
    // SMBus 3.1 §6.5.5: data low byte first. 36.5 °C is (36.5 + 273.15) × 50 =
    // 15482 counts = 0x3C7A, so the wire carries 0x7A then 0x3C — and a
    // big-endian word would have been a different temperature entirely.
    let bytes = assert_parity("byte order", 36.5, 22.0, &read_reg(CMD_TOBJ1, 3));
    assert_eq!(bytes[0], 0x7A, "the LOW byte comes first");
    assert_eq!(bytes[1], 0x3C, "then the high byte");
    assert_eq!(raw(&bytes), 15_482, "36.5 °C at 0.02 K/LSB");
    assert!((raw_to_celsius(raw(&bytes)) - 36.5).abs() < 0.02);
    // The two halves read the other way round would be 31_292 counts — a
    // 352 °C surface, which is what a big-endian response word would report.
    assert_ne!(
        raw(&bytes),
        0x7A3C,
        "a BE word would be a different reading"
    );
}

#[test]
fn object_two_mirrors_object_one() {
    // A single-zone part: §8.4.3's TOBJ2 carries the same measurement, but its
    // PEC differs because the COMMAND byte is inside the checksum.
    let steps = script([read_reg(CMD_TOBJ1, 3), read_reg(CMD_TOBJ2, 3)]);
    let bytes = assert_parity("tobj2", 8.0, 22.0, &steps);
    assert_eq!(&bytes[0..2], &bytes[3..5], "the same measurement");
    assert_ne!(bytes[2], bytes[5], "…and a different PEC, per the command");
}

#[test]
fn a_read_past_the_frame_answers_ff_in_both() {
    let (old, new) = both(18.0, 22.0, &read_reg(CMD_TOBJ1, 6));
    assert_eq!(old, new, "the tail is identical");
    assert_eq!(&new[3..], &[0xFF, 0xFF, 0xFF], "nothing past the PEC");
}

#[test]
fn the_temperatures_hold_until_driven() {
    // No self-running scene: 20 reads of the same command return the same
    // frame, and only `set_input` moves it. (The MLX model's own note is where
    // this project first wrote that rule down.)
    let steps = script((0..20).map(|_| read_reg(CMD_TOBJ1, 3)));
    let bytes = assert_parity("stationary", 18.0, 22.0, &steps);
    for chunk in bytes.chunks(3) {
        assert_eq!(chunk, &bytes[0..3], "the reading moved without a stimulus");
    }
    let driven = assert_parity("driven", 8.0, 22.0, &read_reg(CMD_TOBJ1, 3));
    assert!(
        (raw_to_celsius(raw(&driven)) - 8.0).abs() < 0.02,
        "driven to 8 °C"
    );
    assert_ne!(&driven[..], &bytes[0..3], "…and it did move");
}

// ─── the PEC scope, proved ─────────────────────────────────────────────────

#[test]
fn the_pec_is_a_real_smbus_crc8_over_the_addressed_frame() {
    // Computed independently in the test rather than compared between two
    // models that could agree on a wrong checksum: CRC-8 poly 0x07, init 0,
    // over `[addr << 1, cmd, (addr << 1) | 1, LSB, MSB]` (SMBus 3.1 §6.4.1).
    for (cmd, surface, ambient) in [
        (CMD_TOBJ1, 18.0, 22.0),
        (CMD_TOBJ1, -12.5, 22.0),
        (CMD_TA, 18.0, 40.0),
        (CMD_TOBJ2, 300.0, 22.0),
    ] {
        let bytes = assert_parity("pec", surface, ambient, &read_reg(cmd, 3));
        let expect = legacy::smbus_pec(&[ADDR << 1, cmd, (ADDR << 1) | 1, bytes[0], bytes[1]]);
        assert_eq!(
            bytes[2], expect,
            "cmd {cmd:#04x}: the PEC is not the SMBus CRC-8"
        );
    }
}

#[test]
fn the_pec_covers_the_address_and_the_command() {
    // THE test `covers: transaction` exists for. A checksum over the response
    // WORD alone — which is all `crc8:` could express before — would be
    // IDENTICAL in both halves below, because the two data bytes are identical
    // in both halves. A driver validating the PEC rejects every reading from
    // such a model.
    //
    // Half one: same measurement, different COMMAND.
    let ta = assert_parity("ta", 30.0, 30.0, &read_reg(CMD_TA, 3));
    let tobj = assert_parity("tobj", 30.0, 30.0, &read_reg(CMD_TOBJ1, 3));
    assert_eq!(&ta[0..2], &tobj[0..2], "the same two data bytes");
    assert_ne!(ta[2], tobj[2], "the command byte must be inside the PEC");

    // Half two: same measurement and same command, different ADDRESS. The part
    // is jumpered to a second SMBus address, which is what `i2c_address:` does.
    let (old_a, new_a) = both_at(0x5A, 30.0, 22.0, &read_reg(CMD_TOBJ1, 3));
    let (old_b, new_b) = both_at(0x5B, 30.0, 22.0, &read_reg(CMD_TOBJ1, 3));
    assert_eq!(old_a, new_a, "0x5A: identical to the model");
    assert_eq!(
        old_b, new_b,
        "0x5B: identical to the model at a new address"
    );
    assert_eq!(&new_a[0..2], &new_b[0..2], "the same two data bytes");
    assert_ne!(new_a[2], new_b[2], "the address must be inside the PEC too");
    assert_eq!(
        new_b[2],
        legacy::smbus_pec(&[0x5B << 1, CMD_TOBJ1, (0x5B << 1) | 1, new_b[0], new_b[1]]),
        "the PEC follows the address the device was attached at"
    );
}

// ─── deliberately different ────────────────────────────────────────────────

#[test]
fn an_undecoded_command_answers_ff_instead_of_a_temperature() {
    // THE deliberate change. The hand model's `_` arm answered the object
    // temperature for EVERY command byte, so a driver reading its own SMBus
    // address from EEPROM (0x2E), the emissivity (0x24) or the config register
    // (0x25) was handed a plausible temperature and had no way to tell.
    for cmd in [0x24u8, 0x25, 0x2E, 0x00, 0xFF] {
        let (old, new) = both(18.0, 22.0, &read_reg(cmd, 3));
        assert!(
            (raw_to_celsius(raw(&old)) - 18.0).abs() < 0.02,
            "cmd {cmd:#04x}: the model answered a surface temperature"
        );
        assert_eq!(
            new,
            vec![0xFF, 0xFF, 0xFF],
            "cmd {cmd:#04x}: nothing decoded"
        );
        assert_ne!(old, new, "this difference is the point of the change");
    }
}

#[test]
fn a_read_with_no_command_answers_ff_instead_of_the_parked_pointer() {
    // The model powered up with its RAM pointer already on TOBJ1, so a bare
    // read — a transaction no MLX driver issues — returned an object
    // temperature. A command device has no pointer to park.
    let steps = vec![Step::Start, Step::Read(3), Step::Stop];
    let (old, new) = both(18.0, 22.0, &steps);
    assert!(
        (raw_to_celsius(raw(&old)) - 18.0).abs() < 0.02,
        "the parked read"
    );
    assert_eq!(new, vec![0xFF, 0xFF, 0xFF], "no command, no response");
}

// ─── the config seed, proved end to end ────────────────────────────────────

#[test]
fn the_config_keys_seed_the_channels_they_name() {
    // The hand-written kit took `surface_temp_c` / `ambient_temp_c` in
    // `config:` and served `surface_temp` / `ambient_temp` as runtime channels,
    // and three shipped `system.yaml` files set the former. `InputSpec.config_key`
    // is what keeps that working; without it the seed would parse and change
    // nothing, and the part would boot at the descriptor default.
    //
    // Driven through `i2c_factory::build_i2c_device` because that is the path
    // the ESP32-C3 controller the Leo board uses actually takes — not the kit
    // registry — so this is the seeding the shipped example depends on.
    use labwired_core::peripherals::components::i2c_factory::build_i2c_device;
    use std::collections::HashMap;

    let mut config: HashMap<String, serde_yaml::Value> = HashMap::new();
    config.insert("i2c_address".into(), serde_yaml::Value::from(0x5A));
    config.insert("surface_temp_c".into(), serde_yaml::Value::from(5.0));
    config.insert("ambient_temp_c".into(), serde_yaml::Value::from(21.0));
    let mut dev = build_i2c_device("mlx90614", &config).expect("factory builds the mlx90614");

    let surface = run_i2c(dev.as_mut(), &read_reg(CMD_TOBJ1, 3)).bytes;
    assert!(
        (raw_to_celsius(raw(&surface)) - 5.0).abs() < 0.02,
        "surface_temp_c did not reach the surface_temp channel: {:.2} °C",
        raw_to_celsius(raw(&surface))
    );
    let ambient = run_i2c(dev.as_mut(), &read_reg(CMD_TA, 3)).bytes;
    assert!(
        (raw_to_celsius(raw(&ambient)) - 21.0).abs() < 0.02,
        "ambient_temp_c did not reach the ambient_temp channel: {:.2} °C",
        raw_to_celsius(raw(&ambient))
    );

    // The negative control: with NO config the part boots at the descriptor's
    // declared defaults, so the assertions above cannot be passing by accident.
    let mut bare = build_i2c_device("mlx90614", &HashMap::new()).expect("builds");
    let bytes = run_i2c(bare.as_mut(), &read_reg(CMD_TOBJ1, 3)).bytes;
    assert!(
        (raw_to_celsius(raw(&bytes)) - 18.0).abs() < 0.02,
        "the default"
    );
}
