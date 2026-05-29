// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 peripheral models with vendor-specific register layouts.
//!
//! These live in a dedicated module because the Nordic peripherals are
//! task/event-driven and their register layouts are unrelated to the
//! STM32 / ARM-PrimeCell layouts the generic models target.  Cross-
//! validated against real silicon by the `hw-oracle` crate's
//! `nrf52_onboarding_diff` test.

pub mod aar;
pub mod acl;
pub mod bprot;
pub mod ccm;
pub mod clock;
pub mod comp;
pub mod cryptocell;
pub mod ecb;
pub mod egu;
pub mod ficr;
pub mod gpiote;
pub mod i2s;
pub mod lpcomp;
pub mod mwu;
pub mod nfct;
pub mod nvmc;
pub mod pdm;
pub mod ppi;
pub mod pwm;
pub mod qdec;
pub mod qspi;
pub mod rng;
pub mod rtc;
pub mod saadc;
pub mod temp;
pub mod timer;
pub mod uicr;
pub mod usbd;
pub mod usbregulator;
pub mod wdt;
