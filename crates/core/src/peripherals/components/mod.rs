// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod adxl345;
pub mod aht20;
pub mod bg770a;
pub mod bme280;
pub mod bmp280;
pub mod i2c_factory;
pub mod ili9341;
pub mod iolink_master;
pub mod max31855;
pub mod mpu6050;
pub mod neo6m;
pub mod ntc_thermistor;
pub mod pcd8544;
pub mod shm_i2c;
pub mod sn74hc165;
pub mod ssd1306;
pub mod ssd1680_tricolor_290;
pub mod uc8151d_tricolor_290;

pub use adxl345::Adxl345;
pub use aht20::Aht20;
pub use bg770a::QuectelBg770a;
pub use bme280::Bme280;
pub use bmp280::Bmp280;
pub use i2c_factory::build_i2c_device;
pub use ili9341::Ili9341;
pub use iolink_master::{
    IolinkComSpeed, IolinkFrameKind, IolinkLinkState, IolinkMaster, IolinkXfer,
};
pub use max31855::Max31855;
pub use mpu6050::Mpu6050;
pub use neo6m::Neo6mGps;
pub use ntc_thermistor::NtcThermistor;
pub use pcd8544::Pcd8544;
pub use shm_i2c::ShmI2c;
pub use sn74hc165::Sn74hc165;
pub use ssd1306::Ssd1306;
pub use ssd1680_tricolor_290::Ssd1680Tricolor290;
pub use uc8151d_tricolor_290::Uc8151dTricolor290;
