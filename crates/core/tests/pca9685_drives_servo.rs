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

#[test]
fn servo_source_is_scoped_to_its_declared_i2c_bus() {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    use std::path::PathBuf;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/stm32f103.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: same-address-controllers
chip: "../chips/stm32f103.yaml"
external_devices:
  - id: pwm1
    type: pca9685
    connection: i2c1
    config: { address: 0x40 }
  - id: pwm2
    type: pca9685
    connection: i2c2
    config: { address: 0x40 }
  - id: elbow
    type: servo
    connection: i2c2
    config: { model: mg995, source_address: 64, source_channel: 0 }
"#,
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    for (bus_name, off) in [("i2c1", 205u16), ("i2c2", 409u16)] {
        let mut slaves = bus.take_i2c_slaves(bus_name);
        assert_eq!(slaves.len(), 1);
        init_50hz(slaves[0].as_mut());
        write_reg(
            slaves[0].as_mut(),
            0x06,
            &[0, 0, off as u8, (off >> 8) as u8],
        );
        for slave in slaves {
            bus.attach_i2c_slave(bus_name, slave).unwrap();
        }
    }
    bus.sync_pwm_sourced_servos();
    let servo = bus.observed_of::<Servo>().next().unwrap();
    assert!(
        (servo.pulse_us() - 1996.0).abs() < 2.0,
        "i2c2 pulse was {}",
        servo.pulse_us()
    );
}

#[test]
fn supply_gate_forwards_pwm_query_only_when_powered() {
    use labwired_core::peripherals::components::supply_domain::{GatedI2cDevice, SupplyGate};
    let make_controller = || {
        let mut d = pca();
        init_50hz(&mut d);
        write_reg(&mut d, 0x06, &[0, 0, 0x33, 0x01]);
        d
    };
    let powered = GatedI2cDevice::new(Box::new(make_controller()), SupplyGate::legacy(true));
    let unpowered = GatedI2cDevice::new(Box::new(make_controller()), SupplyGate::legacy(false));
    assert!((powered.pwm_channel_pulse_us(0).unwrap() - 1498.0).abs() < 1.0);
    assert_eq!(unpowered.pwm_channel_pulse_us(0), None);
}

#[test]
fn mux_capability_survives_supply_wrapping_without_misclassifying_slaves() {
    use labwired_core::peripherals::components::supply_domain::{GatedI2cDevice, SupplyGate};
    use labwired_core::peripherals::components::tca9548a::Tca9548a;

    assert!(!pca().is_i2c_mux());
    for powered in [false, true] {
        let gated = GatedI2cDevice::new(Box::new(Tca9548a::new(0x70)), SupplyGate::legacy(powered));
        assert!(gated.is_i2c_mux(), "power state must not erase topology");
    }
}

fn assert_ambiguous_sources_are_not_applied(devices: &str) {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    use std::path::PathBuf;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/stm32f103.yaml")).unwrap();
    let yaml = format!(
        r#"
name: ambiguous-servo-source
chip: "../chips/stm32f103.yaml"
external_devices:
{devices}
  - id: elbow
    type: servo
    connection: i2c1
    config: {{ model: mg995, source_address: 64, source_channel: 0 }}
"#
    );
    let manifest: SystemManifest = serde_yaml::from_str(&yaml).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let mut slaves = bus.take_i2c_slaves("i2c1");
    for slave in &mut slaves {
        if slave.address() == 0x40 {
            init_50hz(slave.as_mut());
            write_reg(slave.as_mut(), 0x06, &[0, 0, 0x33, 0x01]);
        } else if slave.address() == 0x70 {
            // Enable channel 0 and program the actual downstream PCA via the mux.
            slave.select_address(0x70);
            slave.start();
            slave.write(1);
            slave.stop();
            if devices.contains("id: inner") {
                slave.select_address(0x71);
                slave.start();
                slave.write(1);
                slave.stop();
            }
            slave.select_address(0x40);
            init_50hz(slave.as_mut());
            write_reg(slave.as_mut(), 0x06, &[0, 0, 0x33, 0x01]);
            if devices.contains("id: inner") {
                // Read back the actual nested device over both enabled muxes.
                // This proves it received the pulse command without reaching
                // into concrete mux models through runtime downcasts.
                assert!(slave.claims_address(0x40));
                slave.select_address(0x40);
                slave.start();
                slave.write(0x06);
                slave.start();
                let regs = [slave.read(), slave.read(), slave.read(), slave.read()];
                slave.stop();
                assert_eq!(regs, [0, 0, 0x33, 0x01]);
            }
        }
    }
    for slave in slaves {
        bus.attach_i2c_slave("i2c1", slave).unwrap();
    }
    if devices.contains("type: tca9548a") && !devices.contains("id: inner") {
        let mut child_pulse = None;
        bus.for_each_attached_device(&mut |_, d| {
            if d.mux_address == Some(0x70) && d.address == Some(0x40) {
                child_pulse = d.evidence.and_then(|e| e.pwm_channel_pulse_us(0));
            }
        });
        assert!((child_pulse.expect("programmed mux child") - 1498.0).abs() < 1.0);
    }
    bus.sync_pwm_sourced_servos();
    let servo = bus.observed_of::<Servo>().next().unwrap();
    assert!(
        !servo.is_commanded(),
        "ambiguous or routed sources must not command servo"
    );
}

#[test]
fn direct_and_muxed_same_address_are_ambiguous() {
    assert_ambiguous_sources_are_not_applied(
        r#"  - { id: direct, type: pca9685, connection: i2c1, config: { address: 0x40 } }
  - { id: mux, type: tca9548a, connection: i2c1, config: { i2c_address: 0x70 } }
  - { id: child, type: pca9685, connection: mux, channel: 0, config: { address: 0x40 } }"#,
    );
}

#[test]
fn mux_only_source_is_not_assumed() {
    assert_ambiguous_sources_are_not_applied(
        r#"  - { id: mux, type: tca9548a, connection: i2c1, config: { i2c_address: 0x70 } }
  - { id: child, type: pca9685, connection: mux, channel: 0, config: { address: 0x40 } }"#,
    );
}

#[test]
fn duplicate_direct_same_address_is_ambiguous() {
    assert_ambiguous_sources_are_not_applied(
        r#"  - { id: first, type: pca9685, connection: i2c1, config: { address: 0x40 } }
  - { id: second, type: pca9685, connection: i2c1, config: { address: 0x40 } }"#,
    );
}

#[test]
fn direct_and_nested_mux_same_address_are_ambiguous() {
    assert_ambiguous_sources_are_not_applied(
        r#"  - { id: direct, type: pca9685, connection: i2c1, config: { address: 0x40 } }
  - { id: mux, type: tca9548a, connection: i2c1, config: { i2c_address: 0x70 } }
  - { id: inner, type: tca9548a, connection: mux, channel: 0, config: { i2c_address: 0x71 } }
  - { id: child, type: pca9685, connection: inner, channel: 0, config: { address: 0x40 } }"#,
    );
}

#[test]
fn unrelated_mux_child_at_switch_address_does_not_block_direct_source() {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    use std::path::PathBuf;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/stm32f103.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(r#"
name: direct-and-unrelated-child
chip: "../chips/stm32f103.yaml"
external_devices:
  - { id: direct, type: pca9685, connection: i2c1, config: { address: 0x40 } }
  - { id: mux, type: tca9548a, connection: i2c1, config: { i2c_address: 0x71 } }
  - { id: sensor, type: tmp102, connection: mux, channel: 0, config: { i2c_address: 0x70 } }
  - { id: elbow, type: servo, connection: i2c1, config: { model: mg995, source_address: 64, source_channel: 0 } }
"#).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let mut slaves = bus.take_i2c_slaves("i2c1");
    for slave in &mut slaves {
        if slave.address() == 0x40 {
            init_50hz(slave.as_mut());
            write_reg(slave.as_mut(), 0x06, &[0, 0, 0x33, 0x01]);
        }
    }
    for slave in slaves {
        bus.attach_i2c_slave("i2c1", slave).unwrap();
    }
    bus.sync_pwm_sourced_servos();
    let servo = bus.observed_of::<Servo>().next().unwrap();
    assert!(
        (servo.pulse_us() - 1498.0).abs() < 1.0,
        "direct pulse {}",
        servo.pulse_us()
    );
}

/// Dynamic, active-low OE gates new commands. The angle remains the last
/// modeled command while disabled; it is not evidence of physical position.
#[test]
fn pca_servo_dynamic_oe_requires_a_driven_low_gpio() {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::{Bus, SystemBus};
    use std::path::PathBuf;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(r#"
name: pca-servo-oe
chip: "../chips/esp32c3.yaml"
external_devices:
  - { id: pwm1, type: pca9685, connection: i2c0, config: { address: 64 } }
  - { id: elbow, type: servo, connection: i2c0, config: { model: mg995, source_address: 64, source_channel: 0, source_oe_pin: GPIO4 } }
"#).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let mut slaves = bus.take_i2c_slaves("i2c0");
    init_50hz(slaves[0].as_mut());
    write_reg(slaves[0].as_mut(), 0x06, &[0, 0, 0x33, 0x01]);
    for slave in slaves {
        bus.attach_i2c_slave("i2c0", slave).unwrap();
    }
    let commanded = |bus: &SystemBus| bus.observed_of::<Servo>().next().unwrap().is_commanded();
    let angle = |bus: &SystemBus| bus.observed_of::<Servo>().next().unwrap().angle_degrees();
    // Input / unknown is not an enable, even if its output latch is low.
    bus.sync_pwm_sourced_servos();
    assert!(!commanded(&bus));
    const GPIO: u64 = 0x6000_4000;
    bus.write_u32(GPIO + 0x08, 1 << 4).unwrap(); // OUT_W1TS: high
    bus.write_u32(GPIO + 0x24, 1 << 4).unwrap(); // ENABLE_W1TS
    bus.sync_pwm_sourced_servos();
    assert!(!commanded(&bus));
    bus.write_u32(GPIO + 0x0c, 1 << 4).unwrap(); // OUT_W1TC: low enables
    bus.sync_pwm_sourced_servos();
    assert!(commanded(&bus));
    let previous = angle(&bus);
    bus.write_u32(GPIO + 0x08, 1 << 4).unwrap(); // disable output
    let mut slaves = bus.take_i2c_slaves("i2c0");
    write_reg(slaves[0].as_mut(), 0x06, &[0, 0, 0x80, 0x01]);
    for slave in slaves {
        bus.attach_i2c_slave("i2c0", slave).unwrap();
    }
    bus.sync_pwm_sourced_servos();
    assert_eq!(angle(&bus), previous);
    bus.write_u32(GPIO + 0x0c, 1 << 4).unwrap();
    bus.sync_pwm_sourced_servos();
    assert_ne!(angle(&bus), previous);
}
