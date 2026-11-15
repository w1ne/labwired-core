// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `gpio_logic` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test gpio_logic <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

#[allow(dead_code)]
#[path = "../common/keypad.rs"]
mod keypad_fixture;
#[allow(dead_code)]
#[path = "../common/rotary.rs"]
mod rotary_fixture;

mod board_io_button_stimulus;
mod e2e_iolink_dido;
mod gpio_devices_drive_read_only_inputs;
mod gpio_devices_walk_free;
mod gpio_list_roles;
mod gpio_schedule_firmware;
mod iolink_master_config;
mod keypad_scan_directions;
mod led_strip_system_configs;
mod logic_analyzer_f1_rp2040_pad_visibility;
mod logic_analyzer_lab_pad_visibility;
mod logic_capture_bench;
mod logic_capture_late_drive;
mod logic_gate_74series;
mod logic_wire_channels_cross_family;
mod no_bus_device_board_io_dual;
mod no_display_board_io_dual;
mod pad_control_pulls;
mod pin_map_resolution;
mod pin_trace_four_state;
mod reset_input;
mod rotary_encoder_semantics;
