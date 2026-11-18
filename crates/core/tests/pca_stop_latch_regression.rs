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
fn pca_default_output_changes_only_after_stop() {
    let yaml = labwired_config::embedded_device_yaml("pca9685").unwrap();
    let mut dev = GenericI2cDevice::from_yaml(yaml, 0x40).unwrap();
    write(&mut dev, 0x00, &[0x10]);
    write(&mut dev, 0xfe, &[121]);
    write(&mut dev, 0x00, &[0x20]);
    write(&mut dev, 0x06, &[0, 0, 0xcd, 0x00]);
    let before = dev.pwm_channel_pulse_us(0).unwrap();
    dev.start();
    dev.write(0x06);
    for b in [0, 0, 0x33, 0x01] {
        dev.write(b);
    }
    assert_eq!(
        dev.pwm_channel_pulse_us(0),
        Some(before),
        "default OCH must hold the prior output until STOP"
    );
    dev.stop();
    let after = dev.pwm_channel_pulse_us(0).unwrap();
    assert!(after > before + 400.0, "STOP commits new output");
}

#[test]
fn repeated_start_does_not_commit_stop_mode_outputs() {
    let yaml = labwired_config::embedded_device_yaml("pca9685").unwrap();
    let mut dev = GenericI2cDevice::from_yaml(yaml, 0x40).unwrap();
    write(&mut dev, 0x00, &[0x10]);
    write(&mut dev, 0xfe, &[121]);
    write(&mut dev, 0x00, &[0x20]);
    write(&mut dev, 0x06, &[0, 0, 0xcd, 0]);
    let before = dev.pwm_channel_pulse_us(0).unwrap();
    dev.start();
    dev.write(0x06);
    for b in [0, 0, 0x33, 0x01] {
        dev.write(b);
    }
    dev.start(); // a repeated START is not a STOP
    assert_eq!(dev.pwm_channel_pulse_us(0), Some(before));
    dev.stop();
    assert!(dev.pwm_channel_pulse_us(0).unwrap() > before + 400.0);
}

#[test]
fn unrelated_descriptor_does_not_inherit_builtin_pca_latch() {
    let yaml = labwired_config::embedded_device_yaml("pca9685").unwrap();
    let yaml = yaml.replace("type: pca9685", "type: another_controller");
    let mut dev = GenericI2cDevice::from_yaml(&yaml, 0x40).unwrap();
    write(&mut dev, 0x00, &[0x10]);
    write(&mut dev, 0xfe, &[121]);
    write(&mut dev, 0x00, &[0x20]);
    write(&mut dev, 0x06, &[0, 0, 0xcd, 0]);
    let before = dev.pwm_channel_pulse_us(0).unwrap();
    dev.start();
    dev.write(0x06);
    for b in [0, 0, 0x33, 0x01] {
        dev.write(b);
    }
    assert!(dev.pwm_channel_pulse_us(0).unwrap() > before + 400.0);
}

#[test]
fn register_readback_stays_live_while_output_waits_for_stop() {
    let yaml = labwired_config::embedded_device_yaml("pca9685").unwrap();
    let mut dev = GenericI2cDevice::from_yaml(yaml, 0x40).unwrap();
    write(&mut dev, 0x00, &[0x10]);
    write(&mut dev, 0xfe, &[121]);
    write(&mut dev, 0x00, &[0x20]);
    write(&mut dev, 0x06, &[0, 0, 0xcd, 0]);
    let before = dev.pwm_channel_pulse_us(0).unwrap();
    dev.start();
    dev.write(0x06);
    for b in [0, 0, 0x33, 0x01] {
        dev.write(b);
    }
    dev.start(); // Repeated START is required to read a new pointer.
    dev.write(0x08);
    dev.start();
    assert_eq!(dev.read(), 0x33, "register reads expose the new value");
    assert_eq!(
        dev.pwm_channel_pulse_us(0),
        Some(before),
        "output still waits for STOP"
    );
    dev.stop();
    assert!(dev.pwm_channel_pulse_us(0).unwrap() > before + 400.0);
}
