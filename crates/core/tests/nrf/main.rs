// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `nrf` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test nrf <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

#[path = "../common/microbit_sensor_guest.rs"]
mod guest;

mod arduino_nano_33_ble_bme280;
mod microbit_v2_led_matrix;
mod microbit_v2_mag_drdy_guest;
mod microbit_v2_open_drain_guest;
mod microbit_v2_sensor_drdy_guest;
mod microbit_v2_uarte_stack_guest;
mod nrf52833_config;
mod nrf52833_gpio_geometry;
mod nrf52_bus_waveform;
mod nrf52_pwm_pad;
mod nrf52_spim_easydma_executing;
mod nrf5340_clock_boot;
mod nrf5340_ficr;
mod nrf5340_gpiote;
mod nrf5340_uicr;
mod nrf54l15_boot;
mod nrf54l15_ficr;
mod nrf54l15_rramc;
mod nrf54l15_smart_ring_probe;
mod nrf54l_gpio_offsets;
mod nrf54l_wdt;
mod nrf54lm20a_peripheral_estate;
mod nrf_gpio_in_step_vs_run;
mod nrf_grtc_clock;
mod nrf_rtc_lfclk_clock;
mod nrf_timer_base_clock;
mod nrf_wdt_lfclk_clock;
