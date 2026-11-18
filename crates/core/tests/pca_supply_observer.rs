use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::{
    supply_domain::{GatedI2cDevice, SupplyGate},
    GenericI2cDevice,
};
use labwired_core::peripherals::i2c::I2cDevice;
use std::sync::{Arc, Mutex};
fn write_reg(d: &mut dyn I2cDevice, reg: u8, bytes: &[u8]) {
    d.start();
    d.write(reg);
    for &b in bytes {
        d.write(b);
    }
    d.stop();
}
#[test]
fn stale_subscription_cannot_act_after_controller_supply_cycle() {
    let mut bus = SystemBus::new();
    let domain = bus.register_supply_domain("vdd", 3.0).unwrap();
    bus.update_supply_domain("vdd", 3.3).unwrap();
    let pca =
        GenericI2cDevice::from_yaml(labwired_config::embedded_device_yaml("pca9685").unwrap(), 0)
            .unwrap();
    let mut gated = GatedI2cDevice::new(Box::new(pca), SupplyGate::routed(domain, true));
    let pulses = Arc::new(Mutex::new(Vec::new()));
    let out = Arc::clone(&pulses);
    assert!(
        gated.subscribe_pwm_output(Arc::new(move |ch, us| if ch == 0 {
            out.lock().unwrap().push(us)
        }))
    );
    write_reg(&mut gated, 0, &[0x10]);
    write_reg(&mut gated, 0xfe, &[121]);
    write_reg(&mut gated, 0, &[0x20]);
    write_reg(&mut gated, 0x06, &[0, 0, 0x33, 0x01]);
    assert_eq!(pulses.lock().unwrap().len(), 1);
    bus.update_supply_domain("vdd", 0.0).unwrap();
    bus.update_supply_domain("vdd", 3.3).unwrap();
    write_reg(&mut gated, 0x06, &[0, 0, 0x90, 0x01]);
    assert_eq!(
        pulses.lock().unwrap().len(),
        1,
        "power cycle invalidates the old binding until reset and rebind are modeled"
    );
}
