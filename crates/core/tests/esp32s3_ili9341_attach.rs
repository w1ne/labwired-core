//! Characterizes the production ESP32-S3 GP-SPI to ILI9341 attachment path.

use labwired_core::bus::SystemBus;
use labwired_core::inspect::InspectOpts;
use labwired_core::Bus;
use labwired_core::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32s3, Esp32s3Opts,
};

const SYSTEM: &str = r#"
name: esp32s3-doomlike-attach
chip: esp32s3.yaml
external_devices:
  - id: tft
    type: ili9341
    connection: spi2_s3
    config:
      cs_pin: GPIO10
      dc_pin: GPIO11
"#;

const GPIO_OUT: u64 = 0x6000_4004;
const SPI2: u64 = 0x6002_4000;
const SPI_CMD: u64 = 0x00;
const SPI_MS_DLEN: u64 = 0x1c;
const SPI_W0: u64 = 0x98;
const SPI_USR: u32 = 1 << 24;

fn set_dc(bus: &mut SystemBus, data: bool) {
    let value = if data { 1 << 11 } else { 0 };
    bus.write_u32(GPIO_OUT, value).expect("drive GPIO11 D/C");
}

fn spi_write(bus: &mut SystemBus, bytes: &[u8]) {
    assert!(!bytes.is_empty() && bytes.len() <= 64);
    for (word_index, chunk) in bytes.chunks(4).enumerate() {
        let mut word = 0u32;
        for (byte_index, byte) in chunk.iter().enumerate() {
            word |= (*byte as u32) << (byte_index * 8);
        }
        bus.write_u32(SPI2 + SPI_W0 + word_index as u64 * 4, word)
            .expect("fill SPI W buffer");
    }
    bus.write_u32(SPI2 + SPI_MS_DLEN, (bytes.len() as u32 * 8) - 1)
        .expect("set SPI transfer length");
    bus.write_u32(SPI2 + SPI_CMD, SPI_USR)
        .expect("launch SPI transfer");
}

fn command(bus: &mut SystemBus, byte: u8) {
    set_dc(bus, false);
    spi_write(bus, &[byte]);
}

fn data(bus: &mut SystemBus, bytes: &[u8]) {
    set_dc(bus, true);
    spi_write(bus, bytes);
}

#[test]
fn esp32s3_manifest_attaches_ili9341_to_production_spi2() {
    let manifest = serde_yaml::from_str(SYSTEM).expect("manifest parses");
    let mut bus = SystemBus::new();
    let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

    attach_esp32_external_devices(&mut bus, &manifest)
        .expect("ILI9341 must attach to the production ESP32-S3 SPI2 controller");

    let devices = bus.inspect_devices(None, &InspectOpts::default());
    assert!(
        devices.iter().any(|device| device.id == "tft"),
        "the attached ILI9341 must be visible through production inspection"
    );
}

#[test]
fn esp32s3_gpio11_resolves_as_a_display_dc_output() {
    let mut bus = SystemBus::new();
    let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

    let (address, bit) = SystemBus::resolve_pin_odr_pub(&bus, "GPIO11")
        .expect("an ESP32-S3 GPIO label must resolve for an SPI display D/C wire");

    let gpio = bus
        .find_peripheral_index_by_name("gpio")
        .expect("production ESP32-S3 bus exposes gpio");
    assert_eq!(address, bus.peripherals[gpio].base + 0x04);
    assert_eq!(bit, 11);
}

#[test]
fn esp32s3_production_spi_and_gpio_paint_the_attached_ili9341() {
    let manifest = serde_yaml::from_str(SYSTEM).expect("manifest parses");
    let mut bus = SystemBus::new();
    let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    attach_esp32_external_devices(&mut bus, &manifest).expect("attach production TFT");

    command(&mut bus, 0x29); // DISPON
    command(&mut bus, 0x2c); // RAMWR
    data(&mut bus, &[0xf8, 0x00]); // one RGB565 red pixel

    let devices = bus.inspect_devices(
        None,
        &InspectOpts {
            include_bytes: true,
            peripheral: None,
        },
    );
    let framebuffer = devices
        .iter()
        .find(|device| device.id == "tft")
        .and_then(|device| device.artifacts.iter().find(|artifact| artifact.kind == "framebuffer"))
        .expect("attached TFT exposes a framebuffer");

    assert_eq!(framebuffer.meta["display_on"], true);
    assert_eq!(framebuffer.meta["painted_bytes"], 1);
    let bytes = framebuffer.bytes.as_ref().expect("framebuffer bytes requested");
    assert_eq!(&bytes[..2], &[0xf8, 0x00]);
}
