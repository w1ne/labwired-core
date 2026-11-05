// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Focused pressure, switch, STM32 pad and pneumatic regression target.
//! Links the normal core library to keep test-build memory bounded.

mod xgzp6857d {
    use labwired_core::peripherals::components::xgzp6857d::*;
    use labwired_core::peripherals::i2c::I2cDevice;
    use labwired_core::sim_input::SimInput;

    fn command(sensor: &mut Xgzp6857d, value: u8) {
        sensor.start();
        sensor.write(0x30);
        sensor.write(value);
        sensor.stop();
    }

    fn read_register(sensor: &mut Xgzp6857d, register: u8) -> u8 {
        sensor.start();
        sensor.write(register);
        sensor.start();
        sensor.read()
    }

    fn pressure_raw(sensor: &mut Xgzp6857d) -> i32 {
        let high = read_register(sensor, 0x06);
        let mid = sensor.read();
        let low = sensor.read();
        i32::from_be_bytes([if high & 0x80 == 0 { 0 } else { 0xff }, high, mid, low])
    }

    #[test]
    fn signed_samples_use_the_specific_parts_pressure_scale() {
        let mut sensor = Xgzp6857d::new();
        sensor.set_input("pressure_pa", -50_000.0).unwrap();
        sensor.set_input("temperature_c", -12.5).unwrap();
        command(&mut sensor, 0x0a);
        sensor.advance_time_us(20_000);
        assert_eq!(pressure_raw(&mut sensor), -1_600_000);
        let temperature = i16::from_be_bytes([sensor.read(), sensor.read()]);
        assert_eq!(temperature, -3200);
        sensor.set_input("pressure_pa", 200_000.0).unwrap();
        command(&mut sensor, 0x0a);
        sensor.advance_time_us(20_000);
        assert_eq!(pressure_raw(&mut sensor), 6_400_000);
    }

    #[test]
    fn polling_does_not_advance_time_and_samples_publish_at_completion() {
        let mut sensor = Xgzp6857d::new();
        command(&mut sensor, 0x0a);
        for _ in 0..100 {
            assert_eq!(read_register(&mut sensor, 0x30), 0x0a);
        }
        sensor.set_input("pressure_pa", -40_000.0).unwrap();
        sensor.advance_time_us(19_999);
        assert_eq!(pressure_raw(&mut sensor), 0);
        assert_eq!(read_register(&mut sensor, 0x30), 0x0a);
        sensor.advance_time_us(1);
        assert_eq!(read_register(&mut sensor, 0x30), 0x02);
        assert_eq!(pressure_raw(&mut sensor), -1_280_000);
        sensor.set_input("pressure_pa", -60_000.0).unwrap();
        assert_eq!(pressure_raw(&mut sensor), -1_280_000);
    }

    #[test]
    fn periodic_command_keeps_busy_and_sleeps_between_conversions() {
        let mut sensor = Xgzp6857d::new();
        sensor.set_input("pressure_pa", -10_000.0).unwrap();
        command(&mut sensor, 0x1b);
        sensor.advance_time_us(20_000);
        assert_eq!(pressure_raw(&mut sensor), -320_000);
        assert_eq!(read_register(&mut sensor, 0x30), 0x1b);
        sensor.set_input("pressure_pa", -20_000.0).unwrap();
        sensor.advance_time_us(62_500 + 19_999);
        assert_eq!(pressure_raw(&mut sensor), -320_000);
        sensor.advance_time_us(1);
        assert_eq!(pressure_raw(&mut sensor), -640_000);
        command(&mut sensor, 0x00);
        sensor.set_input("pressure_pa", 0.0).unwrap();
        sensor.advance_time_us(1_000_000);
        assert_eq!(pressure_raw(&mut sensor), -640_000);
    }

    #[test]
    fn range_validation_and_read_only_output_registers() {
        let mut sensor = Xgzp6857d::new();
        assert!(sensor.set_input("pressure_pa", f64::NAN).is_err());
        assert!(sensor.set_input("pressure_pa", -100_001.0).is_err());
        assert!(sensor.set_input("temperature_c", 101.0).is_err());
        assert!(sensor.set_input("unknown", 0.0).is_err());
        sensor.start();
        sensor.write(0x06);
        sensor.write(0xff);
        assert_eq!(pressure_raw(&mut sensor), 0);
    }
}

mod pca9545a {
    use labwired_core::peripherals::components::pca9545a::*;
    use labwired_core::peripherals::components::xgzp6857d::Xgzp6857d;
    use labwired_core::peripherals::i2c::I2cDevice;
    use labwired_core::sim_input::SimInput;

    fn select_channels(mux: &mut Pca9545a, mask: u8) {
        mux.select_address(mux.address());
        mux.start();
        mux.write(mask);
        mux.stop();
    }

    fn command(mux: &mut Pca9545a, command: u8) {
        mux.select_address(0x6d);
        mux.start();
        mux.write(0x30);
        mux.write(command);
        mux.stop();
    }

    fn read_pressure_bytes(mux: &mut Pca9545a) -> [u8; 3] {
        mux.select_address(0x6d);
        mux.start();
        mux.write(0x06);
        mux.start();
        [mux.read(), mux.read(), mux.read()]
    }

    #[test]
    fn channel_mask_becomes_active_only_after_stop() {
        let mut mux = Pca9545a::default();
        mux.attach(0, Box::new(Xgzp6857d::new())).unwrap();
        assert!(!mux.claims_address(0x6d));
        mux.select_address(0x70);
        mux.start();
        mux.write(1);
        assert_eq!(mux.control_register(), 1);
        assert!(!mux.claims_address(0x6d));
        mux.select_address(0x6d);
        mux.start();
        assert_eq!(mux.read(), 0xff);
        mux.stop();
        assert!(mux.claims_address(0x6d));
    }

    #[test]
    fn same_address_sensors_are_independent_and_disabled_sensor_keeps_converting() {
        let mut mux = Pca9545a::default();
        for (channel, pressure) in [(0, -30_000.0), (1, -60_000.0)] {
            let mut sensor = Xgzp6857d::new();
            sensor.set_input("pressure_pa", pressure).unwrap();
            sensor.set_component_id(format!("vac{}", channel + 1));
            mux.attach(channel, Box::new(sensor)).unwrap();
        }
        select_channels(&mut mux, 1);
        command(&mut mux, 0x0a);
        select_channels(&mut mux, 2);
        command(&mut mux, 0x0a);
        mux.advance_time_us(20_000);
        let second = read_pressure_bytes(&mut mux);
        assert_eq!(second, (-1_920_000i32).to_be_bytes()[1..]);
        select_channels(&mut mux, 1);
        let first = read_pressure_bytes(&mut mux);
        assert_eq!(first, (-960_000i32).to_be_bytes()[1..]);
        select_channels(&mut mux, 3);
        let collision = read_pressure_bytes(&mut mux);
        assert_eq!(collision, std::array::from_fn(|i| first[i] & second[i]));
        let mut ids = Vec::new();
        mux.for_each_sim_input(&mut |input| {
            ids.push(input.component_id().unwrap().to_owned());
            false
        });
        assert_eq!(ids, ["vac1", "vac2"]);
    }

    #[test]
    fn interrupt_bits_are_read_only_and_reset_is_active_low_input() {
        let mut mux = Pca9545a::new(0x73);
        mux.set_interrupt_asserted(2, true).unwrap();
        select_channels(&mut mux, 0xf3);
        mux.select_address(0x73);
        assert_eq!(mux.read(), 0x43);
        assert!(mux.interrupt_asserted());
        mux.set_reset_asserted(true);
        assert!(!mux.claims_address(0x73));
        assert!(!mux.channel_enabled(0));
        mux.set_reset_asserted(false);
        assert!(mux.claims_address(0x73));
        assert_eq!(mux.control_register(), 0x40);
        assert!(mux.attach(4, Box::new(Xgzp6857d::new())).is_err());
    }
}

mod step_dir {
    use labwired_core::peripherals::components::step_dir_motor::{StepDirConfig, StepDirMotor};
    #[test]
    fn tmc2226_alias_and_configured_microstep_angle() {
        assert!(labwired_core::peripherals::kit::registry::lookup("tmc2226").is_some());
        let motor = StepDirMotor::new("nozzle", 2, 3, Some(4))
            .with_device_type("tmc2226")
            .with_config(StepDirConfig {
                degrees_per_step: 360.0 / 1600.0,
                enable_active_low: true,
            });
        motor.on_gpio_edge(3, true, 0);
        for step in 0..400 {
            motor.on_gpio_edge(2, true, step * 2 + 1);
            motor.on_gpio_edge(2, false, step * 2 + 2);
        }
        assert_eq!(motor.steps(), 400);
        assert_eq!(motor.angle_degrees_f64(), 90.0);
        motor.on_gpio_edge(4, true, 801);
        motor.on_gpio_edge(2, true, 802);
        assert_eq!(motor.steps(), 400);
    }

    #[test]
    fn stm32_cross_port_gpio_writes_drive_steps_without_a_peripheral_tick() {
        use labwired_core::Bus;
        let chip = labwired_config::ChipDescriptor::from_file(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../configs/chips/stm32f407.yaml"),
        )
        .unwrap();
        let mut manifest: labwired_config::SystemManifest = serde_yaml::from_str(
            r#"
name: lumenpnp-stepper-regression
chip: stm32f407
external_devices:
  - id: x
    type: tmc2226
    connection: gpio
    config:
      step_pin: PB15
      dir_pin: PB14
      en_pin: PD9
      degrees_per_step: 0.1125
"#,
        )
        .unwrap();
        let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).unwrap();
        let motor = bus.observed_arcs_of::<StepDirMotor>().next().unwrap();
        assert_eq!(motor.degrees_per_step(), 0.1125);
        // A high output latch on an input-mode pad cannot clock the driver.
        bus.write_u32(0x4002_0414, (1 << 15) | (1 << 14)).unwrap();
        assert_eq!(motor.steps(), 0);
        bus.write_u32(0x4002_0414, 0).unwrap();
        // GPIOB and GPIOD clocks, STEP/DIR output mode, and EN output mode.
        bus.write_u32(0x4002_3830, (1 << 1) | (1 << 3)).unwrap();
        bus.write_u32(0x4002_0400, (1 << 30) | (1 << 28)).unwrap();
        bus.write_u32(0x4002_0c00, 1 << 18).unwrap();
        // A single store sets DIR and raises STEP. The driver sees new DIR.
        bus.write_u32(0x4002_0418, (1 << 14) | (1 << 15)).unwrap();
        assert_eq!(motor.steps(), 1);
        bus.write_u32(0x4002_0418, 1 << 31).unwrap();
        bus.write_u32(0x4002_0418, 1 << 15).unwrap();
        assert_eq!(motor.steps(), 2);
        // EN lives on a separate GPIO port and blocks the next STEP edge.
        bus.write_u32(0x4002_0c18, 1 << 9).unwrap();
        bus.write_u32(0x4002_0418, 1 << 31).unwrap();
        bus.write_u32(0x4002_0418, 1 << 15).unwrap();
        assert_eq!(motor.steps(), 2);
        assert!(!motor.is_enabled());
        // Re-enable and reverse through the same physical pad binding.
        bus.write_u32(0x4002_0c18, 1 << 25).unwrap();
        bus.write_u32(0x4002_0418, (1 << 31) | (1 << 30)).unwrap();
        bus.write_u32(0x4002_0418, 1 << 15).unwrap();
        assert_eq!(motor.steps(), 1);
        assert_eq!(motor.angle_degrees_f64(), 0.1125);
        manifest.external_devices[0]
            .config
            .remove("degrees_per_step");
        for (device_type, expected_angle) in [("tmc2226", 0.225), ("tmc2209", 0.1125)] {
            manifest.external_devices[0].r#type = device_type.to_owned();
            let bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).unwrap();
            assert_eq!(
                bus.observed_of::<StepDirMotor>()
                    .next()
                    .unwrap()
                    .degrees_per_step(),
                expected_angle
            );
        }
        manifest.external_devices[0]
            .config
            .insert("degrees_per_step".to_owned(), serde_yaml::Value::from(0.0));
        assert!(labwired_core::bus::SystemBus::from_config(&chip, &manifest).is_err());
    }

    #[test]
    fn stm32_same_bit_number_on_distinct_ports_keeps_step_and_dir_separate() {
        use labwired_core::Bus;
        let chip = labwired_config::ChipDescriptor::from_file(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../configs/chips/stm32f407.yaml"),
        )
        .unwrap();
        let manifest: labwired_config::SystemManifest = serde_yaml::from_str(
            r#"
name: lumenpnp-nozzle-pad-regression
chip: stm32f407
external_devices:
  - id: A
    type: tmc2226
    connection: gpioc
    config:
      step_pin: PC4
      dir_pin: PA4
      en_pin: PB0
"#,
        )
        .unwrap();
        let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).unwrap();
        let motor = bus.observed_arcs_of::<StepDirMotor>().next().unwrap();
        bus.write_u32(0x4002_3830, 0x07).unwrap();
        bus.write_u32(0x4002_0000, 1 << 8).unwrap();
        bus.write_u32(0x4002_0400, 1).unwrap();
        bus.write_u32(0x4002_0800, 1 << 8).unwrap();
        bus.write_u32(0x4002_0018, 1 << 4).unwrap();
        assert_eq!(motor.steps(), 0, "DIR changes must never clock STEP");
        bus.write_u32(0x4002_0818, 1 << 4).unwrap();
        assert_eq!(motor.steps(), 1);
        bus.write_u32(0x4002_0818, 1 << 20).unwrap();
        bus.write_u32(0x4002_0018, 1 << 20).unwrap();
        assert_eq!(
            motor.steps(),
            1,
            "reverse DIR must leave the step count unchanged"
        );
        bus.write_u32(0x4002_0818, 1 << 4).unwrap();
        assert_eq!(motor.steps(), 0);
        bus.write_u32(0x4002_0418, 1).unwrap();
        bus.write_u32(0x4002_0818, 1 << 20).unwrap();
        bus.write_u32(0x4002_0818, 1 << 4).unwrap();
        assert_eq!(motor.steps(), 0, "separate-port EN blocks steps");
    }
}

mod factory {
    use labwired_core::peripherals::components::i2c_factory;
    #[test]
    fn lumenpnp_pressure_tree_preserves_fixed_addresses_and_four_channel_limit() {
        let mut manifest: labwired_config::SystemManifest = serde_yaml::from_str(
            r#"
name: lumenpnp-pressure
chip: stm32f407
external_devices:
  - id: vac_mux
    type: pca9545a
    connection: i2c1
  - id: vac1
    type: xgzp6857d
    connection: vac_mux
    channel: 0
  - id: vac2
    type: xgzp6857d
    connection: vac_mux
    channel: 1
"#,
        )
        .unwrap();
        assert_eq!(
            i2c_factory::validate_i2c_mux_topology(&manifest).unwrap(),
            ["vac1", "vac2"]
        );
        let mut device = i2c_factory::build_i2c_tree(&manifest, &manifest.external_devices[0])
            .unwrap()
            .unwrap();
        let mux = device
            .as_any_mut()
            .unwrap()
            .downcast_mut::<labwired_core::peripherals::components::pca9545a::Pca9545a>()
            .unwrap();
        assert_eq!(mux.channel_devices(0)[0].address(), 0x6d);
        assert_eq!(mux.channel_devices(1)[0].address(), 0x6d);
        let mut ids = Vec::new();
        device.for_each_sim_input(&mut |input| {
            ids.push(input.component_id().unwrap().to_owned());
            false
        });
        assert_eq!(ids, ["vac1", "vac2"]);
        manifest.external_devices[2].channel = Some(4);
        assert!(i2c_factory::validate_i2c_mux_topology(&manifest).is_err());
    }

    #[test]
    fn lumenpnp_parts_reject_incompatible_i2c_addresses() {
        let config = std::collections::HashMap::new();
        assert_eq!(
            i2c_factory::build_i2c_device("xgzp6857d", &config)
                .unwrap()
                .address(),
            0x6d
        );
        let mut config = config;
        config.insert("i2c_address".to_owned(), serde_yaml::Value::from(0x58));
        assert!(i2c_factory::build_i2c_device("xgzp6857d", &config).is_none());
        config.insert("i2c_address".to_owned(), serde_yaml::Value::from(0x74));
        assert!(i2c_factory::build_i2c_device("pca9545a", &config).is_none());
    }
}

#[path = "../examples/lumenpnp/vacuum.rs"]
mod vacuum;
