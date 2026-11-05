// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-C5 peripheral support for the current DevKitC-1 board profile.
//!
//! The initial slice models UART0 TX through the shared Espressif UART twin.
//! GPIO/RMT, IO_MUX pad routing, radios, USB and the LP core are not claimed.

pub mod factory;
pub mod pcr;
pub mod uart;
