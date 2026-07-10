// Integration test for the generic simulated-input interface
// (`crate::sim_input`): an agent / bridge / test-script can discover and drive
// an input device's channels by name, without knowing the concrete type, on a
// real board built from config.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::Fxos8700;
use labwired_core::peripherals::i2c::{I2c, I2cDevice};
use labwired_core::sim_input::SimInputError;
use std::path::PathBuf;

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    crate_root()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Build the FRDM-KW41Z-LCD bus (the cow demo board): a KinetisI2c with the
/// FXOS8700 accelerometer attached at 0x1f.
fn kw41z_lcd_bus() -> SystemBus {
    let chip = ChipDescriptor::from_file(workspace_root().join("configs/chips/mkw41z4.yaml"))
        .expect("load mkw41z4 chip");
    let sys_path = workspace_root().join("configs/systems/frdm-kw41z-lcd.yaml");
    let mut manifest = SystemManifest::from_file(&sys_path).expect("load frdm-kw41z-lcd system");
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

/// Read one 14-bit left-justified accel axis back out of the FXOS8700 over its
/// I²C register interface — proving a driven value actually reaches the model's
/// register file, not just an internal field.
fn read_axis(bus: &mut SystemBus, msb_reg: u8) -> i16 {
    for entry in bus.peripherals.iter_mut() {
        let Some(any) = entry.dev.as_any_mut() else {
            continue;
        };
        let Some(i2c) = any.downcast_mut::<I2c>() else {
            continue;
        };
        for cell in i2c.attached_devices() {
            let mut dev = cell.borrow_mut();
            if let Some(any) = dev.as_any_mut() {
                if let Some(fxos) = any.downcast_mut::<Fxos8700>() {
                    fxos.stop(); // reset the register-pointer phase (fresh transaction)
                    fxos.write(msb_reg);
                    let hi = fxos.read() as i16;
                    let lo = fxos.read() as i16;
                    return (hi << 8) | (lo & 0xFF);
                }
            }
        }
    }
    panic!("no FXOS8700 on the bus");
}

#[test]
fn lists_the_accelerometer_channels() {
    let mut bus = kw41z_lcd_bus();
    let inputs = bus.list_inputs();
    // Discovery reports the system.yaml external-device id, not the bus name —
    // the same vocabulary set_input's `component` accepts.
    assert!(
        inputs.iter().all(|(owner, _)| owner == "fxos8700"),
        "expected owners to be the external-device id, got {inputs:?}"
    );
    let keys: Vec<_> = inputs.iter().map(|(_, ch)| ch.key).collect();
    assert!(keys.contains(&"x"), "expected an x channel, got {keys:?}");
    assert!(keys.contains(&"y"));
    assert!(keys.contains(&"z"));
    // Channels carry discovery metadata (unit + range) for agents.
    let x = inputs.iter().find(|(_, c)| c.key == "x").unwrap().1;
    assert_eq!(x.unit, "g");
    // Schema range = hardware max full-scale (±8 g); the conversion follows
    // the live xyz_data_cfg FS bits (±2 g at reset).
    assert_eq!((x.min, x.max), (-8.0, 8.0));
}

#[test]
fn set_input_drives_the_device_by_channel_name() {
    let mut bus = kw41z_lcd_bus();

    // Drive purely by channel name — no device type, address, or bus known.
    bus.set_input(None, "x", 1.0).expect("set x to +1 g");
    bus.set_input(None, "y", -0.5).expect("set y to -0.5 g");

    // 1 g = 0x1000 (4096) raw, left-justified 14-bit; read it back over I²C.
    // OUT_X_MSB = 0x01, OUT_Y_MSB = 0x03.
    assert_eq!(read_axis(&mut bus, 0x01), 4096, "x should read +1 g");
    assert_eq!(read_axis(&mut bus, 0x03), -2048, "y should read -0.5 g");

    // The driven value must STICK across reads (manual latch beats the
    // built-in animation) — the property the demo needs.
    assert_eq!(read_axis(&mut bus, 0x01), 4096, "x must not animate away");
}

#[test]
fn set_input_rejects_unknown_channel_and_out_of_range() {
    let mut bus = kw41z_lcd_bus();

    match bus.set_input(None, "nope", 0.0) {
        Err(SimInputError::NoDevice(c)) => assert_eq!(c, "nope"),
        other => panic!("expected NoDevice, got {other:?}"),
    }

    match bus.set_input(None, "x", 9.0) {
        Err(SimInputError::OutOfRange { key, max, .. }) => {
            assert_eq!(key, "x");
            assert_eq!(max, 8.0);
        }
        other => panic!("expected OutOfRange, got {other:?}"),
    }
}

// ── Multi-transport coverage ─────────────────────────────────────────────────
// One synthetic STM32F103 board carrying an input device on EVERY attachment
// point the generic input walk covers: I²C devices, SPI devices, UART streams,
// and bus-direct sensors (HC-SR04) — plus deliberate channel-key collisions to
// exercise `component` disambiguation.

use labwired_core::peripherals::components::{
    Adxl345, Max31855, Mpu6050, Neo6mGps, Sn74hc165, Vl53l1x,
};
use labwired_core::peripherals::spi::Spi;
use labwired_core::peripherals::uart::Uart;

fn f103_input_matrix_bus() -> SystemBus {
    let chip = ChipDescriptor::from_file(workspace_root().join("configs/chips/stm32f103.yaml"))
        .expect("load stm32f103 chip");
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "sim-input-matrix"
chip: "../chips/stm32f103.yaml"
external_devices:
  - id: "imu"
    type: "mpu6050"
    connection: "i2c1"
  - id: "accel1"
    type: "adxl345"
    connection: "i2c1"
    config:
      i2c_address: 0x53
  - id: "accel2"
    type: "adxl345"
    connection: "i2c1"
    config:
      i2c_address: 0x1D
  - id: "tof"
    type: "vl53l1x"
    connection: "i2c1"
  - id: "thermo1"
    type: "max31855"
    connection: "spi1"
  - id: "thermo2"
    type: "max31855"
    connection: "spi2"
  - id: "dio"
    type: "sn74hc165"
    connection: "spi2"
    config:
      cs_pin: "PA3"
  - id: "gps"
    type: "neo6m-gps"
    connection: "uart1"
  - id: "sonar"
    type: "hc-sr04"
    connection: "gpioa"
    config:
      trig_pin: "PA8"
      echo_pin: "PA9"
"#,
    )
    .expect("parse matrix system yaml");
    SystemBus::from_config(&chip, &manifest).expect("build matrix bus")
}

/// Find the unique attached device of concrete type `T` on the bus and run
/// `f` on it — readback that proves a driven value reached the MODEL, not
/// just the walk's bookkeeping.
fn with_device<T: 'static, R>(bus: &mut SystemBus, owner: &str, f: impl FnOnce(&mut T) -> R) -> R {
    for entry in bus.peripherals.iter_mut() {
        if entry.name != owner {
            continue;
        }
        let Some(any) = entry.dev.as_any_mut() else {
            continue;
        };
        if let Some(i2c) = any.downcast_ref::<I2c>() {
            for cell in i2c.attached_devices() {
                let mut dev = cell.borrow_mut();
                if let Some(t) = dev.as_any_mut().and_then(|a| a.downcast_mut::<T>()) {
                    return f(t);
                }
            }
        } else if let Some(spi) = any.downcast_mut::<Spi>() {
            for dev in spi.attached_devices.iter_mut() {
                if let Some(t) = dev.as_any_mut().and_then(|a| a.downcast_mut::<T>()) {
                    return f(t);
                }
            }
        } else if let Some(uart) = any.downcast_mut::<Uart>() {
            for stream in uart.attached_streams.iter_mut() {
                if let Some(t) = stream.as_any_mut().and_then(|a| a.downcast_mut::<T>()) {
                    return f(t);
                }
            }
        }
    }
    panic!("no device of the requested type on '{owner}'");
}

#[test]
fn lists_channels_across_all_transports() {
    let mut bus = f103_input_matrix_bus();
    let inputs = bus.list_inputs();
    let pairs: Vec<(String, &str)> = inputs
        .iter()
        .map(|(owner, ch)| (owner.clone(), ch.key))
        .collect();

    for expected in [
        ("imu", "ax"), // MPU6050 (I²C device)
        ("imu", "gz"),
        ("accel1", "x"),            // first ADXL345
        ("accel2", "x"),            // second ADXL345, same bus
        ("tof", "distance"),        // VL53L1X (I²C device)
        ("thermo1", "temperature"), // MAX31855 (SPI device)
        ("thermo2", "temperature"), // second MAX31855
        ("dio", "ch0"),             // 74HC165 (SPI device)
        ("gps", "lat"),             // NEO-6M (UART stream)
        ("gps", "fix"),
        ("sonar", "distance"), // HC-SR04 (bus-direct)
    ] {
        assert!(
            pairs.iter().any(|(o, k)| (o.as_str(), *k) == expected),
            "missing {expected:?} in {pairs:?}"
        );
    }
}

#[test]
fn drives_each_transport_through_the_generic_api() {
    let mut bus = f103_input_matrix_bus();

    // I²C device (unique key): value must reach the model's register scale.
    bus.set_input(None, "ax", 1.0).expect("drive imu ax");
    let (ax, ..) = with_device::<Mpu6050, _>(&mut bus, "i2c1", |imu| imu.sample());
    assert_eq!(ax, 16384, "1 g at power-on scale = 16384 LSB");

    // SPI device (unique key): single 74HC165 channel goes high.
    bus.set_input(None, "ch3", 1.0).expect("drive dio ch3");
    let dio = with_device::<Sn74hc165, _>(&mut bus, "spi2", |sr| sr.inputs());
    assert_eq!(dio, 0b0000_1000);

    // UART stream (unique key): GPS latitude lands in the NMEA source.
    bus.set_input(None, "lat", 50.45).expect("drive gps lat");
    let (lat, lon) = with_device::<Neo6mGps, _>(&mut bus, "uart1", |gps| gps.position());
    assert_eq!(lat, 50.45);
    assert_ne!(lon, 0.0, "driving lat must preserve lon");
}

#[test]
fn component_disambiguates_colliding_channel_keys() {
    let mut bus = f103_input_matrix_bus();

    // "temperature" exists on both MAX31855s; "distance" on both the VL53L1X
    // (mm) and the HC-SR04 (cm). Undirected sets must fail loudly…
    match bus.set_input(None, "temperature", 300.0) {
        Err(SimInputError::Ambiguous { matches, .. }) => assert_eq!(matches, 2),
        other => panic!("expected Ambiguous, got {other:?}"),
    }
    match bus.set_input(None, "distance", 100.0) {
        Err(SimInputError::Ambiguous { matches, .. }) => assert_eq!(matches, 2),
        other => panic!("expected Ambiguous, got {other:?}"),
    }

    // …and component-directed sets must hit exactly the named owner.
    bus.set_input(Some("spi2"), "temperature", 300.0)
        .expect("drive thermo2");
    let (tc2, _) = with_device::<Max31855, _>(&mut bus, "spi2", |t| t.temperature());
    assert_eq!(tc2, 300.0);
    let (tc1, _) = with_device::<Max31855, _>(&mut bus, "spi1", |t| t.temperature());
    assert_eq!(tc1, 25.0, "spi1 thermocouple must keep its default");

    bus.set_input(Some("i2c1"), "distance", 250.0)
        .expect("drive tof");
    let mm = with_device::<Vl53l1x, _>(&mut bus, "i2c1", |tof| tof.distance_mm());
    assert_eq!(mm, 250);

    bus.set_input(Some("sonar"), "distance", 123.0)
        .expect("drive sonar");
    assert_eq!(bus.hcsr04[0].distance_cm(), 123.0);

    // Two devices of the SAME type on the SAME bus: the peripheral name can't
    // split them, but each device's stamped id can.
    match bus.set_input(None, "x", 1.0) {
        Err(SimInputError::Ambiguous { matches, .. }) => assert_eq!(matches, 2),
        other => panic!("expected Ambiguous, got {other:?}"),
    }
    match bus.set_input(Some("i2c1"), "x", 1.0) {
        Err(SimInputError::Ambiguous { matches, .. }) => assert_eq!(matches, 2),
        other => panic!("expected Ambiguous for the shared bus name, got {other:?}"),
    }
    bus.set_input(Some("accel2"), "x", 1.0)
        .expect("drive accel2");
    let (x2, ..) = with_i2c_device_at::<Adxl345, _>(&mut bus, 0x1D, |a| a.sample());
    assert_eq!(x2, 256, "1 g full-res = 256 LSB");
    let (x1, ..) = with_i2c_device_at::<Adxl345, _>(&mut bus, 0x53, |a| a.sample());
    assert_eq!(x1, 0, "accel1 must be untouched");

    // A component that doesn't own the channel is a NoDevice, not a fallback.
    match bus.set_input(Some("uart1"), "temperature", 30.0) {
        Err(SimInputError::NoDevice(m)) => assert_eq!(m, "uart1/temperature"),
        other => panic!("expected NoDevice, got {other:?}"),
    }
}

/// Like `with_device` but selects an I²C device by address — needed when two
/// devices of the same concrete type share a bus.
fn with_i2c_device_at<T: 'static, R>(
    bus: &mut SystemBus,
    address: u8,
    f: impl FnOnce(&mut T) -> R,
) -> R {
    for entry in bus.peripherals.iter_mut() {
        let Some(any) = entry.dev.as_any_mut() else {
            continue;
        };
        let Some(i2c) = any.downcast_ref::<I2c>() else {
            continue;
        };
        for cell in i2c.attached_devices() {
            let mut dev = cell.borrow_mut();
            if dev.address() != address {
                continue;
            }
            if let Some(t) = dev.as_any_mut().and_then(|a| a.downcast_mut::<T>()) {
                return f(t);
            }
        }
    }
    panic!("no device of the requested type at 0x{address:02x}");
}

#[test]
fn conversion_follows_live_fullscale_config() {
    let mut bus = f103_input_matrix_bus();

    // Power-on scale: ±2 g at 16384 LSB/g.
    bus.set_input(None, "ax", 1.0)
        .expect("drive ax at reset scale");
    let (ax, ..) = with_device::<Mpu6050, _>(&mut bus, "i2c1", |imu| imu.sample());
    assert_eq!(ax, 16384);

    // Firmware reconfigures ACCEL_CONFIG to ±8 g (AFS_SEL=2) over I²C; the
    // same engineering value must now land at the new scale (4096 LSB/g),
    // and values valid at ±8 g must be accepted.
    with_device::<Mpu6050, _>(&mut bus, "i2c1", |imu| {
        use labwired_core::peripherals::i2c::I2cDevice;
        imu.stop();
        imu.write(0x1C);
        imu.write(0x10);
        imu.stop();
    });
    bus.set_input(None, "ax", 4.0)
        .expect("4 g is valid at +/-8 g FS");
    let (ax, ..) = with_device::<Mpu6050, _>(&mut bus, "i2c1", |imu| imu.sample());
    assert_eq!(
        ax,
        4 * 4096,
        "conversion must follow the live AFS_SEL scale"
    );

    // Beyond the configured full-scale the value saturates like the silicon.
    bus.set_input(None, "ax", 16.0)
        .expect("schema allows up to hardware max");
    let (ax, ..) = with_device::<Mpu6050, _>(&mut bus, "i2c1", |imu| imu.sample());
    // +8 g = 32768 saturates to i16::MAX — the same asymmetry as the silicon.
    assert_eq!(
        ax,
        i16::MAX,
        "must clamp at the configured +/-8 g full-scale"
    );
}

#[test]
fn set_inputs_is_all_or_nothing() {
    let mut bus = f103_input_matrix_bus();

    // One bad set (ay beyond hardware max) must abort the WHOLE batch.
    match bus.set_inputs(&[(None, "ax", 1.0), (None, "ay", 99.0)]) {
        Err(SimInputError::OutOfRange { key, .. }) => assert_eq!(key, "ay"),
        other => panic!("expected OutOfRange, got {other:?}"),
    }
    let (ax, ..) = with_device::<Mpu6050, _>(&mut bus, "i2c1", |imu| imu.sample());
    assert_eq!(ax, 0x0123, "failed batch must leave ax at its default");

    // A valid batch applies every set.
    bus.set_inputs(&[
        (None, "ax", 1.0),
        (None, "ay", -1.0),
        (Some("gps"), "lat", 50.45),
    ])
    .expect("valid batch");
    let (ax, ay, ..) = with_device::<Mpu6050, _>(&mut bus, "i2c1", |imu| imu.sample());
    assert_eq!((ax, ay), (16384, -16384));
    let (lat, _) = with_device::<Neo6mGps, _>(&mut bus, "uart1", |gps| gps.position());
    assert_eq!(lat, 50.45);
}

#[test]
fn external_device_id_works_as_component() {
    let mut bus = f103_input_matrix_bus();

    // Authors write the external-device id from system.yaml, not the owning
    // peripheral's bus name — both must resolve (the id is stamped onto the
    // model at attach).
    bus.set_input(Some("thermo1"), "temperature", 40.0)
        .expect("drive thermo1 by external-device id");
    let (tc1, _) = with_device::<Max31855, _>(&mut bus, "spi1", |t| t.temperature());
    assert_eq!(tc1, 40.0);

    bus.set_input(Some("tof"), "distance", 777.0)
        .expect("drive tof by external-device id");
    let mm = with_device::<Vl53l1x, _>(&mut bus, "i2c1", |tof| tof.distance_mm());
    assert_eq!(mm, 777);
}
