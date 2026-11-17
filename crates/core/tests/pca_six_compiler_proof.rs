use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::servo::Servo;
use labwired_core::peripherals::i2c::I2cDevice;
use std::path::PathBuf;

fn write(d: &mut dyn I2cDevice, reg: u8, bytes: &[u8]) {
    d.start();
    d.write(reg);
    for b in bytes {
        d.write(*b);
    }
    d.stop();
}
#[test]
fn six_compiler_emitted_servo_channels_follow_controller_and_disabled_is_silent() {
    let chip = ChipDescriptor::from_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap();
    // Copied from the actual board-config compile output for the six-channel test fixture.
    let manifest: SystemManifest =
        serde_yaml::from_str(include_str!("pca-compiled-six-f401.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let mut slaves = bus.take_i2c_slaves("i2c1");
    assert_eq!(slaves.len(), 1);
    let pca = slaves[0].as_mut();
    write(pca, 0, &[0x10]);
    write(pca, 0xfe, &[121]);
    write(pca, 0, &[0x20]);
    for ch in 0..6u8 {
        let off = 205 + u16::from(ch) * 30;
        write(pca, 0x06 + 4 * ch, &[0, 0, off as u8, (off >> 8) as u8]);
    }
    // Channel 2 is full-off before any servo synchronization.
    write(pca, 0x06 + 4 * 2, &[0, 0, 0, 0x10]);
    for slave in slaves {
        bus.attach_i2c_slave("i2c1", slave).unwrap();
    }
    bus.sync_pwm_sourced_servos();
    let servos: Vec<_> = bus.observed_of::<Servo>().collect();
    assert_eq!(servos.len(), 6);
    assert!(!servos.iter().any(|s| s.id() == "disconnected"));
    for ch in 0..6 {
        let servo = servos
            .iter()
            .find(|s| s.id() == format!("servo{ch}"))
            .unwrap();
        if ch == 2 {
            assert!(!servo.is_commanded());
            continue;
        }
        let expected = (205 + ch * 30) as f32 * 122.0 / 25.0;
        assert!(servo.is_commanded(), "servo {ch}");
        assert!(
            (servo.pulse_us() - expected).abs() < 2.0,
            "servo {ch}: {}",
            servo.pulse_us()
        );
    }
    let mut slaves = bus.take_i2c_slaves("i2c1");
    let off = 205u16 + 2 * 30;
    write(
        slaves[0].as_mut(),
        0x06 + 4 * 2,
        &[0, 0, off as u8, (off >> 8) as u8],
    );
    for slave in slaves {
        bus.attach_i2c_slave("i2c1", slave).unwrap();
    }
    bus.sync_pwm_sourced_servos();
    let servos: Vec<_> = bus.observed_of::<Servo>().collect();
    assert!(
        servos.iter().all(|servo| servo.is_commanded()),
        "all six channels must drive"
    );
    let servo2 = servos.iter().find(|s| s.id() == "servo2").unwrap();
    assert!((servo2.pulse_us() - (off as f32 * 122.0 / 25.0)).abs() < 2.0);
    let mut slaves = bus.take_i2c_slaves("i2c1");
    write(slaves[0].as_mut(), 0, &[0x10]); // sleep makes all outputs silent
    assert!(slaves[0].pwm_channel_pulse_us(0).is_none());
}
