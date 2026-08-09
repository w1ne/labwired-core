//! Characterizes the production ESP32-S3 GP-SPI to ILI9341 attachment path.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32s3::gpspi::Esp32s3Spi;
use labwired_core::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32s3, Esp32s3Opts,
};

const SYSTEM: &str = r#"
name: esp32s3-doomlike-attach
chip: esp32s3.yaml
external_devices:
  - id: tft
    type: ili9341
    connection: spi2
    config:
      cs_pin: GPIO10
      dc_pin: GPIO11
"#;

#[test]
fn esp32s3_manifest_attaches_ili9341_to_spi2() {
    let manifest = serde_yaml::from_str(SYSTEM).expect("manifest parses");
    let mut bus = SystemBus::new();
    let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

    attach_esp32_external_devices(&mut bus, &manifest)
        .expect("ILI9341 must attach to the production ESP32-S3 SPI2 controller");

    bus.refresh_peripheral_index();
    let index = bus
        .find_peripheral_index_by_name("spi2")
        .expect("production ESP32-S3 bus exposes spi2");
    let spi = bus.peripherals[index]
        .dev
        .as_any()
        .expect("spi2 supports inspection")
        .downcast_ref::<Esp32s3Spi>()
        .expect("spi2 is an ESP32-S3 GP-SPI controller");
    assert_eq!(spi.attached_device_count(), 1);
}
