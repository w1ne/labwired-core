// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Nuvoton NPCX Miniature Timer Counter (MTC), the block Renode maps as
//! `Timers.NPCX_MTC` at 0x400B7000 on NVIC 5 of the NPCX9.
//!
//! Sources: Renode `NPCX_MTC.cs` (register map, 1 Hz tick, 25-bit predefined
//! time, IRQ = WIE && PTO) and Renode `nuvoton_npcx9.repl` (base, NVIC 5).
//!
//! Modelled:
//! - `TTC` (+0x00): a 32-bit read/write count that increments once per tick
//!   (`tick_hz`, default 1 Hz as Renode's `LimitTimer` of frequency 1, limit
//!   1). A write sets the count; it does not move the tick phase, which runs
//!   from reset (Renode's timer is not restarted on a `TTC` write);
//! - `WTC` (+0x04): `PT` bits 0..24, `PTO` bit 30 (write 1 to clear), `WIE`
//!   bit 31. `PTO` sets on the increment that brings `TTC[24:0]` to `PT`;
//! - the IRQ as a LEVEL: `WIE && PTO`. A `WTC` write over a set `PTO` with
//!   `WIE` raises it at once, as Renode's write callback does;
//! - `TTC` survives a system reset (Renode `softResettable: false`); `WTC`
//!   clears.
//!
//! The count is derived from the bus cycle clock; with the event scheduler
//! the next `PTO` is a scheduled event, so a core idling in WFI fast-forwards
//! to it. A bus with no cycle clock uses the cycles the legacy walk hands in.
//!
//! NOT modelled: wake-up from deep sleep (`WIE` only gates the line) and the
//! LFCG clock tree (the rate is fixed).

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

const TTC: u64 = 0x00;
const WTC: u64 = 0x04;

pub const WTC_WIE: u32 = 1 << 31;
pub const WTC_PTO: u32 = 1 << 30;
pub const PT_MASK: u32 = (1 << 25) - 1;

/// Renode `NPCX_MTC`: `LimitTimer` frequency 1, limit 1.
pub const DEFAULT_TICK_HZ: u64 = 1;

/// Event token for "raise the IRQ now" (`WIE` set over a set `PTO`).
const RAISE_TOKEN: u32 = u32::MAX;

/// NPCX MTC (see the module docs).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NpcxMtc {
    tick_hz: u64,
    cpu_hz: u64,
    /// Cycle of tick 0 (attach / construction).
    epoch: u64,
    /// `TTC` = `ttc_offset + ticks` (wrapping).
    ttc_offset: u64,
    wie: bool,
    pt: u32,
    /// `PTO` latched at the last anchor.
    pto: bool,
    /// Ticks since `epoch` at the last anchor.
    anchor_tick: u64,
    /// The time base when no cycle clock is attached (hand-built buses).
    walk_cycles: u64,
    /// Bumped on every re-anchor; a stale scheduled event is dropped.
    generation: u32,
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl NpcxMtc {
    pub fn new(tick_hz: u64) -> Self {
        Self {
            tick_hz: tick_hz.max(1),
            cpu_hz: 1_000_000,
            epoch: 0,
            ttc_offset: 0,
            wie: false,
            pt: 0,
            pto: false,
            anchor_tick: 0,
            walk_cycles: 0,
            generation: 0,
            pending_events: Vec::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now(&self) -> u64 {
        self.clock
            .as_ref()
            .map_or(self.walk_cycles, CycleClock::now)
    }

    /// Ticks since `epoch`.
    fn ticks(&self) -> u64 {
        let elapsed = u128::from(self.now().saturating_sub(self.epoch));
        (elapsed * u128::from(self.tick_hz) / u128::from(self.cpu_hz.max(1))) as u64
    }

    fn ttc(&self) -> u32 {
        self.ttc_offset.wrapping_add(self.ticks()) as u32
    }

    /// The first tick after `after` whose increment brings `TTC[24:0]` to `PT`.
    fn next_match(&self, after: u64) -> u64 {
        let first = self.ttc_offset.wrapping_add(after).wrapping_add(1);
        let d = u64::from(self.pt).wrapping_sub(first) & u64::from(PT_MASK);
        after + 1 + d
    }

    fn pto_now(&self) -> bool {
        self.pto || self.next_match(self.anchor_tick) <= self.ticks()
    }

    fn line(&self) -> bool {
        self.wie && self.pto_now()
    }

    fn wtc(&self) -> u32 {
        (if self.wie { WTC_WIE } else { 0 }) | (if self.pto_now() { WTC_PTO } else { 0 }) | self.pt
    }

    fn anchor(&mut self) {
        self.pto = self.pto_now();
        self.anchor_tick = self.ticks();
    }

    /// After a write: schedule the next `PTO`, or an immediate raise when the
    /// write itself asserted the line.
    fn rearm(&mut self, was_line: bool) {
        self.generation = self.generation.wrapping_add(1);
        if !self.scheduler_mode() {
            return;
        }
        if self.line() {
            if !was_line {
                self.pending_events.push((1, RAISE_TOKEN));
            }
            return;
        }
        if self.wie {
            let tick = u128::from(self.next_match(self.anchor_tick));
            let at = (tick * u128::from(self.cpu_hz.max(1))).div_ceil(u128::from(self.tick_hz));
            let at = u128::from(self.epoch) + at;
            let delay = at
                .saturating_sub(u128::from(self.now()))
                .min(u128::from(u64::MAX)) as u64;
            self.pending_events.push((delay.max(1), self.generation));
        }
    }

    fn write_ttc(&mut self, value: u32) {
        let was_line = self.line();
        self.anchor();
        self.ttc_offset = u64::from(value).wrapping_sub(self.anchor_tick);
        self.rearm(was_line);
    }

    fn write_wtc(&mut self, value: u32) {
        let was_line = self.line();
        self.anchor();
        self.wie = value & WTC_WIE != 0;
        if value & WTC_PTO != 0 {
            self.pto = false;
        }
        self.pt = value & PT_MASK;
        self.rearm(was_line);
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            TTC => self.ttc(),
            WTC => self.wtc(),
            _ => {
                crate::census_reg!("npcx_mtc", offset, "read");
                0
            }
        }
    }
}

impl Peripheral for NpcxMtc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_reg(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        let merge = |word: u32| (word & !(0xFF << shift)) | (u32::from(value) << shift);
        match offset & !3 {
            TTC => self.write_ttc(merge(self.ttc())),
            // Build on the stored fields with PTO clear, so a byte store
            // clears PTO only when it writes bit 30 itself.
            WTC => self.write_wtc(merge(self.wtc() & !WTC_PTO)),
            _ => crate::census_reg!("npcx_mtc", offset, "write"),
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            TTC => self.write_ttc(value),
            WTC => self.write_wtc(value),
            _ => crate::census_reg!("npcx_mtc", offset & !3, "write"),
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && self.wie
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    /// `WIE && PTO` is a level: a handler that clears `PTO` must not be
    /// re-entered for the same match.
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.line())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        if self.clock.is_none() {
            self.walk_cycles = self.walk_cycles.wrapping_add(cycles);
        }
        PeripheralTickResult {
            irq: self.line(),
            ..Default::default()
        }
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        std::mem::take(&mut self.pending_events)
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let live = event_token == RAISE_TOKEN || event_token == self.generation;
        crate::sched::EventResult {
            raise_own_irq: live && self.line(),
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.epoch = clock.now();
        self.anchor_tick = 0;
        self.clock = Some(clock);
    }

    fn attach_cpu_hz(&mut self, hz: u64) {
        self.cpu_hz = hz.max(1);
    }

    /// `TTC` keeps counting (Renode `softResettable: false`); `WTC` clears.
    fn on_system_reset(&mut self) {
        self.anchor();
        self.wie = false;
        self.pt = 0;
        self.pto = false;
        self.generation = self.generation.wrapping_add(1);
        self.pending_events.clear();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 15 MHz core, 1 kHz tick: 15,000 cycles per tick.
    fn mtc() -> NpcxMtc {
        let mut t = NpcxMtc::new(1_000);
        t.attach_cpu_hz(15_000_000);
        t
    }

    #[test]
    fn renode_default_is_one_hertz() {
        let mut t = NpcxMtc::new(DEFAULT_TICK_HZ);
        t.attach_cpu_hz(15_000_000);
        t.tick_elapsed(14_999_999);
        assert_eq!(t.read_u32(TTC).unwrap(), 0);
        t.tick_elapsed(1);
        assert_eq!(t.read_u32(TTC).unwrap(), 1);
    }

    #[test]
    fn ttc_counts_and_a_write_keeps_the_phase() {
        let mut t = mtc();
        t.tick_elapsed(15_000 * 3 + 7_000);
        assert_eq!(t.read_u32(TTC).unwrap(), 3);
        t.write_u32(TTC, 0x1000).unwrap();
        assert_eq!(t.read_u32(TTC).unwrap(), 0x1000);
        // The next increment is 8,000 cycles away, not a full tick.
        t.tick_elapsed(7_999);
        assert_eq!(t.read_u32(TTC).unwrap(), 0x1000);
        t.tick_elapsed(1);
        assert_eq!(t.read_u32(TTC).unwrap(), 0x1001);
        t.write_u32(TTC, 0xFFFF_FFFF).unwrap();
        t.tick_elapsed(15_000);
        assert_eq!(t.read_u32(TTC).unwrap(), 0, "wraps");
    }

    #[test]
    fn pto_sets_on_the_matching_increment_and_drives_the_level() {
        let mut t = mtc();
        t.write_u32(TTC, 10).unwrap();
        t.write_u32(WTC, WTC_WIE | 12).unwrap();
        assert_eq!(t.read_u32(WTC).unwrap(), WTC_WIE | 12);
        assert!(!t.tick_elapsed(15_000).irq, "TTC 11");
        assert!(t.tick_elapsed(15_000).irq, "TTC 12");
        assert_eq!(t.read_u32(WTC).unwrap(), WTC_WIE | WTC_PTO | 12);
        // PTO stays set after TTC moves on.
        t.tick_elapsed(15_000);
        assert_eq!(t.irq_line_level(), Some(true));
        // W1C, with a new PT.
        t.write_u32(WTC, WTC_WIE | WTC_PTO | 14).unwrap();
        assert_eq!(t.irq_line_level(), Some(false));
        assert!(t.tick_elapsed(15_000).irq, "TTC 14");
    }

    #[test]
    fn a_write_to_pt_equal_to_ttc_waits_for_the_next_increment() {
        let mut t = mtc();
        t.write_u32(TTC, 5).unwrap();
        t.write_u32(WTC, WTC_WIE | 5).unwrap();
        assert_eq!(t.irq_line_level(), Some(false));
        // TTC must wrap the 25-bit field to come back to 5.
        t.tick_elapsed(15_000 * 100);
        assert_eq!(t.irq_line_level(), Some(false));
    }

    #[test]
    fn pt_compares_the_low_25_bits() {
        let mut t = mtc();
        t.write_u32(TTC, 0x0200_0000 | 6).unwrap();
        t.write_u32(WTC, 7).unwrap();
        t.tick_elapsed(15_000);
        assert_eq!(t.read_u32(WTC).unwrap(), WTC_PTO | 7);
        assert_eq!(t.irq_line_level(), Some(false), "WIE clear");
        // Setting WIE over a set PTO raises.
        t.write_u32(WTC, WTC_WIE | 7).unwrap();
        assert_eq!(t.irq_line_level(), Some(true));
    }

    #[test]
    fn byte_stores_do_not_clear_pto_by_accident() {
        let mut t = mtc();
        t.write_u32(WTC, 1).unwrap();
        t.tick_elapsed(15_000);
        assert_ne!(t.read_u32(WTC).unwrap() & WTC_PTO, 0);
        t.write(WTC, 2).unwrap();
        assert_eq!(t.read_u32(WTC).unwrap(), WTC_PTO | 2);
        t.write(WTC + 3, 0x80).unwrap();
        assert_eq!(t.read_u32(WTC).unwrap(), WTC_WIE | WTC_PTO | 2);
        t.write(WTC + 3, 0xC0).unwrap();
        assert_eq!(t.read_u32(WTC).unwrap(), WTC_WIE | 2);
    }

    #[test]
    fn system_reset_keeps_ttc_and_clears_wtc() {
        let mut t = mtc();
        t.write_u32(TTC, 100).unwrap();
        t.write_u32(WTC, WTC_WIE | 0x1234).unwrap();
        t.tick_elapsed(15_000 * 2);
        t.on_system_reset();
        assert_eq!(t.read_u32(TTC).unwrap(), 102);
        assert_eq!(t.read_u32(WTC).unwrap(), 0);
    }

    #[test]
    fn counts_from_the_cycle_clock_and_schedules_the_match() {
        let clock = CycleClock::default();
        clock.publish(1_000);
        let mut t = mtc();
        t.attach_cycle_clock(clock.clone());
        clock.publish(1_000 + 15_000 * 4 + 500);
        assert_eq!(t.read_u32(TTC).unwrap(), 4);
        t.write_u32(WTC, WTC_WIE | 6).unwrap();
        let events = t.take_scheduled_events();
        if t.scheduler_mode() {
            // Tick 6 starts at 1,000 + 90,000.
            assert_eq!(events, vec![(15_000 * 2 - 500, t.generation)]);
        } else {
            assert!(events.is_empty());
        }
        clock.publish(1_000 + 15_000 * 6 - 1);
        assert_eq!(t.irq_line_level(), Some(false));
        clock.publish(1_000 + 15_000 * 6);
        assert_eq!(t.irq_line_level(), Some(true));
        let mut sched = crate::sched::EventScheduler::new();
        let mut bus = crate::bus::SystemBus::new();
        let gen = t.generation;
        assert!(t.on_event(gen, &mut sched, &mut bus).raise_own_irq);
        t.write_u32(WTC, WTC_WIE | WTC_PTO | 7).unwrap();
        assert_eq!(t.irq_line_level(), Some(false));
        assert!(
            !t.on_event(gen, &mut sched, &mut bus).raise_own_irq,
            "superseded"
        );
        clock.publish(1_000 + 15_000 * 7);
        assert_eq!(t.irq_line_level(), Some(true));
    }
}
