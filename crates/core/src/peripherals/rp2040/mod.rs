// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 native peripheral models (datasheet §2.3.1 SIO GPIO, §4.3 I2C,
//! §4.4 SPI, §4.6 TIMER). The clocks/resets subsystem lives one level up in
//! [`crate::peripherals::rp2040_clocks`].

pub mod dma;
pub mod i2c;
pub mod sio;
pub mod spi;
pub mod timer;
pub mod usb;
pub mod xip_ssi;
