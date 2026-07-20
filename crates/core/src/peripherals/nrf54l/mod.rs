// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF54L peripheral models.
//!
//! The nRF54L family keeps the Nordic task/event register fabric but
//! replaces several nRF52 blocks outright. Peripherals that are still
//! register-compatible with nRF52 (UARTE, TIMER, GPIO, TEMP, EGU, GPIOTE)
//! are reused from [`crate::peripherals::nrf52`]; only the blocks that are
//! genuinely new to this family live here.
//!
//! Currently modelled: GRTC (the Global Real-time Counter, which replaces
//! RTC0/RTC1 and is the kernel tick source for Zephyr on this family).

pub mod factory;
pub mod grtc;
