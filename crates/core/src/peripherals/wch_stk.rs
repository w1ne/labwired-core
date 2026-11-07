// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WCH QingKe V2 system timer (STK) — the CH32V003 "SysTick" at `0xE000_F000`.
//!
//! This is NOT the ARMv7-M SysTick (`systick.rs`): the register file, the
//! count direction and the flag semantics all differ, so the ARM model cannot
//! be reused with a WCH base.
//!
//! ## Register file (WCH `core_riscv.h`, `SysTick_Type`)
//!
//! | off  | reg  | notes                                                    |
//! |------|------|----------------------------------------------------------|
//! | 0x00 | CTLR | STE(0) STIE(1) STCLK(2) STRE(3) … SWIE(31)               |
//! | 0x04 | SR   | CNTIF(0): set when CNT counts up to CMP; write 0 clears  |
//! | 0x08 | CNT  | 32-bit up-counter                                        |
//! | 0x10 | CMP  | 32-bit compare value                                     |
//!
//! Bit positions are cross-checked against a second, independent source:
//! ch32fun `ch32v003hw.h` (`SYSTICK_CTLR_STE/STIE/STCLK/STRE/SWIE`,
//! `SYSTICK_SR_CNTIF`).
//!
//! ## Firmware this serves
//!
//! * WCH EVT `Debug/debug.c` `Delay_Ms`: clear CNTIF, `CMP = n * p_ms`,
//!   `CNT = 0`, set STE, poll `SR.CNTIF`, clear STE. `p_ms` is
//!   `SystemCoreClock / 8000`, i.e. the default HCLK/8 time base (STCLK = 0).
//! * ch32fun `SystemInit`: `CTLR = 1` (or 5 for HCLK) and leaves CNT
//!   free-running; `DelaySysTick` polls `CNT` against a target.
//!
//! ## Time source
//!
//! Same two drive modes as `Esp32c6LpTimer`: with an event-scheduler
//! [`CycleClock`] attached the counter advances lazily at MMIO access; without
//! one the legacy walk drives [`Peripheral::tick_elapsed`]. HCLK is the CPU
//! cycle; STCLK = 0 divides it by 8 and the sub-tick remainder is carried so no
//! time is lost across reads.
//!
//! ## Interrupt
//!
//! With CTLR.STIE set, CNTIF is a LEVEL source on the entry's `irq:` line
//! (`SysTicK_IRQn = 12` through the PFIC): it stays asserted until firmware
//! writes SR = 0, which is what the WCH Arduino core's `SysTick_Handler` does.
//! On the walk path a match raises the line from `tick_elapsed`; on the
//! scheduler path the model books a wake at the exact cycle CNT reaches CMP
//! and raises it from `on_event`.
//!
//! ## What is deliberately NOT modelled
//!
//! * The software interrupt (SWIE). The bit is stored and never delivered.
//! * STRE reload timing beyond "CNT reaches CMP, the next count is 0"; the
//!   WCH reference manual does not spell out the reload cycle and no
//!   shipped example here depends on it.

use crate::{CycleClock, Peripheral, SimResult};
use std::cell::Cell;

/// The STK window. Only 0x00..0x18 is defined; the rest reads as zero.
pub const WCH_STK_SIZE: u64 = 0x100;

const CTLR: u64 = 0x00;
const SR: u64 = 0x04;
const CNT: u64 = 0x08;
const CMP: u64 = 0x10;

const CTLR_STE: u32 = 1 << 0;
const CTLR_STIE: u32 = 1 << 1;
const CTLR_STCLK: u32 = 1 << 2;
const CTLR_STRE: u32 = 1 << 3;
const SR_CNTIF: u32 = 1 << 0;

/// HCLK/8 when CTLR.STCLK is clear.
const HCLK_DIV8: u64 = 8;

#[derive(Debug)]
pub struct WchStk {
    ctlr: u32,
    cmp: u32,
    cnt: Cell<u32>,
    cntif: Cell<bool>,
    /// CPU cycles not yet worth a whole STK count under HCLK/8.
    prescale: Cell<u64>,
    /// Lazy-path anchor: absolute CPU cycle the counter was last advanced to.
    anchor: Cell<u64>,
    /// Bus-published cycle clock; `None` keeps the model on the walk path.
    clock: Option<CycleClock>,
    /// Scheduler wake generation. Every reconfiguration books a new wake; a
    /// wake whose token is not the current generation is stale and ignored.
    generation: u32,
    /// Wakes booked by the last write, drained by `take_scheduled_events`.
    pending_events: Vec<(u64, u32)>,
}

impl Default for WchStk {
    fn default() -> Self {
        Self::new()
    }
}

impl WchStk {
    pub fn new() -> Self {
        Self {
            ctlr: 0,
            cmp: 0,
            cnt: Cell::new(0),
            cntif: Cell::new(false),
            prescale: Cell::new(0),
            anchor: Cell::new(0),
            clock: None,
            generation: 0,
            pending_events: Vec::new(),
        }
    }

    crate::cycle_clock::scheduler_mode!();

    /// Test/differential knob: pin the model to the legacy walk path.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// Apply `cycles` CPU cycles at the current configuration. Returns whether
    /// CNT reached CMP (CNTIF set) inside them.
    fn advance_cycles(&self, cycles: u64) -> bool {
        if cycles == 0 || self.ctlr & CTLR_STE == 0 {
            return false;
        }
        let counts = if self.ctlr & CTLR_STCLK != 0 {
            cycles
        } else {
            let total = self.prescale.get() + cycles;
            self.prescale.set(total % HCLK_DIV8);
            total / HCLK_DIV8
        };
        let (cnt, hit) =
            Self::count_up(self.cnt.get(), self.cmp, self.ctlr & CTLR_STRE != 0, counts);
        self.cnt.set(cnt);
        if hit {
            self.cntif.set(true);
        }
        hit
    }

    fn interrupt_enabled(&self) -> bool {
        self.ctlr & (CTLR_STE | CTLR_STIE) == CTLR_STE | CTLR_STIE
    }

    /// CPU cycles from the current (synced) state until CNT next reaches CMP.
    /// The inverse of `count_up`: from CNT == CMP the next match is a full
    /// period away (`cmp + 1` with STRE, 2^32 without).
    fn cycles_to_next_match(&self) -> u64 {
        let (cnt, cmp) = (self.cnt.get(), self.cmp);
        let counts = if cnt == cmp {
            if self.ctlr & CTLR_STRE != 0 {
                cmp as u64 + 1
            } else {
                1u64 << 32
            }
        } else {
            cmp.wrapping_sub(cnt) as u64
        };
        if self.ctlr & CTLR_STCLK != 0 {
            counts
        } else {
            counts * HCLK_DIV8 - self.prescale.get()
        }
    }

    /// Book the scheduler wake for the next match, superseding any earlier one.
    fn rearm(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending_events.clear();
        if self.scheduler_mode() && self.interrupt_enabled() {
            self.pending_events
                .push((self.cycles_to_next_match(), self.generation));
        }
    }

    /// Closed form of `n` up-counts from `cnt`: returns `(new_cnt, matched)`.
    /// A match is the counter arriving at `cmp` by counting. With `stre` the
    /// count after a match is 0, so the period is `cmp + 1`; without it the
    /// counter runs on and wraps at 2^32.
    fn count_up(mut cnt: u32, cmp: u32, stre: bool, mut n: u64) -> (u32, bool) {
        let mut hit = false;
        while n > 0 {
            if stre && cnt == cmp {
                cnt = 0;
                n -= 1;
                if cmp == 0 {
                    hit = true;
                }
                continue;
            }
            let to_match = match cmp.wrapping_sub(cnt) {
                0 => 1u64 << 32,
                d => d as u64,
            };
            if n < to_match {
                cnt = cnt.wrapping_add(n as u32);
                break;
            }
            cnt = cmp;
            n -= to_match;
            hit = true;
            if stre && n > 0 {
                let period = cmp as u64 + 1;
                let k = n % period;
                cnt = if k == 0 { cmp } else { (k - 1) as u32 };
                break;
            }
        }
        (cnt, hit)
    }

    fn sync_from_clock(&self) {
        if let Some(clock) = &self.clock {
            if self.scheduler_mode() {
                self.advance_to(clock.now());
            }
        }
    }

    fn advance_to(&self, now: u64) {
        let anchor = self.anchor.get();
        if now <= anchor {
            return;
        }
        self.anchor.set(now);
        self.advance_cycles(now - anchor);
    }

    /// Wake drain for tests (what the bus does after a write).
    #[cfg(test)]
    fn take_events(&mut self) -> Vec<(u64, u32)> {
        std::mem::take(&mut self.pending_events)
    }

    /// Live counter (test introspection).
    pub fn counter(&self) -> u32 {
        self.cnt.get()
    }
}

impl Peripheral for WchStk {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_u32(offset & !3)?;
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let sh = (offset & 3) * 8;
        let cur = self.read_u32(aligned)?;
        self.write_u32(aligned, (cur & !(0xFFu32 << sh)) | ((value as u32) << sh))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.sync_from_clock();
        Ok(match offset {
            CTLR => self.ctlr,
            SR => self.cntif.get() as u32,
            CNT => self.cnt.get(),
            CMP => self.cmp,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // Settle elapsed time under the OLD configuration first.
        self.sync_from_clock();
        match offset {
            CTLR => self.ctlr = value,
            // CNTIF is cleared by software writing 0; writing 1 does not set it.
            SR if value & SR_CNTIF == 0 => self.cntif.set(false),
            CNT => self.cnt.set(value),
            CMP => self.cmp = value,
            _ => return Ok(()),
        }
        self.rearm();
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, cycles: u64) -> crate::PeripheralTickResult {
        let hit = !self.scheduler_mode() && self.advance_cycles(cycles);
        crate::PeripheralTickResult {
            irq: hit && self.interrupt_enabled(),
            ..Default::default()
        }
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.ctlr & CTLR_STIE != 0 && self.cntif.get())
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
        if event_token != self.generation || !self.interrupt_enabled() {
            return crate::sched::EventResult::default();
        }
        self.sync_from_clock();
        // The wake lands on the match cycle, so CNTIF is set now. The next
        // wake re-uses this token: the configuration has not changed.
        crate::sched::EventResult {
            raise_own_irq: self.cntif.get(),
            reschedule_delay: Some(self.cycles_to_next_match()),
            ..Default::default()
        }
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn sync_to(&mut self, now_cycle: u64) {
        self.advance_to(now_cycle);
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.anchor.set(clock.now());
        self.clock = Some(clock);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_counter_does_not_move() {
        let mut t = WchStk::new();
        t.tick_elapsed(1000);
        assert_eq!(t.counter(), 0);
    }

    #[test]
    fn hclk_div8_counts_and_carries_the_remainder() {
        let mut t = WchStk::new();
        t.write_u32(CTLR, CTLR_STE).unwrap();
        t.tick_elapsed(7);
        assert_eq!(t.counter(), 0);
        t.tick_elapsed(1);
        assert_eq!(t.counter(), 1);
        for _ in 0..80 {
            t.tick();
        }
        assert_eq!(t.counter(), 11);
    }

    #[test]
    fn stclk_selects_hclk() {
        let mut t = WchStk::new();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK).unwrap();
        t.tick_elapsed(100);
        assert_eq!(t.counter(), 100);
    }

    /// WCH `Delay_Ms(1)` at SystemCoreClock = 24 MHz: p_ms = 3000.
    #[test]
    fn evt_delay_ms_sequence_sets_cntif_after_cmp_counts() {
        let mut t = WchStk::new();
        t.write_u32(SR, 0).unwrap();
        t.write_u32(CMP, 3000).unwrap();
        t.write_u32(CNT, 0).unwrap();
        t.write_u32(CTLR, CTLR_STE).unwrap();
        t.tick_elapsed(3000 * 8 - 1);
        assert_eq!(t.read_u32(SR).unwrap() & SR_CNTIF, 0, "one HCLK early");
        t.tick_elapsed(1);
        assert_eq!(t.read_u32(SR).unwrap() & SR_CNTIF, SR_CNTIF);
        // Without STRE the counter runs on past CMP.
        t.tick_elapsed(80);
        assert_eq!(t.counter(), 3010);
        // Writing 1 does not set or keep the flag set; writing 0 clears it.
        t.write_u32(SR, 1).unwrap();
        assert_eq!(t.read_u32(SR).unwrap(), 1);
        t.write_u32(SR, 0).unwrap();
        assert_eq!(t.read_u32(SR).unwrap(), 0);
    }

    #[test]
    fn stre_restarts_from_zero_after_the_match() {
        let mut t = WchStk::new();
        t.write_u32(CMP, 9).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK | CTLR_STRE)
            .unwrap();
        t.tick_elapsed(9);
        assert_eq!(t.counter(), 9);
        assert_eq!(t.read_u32(SR).unwrap(), 1);
        t.tick_elapsed(1);
        assert_eq!(t.counter(), 0);
        t.tick_elapsed(25);
        assert_eq!(t.counter(), 5, "period is CMP + 1 = 10");
    }

    #[test]
    fn closed_form_matches_single_steps() {
        for &(start, cmp, stre) in &[
            (0u32, 0u32, true),
            (0, 0, false),
            (5, 3, true),
            (5, 3, false),
            (u32::MAX - 2, 1, false),
            (u32::MAX - 2, 1, true),
            (0, 4, true),
        ] {
            let (mut cnt, mut hit) = (start, false);
            for n in 1..40u64 {
                let (c1, h1) = WchStk::count_up(cnt, cmp, stre, 1);
                cnt = c1;
                hit |= h1;
                let (cn, hn) = WchStk::count_up(start, cmp, stre, n);
                assert_eq!(
                    (cn, hn),
                    (cnt, hit),
                    "start={start} cmp={cmp} stre={stre} n={n}"
                );
            }
        }
    }

    #[test]
    fn free_running_cnt_wraps_at_32_bits() {
        let mut t = WchStk::new();
        t.write_u32(CNT, u32::MAX).unwrap();
        t.write_u32(CMP, 0x1000).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK).unwrap();
        t.tick_elapsed(2);
        assert_eq!(t.counter(), 1);
    }

    /// The WCH Arduino core's `main` prologue: SR = 0, CTLR = 0, CNT = 0,
    /// CMP = 48 MHz / 1 kHz - 1, CTLR = STE|STIE|STCLK|STRE (0xF).
    fn arduino_millis_tick() -> WchStk {
        let mut t = WchStk::new();
        t.write_u32(SR, 0).unwrap();
        t.write_u32(CTLR, 0).unwrap();
        t.write_u32(CNT, 0).unwrap();
        t.write_u32(CMP, 0xBB7F).unwrap();
        t.write_u32(CTLR, 0xF).unwrap();
        t
    }

    #[test]
    fn stie_raises_a_level_irq_on_each_match_until_sr_is_cleared() {
        let mut t = arduino_millis_tick();
        assert_eq!(t.irq_line_level(), Some(false));
        assert!(!t.tick_elapsed(0xBB7F - 1).irq, "one count early");
        assert!(t.tick_elapsed(1).irq, "CNT reached CMP");
        assert_eq!(t.irq_line_level(), Some(true));
        // The handler writes SR = 0: the line drops.
        t.write_u32(SR, 0).unwrap();
        assert_eq!(t.irq_line_level(), Some(false));
        // Period is CMP + 1 = 48000 HCLK: the next match is 48000 later.
        assert!(!t.tick_elapsed(47_999).irq);
        assert!(t.tick_elapsed(1).irq);
    }

    #[test]
    fn without_stie_a_match_sets_cntif_but_raises_nothing() {
        let mut t = WchStk::new();
        t.write_u32(CMP, 4).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK).unwrap();
        assert!(!t.tick_elapsed(4).irq);
        assert_eq!(t.read_u32(SR).unwrap(), 1);
        assert_eq!(t.irq_line_level(), Some(false));
    }

    /// The scheduler wake lands on the exact match cycle, under both clock
    /// sources, and a reconfiguration supersedes the booked wake.
    #[test]
    fn next_match_distance_is_exact() {
        let mut t = arduino_millis_tick();
        assert_eq!(t.cycles_to_next_match(), 0xBB7F);
        t.tick_elapsed(0xBB7F);
        assert_eq!(
            t.cycles_to_next_match(),
            48_000,
            "STRE: full period from CNT == CMP"
        );

        let mut t = WchStk::new();
        t.write_u32(CMP, 10).unwrap();
        t.write_u32(CTLR, CTLR_STE).unwrap(); // HCLK/8
        t.tick_elapsed(3); // 3 cycles into the first count
        assert_eq!(t.cycles_to_next_match(), 10 * 8 - 3);
        t.tick_elapsed(10 * 8 - 3);
        assert_eq!(t.read_u32(SR).unwrap(), 1);
    }

    #[test]
    fn writes_book_a_wake_only_in_scheduler_mode_with_stie() {
        let clock = CycleClock::default();
        let mut t = WchStk::new();
        t.attach_cycle_clock(clock);
        if !t.uses_scheduler() {
            return;
        }
        t.write_u32(CMP, 0xBB7F).unwrap();
        t.write_u32(CTLR, CTLR_STE | CTLR_STCLK).unwrap();
        assert!(
            t.take_events().is_empty(),
            "STIE clear: nothing to wake for"
        );
        t.write_u32(CTLR, 0xF).unwrap();
        let events = t.take_events();
        assert_eq!(events, vec![(0xBB7F, t.generation)]);
    }

    #[test]
    fn clock_attach_tracks_published_cycles() {
        let clock = CycleClock::default();
        let mut t = WchStk::new();
        t.attach_cycle_clock(clock.clone());
        if !t.uses_scheduler() {
            return;
        }
        t.write_u32(CTLR, CTLR_STE).unwrap();
        clock.publish(800);
        assert_eq!(t.read_u32(CNT).unwrap(), 100);
        assert_eq!(t.read_u32(CNT).unwrap(), 100, "idempotent at one cycle");
        clock.publish(804);
        assert_eq!(t.read_u32(CNT).unwrap(), 100);
        clock.publish(808);
        assert_eq!(t.read_u32(CNT).unwrap(), 101, "prescaler remainder kept");
    }
}
