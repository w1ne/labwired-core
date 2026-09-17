// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! FXOS8700CQ: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! The deleted `components/fxos8700.rs` is reproduced below as [`legacy`] — the
//! wire behaviour only — and both models are driven through the SAME I²C
//! script. Transcripts that must be identical are asserted equal byte for
//! byte; the ones that must differ are asserted as differences, by name.
//!
//! Held identical (with a manual sample latched, which is the state every
//! driven read is in):
//!   * the whole register map in ONE auto-incrementing burst, every gap
//!     included — STATUS, WHO_AM_I, the config registers, the magnetometer
//!     block, TEMP;
//!   * every 14-bit count of the accelerometer range on every axis at every
//!     full-scale setting, saturating where the model saturated;
//!   * the hybrid auto-increment jump 0x06 → 0x33, which is what
//!     `auto_increment_map` was added for;
//!   * CTRL_REG2.RST being gone by the time firmware reads it back.
//!
//! Deliberately DIFFERENT (each with its own test below):
//!   * the "grazing cow" animation is gone — the part reports what it is driven
//!     and nothing else;
//!   * the hybrid jump is unconditional, where the model gated it on
//!     `M_CTRL_REG2.hyb_autoinc_mode`;
//!   * the magnetometer is a constant field rather than a swept one.

mod common;

use common::transcript::{read_reg, run_i2c, script, write_reg, Step};
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::sim_input::SimInput;

const ADDR: u8 = 0x1F;
const ONE_G_LJ: i16 = 0x1000;

// ─── the model this descriptor replaces ────────────────────────────────────

/// `crates/core/src/peripherals/components/fxos8700.rs` at `origin/main`, wire
/// behaviour only. This is the ONLY place the old behaviour survives.
mod legacy {
    use labwired_core::peripherals::i2c::I2cDevice;

    const ONE_G_LJ: i16 = 0x1000;

    pub struct Fxos8700 {
        current_register: u8,
        register_address_written: bool,
        ctrl_reg1: u8,
        ctrl_reg2: u8,
        xyz_data_cfg: u8,
        m_ctrl_reg1: u8,
        m_ctrl_reg2: u8,
        activity_phase: u32,
        accel: [i16; 3],
        mag: [i16; 3],
        manual: bool,
    }

    impl Fxos8700 {
        pub fn new() -> Self {
            let mut s = Self {
                current_register: 0,
                register_address_written: false,
                ctrl_reg1: 0,
                ctrl_reg2: 0,
                xyz_data_cfg: 0,
                m_ctrl_reg1: 0,
                m_ctrl_reg2: 0,
                activity_phase: 0,
                accel: [0, 0, ONE_G_LJ],
                mag: [0, 0, 0],
                manual: false,
            };
            s.refresh_pose();
            s
        }

        /// The invented "grazing cow" walk this migration removes.
        fn refresh_pose(&mut self) {
            let p = (self.activity_phase % 8) as i32;
            let tri = |k: i32| -> i16 {
                let t = if k < 5 { k } else { 8 - k };
                ((t - 2) * 600) as i16
            };
            self.accel[0] = tri(p);
            self.accel[1] = tri((p + 3) % 8);
            self.accel[2] = ONE_G_LJ - (tri(p).abs() / 4);
            self.mag[0] = tri((p + 1) % 8) * 4;
            self.mag[1] = tri((p + 5) % 8) * 4;
            self.mag[2] = 1200;
        }

        pub fn set_sample(&mut self, key: &str, value: f64) {
            let fs = (self.xyz_data_cfg & 0x03).min(2) as u32;
            let counts_per_g = f64::from(ONE_G_LJ) / f64::from(1u32 << fs);
            let full_scale = f64::from(2u32 << fs);
            let raw = (value.clamp(-full_scale, full_scale) * counts_per_g).round() as i16;
            let axis = match key {
                "x" => 0,
                "y" => 1,
                "z" => 2,
                other => panic!("unknown legacy channel {other}"),
            };
            self.accel[axis] = raw;
            self.manual = true;
        }

        fn read_register(&self, reg: u8) -> u8 {
            match reg {
                0x00 => 0xFF,
                0x01 => (self.accel[0] >> 8) as u8,
                0x02 => (self.accel[0] & 0xFF) as u8,
                0x03 => (self.accel[1] >> 8) as u8,
                0x04 => (self.accel[1] & 0xFF) as u8,
                0x05 => (self.accel[2] >> 8) as u8,
                0x06 => (self.accel[2] & 0xFF) as u8,
                0x0D => 0xC7,
                0x0E => self.xyz_data_cfg,
                0x2A => self.ctrl_reg1,
                0x2B => self.ctrl_reg2,
                0x33 => (self.mag[0] >> 8) as u8,
                0x34 => (self.mag[0] & 0xFF) as u8,
                0x35 => (self.mag[1] >> 8) as u8,
                0x36 => (self.mag[1] & 0xFF) as u8,
                0x37 => (self.mag[2] >> 8) as u8,
                0x38 => (self.mag[2] & 0xFF) as u8,
                0x51 => 0x14,
                0x5B => self.m_ctrl_reg1,
                0x5C => self.m_ctrl_reg2,
                _ => 0,
            }
        }

        fn write_register(&mut self, reg: u8, value: u8) {
            match reg {
                0x0E => self.xyz_data_cfg = value,
                0x2A => self.ctrl_reg1 = value,
                0x2B => self.ctrl_reg2 = value & !0x40,
                0x5B => self.m_ctrl_reg1 = value,
                0x5C => self.m_ctrl_reg2 = value,
                _ => {}
            }
        }
    }

    /// The legacy model converted the stimulus to counts AT `set_input` TIME,
    /// using whatever full scale was configured then. A script therefore has to
    /// drive it through `Step::Input` in the right ORDER, not before the run.
    impl labwired_core::sim_input::SimInput for Fxos8700 {
        fn input_channels(&self) -> &'static [labwired_core::sim_input::InputChannel] {
            use labwired_core::sim_input::InputChannel;
            const CH: &[InputChannel] = &[
                InputChannel {
                    key: "x",
                    label: "X",
                    unit: "g",
                    min: -8.0,
                    max: 8.0,
                },
                InputChannel {
                    key: "y",
                    label: "Y",
                    unit: "g",
                    min: -8.0,
                    max: 8.0,
                },
                InputChannel {
                    key: "z",
                    label: "Z",
                    unit: "g",
                    min: -8.0,
                    max: 8.0,
                },
            ];
            CH
        }

        fn set_input(
            &mut self,
            key: &str,
            value: f64,
        ) -> Result<(), labwired_core::sim_input::SimInputError> {
            self.require_channel(key, value)?;
            self.set_sample(key, value);
            Ok(())
        }

        fn component_id(&self) -> Option<&str> {
            None
        }

        fn set_component_id(&mut self, _id: String) {}
    }

    impl I2cDevice for Fxos8700 {
        fn address(&self) -> u8 {
            0x1F
        }

        fn as_sim_input_mut(&mut self) -> Option<&mut dyn labwired_core::sim_input::SimInput> {
            Some(self)
        }

        fn read(&mut self) -> u8 {
            if self.current_register == 0x01 && !self.manual {
                self.activity_phase = self.activity_phase.wrapping_add(1);
                self.refresh_pose();
            }
            let val = self.read_register(self.current_register);
            let autoinc = (self.m_ctrl_reg2 & 0x20) != 0;
            self.current_register = if autoinc && self.current_register == 0x06 {
                0x33
            } else {
                self.current_register.wrapping_add(1)
            };
            val
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

// ─── harness ───────────────────────────────────────────────────────────────

fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("fxos8700")
        .expect("fxos8700 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("fxos8700.yaml does not build")
}

/// Both models with all three axes DRIVEN — which in the old model also latches
/// `manual`, stopping the invented walk. This is the state every transcript
/// asserted identical starts from, and the only one in which the two CAN agree.
fn both(g: [f64; 3], steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::Fxos8700::new();
    let mut new = declarative();
    for (i, key) in ["x", "y", "z"].iter().enumerate() {
        old.set_sample(key, g[i]);
        new.set_input(key, g[i]).expect("axis channel");
    }
    (
        run_i2c(&mut old, steps).bytes,
        run_i2c(&mut new, steps).bytes,
    )
}

/// Drive the three axes INSIDE the script, so the stimulus lands after whatever
/// the script did before it. The legacy model converts to counts at that
/// moment; the descriptor converts at read time.
fn drive(g: [f64; 3]) -> Vec<Step<'static>> {
    vec![
        Step::Input("x", g[0]),
        Step::Input("y", g[1]),
        Step::Input("z", g[2]),
    ]
}

/// Both models run through one script with nothing pre-driven.
fn both_scripted(steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    both_untouched(steps)
}

/// Both models with NOTHING driven — the state the animation is visible in.
fn both_untouched(steps: &[Step<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut old = legacy::Fxos8700::new();
    let mut new = declarative();
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

fn word(msb: u8, lsb: u8) -> i16 {
    ((i32::from(msb) << 8) | i32::from(lsb)) as u16 as i16
}

/// Set the accelerometer full scale (0 = ±2 g, 1 = ±4 g, 2 = ±8 g).
fn set_fs(fs: u8) -> Vec<Step<'static>> {
    write_reg(0x0E, &[fs])
}

/// Enable the datasheet's hybrid auto-increment (M_CTRL_REG2.hyb_autoinc_mode).
fn enable_hybrid() -> Vec<Step<'static>> {
    write_reg(0x5C, &[0x20])
}

// ─── held identical ────────────────────────────────────────────────────────

#[test]
fn the_whole_register_map_reads_identically_in_one_burst() {
    // Hybrid mode OFF, so the pointer walks straight through 0x00..0x2F and
    // every gap in the map is part of the transcript. (The block that crosses
    // 0x06 has its own test — with the jump enabled, which is the mode this
    // descriptor is written for.)
    let steps = script([read_reg(0x00, 0x07), read_reg(0x0D, 0x23)]);
    let bytes = assert_parity("map", [0.5, -0.25, 1.0], &steps);
    assert_eq!(bytes[0x00], 0xFF, "STATUS: data always ready");
    assert_eq!(bytes[0x07], 0xC7, "WHO_AM_I");
    assert_eq!(bytes[0x07 + 0x2A - 0x0D], 0x00, "CTRL_REG1 out of reset");
    // The upper half: the magnetometer block and TEMP.
    let steps = script([read_reg(0x33, 0x06), read_reg(0x51, 1), read_reg(0x5B, 2)]);
    let bytes = assert_parity("upper map", [0.5, -0.25, 1.0], &steps);
    assert_eq!(word(bytes[0], bytes[1]), -2400, "M_OUT_X");
    assert_eq!(word(bytes[2], bytes[3]), 2400, "M_OUT_Y");
    assert_eq!(word(bytes[4], bytes[5]), 1200, "M_OUT_Z");
    assert_eq!(bytes[6], 0x14, "TEMP ≈ +20 °C");
    assert_eq!(&bytes[7..9], &[0x00, 0x00], "the magnetometer config");
}

#[test]
fn every_count_of_the_accelerometer_range_matches_on_every_axis() {
    // The model's transfer function is `LJ = g × (0x1000 >> FS)` with the
    // acceleration clamped to ±(2 << FS) g first; the descriptor reaches the
    // same numbers as a 14-bit field at bit 2 whose `scale_from` carries
    // `1024 >> FS` and whose `encode` clamps at ±2048 counts — the SAME clamp at
    // every range, because the sensitivity halves as the range doubles.
    //
    // Swept over the whole 14-bit count range at each setting, on all three
    // axes at once and at three different counts, so a field that quietly
    // shared one axis's value could not pass. Stepped by an EVEN number so the
    // third axis's half-count is a whole count too: the one place the two
    // models disagree is a count that is not a whole 14-bit count, and that has
    // its own test below.
    //
    // The range is selected BEFORE the stimulus is driven, because the model
    // converted at `set_input` time — see
    // `a_range_change_rescales_the_reading_instead_of_leaving_it_stale`.
    for fs in 0u8..=2 {
        let counts_per_g = f64::from(ONE_G_LJ) / f64::from(1u32 << fs);
        for f14 in (-2048i32..=2048).step_by(14) {
            let g = f64::from(f14) * 4.0 / counts_per_g;
            let steps = script([set_fs(fs), drive([g, -g, g / 2.0]), read_reg(0x01, 6)]);
            let (old, new) = both_scripted(&steps);
            assert_eq!(
                old, new,
                "FS {fs} at {f14} counts ({g} g): the burst differs"
            );
            assert_eq!(word(new[0], new[1]), (f14 * 4) as i16, "FS {fs} X");
            assert_eq!(word(new[2], new[3]), (-f14 * 4) as i16, "FS {fs} Y");
            assert_eq!(new[1] & 0x03, 0, "the low two bits are silicon's");
        }
        // Full scale and beyond: both saturate at the same count.
        let full = f64::from(2u32 << fs);
        for g in [full, -full, 8.0, -8.0] {
            let steps = script([set_fs(fs), drive([g, 0.0, 1.0]), read_reg(0x01, 2)]);
            let (old, new) = both_scripted(&steps);
            assert_eq!(old, new, "FS {fs} at {g} g: saturation differs");
        }
    }
}

#[test]
fn one_g_on_z_is_the_flat_pose_both_models_start_from() {
    let steps = script([read_reg(0x05, 2)]);
    let bytes = assert_parity("flat", [0.0, 0.0, 1.0], &steps);
    assert_eq!(word(bytes[0], bytes[1]), ONE_G_LJ, "+1 g at the ±2 g range");
}

#[test]
fn the_hybrid_burst_jumps_from_the_accel_block_to_the_mag_block() {
    // §14.2: with hyb_autoinc_mode set, ONE 13-byte transaction from STATUS
    // carries the accelerometer AND the magnetometer. `auto_increment_map` is
    // the primitive; this is the transcript that proves it lands on 0x33 rather
    // than walking into the reserved space at 0x07.
    let steps = script([enable_hybrid(), read_reg(0x00, 13)]);
    let bytes = assert_parity("hybrid burst", [0.25, -0.5, 1.0], &steps);
    assert_eq!(bytes.len(), 13);
    assert_eq!(bytes[0], 0xFF, "STATUS");
    assert_eq!(word(bytes[1], bytes[2]), 1024, "+0.25 g on X");
    assert_eq!(word(bytes[3], bytes[4]), -2048, "−0.5 g on Y");
    assert_eq!(word(bytes[5], bytes[6]), ONE_G_LJ, "+1 g on Z");
    // The jump: byte 7 is M_OUT_X_MSB, not the reserved byte at 0x07.
    assert_eq!(word(bytes[7], bytes[8]), -2400, "M_OUT_X after the jump");
    assert_eq!(word(bytes[9], bytes[10]), 2400, "M_OUT_Y");
    assert_eq!(word(bytes[11], bytes[12]), 1200, "M_OUT_Z");
    // The negative control: an unmapped-space walk would have read 0x00 here.
    assert_ne!(&bytes[7..13], &[0u8; 6], "the jump landed nowhere");
}

#[test]
fn the_reset_bit_is_gone_by_the_time_firmware_reads_it_back() {
    // §6.6: CTRL_REG2.RST (D6) is a momentary "go" bit — the device has already
    // reset by the time the register can be read. The model masked it out of
    // the store; the descriptor says `self_clearing: 0x40`.
    let steps = script([write_reg(0x2B, &[0x7F]), read_reg(0x2B, 1)]);
    let bytes = assert_parity("RST", [0.0, 0.0, 1.0], &steps);
    assert_eq!(bytes, vec![0x3F], "every bit but RST survived the write");
}

#[test]
fn a_block_write_walks_the_pointer_exactly_as_a_block_read_does() {
    // One transaction configures the accelerometer range and then the
    // magnetometer registers it runs into.
    let steps = script([
        write_reg(0x5B, &[0x1F, 0x20]),
        read_reg(0x5B, 2),
        set_fs(2),
        read_reg(0x0E, 1),
    ]);
    let bytes = assert_parity("block write", [1.0, 0.0, 0.0], &steps);
    assert_eq!(bytes, vec![0x1F, 0x20, 0x02], "both mag regs and the FS");
}

// ─── deliberately different ────────────────────────────────────────────────

#[test]
fn the_part_no_longer_invents_a_pose_nothing_drove() {
    // THE deliberate change. The hand model advanced an internal phase on every
    // burst read of OUT_X_MSB and recomputed a fixed triangle walk from it, so
    // an UNDRIVEN part reported motion — and reported a DIFFERENT pose on every
    // read. A twin reports what the world does to it; a scene invents the world.
    //
    // (The engine's own `updates:` + `read_complete` trigger already fires on
    // burst-read completion, so the "advance on every burst read" half needed no
    // new primitive. What it cannot do is choose between an invented pose and a
    // driven one — the model's `manual` latch — which is a state machine.)
    let steps = script([read_reg(0x01, 6), read_reg(0x01, 6)]);
    let (old, new) = both_untouched(&steps);

    assert_ne!(&old[0..6], &old[6..12], "the model moved between two reads");
    assert_eq!(&new[0..6], &new[6..12], "the descriptor holds still");
    assert_eq!(
        &new[0..6],
        &[0x00, 0x00, 0x00, 0x00, 0x10, 0x00],
        "…at the declared flat pose: nothing on X or Y, +1 g on Z"
    );
    assert_ne!(old, new, "this difference is the point of the change");

    // And a DRIVEN part is identical in both, which is the case firmware and
    // the playground sliders actually exercise.
    let driven = assert_parity("driven", [0.5, -0.5, 1.0], &steps);
    assert_eq!(
        &driven[0..6],
        &driven[6..12],
        "a driven value sticks in both"
    );
}

#[test]
fn the_hybrid_jump_is_unconditional_instead_of_reading_the_enable_bit() {
    // The model read `M_CTRL_REG2.hyb_autoinc_mode` on every byte;
    // `auto_increment_map` is a map, and "this map applies only while that bit
    // is set" is a state machine. The observable difference is narrow and
    // stated here so it is a line someone wrote: only a read that runs PAST
    // 0x06 WITHOUT having enabled hybrid mode sees it.
    let steps = script([read_reg(0x00, 13)]); // no enable_hybrid()
    let (old, new) = both([0.25, -0.5, 1.0], &steps);
    assert_eq!(&old[0..7], &new[0..7], "STATUS and the accel block agree");
    assert_eq!(
        &old[7..13],
        &[0u8; 6],
        "the model walked into reserved space"
    );
    assert_eq!(
        word(new[7], new[8]),
        -2400,
        "the descriptor jumps to the mag"
    );
    assert_ne!(old, new, "this difference is the point of the change");

    // Both of the bursts a real driver issues are unaffected: the 7-byte
    // non-hybrid read stops at 0x06, and the 13-byte hybrid read enables the
    // jump first.
    let seven = script([read_reg(0x00, 7)]);
    let (old, new) = both([0.25, -0.5, 1.0], &seven);
    assert_eq!(old, new, "the 7-byte non-hybrid burst is untouched");
    let thirteen = script([enable_hybrid(), read_reg(0x00, 13)]);
    let (old, new) = both([0.25, -0.5, 1.0], &thirteen);
    assert_eq!(old, new, "the 13-byte hybrid burst is untouched");
}

#[test]
fn the_magnetometer_is_a_constant_field_instead_of_a_swept_one() {
    // There is no magnetometer stimulus channel to drive, and the model swept
    // the mag with the same invented walk the accelerometer had. The constants
    // are that walk's power-on sample, so the number is traceable rather than
    // invented twice.
    let steps = script([read_reg(0x33, 6), read_reg(0x33, 6)]);
    let (_, new) = both_untouched(&steps);
    assert_eq!(&new[0..6], &new[6..12], "the field does not move");
    assert_eq!(word(new[0], new[1]), -2400);
    assert_eq!(word(new[2], new[3]), 2400);
    assert_eq!(word(new[4], new[5]), 1200);
}

#[test]
fn the_low_two_bits_are_always_zero_where_the_model_let_data_into_them() {
    // §6.1 makes the accelerometer output a 14-bit count in the TOP 14 bits of
    // a 16-bit word, so the low two bits read 0 on silicon, always. The hand
    // model rounded the acceleration straight into the 16-bit word — its own
    // module comment said "left-justified" while its arithmetic quantised at
    // four times the converter's resolution — so those two bits carried data
    // the part never drives. The descriptor rounds to 14 bits and then shifts.
    //
    // The difference is never more than three LJ counts, i.e. less than one
    // count of the real converter.
    let mut differed = 0usize;
    let mut worst = 0u16;
    for milli in (-2000i32..=2000).step_by(3) {
        let g = f64::from(milli) / 1000.0;
        let steps = script([read_reg(0x01, 2)]);
        let (old, new) = both([g, 0.0, 0.0], &steps);
        assert_eq!(new[1] & 0x03, 0, "{g} g: the descriptor drove a low bit");
        let (o, n) = (word(old[0], old[1]), word(new[0], new[1]));
        worst = worst.max(o.abs_diff(n));
        if old != new {
            differed += 1;
            assert_ne!(old[1] & 0x03, 0, "{g} g: the model drove a low bit");
        }
    }
    assert!(
        worst <= 3,
        "no reading may move by a whole 14-bit count: {worst}"
    );
    assert!(
        differed > 500,
        "only {differed} of 1334 points differed at all"
    );

    // The named vector: −0.99658203125 g is 1020.5 counts of a 14-bit field.
    let steps = script([read_reg(0x01, 2)]);
    let (old, new) = both([-0.996_582_031_25, 0.0, 0.0], &steps);
    assert_eq!(old, vec![0xF0, 0x0E], "the model drove bits 1:0");
    assert_eq!(
        new,
        vec![0xF0, 0x0C],
        "the descriptor rounds to 14 bits first"
    );
}

#[test]
fn a_range_change_rescales_the_reading_instead_of_leaving_it_stale() {
    // A second deliberate change, found by the sweep above. The hand model
    // converted the acceleration to counts inside `set_input`, so a driver that
    // changed XYZ_DATA_CFG.FS afterwards kept reading the count computed at the
    // OLD range — a reading whose scale no longer matched the register that
    // says what the scale is. The descriptor converts at READ time, which is
    // when the part does.
    let steps = script([
        drive([1.0, 0.0, 1.0]),
        read_reg(0x01, 2),
        set_fs(2), // ±8 g: a quarter of the counts per g
        read_reg(0x01, 2),
    ]);
    let (old, new) = both_scripted(&steps);
    assert_eq!(&old[0..2], &new[0..2], "before the range change they agree");
    assert_eq!(word(new[0], new[1]), ONE_G_LJ, "+1 g at ±2 g");
    assert_eq!(
        word(old[2], old[3]),
        ONE_G_LJ,
        "the model kept the ±2 g count after moving to ±8 g"
    );
    assert_eq!(
        word(new[2], new[3]),
        ONE_G_LJ / 4,
        "the descriptor re-scales: the same 1 g is a quarter of the counts"
    );
    assert_ne!(old, new, "this difference is the point of the change");
}
