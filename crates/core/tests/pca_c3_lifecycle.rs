use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, peripherals::components::servo::Servo, Bus};
use std::path::PathBuf;
const I2C: u64 = 0x6001_3000;
const GPIO: u64 = 0x6000_4000;
fn make_bus() -> SystemBus {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/esp32c3.yaml"),
    )
    .unwrap();
    let manifest:SystemManifest=serde_yaml::from_str(r#"
name: c3-pca-servo
chip: esp32c3
external_devices:
 - {id: pca, type: pca9685, connection: i2c0, route: {sda: GPIO5, scl: GPIO6}, config: {i2c_address: 0x41}}
 - {id: servo0, type: servo, connection: i2c0, config: {model: sg90, source_address: 65, source_channel: 0}}
"#).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    bus.peripherals
        .iter_mut()
        .find(|p| p.name == "i2c0")
        .unwrap()
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c>()
        .unwrap()
        .force_legacy_walk();
    for (offset, val) in [
        (0, 199),
        (0x38, 180 | (19 << 9)),
        (0x30, 29),
        (0x40, 199),
        (0x44, 199),
        (0x4c, 199),
        (0x48, 199),
    ] {
        bus.write_u32(I2C + offset, val).unwrap();
    }
    bus
}
fn route(bus: &mut SystemBus) {
    bus.write_u32(GPIO + 0x24, (1 << 5) | (1 << 6)).unwrap();
    bus.write_u32(GPIO + 0x554 + 5 * 4, 54).unwrap();
    bus.write_u32(GPIO + 0x554 + 6 * 4, 53).unwrap();
    bus.write_u32(GPIO + 0x154 + 54 * 4, (1 << 6) | 5).unwrap();
    bus.write_u32(GPIO + 0x154 + 53 * 4, (1 << 6) | 6).unwrap();
}
fn transaction(bus: &mut SystemBus, reg: u8, bytes: &[u8]) -> u32 {
    let cmd = |op: u32, n: u32| (op << 11) | n;
    bus.write_u32(I2C + 0x58, cmd(6, 0)).unwrap();
    bus.write_u32(I2C + 0x5c, cmd(1, bytes.len() as u32 + 2))
        .unwrap();
    bus.write_u32(I2C + 0x60, cmd(2, 0)).unwrap();
    bus.write_u32(I2C + 0x1c, 0x82).unwrap();
    bus.write_u32(I2C + 0x1c, reg as u32).unwrap();
    for &b in bytes {
        bus.write_u32(I2C + 0x1c, b as u32).unwrap();
    }
    bus.write_u32(I2C + 0x24, u32::MAX).unwrap();
    bus.write_u32(I2C + 4, 1 << 5).unwrap();
    for _ in 0..10000 {
        bus.peripherals
            .iter_mut()
            .find(|p| p.name == "i2c0")
            .unwrap()
            .dev
            .tick_elapsed(64);
        if bus.read_u32(I2C + 0x20).unwrap() & (1 << 7) != 0 {
            return bus.read_u32(I2C + 0x20).unwrap();
        }
    }
    panic!("C3 transaction did not complete")
}
fn angle(bus: &SystemBus) -> f32 {
    bus.observed_of::<Servo>()
        .find(|s| s.id() == "servo0")
        .unwrap()
        .angle_degrees()
}
fn init(bus: &mut SystemBus) {
    route(bus);
    for (reg, bytes) in [
        (0, &[0x10][..]),
        (0xfe, &[121][..]),
        (0, &[0x20][..]),
        (0x06, &[0, 0, 0x33, 0x01][..]),
    ] {
        assert_eq!(transaction(bus, reg, bytes) & (1 << 10), 0);
    }
}
#[test]
fn route_disconnect_after_command_preserves_position_and_rejects_new_command() {
    let mut bus = make_bus();
    init(&mut bus);
    let before = angle(&bus);
    assert!(before > 80.0);
    bus.write_u32(GPIO + 0x154 + 54 * 4, (1 << 6) | 4).unwrap();
    assert_ne!(
        transaction(&mut bus, 0x06, &[0, 0, 0x90, 0x01]) & (1 << 10),
        0,
        "unrouted source must NACK"
    );
    assert_eq!(
        angle(&bus),
        before,
        "disconnected route cannot deliver a new command"
    );
}
#[test]
fn removed_c3_source_cannot_drive_its_old_servo() {
    let mut bus = make_bus();
    init(&mut bus);
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
        .downcast_mut::<labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c>()
        .unwrap();
    let fake = labwired_core::peripherals::components::GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("pca9685").unwrap(),
        0x41,
    )
    .unwrap();
    let mut old = std::mem::replace(&mut host.attached_slaves_mut()[0], Box::new(fake));
    old.start();
    for b in [0x06, 0, 0, 0x90, 0x01] {
        old.write(b);
    }
    old.stop();
    assert_eq!(
        angle(&bus),
        before,
        "removed source must not command the old servo"
    );
}
