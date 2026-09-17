// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#![allow(dead_code)]
use crate::*;

impl DeviceDescriptor {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        serde_yaml::from_str(yaml).context("Failed to parse Device Descriptor")
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
/// A `static`, not a `const`: a const of 34 `include_str!` blobs (220 KB of
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
        &["gp2y0a21"],
        include_str!("../../../configs/devices/gp2y0a21.yaml"),
    ),
    (
        &["dc-motor", "dc_motor"],
        include_str!("../../../configs/devices/dc_motor.yaml"),
    ),
    (
        &["bldc-motor", "bldc_motor"],
        include_str!("../../../configs/devices/bldc_motor.yaml"),
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
