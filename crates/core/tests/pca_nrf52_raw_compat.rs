use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, peripherals::components::servo::Servo};
use std::path::PathBuf;
fn make_bus() -> SystemBus {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/nrf52840.yaml"),
    )
    .unwrap();
    let manifest:SystemManifest=serde_yaml::from_str(r#"
name: nrf52-pca-servo
chip: nrf52840
external_devices:
 - {id: pca, type: pca9685, connection: i2c0, config: {i2c_address: 0x41}}
 - {id: servo0, type: servo, connection: i2c0, config: {model: sg90, source_address: 65, source_channel: 0}}
"#).unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}
fn write(d: &mut dyn labwired_core::peripherals::i2c::I2cDevice, r: u8, v: &[u8]) {
    d.start();
    d.write(r);
    for &b in v {
        d.write(b);
    }
    d.stop();
}
#[test]
fn escaped_nrf52_serial_view_remains_safe_and_legacy_readback_works() {
    let bus = make_bus();
    let host = bus
        .peripherals
        .iter()
        .find(|p| p.name == "i2c0")
        .unwrap()
        .dev
        .as_any()
        .unwrap()
        .downcast_ref::<labwired_core::peripherals::nrf52::serial_instance::Nrf52SerialInstance>()
        .unwrap();
    let escaped = host.attached_i2c_devices();
    {
        let mut d = escaped[0].borrow_mut();
        write(d.as_mut(), 0, &[0x10]);
        write(d.as_mut(), 0xfe, &[121]);
        write(d.as_mut(), 0, &[0x20]);
        write(d.as_mut(), 0x06, &[0, 0, 0x33, 0x01]);
    }
    assert!(!bus.observed_of::<Servo>().next().unwrap().is_commanded());
    bus.sync_legacy_pwm_servos();
    let before = bus.observed_of::<Servo>().next().unwrap().angle_degrees();
    assert!(before > 80.0);
    {
        let mut d = escaped[0].borrow_mut();
        write(d.as_mut(), 0x06, &[0, 0, 0x90, 0x01]);
    }
    assert_eq!(
        bus.observed_of::<Servo>().next().unwrap().angle_degrees(),
        before
    );
    bus.sync_legacy_pwm_servos();
    assert!(bus.observed_of::<Servo>().next().unwrap().angle_degrees() > before + 20.0);
}

#[test]
fn escaped_raw_view_replacement_cannot_command_declared_servo() {
    let bus = make_bus();
    let host = bus
        .peripherals
        .iter()
        .find(|p| p.name == "i2c0")
        .unwrap()
        .dev
        .as_any()
        .unwrap()
        .downcast_ref::<labwired_core::peripherals::nrf52::serial_instance::Nrf52SerialInstance>()
        .unwrap();
    let escaped = host.attached_i2c_devices();
    let mut replacement = labwired_core::peripherals::components::GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("pca9685").unwrap(),
        0x41,
    )
    .unwrap();
    write(&mut replacement, 0, &[0x10]);
    write(&mut replacement, 0xfe, &[121]);
    write(&mut replacement, 0, &[0x20]);
    write(&mut replacement, 0x06, &[0, 0, 0x33, 0x01]);
    *escaped[0].borrow_mut() = Box::new(replacement);
    bus.sync_pwm_sourced_servos();
    assert!(
        !bus.observed_of::<Servo>().next().unwrap().is_commanded(),
        "public sync must reject an unidentified replacement"
    );
    bus.sync_legacy_pwm_servos();
    assert!(
        !bus.observed_of::<Servo>().next().unwrap().is_commanded(),
        "unidentified replacement must not command declared servo"
    );
}

#[test]
fn escaped_host_does_not_poll_an_undeclared_extra_source() {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/nrf52840.yaml"),
    )
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(r#"
name: nrf52-extra-source
chip: nrf52840
external_devices:
 - {id: pca, type: pca9685, connection: i2c0, config: {i2c_address: 0x42}}
 - {id: servo0, type: servo, connection: i2c0, config: {model: sg90, source_address: 65, source_channel: 0}}
"#).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let mut extra = labwired_core::peripherals::components::GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("pca9685").unwrap(),
        0x41,
    )
    .unwrap();
    write(&mut extra, 0, &[0x10]);
    write(&mut extra, 0xfe, &[121]);
    write(&mut extra, 0, &[0x20]);
    write(&mut extra, 0x06, &[0, 0, 0x33, 0x01]);
    bus.attach_i2c_slave("i2c0", Box::new(extra)).unwrap();
    bus.sync_pwm_sourced_servos();
    assert!(
        !bus.observed_of::<Servo>().next().unwrap().is_commanded(),
        "direct sync must reject undeclared source before raw escape"
    );
    let host = bus
        .peripherals
        .iter()
        .find(|p| p.name == "i2c0")
        .unwrap()
        .dev
        .as_any()
        .unwrap()
        .downcast_ref::<labwired_core::peripherals::nrf52::serial_instance::Nrf52SerialInstance>()
        .unwrap();
    let _ = host.attached_i2c_devices();
    bus.sync_legacy_pwm_servos();
    assert!(
        !bus.observed_of::<Servo>().next().unwrap().is_commanded(),
        "undeclared source must not drive declared servo"
    );
}
