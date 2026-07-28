// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 UART controller (UART0/1/2).
//!
//! The model itself lives in [`crate::peripherals::esp_uart`] — the ESP32-C3
//! carries the same UART IP, so the twin is chip-neutral and both families
//! wire it. This module keeps the S3's name for it plus the S3-specific
//! interrupt-matrix source ids; the C3's live in
//! [`crate::peripherals::esp32c3::uart`].

pub use crate::peripherals::esp_uart::EspUart as Esp32s3Uart;

/// Interrupt-matrix source for UART0 (`ETS_UART0_INTR_SOURCE`); UART1 = 28 and
/// UART2 = 29 follow it, and the chip yaml names them via `irq:`.
pub const UART0_INTR_SOURCE_ID: u32 = 27;
