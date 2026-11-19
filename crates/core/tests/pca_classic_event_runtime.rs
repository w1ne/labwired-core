use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::servo::Servo;
use labwired_core::system::xtensa::{attach_esp32_external_devices, configure_xtensa_esp32};
use labwired_core::Bus;
fn write(bus: &mut SystemBus, reg: u8, bytes: &[u8]) {
    let base = 0x3ff5_3000u64;
    let cmd = |op: u32, n: u32| (op << 11) | n;
    bus.write_u32(base + 0x58, cmd(0, 0)).unwrap();
    bus.write_u32(base + 0x5c, cmd(1, bytes.len() as u32 + 2))
        .unwrap();
    bus.write_u32(base + 0x60, cmd(3, 0)).unwrap();
    for b in [0x82, reg].into_iter().chain(bytes.iter().copied()) {
        bus.write_u32(base + 0x1c, u32::from(b)).unwrap();
    }
    bus.write_u32(base + 0x24, u32::MAX).unwrap();
    bus.write_u32(base + 0x04, 1 << 5).unwrap();
    assert_ne!(
        bus.read_u32(base + 0x20).unwrap() & (1 << 7),
        0,
        "classic transaction must complete"
    );
    assert_eq!(
        bus.read_u32(base + 0x20).unwrap() & (1 << 10),
        0,
        "PCA must ACK"
    );
}
fn make_bus() -> SystemBus {
    let mut bus = SystemBus::new();
    configure_xtensa_esp32(&mut bus);
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
fn classic_firmware_transaction_updates_servo_without_inspection() {
    let mut bus = make_bus();
    write(&mut bus, 0, &[0x10]);
    write(&mut bus, 0xfe, &[121]);
    write(&mut bus, 0, &[0x20]);
    write(&mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    assert!(
        bus.pwm_channel_pulse_us(0x41, 0).is_some(),
        "classic transaction must commit a known pulse"
    );
    assert!(
        bus.observed_of::<Servo>().next().unwrap().is_commanded(),
        "servo should receive pulse before full-off"
    );
    write(&mut bus, 0x06, &[0, 0, 0, 0x10]);
    let servo = bus
        .observed_of::<Servo>()
        .find(|s| s.id() == "servo0")
        .unwrap();
    assert!(
        servo.is_commanded() && servo.angle_degrees() > 80.0,
        "classic pulse must survive full-off"
    );
}
