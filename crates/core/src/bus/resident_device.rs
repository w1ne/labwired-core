// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! One trait for the tick-driven GPIO-stimulus devices that live directly on
//! the bus — the DHT22 one-wire sensor, the incremental rotary encoder, and the
//! 4×4 matrix keypad. Each of these DRIVES pins the MCU samples as inputs (and
//! answers no register read), so none can be a memory-mapped peripheral; each
//! also exposes exactly one [`SimInput`](crate::sim_input::SimInput) stimulus
//! channel. Rather than a separate `Vec` + `service_<x>`/`drive_<x>` pair per
//! type, they share ONE [`SystemBus::gpio_devices`] list serviced by ONE
//! [`SystemBus::service_gpio_devices`] pass.
//!
//! The HC-SR04 is deliberately NOT one of these: it carries the event-scheduler
//! edge-deadline path (`take_edge_schedule` / `apply_hcsr04_event`), which is
//! genuinely a different shape, so it keeps its own field and service pass.

use super::SystemBus;

/// A stimulus device resident directly on the [`SystemBus`] that drives GPIO
/// input-register pins once per peripheral tick and exposes one SimInput
/// channel. Implemented by `Dht22`, `RotaryEncoder`, and `Keypad`.
pub trait BusResidentDevice: std::fmt::Debug + Send {
    /// Drive this device's output (input-register) pins for simulated cycle
    /// `now`. Called once per peripheral tick, in registration order. Reads
    /// whatever input it needs and writes its pins via the bus's existing
    /// register IO (`read_u32` / `drive_idr_bit`), touching the bus only on a
    /// transition — exactly as the old `drive_<x>` did.
    fn service(&mut self, bus: &mut SystemBus, now: u64);

    /// This device as a SimInput stimulus target (all three expose one channel).
    fn as_sim_input(&mut self) -> &mut dyn crate::sim_input::SimInput;

    /// Stable system.yaml id, for sim-input targeting + diagnostics.
    fn id(&self) -> &str;

    /// Concrete-type escape hatch for typed readback / diagnostics (see
    /// [`SystemBus::gpio_devices_of`]). The service/stimulus paths never
    /// downcast — this is only for callers that want a specific model back out.
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

impl SystemBus {
    /// Iterate the bus-resident GPIO-stimulus devices of concrete type `T`
    /// (e.g. `Dht22`), for readback / diagnostics. The runtime never needs this
    /// — service and stimulus dispatch stay generic over the trait — but tests
    /// and UI readback occasionally want a concrete model back.
    pub fn gpio_devices_of<T: BusResidentDevice + 'static>(&self) -> impl Iterator<Item = &T> {
        self.gpio_devices
            .iter()
            .filter_map(|d| d.as_any().downcast_ref::<T>())
    }

    /// Mutable twin of [`Self::gpio_devices_of`].
    pub fn gpio_devices_of_mut<T: BusResidentDevice + 'static>(
        &mut self,
    ) -> impl Iterator<Item = &mut T> {
        self.gpio_devices
            .iter_mut()
            .filter_map(|d| d.as_any_mut().downcast_mut::<T>())
    }
}
