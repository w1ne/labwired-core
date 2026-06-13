// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Chip-neutral ESP-Xtensa boot infrastructure shared across ESP32 families.
//!
//! These modules model generic ESP-Xtensa bring-up plumbing (ROM thunk bank,
//! APPCPU release flags) and generic peripheral-as-RAM stubs. They are NOT
//! specific to any one SoC's silicon, so they live here rather than under a
//! single chip's peripheral directory.

pub mod rom_thunks;
pub mod system_stub;
