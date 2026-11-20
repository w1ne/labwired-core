// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `session` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test session <file>::`.
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

mod cpu_runtime_snapshot_honesty;
mod debug_schema_is_not_fidelity;
mod hardware_fidelity;
mod host_console_selection;
mod inspect_device_binding_universal;
mod inspect_external_devices;
mod observed_device_registry;
mod observer_store_visibility;
mod ordered_debug_metadata;
mod plugin_machine;
mod plugin_trait;
mod runtime_snapshot;
mod session_builder_arm;
mod session_builder_avr;
mod session_builder_pic16;
mod session_builder_pic18;
mod session_builder_riscv;
mod session_builder_xtensa;
mod session_can;
mod session_catalog;
mod session_cosim;
mod session_machine;
mod session_observe;
mod session_uart;
mod session_uart_selection;
mod sim_input;
mod sim_input_reachability;
mod simctl_abi_single_source;
mod trace_event_contract;
