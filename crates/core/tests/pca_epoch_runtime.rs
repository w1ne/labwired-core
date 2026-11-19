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
fn pca() -> labwired_core::peripherals::components::GenericI2cDevice {
    labwired_core::peripherals::components::GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("pca9685").unwrap(),
        0x41,
    )
    .unwrap()
}
#[test]
fn adding_duplicate_invalidates_the_bound_servo() {
    let mut bus = make_bus();
    let before = angle(&bus);
    assert!(before > 80.0);
    bus.attach_i2c_slave("i2c1", Box::new(pca())).unwrap();
    transaction(&mut bus, 0x06, &[0, 0, 0x90, 0x01]);
    assert_eq!(
        angle(&bus),
        before,
        "ambiguous bus cannot drive stale subscription"
    );
}
#[test]
fn replacing_device_via_public_cell_invalidates_binding_before_removal() {
    let mut bus = make_bus();
    let before = angle(&bus);
    assert!(before > 80.0);
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
    let mut old = std::mem::replace(
        &mut *host.attached_devices()[0].borrow_mut(),
        Box::new(pca()),
    );
    old.start();
    for b in [0x06, 0, 0, 0x90, 0x01] {
        old.write(b);
    }
    old.stop();
    assert_eq!(
        angle(&bus),
        before,
        "detached source cannot actuate a stale servo"
    );
}
#[test]
fn extracting_slaves_invalidates_binding() {
    let mut bus = make_bus();
    let before = angle(&bus);
    assert!(before > 80.0);
    let mut old = bus.take_i2c_slaves("i2c1");
    old[0].start();
    for b in [0x06, 0, 0, 0x90, 0x01] {
        old[0].write(b);
    }
    old[0].stop();
    assert_eq!(
        angle(&bus),
        before,
        "removed source cannot actuate a stale servo"
    );
}

#[test]
fn reattaching_one_unambiguous_source_restores_servo_delivery() {
    let mut bus = make_bus();
    let before = angle(&bus);
    let mut old = bus.take_i2c_slaves("i2c1");
    bus.attach_i2c_slave("i2c1", old.remove(0)).unwrap();
    transaction(&mut bus, 0x06, &[0, 0, 0x90, 0x01]);
    assert!(
        angle(&bus) > before + 20.0,
        "reattached source must drive the servo again"
    );
}

#[test]
fn power_cycle_resets_pca_and_rebinding_allows_fresh_firmware_commands() {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    let yaml = include_str!("pca-event-six-f401.yaml").replace(
        "      i2c_address: 0x41",
        "      i2c_address: 0x41\n      supply_rails:\n        vdd: {min_v: 3.0, max_v: 3.6}",
    );
    let manifest: SystemManifest = serde_yaml::from_str(&yaml).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let power = |bus: &mut SystemBus, v| {
        bus.apply_device_supply_voltages(&[("pca".into(), "vdd".into(), v)])
            .unwrap()
    };
    power(&mut bus, 3.3);
    transaction(&mut bus, 0, &[0x10]);
    transaction(&mut bus, 0xfe, &[121]);
    transaction(&mut bus, 0, &[0x20]);
    transaction(&mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    let before = angle(&bus);
    assert!(before > 80.0, "initial power must bind");
    power(&mut bus, 0.0);
    power(&mut bus, 3.3);
    assert_eq!(
        angle(&bus),
        before,
        "restoring power alone cannot invent a pulse"
    );
    assert_eq!(
        bus.pwm_channel_pulse_us(0x41, 0),
        None,
        "PCA output is reset after power loss"
    );
    transaction(&mut bus, 0, &[0x10]);
    transaction(&mut bus, 0xfe, &[121]);
    transaction(&mut bus, 0, &[0x20]);
    transaction(&mut bus, 0x06, &[0, 0, 0x90, 0x01]);
    assert!(
        angle(&bus) > before + 20.0,
        "fresh firmware command must reach the servo after restore"
    );
}

#[test]
fn powered_restore_does_not_replay_stale_pca_pulse_to_new_servo() {
    use labwired_core::peripherals::components::servo::ServoCal;
    use std::sync::Arc;
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    let yaml = include_str!("pca-event-six-f401.yaml").replace(
        "      i2c_address: 0x41",
        "      i2c_address: 0x41\n      supply_rails:\n        vdd: {min_v: 3.0, max_v: 3.6}",
    );
    let manifest: SystemManifest = serde_yaml::from_str(&yaml).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let power = |b: &mut SystemBus, v| {
        b.apply_device_supply_voltages(&[("pca".into(), "vdd".into(), v)])
            .unwrap()
    };
    power(&mut bus, 3.3);
    transaction(&mut bus, 0, &[0x10]);
    transaction(&mut bus, 0xfe, &[121]);
    transaction(&mut bus, 0, &[0x20]);
    transaction(&mut bus, 0x06, &[0, 0, 0x33, 0x01]);
    power(&mut bus, 0.0);
    power(&mut bus, 3.3);
    bus.observed.clear();
    bus.observe_device(Arc::new(Servo::with_source(
        "servo0",
        ServoCal::sg90(),
        0x41,
        0,
    )));
    bus.bind_direct_pwm_servos();
    assert!(
        !bus.observed_of::<Servo>().next().unwrap().is_commanded(),
        "old PCA output must not be replayed after power cycling"
    );
}
