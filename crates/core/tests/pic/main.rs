// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! One integration-test binary for the `pic` group. Every other file in this
//! directory is a module: a test that was `<name>` in its own `<file>` binary
//! is now `<file>::<name>` here. Run one former file with
//! `cargo test -p labwired-core --test pic <file>::`.
//!
//! Tests that mutate process-global state (environment variables, the working
//! directory, the global allocator), tests behind a cargo feature and tests the
//! CI shard classification names individually keep their own top-level
//! binaries; the reasons are listed next to the `[[test]]` entries in
//! `crates/core/Cargo.toml`.

mod curiosity_pic32mz_nuttx;
mod pic16f1829_smoke;
mod pic16f877a_smoke;
mod pic18f4520_smoke;
mod pic24_ka_smoke;
mod pic24_m0_smoke;
mod pic24_xc16_202_m1;
mod pic24_xc16_202_m2;
mod pic24_xc16_202_m3;
mod pic24_xc16_202_smoke;
mod pic24_xc16_dsp;
mod pic24_xc16_elf_boot;
mod pic24_xc16_m2;
mod pic24_xc16_m3;
