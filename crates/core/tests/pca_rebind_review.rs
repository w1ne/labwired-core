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
fn rebinding_new_servo_replays_the_existing_valid_pulse() {
    use labwired_core::peripherals::components::servo::ServoCal;
    use std::sync::Arc;
    let mut bus = make_bus();
    assert!(angle(&bus) > 80.0);
    bus.observed.clear();
    bus.observe_device(Arc::new(Servo::with_source(
        "servo0",
        ServoCal::sg90(),
        0x41,
        0,
    )));
    bus.bind_direct_pwm_servos();
    assert!(
        angle(&bus) > 80.0,
        "rebound servo must receive the already-active valid signal"
    );
}
#[test]
fn scoped_benign_mutation_rebinds_and_replays_its_command() {
    let mut bus = make_bus();
    let before = angle(&bus);
    bus.with_i2c_slave_mut("i2c1", 0x41, |slave| {
        slave.start();
        for b in [0x06, 0, 0, 0x90, 0x01] {
            slave.write(b);
        }
        slave.stop();
    })
    .unwrap();
    assert!(
        angle(&bus) > before + 20.0,
        "scoped device access must rebind and apply the committed signal"
    );
}

#[test]
fn legacy_readback_recovers_after_existing_raw_mutable_access() {
    let mut bus = make_bus();
    let before = angle(&bus);
    let host = bus
        .peripherals
        .iter_mut()
        .find(|p| p.name == "i2c1")
        .unwrap()
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<labwired_core::peripherals::i2c::I2c>()
        .unwrap();
    let mut device = host.attached_devices()[0].borrow_mut();
    device.start();
    for b in [0x06, 0, 0, 0x90, 0x01] {
        device.write(b);
    }
    device.stop();
    drop(device);
    assert_eq!(
        angle(&bus),
        before,
        "raw access invalidates bus binding until reconciliation"
    );
    bus.sync_legacy_pwm_servos();
    assert!(
        angle(&bus) > before + 20.0,
        "readback must reconcile existing raw-access callers"
    );
}
