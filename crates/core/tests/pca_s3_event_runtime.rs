use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::servo::Servo;
use labwired_core::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32s3, Esp32s3Opts,
};
use labwired_core::Bus;
fn write(bus: &mut SystemBus, reg: u8, bytes: &[u8]) {
    let base = 0x6001_3000u64;
    let cmd = |op: u32, n: u32| (op << 11) | n;
    bus.write_u32(base + 0x58, cmd(6, 0)).unwrap();
    bus.write_u32(base + 0x5c, cmd(1, bytes.len() as u32 + 2))
        .unwrap();
    bus.write_u32(base + 0x60, cmd(2, 0)).unwrap();
    for b in [0x82, reg].into_iter().chain(bytes.iter().copied()) {
        bus.write_u32(base + 0x1c, u32::from(b)).unwrap();
    }
    bus.write_u32(base + 0x24, u32::MAX).unwrap();
    bus.write_u32(base + 0x04, 1 << 5).unwrap();
    assert_eq!(
        bus.read_u32(base + 0x20).unwrap() & (1 << 10),
        0,
        "PCA must ACK"
    );
}
fn make_bus() -> SystemBus {
    let mut bus = SystemBus::new();
    configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let manifest: labwired_config::SystemManifest = serde_yaml::from_str(
        r#"
name: s3-pca-servo
chip: esp32s3
external_devices:
  - id: pca
    type: pca9685
    connection: i2c0
    config: { i2c_address: 0x41 }
  - id: servo0
    type: servo
    connection: i2c0
    config: { model: sg90, source_address: 65, source_channel: 0 }
"#,
    )
    .unwrap();
    attach_esp32_external_devices(&mut bus, &manifest).unwrap();
    bus.refresh_peripheral_index();
    bus
}
#[test]
fn s3_firmware_transaction_updates_servo_without_inspection() {
    let mut bus = make_bus();
    write(&mut bus, 0, &[0x10]);
    write(&mut bus, 0xfe, &[121]);
    write(&mut bus, 0, &[0x20]);
    write(&mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    write(&mut bus, 0x06, &[0, 0, 0, 0x10]);
    let servo = bus
        .observed_of::<Servo>()
        .find(|s| s.id() == "servo0")
        .unwrap();
    assert!(
        servo.is_commanded() && servo.angle_degrees() > 80.0,
        "S3 pulse must survive full-off"
    );
}
fn initialize(bus: &mut SystemBus) {
    write(bus, 0, &[0x10]);
    write(bus, 0xfe, &[121]);
    write(bus, 0, &[0x20]);
    write(bus, 0x06, &[0, 0, 0x33, 0x01]);
}
fn angle(bus: &SystemBus) -> f32 {
    bus.observed_of::<Servo>()
        .find(|s| s.id() == "servo0")
        .unwrap()
        .angle_degrees()
}
fn pca() -> labwired_core::peripherals::components::GenericI2cDevice {
    labwired_core::peripherals::components::GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("pca9685").unwrap(),
        0x41,
    )
    .unwrap()
}
#[test]
fn s3_duplicate_source_stops_actuation() {
    let mut bus = make_bus();
    initialize(&mut bus);
    let before = angle(&bus);
    assert!(before > 80.0);
    bus.attach_i2c_slave("i2c0", Box::new(pca())).unwrap();
    write(&mut bus, 0x06, &[0, 0, 0x90, 0x01]);
    assert_eq!(
        angle(&bus),
        before,
        "ambiguous sources must invalidate the binding"
    );
}
#[test]
fn s3_mutable_slave_replacement_stops_detached_actuation() {
    let mut bus = make_bus();
    initialize(&mut bus);
    let before = angle(&bus);
    assert!(before > 80.0);
    let host = bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "i2c0")
        .unwrap()
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<labwired_core::peripherals::esp32s3::i2c::Esp32s3I2c>()
        .unwrap();
    let mut old = std::mem::replace(&mut host.attached_slaves_mut()[0], Box::new(pca()));
    old.start();
    for b in [0x06, 0, 0, 0x90, 0x01] {
        old.write(b);
    }
    old.stop();
    assert_eq!(
        angle(&bus),
        before,
        "removed controller must not drive former servo"
    );
}
