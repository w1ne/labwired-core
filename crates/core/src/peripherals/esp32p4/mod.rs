// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-P4 HP-core support for the P4X Function EV board smoke.
//!
//! This profile models one HP RISC-V core, the I2C0 command controller, and
//! GPIO8 on the J1 header. The external BME280 fixture attaches at I2C0's
//! controller boundary; physical SDA/SCL pads and GPIO-matrix routing are not modeled.
//! second HP core, LP core, boot ROM, flash controller, radios, USB, audio,
//! Ethernet and multimedia blocks are outside this slice.

pub mod factory;
pub mod gpio;
pub mod io_mux;
