use labwired_core::peripherals::components::GenericI2cDevice;
use labwired_core::peripherals::i2c::I2cDevice;

#[test]
fn servo_angle_observable_does_not_prove_pca_capability() {
    let yaml = labwired_config::embedded_device_yaml("pca9685").unwrap();
    let other = yaml.replacen("type: pca9685", "type: unrelated_pwm", 1);
    let mut device = GenericI2cDevice::from_yaml(&other, 0).unwrap();
    for (reg, bytes) in [
        (0u8, vec![0x10]),
        (0xfe, vec![121]),
        (0, vec![0x20]),
        (6, vec![0, 0, 0x33, 1]),
    ] {
        device.start();
        device.write(reg);
        for byte in bytes {
            device.write(byte);
        }
        device.stop();
    }
    assert_eq!(device.pwm_channel_pulse_us(0), None);
}

#[test]
fn modified_pca_descriptor_does_not_inherit_builtin_output_semantics() {
    let yaml = labwired_config::embedded_device_yaml("pca9685").unwrap();
    let other = yaml.replacen("default_address: 0x40", "default_address: 0x41", 1);
    assert_ne!(yaml, other);
    let mut device = GenericI2cDevice::from_yaml(&other, 0).unwrap();
    for (reg, bytes) in [
        (0u8, vec![0x10]),
        (0xfe, vec![121]),
        (0, vec![0x20]),
        (6, vec![0, 0, 0x33, 1]),
    ] {
        device.start();
        device.write(reg);
        for byte in bytes {
            device.write(byte);
        }
        device.stop();
    }
    assert_eq!(device.pwm_channel_pulse_us(0), None);
}
