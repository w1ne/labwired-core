// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! i.MX RT500 OS event timer (`OSTIMER`, `0x4011_3000`), the Zephyr
//! `nxp,os-timer` system clock on the MIMXRT595-EVK.
//!
//! Source: the MCUXpresso SDK device header `MIMXRT595S_cm33.h`
//! (`OSTIMER_Type`: `EVTIMERL`/`H` @ 0x00/0x04, `CAPTURE_L`/`H` @ 0x08/0x0C,
//! `MATCH_L`/`H` @ 0x10/0x14, `OSEVENT_CTRL` @ 0x1C with `OSTIMER_INTRFLAG`
//! bit 0, `OSTIMER_INTENA` bit 1, `MATCH_WR_RDY` bit 2; `OS_EVENT_IRQn` 41)
//! and the SDK driver `fsl_ostimer.c` (the counter and the match registers
//! hold Gray code; the RT500 has no SYSCTL Gray decoder, so software converts).
//!
//! Modelled:
//! - the 42-bit free-running EVTIMER on its `tick_hz` clock (the 1 MHz LPOSC
//!   the Zephyr board selects), derived from the bus cycle count at
//!   `clock_hz`, 0 at reset, read as Gray code. Reading `EVTIMERL` latches
//!   the whole count, and `EVTIMERH` returns the latched upper bits, so the
//!   SDK's L-then-H read never tears;
//! - the match: `MATCH_L`/`MATCH_H` store Gray code; writing `MATCH_H` arms
//!   a match at the decoded count. `MATCH_WR_RDY` always reads 0 (the write
//!   has crossed into the timer domain at once). When the count reaches the
//!   match, `OSTIMER_INTRFLAG` sets (write 1 to clear) and, while
//!   `OSTIMER_INTENA` is set, the OS event IRQ is raised. Setting `INTENA`
//!   with the flag already set raises it too. With the event scheduler the
//!   match is a scheduled event; without it the legacy walk ticks this block
//!   only while a match is armed or an IRQ is owed.
//!
//! NOT modelled: `CAPTURE_L`/`H` (read 0), the per-core event routing (one
//! CPU view), deep-sleep wake-up, the 42-bit wrap (an armed match at or
//! behind the count never fires; that is ~51 days at 1 MHz), and any skew
//! when firmware re-clocks the core away from `clock_hz`.

use crate::{CycleClock, Peripheral, SimResult};
use std::cell::Cell;

const EVTIMERL: u64 = 0x00;
const EVTIMERH: u64 = 0x04;
const CAPTURE_L: u64 = 0x08;
const CAPTURE_H: u64 = 0x0C;
const MATCH_L: u64 = 0x10;
const MATCH_H: u64 = 0x14;
const OSEVENT_CTRL: u64 = 0x1C;

const INTRFLAG: u32 = 1 << 0;
const INTENA: u32 = 1 << 1;
/// EVTIMER and MATCH are 42 bits wide (`fsl_ostimer.c`).
const COUNT_MASK: u64 = (1 << 42) - 1;
/// The upper 10 bits live in `EVTIMERH` / `MATCH_H`.
const HIGH_MASK: u32 = 0x3FF;

/// The OS timer clock Zephyr's `mimxrt595_evk` uses (LPOSC, 1 MHz).
pub const DEFAULT_TICK_HZ: u64 = 1_000_000;
/// MIMXRT595S `cpu_hz` (48 MHz FRO DIV), the rate bus cycles advance at.
pub const DEFAULT_CLOCK_HZ: u64 = 48_000_000;

/// Event token for "raise the IRQ now" (set `INTENA` over a pending flag).
const RAISE_TOKEN: u32 = u32::MAX;

fn to_gray(v: u64) -> u64 {
    v ^ (v >> 1)
}

fn from_gray(mut g: u64) -> u64 {
    let mut v = 0;
    while g != 0 {
        v ^= g;
        g >>= 1;
    }
    v
}

/// RT500 OS event timer (see the module docs).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Rt5xxOsTimer {
    clock_hz: u64,
    tick_hz: u64,
    match_l: u32,
    match_h: u32,
    ctrl: u32,
    /// The Gray count `EVTIMERL` latched; `EVTIMERH` returns its upper bits.
    latched: Cell<u64>,
    /// Bumped on every match arm; a scheduled event whose token no longer
    /// matches was superseded and is dropped.
    generation: u32,
    /// CPU cycle at which the armed match fires; `None` once it fired.
    armed_cycle: Option<u64>,
    /// Legacy walk only: an IRQ owed by an `INTENA` write over a set flag.
    owed_irq: bool,
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl Rt5xxOsTimer {
    pub fn new(clock_hz: u64, tick_hz: u64) -> Self {
        Self {
            clock_hz: clock_hz.max(1),
            tick_hz: tick_hz.max(1),
            match_l: 0,
            match_h: 0,
            ctrl: 0,
            latched: Cell::new(0),
            generation: 0,
            armed_cycle: None,
            owed_irq: false,
            pending_events: Vec::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now_cycle(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    /// Timer ticks elapsed at CPU cycle `cycle`.
    fn ticks_at(&self, cycle: u64) -> u64 {
        (u128::from(cycle) * u128::from(self.tick_hz) / u128::from(self.clock_hz)) as u64
    }

    /// First CPU cycle at which `ticks` timer ticks have elapsed.
    fn cycle_of_tick(&self, ticks: u64) -> u64 {
        let num = u128::from(ticks) * u128::from(self.clock_hz);
        num.div_ceil(u128::from(self.tick_hz)) as u64
    }

    /// The EVTIMER count now, in binary.
    pub fn count(&self) -> u64 {
        self.ticks_at(self.now_cycle()) & COUNT_MASK
    }

    /// Arm the match at the decoded `MATCH_H:MATCH_L`.
    fn arm_match(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.armed_cycle = None;
        if self.clock.is_none() {
            // No time base: the count never moves, so nothing can match.
            return;
        }
        let target =
            from_gray((u64::from(self.match_h & HIGH_MASK) << 32) | u64::from(self.match_l));
        let now = self.now_cycle();
        if target <= self.ticks_at(now) {
            return;
        }
        let fire = self.cycle_of_tick(target).max(now + 1);
        self.armed_cycle = Some(fire);
        if self.scheduler_mode() {
            self.pending_events.push((fire - now, self.generation));
        }
    }

    /// The armed match fired: latch the flag; the IRQ follows `INTENA`.
    fn fire(&mut self) -> bool {
        self.armed_cycle = None;
        self.ctrl |= INTRFLAG;
        self.ctrl & INTENA != 0
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            EVTIMERL => {
                let gray = to_gray(self.count());
                self.latched.set(gray);
                gray as u32
            }
            EVTIMERH => (self.latched.get() >> 32) as u32 & HIGH_MASK,
            CAPTURE_L | CAPTURE_H => 0,
            MATCH_L => self.match_l,
            MATCH_H => self.match_h,
            // MATCH_WR_RDY (bit 2) is never busy.
            OSEVENT_CTRL => self.ctrl,
            _ => {
                crate::census_reg!("lpc:Rt5xxOsTimer", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            MATCH_L => self.match_l = value,
            MATCH_H => {
                self.match_h = value & HIGH_MASK;
                self.arm_match();
            }
            OSEVENT_CTRL => {
                let enabling = value & INTENA != 0 && self.ctrl & INTENA == 0;
                self.ctrl = (self.ctrl & INTRFLAG & !(value & INTRFLAG)) | (value & INTENA);
                if enabling && self.ctrl & INTRFLAG != 0 {
                    if self.scheduler_mode() {
                        self.pending_events.push((1, RAISE_TOKEN));
                    } else {
                        self.owed_irq = true;
                    }
                }
            }
            EVTIMERL | EVTIMERH | CAPTURE_L | CAPTURE_H => {}
            _ => {
                crate::census_reg!("lpc:Rt5xxOsTimer", offset, "write");
            }
        }
    }
}

impl Default for Rt5xxOsTimer {
    fn default() -> Self {
        Self::new(DEFAULT_CLOCK_HZ, DEFAULT_TICK_HZ)
    }
}

impl Peripheral for Rt5xxOsTimer {
    /// The count is read lazily from the bus cycle clock; only an armed match
    /// or an owed IRQ needs the walk, and only when there is no scheduler.
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && (self.armed_cycle.is_some() || self.owed_irq)
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
        let mut irq = std::mem::take(&mut self.owed_irq) && self.ctrl & INTENA != 0;
        if !self.scheduler_mode() && self.armed_cycle.is_some_and(|at| self.now_cycle() >= at) {
            irq |= self.fire();
        }
        crate::PeripheralTickResult {
            irq,
            ..Default::default()
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_reg(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        // OSEVENT_CTRL's flag is write-1-to-clear: a byte write must not
        // write the flag back from a read-modify-write.
        let base = if aligned == OSEVENT_CTRL {
            self.ctrl & !INTRFLAG
        } else {
            self.read_reg(aligned)
        };
        let word = (base & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_reg(aligned, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
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
        let raise = if event_token == RAISE_TOKEN {
            self.ctrl & (INTRFLAG | INTENA) == INTRFLAG | INTENA
        } else if event_token == self.generation && self.armed_cycle.is_some() {
            self.fire()
        } else {
            false
        };
        crate::sched::EventResult {
            raise_own_irq: raise,
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_node_reset(&mut self) {
        let clock = self.clock.take();
        *self = Self::new(self.clock_hz, self.tick_hz);
        self.clock = clock;
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let clock = self.clock.take();
        *self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Rt5xxOsTimer snapshot decode: {error}"))
        })?;
        self.clock = clock;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize Rt5xxOsTimer")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let clock = self.clock.take();
        *self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "Rt5xxOsTimer runtime snapshot decode: {error}"
            ))
        })?;
        self.clock = clock;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer_at(cycle: u64) -> (Rt5xxOsTimer, CycleClock) {
        let clock = CycleClock::default();
        clock.publish(cycle);
        let mut t = Rt5xxOsTimer::default();
        t.attach_cycle_clock(clock.clone());
        (t, clock)
    }

    /// The SDK's `OSTIMER_GetCurrentTimerValue`: L then H, Gray-decoded.
    fn read_count(t: &Rt5xxOsTimer) -> u64 {
        let l = u64::from(t.read_u32(EVTIMERL).unwrap());
        let h = u64::from(t.read_u32(EVTIMERH).unwrap());
        from_gray((h << 32) | l)
    }

    fn set_match(t: &mut Rt5xxOsTimer, count: u64) {
        let g = to_gray(count);
        t.write_u32(MATCH_L, g as u32).unwrap();
        t.write_u32(MATCH_H, (g >> 32) as u32).unwrap();
    }

    #[test]
    fn gray_round_trips() {
        for v in [0u64, 1, 2, 3, 1000, 0x3_FFFF_FFFF, COUNT_MASK] {
            assert_eq!(from_gray(to_gray(v)), v);
        }
        assert_eq!(to_gray(2), 3);
    }

    #[test]
    fn counts_1mhz_ticks_of_the_cpu_clock_in_gray() {
        let (t, clock) = timer_at(0);
        assert_eq!(read_count(&t), 0);
        clock.publish(DEFAULT_CLOCK_HZ);
        assert_eq!(read_count(&t), 1_000_000);
        // The raw register is Gray code, not binary.
        assert_eq!(u64::from(t.read_u32(EVTIMERL).unwrap()), to_gray(1_000_000));
        // 48 cycles per tick.
        clock.publish(DEFAULT_CLOCK_HZ + 47);
        assert_eq!(read_count(&t), 1_000_000);
        clock.publish(DEFAULT_CLOCK_HZ + 48);
        assert_eq!(read_count(&t), 1_000_001);
    }

    #[test]
    fn evtimerl_latches_the_high_word() {
        // Just below a 2^32 carry.
        let (t, clock) = timer_at(((1u64 << 32) - 1) * 48);
        let l = t.read_u32(EVTIMERL).unwrap();
        clock.publish(clock.now() + 48 * 10);
        let h = t.read_u32(EVTIMERH).unwrap();
        assert_eq!(
            from_gray((u64::from(h) << 32) | u64::from(l)),
            (1 << 32) - 1
        );
    }

    #[test]
    fn match_fires_flag_and_irq_at_the_decoded_count() {
        let (mut t, clock) = timer_at(DEFAULT_CLOCK_HZ); // count 1 000 000
        t.write_u32(OSEVENT_CTRL, INTENA).unwrap();
        set_match(&mut t, 1_000_100);
        assert_eq!(t.read_u32(OSEVENT_CTRL).unwrap() & 0x4, 0, "WR_RDY idle");
        assert_eq!(t.armed_cycle, Some(DEFAULT_CLOCK_HZ + 100 * 48));
        let events = t.take_scheduled_events();
        if t.scheduler_mode() {
            assert_eq!(events, vec![(100 * 48, t.generation)]);
        } else {
            assert!(events.is_empty());
        }
        clock.publish(DEFAULT_CLOCK_HZ + 100 * 48 - 1);
        if !t.scheduler_mode() {
            assert!(!t.tick().irq);
        }
        clock.publish(DEFAULT_CLOCK_HZ + 100 * 48);
        let irq = if t.scheduler_mode() {
            let gen = t.generation;
            let mut sched = crate::sched::EventScheduler::new();
            let mut bus = crate::bus::SystemBus::new();
            t.on_event(gen, &mut sched, &mut bus).raise_own_irq
        } else {
            t.tick().irq
        };
        assert!(irq);
        assert_eq!(t.read_u32(OSEVENT_CTRL).unwrap(), INTENA | INTRFLAG);
        // W1C.
        t.write_u32(OSEVENT_CTRL, INTENA | INTRFLAG).unwrap();
        assert_eq!(t.read_u32(OSEVENT_CTRL).unwrap(), INTENA);
    }

    #[test]
    fn match_behind_the_count_is_not_armed() {
        let (mut t, _clock) = timer_at(DEFAULT_CLOCK_HZ);
        set_match(&mut t, 999_999);
        assert_eq!(t.armed_cycle, None);
        assert!(t.take_scheduled_events().is_empty());
    }

    #[test]
    fn rearming_supersedes_the_old_event() {
        let (mut t, _clock) = timer_at(0);
        set_match(&mut t, 10);
        let old = t.generation;
        set_match(&mut t, 20);
        assert_ne!(old, t.generation);
        let mut sched = crate::sched::EventScheduler::new();
        let mut bus = crate::bus::SystemBus::new();
        assert!(!t.on_event(old, &mut sched, &mut bus).raise_own_irq);
        assert_eq!(t.read_u32(OSEVENT_CTRL).unwrap() & INTRFLAG, 0);
    }

    #[test]
    fn enabling_over_a_set_flag_raises() {
        let (mut t, clock) = timer_at(0);
        set_match(&mut t, 1);
        clock.publish(48);
        // Fired with INTENA clear: flag only.
        let irq = if t.scheduler_mode() {
            let gen = t.generation;
            let mut sched = crate::sched::EventScheduler::new();
            let mut bus = crate::bus::SystemBus::new();
            t.on_event(gen, &mut sched, &mut bus).raise_own_irq
        } else {
            t.tick().irq
        };
        assert!(!irq);
        assert_eq!(t.read_u32(OSEVENT_CTRL).unwrap(), INTRFLAG);
        t.take_scheduled_events();
        t.write_u32(OSEVENT_CTRL, INTENA).unwrap();
        let irq = if t.scheduler_mode() {
            assert_eq!(t.take_scheduled_events(), vec![(1, RAISE_TOKEN)]);
            let mut sched = crate::sched::EventScheduler::new();
            let mut bus = crate::bus::SystemBus::new();
            t.on_event(RAISE_TOKEN, &mut sched, &mut bus).raise_own_irq
        } else {
            t.tick().irq
        };
        assert!(irq);
    }

    #[test]
    fn byte_write_does_not_clear_the_flag() {
        let (mut t, _clock) = timer_at(0);
        t.ctrl = INTRFLAG;
        t.write(OSEVENT_CTRL, INTENA as u8).unwrap();
        assert_eq!(t.read_u32(OSEVENT_CTRL).unwrap(), INTRFLAG | INTENA);
    }
}
