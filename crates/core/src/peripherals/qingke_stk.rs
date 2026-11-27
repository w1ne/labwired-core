// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WCH QingKe V2 system timer (`STK`, "SysTick") at 0xE000F000, as on the
//! CH32V003/V005/V006.
//!
//! Sources: WCH CH32V003 Reference Manual V1.x, chapter "SysTick" (STK_CTLR,
//! STK_SR, STK_CNTR, STK_CMPR), and the WCH noneos-sdk `core_riscv.h`
//! `SysTick_Type` (CTLR +0x00, SR +0x04, CNT +0x08, reserved +0x0C, CMP +0x10)
//! with `debug.c` `Delay_Us`/`Delay_Ms`, which poll `SR.CNTIF`.
//!
//! Modelled:
//! - `CTLR` (+0x00): `STE` bit 0 starts the 32-bit up-counter, `STIE` bit 1
//!   (stored), `STCLK` bit 2 selects HCLK (1) or HCLK/8 (0), `STRE` bit 3
//!   auto-reload: the count restarts from 0 after it reaches `CMP`;
//!   `SWIE` bit 31 (stored);
//! - `SR` (+0x04): `CNTIF` bit 0 sets when the count reaches `CMP`; it stays
//!   set until software writes 0 to it;
//! - `CNT` (+0x08) and `CMP` (+0x10): 32-bit read/write.
//!
//! The count is derived from the bus cycle clock (one CPU cycle = one HCLK).
//! A bus with no cycle clock uses the cycles the legacy walk hands in. Every
//! register write re-anchors the count, so the HCLK/8 prescaler phase
//! restarts at that write.
//!
//! Interrupt: with `STIE` set, each compare match (and enabling `STIE` while
//! `CNTIF` is already set) pends the descriptor's `irq` (SysTick_IRQn = 12) in
//! the PFIC once. The interrupt is a pulse per match, not a level: a handler
//! that leaves `CNTIF` set is not re-entered until the next match.
//!
//! QingKe V4 layout (CH32V20x/V30x, `config: { profile: v4 }`; ch32fun
//! `ch32v20xhw.h` `SysTick_Type`: CTLR, SR, 64-bit CNT, 64-bit CMP; WCH
//! CH32FV2x_V3xRM chapter "SysTick"):
//! - `CNT` and `CMP` are 64 bits wide: `CNTL`/`CNTH` at +0x08/+0x0C and
//!   `CMPL`/`CMPH` at +0x10/+0x14;
//! - `CTLR.MODE` bit 4 counts down. `CNTIF` sets when a down count reaches
//!   0, and with `STRE` the count restarts from `CMP`;
//! - `CTLR.INIT` bit 5 loads the count when written as 1: 0 for an up count,
//!   `CMP` for a down count. WCH's SDK `Delay_Ms` sets it with `STE`. It is
//!   an action, not stored state, so it reads as 0.
//!
//! NOT modelled: `SWIE` (software interrupt 14).

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

const CTLR: u64 = 0x00;
const SR: u64 = 0x04;
const CNT: u64 = 0x08;
const CNTH: u64 = 0x0C;
const CMP: u64 = 0x10;
const CMPH: u64 = 0x14;

pub const CTLR_STE: u32 = 1 << 0;
pub const CTLR_STIE: u32 = 1 << 1;
pub const CTLR_STCLK: u32 = 1 << 2;
pub const CTLR_STRE: u32 = 1 << 3;
pub const CTLR_MODE: u32 = 1 << 4;
pub const CTLR_INIT: u32 = 1 << 5;
pub const CTLR_SWIE: u32 = 1 << 31;
const CTLR_MASK: u32 = CTLR_STE | CTLR_STIE | CTLR_STCLK | CTLR_STRE | CTLR_SWIE;

pub const SR_CNTIF: u32 = 1 << 0;

const LOW_WORD: u64 = 0xFFFF_FFFF;

/// QingKe SysTick (see the module docs).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct QingKeStk {
    /// QingKe V4 layout: 64-bit `CNT`/`CMP`, `MODE` and `INIT`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    v4: bool,
    ctlr: u32,
    cmp: u64,
    /// `CNT` at the last anchor.
    cnt_base: u64,
    /// `CNTIF` latched at the last anchor.
    cntif: bool,
    /// Cycle of the last anchor.
    anchor_cycle: u64,
    /// The time base when no cycle clock is attached (hand-built buses).
    walk_cycles: u64,
    /// Matches since the last anchor already delivered as an interrupt
    /// (legacy walk, used when the event scheduler is off).
    walk_irq_matches: u64,
    /// Bumped on every reschedule; an event whose token no longer matches
    /// was superseded and is dropped.
    generation: u32,
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl QingKeStk {
    pub fn new() -> Self {
        Self::default()
    }

    /// The QingKe V4 SysTick of the CH32V20x/V30x.
    pub fn new_v4() -> Self {
        Self {
            v4: true,
            ..Self::default()
        }
    }

    /// Mask of the counter width: 32 bits on V2, 64 on V4.
    fn width_mask(&self) -> u64 {
        if self.v4 {
            u64::MAX
        } else {
            u64::from(u32::MAX)
        }
    }

    fn ctlr_mask(&self) -> u32 {
        if self.v4 {
            CTLR_MASK | CTLR_MODE
        } else {
            CTLR_MASK
        }
    }

    fn counts_down(&self) -> bool {
        self.ctlr & CTLR_MODE != 0
    }

    /// Ticks from a match to the next one with `STRE` (`CMP + 1`); `None`
    /// when that exceeds the 64-bit range.
    fn reload_period(&self) -> Option<u64> {
        self.cmp.checked_add(1)
    }

    /// Ticks from a match to the next one: the reload period with `STRE`,
    /// otherwise one full wrap of the counter (`None` past 64 bits).
    fn match_period(&self) -> Option<u64> {
        if self.ctlr & CTLR_STRE != 0 {
            self.reload_period()
        } else {
            self.width_mask().checked_add(1)
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now(&self) -> u64 {
        self.clock
            .as_ref()
            .map_or(self.walk_cycles, CycleClock::now)
    }

    fn running(&self) -> bool {
        self.ctlr & CTLR_STE != 0
    }

    /// Counter ticks since the last anchor.
    fn elapsed(&self) -> u64 {
        if !self.running() {
            return 0;
        }
        let cycles = self.now().saturating_sub(self.anchor_cycle);
        if self.ctlr & CTLR_STCLK != 0 {
            cycles
        } else {
            cycles / 8
        }
    }

    /// Ticks from the anchor until the count reaches `CMP` (up) or 0
    /// (down); 0 = at once.
    fn ticks_to_match(&self) -> u64 {
        if self.counts_down() {
            self.cnt_base
        } else {
            self.cmp.wrapping_sub(self.cnt_base) & self.width_mask()
        }
    }

    fn matched(&self, elapsed: u64) -> bool {
        self.running() && elapsed >= self.ticks_to_match()
    }

    fn cnt(&self) -> u64 {
        let elapsed = self.elapsed();
        if self.ctlr & CTLR_STRE != 0 && self.matched(elapsed) {
            // Up: 0..=CMP, then 0 again; down: CMP..=0, then CMP again. A
            // period of CMP + 1 ticks either way.
            let after = elapsed - self.ticks_to_match();
            let phase = self.reload_period().map_or(after, |p| after % p);
            if self.counts_down() {
                self.cmp - phase
            } else {
                phase
            }
        } else if self.counts_down() {
            self.cnt_base.wrapping_sub(elapsed) & self.width_mask()
        } else {
            self.cnt_base.wrapping_add(elapsed) & self.width_mask()
        }
    }

    /// CPU cycles per counter tick.
    fn scale(&self) -> u64 {
        if self.ctlr & CTLR_STCLK != 0 {
            1
        } else {
            8
        }
    }

    /// Compare matches since the last anchor.
    fn matches(&self, elapsed: u64) -> u64 {
        if !self.matched(elapsed) {
            return 0;
        }
        let after = elapsed - self.ticks_to_match();
        1 + self.match_period().map_or(0, |period| after / period)
    }

    /// CPU cycles from now to the next compare match, `None` when the counter
    /// is stopped or (without auto-reload) has already matched.
    fn cycles_to_next_match(&self) -> Option<u64> {
        if !self.running() {
            return None;
        }
        let elapsed = self.elapsed();
        let ttm = self.ticks_to_match();
        let target = if elapsed < ttm {
            ttm
        } else if let Some(period) = self.reload_period().filter(|_| self.ctlr & CTLR_STRE != 0) {
            elapsed + period - (elapsed - ttm) % period
        } else {
            return None;
        };
        let cycle = self.anchor_cycle + target * self.scale();
        Some(cycle.saturating_sub(self.now()).max(1))
    }

    /// Cancel any scheduled interrupt and, while `STIE` is set, arm one for
    /// the next match. `pend_now` (a write that just set `STIE` while
    /// `CNTIF` was already set) arms one at once instead. Any other write
    /// leaves an already-latched `CNTIF` alone: a handler that moves `CMP`
    /// before it clears `SR` must not pend itself a second time.
    fn reschedule(&mut self, pend_now: bool) {
        self.generation = self.generation.wrapping_add(1);
        self.pending_events.clear();
        if !self.scheduler_mode() || self.ctlr & CTLR_STIE == 0 {
            return;
        }
        // The bus adds one cycle to every write-scheduled delay.
        let delay = if pend_now && self.cntif_now() {
            0
        } else if let Some(cycles) = self.cycles_to_next_match() {
            cycles - 1
        } else {
            return;
        };
        self.pending_events.push((delay, self.generation));
    }

    fn cntif_now(&self) -> bool {
        self.cntif || self.matched(self.elapsed())
    }

    fn anchor(&mut self) {
        self.cntif = self.cntif_now();
        self.cnt_base = self.cnt();
        self.anchor_cycle = self.now();
        self.walk_irq_matches = 0;
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            CTLR => self.ctlr,
            SR => u32::from(self.cntif_now()),
            CNT => self.cnt() as u32,
            CMP => self.cmp as u32,
            CNTH if self.v4 => (self.cnt() >> 32) as u32,
            CMPH if self.v4 => (self.cmp >> 32) as u32,
            _ => {
                crate::census_reg!("qingke_stk", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let stie_before = self.ctlr & CTLR_STIE != 0;
        match offset {
            CTLR => {
                self.anchor();
                self.ctlr = value & self.ctlr_mask();
                if self.v4 && value & CTLR_INIT != 0 {
                    self.cnt_base = if self.counts_down() { self.cmp } else { 0 };
                }
            }
            SR => {
                self.anchor();
                // Software clears CNTIF by writing 0; writing 1 keeps it.
                if value & SR_CNTIF == 0 {
                    self.cntif = false;
                }
            }
            CNT => {
                self.anchor();
                self.cnt_base = (self.cnt_base & !LOW_WORD) | u64::from(value);
            }
            CMP => {
                self.anchor();
                self.cmp = (self.cmp & !LOW_WORD) | u64::from(value);
            }
            CNTH if self.v4 => {
                self.anchor();
                self.cnt_base = (self.cnt_base & LOW_WORD) | (u64::from(value) << 32);
            }
            CMPH if self.v4 => {
                self.anchor();
                self.cmp = (self.cmp & LOW_WORD) | (u64::from(value) << 32);
            }
            _ => {
                crate::census_reg!("qingke_stk", offset, "write");
                return;
            }
        }
        self.reschedule(!stie_before && self.ctlr & CTLR_STIE != 0);
    }
}

impl Peripheral for QingKeStk {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_reg(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        let word = offset & !3;
        // SR: build on CNTIF so a byte store to another lane keeps it.
        let current = self.read_reg(word);
        let merged = (current & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_reg(word, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn legacy_tick_active(&self) -> bool {
        // Without the event scheduler, the walk also delivers the interrupt.
        self.running()
            && (self.clock.is_none() || (!self.scheduler_mode() && self.ctlr & CTLR_STIE != 0))
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
        let matches = self.matches(self.elapsed());
        let fired = matches > self.walk_irq_matches;
        self.walk_irq_matches = matches;
        PeripheralTickResult::with_irq(fired && self.ctlr & CTLR_STIE != 0)
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
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
        if event_token != self.generation || self.ctlr & CTLR_STIE == 0 {
            return crate::sched::EventResult::default();
        }
        crate::sched::EventResult {
            raise_own_irq: self.cntif_now(),
            reschedule_delay: self.cycles_to_next_match(),
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.anchor_cycle = clock.now();
        self.clock = Some(clock);
    }

    fn on_system_reset(&mut self) {
        let clock = self.clock.take();
        let walk = self.walk_cycles;
        let generation = self.generation;
        *self = Self {
            v4: self.v4,
            ..Self::default()
        };
        self.walk_cycles = walk;
        self.generation = generation;
        if let Some(clock) = clock {
            self.attach_cycle_clock(clock);
        } else {
            self.anchor_cycle = walk;
        }
        // Drop any interrupt armed before the reset.
        self.reschedule(false);
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noneos_delay_sequence_sets_cntif_after_cmp_hclk8_ticks() {
        // Delay_Ms(1) at 48 MHz: p_ms = 6000, so CMP = 6000 HCLK/8 ticks.
        let mut t = QingKeStk::new();
        let sr = t.read_u32(SR).unwrap();
        t.write_u32(SR, sr & !SR_CNTIF).unwrap();
        t.write_u32(CMP, 6000).unwrap();
        t.write_u32(CNT, 0).unwrap();
        let ctlr = t.read_u32(CTLR).unwrap();
        t.write_u32(CTLR, ctlr | CTLR_STE).unwrap();
        t.tick_elapsed(6000 * 8 - 1);
        assert_eq!(t.read_u32(SR).unwrap() & SR_CNTIF, 0);
        assert_eq!(t.read_u32(CNT).unwrap(), 5999);
        t.tick_elapsed(1);
        assert_eq!(t.read_u32(SR).unwrap() & SR_CNTIF, SR_CNTIF);
        // CNTIF stays set; stopping the counter keeps it.
        let ctlr = t.read_u32(CTLR).unwrap();
        t.write_u32(CTLR, ctlr & !CTLR_STE).unwrap();
        t.tick_elapsed(1000);
        assert_eq!(t.read_u32(SR).unwrap(), SR_CNTIF);
        assert_eq!(t.read_u32(CNT).unwrap(), 6000, "stopped count holds");
        t.write_u32(SR, 0).unwrap();
        assert_eq!(t.read_u32(SR).unwrap(), 0);
    }

    #[test]
    fn stclk_selects_hclk() {
        let mut t = QingKeStk::new();
        t.write_u32(CMP, 100).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK).unwrap();
        t.tick_elapsed(99);
        assert_eq!(t.read_u32(SR).unwrap(), 0);
        t.tick_elapsed(1);
        assert_eq!(t.read_u32(SR).unwrap(), SR_CNTIF);
    }

    #[test]
    fn stre_restarts_the_count_from_zero_after_cmp() {
        let mut t = QingKeStk::new();
        t.write_u32(CMP, 9).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK | CTLR_STRE)
            .unwrap();
        t.tick_elapsed(9);
        assert_eq!(t.read_u32(CNT).unwrap(), 0, "reached CMP, restarted");
        assert_eq!(t.read_u32(SR).unwrap(), SR_CNTIF);
        t.tick_elapsed(4);
        assert_eq!(t.read_u32(CNT).unwrap(), 4);
        t.tick_elapsed(6);
        assert_eq!(t.read_u32(CNT).unwrap(), 0, "period is CMP + 1");
    }

    #[test]
    fn without_stre_the_count_runs_past_cmp() {
        let mut t = QingKeStk::new();
        t.write_u32(CMP, 3).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK).unwrap();
        t.tick_elapsed(10);
        assert_eq!(t.read_u32(CNT).unwrap(), 10);
        assert_eq!(t.read_u32(SR).unwrap(), SR_CNTIF);
    }

    #[test]
    fn a_byte_store_to_ctlr_keeps_the_other_bits() {
        let mut t = QingKeStk::new();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK | CTLR_SWIE)
            .unwrap();
        t.write(CTLR, (CTLR_STCLK | CTLR_STRE) as u8).unwrap();
        assert_eq!(
            t.read_u32(CTLR).unwrap(),
            CTLR_STCLK | CTLR_STRE | CTLR_SWIE
        );
    }

    #[test]
    fn counts_from_the_cycle_clock() {
        let clock = CycleClock::default();
        clock.publish(500);
        let mut t = QingKeStk::new();
        t.attach_cycle_clock(clock.clone());
        t.write_u32(CMP, 80).unwrap();
        t.write_u32(CTLR, CTLR_STE).unwrap();
        clock.publish(500 + 80 * 8 - 1);
        assert_eq!(t.read_u32(SR).unwrap(), 0);
        clock.publish(500 + 80 * 8);
        assert_eq!(t.read_u32(SR).unwrap(), SR_CNTIF);
        assert_eq!(t.read_u32(CNT).unwrap(), 80);
    }

    #[test]
    fn system_reset_clears_the_registers() {
        let mut t = QingKeStk::new();
        t.write_u32(CMP, 1).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK).unwrap();
        t.tick_elapsed(5);
        t.on_system_reset();
        assert_eq!(t.read_u32(CTLR).unwrap(), 0);
        assert_eq!(t.read_u32(SR).unwrap(), 0);
        assert_eq!(t.read_u32(CNT).unwrap(), 0);
        assert_eq!(t.read_u32(CMP).unwrap(), 0);
    }

    #[test]
    fn v4_counts_64_bits_across_the_low_word() {
        let mut t = QingKeStk::new_v4();
        t.write_u32(CNT, 0xFFFF_FFF0).unwrap();
        t.write_u32(CMPH, 1).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK).unwrap();
        t.tick_elapsed(0x20);
        assert_eq!(t.read_u32(CNT).unwrap(), 0x10);
        assert_eq!(t.read_u32(CNTH).unwrap(), 1, "carry into CNTH");
        assert_eq!(t.read_u32(SR).unwrap(), SR_CNTIF, "CMP = 1 << 32 passed");
    }

    #[test]
    fn v4_cmp_high_word_gates_the_match_and_v2_has_none() {
        let mut t = QingKeStk::new_v4();
        t.write_u32(CMP, 5).unwrap();
        t.write_u32(CMPH, 1).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK).unwrap();
        t.tick_elapsed(5);
        assert_eq!(t.read_u32(SR).unwrap(), 0, "only the low word matched");
        assert_eq!(t.read_u32(CMPH).unwrap(), 1);

        let mut v2 = QingKeStk::new();
        v2.write_u32(CMPH, 1).unwrap();
        v2.write_u32(CNTH, 1).unwrap();
        assert_eq!(v2.read_u32(CMPH).unwrap(), 0, "reserved on V2");
        assert_eq!(v2.read_u32(CNTH).unwrap(), 0, "reserved on V2");
    }

    /// WCH SDK `Delay_Ms` on the CH32V20x: clear SR, CMP = n, MODE (count
    /// down), then INIT | STE, and wait for CNTIF.
    #[test]
    fn v4_down_count_with_init_reloads_cmp_and_flags_at_zero() {
        let mut t = QingKeStk::new_v4();
        t.write_u32(SR, 0).unwrap();
        t.write_u32(CMP, 100).unwrap();
        t.write_u32(CTLR, CTLR_MODE).unwrap();
        t.write_u32(CTLR, CTLR_MODE | CTLR_INIT | CTLR_STE | CTLR_STCLK)
            .unwrap();
        assert_eq!(t.read_u32(CNT).unwrap(), 100, "INIT loads CMP");
        assert_eq!(
            t.read_u32(CTLR).unwrap() & CTLR_INIT,
            0,
            "INIT is an action, not state"
        );
        t.tick_elapsed(99);
        assert_eq!(t.read_u32(CNT).unwrap(), 1);
        assert_eq!(t.read_u32(SR).unwrap(), 0);
        t.tick_elapsed(1);
        assert_eq!(t.read_u32(CNT).unwrap(), 0);
        assert_eq!(t.read_u32(SR).unwrap(), SR_CNTIF);
    }

    #[test]
    fn v4_down_count_with_stre_restarts_from_cmp() {
        let mut t = QingKeStk::new_v4();
        t.write_u32(CMP, 9).unwrap();
        t.write_u32(
            CTLR,
            CTLR_MODE | CTLR_INIT | CTLR_STE | CTLR_STCLK | CTLR_STRE,
        )
        .unwrap();
        t.tick_elapsed(9);
        assert_eq!(t.read_u32(CNT).unwrap(), 9, "reached 0, reloaded CMP");
        assert_eq!(t.read_u32(SR).unwrap(), SR_CNTIF);
        t.tick_elapsed(4);
        assert_eq!(t.read_u32(CNT).unwrap(), 5);
        t.tick_elapsed(6);
        assert_eq!(t.read_u32(CNT).unwrap(), 9, "period is CMP + 1");
    }

    #[test]
    fn v4_init_on_an_up_count_loads_zero_and_v2_ignores_mode_and_init() {
        let mut t = QingKeStk::new_v4();
        t.write_u32(CNT, 77).unwrap();
        t.write_u32(CTLR, CTLR_INIT).unwrap();
        assert_eq!(t.read_u32(CNT).unwrap(), 0);

        let mut v2 = QingKeStk::new();
        v2.write_u32(CNT, 77).unwrap();
        v2.write_u32(CTLR, CTLR_MODE | CTLR_INIT).unwrap();
        assert_eq!(v2.read_u32(CTLR).unwrap(), 0);
        assert_eq!(v2.read_u32(CNT).unwrap(), 77);
    }

    /// ch32fun `systick_irq`: CNTIF is still latched from the boot-time
    /// busy-wait when the handler is enabled. Setting STIE pends at once;
    /// the handler then moves CMP before it clears SR, and that write must
    /// arm the next match, not a second immediate interrupt.
    #[test]
    fn a_cmp_write_with_cntif_latched_arms_the_next_match_only() {
        if !crate::cycle_clock::event_scheduler_enabled() {
            return;
        }
        let clock = CycleClock::default();
        let mut t = QingKeStk::new_v4();
        t.attach_cycle_clock(clock.clone());
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK).unwrap();
        clock.publish(20);
        assert_eq!(t.read_u32(SR).unwrap(), SR_CNTIF, "CMP 0 passed");
        t.write_u32(CMP, 100).unwrap();
        t.write_u32(CNT, 0).unwrap();
        let _ = t.take_scheduled_events();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK | CTLR_STIE)
            .unwrap();
        let events = t.take_scheduled_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, 0, "enabling STIE over CNTIF pends at once");
        // The handler: CMP += 100 (low word, then high word), SR = 0.
        clock.publish(25);
        t.write_u32(CMP, 200).unwrap();
        let events = t.take_scheduled_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, 200 - 5 - 1, "armed for CNT = 200");
        t.write_u32(CMPH, 0).unwrap();
        t.write_u32(SR, 0).unwrap();
        let events = t.take_scheduled_events();
        assert_eq!(events[0].0, 200 - 5 - 1);
    }
}
