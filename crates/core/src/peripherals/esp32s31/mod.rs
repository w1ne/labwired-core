// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Focused ESP32-S31 GPIO support for the ESP-Mosaico V1.2 board profile.
//!
//! The model includes only the GPIO7 function button input and GPIO8 haptic
//! control output used by the onboarding smoke.

pub mod factory;
pub mod gpio;
pub mod io_mux;
