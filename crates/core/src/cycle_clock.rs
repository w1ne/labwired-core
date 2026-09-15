// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bus-published simulation cycle clock — the read-side freshness mechanism
//! from the walk-free plan (Part 1, option (b)+(c)).
//!
//! ## The problem it solves
//!
//! Scheduler-migrated peripherals advance lazily: the bus calls
//! `Peripheral::sync_to(current_cycle)` before an MMIO **write** observes
//! them. But firmware also polls free-running counters by **reading** them,
//! and the bus read path is `&self` — a read cannot call `sync_to(&mut …)`.
//! Making reads `&mut` was evaluated and rejected (134 `impl Peripheral`
//! blocks, ~1500 `Bus::read_*` call sites, and — fatally — the CPU holds
//! shared borrows of peripheral-backed buffers during instruction fetch).
//!
//! ## The mechanism
//!
//! The bus owns a [`CycleClock`] (an `Arc<AtomicU64>`) and publishes
//! `current_cycle` into it at exactly the points `bus.current_cycle` itself
//! is refreshed — batch start, batch end, per-step, and idle fast-forward
//! (see `SystemBus::set_current_cycle`). Peripherals receive a clone at
//! attach time via [`crate::Peripheral::attach_cycle_clock`] and may consult
//! it from a `&self` read, advancing `Cell`-held counter state to "now"
//! (the `Peripheral` trait is `Send`, not `Sync`, and a machine is
//! single-threaded, so interior mutability is sound; `Arc<AtomicU64>` rather
//! than `Rc<Cell>` keeps the `Send` bound).
//!
//! ## The determinism contract (batch-boundary freshness)
//!
//! During a CPU batch `current_cycle` holds the **batch-start** cycle, so a
//! read synced to the published clock is exact at batch boundaries and
//! trails the true cycle by strictly less than one `peripheral_tick_interval`
//! mid-batch — **identical to the bound the write-path `sync_to` already
//! ships** (see the doc on `Peripheral::sync_to`). At interval 1 batches are
//! one instruction and the value is exact everywhere the legacy walk was.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Shared, bus-published "now" in CPU cycles. Cheap to clone (one `Arc`);
/// one instance per [`crate::bus::SystemBus`], handed to peripherals at
/// attach time.
#[derive(Debug, Clone, Default)]
pub struct CycleClock {
    inner: Arc<AtomicU64>,
}

impl CycleClock {
    /// The most recently published CPU cycle (the batch-start cycle during a
    /// CPU batch — see the module docs for the freshness bound).
    #[inline]
    pub fn now(&self) -> u64 {
        self.inner.load(Ordering::Relaxed)
    }

    /// Publish `cycle` as "now". Called by the bus wherever
    /// `bus.current_cycle` is refreshed; peripherals never publish.
    #[inline]
    pub fn publish(&self, cycle: u64) {
        self.inner.store(cycle, Ordering::Relaxed);
    }
}

/// Define the private `scheduler_mode()` drive-mode predicate on a peripheral
/// that stores its bus clock in a `clock: Option<CycleClock>` field.
///
/// Invoke it inside the type's own inherent `impl`, where the hand-written
/// method used to sit:
///
/// ```ignore
/// impl Timer {
///     crate::cycle_clock::scheduler_mode!();
/// }
/// ```
///
/// # What the predicate means
///
/// True when the event scheduler owns this model's time base: the
/// `event-scheduler` feature is compiled in AND the bus handed this instance
/// its shared [`CycleClock`] at attach time. Everything drive-mode-related
/// branches on this ONE predicate so the two modes can never mix.
///
/// The clock is the only evidence a peripheral has that it went through
/// `SystemBus::add_peripheral`/`push_peripheral`, and therefore that it sits on
/// a bus whose `Machine` drains the event scheduler. Hand-built buses (tests,
/// embedders that push a `PeripheralEntry` directly and settle with
/// `tick_peripherals*`) stay on the legacy walk with exact historical
/// semantics — the contract documented on [`crate::Peripheral::attach_cycle_clock`].
/// `force_legacy_walk()` drops the clock to pin a model back onto the walk,
/// which is how the walk-vs-scheduler differential gates build their reference
/// lane from the same bus assembly.
///
/// # Why a macro
///
/// This is a *private inherent* method on 30 unrelated peripheral types, so a
/// trait default cannot supply it without widening its visibility, adding a
/// per-type accessor impl and a per-file import, and putting a trait method in
/// name-resolution competition with the four models that keep a hand-written
/// `scheduler_mode` of their own (`Exti`, `Fdcan`, the `I2c` enum, and the nRF52
/// `Timer`, whose bodies are all genuinely different). Expanding the method in
/// place keeps it private, inherent, and callable exactly as before — a pure
/// refactor with no name-resolution change at any call site — while collapsing
/// 30 copies of the `cfg!(feature = "event-scheduler")` conditional-compilation
/// site into this one. See `crate::tests::event_scheduler_cfg_ratchet`.
macro_rules! scheduler_mode {
    () => {
        #[inline]
        fn scheduler_mode(&self) -> bool {
            cfg!(feature = "event-scheduler") && self.clock.is_some()
        }
    };
}

pub(crate) use scheduler_mode;
