//! TI CC2538 sleep timer (`SMWDTHROSC`, `0x400D_5000`).
//!
//! Source: SWRU319C §13 "Sleep Timer" and Table 13-1 "SMWDTHROSC Register Summary"
//! (`ST0`-`ST3` @ `0x40`-`0x4C`, `STLOAD` @ `0x50`, `STCC` @ `0x54`, `STCS`
//! @ `0x58`, `STCV0`-`STCV3` @ `0x5C`-`0x68`); §14 / Table 14-1 for `WDCTL`
//! @ `0x00`.
//!
//! Modelled:
//! - the free-running 32-bit counter on the 32-kHz clock (§13.1), derived
//!   from the bus cycle count at `clock_hz` (the CPU clock the cycles run
//!   at), starting at 0 on reset;
//! - the read latch: reading `ST0` latches the whole count, and `ST1`-`ST3`
//!   return the latched bytes (§13.1 "ST0 must be read first");
//! - the compare (§13.2): `ST3`/`ST2`/`ST1` stage bytes, writing `ST0` loads
//!   all four (reset compare `0xFFFF_FFFF`); when the counter reaches the
//!   compare value the sleep-timer interrupt is raised on this block's IRQ
//!   line (SMT, IRQ 145 in the regular map). `STLOAD.STLOAD` always reads 1:
//!   the load completes at once. With the event scheduler the match is a
//!   scheduled event; without it the legacy walk ticks this block only while
//!   a compare is armed.
//! - `WDCTL`, `STCC` and `STCS` as storage at their reset values.
//!
//! NOT modelled: the watchdog (`WDCTL.EN` arms nothing, no reset on time-out),
//! timer capture (`STCC`/`STCS`/`STCV*`: `STCS.VALID` never sets), PM3 (the
//! count is never lost), the "at least 5 ticks ahead" load race (a compare
//! at or behind the count simply fires after the 32-bit wrap), and any skew
//! when firmware moves `SYS_DIV` away from `clock_hz`.

use super::lane_read;
use crate::{CycleClock, Peripheral, SimResult};
use std::cell::Cell;

const WDCTL: u64 = 0x00;
const ST0: u64 = 0x40;
const ST1: u64 = 0x44;
const ST2: u64 = 0x48;
const ST3: u64 = 0x4C;
const STLOAD: u64 = 0x50;
const STCC: u64 = 0x54;
const STCS: u64 = 0x58;
const STCV0: u64 = 0x5C;
const STCV3: u64 = 0x68;

/// `STCC` reset value (Table 13-1: `PORT` = 3, capture disabled).
const STCC_RESET: u32 = 0x38;
/// Compare value after reset (§13.2).
const COMPARE_RESET: u32 = 0xFFFF_FFFF;
/// The 32-kHz clock rate (§7.2.3, 32.768-kHz crystal or RCOSC).
const CLK_32K_HZ: u128 = 32_768;
/// CC2538 reset SYS clock: 16-MHz HF-RC with `SYS_DIV` = 1 (Table 7-3).
pub const DEFAULT_CLOCK_HZ: u64 = 16_000_000;

/// CC2538 sleep timer + watchdog block.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cc2538SleepTimer {
    clock_hz: u64,
    wdctl: u32,
    stcc: u32,
    stcs: u32,
    /// `ST1`-`ST3` bytes written since the last load (compare bits 31:8).
    staged: u32,
    compare: u32,
    /// The count `ST0` latched, returned by `ST1`-`ST3`.
    latched: Cell<u32>,
    /// Bumped on every compare load; a scheduled event whose token no longer
    /// matches was superseded and is dropped.
    generation: u32,
    /// CPU cycle at which the armed compare matches; `None` once it fired.
    armed_cycle: Option<u64>,
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl Cc2538SleepTimer {
    pub fn new(clock_hz: u64) -> Self {
        Self {
            clock_hz: clock_hz.max(1),
            wdctl: 0,
            stcc: STCC_RESET,
            stcs: 0,
            staged: 0,
            compare: COMPARE_RESET,
            latched: Cell::new(0),
            generation: 0,
            armed_cycle: None,
            pending_events: Vec::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now_cycle(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    /// 32-kHz ticks elapsed at CPU cycle `cycle`.
    fn ticks_at(&self, cycle: u64) -> u64 {
        (u128::from(cycle) * CLK_32K_HZ / u128::from(self.clock_hz)) as u64
    }

    /// First CPU cycle at which `ticks` 32-kHz ticks have elapsed.
    fn cycle_of_tick(&self, ticks: u64) -> u64 {
        let num = u128::from(ticks) * u128::from(self.clock_hz);
        num.div_ceil(CLK_32K_HZ) as u64
    }

    /// The sleep-timer count now (`ST3:ST0`).
    pub fn count(&self) -> u32 {
        self.ticks_at(self.now_cycle()) as u32
    }

    /// Load `staged | st0` as the compare value and arm its match.
    fn load_compare(&mut self, st0: u8) {
        self.compare = self.staged | u32::from(st0);
        self.generation = self.generation.wrapping_add(1);
        self.armed_cycle = None;
        if self.clock.is_none() {
            // No time base: the count never moves, so nothing can match.
            return;
        }
        let now = self.now_cycle();
        let ticks_now = self.ticks_at(now);
        let mut ahead = u64::from(self.compare.wrapping_sub(ticks_now as u32));
        if ahead == 0 {
            // The count already shows the compare value: that edge has
            // passed, so the next match is a full wrap away.
            ahead = 1 << 32;
        }
        let fire = self.cycle_of_tick(ticks_now + ahead).max(now + 1);
        self.armed_cycle = Some(fire);
        if self.scheduler_mode() {
            self.pending_events.push((fire - now, self.generation));
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            WDCTL => self.wdctl,
            ST0 => {
                let count = self.count();
                self.latched.set(count);
                count & 0xFF
            }
            ST1 => (self.latched.get() >> 8) & 0xFF,
            ST2 => (self.latched.get() >> 16) & 0xFF,
            ST3 => self.latched.get() >> 24,
            STLOAD => 1,
            STCC => self.stcc,
            STCS => self.stcs,
            // Capture is not modelled: the capture value stays 0.
            STCV0..=STCV3 => 0,
            _ => {
                crate::census_reg!("cc2538:Cc2538SleepTimer", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let byte = value & 0xFF;
        match offset {
            WDCTL => self.wdctl = byte,
            ST0 => self.load_compare(byte as u8),
            ST1 => self.staged = (self.staged & !0x0000_FF00) | (byte << 8),
            ST2 => self.staged = (self.staged & !0x00FF_0000) | (byte << 16),
            ST3 => self.staged = (self.staged & !0xFF00_0000) | (byte << 24),
            STCC => self.stcc = byte & 0x3F,
            STCS => self.stcs = byte & 0x01,
            STLOAD | STCV0..=STCV3 => {}
            _ => {
                crate::census_reg!("cc2538:Cc2538SleepTimer", offset, "write");
            }
        }
    }
}

impl Default for Cc2538SleepTimer {
    fn default() -> Self {
        Self::new(DEFAULT_CLOCK_HZ)
    }
}

impl Peripheral for Cc2538SleepTimer {
    /// The count is read lazily from the bus cycle clock; only an armed
    /// compare needs the walk, and only when there is no event scheduler.
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && self.armed_cycle.is_some()
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
        let due =
            !self.scheduler_mode() && self.armed_cycle.is_some_and(|at| self.now_cycle() >= at);
        if due {
            self.armed_cycle = None;
        }
        crate::PeripheralTickResult {
            irq: due,
            ..Default::default()
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane_read(self.read_reg(offset & !3), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // Every register is one byte wide in bits 7:0; writes to the upper
        // (reserved) lanes change nothing.
        if offset % 4 == 0 {
            self.write_reg(offset, u32::from(value));
        }
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
        let live = event_token == self.generation && self.armed_cycle.is_some();
        if live {
            self.armed_cycle = None;
        }
        crate::sched::EventResult {
            raise_own_irq: live,
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_node_reset(&mut self) {
        let clock = self.clock.take();
        *self = Self::new(self.clock_hz);
        self.clock = clock;
    }

    snapshot_impl!(Cc2538SleepTimer, "Cc2538SleepTimer", keep[clock]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer_at(cycle: u64) -> (Cc2538SleepTimer, CycleClock) {
        let clock = CycleClock::default();
        clock.publish(cycle);
        let mut t = Cc2538SleepTimer::new(DEFAULT_CLOCK_HZ);
        t.attach_cycle_clock(clock.clone());
        (t, clock)
    }

    fn read_count(t: &Cc2538SleepTimer) -> u32 {
        // ST0 first: it latches ST1-ST3 (§13.1).
        let b0 = t.read_u32(ST0).unwrap();
        b0 | (t.read_u32(ST1).unwrap() << 8)
            | (t.read_u32(ST2).unwrap() << 16)
            | (t.read_u32(ST3).unwrap() << 24)
    }

    #[test]
    fn counts_32k_ticks_of_the_cpu_clock() {
        // One second of 16-MHz cycles is 32 768 ticks.
        let (t, clock) = timer_at(0);
        assert_eq!(read_count(&t), 0);
        clock.publish(DEFAULT_CLOCK_HZ);
        assert_eq!(read_count(&t), 32_768);
        // 16 MHz / 32 768 Hz = 488.28 cycles per tick: 489 cycles is 1 tick.
        clock.publish(DEFAULT_CLOCK_HZ + 489);
        assert_eq!(read_count(&t), 32_769);
    }

    #[test]
    fn st0_latches_the_upper_bytes() {
        let (t, clock) = timer_at(0x0123_4567_u64 * 489);
        let b0 = t.read_u32(ST0).unwrap();
        let latched = t.count();
        // The counter moves on; ST1-ST3 still report the latched count.
        clock.publish(clock.now() + 1_000_000_000);
        assert_eq!(t.read_u32(ST1).unwrap(), (latched >> 8) & 0xFF);
        assert_eq!(t.read_u32(ST3).unwrap(), latched >> 24);
        assert_eq!(b0, latched & 0xFF);
    }

    #[test]
    fn compare_load_schedules_the_match_cycle() {
        let (mut t, _clock) = timer_at(DEFAULT_CLOCK_HZ); // count 32 768
        assert_eq!(t.read_u32(STLOAD).unwrap(), 1);
        let target: u32 = 32_768 + 100;
        t.write_u32(ST3, target >> 24).unwrap();
        t.write_u32(ST2, (target >> 16) & 0xFF).unwrap();
        t.write_u32(ST1, (target >> 8) & 0xFF).unwrap();
        assert_eq!(t.armed_cycle, None, "ST1-ST3 only stage");
        t.write_u32(ST0, target & 0xFF).unwrap();
        // 100 ticks = 48 828.125 cycles: the match is the first cycle at or
        // past the tick edge.
        assert_eq!(t.armed_cycle, Some(DEFAULT_CLOCK_HZ + 48_829));
        let events = t.take_scheduled_events();
        if t.scheduler_mode() {
            assert_eq!(events, vec![(48_829, t.generation)]);
        } else {
            assert!(events.is_empty());
            assert!(t.legacy_tick_active(), "armed compare must stay walked");
        }
    }

    #[test]
    fn legacy_walk_raises_the_irq_once_at_the_match() {
        let (mut t, clock) = timer_at(0);
        t.write_u32(ST0, 10).unwrap();
        let at = t.armed_cycle.unwrap();
        if t.scheduler_mode() {
            return; // covered by the scheduled event instead
        }
        clock.publish(at - 1);
        assert!(!t.tick_elapsed(1).irq);
        clock.publish(at);
        assert!(t.tick_elapsed(1).irq);
        assert!(!t.tick_elapsed(1).irq, "one match, one interrupt");
        assert!(
            !t.legacy_tick_active(),
            "a fired compare drops off the walk"
        );
    }

    #[test]
    fn a_reload_supersedes_the_pending_compare() {
        let (mut t, _clock) = timer_at(0);
        t.write_u32(ST0, 50).unwrap();
        let stale = t.generation;
        t.write_u32(ST0, 80).unwrap();
        assert_ne!(stale, t.generation);
        assert_eq!(t.armed_cycle, Some(t.cycle_of_tick(80)));
    }

    #[test]
    fn a_compare_equal_to_the_count_waits_a_full_wrap() {
        let (mut t, _clock) = timer_at(0);
        t.write_u32(ST0, 0).unwrap();
        assert_eq!(t.armed_cycle, Some(t.cycle_of_tick(1 << 32)));
    }

    #[test]
    fn without_a_clock_it_stays_at_zero_and_off_the_scheduler() {
        let mut t = Cc2538SleepTimer::new(DEFAULT_CLOCK_HZ);
        assert!(!t.uses_scheduler());
        assert_eq!(read_count(&t), 0);
        t.write_u32(ST0, 5).unwrap();
        assert!(t.take_scheduled_events().is_empty());
        assert!(!t.legacy_tick_active());
    }

    #[test]
    fn registers_reset_to_table_values() {
        let t = Cc2538SleepTimer::new(DEFAULT_CLOCK_HZ);
        assert_eq!(t.read_u32(STCC).unwrap(), STCC_RESET);
        assert_eq!(t.read_u32(WDCTL).unwrap(), 0);
        assert_eq!(t.compare, COMPARE_RESET);
    }
}
