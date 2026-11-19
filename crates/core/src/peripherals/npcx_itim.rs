// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Nuvoton NPCX internal timer (ITIM32 / ITIM64), the block Renode maps as
//! `Timers.NPCX_ITIM` at 0x400B0000 + n * 0x2000 (ITIM32_1..6) and
//! 0x400BE000 (ITIM64) on the NPCX9.
//!
//! Sources: Zephyr `soc/nuvoton/npcx/common/reg/reg_def.h` (`struct
//! itim32_reg` / `struct itim64_reg`: `ITPRE` byte at +0x01, `ITCTS` byte at
//! +0x04 with `TO_STS` bit 0, `TO_IE` 2, `TO_WUE` 3, `CKSEL` 4, `ITEN` 7,
//! `ITCNT32` / `ITCNT64L` at +0x08, `ITCNT64H` at +0x0C), Zephyr
//! `dts/arm/nuvoton/npcx/npcx9.dtsi` (ITIM32_1 at 0x400B0000 on NVIC 28,
//! ITIM64 at 0x400BE000; APB2 at 15 MHz) and Renode `NPCX_ITIM.cs`.
//!
//! Modelled:
//! - a down-counter loaded from `ITCNT`: written while `ITEN` is clear it
//!   holds the start value N and reads back as N. Once enabled it counts
//!   N, N-1, .., 0, then reloads N and sets `TO_STS` (period N + 1 ticks,
//!   as Renode's limit of N + 1). A write while enabled reloads the counter;
//! - the tick clock: APB2 (default the attached CPU clock; `apb2_hz`
//!   overrides) or, with `CKSEL` set, LFCLK (`lfclk_hz`, default 32768 Hz),
//!   divided by `ITPRE + 1`;
//! - `TO_STS` write-1-to-clear; clearing `ITEN` clears `TO_STS` and returns
//!   the counter to N (Renode does the same);
//! - the IRQ as a LEVEL: `TO_STS & (TO_IE | TO_WUE)`;
//! - ITIM64 (`wide`): `ITCNT64H:ITCNT64L` form one 64-bit counter.
//!
//! The count is derived from the bus cycle clock and re-anchored on every
//! register write; with the event scheduler the next timeout is a scheduled
//! event. A bus with no cycle clock uses the cycles the legacy walk hands in.
//!
//! NOT modelled: wake-up from deep sleep (`TO_WUE` only gates the line), the
//! `ITEN` clock-domain sync delay (it reads back set at once), the PWDWN_CTL
//! clock gates, and the CDCG/LFCG clock trees (the rates are fixed).

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

const ITPRE: u64 = 0x01;
const ITCTS: u64 = 0x04;
const ITCNT: u64 = 0x08;
const ITCNT64H: u64 = 0x0C;

pub const TO_STS: u8 = 1 << 0;
pub const TO_IE: u8 = 1 << 2;
pub const TO_WUE: u8 = 1 << 3;
pub const CKSEL: u8 = 1 << 4;
pub const ITEN: u8 = 1 << 7;
const CTS_RW: u8 = TO_IE | TO_WUE | CKSEL | ITEN;

/// LFCLK, as Renode's `DefaultLFCLKFrequency`.
pub const DEFAULT_LFCLK_HZ: u64 = 32_768;

/// Event token for "raise the IRQ now" (an enable over a set `TO_STS`).
const RAISE_TOKEN: u32 = u32::MAX;

/// NPCX ITIM32 / ITIM64 (see the module docs).
///
/// The count is derived from the bus cycle clock, not stepped: the hosted
/// fast path widens the bus tick to 1024 cycles while no model walks, so a
/// walk-stepped counter that starts later would be credited a full window
/// per bus tick. Every register write re-anchors the count at the current
/// cycle; with the event scheduler the next timeout is a scheduled event.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NpcxItim {
    wide: bool,
    pre: u8,
    /// `ITCTS`; `TO_STS` here is the flag latched at the last anchor.
    cts: u8,
    /// The load value N (`ITCNT`, or `ITCNT64H:ITCNT64L`).
    load: u64,
    /// The count at `anchor_cycle`.
    anchor_cnt: u64,
    anchor_cycle: u64,
    /// The time base when no cycle clock is attached (hand-built buses):
    /// the cycles the legacy walk has handed to `tick_elapsed`.
    walk_cycles: u64,
    cpu_hz: u64,
    apb2_hz: Option<u64>,
    lfclk_hz: u64,
    /// Bumped on every re-anchor; a scheduled event whose token no longer
    /// matches was superseded and is dropped.
    generation: u32,
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl NpcxItim {
    pub fn new(wide: bool, apb2_hz: Option<u64>, lfclk_hz: u64) -> Self {
        Self {
            wide,
            pre: 0,
            cts: 0,
            load: 0,
            anchor_cnt: 0,
            anchor_cycle: 0,
            walk_cycles: 0,
            cpu_hz: 1_000_000,
            apb2_hz: apb2_hz.map(|hz| hz.max(1)),
            lfclk_hz: lfclk_hz.max(1),
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

    fn mask(&self) -> u64 {
        if self.wide {
            u64::MAX
        } else {
            u64::from(u32::MAX)
        }
    }

    fn running(&self) -> bool {
        self.cts & ITEN != 0
    }

    fn source_hz(&self) -> u64 {
        if self.cts & CKSEL != 0 {
            self.lfclk_hz
        } else {
            self.apb2_hz.unwrap_or(self.cpu_hz)
        }
    }

    /// CPU cycles per timer tick, as a fraction `num / den`.
    fn cycles_per_tick(&self) -> (u128, u128) {
        (
            (u128::from(self.pre) + 1) * u128::from(self.cpu_hz.max(1)),
            u128::from(self.source_hz()),
        )
    }

    /// Timer ticks since the anchor.
    fn ticks(&self) -> u128 {
        if !self.running() {
            return 0;
        }
        let (num, den) = self.cycles_per_tick();
        u128::from(self.now().saturating_sub(self.anchor_cycle)) * den / num
    }

    /// The counter now: N, N-1, .., 0, then N again with a timeout.
    fn count(&self) -> u64 {
        let t = self.ticks();
        let anchor = u128::from(self.anchor_cnt);
        if t <= anchor {
            return (anchor - t) as u64;
        }
        let period = u128::from(self.load) + 1;
        (u128::from(self.load) - (t - anchor - 1) % period) as u64
    }

    fn timed_out(&self) -> bool {
        self.ticks() > u128::from(self.anchor_cnt)
    }

    fn cts_now(&self) -> u8 {
        self.cts | if self.timed_out() { TO_STS } else { 0 }
    }

    fn line(&self) -> bool {
        let cts = self.cts_now();
        cts & TO_STS != 0 && cts & (TO_IE | TO_WUE) != 0
    }

    /// Latch the count and `TO_STS` at the current cycle.
    fn anchor(&mut self) {
        let cnt = self.count();
        self.cts = self.cts_now();
        self.anchor_cnt = cnt;
        self.anchor_cycle = self.now();
    }

    /// After a write: schedule the next timeout, or an immediate raise when
    /// the write itself asserted the line.
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
        if self.running() && self.cts & (TO_IE | TO_WUE) != 0 {
            let (num, den) = self.cycles_per_tick();
            let ticks = u128::from(self.anchor_cnt) + 1;
            let delay = (ticks * num).div_ceil(den).min(u128::from(u64::MAX)) as u64;
            self.pending_events.push((delay.max(1), self.generation));
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => u32::from(self.pre) << 8,
            ITCTS => u32::from(self.cts_now()),
            ITCNT => self.count() as u32,
            ITCNT64H if self.wide => (self.count() >> 32) as u32,
            _ => {
                crate::census_reg!("npcx_itim:NpcxItim", offset, "read");
                0
            }
        }
    }

    fn write_pre(&mut self, value: u8) {
        let was_line = self.line();
        self.anchor();
        self.pre = value;
        self.rearm(was_line);
    }

    fn write_cts(&mut self, value: u8) {
        let was_line = self.line();
        self.anchor();
        let was_running = self.running();
        let sts = self.cts & TO_STS & !(value & TO_STS);
        self.cts = sts | (value & CTS_RW);
        if was_running && !self.running() {
            self.cts &= !TO_STS;
            self.anchor_cnt = self.load;
        }
        self.rearm(was_line);
    }

    fn write_load(&mut self, value: u64) {
        let was_line = self.line();
        self.anchor();
        self.load = value & self.mask();
        self.anchor_cnt = self.load;
        self.rearm(was_line);
    }
}

impl Peripheral for NpcxItim {
    fn read(&self, offset: u64) -> SimResult<u8> {
        if offset == ITPRE {
            return Ok(self.pre);
        }
        let aligned = offset & !3;
        Ok((self.read_reg(aligned) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            ITPRE => self.write_pre(value),
            ITCTS => self.write_cts(value),
            0x08..=0x0F if offset < 0x0C || self.wide => {
                // Byte stores build on the load value, not the running count.
                let shift = (offset & 7) * 8;
                let word = (self.load & !(0xFF << shift)) | (u64::from(value) << shift);
                self.write_load(word);
            }
            0x00 | 0x02 | 0x03 | 0x05..=0x07 => {}
            _ => crate::census_reg!("npcx_itim:NpcxItim", offset, "write"),
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            0x00 => self.write_pre((value >> 8) as u8),
            ITCTS => self.write_cts(value as u8),
            ITCNT => self.write_load((self.load & !0xFFFF_FFFF) | u64::from(value)),
            ITCNT64H if self.wide => {
                self.write_load((self.load & 0xFFFF_FFFF) | (u64::from(value) << 32))
            }
            _ => crate::census_reg!("npcx_itim:NpcxItim", offset & !3, "write"),
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
        !self.scheduler_mode() && (self.running() || self.line())
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    /// `TO_STS & (TO_IE | TO_WUE)` is a level: a handler that clears
    /// `TO_STS` must not be re-entered for the same timeout.
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
        self.anchor_cycle = clock.now();
        self.clock = Some(clock);
    }

    fn attach_cpu_hz(&mut self, hz: u64) {
        self.cpu_hz = hz.max(1);
    }

    fn on_system_reset(&mut self) {
        let clock = self.clock.take();
        let cpu_hz = self.cpu_hz;
        *self = Self::new(self.wide, self.apb2_hz, self.lfclk_hz);
        self.cpu_hz = cpu_hz;
        if let Some(clock) = clock {
            self.attach_cycle_clock(clock);
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn itim32() -> NpcxItim {
        let mut t = NpcxItim::new(false, None, DEFAULT_LFCLK_HZ);
        t.attach_cpu_hz(15_000_000);
        t
    }

    #[test]
    fn zephyr_byte_layout() {
        let mut t = itim32();
        t.write(ITPRE, 0x0E).unwrap();
        assert_eq!(t.read(ITPRE).unwrap(), 0x0E);
        assert_eq!(t.read_u32(0).unwrap(), 0x0E00);
        t.write(ITCTS, TO_IE | 0x60).unwrap();
        assert_eq!(t.read(ITCTS).unwrap(), TO_IE, "bits 5..6 reserved");
        t.write_u32(ITCNT, 0x1234_5678).unwrap();
        assert_eq!(t.read_u32(ITCNT).unwrap(), 0x1234_5678);
        assert_eq!(t.read_u32(ITCNT64H).unwrap(), 0, "no high word on ITIM32");
    }

    #[test]
    fn counts_down_on_apb2_divided_by_itpre_plus_one() {
        let mut t = itim32();
        t.write(ITPRE, 14).unwrap(); // 15 MHz / 15 = 1 MHz = 15 cycles per tick
        t.write_u32(ITCNT, 1000).unwrap();
        t.write(ITCTS, ITEN).unwrap();
        t.tick_elapsed(14);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 1000);
        t.tick_elapsed(1);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 999);
        t.tick_elapsed(150);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 989);
    }

    #[test]
    fn timeout_reloads_and_latches_the_level_line() {
        let mut t = itim32();
        t.write_u32(ITCNT, 9).unwrap();
        t.write(ITCTS, ITEN).unwrap();
        // Nine ticks take 9 to 0; the tenth reloads (period N + 1).
        assert!(!t.tick_elapsed(9).irq);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 0);
        assert_eq!(t.read(ITCTS).unwrap() & TO_STS, 0);
        assert!(!t.tick_elapsed(1).irq, "TO_STS without TO_IE");
        assert_eq!(t.read_u32(ITCNT).unwrap(), 9);
        assert_eq!(t.read(ITCTS).unwrap(), ITEN | TO_STS);
        t.write(ITCTS, ITEN | TO_IE).unwrap();
        assert_eq!(t.irq_line_level(), Some(true));
        // W1C, as Zephyr's ISR does.
        t.write(ITCTS, ITEN | TO_IE | TO_STS).unwrap();
        assert_eq!(t.irq_line_level(), Some(false));
        // Many periods at once: 25 ticks from 9 reload at 10 and 20; ends at 4.
        t.tick_elapsed(25);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 4);
        assert!(t.tick_elapsed(0).irq);
    }

    #[test]
    fn wake_up_enable_also_drives_the_line() {
        let mut t = itim32();
        t.write(ITCTS, ITEN | TO_WUE).unwrap();
        t.tick_elapsed(1);
        assert_eq!(t.irq_line_level(), Some(true));
    }

    #[test]
    fn disable_clears_status_and_restores_the_load() {
        let mut t = itim32();
        t.write_u32(ITCNT, 3).unwrap();
        t.write(ITCTS, ITEN | TO_IE).unwrap();
        t.tick_elapsed(6);
        assert_eq!(t.read(ITCTS).unwrap(), ITEN | TO_IE | TO_STS);
        t.write(ITCTS, TO_IE).unwrap();
        assert_eq!(t.read(ITCTS).unwrap(), TO_IE);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 3);
        t.tick_elapsed(100);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 3, "stopped");
    }

    #[test]
    fn cksel_selects_lfclk() {
        let mut t = itim32();
        t.write_u32(ITCNT, 100).unwrap();
        t.write(ITCTS, ITEN | CKSEL).unwrap();
        // 15 MHz / 32768 Hz = 457.76 cycles per tick.
        t.tick_elapsed(457);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 100);
        t.tick_elapsed(1);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 99);
    }

    #[test]
    fn counts_from_the_cycle_clock_and_schedules_the_timeout() {
        let clock = CycleClock::default();
        clock.publish(1_000);
        let mut t = itim32();
        t.attach_cycle_clock(clock.clone());
        t.write(ITPRE, 14).unwrap();
        t.write_u32(ITCNT, 99).unwrap();
        t.take_scheduled_events();
        t.write(ITCTS, ITEN | TO_IE).unwrap();
        let events = t.take_scheduled_events();
        if t.scheduler_mode() {
            // 100 ticks of 15 cycles.
            assert_eq!(events, vec![(1500, t.generation)]);
        } else {
            assert!(events.is_empty());
        }
        clock.publish(1_000 + 15 * 40);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 59);
        clock.publish(1_000 + 1499);
        assert_eq!(t.irq_line_level(), Some(false));
        clock.publish(1_000 + 1500);
        assert_eq!(t.irq_line_level(), Some(true));
        assert_eq!(t.read_u32(ITCNT).unwrap(), 99);
        let mut sched = crate::sched::EventScheduler::new();
        let mut bus = crate::bus::SystemBus::new();
        let gen = t.generation;
        assert!(t.on_event(gen, &mut sched, &mut bus).raise_own_irq);
        // W1C re-anchors: the next timeout is a full period away.
        t.write(ITCTS, ITEN | TO_IE | TO_STS).unwrap();
        assert_eq!(t.irq_line_level(), Some(false));
        assert!(
            !t.on_event(gen, &mut sched, &mut bus).raise_own_irq,
            "superseded"
        );
        clock.publish(1_000 + 1500 + 1499);
        assert_eq!(t.irq_line_level(), Some(false));
        clock.publish(1_000 + 1500 + 1500);
        assert_eq!(t.irq_line_level(), Some(true));
    }

    #[test]
    fn enabling_the_interrupt_over_a_set_flag_raises() {
        let clock = CycleClock::default();
        let mut t = itim32();
        t.attach_cycle_clock(clock.clone());
        t.write(ITCTS, ITEN).unwrap();
        clock.publish(1);
        assert_eq!(t.read(ITCTS).unwrap(), ITEN | TO_STS);
        t.take_scheduled_events();
        t.write(ITCTS, ITEN | TO_IE).unwrap();
        assert_eq!(t.irq_line_level(), Some(true));
        if t.scheduler_mode() {
            assert_eq!(t.take_scheduled_events(), vec![(1, RAISE_TOKEN)]);
        }
    }

    #[test]
    fn itim64_counts_across_the_word_boundary() {
        let mut t = NpcxItim::new(true, Some(15_000_000), DEFAULT_LFCLK_HZ);
        t.attach_cpu_hz(15_000_000);
        t.write_u32(ITCNT, 0).unwrap();
        t.write_u32(ITCNT64H, 1).unwrap();
        assert_eq!(t.read_u32(ITCNT64H).unwrap(), 1);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 0);
        t.write(ITCTS, ITEN).unwrap();
        t.tick_elapsed(1);
        assert_eq!(t.read_u32(ITCNT64H).unwrap(), 0);
        assert_eq!(t.read_u32(ITCNT).unwrap(), 0xFFFF_FFFF);
    }
}
