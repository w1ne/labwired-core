use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::{
    supply_domain::{GatedI2cDevice, SupplyGate},
    GenericI2cDevice,
};
use labwired_core::peripherals::i2c::I2cDevice;
fn write_reg(d: &mut dyn I2cDevice, reg: u8, bytes: &[u8]) {
    d.start();
    d.write(reg);
    for &b in bytes {
        d.write(b);
    }
    d.stop();
}
#[test]
fn routed_gate_resets_pwm_through_a_legacy_factory_gate() {
    let mut bus = SystemBus::new();
    let domain = bus.register_supply_domain("vdd", 3.0).unwrap();
    bus.update_supply_domain("vdd", 3.3).unwrap();
    let pca = GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("pca9685").unwrap(),
        0x41,
    )
    .unwrap();
    let inner =
        GatedI2cDevice::with_attachment_id(Box::new(pca), SupplyGate::legacy(true), "pca".into());
    let mut outer = GatedI2cDevice::new(Box::new(inner), SupplyGate::routed(domain, true));
    write_reg(&mut outer, 0, &[0x10]);
    write_reg(&mut outer, 0xfe, &[121]);
    write_reg(&mut outer, 0, &[0x20]);
    write_reg(&mut outer, 0x06, &[0, 0, 0x33, 0x01]);
    assert!(outer.pwm_channel_pulse_us(0).is_some());
    bus.update_supply_domain("vdd", 0.0).unwrap();
    bus.update_supply_domain("vdd", 3.3).unwrap();
    assert_eq!(outer.pwm_channel_pulse_us(0), None);
    outer.start();
    assert_eq!(
        outer.pwm_channel_pulse_us(0),
        None,
        "power restoration must reset the nested PCA before any new command"
    );
}
