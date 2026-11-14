// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `gates` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test gates <file>::`.
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

mod arch_chip_isolation;
mod board_config_gates;
mod board_coverage_ratchet;
mod board_registry_consistency;
mod chip_conformance;
mod event_scheduler;
mod firmware_survival;
mod jit_lockstep;
mod model_paths_drift_gate;
mod release_only_cfg_ratchet;
mod svd_conformance;
