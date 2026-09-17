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

/// The embedded `configs/devices/*.yaml` descriptors, keyed by `type:` string.
/// `include_str!` bundles them so wasm builds (no `std::fs`) resolve them too.
pub fn embedded_device_yaml(device_type: &str) -> Option<&'static str> {
    match device_type {
        "rotary_encoder" | "rotary-encoder" => {
            Some(include_str!("../../../configs/devices/rotary_encoder.yaml"))
        }
        "keypad" => Some(include_str!("../../../configs/devices/keypad.yaml")),
        "dht22" | "am2302" => Some(include_str!("../../../configs/devices/dht22.yaml")),
        "hc-sr04" | "hcsr04" => Some(include_str!("../../../configs/devices/hc_sr04.yaml")),
        "sht31" => Some(include_str!("../../../configs/devices/sht31.yaml")),
        "adxl345_spi" => Some(include_str!("../../../configs/devices/adxl345_spi.yaml")),
        "max31855" => Some(include_str!("../../../configs/devices/max31855.yaml")),
        "bh1750" => Some(include_str!("../../../configs/devices/bh1750.yaml")),
        "veml7700" => Some(include_str!("../../../configs/devices/veml7700.yaml")),
        "tmp102" => Some(include_str!("../../../configs/devices/tmp102.yaml")),
        "mcp9808" => Some(include_str!("../../../configs/devices/mcp9808.yaml")),
        "pca9685" => Some(include_str!("../../../configs/devices/pca9685.yaml")),
        "vcnl4010" => Some(include_str!("../../../configs/devices/vcnl4010.yaml")),
        "pcf8574" => Some(include_str!("../../../configs/devices/pcf8574.yaml")),
        "vl53l0x" => Some(include_str!("../../../configs/devices/vl53l0x.yaml")),
        "as5600" => Some(include_str!("../../../configs/devices/as5600.yaml")),
        "sht30" => Some(include_str!("../../../configs/devices/sht30.yaml")),
        "at24c256" => Some(include_str!("../../../configs/devices/at24c256.yaml")),
        "tmp117" => Some(include_str!("../../../configs/devices/tmp117.yaml")),
        "ds3231" => Some(include_str!("../../../configs/devices/ds3231.yaml")),
        "adxl345" => Some(include_str!("../../../configs/devices/adxl345.yaml")),
        "mpu6050" => Some(include_str!("../../../configs/devices/mpu6050.yaml")),
        "hx711" => Some(include_str!("../../../configs/devices/hx711.yaml")),
        "ina219" => Some(include_str!("../../../configs/devices/ina219.yaml")),
        "ads1115" => Some(include_str!("../../../configs/devices/ads1115.yaml")),
        "mma8451q" => Some(include_str!("../../../configs/devices/mma8451q.yaml")),
        "fxos8700" => Some(include_str!("../../../configs/devices/fxos8700.yaml")),
        "mlx90614" => Some(include_str!("../../../configs/devices/mlx90614.yaml")),
        "oled-ssd1306" => Some(include_str!("../../../configs/devices/ssd1306.yaml")),
        "oled-ssd1306-128x32" => Some(include_str!("../../../configs/devices/ssd1306_128x32.yaml")),
        "st7789-170x320" => Some(include_str!("../../../configs/devices/st7789.yaml")),
        "gp2y0a21" => Some(include_str!("../../../configs/devices/gp2y0a21.yaml")),
        "dc-motor" | "dc_motor" => Some(include_str!("../../../configs/devices/dc_motor.yaml")),
        "bldc-motor" | "bldc_motor" => {
            Some(include_str!("../../../configs/devices/bldc_motor.yaml"))
        }
        _ => None,
    }
}
