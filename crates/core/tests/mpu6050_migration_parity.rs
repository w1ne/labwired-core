// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! MPU6050: the declarative descriptor against the hand-written model it
//! replaces.
//!
//! `components/mpu6050.rs` is DELETED. The transcript that model produced over
//! the registers it got RIGHT is pinned here as golden bytes, captured by
//! running [`migration_script`] against it on the commit that removed it.
//!
//! Held identical:
//!   * WHO_AM_I 0x68 and PWR_MGMT_1's 0x40 power-on (SLEEP set), and the write
//!     that wakes it;
//!   * GYRO_CONFIG / ACCEL_CONFIG storage;
//!   * the three ACCELEROMETER axes at 0x3B..0x40, at their power-on sample and
//!     driven, including the live AFS_SEL scale.
//!
//! ⚠️ DELIBERATELY DIFFERENT — and this is why the port matters. The old model
//! computed its data channel as `(reg - 0x3B) / 2` across the whole 0x3B..0x48
//! span. There is no TEMP_OUT in that arithmetic, so every gyro axis sat one
//! register pair too low:
//!
//! | address | datasheet   | old model    |
//! |---------|-------------|--------------|
//! | 0x41    | TEMP_OUT_H  | GYRO_XOUT_H  |
//! | 0x43    | GYRO_XOUT_H | GYRO_YOUT_H  |
//! | 0x45    | GYRO_YOUT_H | GYRO_ZOUT_H  |
//! | 0x47    | GYRO_ZOUT_H | GYRO_ZOUT_H  |
//!
//! Both the Adafruit and the jrowberg drivers read fourteen bytes from 0x3B in
//! one burst and slice ACCEL·TEMP·GYRO out of it, so six of every fourteen
//! bytes were wrong. `the_burst_read_is_accel_temp_gyro` is the assertion that
//! keeps the fix.
//!
//! Also new: the DATA_RDY interrupt reaches a pad, and the `noise_sigma`
//! config key now reaches all six motion axes through `noise_sigma_key`.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::sim_input::SimInput;

mod common;
use common::transcript::{read_reg, run_i2c, script, write_reg, Step};

const ADDR: u8 = 0x68;

fn declarative() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("mpu6050")
        .expect("mpu6050 descriptor is not embedded — check embedded_device_yaml");
    GenericI2cDevice::from_yaml(yaml, ADDR).expect("mpu6050.yaml does not build")
}

/// The conversation the deleted model was driven through, restricted to the
/// registers it modelled correctly — see the module note for the ones it did
/// not, which are asserted separately below.
fn migration_script() -> Vec<Step<'static>> {
    script([
        read_reg(0x75, 1),        // WHO_AM_I
        read_reg(0x6B, 1),        // PWR_MGMT_1 at power-on: SLEEP set
        write_reg(0x6B, &[0x00]), // wake
        read_reg(0x6B, 1),
        read_reg(0x1B, 2), // GYRO_CONFIG, ACCEL_CONFIG
        read_reg(0x3B, 6), // the power-on accelerometer sample
        vec![
            Step::Input("ax", 0.5),
            Step::Input("ay", -1.0),
            Step::Input("az", 0.25),
        ],
        read_reg(0x3B, 6),
        // AFS_SEL = 3 (±16 g, 2048 LSB/g): the SAME 0.5 g is a quarter of the
        // counts. The scale is read at read time, from the register.
        write_reg(0x1C, &[0x18]),
        vec![Step::Input("ax", 0.5)],
        read_reg(0x3B, 2),
    ])
}

/// ⚠️ GOLDEN — captured from `components/mpu6050.rs` on the commit that deleted
/// it, by running [`migration_script`] against it.
const MPU6050_GOLDEN: &[u8] = &[
    0x68, // WHO_AM_I
    0x40, // PWR_MGMT_1: SLEEP
    0x00, // …after the wake write
    0x00, 0x00, // GYRO_CONFIG, ACCEL_CONFIG
    0x01, 0x23, 0x04, 0x56, 0x40, 0x00, // the power-on sample
    0x20, 0x00, 0xC0, 0x00, 0x10, 0x00, // 0.5 g, -1 g, 0.25 g at 16384 LSB/g
    0x04, 0x00, // 0.5 g at 2048 LSB/g
];

#[test]
fn the_wire_transcript_is_byte_identical_to_the_deleted_model() {
    let transcript = run_i2c(&mut declarative(), &migration_script());
    assert_eq!(
        transcript.bytes,
        MPU6050_GOLDEN,
        "the declarative MPU6050 moved a byte the hand-written one did not.\n\
         got:\n{}\nexpected:\n{}\n\
         If a change here is deliberate, say which datasheet line justifies it \
         and re-bless with:\n  {}",
        transcript.render(),
        common::transcript::Transcript {
            bytes: MPU6050_GOLDEN.to_vec()
        }
        .render(),
        transcript.as_literal()
    );
}

// ─── the bug the port fixes ────────────────────────────────────────────────

/// The fourteen-byte burst every MPU6050 driver reads is
/// ACCEL(6) · TEMP(2) · GYRO(6). The old model had no TEMP_OUT and shifted the
/// whole gyro block down one register pair, so this is the assertion that
/// keeps the fix: every axis is posed to a DIFFERENT value and each has to land
/// at its datasheet address.
#[test]
fn the_burst_read_is_accel_temp_gyro() {
    let mut dev = declarative();
    for (k, v) in [
        ("ax", 1.0),
        ("ay", 2.0),
        ("az", 3.0),
        ("gx", 10.0),
        ("gy", 20.0),
        ("gz", 30.0),
        ("temp", 25.0),
    ] {
        dev.set_input(k, v).expect("channel");
    }
    dev.start();
    dev.write(0x3B);
    dev.start();
    let burst: Vec<u8> = (0..14).map(|_| dev.read()).collect();
    dev.stop();

    let w = |i: usize| i16::from_be_bytes([burst[i], burst[i + 1]]);
    assert_eq!(w(0), 16384, "ACCEL_XOUT at 0x3B");
    assert_eq!(w(2), 32767, "ACCEL_YOUT at 0x3D — 2 g saturates the word");
    assert_eq!(w(4), 32767, "ACCEL_ZOUT at 0x3F");
    // T_degC = TEMP_OUT/340 + 36.53  ⇒  25 °C = 25 × 340 − 12420.2 = -3920.
    assert_eq!(w(6), -3920, "TEMP_OUT at 0x41 — the old model had none");
    assert_eq!(w(8), 1310, "GYRO_XOUT at 0x43 (10 °/s × 131)");
    assert_eq!(w(10), 2620, "GYRO_YOUT at 0x45");
    assert_eq!(w(12), 3930, "GYRO_ZOUT at 0x47");
}

/// The old model clamped the engineering value to the full scale and then cast
/// to `i16`, so +2.0 g at AFS_SEL = 0 became 32768 — which as an `i16` is
/// −32768. A reading at positive full scale came back at negative full scale.
#[test]
fn full_scale_saturates_positive_rather_than_wrapping() {
    let mut dev = declarative();
    dev.set_input("ax", 2.0).expect("channel");
    assert_eq!(dev.register_word("ACCEL_XOUT"), Some(32767));
    dev.set_input("ax", -2.0).expect("channel");
    assert_eq!(dev.register_word("ACCEL_XOUT"), Some(-32768));
}

/// Both full-scale selects are live and read at READ time.
#[test]
fn both_full_scale_selects_follow_their_registers() {
    let mut dev = declarative();
    dev.set_input("ax", 1.0).expect("channel");
    dev.set_input("gx", 100.0).expect("channel");
    for (afs, counts) in [(0x00u8, 16384i64), (0x08, 8192), (0x10, 4096), (0x18, 2048)] {
        write(&mut dev, 0x1C, afs);
        assert_eq!(
            dev.register_word("ACCEL_XOUT"),
            Some(counts),
            "AFS {afs:#04X}"
        );
    }
    for (fs, counts) in [(0x00u8, 13100i64), (0x08, 6550), (0x10, 3280), (0x18, 1640)] {
        write(&mut dev, 0x1B, fs);
        assert_eq!(dev.register_word("GYRO_XOUT"), Some(counts), "FS {fs:#04X}");
    }
}

// ─── the Tier-2 half ───────────────────────────────────────────────────────

/// DATA_RDY end to end: the sample timer raises it, INT_ENABLE gates the pad,
/// and ANY read of INT_STATUS clears both.
#[test]
fn data_ready_raises_the_flag_and_the_pad_and_a_read_clears_both() {
    let mut dev = declarative();
    write(&mut dev, 0x38, 0x01); // INT_ENABLE.DATA_RDY_EN
    dev.advance_time_us(1_000);
    assert_eq!(read_byte(&mut dev, 0x3A), 0x01, "INT_STATUS.DATA_RDY");
    // That read cleared it — the datasheet's "cleared on any read".
    assert_eq!(read_byte(&mut dev, 0x3A), 0x00);

    // The pad: it rises on the sample and falls on the status read.
    let _ = dev.take_pin_drives();
    dev.advance_time_us(1_000);
    assert_eq!(dev.take_pin_drives(), vec![("INT".to_string(), true)]);
    let _ = read_byte(&mut dev, 0x3A);
    assert_eq!(dev.take_pin_drives(), vec![("INT".to_string(), false)]);
}

/// With DATA_RDY_EN clear the flag still latches but the pad never rises. A
/// model that drove the pad off the timer alone would interrupt a driver that
/// asked for no interrupts.
#[test]
fn an_unenabled_data_ready_never_reaches_the_pad() {
    let mut dev = declarative();
    for _ in 0..5 {
        dev.advance_time_us(1_000);
        assert!(
            dev.take_pin_drives().iter().all(|(_, level)| !*level),
            "the pad must stay low while INT_ENABLE.DATA_RDY_EN is clear"
        );
    }
    assert_eq!(read_byte(&mut dev, 0x3A), 0x01, "the flag still latches");
}

// ─── the config-keyed noise sigma ──────────────────────────────────────────

/// `noise_sigma_key` is the primitive that replaces the hand-written kit's
/// `with_noise_sigma` scalar: ONE `config:` value reaches all six motion axes.
/// Checked through the kit, because the kit is where a `config:` key is read.
#[test]
fn the_noise_sigma_config_key_reaches_every_motion_axis() {
    use labwired_config::DeviceDescriptor;
    let yaml = labwired_config::embedded_device_yaml("mpu6050").expect("embedded");
    let desc = DeviceDescriptor::from_yaml(yaml).expect("parses");
    let inputs = &desc.metadata.as_ref().expect("metadata").inputs;
    let keyed: Vec<&str> = inputs
        .iter()
        .filter(|i| i.noise_sigma_key.as_deref() == Some("noise_sigma"))
        .map(|i| i.key.as_str())
        .collect();
    assert_eq!(
        keyed,
        vec!["ax", "ay", "az", "gx", "gy", "gz"],
        "the hand-written kit's sigma was documented in g and °/s, so it covers \
         the six motion axes and not the die temperature"
    );

    // And it actually moves what the WIRE carries. `register_word` is the
    // noise-free inspection view (it is what the deleted model's `sample()`
    // returned), so the sigma has to be checked through a real read.
    let mut quiet = declarative();
    quiet.set_channel_noise_sigma("ax", 0.0);
    let clean: Vec<u8> = (0..8).flat_map(|_| word_at(&mut quiet, 0x3B)).collect();
    assert!(
        clean.chunks(2).all(|w| w == [0x01, 0x23]),
        "a sigma of 0 must leave the reads byte-identical: {clean:?}"
    );

    let mut noisy = declarative();
    noisy.set_component_id("imu-a".to_string());
    noisy.set_channel_noise_sigma("ax", 0.05);
    let observed: Vec<[u8; 2]> = (0..8).map(|_| word_at(&mut noisy, 0x3B)).collect();
    assert!(
        observed.iter().any(|w| *w != [0x01, 0x23]),
        "a sigma of 0.05 g must move the counts: {observed:?}"
    );

    // Two identical parts on one bus must diverge, which is what re-keying the
    // noise by component id is for.
    let mut other = declarative();
    other.set_component_id("imu-b".to_string());
    other.set_channel_noise_sigma("ax", 0.05);
    let second: Vec<[u8; 2]> = (0..8).map(|_| word_at(&mut other, 0x3B)).collect();
    assert_ne!(
        observed, second,
        "two IMUs with one sigma must not correlate"
    );
}

fn word_at(dev: &mut GenericI2cDevice, reg: u8) -> [u8; 2] {
    dev.start();
    dev.write(reg);
    dev.start();
    let w = [dev.read(), dev.read()];
    dev.stop();
    w
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
