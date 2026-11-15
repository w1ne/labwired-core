// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `device_parity` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test device_parity <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

mod ads1115_migration_parity;
mod adxl345_migration_parity;
mod aht20_migration_parity;
mod as5600_migration_parity;
mod at24c256_migration_parity;
mod bme280_migration_parity;
mod bmi270_migration_parity;
mod bmp280_migration_parity;
mod bno055_migration_parity;
mod cap1188_migration_parity;
mod declarative_device_byte_parity;
mod display_migration_parity;
mod ds3231_migration_parity;
mod fxos8700_migration_parity;
mod gpio_artifact_migration_parity;
mod hx711_migration_parity;
mod ina219_migration_parity;
mod keypad_migration_parity;
mod led_strip_migration_parity;
mod mlx90614_migration_parity;
mod mma8451q_migration_parity;
mod mpu6050_migration_parity;
mod pca9685_tmp102_parity;
mod pcf8574_migration_parity;
mod rotary_migration_parity;
mod sensirion_migration_parity;
mod sht30_migration_parity;
mod sn74hc165_migration_parity;
mod spi_frame_migration_parity;
mod spi_shell_migration_parity;
mod tmp117_migration_parity;
mod uart_migration_parity;
mod uart_parity_ratchet;
mod vl53l0x_migration_parity;
mod vl53l1x_migration_parity;
mod ydlidar_silicon_parity;
