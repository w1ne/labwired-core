// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `cosim` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test cosim <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

#[path = "../common/mod.rs"]
mod common;

mod cosim_canonical_basket;
mod cosim_contract;
mod cosim_external_process_bounds;
mod cosim_pin_routing;
mod cosim_runner_hot_path;
mod cosim_session_advance;
mod cosim_shm_transport;
