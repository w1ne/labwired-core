use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::servo::Servo;
use std::path::PathBuf;

#[test]
fn substitution_cannot_satisfy_an_unknown_declared_peer() {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    let manifest:SystemManifest=serde_yaml::from_str(r#"
name: declared-unknown-peer
chip: inline
external_devices:
  - { id: pca, type: pca9685, connection: i2c1, config: { i2c_address: 0x41 } }
  - { id: sensor, type: tmp102, connection: i2c1 }
  - { id: servo0, type: servo, connection: i2c1, config: {model: sg90, source_address: 65, source_channel: 0} }
"#).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let mut slaves = bus.take_i2c_slaves("i2c1");
    let first = slaves.iter().position(|d| d.address() == 0x41).unwrap();
    let mut pca = slaves.remove(first);
    fn w(d: &mut dyn labwired_core::peripherals::i2c::I2cDevice, r: u8, v: &[u8]) {
        d.start();
        d.write(r);
        for &b in v {
            d.write(b);
        }
        d.stop();
    }
    w(pca.as_mut(), 0, &[0x10]);
    w(pca.as_mut(), 0xfe, &[121]);
    w(pca.as_mut(), 0, &[0x20]);
    w(pca.as_mut(), 0x06, &[0, 0, 0x33, 0x01]);
    bus.attach_i2c_slave("i2c1", pca).unwrap();
    assert!(
        !bus.observed_of::<Servo>().next().unwrap().is_commanded(),
        "partial topology cannot replay before default-address peer is restored"
    );
    let fake = labwired_core::peripherals::components::GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("tmp102").unwrap(),
        0x49,
    )
    .unwrap();
    bus.attach_i2c_slave("i2c1", Box::new(fake)).unwrap();
    assert!(
        !bus.observed_of::<Servo>().next().unwrap().is_commanded(),
        "unidentified substitute cannot prove declared peer was restored"
    );
}
