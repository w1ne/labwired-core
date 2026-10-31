// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RA low-power asynchronous general-purpose timer, 16-bit AGT.
//!
//! Register layout from FSP `R7FA8M1AH.h` (`R_AGTX0_AGT16_Type`): `AGT`
//! (counter / reload, u16) +0x00, `AGTCMA` +0x02, `AGTCMB` +0x04, then the
//! byte registers `AGTCR` +0x08, `AGTMR1` +0x09, `AGTMR2` +0x0A,
//! `AGTIOSEL_ALT` +0x0B, `AGTIOC` +0x0C, `AGTISR` +0x0D, `AGTCMSR` +0x0E,
//! `AGTIOSEL` +0x0F. One instance per 0x100 window (AGT0 0x4022_1000, AGT1
//! 0x4022_1100 on the RA8M1; FSP `bsp_feature.h`: two 16-bit AGT channels,
//! no 32-bit AGTW). Behaviour from FSP `r_agt.c` and Zephyr's
//! `counter_renesas_ra_agt.c`, which both drive it.
//!
//! Modelled (timer mode, `AGTMR1.TMOD` = 0):
//! - a down-counter on the selected count source (`AGTMR1.TCK`: PCLKB,
//!   PCLKB/8, PCLKB/2, or AGTLCLK/AGTSCLK = 32.768 kHz divided by
//!   2^`AGTMR2.CKS`), derived lazily from the bus cycle count at `clock_hz`;
//! - `AGT` writes load the reload register AND the counter at once (FSP
//!   `R_AGT_PeriodSet`: "The new period is updated immediately and the
//!   counter is reset");
//! - underflow (count past 0): reload, set `AGTCR.TUNDF`, fire the AGTI event;
//! - compare match A/B when the count steps onto `AGTCMA`/`AGTCMB` with
//!   `AGTCMSR.TCMEA`/`TCMEB` set: set `TCMAF`/`TCMBF`, fire the event;
//! - `AGTCR.TSTART` starts/stops, `TCSTF` follows it at once (silicon syncs
//!   it to the count source); the four flags are write-0-to-clear; `TSTOP`
//!   forces a stop and sets `AGT`, `AGTCMA`, `AGTCMB` to 0xFFFF.
//!
//! Interrupts are ELC events, not NVIC lines: `INT` (`elc_event`), compare
//! A (`elc_event` + 1) and compare B (`elc_event` + 2), as in FSP
//! `bsp_elc.h` (`ELC_EVENT_AGT0_INT` 0x046 / `_COMPARE_A` 0x047 /
//! `_COMPARE_B` 0x048 on the RA8M1). They leave through `explicit_irqs`
//! with [`Peripheral::explicit_irqs_are_link_events`], so the bus links
//! them through the ICU `IELSRn` ([`super::ra_icu`]).
//!
//! NOT modelled: the pulse output, event counter, pulse width / period
//! measurement modes and the AGTIO/AGTEE pins (`AGTIOC`, `AGTISR`,
//! `AGTIOSEL*` are stored only), the AGT0-underflow count source of AGT1
//! (it does not count), `AGTMR2.LPM`, and any skew when firmware moves the
//! CPU clock away from `clock_hz`.

use crate::{CycleClock, Peripheral, SimResult};

const AGT: u64 = 0x00;
const AGTCMA: u64 = 0x02;
const AGTCMB: u64 = 0x04;
const AGTCR: u64 = 0x08;
const AGTMR1: u64 = 0x09;
const AGTMR2: u64 = 0x0A;
const AGTIOSEL_ALT: u64 = 0x0B;
const AGTIOC: u64 = 0x0C;
const AGTISR: u64 = 0x0D;
const AGTCMSR: u64 = 0x0E;
const AGTIOSEL: u64 = 0x0F;

const TSTART: u8 = 1 << 0;
const TCSTF: u8 = 1 << 1;
const TSTOP: u8 = 1 << 2;
const TEDGF: u8 = 1 << 4;
const TUNDF: u8 = 1 << 5;
const TCMAF: u8 = 1 << 6;
const TCMBF: u8 = 1 << 7;
const FLAGS: u8 = TEDGF | TUNDF | TCMAF | TCMBF;

const TCMEA: u8 = 1 << 0;
const TCMEB: u8 = 1 << 4;

/// AGTLCLK (LOCO) / AGTSCLK (sub-clock) rate.
const LOW_CLK_HZ: u64 = 32_768;
/// RA8M1 PCLKB under Zephyr/FSP's EK-RA8M1 clock tree (PLL1P 480 MHz / 8).
pub const DEFAULT_PCLKB_HZ: u64 = 60_000_000;
/// The rate the bus cycles run at (Zephyr's EK-RA8M1 ICLK, 480 MHz).
pub const DEFAULT_CLOCK_HZ: u64 = 480_000_000;
/// `ELC_EVENT_AGT0_INT` on the RA8M1.
pub const DEFAULT_ELC_EVENT: u32 = 0x046;

/// What the next count edge(s) at `armed_tick` raise.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Due {
    underflow: bool,
    cma: bool,
    cmb: bool,
}

/// One RA AGT channel.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RaAgt {
    clock_hz: u64,
    pclkb_hz: u64,
    elc_event: u32,
    reload: u16,
    cma: u16,
    cmb: u16,
    agtcr: u8,
    agtmr1: u8,
    agtmr2: u8,
    agtiosel_alt: u8,
    agtioc: u8,
    agtisr: u8,
    agtcmsr: u8,
    agtiosel: u8,
    /// Counter value at `anchor_tick` (count-source ticks since cycle 0).
    anchor_count: u16,
    anchor_tick: u64,
    /// The next event: its count-source tick and what it raises.
    armed: Option<(u64, Due)>,
    /// CPU cycle `armed` was queued for (detects a count-source change that
    /// moves the same tick to another cycle).
    armed_cycle_was: Option<u64>,
    /// Bumped whenever `armed` is recomputed; a stale scheduled event whose
    /// token no longer matches is dropped.
    generation: u32,
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl RaAgt {
    pub fn new(clock_hz: u64, pclkb_hz: u64, elc_event: u32) -> Self {
        Self {
            clock_hz: clock_hz.max(1),
            pclkb_hz: pclkb_hz.max(1),
            elc_event,
            reload: 0xFFFF,
            cma: 0xFFFF,
            cmb: 0xFFFF,
            agtcr: 0,
            agtmr1: 0,
            agtmr2: 0,
            agtiosel_alt: 0,
            agtioc: 0,
            agtisr: 0,
            agtcmsr: 0,
            agtiosel: 0,
            anchor_count: 0xFFFF,
            anchor_tick: 0,
            armed: None,
            armed_cycle_was: None,
            generation: 0,
            pending_events: Vec::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now_cycle(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn running(&self) -> bool {
        self.agtcr & TSTART != 0
    }

    /// Count-source rate from `AGTMR1.TCK` / `AGTMR2.CKS`; `None` for the
    /// AGT0-underflow source (not modelled) and reserved encodings.
    fn source_hz(&self) -> Option<u64> {
        match (self.agtmr1 >> 4) & 7 {
            0 => Some(self.pclkb_hz),
            1 => Some(self.pclkb_hz / 8),
            3 => Some(self.pclkb_hz / 2),
            4 | 6 => Some(LOW_CLK_HZ >> (self.agtmr2 & 7)),
            _ => None,
        }
    }

    /// Count-source ticks elapsed at CPU cycle `cycle`.
    fn ticks_at(&self, cycle: u64, hz: u64) -> u64 {
        (u128::from(cycle) * u128::from(hz) / u128::from(self.clock_hz)) as u64
    }

    /// First CPU cycle at which `ticks` count-source ticks have elapsed.
    fn cycle_of_tick(&self, ticks: u64, hz: u64) -> u64 {
        (u128::from(ticks) * u128::from(self.clock_hz)).div_ceil(u128::from(hz)) as u64
    }

    fn ticks_now(&self) -> Option<u64> {
        let hz = self.source_hz()?;
        Some(self.ticks_at(self.now_cycle(), hz))
    }

    /// Counter value `n` count edges after the anchor.
    fn count_after(&self, n: u64) -> u16 {
        let c = u64::from(self.anchor_count);
        if n <= c {
            return (c - n) as u16;
        }
        let period = u64::from(self.reload) + 1;
        (u64::from(self.reload) - (n - c - 1) % period) as u16
    }

    /// The counter now.
    pub fn count(&self) -> u16 {
        if !self.running() {
            return self.anchor_count;
        }
        match self.ticks_now() {
            Some(t) => self.count_after(t.saturating_sub(self.anchor_tick)),
            None => self.anchor_count,
        }
    }

    /// Edges from a count of `c` until the counter steps onto `x`.
    fn edges_to(&self, c: u16, x: u16) -> Option<u64> {
        if x < c {
            Some(u64::from(c - x))
        } else if x <= self.reload {
            Some(u64::from(c) + 1 + u64::from(self.reload - x))
        } else {
            None
        }
    }

    /// Recompute the next event from the anchor (anchor = now).
    ///
    /// A write that leaves the next edge where it was (a flag clear, a
    /// compare-B change while B is off) keeps the queued event and its token:
    /// re-arming it under a new token would leave the old one resident until
    /// its deadline (the scheduler has no cancel), and Zephyr's alarm path
    /// does several such writes per alarm while a 32 s underflow is pending.
    fn rearm(&mut self) {
        let previous = self.armed.take().zip(self.armed_cycle_was);
        self.armed_cycle_was = None;
        let next = self.next_edge();
        if let (Some((tick, due, hz)), Some(((ptick, pdue), pcycle))) = (next, previous) {
            if (tick, due) == (ptick, pdue) && self.cycle_of_tick(tick, hz) == pcycle {
                self.armed = Some((tick, due));
                self.armed_cycle_was = Some(pcycle);
                return;
            }
        }
        self.generation = self.generation.wrapping_add(1);
        let Some((tick, due, hz)) = next else { return };
        self.armed = Some((tick, due));
        let fire_cycle = self.cycle_of_tick(tick, hz);
        self.armed_cycle_was = Some(fire_cycle);
        if self.scheduler_mode() {
            let now = self.now_cycle();
            let fire = fire_cycle.max(now + 1);
            self.pending_events.push((fire - now, self.generation));
        }
    }

    /// The next count edge that raises something, from the anchor:
    /// `(tick, what, source Hz)`; `None` when stopped or not counting.
    fn next_edge(&self) -> Option<(u64, Due, u64)> {
        if !self.running() || self.clock.is_none() {
            return None;
        }
        let hz = self.source_hz()?;
        let c = self.anchor_count;
        let mut best: Option<(u64, Due)> = None;
        let mut consider = |d: Option<u64>, set: fn(&mut Due)| {
            let Some(d) = d else { return };
            match &mut best {
                Some((bd, due)) if *bd == d => set(due),
                Some((bd, _)) if *bd < d => {}
                _ => {
                    let mut due = Due::default();
                    set(&mut due);
                    best = Some((d, due));
                }
            }
        };
        consider(Some(u64::from(c) + 1), |d| d.underflow = true);
        if self.agtcmsr & TCMEA != 0 {
            consider(self.edges_to(c, self.cma), |d| d.cma = true);
        }
        if self.agtcmsr & TCMEB != 0 {
            consider(self.edges_to(c, self.cmb), |d| d.cmb = true);
        }
        let (d, due) = best?;
        Some((self.anchor_tick + d, due, hz))
    }

    /// CPU cycle of the armed event.
    fn armed_cycle(&self) -> Option<u64> {
        let (tick, _) = self.armed?;
        Some(self.cycle_of_tick(tick, self.source_hz()?))
    }

    /// Fire the armed event: set flags, re-anchor on its edge, arm the next.
    /// Returns the ELC events raised.
    fn fire(&mut self) -> Vec<u32> {
        let Some((tick, due)) = self.armed.take() else {
            return Vec::new();
        };
        let n = tick - self.anchor_tick;
        self.anchor_count = self.count_after(n);
        self.anchor_tick = tick;
        let mut events = Vec::new();
        if due.underflow {
            self.agtcr |= TUNDF;
            events.push(self.elc_event);
        }
        if due.cma {
            self.agtcr |= TCMAF;
            events.push(self.elc_event + 1);
        }
        if due.cmb {
            self.agtcr |= TCMBF;
            events.push(self.elc_event + 2);
        }
        self.rearm();
        events
    }

    fn read_reg(&self, offset: u64) -> u8 {
        let half = |v: u16| (v >> ((offset & 1) * 8)) as u8;
        match offset {
            o if o & !1 == AGT => half(self.count()),
            o if o & !1 == AGTCMA => half(self.cma),
            o if o & !1 == AGTCMB => half(self.cmb),
            AGTCR => {
                let tcstf = if self.running() { TCSTF } else { 0 };
                (self.agtcr & !(TCSTF | TSTOP)) | tcstf
            }
            AGTMR1 => self.agtmr1,
            AGTMR2 => self.agtmr2,
            AGTIOSEL_ALT => self.agtiosel_alt,
            AGTIOC => self.agtioc,
            AGTISR => self.agtisr,
            AGTCMSR => self.agtcmsr,
            AGTIOSEL => self.agtiosel,
            _ => 0,
        }
    }

    fn write_agtcr(&mut self, value: u8) {
        // Freeze the count under the old run state first.
        let count = self.count();
        self.anchor_count = count;
        self.anchor_tick = self.ticks_now().unwrap_or(0);
        // Flags: write 0 clears, write 1 keeps.
        let flags = self.agtcr & value & FLAGS;
        if value & TSTOP != 0 {
            self.agtcr = 0;
            self.reload = 0xFFFF;
            self.cma = 0xFFFF;
            self.cmb = 0xFFFF;
            self.anchor_count = 0xFFFF;
        } else {
            self.agtcr = flags | (value & TSTART);
        }
        self.rearm();
    }

    fn write_reg(&mut self, offset: u64, value: u8) {
        let set_half = |v: &mut u16| {
            let shift = (offset & 1) * 8;
            *v = (*v & !(0xFF << shift)) | (u16::from(value) << shift);
        };
        match offset {
            o if o & !1 == AGT => {
                let mut r = self.reload;
                set_half(&mut r);
                self.reload = r;
                self.anchor_count = r;
                self.anchor_tick = self.ticks_now().unwrap_or(0);
                self.rearm();
            }
            o if o & !1 == AGTCMA => {
                self.reanchor_with(|s| {
                    let mut v = s.cma;
                    set_half(&mut v);
                    s.cma = v;
                });
            }
            o if o & !1 == AGTCMB => {
                self.reanchor_with(|s| {
                    let mut v = s.cmb;
                    set_half(&mut v);
                    s.cmb = v;
                });
            }
            AGTCR => self.write_agtcr(value),
            AGTMR1 => self.reanchor_with(|s| s.agtmr1 = value & 0x7F),
            AGTMR2 => self.reanchor_with(|s| s.agtmr2 = value & 0x87),
            AGTIOSEL_ALT => self.agtiosel_alt = value & 0x13,
            AGTIOC => self.agtioc = value & 0xF5,
            AGTISR => self.agtisr = value & 0x04,
            AGTCMSR => self.reanchor_with(|s| s.agtcmsr = value & 0x77),
            AGTIOSEL => self.agtiosel = value & 0x13,
            _ => {
                crate::census_reg!("ra:RaAgt", offset, "write");
            }
        }
    }

    /// Freeze the count, apply `f` (which may change the count source or a
    /// compare), and re-arm from the frozen count.
    fn reanchor_with(&mut self, f: impl FnOnce(&mut Self)) {
        let count = self.count();
        f(self);
        self.anchor_count = count;
        self.anchor_tick = self.ticks_now().unwrap_or(0);
        self.rearm();
    }
}

impl Default for RaAgt {
    fn default() -> Self {
        Self::new(DEFAULT_CLOCK_HZ, DEFAULT_PCLKB_HZ, DEFAULT_ELC_EVENT)
    }
}

impl Peripheral for RaAgt {
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && self.armed.is_some()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn explicit_irqs_are_link_events(&self) -> bool {
        true
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> crate::PeripheralTickResult {
        if self.scheduler_mode() {
            return crate::PeripheralTickResult::default();
        }
        let mut events = Vec::new();
        while self.armed_cycle().is_some_and(|at| self.now_cycle() >= at) {
            events.extend(self.fire());
        }
        crate::PeripheralTickResult {
            explicit_irqs: (!events.is_empty()).then_some(events),
            ..Default::default()
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.read_reg(offset & 0xFF))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_reg(offset & 0xFF, value);
        Ok(())
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
        if event_token != self.generation || self.armed.is_none() {
            return crate::sched::EventResult::default();
        }
        let events = self.fire();
        // The drain does not harvest `take_scheduled_events` after an event,
        // so the next edge `fire` armed goes back as a reschedule of this
        // token, and the generation returns to it to match.
        let reschedule_delay = self.pending_events.pop().map(|(delay, _)| {
            self.generation = event_token;
            delay
        });
        self.pending_events.clear();
        crate::sched::EventResult {
            explicit_irqs: events,
            reschedule_delay,
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_node_reset(&mut self) {
        let clock = self.clock.take();
        *self = Self::new(self.clock_hz, self.pclkb_hz, self.elc_event);
        self.clock = clock;
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        #[allow(unused_mut)]
        let mut restored: RaAgt = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("RaAgt snapshot decode: {error}"))
        })?;
        restored.clock = self.clock.take();
        *self = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 MHz CPU so one 32.768-kHz tick is 30.5 cycles.
    const HZ: u64 = 1_000_000;

    fn agt_at(cycle: u64) -> (RaAgt, CycleClock) {
        let clock = CycleClock::default();
        clock.publish(cycle);
        let mut t = RaAgt::new(HZ, 8 * HZ, DEFAULT_ELC_EVENT);
        t.attach_cycle_clock(clock.clone());
        (t, clock)
    }

    fn w16(t: &mut RaAgt, off: u64, v: u16) {
        t.write(off, v as u8).unwrap();
        t.write(off + 1, (v >> 8) as u8).unwrap();
    }

    fn r16(t: &RaAgt, off: u64) -> u16 {
        u16::from(t.read(off).unwrap()) | (u16::from(t.read(off + 1).unwrap()) << 8)
    }

    /// LOCO count source, /1 (Zephyr `AGT_CLOCK_LOCO`, prescaler 0).
    fn loco(t: &mut RaAgt) {
        t.write(AGTMR1, 4 << 4).unwrap();
        t.write(AGTMR2, 0).unwrap();
    }

    /// Advance to `cycle`, delivering every armed edge on the way the way
    /// the build's time base does (legacy walk, or the scheduled event),
    /// and collect the events.
    fn walk_to(t: &mut RaAgt, clock: &CycleClock, cycle: u64) -> Vec<(u64, u32)> {
        let mut out = Vec::new();
        while let Some(at) = t.armed_cycle().filter(|&at| at <= cycle) {
            clock.publish(at);
            let events = if t.scheduler_mode() {
                t.fire()
            } else {
                t.tick_elapsed(1).explicit_irqs.unwrap_or_default()
            };
            assert!(!events.is_empty(), "armed edge at {at} delivered nothing");
            for e in events {
                out.push((at, e));
            }
        }
        clock.publish(cycle);
        out
    }

    #[test]
    fn reset_values_and_stopped_counter() {
        let (t, _c) = agt_at(0);
        assert_eq!(r16(&t, AGT), 0xFFFF);
        assert_eq!(r16(&t, AGTCMA), 0xFFFF);
        assert_eq!(t.read(AGTCR).unwrap(), 0);
        assert!(t.explicit_irqs_are_link_events());
    }

    #[test]
    fn counts_down_on_loco_and_tcstf_follows_tstart() {
        let (mut t, clock) = agt_at(0);
        loco(&mut t);
        w16(&mut t, AGT, 1000);
        t.write(AGTCR, TSTART).unwrap();
        assert_eq!(t.read(AGTCR).unwrap() & TCSTF, TCSTF);
        // 100 LOCO ticks at 1 MHz CPU = 3051.7 cycles.
        clock.publish(3052);
        assert_eq!(r16(&t, AGT), 900);
        // Stop freezes the count.
        t.write(AGTCR, 0).unwrap();
        assert_eq!(t.read(AGTCR).unwrap() & TCSTF, 0);
        clock.publish(1_000_000);
        assert_eq!(r16(&t, AGT), 900);
    }

    #[test]
    fn prescaler_divides_the_low_clock() {
        let (mut t, clock) = agt_at(0);
        t.write(AGTMR1, 4 << 4).unwrap();
        t.write(AGTMR2, 4).unwrap(); // 32768 / 16 = 2048 Hz
        t.write(AGTCR, TSTART).unwrap();
        clock.publish(HZ); // one second
        assert_eq!(r16(&t, AGT), 0xFFFF - 2048);
    }

    #[test]
    fn underflow_reloads_sets_tundf_and_fires_agti() {
        let (mut t, clock) = agt_at(0);
        loco(&mut t);
        w16(&mut t, AGT, 9); // period 10 ticks
        t.write(AGTCR, TSTART).unwrap();
        let end = t.cycle_of_tick(25, LOW_CLK_HZ);
        let ev = walk_to(&mut t, &clock, end);
        let at: Vec<u64> = ev.iter().map(|e| e.0).collect();
        assert_eq!(
            at,
            vec![
                t.cycle_of_tick(10, LOW_CLK_HZ),
                t.cycle_of_tick(20, LOW_CLK_HZ)
            ]
        );
        assert!(ev.iter().all(|e| e.1 == DEFAULT_ELC_EVENT));
        assert_eq!(t.read(AGTCR).unwrap() & TUNDF, TUNDF);
        // 25 ticks in: 5 after the second reload.
        assert_eq!(r16(&t, AGT), 4);
    }

    #[test]
    fn compare_a_fires_when_the_count_steps_onto_agtcma() {
        let (mut t, clock) = agt_at(0);
        loco(&mut t);
        w16(&mut t, AGTCMA, 0xFFFF - 50);
        t.write(AGTCMSR, TCMEA).unwrap();
        t.write(AGTCR, TSTART).unwrap();
        let end = t.cycle_of_tick(60, LOW_CLK_HZ);
        let ev = walk_to(&mut t, &clock, end);
        assert_eq!(
            ev,
            vec![(t.cycle_of_tick(50, LOW_CLK_HZ), DEFAULT_ELC_EVENT + 1)]
        );
        assert_eq!(t.read(AGTCR).unwrap() & TCMAF, TCMAF);
        // Disabled compare: no event.
        t.write(AGTCMSR, 0).unwrap();
        let cma = r16(&t, AGT) - 5;
        w16(&mut t, AGTCMA, cma);
        let end = t.cycle_of_tick(80, LOW_CLK_HZ);
        assert!(walk_to(&mut t, &clock, end).is_empty());
    }

    #[test]
    fn flags_are_write_zero_to_clear() {
        let (mut t, clock) = agt_at(0);
        loco(&mut t);
        w16(&mut t, AGT, 0);
        t.write(AGTCR, TSTART).unwrap();
        let end = t.cycle_of_tick(1, LOW_CLK_HZ);
        walk_to(&mut t, &clock, end);
        assert_eq!(t.read(AGTCR).unwrap() & TUNDF, TUNDF);
        // Zephyr's RMW of TCMAF=0 writes TUNDF back as 1: kept.
        let v = t.read(AGTCR).unwrap();
        t.write(AGTCR, v & !TCMAF).unwrap();
        assert_eq!(t.read(AGTCR).unwrap() & TUNDF, TUNDF);
        t.write(AGTCR, (v & !TUNDF) | TSTART).unwrap();
        assert_eq!(t.read(AGTCR).unwrap() & TUNDF, 0);
        // Writing 1 to a clear flag does not set it.
        t.write(AGTCR, TSTART | TCMAF | TCMBF | TEDGF).unwrap();
        assert_eq!(t.read(AGTCR).unwrap() & FLAGS, 0);
    }

    #[test]
    fn tstop_stops_and_presets_the_registers() {
        let (mut t, _c) = agt_at(0);
        loco(&mut t);
        w16(&mut t, AGT, 10);
        w16(&mut t, AGTCMA, 3);
        t.write(AGTCR, TSTART).unwrap();
        t.write(AGTCR, TSTOP).unwrap();
        assert_eq!(t.read(AGTCR).unwrap(), 0);
        assert_eq!(r16(&t, AGT), 0xFFFF);
        assert_eq!(r16(&t, AGTCMA), 0xFFFF);
        assert_eq!(r16(&t, AGTCMB), 0xFFFF);
        assert!(t.armed.is_none());
    }

    #[test]
    fn scheduler_mode_queues_the_armed_cycle() {
        let (mut t, _c) = agt_at(0);
        loco(&mut t);
        w16(&mut t, AGT, 99);
        t.take_scheduled_events();
        t.write(AGTCR, TSTART).unwrap();
        let events = t.take_scheduled_events();
        if t.scheduler_mode() {
            assert_eq!(
                events,
                vec![(t.cycle_of_tick(100, LOW_CLK_HZ), t.generation)]
            );
        } else {
            assert!(events.is_empty());
            assert!(t.legacy_tick_active());
        }
    }

    #[test]
    fn a_write_that_keeps_the_next_edge_queues_nothing_new() {
        let (mut t, clock) = agt_at(0);
        loco(&mut t);
        w16(&mut t, AGTCMA, 0xFFFF - 50);
        t.write(AGTCMSR, TCMEA).unwrap();
        t.write(AGTCR, TSTART).unwrap();
        t.take_scheduled_events();
        let token = t.generation;
        clock.publish(t.cycle_of_tick(10, LOW_CLK_HZ));
        // Zephyr's RMW flag clears and a compare-B write while B is off.
        for _ in 0..20 {
            let v = t.read(AGTCR).unwrap();
            t.write(AGTCR, v & !TCMAF).unwrap();
            w16(&mut t, AGTCMB, 0x1234);
        }
        assert!(t.take_scheduled_events().is_empty());
        assert_eq!(t.generation, token, "the queued event keeps its token");
        assert_eq!(t.armed_cycle(), Some(t.cycle_of_tick(50, LOW_CLK_HZ)));
        // Moving compare A does re-arm.
        w16(&mut t, AGTCMA, 0xFFFF - 30);
        assert_ne!(t.generation, token);
        assert_eq!(t.armed_cycle(), Some(t.cycle_of_tick(30, LOW_CLK_HZ)));
    }

    #[test]
    fn without_a_clock_nothing_counts_or_arms() {
        let mut t = RaAgt::default();
        t.write(AGTMR1, 4 << 4).unwrap();
        t.write(AGTCR, TSTART).unwrap();
        assert_eq!(r16(&t, AGT), 0xFFFF);
        assert!(t.armed.is_none());
        assert!(!t.legacy_tick_active());
    }
}
