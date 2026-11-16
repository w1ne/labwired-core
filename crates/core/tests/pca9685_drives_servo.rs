// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A PCA9685 output channel drives a servo part through that servo's
//! calibration. Nothing here is robot specific: any servo declared with
//! `source_address` / `source_channel` follows the pulse the firmware wrote to
//! that PWM controller channel.

use labwired_core::peripherals::components::servo::{Servo, ServoCal};
use labwired_core::peripherals::components::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;

fn pca() -> GenericI2cDevice {
    let yaml = labwired_config::embedded_device_yaml("pca9685").expect("pca9685 descriptor");
    GenericI2cDevice::from_yaml(yaml, 0).expect("valid descriptor")
}

fn write_reg(d: &mut dyn I2cDevice, reg: u8, bytes: &[u8]) {
    d.start();
    d.write(reg);
    for b in bytes {
        d.write(*b);
    }
    d.stop();
}

/// Same sequence the AXIOM firmware uses: sleep, PRE_SCALE 121 (50 Hz), wake with
/// auto-increment, then four bytes ON_L ON_H OFF_L OFF_H into a channel.
fn init_50hz(d: &mut dyn I2cDevice) {
    write_reg(d, 0x00, &[0x10]);
    write_reg(d, 0xFE, &[121]);
    write_reg(d, 0x00, &[0x20]);
}

#[test]
fn unwritten_channel_has_no_pulse() {
    let mut d = pca();
    init_50hz(&mut d);
    assert_eq!(d.pwm_channel_pulse_us(3), None);
}

#[test]
fn off_count_becomes_pulse_width_from_prescale() {
    let mut d = pca();
    init_50hz(&mut d);
    // OFF = 307 counts, ON = 0, on channel 3 (LED3 registers start at 0x06 + 4*3).
    write_reg(
        &mut d,
        0x06 + 4 * 3,
        &[0, 0, (307 & 0xFF) as u8, (307 >> 8) as u8],
    );
    let us = d.pwm_channel_pulse_us(3).expect("channel 3 was written");
    // period = 4096 * (121 + 1) / 25 MHz = 19988 us; 307 / 4096 of it.
    assert!((us - 1498.0).abs() < 1.0, "pulse {us}");
    assert_eq!(d.pwm_channel_pulse_us(4), None);
}

#[test]
fn on_count_is_subtracted_and_full_off_is_silent() {
    let mut d = pca();
    init_50hz(&mut d);
    write_reg(
        &mut d,
        0x06,
        &[100, 0, (407 & 0xFF) as u8, (407 >> 8) as u8],
    );
    let us = d.pwm_channel_pulse_us(0).unwrap();
    assert!((us - 307.0 / 4096.0 * 19988.0).abs() < 1.0, "pulse {us}");
    // OFF_H bit 4 = full off: the output is held low, whatever the count says.
    write_reg(&mut d, 0x06, &[0, 0, 0x33, 0x11]);
    assert_eq!(d.pwm_channel_pulse_us(0), None);
}

#[test]
fn a_sourced_servo_follows_the_controller_pulse_through_its_calibration() {
    let servo = Servo::with_source("shoulder", ServoCal::mg995(), 0x40, 3);
    assert_eq!(servo.source(), Some((0x40, 3)));
    assert!(!servo.is_commanded());
    servo.apply_pulse_us(1498.0);
    assert!(servo.is_commanded());
    let cal = ServoCal::mg995();
    let expected = cal.min_angle
        + (cal.max_angle - cal.min_angle) * (1498.0 - cal.min_us) / (cal.max_us - cal.min_us);
    assert!(
        (servo.angle_degrees() - expected).abs() < 0.5,
        "angle {}",
        servo.angle_degrees()
    );
    assert!((servo.pulse_us() - 1498.0).abs() < 1e-3);
}

/// A manifest can declare the wiring: the servo attaches without a GPIO pin and
/// stays at rest until the firmware has written its channel.
#[test]
fn manifest_declares_a_servo_on_a_controller_channel() {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    use std::path::PathBuf;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: pca-servo-rig
chip: "../chips/esp32.yaml"
external_devices:
  - id: pwm1
    type: pca9685
    connection: i2c0
    config: { address: 0x40 }
  - id: elbow
    type: servo
    connection: i2c0
    config: { model: mg995, source_address: 0x40, source_channel: 3 }
"#,
    )
    .unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).expect("bus builds");
    let servos: Vec<_> = bus
        .observed_of::<labwired_core::peripherals::components::servo::Servo>()
        .collect();
    assert_eq!(servos.len(), 1);
    assert_eq!(servos[0].source(), Some((0x40, 3)));
    bus.sync_pwm_sourced_servos();
    assert!(!servos[0].is_commanded(), "nothing written yet");
    assert_eq!(bus.pwm_channel_pulse_us(0x40, 3), None);
}
