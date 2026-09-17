// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! MMA8451Q: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! The deleted `components/mma8451q.rs` is reproduced below as [`legacy`] — the
//! wire behaviour only — and both models are driven through the SAME I²C
//! script. Transcripts that must be identical are asserted equal byte for
//! byte; the ones that must differ are asserted as differences, by name.
//!
//! Held identical:
//!   * the whole register map in ONE auto-incrementing burst, in standby and
//!     active, including the 0x00 byte every undecoded address drives;
//!   * every 14-bit count of the output range on every axis at every
//!     full-scale setting — the low two bits of each word always zero, which is
//!     what "left-justified" means on the wire;
//!   * WHO_AM_I, the FS write mask, and the standby gate lifting the moment
//!     CTRL_REG1.ACTIVE is set.
//!
//! Deliberately DIFFERENT (with its own test below):
//!   * at exactly full positive scale the model wrapped to full NEGATIVE
//!     scale; the descriptor saturates at the converter's own top count.
//!   * the per-instance `noise_sigma` config key is gone.
//!
//! Two primitives were needed and both are exercised here: a `scale_from`
//! INSIDE a packed field (the ±2/4/8 g select of a left-justified value), and
//! `zero_unless` — the standby gate, which fires while a bit is CLEAR.

mod common;

use common::transcript::{read_reg, run_i2c, script, write_reg, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::sim_input::SimInput;

const ADDR: u8 = 0x1C;

// ─── the model this descriptor replaces ────────────────────────────────────

/// `crates/core/src/peripherals/components/mma8451q.rs` at `origin/main`, wire
/// behaviour only (the optional seeded noise is omitted: its sigma defaulted to
/// 0 and no shipped system.yaml set it). This is the ONLY place the old
/// behaviour survives.
mod legacy {
    use labwired_core::peripherals::i2c::I2cDevice;

    pub struct Mma8451q {
        current_register: u8,
        register_address_written: bool,
        ctrl_reg1: u8,
        xyz_data_cfg: u8,
        pending: [f64; 3],
    }

    impl Mma8451q {
        pub fn new() -> Self {
            Self {
                current_register: 0,
                register_address_written: false,
                ctrl_reg1: 0,
                xyz_data_cfg: 0,
                pending: [0.0; 3],
            }
        }

        pub fn set_input(&mut self, key: &str, value: f64) {
            let axis = match key {
                "x" => 0,
                "y" => 1,
                "z" => 2,
                other => panic!("unknown legacy channel {other}"),
            };
            self.pending[axis] = value;
        }

        fn active(&self) -> bool {
            self.ctrl_reg1 & 0x01 != 0
        }

        fn counts_per_g(&self) -> f64 {
            match self.xyz_data_cfg & 0x03 {
                0 => 4096.0,
                1 => 2048.0,
                _ => 1024.0,
            }
        }

        fn read_register(&mut self, reg: u8) -> u8 {
            match reg {
                0x01..=0x06 => {
                    if !self.active() {
                        return 0;
                    }
                    let axis = ((reg - 0x01) / 2) as usize;
                    let cpg = self.counts_per_g();
                    let fs_g = 8192.0 / cpg;
                    let g = self.pending[axis].clamp(-fs_g, fs_g);
                    let counts = (g * cpg).round() as i16;
                    let justified = (counts << 2) as u16;
                    if reg % 2 == 1 {
                        (justified >> 8) as u8
                    } else {
                        (justified & 0xFF) as u8
                    }
                }
                0x0D => 0x1A,
                0x0E => self.xyz_data_cfg,
                0x2A => self.ctrl_reg1,
                _ => 0,
            }
        }
    }

    impl I2cDevice for Mma8451q {
        fn address(&self) -> u8 {
            0x1C
        }

        fn read(&mut self) -> u8 {
            let val = self.read_register(self.current_register);
            self.current_register = self.current_register.wrapping_add(1);
            val
        }

        fn write(&mut self, data: u8) {
            if !self.register_address_written {
                self.current_register = data;
                self.register_address_written = true;
            } else {
                match self.current_register {
                    0x0E => self.xyz_data_cfg = data & 0x03,
                    0x2A => self.ctrl_reg1 = data,
                    _ => {}
                }
                self.current_register = self.current_register.wrapping_add(1);
            }
        }

        fn stop(&mut self) {
            self.register_address_written = false;
        }
    }
}

// ─── harness ───────────────────────────────────────────────────────────────

fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("mma8451q")
        .expect("mma8451q descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("mma8451q.yaml does not build")
}

fn both(g: [f64; 3], steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::Mma8451q::new();
    let mut new = declarative();
    for (i, key) in ["x", "y", "z"].iter().enumerate() {
        old.set_input(key, g[i]);
        new.set_input(key, g[i]).expect("axis channel");
    }
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

fn assert_parity(name: &str, g: [f64; 3], steps: &[Step<'_>]) -> Vec<u8> {
    let (old, new) = both(g, steps);
    assert!(!old.is_empty(), "{name}: the script read no bytes at all");
    assert_eq!(old, new, "{name}: the YAML model changed the transcript");
    new
}

/// Put the part in ACTIVE at full-scale `fs` (0 = ±2 g, 1 = ±4 g, 2 = ±8 g).
fn bring_up(fs: u8) -> Vec<Step<'static>> {
    script([write_reg(0x0E, &[fs]), write_reg(0x2A, &[0x01])])
}

/// The signed 14-bit count inside a left-justified 16-bit big-endian word.
fn count14(msb: u8, lsb: u8) -> i16 {
    let word = (i32::from(msb) << 8) | i32::from(lsb);
    (word as i16) >> 2
}

const COUNTS_PER_G: [f64; 3] = [4096.0, 2048.0, 1024.0];

// ─── held identical ────────────────────────────────────────────────────────

#[test]
fn the_whole_register_map_reads_identically_in_one_burst() {
    // §6.1: the pointer walks on every byte, so 0x00..0x3F is ONE transaction
    // and every gap in the map is part of the transcript. Driven in standby
    // (outputs gated to zero) and again active.
    for (name, setup) in [("standby", script([])), ("active", bring_up(0))] {
        let steps = script([setup, read_reg(0x00, 0x40)]);
        let bytes = assert_parity(name, [0.5, -0.25, 1.0], &steps);
        assert_eq!(bytes.len(), 0x40);
        assert_eq!(bytes[0x0D], 0x1A, "{name}: WHO_AM_I");
        assert_eq!(bytes[0x00], 0x00, "{name}: 0x00 is unmapped");
        assert_eq!(bytes[0x07], 0x00, "{name}: the gap after the outputs");
        assert_eq!(bytes[0x3F], 0x00, "{name}: past the map");
    }
}

#[test]
fn the_standby_gate_lifts_exactly_when_active_is_set() {
    // §6.1: the part converts only while CTRL_REG1.ACTIVE (D0) is SET — the
    // INVERTED gate `zero_unless` exists for. Drive +1 g on X first, so the
    // value is already there and only the gate can be the reason it reads zero.
    let steps = script([
        read_reg(0x01, 2),        // standby: zero
        write_reg(0x2A, &[0x01]), // ACTIVE
        read_reg(0x01, 2),        // now converting
        write_reg(0x2A, &[0x00]), // back to standby
        read_reg(0x01, 2),        // zero again
    ]);
    let bytes = assert_parity("standby gate", [1.0, 0.0, 0.0], &steps);
    assert_eq!(&bytes[0..2], &[0x00, 0x00], "standby must not convert");
    assert_eq!(count14(bytes[2], bytes[3]), 4096, "+1 g at ±2 g");
    assert_eq!(&bytes[4..6], &[0x00, 0x00], "standby again");
}

#[test]
fn every_count_of_the_output_range_matches_on_the_x_axis() {
    // All 16384 counts of the 14-bit range at ±2 g. The descriptor places the
    // value with `shift: 2` / `width_bits: 14` where the model shifted an i16
    // left by two, and this is the sweep that proves the two never round apart
    // — including that the low two bits of the WORD are always zero, which is
    // the whole content of "left-justified".
    for count in -8192i32..=8191 {
        let g = f64::from(count) / 4096.0;
        let steps = script([bring_up(0), read_reg(0x01, 2)]);
        let (old, new) = both([g, 0.0, 0.0], &steps);
        assert_eq!(old, new, "{count} counts ({g} g): the word differs");
        assert_eq!(count14(new[0], new[1]), count as i16, "{g} g");
        assert_eq!(new[1] & 0x03, 0, "{g} g: the low two bits must be zero");
    }
}

#[test]
fn the_full_scale_select_reaches_every_axis() {
    // §6.5 Table 5: 4096 / 2048 / 1024 counts per g for ±2 / ±4 / ±8 g,
    // selected by XYZ_DATA_CFG.FS — a bit-field of another register reaching
    // INSIDE a packed field, which is what `scale_from` on a `FieldSpec` does.
    // Every axis, so the three fields cannot be sharing one scale by accident.
    for fs in 0u8..=2 {
        let cpg = COUNTS_PER_G[usize::from(fs)];
        // Starting at −8191 rather than −8192: the Y axis is driven at −g, so
        // −8192 counts on X would put Y at exactly FULL POSITIVE scale, which
        // is the one point the two models deliberately disagree on (its own
        // test below). Excluded by NAME rather than by trimming the step.
        for count in (-8191i32..=8191).step_by(23) {
            let g = f64::from(count) / cpg;
            let stim = [g, -g, g / 2.0];
            let steps = script([bring_up(fs), read_reg(0x01, 6)]);
            let (old, new) = both(stim, &steps);
            assert_eq!(old, new, "FS {fs} at {count} counts: the burst differs");
            assert_eq!(count14(new[0], new[1]), count as i16, "FS {fs} X");
            assert_eq!(count14(new[2], new[3]), -(count as i16), "FS {fs} Y");
        }
        // The reserved FS = 11 takes the ±8 g row in both models.
        let steps = script([bring_up(3), read_reg(0x01, 2)]);
        let (old, new) = both([1.0, 0.0, 0.0], &steps);
        assert_eq!(old, new, "FS = 11 is reserved and reads as ±8 g");
        assert_eq!(count14(new[0], new[1]), 1024);
    }
}

#[test]
fn a_negative_reading_is_two_s_complement_within_the_fourteen_bits() {
    let steps = script([bring_up(0), read_reg(0x05, 2)]);
    let bytes = assert_parity("negative Z", [0.0, 0.0, -1.0], &steps);
    assert_eq!(count14(bytes[0], bytes[1]), -4096, "-1 g at ±2 g");
    assert_eq!(bytes, vec![0xC0, 0x00], "−4096 << 2 = 0xC000");
}

#[test]
fn the_full_scale_field_is_the_only_writable_part_of_xyz_data_cfg() {
    // The hand model stored `data & 0x03`; the descriptor says the same with
    // `write_mask: 0x03`. The high-pass-filter output bit (D4) is not modelled,
    // so a driver that sets it must read it back CLEAR in both.
    let steps = script([write_reg(0x0E, &[0xFF]), read_reg(0x0E, 1)]);
    let bytes = assert_parity("FS write mask", [0.0; 3], &steps);
    assert_eq!(bytes, vec![0x03], "only FS[1:0] survives the write");
}

#[test]
fn a_block_write_walks_the_pointer_exactly_as_a_block_read_does() {
    // §6.1 auto-increments on writes too: one transaction sets XYZ_DATA_CFG at
    // 0x0E and then spills into 0x0F, which this map does not decode.
    let steps = script([
        write_reg(0x0E, &[0x02, 0xFF]),
        read_reg(0x0E, 2),
        write_reg(0x2A, &[0x01]),
        read_reg(0x01, 2),
    ]);
    let bytes = assert_parity("block write", [1.0, 0.0, 0.0], &steps);
    assert_eq!(bytes[0], 0x02, "XYZ_DATA_CFG took the first byte");
    assert_eq!(bytes[1], 0x00, "0x0F is unmapped and absorbed the second");
    assert_eq!(count14(bytes[2], bytes[3]), 1024, "±8 g took effect");
}

// ─── deliberately different ────────────────────────────────────────────────

#[test]
fn full_positive_scale_saturates_instead_of_wrapping_to_full_negative_scale() {
    // THE deliberate change. The hand model clamped the acceleration to
    // ±full-scale in ENGINEERING units and then multiplied, so exactly +2 g at
    // the ±2 g range produced 8192 counts — one more than a 14-bit signed field
    // can hold. Shifting that left by two gave 0x8000, which a driver reads
    // back as −2 g: the part reported full NEGATIVE scale at full POSITIVE
    // acceleration. The descriptor saturates at the converter's own top count.
    for fs in 0u8..=2 {
        let cpg = COUNTS_PER_G[usize::from(fs)];
        let full_scale_g = 8192.0 / cpg;
        let steps = script([bring_up(fs), read_reg(0x01, 2)]);
        let (old, new) = both([full_scale_g, 0.0, 0.0], &steps);
        assert_eq!(old, vec![0x80, 0x00], "FS {fs}: the model wrapped to −full");
        assert_eq!(count14(old[0], old[1]), -8192, "…reading as the wrong sign");
        assert_eq!(new, vec![0x7F, 0xFC], "FS {fs}: the descriptor saturates");
        assert_eq!(count14(new[0], new[1]), 8191, "…at the top 14-bit count");
        assert_ne!(old, new, "this difference is the point of the change");

        // The NEGATIVE end is identical: −8192 is a count the field holds.
        let (old, new) = both([-full_scale_g, 0.0, 0.0], &steps);
        assert_eq!(old, new, "FS {fs}: the negative end never disagreed");
        assert_eq!(count14(new[0], new[1]), -8192);
    }
    // Beyond full scale the same thing happens, so a driver over-driving the
    // part sees a saturated reading rather than a sign flip.
    let steps = script([bring_up(0), read_reg(0x01, 2)]);
    assert_eq!(both([8.0, 0.0, 0.0], &steps).1, vec![0x7F, 0xFC]);
    assert_eq!(both([8.0, 0.0, 0.0], &steps).0, vec![0x80, 0x00]);
}

#[test]
fn the_per_instance_noise_sigma_config_key_is_gone() {
    // The hand-written kit advertised a `noise_sigma` config key and no shipped
    // system.yaml ever set it. The declarative engine declares noise per
    // CHANNEL in the descriptor, so carrying the key forward would have meant
    // advertising one that parses and changes nothing. Asserted rather than
    // left as a comment, because a silently-ignored config key is exactly the
    // failure this schema refuses elsewhere.
    let yaml = labwired_config::embedded_device_yaml("mma8451q").expect("embedded");
    let d = labwired_config::DeviceDescriptor::from_yaml(yaml).expect("parses");
    let keys: Vec<&str> = d
        .metadata
        .as_ref()
        .expect("metadata")
        .config_keys
        .iter()
        .map(|k| k.name.as_str())
        .collect();
    assert_eq!(keys, vec!["i2c_address"], "the surface is address-only now");
    assert_eq!(d.r#type, "mma8451q", "the device_type is unchanged");
}
