use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;

fn write(dev: &mut dyn I2cDevice, reg: u8, bytes: &[u8]) {
    dev.start();
    dev.write(reg);
    for &b in bytes {
        dev.write(b);
    }
    dev.stop();
}

#[test]
fn ack_mode_waits_for_four_distinct_channel_registers() {
    let yaml = labwired_config::embedded_device_yaml("pca9685").unwrap();
    let mut dev = GenericI2cDevice::from_yaml(yaml, 0x40).unwrap();
    write(&mut dev, 0x00, &[0x10]);
    write(&mut dev, 0xfe, &[121]);
    write(&mut dev, 0x00, &[0x20]);
    write(&mut dev, 0x06, &[0, 0, 0xcd, 0]);
    let before = dev.pwm_channel_pulse_us(0).unwrap();
    write(&mut dev, 0x01, &[0x0c]);
    dev.start();
    dev.write(0x06);
    for byte in [0, 0, 0x33] {
        dev.write(byte);
        assert_eq!(dev.pwm_channel_pulse_us(0), Some(before));
    }
    // A repeated START and another channel do not complete channel 0.
    dev.start();
    dev.write(0x0a);
    dev.write(0);
    assert_eq!(dev.pwm_channel_pulse_us(0), Some(before));
    dev.start();
    dev.write(0x09);
    dev.write(0x01);
    let after = dev.pwm_channel_pulse_us(0).unwrap();
    assert!(after > before + 400.0, "fourth register commits on ACK");
    dev.stop();
    assert_eq!(dev.pwm_channel_pulse_us(0), Some(after));
}

#[test]
fn ack_mode_partial_write_does_not_commit_on_stop() {
    let yaml = labwired_config::embedded_device_yaml("pca9685").unwrap();
    let mut dev = GenericI2cDevice::from_yaml(yaml, 0x40).unwrap();
    write(&mut dev, 0x00, &[0x10]);
    write(&mut dev, 0xfe, &[121]);
    write(&mut dev, 0x00, &[0x20]);
    write(&mut dev, 0x06, &[0, 0, 0xcd, 0]);
    let before = dev.pwm_channel_pulse_us(0).unwrap();
    write(&mut dev, 0x01, &[0x0c]);
    write(&mut dev, 0x08, &[0x33, 0x01]);
    assert_eq!(dev.pwm_channel_pulse_us(0), Some(before));
}
