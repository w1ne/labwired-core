//! Cross-repository evidence only: generic app-compiled 20-servo system.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::{Bus, SystemBus};
use labwired_core::peripherals::components::servo::Servo;
use labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c;
use labwired_core::peripherals::i2c::I2cDevice;
use std::path::PathBuf;

fn write_reg(d: &mut dyn I2cDevice, reg: u8, bytes: &[u8]) {
    d.start();
    d.write(reg);
    for byte in bytes {
        d.write(*byte);
    }
    d.stop();
}
fn program(bus: &mut SystemBus, offset: u8) {
    let idx = bus.find_peripheral_index_by_name("i2c0").unwrap();
    let i2c = bus.peripherals[idx]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Esp32c3I2c>()
        .unwrap();
    for pca in i2c.attached_slaves_mut() {
        if pca.address() != 0x40 && pca.address() != 0x41 {
            continue;
        }
        write_reg(pca.as_mut(), 0x00, &[0x10]);
        write_reg(pca.as_mut(), 0xfe, &[121]);
        write_reg(pca.as_mut(), 0x00, &[0x20]);
        for channel in 0..16 {
            write_reg(
                pca.as_mut(),
                0x06 + 4 * channel,
                &[0, 0, 0x20 + offset + channel, 0x01],
            );
        }
    }
}
fn values(bus: &SystemBus) -> Vec<(String, bool, f32)> {
    let mut result: Vec<_> = bus
        .observed_of::<Servo>()
        .map(|s| (s.id().to_owned(), s.is_commanded(), s.angle_degrees()))
        .collect();
    result.sort_by_key(|(id, _, _)| id.trim_start_matches("servo").parse::<usize>().unwrap());
    result
}
#[test]
fn generated_20_servo_topology_is_gated_by_each_controller_oe() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml")).unwrap();
    let yaml = include_str!("fixtures/pca_oe_paired/generated-system.yaml");
    let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let sources: std::collections::HashMap<_, _> = bus
        .observed_of::<Servo>()
        .map(|servo| (servo.id().to_owned(), servo.source()))
        .collect();
    assert_eq!(sources.len(), 20);
    for i in 0..20u8 {
        assert_eq!(
            sources.get(&format!("servo{i}")),
            Some(&Some((if i < 16 { 0x40 } else { 0x41 }, i % 16)))
        );
    }
    program(&mut bus, 0);
    bus.sync_pwm_sourced_servos();
    let initial = values(&bus);
    assert_eq!(initial.len(), 20);
    assert!(initial.iter().all(|x| !x.1));
    const GPIO: u64 = 0x6000_4000;
    bus.write_u32(GPIO + 0x08, (1 << 3) | (1 << 4)).unwrap();
    bus.write_u32(GPIO + 0x24, (1 << 3) | (1 << 4)).unwrap();
    bus.sync_pwm_sourced_servos();
    assert!(values(&bus).iter().all(|x| !x.1));
    bus.write_u32(GPIO + 0x0c, 1 << 3).unwrap();
    bus.sync_pwm_sourced_servos();
    let first = values(&bus);
    assert!(first[..16].iter().all(|x| x.1));
    assert!(first[16..].iter().all(|x| !x.1));
    bus.write_u32(GPIO + 0x08, 1 << 3).unwrap();
    bus.write_u32(GPIO + 0x0c, 1 << 4).unwrap();
    program(&mut bus, 32);
    bus.sync_pwm_sourced_servos();
    let second = values(&bus);
    assert_eq!(first[..16], second[..16]);
    assert!(second[16..].iter().all(|x| x.1));
    bus.write_u32(GPIO + 0x0c, 1 << 3).unwrap();
    bus.sync_pwm_sourced_servos();
    let both = values(&bus);
    assert!(both[..16]
        .iter()
        .zip(&first[..16])
        .all(|(new, old)| new.2 != old.2));
    assert_eq!(both[16..], second[16..]);
}
