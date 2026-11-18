use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::servo::Servo;
use std::path::PathBuf;
fn write_reg(d: &mut dyn labwired_core::peripherals::i2c::I2cDevice, reg: u8, bytes: &[u8]) {
    d.start();
    d.write(reg);
    for &b in bytes {
        d.write(b);
    }
    d.stop();
}
#[test]
fn c3_raw_access_is_reconciled_at_legacy_readback() {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/esp32c3.yaml"),
    )
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: c3-pca
chip: esp32c3
external_devices:
  - id: pca
    type: pca9685
    connection: i2c0
    route: { sda: GPIO5, scl: GPIO6 }
    config: { i2c_address: 0x41 }
  - id: servo0
    type: servo
    connection: i2c0
    config: { model: sg90, source_address: 65, source_channel: 0 }
"#,
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let host = bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "i2c0")
        .unwrap()
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c>()
        .unwrap();
    let d = host.attached_slaves_mut()[0].as_mut();
    write_reg(d, 0, &[0x10]);
    write_reg(d, 0xfe, &[121]);
    write_reg(d, 0, &[0x20]);
    write_reg(d, 0x06, &[0, 0, 0x33, 0x01]);
    assert!(!bus.observed_of::<Servo>().next().unwrap().is_commanded());
    bus.sync_legacy_pwm_servos();
    assert!(bus.observed_of::<Servo>().next().unwrap().angle_degrees() > 80.0);
}
