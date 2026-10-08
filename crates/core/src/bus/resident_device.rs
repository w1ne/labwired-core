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
//! Finite GPIO waveforms share the same resident list and expose generic
//! deadlines, serviced independently of the ordinary peripheral tick.

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

    /// Open-drain host level: a disabled output driver releases the line.
    fn released_output_bit(&self, addr: u64, bit: u8) -> Option<bool> {
        self.output_bit(addr, bit)
    }

    /// The PAD level of pin `bit` on the GPIO port whose output register is
    /// at `addr` — what a probe on the wire reads, including a peripheral
    /// that owns the pin (an nRF GPIOTE Task-mode channel, a UART TX) rather
    /// than the port's output register. Defaults to [`Self::output_bit`], the
    /// right answer wherever the port register is the only driver.
    fn pad_bit(&self, addr: u64, bit: u8) -> Option<bool> {
        self.output_bit(addr, bit)
    }

    /// Configured waveform grid, independent of coalesced tick accounting.
    fn peripheral_tick_interval(&self) -> u64 {
        1
    }

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

    fn released_output_bit(&self, addr: u64, bit: u8) -> Option<bool> {
        use crate::Bus;
        let esp = self.find_peripheral_index(addr).is_some_and(|idx| {
            self.peripherals[idx].dev.as_any().is_some_and(|dev| {
                dev.is::<crate::peripherals::esp32c3::gpio::Esp32c3Gpio>()
                    || dev.is::<crate::peripherals::esp32::gpio::Esp32Gpio>()
                    || dev.is::<crate::peripherals::esp32s3::gpio::Esp32s3Gpio>()
            })
        });
        if esp
            && self
                .read_u32(addr.wrapping_add(0x1C))
                .is_ok_and(|enabled| enabled & (1 << bit) == 0)
        {
            return Some(true);
        }
        self.output_bit(addr, bit)
    }

    fn pad_bit(&self, addr: u64, bit: u8) -> Option<bool> {
        let idx = self.find_peripheral_index(addr)?;
        self.peripherals[idx]
            .dev
            .read_gpio_pad(bit)
            .or_else(|| self.output_bit(addr, bit))
    }

    fn peripheral_tick_interval(&self) -> u64 {
        self.resident_tick_interval_override
            .unwrap_or(u64::from(self.config.peripheral_tick_interval.max(1)))
    }

    fn drive_idr_bit(&mut self, addr: u64, bit: u8, high: bool) {
        // Explicit path: the inherent method, not this trait method.
        SystemBus::drive_idr_bit(self, addr, bit, high)
    }

    fn drive_input_bit(&mut self, addr: u64, bit: u8, high: bool) -> bool {
        SystemBus::drive_input_bit(self, addr, bit, high)
    }
}

/// One `outputs:` role of a declarative I²C / SPI part, resolved to a pad.
///
/// The device only ever speaks role names; the bus owns the translation. That
/// asymmetry is the point: a part descriptor is portable across boards because
/// it never learns which pin it was wired to, and the bus never learns what a
/// part's rules do.
#[derive(Debug, Clone)]
pub struct DevicePinPad {
    /// system.yaml `external_devices` id of the device that drives it.
    pub device_id: String,
    /// The descriptor's `outputs:` role name (`INT`, `DRDY`).
    pub role: String,
    /// Resolved input-register address the MCU samples.
    pub addr: u64,
    pub bit: u8,
}

/// A stimulus device resident directly on the [`SystemBus`] that drives GPIO
/// input-register pins once per peripheral tick and exposes one SimInput
/// channels. Declarative GPIO devices and simple contacts share this contract.
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

    /// Next effective waveform deadline, including already-due work.
    fn next_edge_deadline_cycle(&self, _now: u64, _interval: u64) -> Option<u64> {
        None
    }

    /// Apply due waveform edges without observing host pins or advancing timers.
    fn service_scheduled_edges(&mut self, _pins: &mut dyn DevicePins, _now: u64, _interval: u64) {}

    fn schedule_revision(&self) -> u64 {
        0
    }

    /// Whether configured waveforms depend on the peripheral tick grid.
    fn has_grid_schedules(&self) -> bool {
        false
    }

    /// This device as a SimInput stimulus target (all three expose one channel).
    fn as_sim_input(&mut self) -> &mut dyn crate::sim_input::SimInput;

    /// Stable system.yaml id, for sim-input targeting + diagnostics.
    fn id(&self) -> &str;

    /// Whether this device needs the per-cycle [`service`](Self::service) pass
    /// to stay correct.
    ///
    /// `true` is the default — the safe answer — so a device added later gets
    /// serviced unless its author deliberately opts out. Two kinds of device
    /// opt out, for two different reasons, and both reasons have to be stated
    /// or the predicate turns into "whatever the button needed":
    ///
    /// * **Level-driven on stimulus.** A push button's contact holds its level
    ///   until something moves it, and that level is applied at the stimulus
    ///   apply point. Nothing a tick could do would change it.
    /// * **Edge-serviced from the write hook.** A device whose pads move ONLY
    ///   when firmware stores to a GPIO output register, that owns no timer and
    ///   drives no pad, is already serviced synchronously inside the MMIO write
    ///   path (see [`edge_service_addrs`](Self::edge_service_addrs)). A tick
    ///   pass would resample pads that cannot have moved since the last store.
    ///   The bit-banged TM1637 and the direct-drive 7-segment digit are both
    ///   this — which is exactly why their hand-written predecessors lived
    ///   OUTSIDE `gpio_devices` on their own typed bus fields and cost no bus
    ///   its fast path.
    ///
    /// Everything else here is scanned or sampled per tick — a keypad re-reads
    /// the driven row every cycle, an encoder walks a Gray sequence, a DHT22
    /// clocks out a timed frame, an HX711 arms a power-on timer — and must say
    /// `true`.
    ///
    /// This exists so a bus hosting only such devices keeps the walk-free fast
    /// path (see [`SystemBus::per_cycle_tick_is_trivial`]) and its relaxed tick
    /// interval, without that optimisation ever being able to silently un-wire
    /// a device that does need servicing.
    ///
    /// [`SystemBus::per_cycle_tick_is_trivial`]: crate::bus::SystemBus
    fn needs_per_cycle_service(&self) -> bool {
        true
    }

    /// Whether the machine may skip an idle (WFI) window right now without
    /// this device missing anything, even though it
    /// [needs per-cycle service](Self::needs_per_cycle_service) while it works.
    ///
    /// The default is exactly that predicate negated, so no device changes
    /// unless it opts in. A device opts in when everything it does on its own
    /// clock is in its [scheduled edges](Self::next_edge_deadline_cycle) and
    /// everything else is answered from the write hook: with no edge pending it
    /// is idle, and a skip moves time it would not have used. With an edge
    /// pending it must say `false`, so an in-flight waveform stays
    /// cycle-accurate.
    fn idle_skip_safe(&self) -> bool {
        !self.needs_per_cycle_service()
    }

    /// Output-register addresses whose MMIO writes must service this device
    /// **synchronously**, instead of waiting for the next peripheral tick.
    ///
    /// Empty (the default) ⇒ tick-driven, which is what every device written
    /// before this method existed is, and what a device sampled or scanned on a
    /// schedule should stay.
    ///
    /// ## Why a tick is not enough for some parts
    ///
    /// A bit-banged part is clocked by FIRMWARE, not by the simulator. An
    /// HX711 read loop toggles SCK and reads DOUT between two instructions;
    /// a TM1637 frames a byte from the order two pads move in. The peripheral
    /// tick runs every N cycles, so a service pass that only runs there sees
    /// the pad AFTER the firmware has already moved it back — it samples
    /// levels and misses EDGES. A 24-bit shift-out clocked by 48 stores inside
    /// one tick interval delivers one edge, or none.
    ///
    /// That is why the hand-written HX711 and TM1637 models each grew their own
    /// `Vec` on the bus and their own write hook. This is the one hook they
    /// were each re-implementing, available to any resident device — and, in
    /// practice, to the `gpio_device` primitive, which is how a bit-banged part
    /// stops being Rust.
    ///
    /// The addresses are OUTPUT registers (what the MCU drives), because those
    /// are the writes that can move an observed pad. A device lists the pads it
    /// watches; the bus decides which peripheral hosts each one.
    ///
    /// Returning a slice rather than a `Vec` on purpose: this is consulted on
    /// every MMIO write to a peripheral, and an allocation per write would be a
    /// cost the whole engine pays for one part's wiring.
    fn edge_service_addrs(&self) -> &[u64] {
        &[]
    }

    /// The service the write hook runs, after a store to a peripheral that
    /// hosts one of [`edge_service_addrs`](Self::edge_service_addrs).
    ///
    /// Default: [`service`](Self::service), the same pass the tick runs. A
    /// device whose pads can move only on a store overrides it to read its
    /// pads here, and keeps [`service`](Self::service) (called on every tick
    /// on a bus whose tick is not trivial) free of pad reads.
    fn service_edge(&mut self, pins: &mut dyn DevicePins, now: u64) {
        self.service(pins, now);
    }

    /// What this device currently holds, for the inspect/evidence walk — or
    /// `None` when it has nothing to show.
    ///
    /// A bus-resident device that is a DISPLAY has no controller trait to hang
    /// its artifacts on: it binds on PINS. Before this method, the only way one
    /// could report was to sit on its own typed `SystemBus` field with its own
    /// arm in `for_each_bus_resident_device` — which is precisely the plumbing
    /// the TM1637 and the 7-segment digit were kept in Rust by. Evidence is a
    /// read-only inspection seam, not an engine handle, so it does not widen
    /// [`DevicePins`]: nothing here can touch the machine.
    fn evidence(&self) -> Option<&dyn crate::inspect::DeviceEvidence> {
        None
    }

    /// The named text logs this device records during a run, one line per
    /// entry (see [`crate::peripheral_log`]). `labwired test` asserts on them
    /// with `peripheral_log`, naming the device by its `external_devices` id.
    ///
    /// Return every log the device keeps, also an empty one: the names given
    /// here are the only list of valid names. Default: no logs.
    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        Vec::new()
    }

    /// Concrete-type escape hatch for typed readback / diagnostics (see
    /// [`SystemBus::gpio_devices_of`]). The service/stimulus paths never
    /// downcast — this is only for callers that want a specific model back out.
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

impl SystemBus {
    /// Iterate the bus-resident GPIO-stimulus devices of concrete type `T`
    /// (e.g. `DeclarativeGpioDevice`), for readback / diagnostics. The runtime never needs this
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
