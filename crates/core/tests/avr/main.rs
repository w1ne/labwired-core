// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `avr` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test avr <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

mod atmega1281_walk_differential;
mod avr_atmega1281;
mod avr_atmega1281_firmware;
mod avr_atmega2561;
mod avr_atmega328pb_usart1;
mod avr_capture_temp;
mod avr_cpu_held_i2c_inspect;
mod avr_logic_capture_differential;
mod avr_machine_clock;
mod avr_max7219_cascade;
mod avr_mega2560_io;
mod avr_nano_golden_survival;
mod avr_nano_live_run;
mod avr_nano_machine_run;
mod avr_nano_wdt;
mod avr_pull_up_disable;
mod avr_pwm;
mod avr_pwm_pin_read;
mod avr_servo;
mod avr_spi_dc_inspect;
mod avr_supply_supervisor;
mod avr_tone;
mod avr_uno_io;
mod avr_uno_machine_run;
mod cosim_avr_touch_rc;
