// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 peripheral models with vendor-specific register layouts.
//!
//! These live in a dedicated module because the Nordic peripherals are
//! task/event-driven and their register layouts are unrelated to the
//! STM32 / ARM-PrimeCell layouts the generic models target.  Cross-
//! validated against real silicon by the `hw-oracle` crate's
//! `nrf52_onboarding_diff` test.
//!
//! # Walk deletion on nRF52: `uses_scheduler()` is the whole story
//!
//! ONE home for the rule every scheduler-driven model in this module follows,
//! so it is not restated (and drifted) seven times.
//!
//! [`crate::bus::SystemBus::derive_walk_deletable`] is the sole consumer of
//! [`crate::Peripheral::needs_legacy_walk`] in the tree:
//!
//! ```ignore
//! self.peripherals.iter().all(|p| p.dev.uses_scheduler() || !p.dev.needs_legacy_walk())
//! ```
//!
//! An unconditional `uses_scheduler() -> true` short-circuits that OR, so on
//! such a model `needs_legacy_walk()` is dead: it cannot affect walk deletion,
//! and it cannot affect the per-cycle walk's skip either — that keys on
//! `uses_scheduler()` alone (`bus/tick.rs`, the `p.dev.uses_scheduler() &&
//! !force_scheduler_walk` early return).
//!
//! These models therefore leave `needs_legacy_walk()` at its conservative
//! default (`true`) and carry the walk-deletion claim entirely on
//! `uses_scheduler()`. Overriding it to `false` alongside a non-default
//! `tick()` — which several of them used to do — bought nothing and asserted
//! something untrue: `tick()` here pends IRQs and drains event latches. The
//! static contract in `crate::tests::walk_starvation_contract` (rule A) rejects
//! that shape precisely because it is the fingerprint of a REAL starvation
//! (ESP32-C3 RMT, RP2040 I2C0), where the model had no `uses_scheduler()` to
//! carry it and the `false` really did delete the walk out from under a live
//! `tick()`.
//!
//! The load-bearing consequence: dropping `uses_scheduler()` from any model
//! here turns it into a walk FORCER (default `needs_legacy_walk() == true`) and
//! costs the whole nRF52 bus its 512x peripheral-tick batching. Pinned by
//! `the_seven_stay_scheduler_driven` in
//! `crates/core/tests/nrf52_walk_starvation_delivery.rs`, alongside the NVIC
//! delivery probes for each model's interrupt.

pub mod aar;
pub mod acl;
pub mod bprot;
pub mod ccm;
pub mod clock;
pub mod comp;
pub mod cryptocell;
pub mod ecb;
pub mod egu;
pub mod factory;
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
pub mod radio;
pub mod rng;
pub mod rtc;
pub mod saadc;
pub mod serial_instance;
pub mod spis;
pub mod temp;
pub mod timer;
pub mod twim;
pub mod twis;
pub mod uarte;
pub mod uicr;
pub mod usbd;
pub mod usbregulator;
pub mod wdt;
