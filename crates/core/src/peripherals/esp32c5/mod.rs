// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-C5 peripheral support for the current DevKitC-1 board profile.
//!
//! The board slice models UART0 TX, the direct GPIO27 output used by its
//! addressable RGB LED, and the GPIO27 IO_MUX function select. GPIO/RMT beyond
//! that one output path, radios, USB and the LP core are not claimed.

pub mod factory;
pub mod gpio;
pub mod io_mux;
pub mod pcr;
pub mod uart;
