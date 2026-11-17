// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `bus` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test bus <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

#[path = "../common/src_roots.rs"]
mod src_roots;

mod bus_resident_device_port;
mod bus_visibility;
mod cpu_dyn_forwarding;
mod cpu_hz_single_source;
mod cpu_trace_conformance;
mod fault_lockstep;
mod i2c_clock_stretch_wrappers;
mod i2c_component;
mod i2c_factory_kit_coverage;
mod i2c_mux_tca9548a;
mod part_pack_contract;
mod parts_batch5;
mod peripheral_kit_gate;
mod rcc_kernel_clock_gate;
mod register_compliance;
mod spi_gpio_decoder;
mod tier2_device_pins;
mod uart_attach_seam;
mod unknown_peripheral_type;
mod walk_deletion;
mod walk_differential_selftest;
mod yaml_mux_resolution;
