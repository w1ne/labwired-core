// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod aht20;
pub mod bmp280;
pub mod i2c_factory;
pub mod mpu6050;
pub mod shm_i2c;

pub use aht20::Aht20;
pub use bmp280::Bmp280;
pub use i2c_factory::build_i2c_device;
pub use mpu6050::Mpu6050;
pub use shm_i2c::ShmI2c;
