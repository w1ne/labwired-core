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

fn make_bus() -> SystemBus {
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
    bus
}
fn angle(bus: &SystemBus) -> f32 {
    bus.observed_of::<Servo>()
        .find(|s| s.id() == "servo0")
        .unwrap()
        .angle_degrees()
}
#[test]
fn escaped_legacy_slice_cannot_reactivate_a_stale_event_binding() {
    let bus = make_bus();
    let before = angle(&bus);
    let host = bus
        .peripherals
        .iter()
        .find(|p| p.name == "i2c1")
        .unwrap()
        .dev
        .as_any()
        .unwrap()
        .downcast_ref::<labwired_core::peripherals::i2c::I2c>()
        .unwrap();
    let escaped = host.attached_devices();
    bus.sync_legacy_pwm_servos();
    {
        let mut d = escaped[0].borrow_mut();
        d.start();
        for b in [0x06, 0, 0, 0x90, 0x01] {
            d.write(b);
        }
        d.stop();
    }
    assert_eq!(
        angle(&bus),
        before,
        "escaped raw view must not retain a stale event callback"
    );
    bus.sync_legacy_pwm_servos();
    let updated = angle(&bus);
    assert!(
        updated > before + 20.0,
        "legacy readback still reconciles the command"
    );
    {
        let mut d = escaped[0].borrow_mut();
        d.start();
        for b in [0x06, 0, 0, 0x33, 0x01] {
            d.write(b);
        }
        d.stop();
    }
    assert_eq!(
        angle(&bus),
        updated,
        "sync cannot reactivate a callback while the old view still exists"
    );
    bus.sync_legacy_pwm_servos();
    assert!((angle(&bus) - before).abs() < 0.01);
}
