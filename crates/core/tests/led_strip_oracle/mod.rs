// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **The deleted Rust LED-strip models, kept verbatim as migration oracles.**
//!
//! `components/apa102.rs` and `components/ws2812.rs` were the APA102 and the
//! WS2812 until the `led_strip` primitive replaced them with
//! `configs/devices/{apa102,ws2812}.yaml`. Their model halves — everything
//! above the `PeripheralKit` registration block — are copied here BYTE FOR BYTE
//! off the commit that deleted them, with the same mechanical edits the
//! `display_oracle/` module lists and no others:
//!
//!   * `use crate::…` → `use labwired_core::…`, because an integration test is
//!     a separate crate;
//!   * `crate::inspect::` → `labwired_core::inspect::`, same reason;
//!   * intra-doc links that no longer resolve, demoted to code font.
//!
//! WHY A COPY AND NOT A GOLDEN TABLE: see `display_oracle/mod.rs`. A golden
//! byte array records what the old model did on the ONE script somebody thought
//! to write down; keeping the model lets `led_strip_migration_parity.rs` drive
//! both implementations through the same script and compare everything they
//! produce.
//!
//! NOTHING OUTSIDE THIS TEST MAY USE THESE TYPES. They are not registered, not
//! reachable from any manifest, and not compiled into the engine.

#![allow(dead_code)]

pub mod apa102;
pub mod ws2812;
