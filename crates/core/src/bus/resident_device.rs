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

/// The ONLY thing a [`BusResidentDevice`] is handed while it drives its pins:
/// three pad operations over `(address, bit)`, and nothing else.
///
/// # Why this exists
///
/// `service` used to take `&mut SystemBus`. Every off-chip stimulus device in
/// the tree — a push button, a keypad, an encoder, a temperature sensor — was
/// therefore typed against the whole machine: 47 public fields and 90 public
/// inherent methods (240 inherent methods in all, every one of them reachable
/// because the devices live in the same crate), plus the `Bus` trait, plus
/// flash, RAM, the peripheral table, the interrupt fabric and the trace log.
///
/// What the four implementations actually used was three operations, and two of
/// them use only two. That gap is the binding cost the C-1 ledger row names: a
/// device could not be written, moved or tested without the entire bus in
/// scope, and nothing in the type system said it did not need it.
///
/// # What each method means
///
/// A stimulus device is on the far side of the pad from the MCU. It reads what
/// the MCU is *driving out* (`output_bit`) and it drives what the MCU *samples
/// in* — through the input register for pad-level models
/// ([`drive_idr_bit`](Self::drive_idr_bit)) and through the external-world seam
/// for models where IN is not a writable register
/// ([`drive_input_bit`](Self::drive_input_bit), the ESP32 GPIO case). Both
/// writers are transition-only at the bus, so an idle device costs nothing.
///
/// The port stays primitive on purpose: every argument and return is a `u64`,
/// `u8` or `bool`. Handing back a `&mut SystemBus` — or any other engine type —
/// through a new method would undo the narrowing without breaking a single
/// build, so `resident_device_port_stays_narrow` in
/// `crates/core/tests/bus_resident_device_port.rs` reads this trait's body and
/// fails if an engine type reappears in it.
pub trait DevicePins {
    /// Level the MCU is currently driving on the output register at `addr`,
    /// bit `bit`, or `None` when that address does not read back. The caller
    /// picks the default for `None`; an undriven line is not universally high.
    fn output_bit(&self, addr: u64, bit: u8) -> Option<bool>;

    /// Set or clear one bit of a GPIO input (IDR) register, writing back only
    /// when the bit actually changes.
    fn drive_idr_bit(&mut self, addr: u64, bit: u8, high: bool);

    /// Drive one pad's external level through the peripheral's own input seam
    /// (`set_gpio_input`), for models where IN is read-only to MMIO and a store
    /// is correctly ignored. Returns whether a peripheral claimed the address.
    fn drive_input_bit(&mut self, addr: u64, bit: u8, high: bool) -> bool;
}

/// The bus is the one real port. The devices never learn that.
impl DevicePins for SystemBus {
    fn output_bit(&self, addr: u64, bit: u8) -> Option<bool> {
        use crate::Bus; // `read_u32` is a Bus-trait method
        self.read_u32(addr).ok().map(|v| (v >> bit) & 1 != 0)
    }

    fn drive_idr_bit(&mut self, addr: u64, bit: u8, high: bool) {
        // Explicit path: the inherent method, not this trait method.
        SystemBus::drive_idr_bit(self, addr, bit, high)
    }

    fn drive_input_bit(&mut self, addr: u64, bit: u8, high: bool) -> bool {
        SystemBus::drive_input_bit(self, addr, bit, high)
    }
}

/// A stimulus device resident directly on the [`SystemBus`] that drives GPIO
/// input-register pins once per peripheral tick and exposes one SimInput
/// channel. Implemented by `Dht22`, `RotaryEncoder`, `Keypad` and `Button`.
pub trait BusResidentDevice: std::fmt::Debug + Send {
    /// Drive this device's output (input-register) pins for simulated cycle
    /// `now`. Called once per peripheral tick, in registration order. Reads
    /// whatever input it needs and writes its pins through [`DevicePins`],
    /// touching the bus only on a transition — exactly as the old `drive_<x>`
    /// did.
    ///
    /// `pins` is the whole machine this device may touch. It is deliberately
    /// not the bus: see [`DevicePins`].
    fn service(&mut self, pins: &mut dyn DevicePins, now: u64);

    /// This device as a SimInput stimulus target (all three expose one channel).
    fn as_sim_input(&mut self) -> &mut dyn crate::sim_input::SimInput;

    /// Stable system.yaml id, for sim-input targeting + diagnostics.
    fn id(&self) -> &str;

    /// Whether this device's pin level changes ONLY when a stimulus drives it,
    /// so it needs no per-cycle [`service`](Self::service) pass to stay correct.
    ///
    /// A push button is the one such device: a contact holds its level until
    /// something moves it, and that level is applied at the stimulus apply
    /// point. Everything else here is scanned or sampled per tick — a keypad
    /// re-reads the driven row every cycle, an encoder walks a Gray sequence, a
    /// DHT22 clocks out a timed frame — and must say `false`.
    ///
    /// This exists so a bus hosting only a button keeps the walk-free fast path
    /// (see [`SystemBus::per_cycle_tick_is_trivial`]) without that optimisation
    /// ever being able to silently un-wire a device that does need servicing.
    /// It defaults to `false` — the safe answer — so a device added later gets
    /// serviced unless its author deliberately opts out.
    ///
    /// [`SystemBus::per_cycle_tick_is_trivial`]: crate::bus::SystemBus
    fn is_level_driven_on_stimulus(&self) -> bool {
        false
    }

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
