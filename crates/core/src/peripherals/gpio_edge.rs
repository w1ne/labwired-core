// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Shared GPIO edge-observer contract for bit-bang peripherals.
//!
//! Classic ESP32, ESP32-S3, ESP32-C3, and STM32 [`GpioPort`](super::gpio::GpioPort)
//! all notify the same trait when an output pad changes level. Kits install
//! one observer via [`crate::bus::SystemBus::install_gpio_observer`].

use std::sync::Arc;

/// Notified synchronously on every GPIO pin transition.
pub trait GpioEdgeObserver: Send + Sync + std::fmt::Debug {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64);
}

/// Remap local port bits onto a global pin id (`port_index * 16 + bit`).
#[derive(Debug)]
pub struct PinOffsetObserver<T: ?Sized> {
    inner: Arc<T>,
    offset: u8,
}

impl<T: ?Sized> PinOffsetObserver<T> {
    pub fn new(inner: Arc<T>, offset: u8) -> Self {
        Self { inner, offset }
    }
}

impl<T: GpioEdgeObserver + ?Sized> GpioEdgeObserver for PinOffsetObserver<T> {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64) {
        self.inner
            .on_pin_change(self.offset.saturating_add(pin), from, to, sim_cycle);
    }
}

pub fn notify_bits_changed(
    observers: &[Arc<dyn GpioEdgeObserver>],
    old: u32,
    new: u32,
    width: u8,
    sim_cycle: u64,
) {
    let diff = old ^ new;
    if diff == 0 || observers.is_empty() {
        return;
    }
    for pin in 0..width {
        let mask = 1u32 << pin;
        if diff & mask != 0 {
            let from = old & mask != 0;
            let to = new & mask != 0;
            for obs in observers {
                obs.on_pin_change(pin, from, to, sim_cycle);
            }
        }
    }
}
