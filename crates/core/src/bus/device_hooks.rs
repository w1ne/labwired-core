// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! External-device tick service and GPIO write-hooks (HC-SR04, TM1637, SPI D/C) + clock-gating probe.

use super::*;

impl SystemBus {
    /// Service all HC-SR04 sensors for one tick: compute each sensor's ECHO
    /// level from its (write-hook-armed) echo window and drive it onto the ECHO
    /// input register, touching the bus only on a level transition. TRIG is NOT
    /// polled here — `maybe_arm_hcsr04` arms the window on the GPIO write, which
    /// is cycle-exact (see `Machine::step`). No-op when no sensors are wired.
    pub(crate) fn service_hcsr04(&mut self) {
        if self.hcsr04.is_empty() {
            return;
        }
        for i in 0..self.hcsr04.len() {
            // TRIG is no longer polled here — `maybe_arm_hcsr04` arms the window
            // on the GPIO write (cycle-exact, see the note in `Machine::step`).
            // The per-cycle work is two integer comparisons plus, only on a
            // transition, one read-modify-write of the ECHO input bit.
            self.drive_hcsr04_echo(i);
        }
    }

    /// Drive sensor `i`'s ECHO input register to the level its armed window
    /// implies at `self.current_cycle`, touching the bus only on a transition.
    /// The single choke point shared by the per-cycle [`service_hcsr04`] pass and
    /// the event-scheduler edge handler ([`apply_hcsr04_event`]) — routing both
    /// through the same `write_u32` keeps logic-analyzer probe capture on the
    /// ECHO pad byte-identical across the two paths.
    ///
    /// [`service_hcsr04`]: Self::service_hcsr04
    /// [`apply_hcsr04_event`]: Self::apply_hcsr04_event
    fn drive_hcsr04_echo(&mut self, i: usize) {
        let now = self.current_cycle;
        let echo_high = self.hcsr04[i].echo_high_at(now);
        if echo_high == self.hcsr04[i].last_echo_high() {
            return;
        }
        let echo_addr = self.hcsr04[i].echo_idr_addr;
        let echo_bit = self.hcsr04[i].echo_bit;
        let idr = self.read_u32(echo_addr).unwrap_or(0);
        let new_idr = if echo_high {
            idr | (1 << echo_bit)
        } else {
            idr & !(1 << echo_bit)
        };
        if new_idr != idr {
            let _ = self.write_u32(echo_addr, new_idr);
        }
        self.hcsr04[i].set_last_echo_high(echo_high);
    }

    /// Event-scheduler edge handler: a scheduled ECHO rise/fall for sensor
    /// `sensor` came due, so drive its ECHO input register to the current
    /// window level. Recomputing from the window (rather than trusting a
    /// hard-coded rise/fall) makes the handler idempotent and self-correcting
    /// through the same choke point the per-cycle pass uses. Called by
    /// `Machine::drain_scheduler_events` for [`crate::sched::SUBSYSTEM_PERIPHERAL_IDX`]
    /// events. Out-of-range `sensor` (a sensor removed after scheduling) is a
    /// no-op.
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn apply_hcsr04_event(&mut self, sensor: usize) {
        if sensor < self.hcsr04.len() {
            self.drive_hcsr04_echo(sensor);
        }
    }

    /// Event-scheduler path: the earliest cycle at which any event-scheduled
    /// HC-SR04 must next drive its ECHO pad, or `None` when no sensor has a
    /// pending edge. The run loop clamps its batch to end exactly here so a
    /// busy-polling firmware observes the edge on time. Scoped to HC-SR04 (not
    /// every scheduled peripheral): the SPI wire engine self-corrects against
    /// its anchor when its event fires a batch late, so clamping to it would only
    /// shrink batches during framebuffer pushes for no correctness gain — HC-SR04
    /// is the one device that is polled cycle-tight and must not be observed late.
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn next_hcsr04_deadline_cycle(&self) -> Option<u64> {
        if !self.hcsr04_event_scheduled() {
            return None;
        }
        let interval = (self.config.peripheral_tick_interval as u64).max(1);
        let now = self.current_cycle;
        self.hcsr04
            .iter()
            .filter_map(|s| s.next_edge_deadline_cycle(now, interval))
            .min()
    }

    /// Event-scheduler path: harvest any sensor whose echo window was (re)armed
    /// since the last harvest, returning `(sensor_idx, rise_cycle, fall_cycle)`
    /// absolute cycle deadlines (quantised up to the tick grid — see
    /// `HcSr04::take_edge_schedule`) for `Machine::drain_scheduler_events`
    /// to enqueue. No allocation when nothing was armed.
    #[cfg(feature = "event-scheduler")]
    pub(crate) fn harvest_hcsr04_edges(&mut self, interval: u64, out: &mut Vec<(usize, u64, u64)>) {
        for i in 0..self.hcsr04.len() {
            if let Some((rise, fall)) = self.hcsr04[i].take_edge_schedule(interval) {
                out.push((i, rise, fall));
            }
        }
    }

    /// Write-hook mirror of [`maybe_latch_dc`](Self::maybe_latch_dc) for the
    /// HC-SR04: after an MMIO write to peripheral `idx`, if that peripheral is
    /// the GPIO hosting any sensor's TRIG line, re-read the TRIG ODR bit and run
    /// the sensor's rising-edge/arm logic at `now = self.current_cycle`.
    ///
    /// Because TRIG only changes via a GPIO write, edge detection on the write is
    /// exactly equivalent to the old per-cycle TRIG poll, and `current_cycle`
    /// here equals the value the immediately-following `service_hcsr04` tick sees
    /// (see `Machine::step`), so the arming is cycle-exact.
    pub(crate) fn maybe_arm_hcsr04(&mut self, idx: usize) {
        if self.hcsr04.is_empty() {
            return;
        }
        let now = self.current_cycle;
        for i in 0..self.hcsr04.len() {
            // Resolve & cache the TRIG GPIO's peripheral index on first use.
            let trig_idx = match self.hcsr04[i].trig_peripheral_idx() {
                Some(t) => t,
                None => {
                    let trig_addr = self.hcsr04[i].trig_odr_addr;
                    match self.find_peripheral_index(trig_addr) {
                        Some(t) => {
                            self.hcsr04[i].set_trig_peripheral_idx(t);
                            t
                        }
                        None => continue,
                    }
                }
            };
            if trig_idx != idx {
                continue;
            }
            let trig_addr = self.hcsr04[i].trig_odr_addr;
            let trig_bit = self.hcsr04[i].trig_bit;
            let trig_high = self
                .read_u32(trig_addr)
                .map(|v| (v >> trig_bit) & 1 != 0)
                .unwrap_or(false);
            self.hcsr04[i].observe_trig(trig_high, now);
        }
    }

    /// Write-hook sibling of [`maybe_arm_hcsr04`](Self::maybe_arm_hcsr04) for
    /// bit-banged TM1637 displays: after an MMIO write to peripheral `idx`, if
    /// that peripheral hosts a display's CLK or DIO line, re-read both output
    /// bits and feed the `(clk, dio)` levels to the display's protocol state
    /// machine. Both lines are MCU outputs while writing, so every edge the
    /// firmware bit-bangs arrives as one of these write-hook calls — no polling.
    pub(crate) fn maybe_clock_tm1637(&mut self, idx: usize) {
        if self.tm1637.is_empty() {
            return;
        }
        for i in 0..self.tm1637.len() {
            // Resolve & cache the CLK / DIO GPIO peripheral indices on first use.
            let clk_idx = match self.tm1637[i].clk_peripheral_idx() {
                Some(t) => t,
                None => {
                    let addr = self.tm1637[i].clk_odr_addr;
                    match self.find_peripheral_index(addr) {
                        Some(t) => {
                            self.tm1637[i].set_clk_peripheral_idx(t);
                            t
                        }
                        None => continue,
                    }
                }
            };
            let dio_idx = match self.tm1637[i].dio_peripheral_idx() {
                Some(t) => t,
                None => {
                    let addr = self.tm1637[i].dio_odr_addr;
                    match self.find_peripheral_index(addr) {
                        Some(t) => {
                            self.tm1637[i].set_dio_peripheral_idx(t);
                            t
                        }
                        None => continue,
                    }
                }
            };
            // Only react when this write actually touched the CLK or DIO port.
            if clk_idx != idx && dio_idx != idx {
                continue;
            }
            let clk_addr = self.tm1637[i].clk_odr_addr;
            let clk_bit = self.tm1637[i].clk_bit;
            let dio_addr = self.tm1637[i].dio_odr_addr;
            let dio_bit = self.tm1637[i].dio_bit;
            let clk = self
                .read_u32(clk_addr)
                .map(|v| (v >> clk_bit) & 1 != 0)
                .unwrap_or(true);
            let dio = self
                .read_u32(dio_addr)
                .map(|v| (v >> dio_bit) & 1 != 0)
                .unwrap_or(true);
            self.tm1637[i].observe_lines(clk, dio);
        }
    }

    /// Write-hook sibling of [`maybe_clock_tm1637`](Self::maybe_clock_tm1637)
    /// for direct-drive 7-segment digits: after an MMIO write to peripheral
    /// `idx`, if that peripheral hosts any of the display's nine pins, re-read
    /// all eight segment output bits plus COM and recompute the lit segments.
    ///
    /// Unlike the TM1637 there is no protocol here — the digit is combinational
    /// logic, so the hook simply resamples. COM polarity (low = common cathode,
    /// high = common anode) is resolved inside
    /// [`SevenSegment::observe_levels`](crate::peripherals::components::seven_segment::SevenSegment::observe_levels).
    pub(crate) fn maybe_sample_seven_segment(&mut self, idx: usize) {
        if self.seven_segment.is_empty() {
            return;
        }
        for i in 0..self.seven_segment.len() {
            // Resolve & cache the nine GPIO peripheral indices on first use.
            let mut relevant = false;
            let mut resolved = true;
            for s in 0..crate::peripherals::components::seven_segment::SEGMENTS {
                let seg_idx = match self.seven_segment[i].seg_peripheral_idx(s) {
                    Some(t) => t,
                    None => {
                        let addr = self.seven_segment[i].seg_odr[s].0;
                        match self.find_peripheral_index(addr) {
                            Some(t) => {
                                self.seven_segment[i].set_seg_peripheral_idx(s, t);
                                t
                            }
                            None => {
                                resolved = false;
                                break;
                            }
                        }
                    }
                };
                relevant |= seg_idx == idx;
            }
            if !resolved {
                continue;
            }
            let com_idx = match self.seven_segment[i].com_peripheral_idx() {
                Some(t) => t,
                None => {
                    let addr = self.seven_segment[i].com_odr_addr;
                    match self.find_peripheral_index(addr) {
                        Some(t) => {
                            self.seven_segment[i].set_com_peripheral_idx(t);
                            t
                        }
                        None => continue,
                    }
                }
            };
            relevant |= com_idx == idx;
            // Only react when this write actually touched one of the pins' ports.
            if !relevant {
                continue;
            }
            let read_pin = |bus: &Self, addr: u64, bit: u8| {
                bus.read_u32(addr)
                    .map(|v| (v >> bit) & 1 != 0)
                    .unwrap_or(false)
            };
            let seg_odr = self.seven_segment[i].seg_odr;
            let levels: [bool; crate::peripherals::components::seven_segment::SEGMENTS] =
                std::array::from_fn(|s| read_pin(self, seg_odr[s].0, seg_odr[s].1));
            let com = read_pin(
                self,
                self.seven_segment[i].com_odr_addr,
                self.seven_segment[i].com_bit,
            );
            self.seven_segment[i].observe_levels(levels, com);
        }
    }

    /// Before an SPI transfer, refresh the D/C level of any attached
    /// display that observes a D/C GPIO line (e.g. the PCD8544 Nokia 5110)
    /// by reading the driving GPIO's output bit. No-op for non-SPI writes and
    /// for SPI peripherals with no D/C-observing device (one cheap downcast).
    pub(crate) fn maybe_latch_dc(&mut self, idx: usize) {
        use crate::peripherals::esp32::spi::Esp32Spi;
        use crate::peripherals::esp32c3::spi::Esp32c3Spi;
        use crate::peripherals::spi::{Spi, SpiDevice};

        // Borrow the attached-device list off whichever SPI peripheral kind
        // this is (generic `Spi` for STM32/Nordic, ESP32-family SPI variants).
        fn attached_ref(any: &dyn std::any::Any) -> Option<&Vec<Box<dyn SpiDevice>>> {
            if let Some(s) = any.downcast_ref::<Spi>() {
                return Some(&s.attached_devices);
            }
            if let Some(s) = any.downcast_ref::<Esp32Spi>() {
                return Some(&s.attached_devices);
            }
            if let Some(s) = any.downcast_ref::<Esp32c3Spi>() {
                return Some(&s.attached_devices);
            }
            None
        }
        fn attached_mut(any: &mut dyn std::any::Any) -> Option<&mut Vec<Box<dyn SpiDevice>>> {
            if any.is::<Spi>() {
                return any.downcast_mut::<Spi>().map(|s| &mut s.attached_devices);
            }
            if any.is::<Esp32Spi>() {
                return any
                    .downcast_mut::<Esp32Spi>()
                    .map(|s| &mut s.attached_devices);
            }
            if any.is::<Esp32c3Spi>() {
                return any
                    .downcast_mut::<Esp32c3Spi>()
                    .map(|s| &mut s.attached_devices);
            }
            None
        }

        // Phase 1: collect (attached_index, odr_addr, bit) — immutable borrow.
        let sources: Vec<(usize, u64, u8)> = {
            let Some(any) = self.peripherals[idx].dev.as_any() else {
                return;
            };
            let Some(devs) = attached_ref(any) else {
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
        if let Some(any) = self.peripherals[idx].dev.as_any_mut() {
            if let Some(devs) = attached_mut(any) {
                for (i, lvl) in levels {
                    if let Some(d) = devs.get_mut(i) {
                        d.set_dc_level(lvl);
                    }
                }
            }
        }
    }

    /// Whether peripheral `idx` is currently clocked. `true` (always-on) for any
    /// peripheral without a declared clock-gate — the safe default that keeps
    /// every existing config/firmware working. For a gated peripheral, reads the
    /// RCC enable register the gate points at and returns whether the gate bit is
    /// set. If no RCC peripheral is registered, or its register read fails, the
    /// peripheral is treated as clocked (fail-open: never wedge a chip that has
    /// no modelled RCC). Cheap: one `Option` check, then on the rare gated path a
    /// single cached-index RCC register read.
    pub(crate) fn is_peripheral_clocked(&self, idx: usize) -> bool {
        // missing_clock fault: force the peripheral unclocked and count the
        // suppressed access as the runtime fired-observation. Checked before the
        // bypass so a fault is honoured even under measurement mode.
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
        let Some(rcc_idx) = self.rcc_idx else {
            return true; // no RCC modelled → don't gate
        };
        match self.peripherals[rcc_idx].dev.read_u32(gate.reg_offset) {
            Ok(reg) => (reg >> gate.bit) & 1 != 0,
            Err(_) => true,
        }
    }
}
