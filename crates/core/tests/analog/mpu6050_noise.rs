// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Seeded per-channel noise on the MPU6050's accelerometer reads.
//!
//! The part is a `configs/devices/mpu6050.yaml` descriptor; the sigma that used
//! to be the hand-written kit's `with_noise_sigma` scalar is now the
//! `noise_sigma` **config key**, reaching all six motion axes through
//! `InputSpec::noise_sigma_key`. These tests drive the same three claims they
//! always did — noise MOVES the reads, the same seed REPLAYS bit-identically,
//! and sigma 0 is byte-identical to no noise at all — through that path.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::sim_input::SimInput;

/// The six motion axes the `noise_sigma` config key covers (`temp` is not one:
/// the documented sigma is in g and °/s).
const MOTION_AXES: &[&str] = &["ax", "ay", "az", "gx", "gy", "gz"];

fn mpu6050(sigma: f64) -> GenericI2cDevice {
    let mut dev = GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("mpu6050").expect("mpu6050 is embedded"),
        0x68,
    )
    .expect("mpu6050.yaml builds");
    // What `DeclarativeI2cKit::attach` does when the placement sets
    // `config: { noise_sigma: … }`, done by hand so this test needs no bus.
    for axis in MOTION_AXES {
        dev.set_channel_noise_sigma(axis, sigma);
    }
    dev
}

fn read_reg(dev: &mut GenericI2cDevice, reg: u8) -> u8 {
    dev.start();
    dev.write(reg);
    dev.start();
    let v = dev.read();
    dev.stop();
    v
}

#[test]
fn noise_moves_accel_reads_and_replays() {
    let mut a = mpu6050(0.02);
    let mut b = mpu6050(0.02);
    for d in [&mut a, &mut b] {
        SimInput::set_component_id(d, "imu".into());
        SimInput::set_input(d, "ax", 1.0).unwrap();
    }
    let ra: Vec<u8> = (0..8).map(|_| read_reg(&mut a, 0x3B)).collect();
    let rb: Vec<u8> = (0..8).map(|_| read_reg(&mut b, 0x3B)).collect();
    // 1 g at ±2g = 16384 counts; σ = 0.02 g ≈ 328 counts → MSB must move sometimes.
    let ideal_msb = (16384u16 >> 8) as u8; // 0x40
    assert!(
        ra.iter().any(|&r| r != ideal_msb),
        "noise never moved the MSB: {ra:?}"
    );
    assert_eq!(ra, rb, "same seed must replay bit-identically");
}

#[test]
fn no_noise_config_is_byte_identical_to_before() {
    let mut dev = mpu6050(0.0);
    SimInput::set_input(&mut dev, "ax", 1.0).unwrap();
    for _ in 0..4 {
        assert_eq!(read_reg(&mut dev, 0x3B), 0x40);
        assert_eq!(read_reg(&mut dev, 0x3C), 0x00);
    }
}

#[test]
fn noise_sigma_zero_is_byte_identical() {
    let mut dev = mpu6050(0.0);
    SimInput::set_component_id(&mut dev, "imu".into());
    SimInput::set_input(&mut dev, "ax", 1.0).unwrap();
    for _ in 0..4 {
        assert_eq!(read_reg(&mut dev, 0x3B), 0x40);
    }
}

/// The sigma reaches EVERY motion axis, not just the one the tests above read.
/// That is the whole point of `noise_sigma_key`: one config value, a channel
/// set. A per-channel wiring that only reached `ax` would pass every test
/// above.
#[test]
fn the_sigma_reaches_all_six_motion_axes() {
    // ACCEL_XOUT_H … GYRO_ZOUT_H, one high byte per axis.
    for (axis, reg, value) in [
        ("ax", 0x3Bu8, 1.0),
        ("ay", 0x3D, 1.0),
        ("az", 0x3F, 1.0),
        ("gx", 0x43, 200.0),
        ("gy", 0x45, 200.0),
        ("gz", 0x47, 200.0),
    ] {
        let mut dev = mpu6050(0.5);
        SimInput::set_component_id(&mut dev, format!("imu-{axis}"));
        SimInput::set_input(&mut dev, axis, value).unwrap();
        // The FULL 16-bit word, not its high byte: σ = 0.5 unit is well under
        // 256 counts on several of these axes, so a high-byte-only check would
        // be seed-dependent — which is how a test that "passes" tells you
        // nothing about the five axes it did not happen to move.
        let quiet = {
            let mut d = mpu6050(0.0);
            SimInput::set_input(&mut d, axis, value).unwrap();
            read_word(&mut d, reg)
        };
        let observed: Vec<u16> = (0..16).map(|_| read_word(&mut dev, reg)).collect();
        assert!(
            observed.iter().any(|&r| r != quiet),
            "{axis}: the sigma never reached this axis (all reads {quiet:#06X})"
        );
    }
}

/// A full big-endian register word (`_H` then `_L`) out of one transaction.
fn read_word(dev: &mut GenericI2cDevice, reg: u8) -> u16 {
    dev.start();
    dev.write(reg);
    dev.start();
    let w = u16::from_be_bytes([dev.read(), dev.read()]);
    dev.stop();
    w
}
