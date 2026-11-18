use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
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
    transaction(&mut bus, 1, &[0x00]);
    assert_eq!(
        bus.pwm_channel_pulse_us(0x41, 0),
        None,
        "open-drain pull-up is not modeled"
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
fn awake_prescale_write_cannot_produce_a_guessed_servo_pulse() {
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
    assert!(
        bus.pwm_channel_pulse_us(0x41, 0).is_some(),
        "legal sleep-mode prescale"
    );
    transaction(&mut bus, 0xfe, &[80]);
    assert_eq!(
        bus.pwm_channel_pulse_us(0x41, 0),
        None,
        "awake prescale writes are not modeled"
    );
    transaction(&mut bus, 0, &[0x10]);
    transaction(&mut bus, 0xfe, &[121]);
    transaction(&mut bus, 0, &[0x20]);
    assert!(
        bus.pwm_channel_pulse_us(0x41, 0).is_some(),
        "a valid sleep-mode write restores known prescale"
    );
}

#[test]
fn out_of_range_prescale_cannot_produce_a_guessed_servo_pulse() {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str(include_str!("pca-event-six-f401.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    transaction(&mut bus, 0, &[0x10]);
    transaction(&mut bus, 0xfe, &[2]);
    transaction(&mut bus, 0, &[0x20]);
    transaction(&mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    assert_eq!(
        bus.pwm_channel_pulse_us(0x41, 0),
        None,
        "PCA prescale below 3 is invalid"
    );
}
