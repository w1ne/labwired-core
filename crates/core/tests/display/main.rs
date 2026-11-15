// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `display` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test display <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/src_roots.rs"]
mod src_roots;

mod apa102_strip;
mod desk_controller_bridge;
mod desk_controller_inputs;
mod display_evidence_one_seam;
mod display_one_door;
mod display_resolution_ignores_board_io_type;
mod e2e_ili9341_16bit_gpio_smoke;
mod e2e_spi3_dc_epaper;
mod epaper_ssd1680_differential;
mod epaper_twin_single_source;
mod esp32_ili9341_attach;
mod esp32s3_camera_dvp;
mod esp32s3_ili9341_attach;
mod esp32s3_lcd_i80_pixels;
mod esp32s3_oled_profile;
mod esp32s3_ssd1306_differential;
mod everyday_display_set;
mod leo_oled_i2c_readback;
mod nokia5110_pcd8544_differential;
mod panel_artifact_evidence;
