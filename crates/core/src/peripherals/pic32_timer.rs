// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! PIC32MZ 16-bit timers (PIC32MZ EF data sheet DS60001320, "Timer1" and
//! "Timer2/3, Timer4/5, Timer6/7 and Timer8/9"; PIC32 Family Reference
//! Manual section 14). One timer = `TxCON` +0x00, `TMRx` +0x10, `PRx`
//! +0x20, each with the CLR/SET/INV aliases at +0x4/+0x8/+0xC.
//!
//! Modelled:
//! - with `TxCON.ON` set, `TMRx` counts up at the peripheral bus clock
//!   (`pbclk_hz`, PBCLK3 on the PIC32MZ) divided by the prescaler: Type A
//!   (Timer1) `TCKPS<5:4>` = 1, 8, 64, 256; Type B (Timer2-9) `TCKPS<6:4>`
//!   = 1, 2, 4, 8, 16, 32, 64, 256. Timer1's `TCS` selects the secondary
//!   oscillator (`sosc_hz`, 32.768 kHz) instead. The count is derived
//!   lazily from the bus cycle count at `clock_hz`;
//! - when `TMRx` equals `PRx` the next count clears it to 0 and sets the
//!   timer's EVIC flag (`irq`). A count above `PRx` runs to 0xFFFF and
//!   wraps first, without a flag;
//! - `TMRx`/`PRx` writes take effect at once; clearing `ON` freezes the
//!   count.
//!
//! NOT modelled: gated accumulation (`TGATE`), 32-bit timer pairs (`T32`),
//! the asynchronous external clock input (`TxCK`) of Type B timers, and
//! freeze-in-debug (`SIDL`/`FRZ`).

use crate::{CycleClock, Peripheral, SimResult, SimulationError};

const CON_ON: u32 = 1 << 15;
const CON_TCS: u32 = 1 << 1;

/// The PIC32MZ EF on the Curiosity 2.0 under NuttX: SYSCLK 200 MHz,
/// PBCLK3 50 MHz.
pub const DEFAULT_CLOCK_HZ: u64 = 200_000_000;
pub const DEFAULT_PBCLK_HZ: u64 = 50_000_000;
pub const DEFAULT_SOSC_HZ: u64 = 32_768;

/// One PIC32 timer.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Pic32Timer {
    clock_hz: u64,
    pbclk_hz: u64,
    sosc_hz: u64,
    /// Type A (Timer1) or Type B (Timer2-9) prescaler encoding.
    type_a: bool,
    irq: u32,
    con: u32,
    pr: u32,
    /// Count at `anchor_tick` (count-source ticks since cycle 0).
    anchor_count: u32,
    anchor_tick: u64,
    /// Count-source tick of the next period match.
    armed: Option<u64>,
    /// CPU cycle of the one live scheduler wake (see `RxCmt`).
    live_at: Option<u64>,
    generation: u32,
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl Pic32Timer {
    pub fn new(clock_hz: u64, pbclk_hz: u64, sosc_hz: u64, type_a: bool, irq: u32) -> Self {
        Self {
            clock_hz: clock_hz.max(1),
            pbclk_hz: pbclk_hz.max(1),
            sosc_hz: sosc_hz.max(1),
            type_a,
            irq,
            con: 0,
            pr: 0xFFFF,
            anchor_count: 0,
            anchor_tick: 0,
            armed: None,
            live_at: None,
            generation: 0,
            pending_events: Vec::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn running(&self) -> bool {
        self.con & CON_ON != 0
    }

    fn now_cycle(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn prescale(&self) -> u64 {
        if self.type_a {
            [1, 8, 64, 256][((self.con >> 4) & 3) as usize]
        } else {
            [1, 2, 4, 8, 16, 32, 64, 256][((self.con >> 4) & 7) as usize]
        }
    }

    fn source_hz(&self) -> u64 {
        let src = if self.type_a && self.con & CON_TCS != 0 {
            self.sosc_hz
        } else {
            self.pbclk_hz
        };
        (src / self.prescale()).max(1)
    }

    fn ticks_at(&self, cycle: u64) -> u64 {
        (u128::from(cycle) * u128::from(self.source_hz()) / u128::from(self.clock_hz)) as u64
    }

    /// First CPU cycle at which `ticks` count-source ticks have elapsed.
    fn cycle_of_tick(&self, ticks: u64) -> u64 {
        let hz = u128::from(self.source_hz());
        ((u128::from(ticks) * u128::from(self.clock_hz)).div_ceil(hz)) as u64
    }

    fn count_after(&self, n: u64) -> u32 {
        let period = u64::from(self.pr) + 1;
        let start = u64::from(self.anchor_count);
        if start <= u64::from(self.pr) {
            ((start + n) % period) as u32
        } else {
            let to_wrap = 0x1_0000 - start;
            if n < to_wrap {
                (start + n) as u32
            } else {
                ((n - to_wrap) % period) as u32
            }
        }
    }

    fn count(&self) -> u32 {
        if !self.running() || self.clock.is_none() {
            return self.anchor_count;
        }
        let n = self
            .ticks_at(self.now_cycle())
            .saturating_sub(self.anchor_tick);
        self.count_after(n)
    }

    /// Ticks from the anchor to the next period-match clear.
    fn ticks_to_match(&self) -> u64 {
        let start = u64::from(self.anchor_count);
        let pr = u64::from(self.pr);
        if start <= pr {
            pr + 1 - start
        } else {
            (0x1_0000 - start) + pr + 1
        }
    }

    fn token(&self) -> u32 {
        self.generation
    }

    fn rearm(&mut self) {
        self.armed = None;
        if !self.running() || self.clock.is_none() {
            return;
        }
        let tick = self.anchor_tick + self.ticks_to_match();
        self.armed = Some(tick);
        if self.scheduler_mode() {
            let now = self.now_cycle();
            let fire = self.cycle_of_tick(tick).max(now + 1);
            if self.live_at.is_some_and(|live| live <= fire) {
                return;
            }
            self.generation = self.generation.wrapping_add(1);
            self.live_at = Some(fire);
            self.pending_events.push((fire - now, self.token()));
        }
    }

    /// Freeze the count, apply `f`, re-anchor and re-arm.
    fn reanchor_with(&mut self, f: impl FnOnce(&mut Self)) {
        let count = self.count();
        f(self);
        self.anchor_count = count;
        self.anchor_tick = if self.clock.is_some() {
            self.ticks_at(self.now_cycle())
        } else {
            0
        };
        self.rearm();
    }

    /// Take the armed period match: the counter is 0 on that edge.
    fn fire(&mut self) -> Option<u32> {
        let tick = self.armed.take()?;
        self.anchor_count = 0;
        self.anchor_tick = tick;
        self.rearm();
        Some(self.irq)
    }

    fn armed_cycle(&self) -> Option<u64> {
        self.armed.map(|t| self.cycle_of_tick(t))
    }

    fn read_reg(&self, off: u64) -> u32 {
        match off {
            0x00 => self.con,
            0x10 => self.count(),
            0x20 => self.pr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, off: u64, v: u32) -> SimResult<()> {
        if off % 4 != 0 || off >= 0x30 {
            return Err(SimulationError::NotImplemented(format!(
                "PIC32 timer offset {off:#x}"
            )));
        }
        let reg = off & !0xF;
        let old = self.read_reg(reg);
        let new = match off & 0xC {
            0x0 => v,
            0x4 => old & !v,
            0x8 => old | v,
            _ => old ^ v,
        };
        match reg {
            0x00 => {
                let mask = if self.type_a { 0xB3B6 } else { 0xA0FA };
                self.reanchor_with(|t| t.con = new & mask);
            }
            0x10 => {
                self.reanchor_with(|_| {});
                self.anchor_count = new & 0xFFFF;
                self.rearm();
            }
            _ => self.reanchor_with(|t| t.pr = new & 0xFFFF),
        }
        Ok(())
    }
}

impl Default for Pic32Timer {
    fn default() -> Self {
        Self::new(DEFAULT_CLOCK_HZ, DEFAULT_PBCLK_HZ, DEFAULT_SOSC_HZ, true, 4)
    }
}

impl Peripheral for Pic32Timer {
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && self.armed.is_some()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> crate::PeripheralTickResult {
        if self.scheduler_mode() {
            return crate::PeripheralTickResult::default();
        }
        let mut irqs = Vec::new();
        while self.armed_cycle().is_some_and(|at| self.now_cycle() >= at) {
            irqs.extend(self.fire());
        }
        crate::PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_reg(offset & !3 & !0xC);
        let w = if offset & 0xC == 0 { w } else { 0 };
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset & 0xC != 0 {
            // CLR/SET/INV read as zero.
            return Ok(0);
        }
        Ok(self.read_reg(offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_reg(offset & !3, u32::from(value) << ((offset & 3) * 8))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset, value)
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
        if self.token() != event_token {
            // Superseded by an earlier wake.
            return crate::sched::EventResult::default();
        }
        self.live_at = None;
        let now = self.now_cycle();
        let Some(due) = self.armed_cycle() else {
            return crate::sched::EventResult::default();
        };
        if due > now {
            self.live_at = Some(due);
            return crate::sched::EventResult {
                reschedule_delay: Some(due - now),
                ..Default::default()
            };
        }
        let irq = self.fire();
        let reschedule_delay = self.pending_events.pop().map(|(delay, _)| {
            self.generation = event_token;
            delay
        });
        self.pending_events.clear();
        crate::sched::EventResult {
            explicit_irqs: irq.into_iter().collect(),
            reschedule_delay,
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_node_reset(&mut self) {
        let clock = self.clock.take();
        *self = Self::new(
            self.clock_hz,
            self.pbclk_hz,
            self.sosc_hz,
            self.type_a,
            self.irq,
        );
        self.clock = clock;
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Pic32Timer = serde_json::from_value(state).map_err(|error| {
            SimulationError::NotImplemented(format!("Pic32Timer snapshot decode: {error}"))
        })?;
        let clock = self.clock.take();
        *self = restored;
        self.clock = clock;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SYSCLK 200 MHz, PBCLK3 50 MHz: prescale 8 counts every 32 cycles.
    fn timer_at(cycle: u64) -> (Pic32Timer, CycleClock) {
        let clock = CycleClock::default();
        clock.publish(cycle);
        let mut t = Pic32Timer::default();
        t.attach_cycle_clock(clock.clone());
        (t, clock)
    }

    fn walk_to(t: &mut Pic32Timer, clock: &CycleClock, cycle: u64) -> Vec<u32> {
        let mut out = Vec::new();
        let mut now = clock.now();
        while now < cycle {
            now += 1;
            clock.publish(now);
            if !t.scheduler_mode() {
                out.extend(t.tick_elapsed(1).explicit_irqs.unwrap_or_default());
            }
        }
        out
    }

    #[test]
    fn nuttx_tick_setup_counts_at_pbclk_over_8_and_flags_on_period() {
        // NuttX up_timer_initialize: TCKPS = 8, PR = 62499, then CONSET ON.
        let (mut t, clock) = timer_at(0);
        t.write_u32(0x00, 1 << 4).unwrap();
        t.write_u32(0x10, 0).unwrap();
        t.write_u32(0x20, 62_499).unwrap();
        t.write_u32(0x08, CON_ON).unwrap();
        assert_eq!(t.read_u32(0x00).unwrap(), CON_ON | (1 << 4));
        clock.publish(32 * 100);
        assert_eq!(t.read_u32(0x10).unwrap(), 100);
        if !t.scheduler_mode() {
            // 10 ms period: 62500 ticks of 32 cycles.
            let irqs = walk_to(&mut t, &clock, 32 * 62_500);
            assert_eq!(irqs, vec![4]);
            assert_eq!(t.read_u32(0x10).unwrap(), 0);
        }
        assert_eq!(t.read_u32(0x04).unwrap(), 0, "aliases read as zero");
    }

    #[test]
    fn clearing_on_freezes_and_sosc_selects_32k() {
        let (mut t, clock) = timer_at(0);
        t.write_u32(0x00, CON_ON | CON_TCS).unwrap();
        // 32.768 kHz at 200 MHz: one tick per 6103.5 cycles.
        clock.publish(200_000_000 / 32_768 * 10 + 10);
        assert_eq!(t.read_u32(0x10).unwrap(), 10);
        t.write_u32(0x04, CON_ON).unwrap();
        clock.publish(200_000_000);
        assert_eq!(t.read_u32(0x10).unwrap(), 10);
        assert!(t.write_u32(0x30, 0).is_err());
    }

    #[test]
    fn type_b_prescaler_table() {
        let mut t = Pic32Timer::new(200_000_000, 50_000_000, 32_768, false, 9);
        t.write_u32(0x00, 3 << 4).unwrap();
        assert_eq!(t.prescale(), 8);
        t.write_u32(0x00, 7 << 4).unwrap();
        assert_eq!(t.prescale(), 256);
    }
}
