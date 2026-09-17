// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **The deleted Rust display models, kept verbatim as migration oracles.**
//!
//! `components/ssd1306.rs`, `components/st7789.rs` and `components/sh1107.rs`
//! were the SSD1306, ST7789V and SH1107 models until the `display` primitive
//! replaced them with `configs/devices/{ssd1306,ssd1306_128x32,st7789,
//! sh1107}.yaml`. Their model halves —
//! everything above the `PeripheralKit` registration block — are copied here
//! BYTE FOR BYTE off the commit that deleted them, with three mechanical edits
//! and no others:
//!
//!   * `use crate::…` → `use labwired_core::…`, because an integration test is
//!     a separate crate;
//!   * `crate::inspect::` → `labwired_core::inspect::`, same reason;
//!   * one intra-doc link that no longer resolves, demoted to code font.
//!
//! WHY A COPY AND NOT A GOLDEN TABLE. A golden byte array records what the old
//! model did on the ONE script somebody thought to write down. Keeping the model
//! itself lets `display_migration_parity.rs` drive both implementations through
//! the same script and compare everything they produce — the wire transcript,
//! the whole framebuffer, and every field of the paint artifact — and lets a
//! later reviewer add a script without having to resurrect the old model to
//! bless its golden. It is also the pattern this repo already uses for the
//! VEML7700 port (`components/veml7700.rs`, `#[cfg(test)]`).
//!
//! NOTHING OUTSIDE THIS TEST MAY USE THESE TYPES. They are not registered, not
//! reachable from any manifest, and not compiled into the engine. When the
//! parity test is eventually retired, this directory goes with it.

#![allow(dead_code)]

pub mod ili9341;
pub mod pcd8544;
pub mod rm67162;
pub mod sh1107;
pub mod ssd1306;
pub mod st7789;
