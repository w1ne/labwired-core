use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::servo::Servo;
use std::path::PathBuf;

#[test]
fn substitute_mux_cannot_replace_the_declared_mux_identity() {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    let manifest:SystemManifest=serde_yaml::from_str(r#"
name: declared-unknown-peer
chip: inline
external_devices:
  - { id: pca, type: pca9685, connection: i2c1, config: { i2c_address: 0x41 } }
  - { id: mux, type: tca9548a, connection: i2c1, config: { i2c_address: 0x71 } }
  - { id: downstream, type: pca9685, connection: mux, channel: 0, config: { i2c_address: 0x41 } }
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
    let fake = labwired_core::peripherals::components::tca9548a::Tca9548a::new(0x71);
    bus.attach_i2c_slave("i2c1", Box::new(fake)).unwrap();
    assert!(
        !bus.observed_of::<Servo>().next().unwrap().is_commanded(),
        "unidentified substitute cannot prove declared peer was restored"
    );
}
