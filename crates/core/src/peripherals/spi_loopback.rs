// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Wire loopback for an SPI controller (`SDO` tied to `SDI`).
//!
//! This is the harness counterpart of the jumper a board bring-up uses to
//! check that a controller shifts bits at all: every byte sent comes back
//! unchanged, so the firmware can verify the transfer path without a real
//! slave. It is not a silicon model and implements no protocol.
//!
//! The PIC24 M3 example declares it as an `external_devices` entry with
//! `type: "loopback"` and `connection: "spi1"`; the session builder attaches
//! it to the CPU-internal SPI1 controller.

use crate::peripherals::device::SpiDevice;

#[derive(Debug, Clone)]
pub struct SpiLoopback {
    /// CS label the device is wired to (kept for the `SpiDevice` contract;
    /// the PIC24 SPI model has no CS routing).
    cs: String,
}

impl SpiLoopback {
    pub fn new(cs: impl Into<String>) -> Self {
        Self { cs: cs.into() }
    }
}

impl Default for SpiLoopback {
    fn default() -> Self {
        Self::new("")
    }
}

impl SpiDevice for SpiLoopback {
    fn transfer(&mut self, mosi: u8) -> u8 {
        mosi
    }

    fn cs_pin(&self) -> &str {
        &self.cs
    }
}
