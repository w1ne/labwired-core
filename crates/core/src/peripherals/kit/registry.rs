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
    &components::ina219::INA219_KIT,
    &components::ads1115::ADS1115_KIT,
    &components::ds3231::DS3231_KIT,
    &components::hx711::HX711_KIT,
    &components::as5600::AS5600_KIT,
    &components::bno055::BNO055_KIT,
    &components::hc05::HC05_KIT,
    &components::nrf24l01::NRF24L01_KIT,
    &components::microsd::MICROSD_KIT,
    &components::mcp2515::MCP2515_KIT,
    &components::mpu6050::MPU6050_KIT,
    &components::mma8451q::MMA8451Q_KIT,
    &components::bme280::BME280_KIT,
    &components::aht20::AHT20_KIT,
    &components::bmp280::BMP280_KIT,
    &components::pcf8574::PCF8574_KIT,
    &components::rc522::RC522_KIT,
    &components::sht30::SHT30_KIT,
    &components::at24c256::AT24C256_KIT,
    &components::atecc608a::ATECC608A_KIT,
    &components::pn532::PN532_KIT,
    &components::lora_sx1278::LORA_SX1278_KIT,
    &components::sim800l::SIM800L_KIT,
    &components::ssd1306::SSD1306_KIT,
    &components::ssd1306::SSD1306_128X32_KIT,
    &components::sh1107::SH1107_KIT,
    &components::ili9341::ILI9341_KIT,
    &components::rm67162::RM67162_KIT,
    &components::st7789::ST7789_KIT,
    &components::ili9341_parallel::ILI9341_PARALLEL_KIT,
    &components::ssd1680_tricolor_290::SSD1680_TRICOLOR_290_KIT,
    &components::uc8151d_tricolor_290::UC8151D_TRICOLOR_290_KIT,
    &components::sn74hc165::SN74HC165_KIT,
    &components::hc595_7seg::HC595_7SEG_KIT,
    &components::tm1637_7seg::TM1637_7SEG_KIT,
    &components::seven_segment::SEVEN_SEGMENT_KIT,
    &components::pcd8544::PCD8544_KIT,
    &components::iolink_master::IOLINK_MASTER_KIT,
    &components::ntc_thermistor::NTC_THERMISTOR_KIT,
    &components::potentiometer::POTENTIOMETER_KIT,
    &components::lipo_charger::LIPO_CHARGER_KIT,
    &components::ldr::LDR_KIT,
    &components::mq6::MQ6_KIT,
    &components::declarative_analog::GP2Y0A21_KIT,
    &components::soil_moisture::SOIL_MOISTURE_KIT,
    &components::hc595::HC595_KIT,
    &components::vl53l1x::VL53L1X_KIT,
    // Leo air-quality board sensors (ESP32-C3 I²C).
    &components::scd41::SCD41_KIT,
    &components::sgp41::SGP41_KIT,
    &components::sps30::SPS30_KIT,
    &components::mlx90614::MLX90614_KIT,
    &components::max7219::MAX7219_KIT,
    &components::lcd1602::LCD1602_KIT,
    // Declarative I²C devices — model lives entirely in configs/devices/*.yaml,
    // interpreted by the generic GenericI2cDevice (zero per-part Rust). VEML7700
    // was migrated here from a hand-written model; the model survives only as the
    // byte-parity oracle (components::veml7700, #[cfg(test)]).
    &components::declarative_i2c::SHT31_KIT,
    &components::declarative_i2c::MCP9808_KIT,
    &components::declarative_i2c::BH1750_KIT,
    &components::declarative_i2c::VEML7700_KIT,
    // TMP102 (register-pointer + drift) and PCA9685 (byte register file + servo
    // observable) were migrated from hand-written models; those survive only as
    // the byte-parity oracles (pca9685_tmp102_parity.rs).
    &components::declarative_i2c::TMP102_KIT,
    &components::declarative_i2c::PCA9685_KIT,
    // VCNL4010: declarative from the start, no hand-written predecessor.
    &components::declarative_i2c::VCNL4010_KIT,
    // VL53L0X: migrated from a hand-written model that is DELETED, not kept as
    // an oracle — its ready flag latched forever with no conversion time, so an
    // oracle would be asserting the bug. See vl53l0x_migration_parity.rs.
    &components::declarative_i2c::VL53L0X_KIT,
    // Declarative SPI devices — model lives entirely in configs/devices/*.yaml,
    // interpreted by the generic GenericSpiDevice (zero per-part Rust).
    &components::declarative_spi::ADXL345_KIT,
    &components::declarative_spi::MAX31855_KIT,
    &components::apa102::APA102_KIT,
    // Migrated from i2c_factory-only → universal kit attach (any MCU).
    &components::tmp117::TMP117_KIT,
    &components::bmi270::BMI270_KIT,
    &components::fxos8700::FXOS8700_KIT,
    &components::max30102::MAX30102_KIT,
    &components::cap1188::CAP1188_KIT,
    &components::drv2605::DRV2605_KIT,
    &components::mlx90640::MLX90640_KIT,
    // GPIO-group actuators migrated off from_config residual arms.
    &components::servo::SERVO_KIT,
    &components::ws2812::WS2812_KIT,
    &components::step_dir_motor::STEP_DIR_MOTOR_KIT,
    &components::h_bridge_motor::H_BRIDGE_MOTOR_KIT,
    &components::unipolar_stepper::UNIPOLAR_STEPPER_KIT,
    // Host-side CAN tools (were residual from_config arms).
    &components::can_testers::CAN_DIAGNOSTIC_TESTER_KIT,
    &components::can_testers::CAN_UDS_TESTER_KIT,
    &components::can_testers::CAN_LOG_PLAYER_KIT,
];

/// Borrow the registry slice.
pub fn kits() -> &'static [&'static dyn PeripheralKit] {
    KITS
}

/// Legacy `type:` spellings that predate the canonical `device_type` and are
/// still accepted. These used to be understood only by the ESP32-classic attach
/// arm, which meant the same manifest resolved on one chip and was skipped as
/// "unsupported" on every other — resolving them here instead makes them mean
/// the same thing on every MCU. Nothing in the repo emits them; the list exists
/// so a hand-written manifest that already worked keeps working, and it should
/// not grow.
const TYPE_ALIASES: &[(&str, &str)] = &[
    ("epd-2in9-tricolor", "ssd1680_tricolor_290"),
    ("gxepd2_290_c90c", "ssd1680_tricolor_290"),
    ("epd-2in9-uc8151d", "uc8151d_tricolor_290"),
    // DRV2605L is the same die / register map for simulation purposes.
    ("drv2605l", "drv2605"),
    // Underscore spelling of the parallel ILI9341 kit.
    ("ili9341_16bit", "ili9341-16bit"),
    // Hobby servo calibrations as top-level type strings.
    ("sg90", "servo"),
    ("mg996r", "servo"),
    // WS2812 synonym for neopixel kit.
    ("ws2812", "neopixel"),
    // STEP/DIR driver family.
    ("drv8825", "a4988"),
    ("tmc2209", "a4988"),
    // H-bridge family.
    ("tb6612", "l298n"),
    ("l293d", "l298n"),
    // A fader is a pot. Same three-terminal carbon track and the same wiper
    // voltage the ADC reads; only the mechanism the human touches differs, and
    // the catalog keeps them apart for the BODY (an 88mm fader is not a 9.53mm
    // trimmer), not for the electrical model.
    ("slide-potentiometer", "potentiometer"),
    // Unipolar stepper.
    ("stepper-28byj48", "uln2003"),
    // CAN diagnostic one-shot injector alias.
    ("uds-diagnostic-tester", "can-diagnostic-tester"),
];

/// Resolve a `device_type` spelling to its canonical form.
///
/// This is the ONE home for alias resolution. Anything that decides "is this
/// binding the device I want?" must go through here rather than matching the
/// legacy spelling itself — a second copy of the table is invisible until
/// somebody authors a manifest with the one row that copy left out, and then
/// the device is simply not found. Unknown spellings pass through unchanged so
/// the caller's own "not found" path still reports the string the author wrote.
pub fn canonical_device_type(device_type: &str) -> &str {
    TYPE_ALIASES
        .iter()
        .find(|(alias, _)| *alias == device_type)
        .map_or(device_type, |(_, canonical)| *canonical)
}

/// Every `device_type` spelling the engine accepts — canonical kit types plus
/// the legacy aliases. Ordered and de-duplicated.
pub fn known_device_types() -> Vec<String> {
    let mut out: Vec<String> = KITS
        .iter()
        .map(|k| k.metadata().device_type.to_string())
        .chain(TYPE_ALIASES.iter().map(|(alias, _)| alias.to_string()))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Lookup a kit by the `device_type` string used in `system.yaml`.
pub fn lookup(device_type: &str) -> Option<&'static dyn PeripheralKit> {
    let canonical = canonical_device_type(device_type);
    KITS.iter()
        .copied()
        .find(|k| k.metadata().device_type == canonical)
}
