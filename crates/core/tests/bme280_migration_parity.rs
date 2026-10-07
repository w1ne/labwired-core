//! Byte-level BME280 parity against the unchanged Rust model, including its
//! integer inverse selection and its ignored soft-reset write.
use labwired_core::peripherals::components::bme280::Bme280;
use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;
use labwired_core::peripherals::components::i2c_factory::build_i2c_device;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::sim_input::SimInput;

fn yaml() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/devices/bme280.yaml"
    ))
    .expect("BME280 must have an actual YAML model")
}

fn models(address: u8) -> (Bme280, GenericI2cDevice) {
    (
        Bme280::new(address),
        GenericI2cDevice::from_yaml(&yaml(), address).unwrap(),
    )
}

fn read(device: &mut dyn I2cDevice, register: u8, count: usize) -> Vec<u8> {
    device.stop();
    device.write(register);
    device.start();
    let bytes = (0..count).map(|_| device.read()).collect();
    device.stop();
    bytes
}

fn write(device: &mut dyn I2cDevice, register: u8, bytes: &[u8]) {
    device.stop();
    device.write(register);
    for byte in bytes {
        device.write(*byte);
    }
    device.stop();
}

fn assert_measurements(old: &mut Bme280, new: &mut GenericI2cDevice, context: &str) {
    let expected = read(old, 0xf7, 8);
    let actual = read(new, 0xf7, 8);
    assert_eq!(actual, expected, "{context}");
    assert_eq!(actual[2] & 15, 0);
    assert_eq!(actual[5] & 15, 0);
}

#[test]
fn factory_uses_descriptor_at_both_addresses_and_preserves_metadata() {
    for address in [0x76u8, 0x77] {
        let config = [("i2c_address".to_string(), serde_yaml::Value::from(address))].into();
        let device = build_i2c_device("bme280", &config).unwrap();
        assert!(device.as_any().unwrap().is::<GenericI2cDevice>());
        assert_eq!(device.address(), address);
        let (mut old, mut new) = models(address);
        assert_eq!(read(&mut old, 0, 256), read(&mut new, 0, 256));
        let old_inputs = old.input_channels();
        let new_inputs = new.input_channels();
        assert_eq!(old_inputs.len(), new_inputs.len());
        for (a, b) in old_inputs.iter().zip(new_inputs) {
            assert_eq!(
                (&a.key, &a.label, &a.unit, a.min, a.max),
                (&b.key, &b.label, &b.unit, b.min, b.max)
            );
        }
    }
}

#[test]
fn controls_read_only_registers_unknown_bytes_and_pointer_wrap_match() {
    let (mut old, mut new) = models(0x76);
    for (register, bytes) in [
        (0xf2, vec![7, 255, 0x27, 0xa5]),
        (0xe0, vec![0xb6]), // The reference deliberately ignores soft reset.
        (0x88, vec![0; 26]),
        (0xd0, vec![0xff]),
        (0xe1, vec![0xff; 7]),
        (0xf7, vec![0xff; 8]),
        (0xff, vec![0x11, 0x22]),
    ] {
        write(&mut old, register, &bytes);
        write(&mut new, register, &bytes);
        assert_eq!(
            read(&mut old, 0, 256),
            read(&mut new, 0, 256),
            "write {register:#x}"
        );
    }
    assert_eq!(read(&mut new, 0xf2, 4), vec![7, 0, 0x27, 0xa5]);
    assert_eq!(read(&mut old, 0xfd, 9), read(&mut new, 0xfd, 9));
    // STOP releases pointer-write framing but retains the next read address.
    for device in [&mut old as &mut dyn I2cDevice, &mut new] {
        device.stop();
        device.write(0xd0);
        device.stop();
        device.start();
        assert_eq!(device.read(), 0x60);
        assert_eq!(device.read(), 0);
    }
}

#[test]
fn stimulus_updates_rederive_all_raw_channels_exactly() {
    let (mut old, mut new) = models(0x76);
    assert_measurements(&mut old, &mut new, "defaults");
    let mut samples = vec![
        [-40.0, 0.0, 300.0],
        [85.0, 100.0, 1100.0],
        [25.0, 50.0, 1013.25],
        [-20.25, 12.5, 450.0],
        [63.75, 88.125, 1080.0],
    ];
    let mut rng = 0x18b2_05adu32;
    for _ in 0..256 {
        let mut unit = || {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            f64::from(rng) / f64::from(u32::MAX)
        };
        samples.push([
            -40.0 + unit() * 125.0,
            unit() * 100.0,
            300.0 + unit() * 800.0,
        ]);
    }
    for sample in samples {
        for (key, value) in ["temperature", "humidity", "pressure"]
            .into_iter()
            .zip(sample)
        {
            old.set_input(key, value).unwrap();
            new.set_input(key, value).unwrap();
            // Check after EACH update, not only after all three: a temperature
            // update must also re-invert humidity and pressure using t_fine.
            assert_measurements(&mut old, &mut new, &format!("{key}={value}"));
        }
    }
}

#[test]
fn target_quantization_ties_and_rejected_inputs_match() {
    let (mut old, mut new) = models(0x76);
    for (key, targets, scale) in [
        (
            "temperature",
            vec![-3999.5, -0.5, 0.5, 2499.5, 8499.5],
            100.0,
        ),
        ("humidity", vec![0.5, 1023.5, 51199.5, 102399.5], 1024.0),
        ("pressure", vec![7680000.5, 25939199.5, 28159999.5], 25600.0),
    ] {
        for target in targets {
            for delta in [-1e-10, 0.0, 1e-10] {
                let value = target / scale + delta;
                old.set_input(key, value).unwrap();
                new.set_input(key, value).unwrap();
                assert_measurements(&mut old, &mut new, &format!("quantization {key}={value}"));
            }
        }
    }
    for (key, value) in [
        ("temperature", -40.01),
        ("temperature", 85.01),
        ("humidity", -0.01),
        ("humidity", 100.01),
        ("pressure", 299.99),
        ("pressure", 1100.01),
        ("humidity", f64::INFINITY),
        ("missing", 0.0),
    ] {
        assert!(old.set_input(key, value).is_err());
        assert!(new.set_input(key, value).is_err());
        assert_measurements(&mut old, &mut new, "rejected update preserves state");
    }
}

#[test]
fn nan_is_rejected_explicitly_instead_of_silently_becoming_an_adc_endpoint() {
    let (mut old, mut new) = models(0x76);
    let old_before = read(&mut old, 0xf7, 8);
    let before = read(&mut new, 0xf7, 8);
    // Shared input validation now protects both the Rust and declarative models.
    assert!(old.set_input("temperature", f64::NAN).is_err());
    assert!(new.set_input("temperature", f64::NAN).is_err());
    assert_eq!(read(&mut old, 0xf7, 8), old_before);
    assert_eq!(read(&mut new, 0xf7, 8), before);
}

#[test]
fn each_inverse_is_live_not_a_constant_register_or_rust_thunk() {
    let mut old = Bme280::new(0x76);
    let expected = read(&mut old, 0xf7, 8);
    for (from, to) in [
        ("temperature * 100", "temperature * 90"),
        ("pressure * 100 * 256", "pressure * 100 * 250"),
        ("humidity * 1024", "humidity * 1000"),
    ] {
        let source = yaml();
        assert!(source.contains(from));
        let mut changed = GenericI2cDevice::from_yaml(&source.replace(from, to), 0x76).unwrap();
        assert_ne!(read(&mut changed, 0xf7, 8), expected, "sabotage {from}");
    }
}
