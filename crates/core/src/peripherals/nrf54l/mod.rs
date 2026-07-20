// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF54L peripheral models.
//!
//! The nRF54L family keeps the Nordic task/event register fabric but
//! replaces several nRF52 blocks outright. Peripherals that are still
//! register-compatible with nRF52 (TIMER, GPIO, TEMP, EGU, GPIOTE) are
//! reused from [`crate::peripherals::nrf52`]; only the blocks that are
//! genuinely new to this family live here.
//!
//! Currently modelled:
//!   * GRTC — the Global Real-time Counter, which replaces RTC0/RTC1 and is
//!     the kernel tick source for Zephyr on this family.
//!   * UARTE — the nRF54L-generation UART with EasyDMA. It is *not*
//!     register-compatible with the nRF52 UARTE: EasyDMA moved into a
//!     `DMA.{RX,TX}` cluster and the task/event offsets were renumbered.
//!   * TWIM — the nRF54L-generation I²C master. Same DMA-cluster change as
//!     the UARTE, so likewise not nRF52-compatible.
//!   * CLOCK — oscillator control. Also *not* nRF52-compatible despite the
//!     shared `nordic,nrf-clock` devicetree binding: the HFCLK/LFCLK task
//!     pair became an XO/PLL/LFCLK trio and every status register moved.

pub mod clock;
pub mod factory;
pub mod grtc;
pub mod twim;
pub mod uarte;
