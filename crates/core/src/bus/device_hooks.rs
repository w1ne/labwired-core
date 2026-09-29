// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! External-device tick service and GPIO write-hooks (HC-SR04, TM1637, SPI D/C) + clock-gating probe.

use super::*;

impl SystemBus {
    /// Earliest waveform deadline, recomputed against the current grid.
    pub(crate) fn next_resident_edge_deadline_cycle(&self) -> Option<u64> {
        // `min()` over an empty iterator is `None`, so this is the same answer
        // without the interval lookup. Worth the line because the plan path
        // lost its `#[cfg(feature = "event-scheduler")]` guard and now asks
        // this once per instruction in step mode.
        if self.gpio_devices.is_empty() {
            return None;
        }
        let interval = DevicePins::peripheral_tick_interval(self);
        self.gpio_devices
            .iter()
            .filter_map(|device| device.next_edge_deadline_cycle(self.current_cycle, interval))
            .min()
    }

    /// The single due-edge service point, independent of ordinary tick work.
    pub(crate) fn service_resident_scheduled_edges(&mut self) {
        if self.gpio_devices.is_empty() {
            return;
        }
        let now = self.current_cycle;
        let interval = DevicePins::peripheral_tick_interval(self);
        let mut devices = std::mem::take(&mut self.gpio_devices);
        for device in &mut devices {
            device.service_scheduled_edges(self, now, interval);
        }
        self.gpio_devices = devices;
    }

    /// Service every bus-resident GPIO-stimulus device (DHT22 / rotary encoder /
    /// keypad — the [`BusResidentDevice`](crate::bus::BusResidentDevice)s) for
    /// one tick, in registration order, driving each device's input-register
    /// pins for `self.current_cycle`. Each device touches the bus only on a
    /// level transition (via [`drive_idr_bit`](Self::drive_idr_bit)), so an idle
    /// device costs no bus accesses. No-op when none are wired.
    ///
    /// The devices need `&mut self` (the bus) to reach register IO while they
    /// themselves live in `self.gpio_devices`, so the list is `mem::take`-n out
    /// for the pass and put back afterwards — nothing serviced here mutates
    /// `gpio_devices`, so the swap is transparent.
    ///
    /// Replaces the former three separate passes (`service_dht22` /
    /// `service_rotary_encoders` / `service_keypads`); the three device types
    /// drive DISJOINT pins, so merging their passes into one insertion-ordered
    /// pass leaves every register's final value unchanged.
    /// **Tier-2 device pin drive.** Collect every declarative I²C / SPI device's
    /// queued `(role, level)` transitions and put them on their pads.
    ///
    /// Two phases on purpose. Phase one walks the controllers, which each hand
    /// back what their attached devices queued; phase two writes the pads. They
    /// cannot be one loop because both borrow the bus — and keeping them apart
    /// is also what makes the pad write go through the ordinary
    /// [`DevicePins`](crate::bus::DevicePins) methods rather than a second,
    /// wider path into the peripheral table.
    ///
    /// Early-outs on a bus with no such device, which is almost every bus.
    pub(crate) fn service_device_pin_drives(&mut self) {
        if self.device_pin_pads.is_empty() {
            return;
        }
        let mut drives: Vec<(String, String, bool)> = Vec::new();
        for entry in self.peripherals.iter_mut() {
            entry.dev.drain_attached_pin_drives(&mut drives);
        }
        if drives.is_empty() {
            return;
        }
        for (device_id, role, level) in drives {
            let Some(pad) = self
                .device_pin_pads
                .iter()
                .find(|p| p.device_id == device_id && p.role == role)
                .cloned()
            else {
                // A role with no pad is a part wired for an interrupt line the
                // placement did not connect. That is a real board, not a bug:
                // the rule still ran, the line simply goes nowhere.
                continue;
            };
            // ⚠️ BOTH SEAMS — see `rotary_encoder.rs`. `drive_idr_bit` lands
            // only where a store to the input register lands (STM32);
            // `drive_input_bit` is the external-world seam the read-only-IN
            // models (EFR32, SAM, ESP32-C3) actually sample.
            let _ = crate::bus::DevicePins::drive_input_bit(self, pad.addr, pad.bit, level);
            crate::bus::DevicePins::drive_idr_bit(self, pad.addr, pad.bit, level);
        }
    }

    /// Hand every timer-input edge the GPIO port at `gpio_idx` recorded to its
    /// timer, stamped with `current_cycle` — the engine cycle at which the
    /// outside world moved the pad. A scheduled device edge (HC-SR04 ECHO,
    /// a `timing: exact` schedule) is serviced AT its deadline cycle, so the
    /// timer latches CNT as of the edge itself, not as of the next firmware
    /// read.
    ///
    /// The timer is scheduler-synced first (processing any filtered edge due
    /// by now), then its fresh event deadline is harvested: a capture with
    /// CCxIE set pends the NVIC line one cycle later, exactly like an MMIO
    /// write that arms an interrupt.
    pub fn deliver_timer_input_edges(&mut self, gpio_idx: usize) {
        if !self.timer_capture_wired || gpio_idx >= self.peripherals.len() {
            return;
        }
        let edges = self.peripherals[gpio_idx].dev.take_timer_input_edges();
        for edge in edges {
            if edge.timer >= self.peripherals.len() {
                continue;
            }
            // Scheduler sync without a feature fork: `sync_to` is a no-op on a
            // walk-driven timer.
            let now = self.current_cycle;
            if self.peripherals[edge.timer].dev.uses_scheduler() {
                self.peripherals[edge.timer].dev.sync_to(now);
            }
            if self.peripherals[edge.timer]
                .dev
                .timer_input_edge(edge.ti, edge.level, now)
            {
                self.collect_scheduled_events(edge.timer);
            }
        }
    }

    /// Level reconcile at the READ choke, for the sources whose status flag a
    /// READ clears (an STM32 timer channel in input-capture mode: reading
    /// CCRx clears CCxIF). Mirrors the write-choke reconcile: without it the
    /// event chain's last per-cycle re-pend, set while the handler was still
    /// running, survives the read that deasserted the line and re-enters the
    /// handler once. Only a DROP is applied here — a read never pends.
    #[inline]
    pub(crate) fn reconcile_level_after_read(&self, idx: usize) {
        let p = &self.peripherals[idx];
        let Some(irq) = p.irq else {
            return;
        };
        if !p.dev.reads_can_deassert_irq() {
            return;
        }
        if p.dev.irq_line_level() == Some(false) {
            super::reconcile_nvic_level(&self.nvic, irq, false);
        }
    }

    /// Drive an external input level on `pin` of the GPIO peripheral at bus
    /// index `idx` through its `set_gpio_input` seam, then deliver any
    /// timer-input edge that produced. Every caller that holds a peripheral
    /// index instead of an input-register address (browser board I/O, session
    /// bindings, motor feedback) goes through here so a timer on that pad
    /// sees the edge.
    pub fn set_peripheral_gpio_input(&mut self, idx: usize, pin: u8, level: bool) -> bool {
        let Some(p) = self.peripherals.get_mut(idx) else {
            return false;
        };
        let ok = p.dev.set_gpio_input(pin, level);
        self.deliver_timer_input_edges(idx);
        ok
    }

    pub(crate) fn service_gpio_devices(&mut self) {
        if self.gpio_devices.is_empty() {
            return;
        }
        let now = self.current_cycle;
        let mut devices = std::mem::take(&mut self.gpio_devices);
        for device in &mut devices {
            device.service(self, now);
        }
        self.gpio_devices = devices;
    }

    /// Write-hook for a bus-resident device that is clocked by FIRMWARE rather
    /// than by the tick: after an MMIO write to peripheral `idx`, service every
    /// device that named an output-register address this peripheral hosts.
    ///
    /// This is the generic form of the three bespoke hooks that preceded it —
    /// `maybe_clock_hx711`, `maybe_clock_tm1637` and `maybe_sample_seven_segment`,
    /// each one part's private copy of it, with its own typed `Vec` on the bus.
    /// All three are gone; see
    /// [`BusResidentDevice::edge_service_addrs`] for why a tick-only pass loses
    /// edges: the device sees the pad after firmware has already moved it back.
    ///
    /// The pads a device DRIVES still go out through the narrowed
    /// [`DevicePins`](crate::bus::DevicePins) port, exactly as they do on the
    /// tick pass — this changes WHEN `service` runs, not what it may touch.
    /// Cheap half, inlined into the write paths. See the note on
    /// `notify_peripheral_store`: with no GPIO devices attached the body was
    /// already unreachable and only the call remained.
    #[inline]
    pub(crate) fn maybe_service_edge_driven_gpio_devices(&mut self, idx: usize) {
        // A select pad of an analog mux may have moved: re-route before the
        // next instruction can start a conversion.
        self.route_analog_muxes_on_write(idx);
        // A store to a GPIO input register (a device driving a pad through
        // `drive_idr_bit`) is an external edge like `set_gpio_input`.
        if self.timer_capture_wired {
            self.deliver_timer_input_edges(idx);
        }
        if self.gpio_devices.is_empty() {
            return;
        }
        self.service_edge_driven_gpio_devices_cold(idx);
    }

    /// The body. Outlined so a bus with no GPIO devices pays one length check.
    #[inline(never)]
    fn service_edge_driven_gpio_devices_cold(&mut self, idx: usize) {
        // Cheap gate: almost every bus has no edge-driven device at all, and
        // this runs on every MMIO write.
        if !self
            .gpio_devices
            .iter()
            .any(|d| !d.edge_service_addrs().is_empty())
        {
            return;
        }
        let now = self.current_cycle;
        let (base, size) = (self.peripherals[idx].base, self.peripherals[idx].size);
        let mut devices = std::mem::take(&mut self.gpio_devices);
        for device in &mut devices {
            // An address outside the written peripheral's window cannot be
            // hosted by it; only an address inside it needs the routing
            // lookup (a narrower window may still own it). This keeps the
            // lookup off the writes to every other peripheral.
            let hosted = device.edge_service_addrs().iter().any(|a| {
                a.wrapping_sub(base) < size && self.find_peripheral_index(*a) == Some(idx)
            });
            if hosted {
                device.service_edge(self, now);
            }
        }
        self.gpio_devices = devices;
    }

    /// Set or clear a single bit of a GPIO input (IDR) register, writing back
    /// only when the bit actually changes. Shared by every
    /// [`BusResidentDevice`](crate::bus::BusResidentDevice)'s `service` impl.
    pub(crate) fn drive_idr_bit(&mut self, idr_addr: u64, bit: u8, high: bool) {
        let idr = self.read_u32(idr_addr).unwrap_or(0);
        let new_idr = if high {
            idr | (1 << bit)
        } else {
            idr & !(1 << bit)
        };
        if new_idr != idr {
            let _ = self.write_u32(idr_addr, new_idr);
        }
    }

    /// Before an SPI transfer, refresh the D/C level of any attached
    /// display that observes a D/C GPIO line (e.g. the PCD8544 Nokia 5110)
    /// by reading the driving GPIO's output bit. No-op for non-SPI writes and
    /// for SPI peripherals with no D/C-observing device (one cheap downcast).
    pub(crate) fn maybe_latch_dc(&mut self, idx: usize) {
        // Was: `as_any()` then four `downcast_ref` attempts (Spi, Esp32Spi,
        // Esp32c3Spi, Esp32s3Spi) to discover whether this peripheral is an
        // SPI controller at all. This runs from all three MMIO WRITE paths,
        // so every write to every peripheral paid up to four `TypeId`
        // comparisons -- and the overwhelming majority of writes go to
        // something that is not an SPI, so all four failed.
        //
        // `Peripheral::spi_attached_devices` answers the same question in one
        // vtable call, and returns `None` immediately for everything else.
        // Measured on classic ESP32 at 10M steps: `maybe_latch_dc` was 1.63%
        // of the run inside `core::any`, and absent from nrf52840 and
        // esp32c3.

        // Phase 1: collect (attached_index, odr_addr, bit) — immutable borrow.
        let sources: Vec<(usize, u64, u8)> = {
            let Some(devs) = self.peripherals[idx].dev.spi_attached_devices() else {
                return;
            };
            devs.iter()
                .enumerate()
                .filter_map(|(i, d)| d.dc_source().map(|(a, b)| (i, a, b)))
                .collect()
        };
        if sources.is_empty() {
            return;
        }
        // Phase 2: sample the GPIO output bits via the bus.
        let levels: Vec<(usize, bool)> = sources
            .iter()
            .map(|&(i, addr, bit)| {
                let lvl = crate::Bus::read_u32(self, addr)
                    .map(|v| (v >> bit) & 1 != 0)
                    .unwrap_or(false);
                (i, lvl)
            })
            .collect();
        // Phase 3: push the latched levels into the devices — mutable borrow.
        if let Some(devs) = self.peripherals[idx].dev.spi_attached_devices_mut() {
            for (i, lvl) in levels {
                if let Some(d) = devs.get_mut(i) {
                    d.set_dc_level(lvl);
                }
            }
        }
    }

    /// Whether peripheral `idx` is currently clocked — **the one place in the
    /// engine that answers that question.**
    ///
    /// `true` (always-on) for any peripheral without a declared clock-gate — the
    /// safe default that keeps every existing config/firmware working. For a
    /// gated peripheral, reads the *live* controller register map: every bit the
    /// gate requires must be set right now. That is deliberately a read of the
    /// clock-controller model rather than a value latched at build time, so
    /// firmware that turns a clock back off silences the peripheral again
    /// mid-run, the way silicon does.
    ///
    /// A gate may require more than one bit because silicon can withhold a clock
    /// for more than one reason: the bus-enable bit in an `xxxENR` register, and
    /// — for a peripheral fed by its own kernel clock, e.g. the STM32L0 RNG on
    /// HSI48 — the source's ready bit. Both are entries in the same list, so a
    /// peripheral model never needs (and must never grow) a clock check of its
    /// own; see [`crate::bus::ResolvedClockGate`].
    ///
    /// When `gclk_id` is set, the SAM GCLK channel must also be enabled.
    /// If a controller register read fails, the peripheral is treated as clocked
    /// (fail-open: never wedge a chip that has no modelled clock unit).
    pub(crate) fn is_peripheral_clocked(&self, idx: usize) -> bool {
        // missing_clock fault: force the peripheral unclocked and count the
        // suppressed access as the runtime fired-observation. Checked before the
        // bypass so a fault is honoured even under measurement mode.
        //
        // This ran behind an `is_empty()` guard for one commit, on the stated
        // grounds that `HashMap::get` hashes its key even when the map is empty
        // — which it is on every bus outside a fault-injection test. MEASURED,
        // that claim is false: profiling esp32 at 10M steps with and without
        // the guard, on trees differing by nothing else, moved PROGRAM TOTAL by
        // 2,337 Ir out of 3,597,5xx,xxx. The guard bought 0.00006%, which is
        // startup noise, so the lookup was already short-circuiting on an empty
        // table without help.
        //
        // The guard did move 5,030,626 Ir INTO this function in the profile,
        // which is what made it look like a regression. That was inlining
        // re-attributing existing work, not new work: the two program totals
        // agree. A per-function delta is not a cost until the total moves with
        // it.
        if let Some(suppressed) = self.fault_unclocked.get(&idx) {
            suppressed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        if self.clock_gating_bypass {
            return true; // measurement mode: ignore gating (see set_clock_gating_bypass)
        }
        let Some(gate) = self
            .peripherals
            .get(idx)
            .and_then(|p| p.clock_gate.as_ref())
        else {
            return true; // ungated → always accessible
        };
        let bus_clocked = gate.requires.iter().all(|req| {
            if req.controller_idx >= self.peripherals.len() {
                return true; // stale index → don't gate
            }
            match self.peripherals[req.controller_idx]
                .dev
                .read_u32(req.reg_offset)
            {
                Ok(reg) => (reg >> req.bit) & 1 != 0,
                Err(_) => true, // unreadable controller register → fail open
            }
        });
        if !bus_clocked {
            return false;
        }
        let Some(gclk_id) = gate.gclk_id else {
            return true; // PM/RCC bit alone (STM32 unchanged)
        };
        let Some(gclk_idx) = gate.gclk_idx else {
            return false; // gclk_id declared but GCLK was not resolved at build
        };
        if gclk_idx >= self.peripherals.len() {
            return false;
        }
        match self.peripherals[gclk_idx]
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::sam_clock::SamGclk>())
        {
            Some(g) => g.clk_enabled(gclk_id),
            None => false,
        }
    }
}
