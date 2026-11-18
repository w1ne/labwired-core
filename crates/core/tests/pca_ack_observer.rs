use labwired_core::peripherals::components::GenericI2cDevice;
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
fn ack_mode_publishes_only_after_fourth_channel_byte() {
    let mut pca =
        GenericI2cDevice::from_yaml(labwired_config::embedded_device_yaml("pca9685").unwrap(), 0)
            .unwrap();
    let pulses = Arc::new(Mutex::new(Vec::new()));
    let out = Arc::clone(&pulses);
    assert!(pca.subscribe_pwm_output(Arc::new(move |ch, us| if ch == 0 {
        out.lock().unwrap().push(us)
    })));
    write_reg(&mut pca, 0, &[0x10]);
    write_reg(&mut pca, 0xfe, &[121]);
    write_reg(&mut pca, 0, &[0x20]);
    write_reg(&mut pca, 1, &[0x0c]);
    pca.start();
    pca.write(0x06);
    for byte in [0, 0, 0x33] {
        pca.write(byte);
        assert!(pulses.lock().unwrap().is_empty());
    }
    pca.write(0x01);
    assert!((pulses.lock().unwrap().last().copied().unwrap() - 1498.0).abs() < 1.0);
    pca.stop();
}
