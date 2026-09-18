// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#![allow(dead_code)]
use crate::*;

impl DeviceDescriptor {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let mut desc: Self =
            serde_yaml::from_str(yaml).context("Failed to parse Device Descriptor")?;
        // ONE expansion point for `metadata.inputs[].bits:` channel groups.
        // Every consumer of `metadata.inputs` — the kit metadata, the offline
        // peripherals manifest, `SimInput`, a register field's `source:` — goes
        // through a parsed descriptor, so expanding here is what makes the
        // group invisible everywhere else instead of a case each of the six
        // primitives has to remember.
        if let Some(meta) = desc.metadata.as_mut() {
            if meta.inputs.iter().any(|i| i.bits.is_some()) {
                meta.inputs = meta.inputs.iter().flat_map(|i| i.expand()).collect();
            }
        }
        Ok(desc)
    }

    /// Look up and parse the embedded descriptor for a device `type:` string
    /// (accepts either spelling for the encoder). Returns `Ok(None)` for a type
    /// with no declarative descriptor. This is the SINGLE embed point — the
    /// runtime attach path (`core`'s `bus/declarative_device.rs`) resolves
    /// descriptors through here, so there is one source of truth for the
    /// `configs/devices/*.yaml` set.
    pub fn embedded(device_type: &str) -> Result<Option<Self>> {
        match embedded_device_yaml(device_type) {
            Some(yaml) => Ok(Some(Self::from_yaml(yaml).with_context(|| {
                format!("Failed to parse embedded device descriptor for '{device_type}'")
            })?)),
            None => Ok(None),
        }
    }
}

/// Every embedded `configs/devices/*.yaml` descriptor: the `type:` spellings it
/// answers to, and its YAML text. ONE table, so the descriptor set is ENUMERABLE
/// as well as look-up-able. That is what lets the kit registry derive a
/// `PeripheralKit` from every descriptor instead of from a hand-kept list of
/// wrappers — a part that is a row here can no longer fall out of
/// `peripherals-manifest.json` by nobody remembering to write its wrapper.
///
/// A `static`, not a `const`: a const of 37 `include_str!` blobs (220 KB of
/// YAML) is inlined at every use site, while a static has one address and is
/// materialised once.
///
/// The FIRST spelling in a row is the canonical `type:` the YAML itself declares;
/// the rest are legacy aliases resolving to the same file.
pub static EMBEDDED_DEVICES: &[(&[&str], &str)] = &[
    (
        &["rotary_encoder", "rotary-encoder"],
        include_str!("../../../configs/devices/rotary_encoder.yaml"),
    ),
    (
        &["keypad"],
        include_str!("../../../configs/devices/keypad.yaml"),
    ),
    // The two segment displays. The FIRST spelling is the `type:` the YAML
    // declares, and it is the hyphenated one deliberately: it is the
    // `device_type` the hand-written kits published, so every shipped manifest,
    // lab and browser entry that says `tm1637-7seg` keeps resolving. The
    // underscored alias matches the file name and the way the rest of the
    // gpio descriptors spell themselves.
    (
        &["tm1637-7seg", "tm1637_7seg"],
        include_str!("../../../configs/devices/tm1637_7seg.yaml"),
    ),
    (
        &["seven-segment", "seven_segment"],
        include_str!("../../../configs/devices/seven_segment.yaml"),
    ),
    (
        &["dht22", "am2302"],
        include_str!("../../../configs/devices/dht22.yaml"),
    ),
    (
        &["hc-sr04", "hcsr04"],
        include_str!("../../../configs/devices/hc_sr04.yaml"),
    ),
    (
        &["sht31"],
        include_str!("../../../configs/devices/sht31.yaml"),
    ),
    (
        &["adxl345_spi"],
        include_str!("../../../configs/devices/adxl345_spi.yaml"),
    ),
    (
        &["max31855"],
        include_str!("../../../configs/devices/max31855.yaml"),
    ),
    (
        &["sn74hc165"],
        include_str!("../../../configs/devices/sn74hc165.yaml"),
    ),
    (
        &["aht20"],
        include_str!("../../../configs/devices/aht20.yaml"),
    ),
    (
        &["bh1750"],
        include_str!("../../../configs/devices/bh1750.yaml"),
    ),
    (
        &["veml7700"],
        include_str!("../../../configs/devices/veml7700.yaml"),
    ),
    (
        &["tmp102"],
        include_str!("../../../configs/devices/tmp102.yaml"),
    ),
    (
        &["mcp9808"],
        include_str!("../../../configs/devices/mcp9808.yaml"),
    ),
    (
        &["pca9685"],
        include_str!("../../../configs/devices/pca9685.yaml"),
    ),
    (
        &["vcnl4010"],
        include_str!("../../../configs/devices/vcnl4010.yaml"),
    ),
    (
        &["pcf8574"],
        include_str!("../../../configs/devices/pcf8574.yaml"),
    ),
    (
        &["vl53l0x"],
        include_str!("../../../configs/devices/vl53l0x.yaml"),
    ),
    (
        &["vl53l1x"],
        include_str!("../../../configs/devices/vl53l1x.yaml"),
    ),
    (
        &["bno055"],
        include_str!("../../../configs/devices/bno055.yaml"),
    ),
    (
        &["bmp280"],
        include_str!("../../../configs/devices/bmp280.yaml"),
    ),
    (
        &["as5600"],
        include_str!("../../../configs/devices/as5600.yaml"),
    ),
    (
        &["sht30"],
        include_str!("../../../configs/devices/sht30.yaml"),
    ),
    (
        &["at24c256"],
        include_str!("../../../configs/devices/at24c256.yaml"),
    ),
    (
        &["tmp117"],
        include_str!("../../../configs/devices/tmp117.yaml"),
    ),
    (
        &["ds3231"],
        include_str!("../../../configs/devices/ds3231.yaml"),
    ),
    (
        &["adxl345"],
        include_str!("../../../configs/devices/adxl345.yaml"),
    ),
    (
        &["mpu6050"],
        include_str!("../../../configs/devices/mpu6050.yaml"),
    ),
    (
        &["hx711"],
        include_str!("../../../configs/devices/hx711.yaml"),
    ),
    (
        &["ina219"],
        include_str!("../../../configs/devices/ina219.yaml"),
    ),
    (
        &["ads1115"],
        include_str!("../../../configs/devices/ads1115.yaml"),
    ),
    (
        &["mma8451q"],
        include_str!("../../../configs/devices/mma8451q.yaml"),
    ),
    (
        &["fxos8700"],
        include_str!("../../../configs/devices/fxos8700.yaml"),
    ),
    (
        &["mlx90614"],
        include_str!("../../../configs/devices/mlx90614.yaml"),
    ),
    (
        &["oled-ssd1306"],
        include_str!("../../../configs/devices/ssd1306.yaml"),
    ),
    (
        &["oled-ssd1306-128x32"],
        include_str!("../../../configs/devices/ssd1306_128x32.yaml"),
    ),
    (
        &["st7789-170x320"],
        include_str!("../../../configs/devices/st7789.yaml"),
    ),
    (
        &["oled-sh1107"],
        include_str!("../../../configs/devices/sh1107.yaml"),
    ),
    (
        &["pcd8544"],
        include_str!("../../../configs/devices/pcd8544.yaml"),
    ),
    (
        &["ili9341"],
        include_str!("../../../configs/devices/ili9341.yaml"),
    ),
    (
        &["amoled-rm67162"],
        include_str!("../../../configs/devices/rm67162.yaml"),
    ),
    (
        &["ssd1680_tricolor_290"],
        include_str!("../../../configs/devices/ssd1680_tricolor_290.yaml"),
    ),
    (
        &["uc8151d_tricolor_290"],
        include_str!("../../../configs/devices/uc8151d_tricolor_290.yaml"),
    ),
    (
        &["apa102"],
        include_str!("../../../configs/devices/apa102.yaml"),
    ),
    (
        &["neopixel", "ws2812"],
        include_str!("../../../configs/devices/ws2812.yaml"),
    ),
    (
        &["gp2y0a21"],
        include_str!("../../../configs/devices/gp2y0a21.yaml"),
    ),
    // The analog plants: each replaces a hand-written Rust model of the same
    // `type:`, deleted in the same change. The spellings are the ones the old
    // kits advertised, so every manifest that already worked still resolves.
    (&["ldr"], include_str!("../../../configs/devices/ldr.yaml")),
    (
        &["potentiometer", "slide-potentiometer"],
        include_str!("../../../configs/devices/potentiometer.yaml"),
    ),
    (
        &["ntc-thermistor"],
        include_str!("../../../configs/devices/ntc_thermistor.yaml"),
    ),
    (&["mq-6"], include_str!("../../../configs/devices/mq6.yaml")),
    (
        &["soil-moisture"],
        include_str!("../../../configs/devices/soil_moisture.yaml"),
    ),
    (
        &["lipo_charger"],
        include_str!("../../../configs/devices/lipo_charger.yaml"),
    ),
    (
        &["dc-motor", "dc_motor"],
        include_str!("../../../configs/devices/dc_motor.yaml"),
    ),
    (
        &["bldc-motor", "bldc_motor"],
        include_str!("../../../configs/devices/bldc_motor.yaml"),
    ),
    (
        &["hc-05", "hc05"],
        include_str!("../../../configs/devices/hc-05.yaml"),
    ),
    (
        &["sim800l"],
        include_str!("../../../configs/devices/sim800l.yaml"),
    ),
    (
        &["neo6m-gps"],
        include_str!("../../../configs/devices/neo6m-gps.yaml"),
    ),
    (
        &["lora-sx1278"],
        include_str!("../../../configs/devices/lora_sx1278.yaml"),
    ),
    (
        &["rc522"],
        include_str!("../../../configs/devices/rc522.yaml"),
    ),
    (
        &["nrf24l01"],
        include_str!("../../../configs/devices/nrf24l01.yaml"),
    ),
    (
        &["scd41"],
        include_str!("../../../configs/devices/scd41.yaml"),
    ),
    (
        &["sgp41"],
        include_str!("../../../configs/devices/sgp41.yaml"),
    ),
    (
        &["bmi270"],
        include_str!("../../../configs/devices/bmi270.yaml"),
    ),
    (
        &["cap1188"],
        include_str!("../../../configs/devices/cap1188.yaml"),
    ),
    // ── shift-register / serial display drivers (`spi_device` + `frames:`) ─
    //
    // Three parts whose unit of work is a MESSAGE rather than a register, all
    // three migrated from hand-written Rust. The `device_type` strings are the
    // ones the manifests and the browser already use and are UNCHANGED.
    (
        &["led-matrix", "max7219"],
        include_str!("../../../configs/devices/max7219.yaml"),
    ),
    (
        &["74hc595", "hc595"],
        include_str!("../../../configs/devices/hc595.yaml"),
    ),
    (
        &["hc595-7seg", "hc595_7seg"],
        include_str!("../../../configs/devices/hc595_7seg.yaml"),
    ),
    // ── 74-series logic (`logic_gate`) ─────────────────────────────────────
    //
    // The third largest model gap in the 39-project KiCad corpus: 139 dropped
    // symbols. The extra spellings are the ones the corpus actually uses — an
    // LS, an HCT and an LVT part differ in levels and speed, not in logic, and
    // a placement that names one must not be dropped for want of a row.
    (
        &["74hc04", "74ls04", "74hct04"],
        include_str!("../../../configs/devices/74hc04.yaml"),
    ),
    (
        &["74hc00", "74ls00", "74hct00"],
        include_str!("../../../configs/devices/74hc00.yaml"),
    ),
    (
        &["74hc08", "74ls08", "74hct08"],
        include_str!("../../../configs/devices/74hc08.yaml"),
    ),
    (
        &["74hc32", "74ls32", "74hct32"],
        include_str!("../../../configs/devices/74hc32.yaml"),
    ),
    (
        &["74hc125", "74ls125", "74lvth125", "sn74lvth125"],
        include_str!("../../../configs/devices/74hc125.yaml"),
    ),
    (
        &["74lvc1t45", "sn74lvc1t45"],
        include_str!("../../../configs/devices/74lvc1t45.yaml"),
    ),
    (
        &["74hc245", "74ls245", "74lvc245", "sn74lvc245a"],
        include_str!("../../../configs/devices/74hc245.yaml"),
    ),
    (
        &["74cbtlv3257", "sn74cbtlv3257"],
        include_str!("../../../configs/devices/74cbtlv3257.yaml"),
    ),
];

/// The embedded `configs/devices/*.yaml` descriptors, keyed by `type:` string.
/// `include_str!` bundles them so wasm builds (no `std::fs`) resolve them too.
pub fn embedded_device_yaml(device_type: &str) -> Option<&'static str> {
    EMBEDDED_DEVICES
        .iter()
        .find(|(spellings, _)| spellings.contains(&device_type))
        .map(|(_, yaml)| *yaml)
}

/// Every embedded descriptor's YAML text, in table order.
///
/// The kit registry walks this to build a kit per descriptor. Enumerating the
/// descriptors — rather than each one opting in by having somebody write a kit
/// wrapper for it — is the whole point: `keypad`, `dht22` and `rotary_encoder`
/// were absent from the peripherals manifest for exactly as long as opting in
/// was a manual step.
pub fn embedded_device_yamls() -> impl Iterator<Item = &'static str> {
    EMBEDDED_DEVICES.iter().map(|(_, yaml)| *yaml)
}
