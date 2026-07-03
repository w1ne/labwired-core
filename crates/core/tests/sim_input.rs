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
    let bus = kw41z_lcd_bus();
    let inputs = bus.list_inputs();
    let keys: Vec<_> = inputs.iter().map(|(_, ch)| ch.key).collect();
    assert!(keys.contains(&"x"), "expected an x channel, got {keys:?}");
    assert!(keys.contains(&"y"));
    assert!(keys.contains(&"z"));
    // Channels carry discovery metadata (unit + range) for agents.
    let x = inputs.iter().find(|(_, c)| c.key == "x").unwrap().1;
    assert_eq!(x.unit, "g");
    assert_eq!((x.min, x.max), (-2.0, 2.0));
}

#[test]
fn set_input_drives_the_device_by_channel_name() {
    let mut bus = kw41z_lcd_bus();

    // Drive purely by channel name — no device type, address, or bus known.
    bus.set_input("x", 1.0).expect("set x to +1 g");
    bus.set_input("y", -0.5).expect("set y to -0.5 g");

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

    match bus.set_input("nope", 0.0) {
        Err(SimInputError::NoDevice(c)) => assert_eq!(c, "nope"),
        other => panic!("expected NoDevice, got {other:?}"),
    }

    match bus.set_input("x", 9.0) {
        Err(SimInputError::OutOfRange { key, max, .. }) => {
            assert_eq!(key, "x");
            assert_eq!(max, 2.0);
        }
        other => panic!("expected OutOfRange, got {other:?}"),
    }
}
