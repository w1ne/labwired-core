// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 RTC — real-time clock and alarm (datasheet §4.8, base `0x4005C000`).
//!
//! Unlike the counter-style RTCs on most Cortex-M parts, this one keeps a
//! *calendar*: year / month / day / day-of-week / hour / minute / second in
//! packed BCD-free binary fields. Twelve registers: `CLKDIV_M1`, `SETUP_0`,
//! `SETUP_1`, `CTRL`, `IRQ_SETUP_0`, `IRQ_SETUP_1`, `RTC_1`, `RTC_0`, `INTR`,
//! `INTE`, `INTF`, `INTS`. Offsets, field positions and `RTC_IRQ` (NVIC 25) are
//! taken from the vendored SVD (`tests/fixtures/real_world/rp2040.svd`).
//!
//! ## What silicon does, and what this models
//!
//! * **The `RTC_1`-before-`RTC_0` read latch.** This is the trap firmware
//!   actually falls into on real hardware, so it is modelled exactly: reading
//!   `RTC_1` (the date) latches the *whole* date+time pair, and `RTC_0` returns
//!   the latched time. Firmware that reads `RTC_0` first gets whatever was
//!   latched last — a stale time, and a torn one if it straddles a rollover.
//!   The bus read path is `&self`, so the latch lives in a `Cell` (the same
//!   shape the PL022 SPI model uses for its read-to-drain FIFO).
//! * **`CTRL.LOAD` commits `SETUP_0`/`SETUP_1`** into the live calendar and
//!   restarts the sub-second divider; `CTRL.RTC_ENABLE` starts it and
//!   `RTC_ACTIVE` follows, which is what makes `pico-sdk`'s
//!   `rtc_set_datetime()` poll terminate.
//! * **`CLKDIV_M1` really divides.** A second elapses every `CLKDIV_M1 + 1`
//!   ticks, so firmware that programs 46874 (the 46875 Hz `clk_rtc` default)
//!   gets a 46875-times-slower calendar than one that programs 0. The rate is
//!   arbitrary in absolute terms — the simulator has no wall clock, exactly as
//!   [`crate::peripherals::rp2040::timer`] documents — but the ratio is real.
//! * **Calendar arithmetic**, including the RP2040's leap rule: the hardware
//!   tests `year % 4 == 0` only, which is why `CTRL.FORCE_NOTLEAPYEAR` exists
//!   for the century years the simple rule gets wrong. Both are modelled, so
//!   1900 is a leap year here unless firmware sets that bit — because that is
//!   what the silicon does.
//! * **The alarm.** `IRQ_SETUP_0`/`IRQ_SETUP_1` carry a target plus a per-field
//!   enable (`DAY_ENA`, `MONTH_ENA`, `YEAR_ENA`, `SEC_ENA`, `MIN_ENA`,
//!   `HOUR_ENA`, `DOTW_ENA`). With `MATCH_ENA` set, `MATCH_ACTIVE` and
//!   `INTR.RTC` assert while every *enabled* field matches the live calendar.
//!   `INTR` is a read-only level, not a latch — there is nothing to write-clear,
//!   which is why `pico-sdk`'s alarm ISR calls `rtc_disable_alarm()`.
//!   `RTC_IRQ` is held while `(INTR | INTF) & INTE`, matching the timer, PWM
//!   and ADC models.
//!
//! ## Deliberately not modelled
//!
//! * **The enable/load synchronisers.** On silicon `RTC_ACTIVE` lags
//!   `RTC_ENABLE` by a few `clk_rtc` edges, and a `SETUP` write needs the RTC
//!   stopped. Here `RTC_ACTIVE` follows `RTC_ENABLE` immediately and a `LOAD`
//!   always commits. Firmware that polls (every driver does) is unaffected;
//!   firmware relying on the delay to sequence something else would not see it.
//! * **Day-of-week derivation.** `DOTW` is stored and incremented modulo 7, not
//!   computed from the date — the hardware does not compute it either, which is
//!   why `SETUP_1.DOTW` must be programmed by the caller. A wrong `DOTW` in
//!   equals a wrong `DOTW` out, as on silicon.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;

// Register offsets (relative to the RTC base) — SVD-verified.
const CLKDIV_M1: u64 = 0x00;
const SETUP_0: u64 = 0x04;
const SETUP_1: u64 = 0x08;
const CTRL: u64 = 0x0C;
const IRQ_SETUP_0: u64 = 0x10;
const IRQ_SETUP_1: u64 = 0x14;
const RTC_1: u64 = 0x18;
const RTC_0: u64 = 0x1C;
const INTR: u64 = 0x20;
const INTE: u64 = 0x24;
const INTF: u64 = 0x28;
const INTS: u64 = 0x2C;

// CTRL fields.
const CTRL_RTC_ENABLE: u32 = 1 << 0;
const CTRL_RTC_ACTIVE: u32 = 1 << 1; // read-only
const CTRL_LOAD: u32 = 1 << 4; // write-only
const CTRL_FORCE_NOTLEAPYEAR: u32 = 1 << 8;

// Date word (SETUP_0 / RTC_1 / IRQ_SETUP_0): YEAR[23:12] MONTH[11:8] DAY[4:0].
const DAY_SHIFT: u32 = 0;
const DAY_MASK: u32 = 0x1F;
const MONTH_SHIFT: u32 = 8;
const MONTH_MASK: u32 = 0xF;
const YEAR_SHIFT: u32 = 12;
const YEAR_MASK: u32 = 0xFFF;

// Time word (SETUP_1 / RTC_0 / IRQ_SETUP_1): DOTW[26:24] HOUR[20:16]
// MIN[13:8] SEC[5:0].
const SEC_SHIFT: u32 = 0;
const SEC_MASK: u32 = 0x3F;
const MIN_SHIFT: u32 = 8;
const MIN_MASK: u32 = 0x3F;
const HOUR_SHIFT: u32 = 16;
const HOUR_MASK: u32 = 0x1F;
const DOTW_SHIFT: u32 = 24;
const DOTW_MASK: u32 = 0x7;

// IRQ_SETUP_0 enables (SVD: DAY_ENA[24], MONTH_ENA[25], YEAR_ENA[26],
// MATCH_ENA[28], MATCH_ACTIVE[29]).
const DAY_ENA: u32 = 1 << 24;
const MONTH_ENA: u32 = 1 << 25;
const YEAR_ENA: u32 = 1 << 26;
const MATCH_ENA: u32 = 1 << 28;
const MATCH_ACTIVE: u32 = 1 << 29; // read-only

// IRQ_SETUP_1 enables (SVD: SEC_ENA[28], MIN_ENA[29], HOUR_ENA[30],
// DOTW_ENA[31]).
const SEC_ENA: u32 = 1 << 28;
const MIN_ENA: u32 = 1 << 29;
const HOUR_ENA: u32 = 1 << 30;
const DOTW_ENA: u32 = 1 << 31;

/// `RTC_IRQ` (SVD interrupt table).
const RTC_IRQ: u32 = 25;

/// The live calendar, unpacked.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Calendar {
    year: u32,
    month: u32,
    day: u32,
    dotw: u32,
    hour: u32,
    min: u32,
    sec: u32,
}

impl Calendar {
    fn from_words(date: u32, time: u32) -> Self {
        Self {
            year: (date >> YEAR_SHIFT) & YEAR_MASK,
            month: (date >> MONTH_SHIFT) & MONTH_MASK,
            day: (date >> DAY_SHIFT) & DAY_MASK,
            dotw: (time >> DOTW_SHIFT) & DOTW_MASK,
            hour: (time >> HOUR_SHIFT) & HOUR_MASK,
            min: (time >> MIN_SHIFT) & MIN_MASK,
            sec: (time >> SEC_SHIFT) & SEC_MASK,
        }
    }

    fn date_word(&self) -> u32 {
        ((self.year & YEAR_MASK) << YEAR_SHIFT)
            | ((self.month & MONTH_MASK) << MONTH_SHIFT)
            | ((self.day & DAY_MASK) << DAY_SHIFT)
    }

    fn time_word(&self) -> u32 {
        ((self.dotw & DOTW_MASK) << DOTW_SHIFT)
            | ((self.hour & HOUR_MASK) << HOUR_SHIFT)
            | ((self.min & MIN_MASK) << MIN_SHIFT)
            | ((self.sec & SEC_MASK) << SEC_SHIFT)
    }

    /// Days in `month`, using the RP2040's leap rule (`year % 4 == 0`, unless
    /// `FORCE_NOTLEAPYEAR` overrides it). An out-of-range month reports 31 so a
    /// half-programmed calendar still advances instead of stalling.
    fn days_in_month(&self, force_notleapyear: bool) -> u32 {
        match self.month {
            2 => {
                if self.year % 4 == 0 && !force_notleapyear {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }

    /// Advance by one second, carrying through the calendar.
    fn advance_second(&mut self, force_notleapyear: bool) {
        self.sec += 1;
        if self.sec < 60 {
            return;
        }
        self.sec = 0;
        self.min += 1;
        if self.min < 60 {
            return;
        }
        self.min = 0;
        self.hour += 1;
        if self.hour < 24 {
            return;
        }
        self.hour = 0;
        self.dotw = (self.dotw + 1) % 7;
        self.day += 1;
        if self.day <= self.days_in_month(force_notleapyear) && self.day != 0 {
            return;
        }
        self.day = 1;
        self.month += 1;
        if self.month <= 12 && self.month != 0 {
            return;
        }
        self.month = 1;
        self.year = (self.year + 1) & YEAR_MASK;
    }
}

#[derive(Debug)]
pub struct Rp2040Rtc {
    /// `CLKDIV_M1` — ticks per second, minus one.
    clkdiv_m1: u32,
    /// Sub-second divider phase.
    div_count: u32,
    setup_0: u32,
    setup_1: u32,
    enabled: bool,
    force_notleapyear: bool,
    now: Calendar,
    /// `IRQ_SETUP_0`/`IRQ_SETUP_1` minus their read-only `MATCH_ACTIVE` bit.
    irq_setup_0: u32,
    irq_setup_1: u32,
    inte: u32,
    intf: u32,
    /// The `RTC_1` read latch: `(date, time)` captured when `RTC_1` is read.
    /// `RTC_0` returns the time half of this, never the live value.
    latch: Cell<(u32, u32)>,
    /// Bus-published cycle clock (event-scheduler). When present the model is
    /// walk-independent: calendar advance rides scheduled events.
    clock: Option<CycleClock>,
    /// Bumped each arm so stale calendar events die on arrival.
    arm_seq: u32,
}

impl Default for Rp2040Rtc {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040Rtc {
    pub fn new() -> Self {
        Self {
            clkdiv_m1: 0,
            div_count: 0,
            setup_0: 0,
            setup_1: 0,
            enabled: false,
            force_notleapyear: false,
            now: Calendar::default(),
            irq_setup_0: 0,
            irq_setup_1: 0,
            inte: 0,
            intf: 0,
            latch: Cell::new((0, 0)),
            clock: None,
            arm_seq: 0,
        }
    }

    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    fn needs_scheduler_wake(&self) -> bool {
        self.enabled || self.ints() != 0
    }

    /// One peripheral-tick of calendar advance + level IRQ vector.
    fn step_one(&mut self) -> PeripheralTickResult {
        if self.enabled {
            if self.div_count >= self.clkdiv_m1 {
                self.div_count = 0;
                self.now.advance_second(self.force_notleapyear);
            } else {
                self.div_count += 1;
            }
        }
        let explicit_irqs = (self.ints() != 0).then(|| vec![RTC_IRQ]);
        PeripheralTickResult {
            explicit_irqs,
            ..Default::default()
        }
    }

    /// `MATCH_ACTIVE` / `INTR.RTC`: every *enabled* alarm field matches.
    fn match_active(&self) -> bool {
        if self.irq_setup_0 & MATCH_ENA == 0 {
            return false;
        }
        let target_date = Calendar::from_words(self.irq_setup_0, self.irq_setup_1);
        let checks = [
            (
                self.irq_setup_0 & DAY_ENA != 0,
                target_date.day,
                self.now.day,
            ),
            (
                self.irq_setup_0 & MONTH_ENA != 0,
                target_date.month,
                self.now.month,
            ),
            (
                self.irq_setup_0 & YEAR_ENA != 0,
                target_date.year,
                self.now.year,
            ),
            (
                self.irq_setup_1 & SEC_ENA != 0,
                target_date.sec,
                self.now.sec,
            ),
            (
                self.irq_setup_1 & MIN_ENA != 0,
                target_date.min,
                self.now.min,
            ),
            (
                self.irq_setup_1 & HOUR_ENA != 0,
                target_date.hour,
                self.now.hour,
            ),
            (
                self.irq_setup_1 & DOTW_ENA != 0,
                target_date.dotw,
                self.now.dotw,
            ),
        ];
        checks
            .iter()
            .all(|(enabled, target, live)| !enabled || target == live)
    }

    fn intr(&self) -> u32 {
        u32::from(self.match_active())
    }

    fn ints(&self) -> u32 {
        (self.intr() | self.intf) & self.inte & 0x1
    }
}

impl Peripheral for Rp2040Rtc {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            CLKDIV_M1 => self.clkdiv_m1,
            SETUP_0 => self.setup_0,
            SETUP_1 => self.setup_1,
            CTRL => {
                (if self.enabled { CTRL_RTC_ENABLE } else { 0 })
                    | (if self.enabled { CTRL_RTC_ACTIVE } else { 0 })
                    | (if self.force_notleapyear {
                        CTRL_FORCE_NOTLEAPYEAR
                    } else {
                        0
                    })
            }
            IRQ_SETUP_0 => self.irq_setup_0 | if self.match_active() { MATCH_ACTIVE } else { 0 },
            IRQ_SETUP_1 => self.irq_setup_1,
            // Reading RTC_1 latches the whole date+time pair; RTC_0 then reads
            // the latched time. Reading RTC_0 first returns the previous latch.
            RTC_1 => {
                let pair = (self.now.date_word(), self.now.time_word());
                self.latch.set(pair);
                pair.0
            }
            RTC_0 => self.latch.get().1,
            INTR => self.intr(),
            INTE => self.inte,
            INTF => self.intf,
            INTS => self.ints(),
            _ => {
                crate::census_reg!("rp2040.rtc:Rp2040Rtc", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            CLKDIV_M1 => {
                self.clkdiv_m1 = value & 0xFFFF;
                self.div_count = 0;
            }
            SETUP_0 => {
                self.setup_0 = value
                    & ((YEAR_MASK << YEAR_SHIFT)
                        | (MONTH_MASK << MONTH_SHIFT)
                        | (DAY_MASK << DAY_SHIFT))
            }
            SETUP_1 => {
                self.setup_1 = value
                    & ((DOTW_MASK << DOTW_SHIFT)
                        | (HOUR_MASK << HOUR_SHIFT)
                        | (MIN_MASK << MIN_SHIFT)
                        | (SEC_MASK << SEC_SHIFT))
            }
            CTRL => {
                self.force_notleapyear = value & CTRL_FORCE_NOTLEAPYEAR != 0;
                self.enabled = value & CTRL_RTC_ENABLE != 0;
                // LOAD is write-only and self-clearing: it commits the staged
                // SETUP words and restarts the sub-second divider.
                if value & CTRL_LOAD != 0 {
                    self.now = Calendar::from_words(self.setup_0, self.setup_1);
                    self.div_count = 0;
                }
            }
            // MATCH_ACTIVE is read-only.
            IRQ_SETUP_0 => self.irq_setup_0 = value & !MATCH_ACTIVE,
            IRQ_SETUP_1 => self.irq_setup_1 = value,
            // RTC_1 / RTC_0 / INTR / INTS are read-only.
            RTC_1 | RTC_0 | INTR | INTS => {}
            INTE => self.inte = value & 0x1,
            INTF => self.intf = value & 0x1,
            _ => {
                crate::census_reg!("rp2040.rtc:Rp2040Rtc", offset, "write");
            }
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        if aligned == RTC_1 {
            return Ok(()); // read-only, and reading it here would re-latch
        }
        let shift = (offset & 0x3) * 8;
        let cur = self.read_u32(aligned)?;
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Legacy / feature-off path. Scheduler mode skips the walk.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.step_one()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() || !self.needs_scheduler_wake() {
            return Vec::new();
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        vec![(0, self.arm_seq)]
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != self.arm_seq {
            return crate::sched::EventResult::default();
        }
        let res = self.step_one();
        crate::sched::EventResult {
            explicit_irqs: res.explicit_irqs.unwrap_or_default(),
            reschedule_delay: self.needs_scheduler_wake().then_some(1),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: u32, month: u32, day: u32) -> u32 {
        (year << YEAR_SHIFT) | (month << MONTH_SHIFT) | day
    }

    fn time(dotw: u32, hour: u32, min: u32, sec: u32) -> u32 {
        (dotw << DOTW_SHIFT) | (hour << HOUR_SHIFT) | (min << MIN_SHIFT) | sec
    }

    /// Load a calendar and start the clock at one second per tick.
    fn running(d: u32, t: u32) -> Rp2040Rtc {
        let mut r = Rp2040Rtc::new();
        r.write_u32(CLKDIV_M1, 0).unwrap();
        r.write_u32(SETUP_0, d).unwrap();
        r.write_u32(SETUP_1, t).unwrap();
        r.write_u32(CTRL, CTRL_LOAD).unwrap();
        r.write_u32(CTRL, CTRL_RTC_ENABLE).unwrap();
        r
    }

    /// Read the live pair in the datasheet's order.
    fn read_pair(r: &Rp2040Rtc) -> (u32, u32) {
        let d = r.read_u32(RTC_1).unwrap();
        (d, r.read_u32(RTC_0).unwrap())
    }

    /// Reset state matches the SVD (all-zero) and RTC_ACTIVE follows enable.
    #[test]
    fn reset_state_and_active_follows_enable() {
        let mut r = Rp2040Rtc::new();
        assert_eq!(r.read_u32(CTRL).unwrap(), 0);
        r.write_u32(CTRL, CTRL_RTC_ENABLE).unwrap();
        assert_ne!(r.read_u32(CTRL).unwrap() & CTRL_RTC_ACTIVE, 0);
        r.write_u32(CTRL, 0).unwrap();
        assert_eq!(r.read_u32(CTRL).unwrap() & CTRL_RTC_ACTIVE, 0);
    }

    /// CTRL.LOAD commits the staged SETUP words, and only then.
    #[test]
    fn load_commits_the_setup_words() {
        let mut r = Rp2040Rtc::new();
        r.write_u32(SETUP_0, date(2026, 7, 27)).unwrap();
        r.write_u32(SETUP_1, time(1, 12, 34, 56)).unwrap();
        assert_eq!(read_pair(&r), (0, 0), "not loaded yet");
        r.write_u32(CTRL, CTRL_LOAD).unwrap();
        assert_eq!(read_pair(&r), (date(2026, 7, 27), time(1, 12, 34, 56)));
        // LOAD is write-only and never reads back.
        assert_eq!(r.read_u32(CTRL).unwrap() & CTRL_LOAD, 0);
    }

    /// A disabled RTC holds its calendar.
    #[test]
    fn disabled_rtc_does_not_advance() {
        let mut r = Rp2040Rtc::new();
        r.write_u32(SETUP_1, time(0, 0, 0, 10)).unwrap();
        r.write_u32(CTRL, CTRL_LOAD).unwrap();
        for _ in 0..100 {
            r.tick();
        }
        assert_eq!(read_pair(&r).1, time(0, 0, 0, 10));
    }

    /// Seconds advance, and CLKDIV_M1 really sets the ticks-per-second ratio.
    #[test]
    fn clkdiv_sets_the_seconds_rate() {
        let mut r = running(date(2026, 1, 1), time(0, 0, 0, 0));
        for _ in 0..5 {
            r.tick();
        }
        assert_eq!(read_pair(&r).1, time(0, 0, 0, 5), "1 tick per second");

        // Reload with a divide-by-4 and confirm four ticks buy one second.
        let mut r = Rp2040Rtc::new();
        r.write_u32(CLKDIV_M1, 3).unwrap();
        r.write_u32(SETUP_1, time(0, 0, 0, 0)).unwrap();
        r.write_u32(CTRL, CTRL_LOAD).unwrap();
        r.write_u32(CTRL, CTRL_RTC_ENABLE).unwrap();
        for _ in 0..20 {
            r.tick();
        }
        assert_eq!(read_pair(&r).1, time(0, 0, 0, 5));
    }

    /// The whole calendar carries: 23:59:59 on 31 December rolls the year.
    #[test]
    fn calendar_carries_through_new_year() {
        let mut r = running(date(2026, 12, 31), time(4, 23, 59, 59));
        r.tick();
        assert_eq!(read_pair(&r), (date(2027, 1, 1), time(5, 0, 0, 0)));
    }

    /// February follows the RP2040's own leap rule: `year % 4 == 0`.
    #[test]
    fn february_uses_the_hardware_leap_rule() {
        let mut r = running(date(2024, 2, 28), time(0, 23, 59, 59));
        r.tick();
        assert_eq!(read_pair(&r).0, date(2024, 2, 29), "2024 is a leap year");

        let mut r = running(date(2026, 2, 28), time(0, 23, 59, 59));
        r.tick();
        assert_eq!(read_pair(&r).0, date(2026, 3, 1), "2026 is not");
    }

    /// FORCE_NOTLEAPYEAR exists because the hardware rule gets century years
    /// wrong — 1900 is a leap year to this RTC unless firmware says otherwise.
    #[test]
    fn force_notleapyear_overrides_the_simple_rule() {
        let mut r = running(date(1900, 2, 28), time(0, 23, 59, 59));
        r.tick();
        assert_eq!(read_pair(&r).0, date(1900, 2, 29), "hardware rule: %4 only");

        let mut r = Rp2040Rtc::new();
        r.write_u32(SETUP_0, date(1900, 2, 28)).unwrap();
        r.write_u32(SETUP_1, time(0, 23, 59, 59)).unwrap();
        r.write_u32(CTRL, CTRL_LOAD | CTRL_FORCE_NOTLEAPYEAR)
            .unwrap();
        r.write_u32(CTRL, CTRL_RTC_ENABLE | CTRL_FORCE_NOTLEAPYEAR)
            .unwrap();
        r.tick();
        assert_eq!(read_pair(&r).0, date(1900, 3, 1));
    }

    /// The read latch: RTC_0 is frozen until RTC_1 is read again. This is the
    /// datasheet's ordering rule, and getting it wrong on silicon yields a
    /// stale time — so it must yield a stale time here.
    #[test]
    fn rtc_0_is_frozen_until_rtc_1_is_reread() {
        let mut r = running(date(2026, 7, 27), time(1, 12, 0, 0));
        let first = read_pair(&r);
        assert_eq!(first.1, time(1, 12, 0, 0));

        for _ in 0..5 {
            r.tick();
        }
        // No RTC_1 read: RTC_0 still reports the latched value.
        assert_eq!(r.read_u32(RTC_0).unwrap(), time(1, 12, 0, 0));
        // Re-latching picks up the new time.
        assert_eq!(read_pair(&r).1, time(1, 12, 0, 5));
    }

    /// Reading RTC_0 without ever reading RTC_1 gets the reset latch, not the
    /// live calendar — the exact firmware bug the ordering rule guards against.
    #[test]
    fn reading_rtc_0_first_returns_the_stale_latch() {
        let mut r = running(date(2026, 7, 27), time(1, 12, 0, 0));
        for _ in 0..3 {
            r.tick();
        }
        assert_eq!(r.read_u32(RTC_0).unwrap(), 0, "never latched");
        assert_eq!(read_pair(&r).1, time(1, 12, 0, 3));
    }

    /// The alarm matches only the enabled fields, and asserts MATCH_ACTIVE +
    /// INTR while the match holds.
    #[test]
    fn alarm_matches_enabled_fields_only() {
        let mut r = running(date(2026, 7, 27), time(1, 12, 0, 0));
        // Alarm on second == 3, every other field ignored.
        r.write_u32(IRQ_SETUP_1, SEC_ENA | 3).unwrap();
        r.write_u32(IRQ_SETUP_0, MATCH_ENA).unwrap();
        for _ in 0..3 {
            assert_eq!(r.read_u32(INTR).unwrap(), 0);
            r.tick();
        }
        assert_eq!(r.read_u32(INTR).unwrap(), 1);
        assert_ne!(r.read_u32(IRQ_SETUP_0).unwrap() & MATCH_ACTIVE, 0);
        // MATCH_ACTIVE is read-only and drops when the second moves on.
        r.tick();
        assert_eq!(r.read_u32(INTR).unwrap(), 0);
    }

    /// Without MATCH_ENA there is no alarm however well the fields line up.
    #[test]
    fn match_ena_gates_the_alarm() {
        let mut r = running(date(2026, 7, 27), time(1, 12, 0, 0));
        r.write_u32(IRQ_SETUP_1, SEC_ENA).unwrap(); // sec == 0, matches now
        r.write_u32(IRQ_SETUP_0, 0).unwrap();
        assert_eq!(r.read_u32(INTR).unwrap(), 0);
        r.write_u32(IRQ_SETUP_0, MATCH_ENA).unwrap();
        assert_eq!(r.read_u32(INTR).unwrap(), 1);
    }

    /// RTC_IRQ is delivered level-sensitively and gated by INTE; INTF forces it.
    #[test]
    fn rtc_irq_is_level_held_and_gated_by_inte() {
        // Masked (INTE = 0): the raw level rises but no NVIC line is raised.
        let mut r = running(date(2026, 7, 27), time(1, 12, 0, 0));
        r.write_u32(IRQ_SETUP_1, SEC_ENA | 2).unwrap();
        r.write_u32(IRQ_SETUP_0, MATCH_ENA).unwrap();
        assert_eq!(r.tick().explicit_irqs, None); // sec 1
        assert_eq!(r.tick().explicit_irqs, None); // sec 2 — matched, but masked
        assert_eq!(r.read_u32(INTR).unwrap(), 1);
        assert_eq!(r.read_u32(INTS).unwrap(), 0);
        r.write_u32(INTE, 1).unwrap();
        assert_eq!(r.read_u32(INTS).unwrap(), 1, "unmasking exposes the level");

        // Delivery: an alarm on MIN == 0 holds for a whole minute, so the line
        // is genuinely re-asserted tick after tick.
        let mut r = running(date(2026, 7, 27), time(1, 12, 0, 0));
        r.write_u32(INTE, 1).unwrap();
        r.write_u32(IRQ_SETUP_1, MIN_ENA).unwrap();
        r.write_u32(IRQ_SETUP_0, MATCH_ENA).unwrap();
        assert_eq!(r.tick().explicit_irqs, Some(vec![RTC_IRQ]));
        assert_eq!(r.tick().explicit_irqs, Some(vec![RTC_IRQ]), "held");
        // Disabling the alarm is the acknowledgement — there is no INTR write.
        r.write_u32(IRQ_SETUP_0, 0).unwrap();
        assert_eq!(r.tick().explicit_irqs, None);

        // INTF forces the line with no alarm configured at all.
        r.write_u32(INTF, 1).unwrap();
        assert_eq!(r.tick().explicit_irqs, Some(vec![RTC_IRQ]));
    }

    /// DOTW is stored and stepped modulo 7, never derived from the date.
    #[test]
    fn dotw_wraps_modulo_seven() {
        let mut r = running(date(2026, 7, 27), time(6, 23, 59, 59));
        r.tick();
        assert_eq!((read_pair(&r).1 >> DOTW_SHIFT) & DOTW_MASK, 0);
    }
}
