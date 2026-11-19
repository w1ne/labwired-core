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
#[test]
fn servo_rebinding_does_not_remove_an_independent_pwm_subscriber() {
    use std::sync::{Arc, Mutex};
    let mut bus = make_bus();
    bus.register_supply_domain("aux", 3.0).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let out = Arc::clone(&events);
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
    assert!(host.attached_devices()[0]
        .borrow()
        .subscribe_pwm_output(Arc::new(move |ch, us| if ch == 0 {
            out.lock().unwrap().push(us);
        })));
    bus.update_supply_domain("aux", 3.3).unwrap();
    transaction(&mut bus, 0x06, &[0, 0, 0x90, 0x01]);
    assert_eq!(
        events.lock().unwrap().len(),
        1,
        "rebinding servo must preserve independent observer"
    );
}
