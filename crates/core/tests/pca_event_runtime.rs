use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::servo::Servo;
use std::path::PathBuf;

fn transaction(bus: &mut SystemBus, reg: u8, bytes: &[u8]) {
    let dev = &mut bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "i2c1")
        .unwrap()
        .dev;
    dev.write(0x15, 0).unwrap();
    dev.write(0x00, 1).unwrap();
    dev.write(0x01, 1).unwrap();
    for _ in 0..10 {
        dev.tick();
    }
    dev.write(0x10, 0x82).unwrap();
    for _ in 0..40 {
        dev.tick();
    }
    for byte in std::iter::once(&reg).chain(bytes.iter()) {
        dev.write(0x10, *byte).unwrap();
        for _ in 0..20 {
            dev.tick();
        }
    }
    dev.write(0x01, 2).unwrap();
    for _ in 0..10 {
        dev.tick();
    }
}

#[test]
fn servo_command_is_applied_at_i2c_stop_without_inspection() {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str(include_str!("pca-event-six-f401.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    transaction(&mut bus, 0, &[0x10]);
    transaction(&mut bus, 0xfe, &[121]);
    transaction(&mut bus, 0, &[0x20]);
    transaction(&mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    let servo = bus
        .observed_of::<Servo>()
        .find(|s| s.id() == "servo0")
        .unwrap();
    assert!(
        servo.is_commanded(),
        "servo must receive the controller output without inspection"
    );
    let angle = servo.angle_degrees();
    assert!(
        angle > 80.0 && angle < 110.0,
        "unexpected servo angle {angle}"
    );
    transaction(&mut bus, 0x06, &[0, 0, 0, 0x10]);
    let servo = bus
        .observed_of::<Servo>()
        .find(|s| s.id() == "servo0")
        .unwrap();
    assert!(servo.is_commanded());
    assert_eq!(
        servo.angle_degrees(),
        angle,
        "full-off must retain the last pose"
    );
}

#[test]
fn static_and_unmodeled_pca_outputs_do_not_produce_servo_pulses() {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str(include_str!("pca-event-six-f401.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    transaction(&mut bus, 0, &[0x10]);
    transaction(&mut bus, 0xfe, &[121]);
    transaction(&mut bus, 0, &[0x20]);
    transaction(&mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    assert!(bus.pwm_channel_pulse_us(0x41, 0).is_some());
    transaction(&mut bus, 0x06, &[0, 0x10, 0x33, 0x01]);
    assert_eq!(
        bus.pwm_channel_pulse_us(0x41, 0),
        None,
        "full on is a static level"
    );
    transaction(&mut bus, 0x06, &[1, 0, 1, 0]);
    assert_eq!(
        bus.pwm_channel_pulse_us(0x41, 0),
        None,
        "equal counts are static"
    );
    transaction(&mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    transaction(&mut bus, 1, &[0x14]);
    assert_eq!(
        bus.pwm_channel_pulse_us(0x41, 0),
        None,
        "inverted output is not modeled"
    );
    transaction(&mut bus, 1, &[0x04]);
    transaction(&mut bus, 0, &[0x60]);
    assert_eq!(
        bus.pwm_channel_pulse_us(0x41, 0),
        None,
        "external clock frequency is unknown"
    );
    transaction(&mut bus, 0, &[0x20]);
    assert_eq!(
        bus.pwm_channel_pulse_us(0x41, 0),
        None,
        "external clock remains selected until reset"
    );
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    transaction(&mut bus, 0, &[0x10]);
    transaction(&mut bus, 0xfe, &[121]);
    transaction(&mut bus, 0, &[0x20]);
    transaction(&mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    assert!(bus.pwm_channel_pulse_us(0x41, 0).is_some());
    transaction(&mut bus, 0xfa, &[0, 0, 0, 0]);
    assert_eq!(
        bus.pwm_channel_pulse_us(0x41, 0),
        None,
        "global channel writes are not modeled"
    );
}

#[test]
fn separate_channel_commands_survive_full_off_without_reading_servo_state() {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str(include_str!("pca-event-six-f401.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    transaction(&mut bus, 0, &[0x10]);
    transaction(&mut bus, 0xfe, &[121]);
    transaction(&mut bus, 0, &[0x20]);
    // Both channels are commanded and disabled before any servo state read.
    transaction(&mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    transaction(&mut bus, 0x0a, &[0, 0, 0x99, 0x01]);
    transaction(&mut bus, 0x06, &[0, 0, 0, 0x10]);
    transaction(&mut bus, 0x0a, &[0, 0, 0, 0x10]);
    let servos: Vec<_> = bus.observed_of::<Servo>().collect();
    let servo0 = servos.iter().find(|s| s.id() == "servo0").unwrap();
    let servo1 = servos.iter().find(|s| s.id() == "servo1").unwrap();
    assert!(servo0.is_commanded(), "channel 0 pulse was lost");
    assert!(servo1.is_commanded(), "channel 1 pulse was lost");
    assert!(servo1.angle_degrees() > servo0.angle_degrees());
}
