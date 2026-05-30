// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Clock domain registry + observer fan-out.
//!
//! Clock-change is NOT scheduled through `EventScheduler`. `set_rate` is a
//! synchronous call that iterates observers and invokes
//! `Peripheral::on_clock_change` directly. This avoids a circular dependency
//! and keeps the heap focused on per-peripheral wakeups.
//!
//! Hooking ClockGraph into actual ESP32 register writes
//! (`DPORT_CPU_PER_CONF`, `RTC_CNTL_CLK_CONF`) is out of scope for 2B.1 and
//! lands with the matching DPORT / RTC_CNTL migration PRs (per design §12a).

use std::collections::HashMap;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ClockDomain {
    Cpu,
    Apb,
    RtcSlow,
    RtcFast,
    Xtal,
    Wifi,
    Bt,
    Custom(u32),
}

#[derive(Debug, Default)]
pub struct ClockGraph {
    rates: HashMap<ClockDomain, u64>,
    observers: HashMap<ClockDomain, Vec<u32>>,
}

impl ClockGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current rate in Hz for `domain`. Returns 0 if the domain was never
    /// configured — peripherals must seed their domain via `set_rate` (with
    /// no observers attached yet) at construction time.
    pub fn rate(&self, domain: ClockDomain) -> u64 {
        self.rates.get(&domain).copied().unwrap_or(0)
    }

    /// Update the rate for `domain` and synchronously notify every observer
    /// peripheral. Out-of-range peripheral indices are silently skipped.
    pub fn set_rate<F>(&mut self, domain: ClockDomain, new_hz: u64, mut notify: F)
    where
        F: FnMut(u32, ClockDomain, u64),
    {
        self.rates.insert(domain, new_hz);
        if let Some(observers) = self.observers.get(&domain) {
            for &idx in observers {
                notify(idx, domain, new_hz);
            }
        }
    }

    /// Subscribe `peripheral_idx` to clock-rate changes on `domain`.
    /// Idempotent: duplicate subscriptions are deduplicated.
    pub fn subscribe(&mut self, domain: ClockDomain, peripheral_idx: u32) {
        let list = self.observers.entry(domain).or_default();
        if !list.contains(&peripheral_idx) {
            list.push(peripheral_idx);
        }
    }

    pub fn observers(&self, domain: ClockDomain) -> &[u32] {
        self.observers
            .get(&domain)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}
