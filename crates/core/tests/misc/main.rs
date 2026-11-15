// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `misc` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test misc <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

mod adxl345_register_map;
mod bg770a_validation;
mod bmi270_spi_protocol;
mod dfplayer_mini;
mod everyday_sensor_set;
mod mlx90640_clock;
mod rs485_modbus;
mod tmp102_config_writable;
mod vcnl4010_bay_occupancy;
mod vl53l0x_vendor_init;
