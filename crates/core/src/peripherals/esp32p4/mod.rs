// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-P4 HP-core support for the P4X Function EV board GPIO smoke.
//!
//! This profile models one HP RISC-V core and GPIO8 on the J1 header. The
//! second HP core, LP core, boot ROM, flash controller, radios, USB, audio,
//! Ethernet and multimedia blocks are outside this slice.

pub mod factory;
pub mod gpio;
pub mod io_mux;
