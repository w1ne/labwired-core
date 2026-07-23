// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The single source of truth for migrated peripherals.
//!
//! To add or migrate a peripheral: implement [`super::PeripheralKit`] for
//! it (typically as a unit struct living next to the model), expose a
//! `pub static` instance, and append it to the [`kits`] slice below. The
//! peripheral_kit_gate test verifies each entry is well-formed and unique;
//! the manifest generator (`labwired-peripherals-manifest`) iterates this
//! slice to produce `peripherals-manifest.json` for the playground.

use super::PeripheralKit;
use crate::peripherals::components;

/// All peripherals that have migrated to the [`PeripheralKit`] contract.
/// Peripherals not listed here still use the legacy hand-written arms in
/// `bus/mod.rs` — both paths coexist during migration.
pub static KITS: &[&'static dyn PeripheralKit] = &[
    &components::bg770a::BG770A_KIT,
    &components::neo6m::NEO6M_KIT,
    &components::adxl345::ADXL345_KIT,
    &components::mpu6050::MPU6050_KIT,
    &components::bme280::BME280_KIT,
    &components::ssd1306::SSD1306_KIT,
    &components::ssd1306::SSD1306_128X32_KIT,
    &components::sh1107::SH1107_KIT,
    &components::ili9341::ILI9341_KIT,
    &components::max31855::MAX31855_KIT,
    &components::ssd1680_tricolor_290::SSD1680_TRICOLOR_290_KIT,
    &components::sn74hc165::SN74HC165_KIT,
    &components::hc595_7seg::HC595_7SEG_KIT,
    &components::tm1637_7seg::TM1637_7SEG_KIT,
    &components::seven_segment::SEVEN_SEGMENT_KIT,
    &components::pcd8544::PCD8544_KIT,
    &components::iolink_master::IOLINK_MASTER_KIT,
    &components::ntc_thermistor::NTC_THERMISTOR_KIT,
    &components::potentiometer::POTENTIOMETER_KIT,
    &components::ldr::LDR_KIT,
    &components::hc595::HC595_KIT,
    &components::vl53l1x::VL53L1X_KIT,
    // Leo air-quality board sensors (ESP32-C3 I²C).
    &components::scd41::SCD41_KIT,
    &components::sgp41::SGP41_KIT,
    &components::sps30::SPS30_KIT,
    &components::veml7700::VEML7700_KIT,
    &components::mlx90614::MLX90614_KIT,
    &components::max7219::MAX7219_KIT,
    &components::lcd1602::LCD1602_KIT,
    // Declarative I²C devices — model lives entirely in configs/devices/*.yaml,
    // interpreted by the generic GenericI2cDevice (zero per-part Rust).
    &components::declarative_i2c::SHT31_KIT,
    &components::declarative_i2c::BH1750_KIT,
];

/// Borrow the registry slice.
pub fn kits() -> &'static [&'static dyn PeripheralKit] {
    KITS
}

/// Lookup a kit by the `device_type` string used in `system.yaml`.
pub fn lookup(device_type: &str) -> Option<&'static dyn PeripheralKit> {
    KITS.iter()
        .copied()
        .find(|k| k.metadata().device_type == device_type)
}
