// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 Wi-Fi MAC (WDEV) at `0x6003_3000`.
//!
//! The S3 uses the same MAC IP as the C3: RX descriptor ring, TX PLCP kick,
//! event `0xC3C`/`0xC40`, interrupt-matrix source 0 (`ETS_WIFI_MAC_INTR_SOURCE`).
//! A `wifi-ap` on the diagram attaches this MAC to [`virtual_wifi`] through
//! [`crate::system::wifi::attach_configured_wifi_ap`].

pub use crate::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac as Esp32s3WifiMac;
