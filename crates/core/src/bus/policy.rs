// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bus policy: path resolve, cycle-accurate mode, safe tick interval, HC-SR04 schedule helpers.

use super::*;

impl SystemBus {
    pub(crate) fn resolve_peripheral_path(
        manifest: &SystemManifest,
        descriptor_path: &str,
    ) -> PathBuf {
        let raw = PathBuf::from(descriptor_path);
        if raw.is_absolute() {
            return raw;
        }

        let chip_path = Path::new(&manifest.chip);
        let chip_dir = chip_path.parent().unwrap_or_else(|| Path::new("."));
        let chip_relative = chip_dir.join(descriptor_path);
        if chip_relative.exists() {
            chip_relative
        } else {
            raw
        }
    }

    /// True when the wired devices need cycle-accurate (non-batched) execution
    /// to behave correctly. Some external devices are driven from `tick_peripherals`
    /// and observed by cycle-tight firmware loops — e.g. the HC-SR04 holds ECHO
    /// high for a pulse the firmware times by polling GPIO IN in a busy loop.
    /// Batched execution advances many instructions before ticking peripherals,
    /// so the firmware polls a frozen ECHO and measures nothing. Runners should
    /// disable instruction batching when this returns true (correctness > speed).
    /// New per-tick GPIO-timing devices should extend this predicate.
    ///
    /// Also true when an H5 op-modeling FLASH is on the bus (`flash_models_ops`,
    /// cached in `rebuild_peripheral_ranges`): its erase/bank-swap ops are
    /// recorded as pending and drained+applied per instruction by the machine
    /// layer, an invariant that only holds at batch size 1. Without this the
    /// CLI/batch run path would record the op in the FLASH cell but never apply
    /// it (no 0xFF fill, no bank swap, no reset).
    ///
    /// An attached IO-Link master used to be an arm here. It no longer is: the
    /// shared `Uart` now replays one `poll` per tick-equivalent when it is
    /// serviced on a widened interval (`Uart::advance_ticks`), so the master
    /// sees exactly the poll count per simulated cycle it saw at interval 1 and
    /// its tick-counted startup schedule keeps its original length. Pinning the
    /// whole machine to one instruction per batch for it was costing every lab
    /// on the bus, not just the IO-Link ones.
    ///
    /// HOT: called per batch plan (`machine/plan.rs`), per interpreted step
    /// (`cpu/riscv.rs`) and in the idle fast-forward check (`lib.rs`), so every
    /// clause must be O(1). `flash_models_ops` is a bool cached at bus
    /// build/mutation; the
    /// HC-SR04 clause is deliberately NOT cached because it is run-dynamic —
    /// `hcsr04_event_scheduled` gates on `config.peripheral_tick_interval`,
    /// which the wasm engine (`set_peripheral_tick_interval`) and the
    /// differential tests change after build. It stays cheap on its own terms:
    /// a `Vec::is_empty` plus a few bool/int reads, no scan and no downcast.
    #[inline]
    pub fn requires_cycle_accurate(&self) -> bool {
        let hcsr04_needs_cycle_accurate = !self.hcsr04.is_empty() && !self.hcsr04_event_scheduled();
        // DHT22/DHT11 (and keypad / rotary) drive timed pad edges from
        // `service_gpio_devices`. Firmware times them with digitalRead + micros
        // busy-loops whose MMIO is SideEffectFree — so timer-poll idle
        // fast-forward would leap over the whole frame while the pad stays
        // frozen, and every freehand DHT read returns NaN (ESP32-C3, 2026-08-11).
        // Buttons opt out via `is_level_driven_on_stimulus` and do not force this.
        let gpio_timing_devices = self
            .gpio_devices
            .iter()
            .any(|d| !d.is_level_driven_on_stimulus());
        hcsr04_needs_cycle_accurate || self.flash_models_ops || gpio_timing_devices
    }

    /// The largest `peripheral_tick_interval` this bus can run at without
    /// losing fidelity: [`RECOMMENDED_TICK_INTERVAL`] when every peripheral is
    /// scheduler-driven (cycle-exact event deadlines, observation quantised by
    /// at most one interval), `1` when anything non-relaxable is present.
    ///
    /// Non-relaxable arms are checked directly rather than through
    /// [`Self::requires_cycle_accurate`]: that predicate treats HC-SR04 as
    /// cycle-accurate until the interval is ALREADY raised above 1
    /// (`hcsr04_event_scheduled` gates on it), so consulting it at interval 1
    /// would always answer "stay at 1". HC-SR04 itself is relaxable — its ECHO
    /// edges become scheduler events (batch-clamped to the exact edge) the
    /// moment the interval rises — except under the test-only
    /// `hcsr04_scheduling_disabled` override, which pins the legacy per-tick
    /// path. Callers (the wasm `recommended_tick_interval` getter) apply the
    /// result via `set_peripheral_tick_interval` at engine init.
    ///
    /// **H5 FLASH (`flash_models_ops`) is intentionally NOT a max_safe arm.**
    /// Erase/bank-swap ops are drained per instruction boundary by
    /// `Machine::apply_pending_flash_op`, and [`Self::requires_cycle_accurate`]
    /// still clamps the CPU quantum to 1 so no op is lost mid-batch. That is
    /// orthogonal to the peripheral tick interval: a walk-deleted H5 bus can
    /// run `RECOMMENDED_TICK_INTERVAL` for scheduler-paced peripherals while
    /// remaining cycle-accurate at the CPU/FLASH layer.
    pub fn max_safe_tick_interval(&self) -> u32 {
        // Per-tick GPIO-timing devices (DHT one-wire, keypad scan, rotary) need
        // a service pass every cycle until they grow an event-scheduled edge
        // path like HC-SR04. Raising the interval freezes the pad for N cycles
        // between services and under-samples µs-scale pulse widths.
        if self
            .gpio_devices
            .iter()
            .any(|d| !d.is_level_driven_on_stimulus())
        {
            return 1;
        }
        #[cfg(feature = "event-scheduler")]
        {
            let hcsr04_forced_legacy = !self.hcsr04.is_empty() && self.hcsr04_scheduling_disabled;
            if self.legacy_walk_disabled && !hcsr04_forced_legacy {
                return RECOMMENDED_TICK_INTERVAL;
            }
        }
        1
    }

    /// True when the HC-SR04 echo waveform is driven by the event scheduler
    /// (rise/fall edges scheduled at their exact cycles and drained by
    /// `Machine::drain_scheduler_events`) rather than the per-cycle
    /// `service_hcsr04` pass. Active only under the `event-scheduler` feature on
    /// a walk-deleted bus (`legacy_walk_disabled`) — the same buses that already
    /// route every migrated peripheral through the scheduler. On the legacy-walk
    /// or feature-off path the sensor stays on the per-tick service path and
    /// `requires_cycle_accurate` keeps batches at one instruction. The
    /// `hcsr04_scheduling_disabled` override forces the legacy path (differential
    /// determinism test only).
    ///
    /// Gated on `peripheral_tick_interval > 1`: at interval 1 there is no
    /// instruction batching to unlock (batches are already one instruction), so
    /// the scheduled path would only add per-cycle drain overhead for no win —
    /// the proven per-tick service path is kept, byte-for-byte identical to the
    /// pre-migration build. The scheduled path activates exactly when the browser
    /// raises the interval to batch, which is when it pays off (see the throughput
    /// numbers in the migration notes).
    #[inline]
    pub(crate) fn hcsr04_event_scheduled(&self) -> bool {
        cfg!(feature = "event-scheduler")
            && self.legacy_walk_disabled
            && !self.hcsr04.is_empty()
            && !self.hcsr04_scheduling_disabled
            && self.config.peripheral_tick_interval > 1
    }

    /// True when the per-cycle tick (`tick_peripherals_fully`) has no orchestration
    /// work beyond the NVIC scan: the legacy peripheral walk is deleted, no
    /// bus-aware peripheral needs a pre-tick pass, no Nordic GPIO/GPIOTE service
    /// is wired, no CAN synthetic testers are attached, and every HC-SR04 (if any)
    /// is event-scheduled. On such a bus the tick early-outs to just the NVIC
    /// aggregation, avoiding the phase-1 orchestration and its allocations every
    /// cycle. Only meaningful under the `event-scheduler` feature (the walk is
    /// never deleted otherwise).
    ///
    /// ESP32-C3 IRQ routing no longer pins this to `false` when the cached
    /// aggregation is available: on a walk-deleted C3 bus there are no
    /// tick-produced peripheral sources (nothing walks), and the remaining
    /// routing inputs — INTC config + FROM_CPU IPI — are re-aggregated at
    /// their MMIO write choke (`sync_esp32c3_irq_cache_write`), so the
    /// per-cycle tick genuinely has nothing left to do. Without the cache
    /// (hand-built buses) the per-tick register-read fallback is the only
    /// aggregation point, so it keeps the walk-era behaviour.
    ///
    /// `gpio_devices` counts as work. A bus-resident device (keypad, rotary
    /// encoder, DHT22) DRIVES pins the firmware samples, and
    /// `service_gpio_devices` — the pass that does the driving — lives inside
    /// the phase-1 body this predicate skips. Omitting the check made those
    /// devices silently inert on every walk-deleted bus: attach succeeded,
    /// `list_inputs` advertised the channel, `set_input` returned Ok, and the
    /// pin never moved. Walk deletion is a performance decision about
    /// peripheral ORCHESTRATION; a device driving a pin is not orchestration,
    /// and no fast path may decide it stops existing.
    ///
    /// The cost is bounded: only buses that actually host such a device give up
    /// the fast path, and `service_gpio_devices` early-outs on an empty list,
    /// so a bus without one is unaffected. Buttons deliberately do NOT rely on
    /// this — a contact level changes only when something drives it, so it is
    /// applied at the stimulus point (`sync_button_inputs`) and a button alone
    /// never costs a bus its fast path.
    #[cfg(feature = "event-scheduler")]
    #[inline]
    pub(crate) fn per_cycle_tick_is_trivial(&self) -> bool {
        self.legacy_walk_disabled
            && self.bus_tick_indices.is_empty()
            && !self.nordic_gpio_service
            && self.irq_fabric.per_cycle_aggregation_free()
            && self.can_diagnostic_testers.is_empty()
            && self.can_uds_testers.is_empty()
            && self.can_log_players.is_empty()
            && self.no_gpio_device_needs_service()
            && (self.hcsr04.is_empty() || self.hcsr04_event_scheduled())
    }

    /// Whether NO attached bus-resident device needs the per-cycle service pass
    /// — i.e. skipping [`service_gpio_devices`](Self::service_gpio_devices)
    /// changes nothing observable.
    ///
    /// A [`Button`](crate::peripherals::components::button::Button) is exempt:
    /// its level is applied when the contact is driven, not per cycle, so a
    /// canvas that adds a push button keeps the walk-free fast path. Every
    /// other device is scanned or sampled per tick and does need it.
    ///
    /// Vacuously true on an empty list, which is what preserves the fast path
    /// for the overwhelmingly common bus that hosts no such device at all.
    #[cfg(feature = "event-scheduler")]
    #[inline]
    fn no_gpio_device_needs_service(&self) -> bool {
        self.gpio_devices
            .iter()
            .all(|d| d.is_level_driven_on_stimulus())
    }
}
