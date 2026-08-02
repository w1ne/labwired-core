//! An ILI9341 declared on a classic-ESP32 manifest must actually attach.
//!
//! It used not to. The classic-ESP32 external-device factory matched two
//! e-paper controllers and a potentiometer, and every other type fell into an
//! `other =>` arm that logged `tracing::warn!` and `continue`d. Nothing above
//! it saw a failure: `attach_esp32_external_devices` returned `Ok`, the machine
//! built, the firmware ran, and `Adafruit_ILI9341` painted into a panel that
//! was never on the bus. A green run with a blank screen and no error anywhere.
//!
//! That matters most on this exact chip. The ILI9341 the twin models is the
//! panel on Adafruit's 2.4" TFT FeatherWing, a Feather-shaped carrier — so the
//! board it stacks onto is usually a classic-ESP32 Feather, the one family
//! where it silently did nothing.
//!
//! Two properties, because fixing only the first would leave the trap intact
//! for the next device type:
//!   1. a declared ili9341 IS on the bus and answers SPI traffic;
//!   2. a type the factory cannot build is now a hard error, not a warning.

use labwired_core::bus::SystemBus;
use labwired_core::system::xtensa::{attach_esp32_external_devices, configure_xtensa_esp32};

fn manifest_with(device_type: &str) -> labwired_config::SystemManifest {
    serde_yaml::from_str(&format!(
        r#"
name: feather-tft-lab
chip: esp32
external_devices:
  - id: tft
    type: {device_type}
    connection: spi3
    config:
      cs_pin: GPIO15
      dc_pin: GPIO33
"#
    ))
    .expect("manifest parses")
}

#[test]
fn an_ili9341_declared_on_spi3_is_actually_attached() {
    let mut bus = SystemBus::new();
    let _cpu = configure_xtensa_esp32(&mut bus);

    attach_esp32_external_devices(&mut bus, &manifest_with("ili9341"))
        .expect("an ili9341 on spi3 must attach on classic ESP32");

    // `Ok` alone is exactly what the old code returned while skipping the
    // panel, so it proves nothing on its own. Look at the controller itself.
    bus.refresh_peripheral_index();
    let idx = bus
        .find_peripheral_index_by_name("spi3")
        .expect("spi3 exists on a classic-ESP32 bus");
    let any = bus.peripherals[idx]
        .dev
        .as_any_mut()
        .expect("spi3 is downcastable");
    let spi = any
        .downcast_mut::<labwired_core::peripherals::esp32::spi::Esp32Spi>()
        .expect("spi3 is the classic-ESP32 SPI controller");
    assert_eq!(
        spi.attached_devices.len(),
        1,
        "spi3 carries no device: the panel was skipped, not attached",
    );
}

#[test]
fn a_device_type_the_factory_cannot_build_is_an_error_not_a_warning() {
    let mut bus = SystemBus::new();
    let _cpu = configure_xtensa_esp32(&mut bus);

    let err = attach_esp32_external_devices(&mut bus, &manifest_with("definitely_not_a_panel"))
        .expect_err("an unbuildable device type must fail the attach, not be skipped");
    let message = err.to_string();
    assert!(
        message.contains("definitely_not_a_panel"),
        "the error must name the offending type, got: {message}",
    );
}
