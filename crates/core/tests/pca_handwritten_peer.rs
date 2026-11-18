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
    dev.write(0x10, 0x80).unwrap();
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
fn direct_pca_servo_works_with_handwritten_i2c_peer() {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    let manifest:SystemManifest=serde_yaml::from_str(r#"
name: direct-pca-unrelated-mux
chip: inline
external_devices:
  - { id: direct, type: pca9685, connection: i2c1, config: { i2c_address: 0x40 } }
  - { id: sensor, type: max30102, connection: i2c1, config: { i2c_address: 0x57 } }
  - { id: servo0, type: servo, connection: i2c1, config: { model: sg90, source_address: 64, source_channel: 0 } }
"#).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    transaction(&mut bus, 0, &[0x10]);
    transaction(&mut bus, 0xfe, &[121]);
    transaction(&mut bus, 0, &[0x20]);
    transaction(&mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    transaction(&mut bus, 0x06, &[0, 0, 0, 0x10]);
    let servo = bus
        .observed_of::<Servo>()
        .find(|s| s.id() == "servo0")
        .unwrap();
    assert!(
        servo.is_commanded() && servo.angle_degrees() > 80.0,
        "direct source must actuate without inspection beside handwritten I2C peer"
    );
    bus.sync_legacy_pwm_servos();
    assert!(
        bus.observed_of::<Servo>()
            .find(|s| s.id() == "servo0")
            .unwrap()
            .angle_degrees()
            > 80.0,
        "WASM reconciliation must preserve held pose"
    );
}
