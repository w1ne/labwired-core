// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 RTC peripheral.
//!
//! Source: nRF52840 PS rev 1.7 §6.21 (RTC). Models RTC0..RTC2 on a
//! 32.768 kHz LFCLK; 24-bit counter with a 12-bit prescaler:
//! f_RTC = 32_768 / (PRESCALER + 1) Hz.
//!
//! Instance-specific CC count (PS §6.21, table 97):
//!   RTC0       — 3 CC registers (CC[0..2])
//!   RTC1, RTC2 — 4 CC registers (CC[0..3])
//! Accesses to CC[i] where i >= num_cc are silently ignored on write and
//! return 0 on read. INTEN and EVTEN compare bits are masked to num_cc.
//!
//! EVENTS_* semantics: hardware-generated only. Writes of 1 are ignored;
//! only writes of 0 clear the event register. HW sets events via tick() /
//! lazy advance.
//!
//! ## Drive modes (walk-free plan Part 1)
//!
//! * **Scheduler mode** (`event-scheduler` feature + a bus [`CycleClock`]):
//!   free-running COUNTER / prescaler / LFCLK phase live in `Cell`s. A
//!   `&self` COUNTER (and other) read advances state to the published clock
//!   (batch-boundary freshness — same bound as write-path `sync_to`). Compare /
//!   TICK / OVRFLW IRQs ride scheduled events; event latches also materialise
//!   on the lazy advance so a poll of EVENTS_* stays self-consistent with
//!   COUNTER.
//! * **Legacy mode** (feature off, or no clock): per-cycle `tick()` advances
//!   eagerly, byte-identical to the historical model.
//!
//! Unit tests that call tick() directly use `Nrf52Rtc::new_fast()` which sets
//! LFCLK ratio 1:1 so small tick counts suffice.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;

// ── Register offsets (PS §6.21.13) ───────────────────────────────────────────

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_TASKS_CLEAR: u64 = 0x008;
const OFF_TASKS_TRIGOVRFLW: u64 = 0x00C;
const OFF_EVENTS_TICK: u64 = 0x100;
const OFF_EVENTS_OVRFLW: u64 = 0x104;
const OFF_EVENTS_COMPARE0: u64 = 0x140;
const OFF_EVENTS_COMPARE3: u64 = 0x14C;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_EVTEN: u64 = 0x340;
const OFF_EVTENSET: u64 = 0x344;
const OFF_EVTENCLR: u64 = 0x348;
const OFF_COUNTER: u64 = 0x504;
const OFF_PRESCALER: u64 = 0x508;
const OFF_CC0: u64 = 0x540;
const OFF_CC3: u64 = 0x54C;

// INTEN / EVTEN bits (PS table 109):
//   TICK     bit 0
//   OVRFLW   bit 1
//   COMPARE0 bit 16, COMPARE1 bit 17, COMPARE2 bit 18, COMPARE3 bit 19
const EN_TICK: u32 = 1 << 0;
const EN_OVRFLW: u32 = 1 << 1;
const EN_COMPARE_SHIFT: u32 = 16;

const COUNTER_MASK: u32 = 0x00FF_FFFF;
const PRESCALER_MASK: u32 = 0xFFF;

/// CPU cycles per LFCLK base-clock tick (nRF52840: 64 MHz CPU, 32.768 kHz LFCLK).
///
/// 64_000_000 / 32_768 = 1953.125 exactly = 15625 / 8.
/// We advance `lfclk_accum` by LFCLK_ACCUM_INC each `tick()` call (once per
/// CPU cycle) and fire one LFCLK edge when it reaches LFCLK_ACCUM_PERIOD.
/// This gives exactly 32768 Hz without accumulating rounding error.
///
/// Unit tests that call tick() directly use `Nrf52Rtc::new_fast()` which sets
/// both to 1, giving a 1:1 CPU:LFCLK ratio so small tick counts suffice.
pub const LFCLK_ACCUM_INC_DEFAULT: u32 = 8;
pub const LFCLK_ACCUM_PERIOD_DEFAULT: u32 = 15625; // 64_000_000 / (32_768 / 8)

#[derive(Debug)]
pub struct Nrf52Rtc {
    /// Number of CC/EVENTS_COMPARE channels present on this instance.
    /// RTC0 = 3; RTC1/RTC2 = 4. Default: 3.
    num_cc: usize,
    events_tick: Cell<u32>,
    events_ovrflw: Cell<u32>,
    events_compare: [Cell<u32>; 4],
    inten: u32,
    evten: u32,
    /// 24-bit free-running counter. `Cell` so `&self` COUNTER reads can
    /// lazy-sync under the event-scheduler path (batch-boundary freshness).
    counter: Cell<u32>,
    prescaler: u32,

    cc: [u32; 4],

    running: bool,
    /// PRESCALER phase. `Cell` for the same lazy `&self` advance as `counter`.
    prescaler_accum: Cell<u32>,
    /// Fractional LFCLK accumulator. Incremented by `lfclk_inc` each CPU
    /// cycle; when it reaches `lfclk_period` one LFCLK base-clock cycle fires
    /// and is fed into the PRESCALER divider. This models the ratio
    /// 64 MHz CPU : 32.768 kHz LFCLK = 1953.125 CPU cycles per LFCLK tick.
    ///
    /// Set both to 1 (via `new_fast`) for unit tests that call tick() directly.
    lfclk_accum: Cell<u32>,
    lfclk_inc: u32,
    lfclk_period: u32,
    clock: Option<CycleClock>,
    /// CPU cycle of the last advance (`Cell` so `&self` read-sync is idempotent).
    anchor: Cell<u64>,
    arm_seq: u32,
    /// IRQ latched by a lazy advance that has not yet been claimed by `on_event`.
    pending_irq: Cell<bool>,
    /// Event bitmap (INTEN/EVTEN bit positions) for PPI `fired_events` not yet
    /// claimed by the event drain.
    pending_fired: Cell<u32>,
}

impl Default for Nrf52Rtc {
    fn default() -> Self {
        Self {
            num_cc: 3,
            events_tick: Cell::new(0),
            events_ovrflw: Cell::new(0),
            events_compare: std::array::from_fn(|_| Cell::new(0)),
            inten: 0,
            evten: 0,
            counter: Cell::new(0),
            prescaler: 0,
            cc: [0u32; 4],
            running: false,
            prescaler_accum: Cell::new(0),
            lfclk_accum: Cell::new(0),
            lfclk_inc: LFCLK_ACCUM_INC_DEFAULT,
            lfclk_period: LFCLK_ACCUM_PERIOD_DEFAULT,
            clock: None,
            anchor: Cell::new(0),
            arm_seq: 0,
            pending_irq: Cell::new(false),
            pending_fired: Cell::new(0),
        }
    }
}

impl Nrf52Rtc {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with an explicit CC count. Use `num_cc: 4` for RTC1/RTC2.
    pub fn new_with_cc(num_cc: usize) -> Self {
        Self {
            num_cc: num_cc.clamp(1, 4),
            ..Self::default()
        }
    }

    /// Construct a "fast" RTC with 1:1 CPU:LFCLK ratio. Intended for unit
    /// tests that call `tick()` directly and want small tick counts to fire
    /// events, without requiring 1953 ticks per counter increment.
    #[cfg(test)]
    pub fn new_fast() -> Self {
        Self {
            lfclk_inc: 1,
            lfclk_period: 1,
            ..Self::default()
        }
    }

    /// True when the event scheduler owns this RTC's time base (feature on AND
    /// bus clock attached). Everything time-related branches on this ONE
    /// predicate so the two drive modes can never mix.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Advance the LFCLK accumulator by one CPU-cycle increment. Returns true
    /// if a LFCLK base-clock edge fired (i.e. the prescaler/counter should
    /// advance this cycle).
    #[inline]
    fn advance_lfclk(&self) -> bool {
        let accum = self.lfclk_accum.get().wrapping_add(self.lfclk_inc);
        if accum >= self.lfclk_period {
            self.lfclk_accum.set(accum - self.lfclk_period);
            true
        } else {
            self.lfclk_accum.set(accum);
            false
        }
    }

    /// INTEN/EVTEN compare-bit mask: bits 16..16+num_cc.
    fn compare_mask(&self) -> u32 {
        let bits = (1u32 << self.num_cc) - 1;
        bits << EN_COMPARE_SHIFT
    }

    /// Shared advance used by both drive modes: consume `cycles` CPU cycles,
    /// step LFCLK → PRESCALER → COUNTER, and evaluate EVTEN/INTEN against the
    /// new value. Mutates only `Cell`-held state (callable from `&self`).
    ///
    /// Returns `(irq, fired_mask)` where `fired_mask` uses INTEN/EVTEN bit
    /// positions for PPI / event-drain claim.
    fn advance_and_eval(&self, cycles: u64) -> (bool, u32) {
        if !self.running || cycles == 0 {
            return (false, 0);
        }
        let mut irq = false;
        let mut fired_mask = 0u32;
        for _ in 0..cycles {
            if !self.advance_lfclk() {
                continue;
            }
            let divider = (self.prescaler & PRESCALER_MASK) + 1;
            let psc = self.prescaler_accum.get().wrapping_add(1);
            if psc < divider {
                self.prescaler_accum.set(psc);
                continue;
            }
            self.prescaler_accum.set(0);
            let prev = self.counter.get();
            let next = (prev.wrapping_add(1)) & COUNTER_MASK;
            self.counter.set(next);
            if self.evten & EN_TICK != 0 {
                self.events_tick.set(1);
                fired_mask |= EN_TICK;
            }
            if self.inten & EN_TICK != 0 {
                irq = true;
            }
            if prev == COUNTER_MASK && next == 0 {
                if self.evten & EN_OVRFLW != 0 {
                    self.events_ovrflw.set(1);
                    fired_mask |= EN_OVRFLW;
                }
                if self.inten & EN_OVRFLW != 0 {
                    irq = true;
                }
            }
            for i in 0..self.num_cc {
                if next == (self.cc[i] & COUNTER_MASK) {
                    let bit = 1u32 << (EN_COMPARE_SHIFT + i as u32);
                    if self.evten & bit != 0 {
                        self.events_compare[i].set(1);
                        fired_mask |= bit;
                    }
                    if self.inten & bit != 0 {
                        irq = true;
                    }
                }
            }
        }
        (irq, fired_mask)
    }

    fn fired_mask_to_events(&self, fired_mask: u32) -> Vec<u32> {
        let mut fired_events = Vec::new();
        if fired_mask & EN_TICK != 0 {
            fired_events.push(OFF_EVENTS_TICK as u32);
        }
        if fired_mask & EN_OVRFLW != 0 {
            fired_events.push(OFF_EVENTS_OVRFLW as u32);
        }
        for i in 0..self.num_cc {
            let bit = 1u32 << (EN_COMPARE_SHIFT + i as u32);
            if fired_mask & bit != 0 {
                fired_events.push(OFF_EVENTS_COMPARE0 as u32 + 4 * i as u32);
            }
        }
        fired_events
    }

    /// Lazy advance to absolute published cycle `now` — callable from `&self`.
    /// Idempotent; a `now` older than the anchor is ignored. Fired events /
    /// IRQs accumulate into pending Cells for the event drain.
    fn advance_to(&self, now: u64) {
        let anchor = self.anchor.get();
        if now <= anchor {
            return;
        }
        self.anchor.set(now);
        let (irq, fired_mask) = self.advance_and_eval(now - anchor);
        if irq {
            self.pending_irq.set(true);
        }
        if fired_mask != 0 {
            self.pending_fired
                .set(self.pending_fired.get() | fired_mask);
        }
    }

    /// Pull "now" from the bus-published clock and advance. No-op without an
    /// attached clock (legacy mode — the walk advances the counter instead).
    fn sync_from_clock(&self) {
        if self.scheduler_mode() {
            if let Some(clock) = self.clock.as_ref() {
                self.advance_to(clock.now());
            }
        }
    }

    /// Conservative upper bound: next counter increment that can fire any
    /// enabled compare / overflow / tick. Uses the LFCLK ratio + prescaler.
    fn cycles_until_next_event(&self) -> Option<u64> {
        if !self.running {
            return None;
        }
        // CPU cycles per LFCLK edge (approx ceil of period/inc).
        let lfclk_period = self.lfclk_period.max(1) as u64;
        let lfclk_inc = self.lfclk_inc.max(1) as u64;
        // Remaining LFCLK fractional units until next edge.
        let lfclk_accum = self.lfclk_accum.get();
        let remain_frac = if lfclk_accum >= self.lfclk_period {
            lfclk_period
        } else {
            (self.lfclk_period - lfclk_accum) as u64
        };
        let cpu_to_lfclk = remain_frac.div_ceil(lfclk_inc).max(1);
        let divider = ((self.prescaler & PRESCALER_MASK) + 1) as u64;
        let prescaler_accum = self.prescaler_accum.get();
        let lfclk_to_counter = if prescaler_accum >= divider as u32 {
            1u64
        } else {
            (divider - prescaler_accum as u64).max(1)
        };
        // If TICK is enabled (INTEN or EVTEN), next counter tick is the deadline.
        if self.inten & EN_TICK != 0 || self.evten & EN_TICK != 0 {
            return Some(cpu_to_lfclk * lfclk_to_counter);
        }
        // Else next compare or overflow.
        let mut best_steps: Option<u64> = None;
        let cur = self.counter.get() & COUNTER_MASK;
        for i in 0..self.num_cc {
            let bit = EN_COMPARE_SHIFT + i as u32;
            if self.inten & (1 << bit) == 0 && self.evten & (1 << bit) == 0 {
                continue;
            }
            let target = self.cc[i] & COUNTER_MASK;
            let steps = if target > cur {
                (target - cur) as u64
            } else {
                (COUNTER_MASK as u64 + 1) - cur as u64 + target as u64
            };
            best_steps = Some(best_steps.map_or(steps, |b| b.min(steps)));
        }
        if self.inten & EN_OVRFLW != 0 || self.evten & EN_OVRFLW != 0 {
            let steps = (COUNTER_MASK - cur) as u64 + 1;
            best_steps = Some(best_steps.map_or(steps, |b| b.min(steps)));
        }
        let steps = best_steps.unwrap_or(1);
        Some(
            cpu_to_lfclk + (steps.saturating_sub(1)) * (lfclk_period.div_ceil(lfclk_inc) * divider),
        )
    }
}

impl Peripheral for Nrf52Rtc {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        // Scheduler mode: advance free-running state (and materialise any
        // events crossed) to the published "now" first, so a polled COUNTER
        // read observes batch-boundary-fresh time.
        self.sync_from_clock();
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_STOP | OFF_TASKS_CLEAR | OFF_TASKS_TRIGOVRFLW => 0,
            OFF_EVENTS_TICK => self.events_tick.get(),
            OFF_EVENTS_OVRFLW => self.events_ovrflw.get(),
            // EVENTS_COMPARE[i]: return 0 for i >= num_cc.
            OFF_EVENTS_COMPARE0..=OFF_EVENTS_COMPARE3 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_EVENTS_COMPARE0) / 4) as usize;
                if i < self.num_cc {
                    self.events_compare[i].get()
                } else {
                    0
                }
            }
            // INTENSET/INTENCLR: mask to valid compare bits + TICK + OVRFLW.
            OFF_INTENSET | OFF_INTENCLR => self.inten & (EN_TICK | EN_OVRFLW | self.compare_mask()),
            // EVTEN/EVTENSET/EVTENCLR: same mask.
            OFF_EVTEN | OFF_EVTENSET | OFF_EVTENCLR => {
                self.evten & (EN_TICK | EN_OVRFLW | self.compare_mask())
            }
            OFF_COUNTER => self.counter.get() & COUNTER_MASK,
            OFF_PRESCALER => self.prescaler,
            // CC[i]: return 0 for i >= num_cc.
            OFF_CC0..=OFF_CC3 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_CC0) / 4) as usize;
                if i < self.num_cc {
                    self.cc[i]
                } else {
                    0
                }
            }
            _ => {
                crate::census_reg!("nrf52.rtc:Nrf52Rtc", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START
                if value & 1 != 0 => {
                    self.running = true;
                }
            OFF_TASKS_STOP
                if value & 1 != 0 => {
                    self.running = false;
                }
            OFF_TASKS_CLEAR
                if value & 1 != 0 => {
                    self.counter.set(0);
                    self.prescaler_accum.set(0);
                    self.lfclk_accum.set(0);
                }
            OFF_TASKS_TRIGOVRFLW
                // Per PS §6.21.5: sets COUNTER to 0x00FFFFF0 to trigger overflow
                // 16 ticks later. Useful for test programs.
                if value & 1 != 0 => {
                    self.counter.set(0x00FF_FFF0);
                }
            // EVENTS_TICK/OVRFLW: hardware-generated; SW may only clear (write 0).
            OFF_EVENTS_TICK if value == 0 => self.events_tick.set(0),
            OFF_EVENTS_OVRFLW if value == 0 => self.events_ovrflw.set(0),
            // EVENTS_COMPARE[i]: write-1 ignored; write-0 clears within num_cc.
            OFF_EVENTS_COMPARE0..=OFF_EVENTS_COMPARE3 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_EVENTS_COMPARE0) / 4) as usize;
                if i < self.num_cc && value == 0 {
                    self.events_compare[i].set(0);
                }
            }
            // INTENSET/INTENCLR: mask to valid bits.
            OFF_INTENSET => self.inten |= value & (EN_TICK | EN_OVRFLW | self.compare_mask()),
            OFF_INTENCLR => self.inten &= !value,
            // EVTEN/EVTENSET/EVTENCLR: mask to valid bits.
            OFF_EVTEN => self.evten = value & (EN_TICK | EN_OVRFLW | self.compare_mask()),
            OFF_EVTENSET => self.evten |= value & (EN_TICK | EN_OVRFLW | self.compare_mask()),
            OFF_EVTENCLR => self.evten &= !value,
            // COUNTER is RO.
            OFF_COUNTER => {}
            OFF_PRESCALER
                // PS §6.21.5: PRESCALER can only be written while STOPPED.
                if !self.running => {
                    self.prescaler = value & PRESCALER_MASK;
                }
            // CC[i]: only valid for i < num_cc.
            OFF_CC0..=OFF_CC3 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_CC0) / 4) as usize;
                if i < self.num_cc {
                    self.cc[i] = value & COUNTER_MASK;
                }
            }
            _ => { crate::census_reg!("nrf52.rtc:Nrf52Rtc", offset, "write"); }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Legacy / feature-off path. Scheduler mode skips the walk.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        let (irq, fired_mask) = self.advance_and_eval(1);
        PeripheralTickResult {
            irq,
            cycles: 1,
            fired_events: self.fired_mask_to_events(fired_mask),
            ..Default::default()
        }
    }

    /// Compatibility hook for the bare-CPU forced walk
    /// (`tick_peripherals_fully_forced`). In scheduler mode `tick` above
    /// deliberately no-ops and the counter advances only via `sync_to` /
    /// `advance_to`, which reads the bus-published cycle clock. That clock is
    /// frozen while the forced walk runs, so `now <= anchor` on every call and
    /// the RTC can never reach its compare — `nrf52840_onboarding_rtc0_fires_
    /// compare_and_pends_irq` saw EVENTS_COMPARE[0] stay 0 through all 10000
    /// forced ticks. Invisible under `-p labwired-core` (feature off, so
    /// `scheduler_mode()` is false and the legacy walk runs), and only failing
    /// under `cargo test --workspace`, where Cargo unifies `event-scheduler`
    /// on from `crates/wasm`. Same contract, and the same discovery path, as
    /// the EXTI and DMA models that already override this.
    ///
    /// Advances one cycle to match the legacy walk's one-tick-per-bus-tick
    /// contract, and carries `anchor` with it so a later clock-driven
    /// `advance_to` cannot replay the cycles settled here. Never reached from
    /// production `Machine` execution: there the event chain
    /// (`take_scheduled_events` / `on_event`) stays the sole owner.
    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        if self.scheduler_mode() {
            self.anchor.set(self.anchor.get().wrapping_add(1));
        }
        let (irq, fired_mask) = self.advance_and_eval(1);
        PeripheralTickResult {
            irq,
            cycles: 1,
            fired_events: self.fired_mask_to_events(fired_mask),
            ..Default::default()
        }
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        // Anchor at the clock's current value so cycles that elapsed before
        // attach are not retroactively replayed into the counter.
        self.anchor.set(clock.now());
        self.clock = Some(clock);
    }

    fn sync_to(&mut self, now_cycle: u64) {
        if !self.scheduler_mode() {
            return;
        }
        self.advance_to(now_cycle);
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() || !self.running {
            return Vec::new();
        }
        if self.pending_irq.get() || self.pending_fired.get() != 0 {
            // A compare/tick already materialised (a COUNTER/EVENTS read
            // synced past its deadline): deliver at the next drain.
            self.arm_seq = self.arm_seq.wrapping_add(1);
            return vec![(0, self.arm_seq)];
        }
        let Some(d) = self.cycles_until_next_event() else {
            return Vec::new();
        };
        self.arm_seq = self.arm_seq.wrapping_add(1);
        vec![(d.saturating_sub(1), self.arm_seq)]
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != self.arm_seq {
            return crate::sched::EventResult::default();
        }
        // Bring lazy free-running state up to the drain cycle; this
        // materialises events this wake was scheduled for (and is a no-op
        // if a prior COUNTER/EVENTS poll already advanced to `now`).
        self.advance_to(sched.now());
        let irq = self.pending_irq.replace(false);
        let fired_mask = self.pending_fired.replace(0);
        let next = self.cycles_until_next_event();
        crate::sched::EventResult {
            raise_own_irq: irq,
            fired_events: self.fired_mask_to_events(fired_mask),
            reschedule_delay: next.map(|d| d.saturating_sub(1)),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prescaler_masks_to_12_bits() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_PRESCALER, 0xFFFF_FFFF).unwrap();
        assert_eq!(r.read_u32(OFF_PRESCALER).unwrap(), 0xFFF);
    }

    #[test]
    fn cc_above_num_cc_reads_zero() {
        // RTC0 has 3 CCs; CC[3] is absent.
        let mut r = Nrf52Rtc::new(); // default num_cc=3
        r.write_u32(OFF_CC0 + 3 * 4, 0x00_FFFF).unwrap(); // CC[3] — ignored
        assert_eq!(r.read_u32(OFF_CC0 + 3 * 4).unwrap(), 0);
    }

    #[test]
    fn inten_masked_to_num_cc() {
        // RTC0 (3 CC): bits 16/17/18 valid; bit 19 must be masked out.
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_INTENSET, 0x000F_0003).unwrap();
        // Bit 19 (CC[3]) should be dropped; bits 16..18 + TICK + OVRFLW remain.
        assert_eq!(r.read_u32(OFF_INTENSET).unwrap(), 0x0007_0003);

        // RTC1/2 (4 CC): bits 16..19 all valid.
        let mut r4 = Nrf52Rtc::new_with_cc(4);
        r4.write_u32(OFF_INTENSET, 0x000F_0003).unwrap();
        assert_eq!(r4.read_u32(OFF_INTENSET).unwrap(), 0x000F_0003);
    }

    #[test]
    fn events_write_one_ignored() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_EVENTS_TICK, 1).unwrap();
        assert_eq!(
            r.read_u32(OFF_EVENTS_TICK).unwrap(),
            0,
            "EVENTS_TICK write-1 must be no-op"
        );
        r.write_u32(OFF_EVENTS_OVRFLW, 1).unwrap();
        assert_eq!(
            r.read_u32(OFF_EVENTS_OVRFLW).unwrap(),
            0,
            "EVENTS_OVRFLW write-1 must be no-op"
        );
        r.write_u32(OFF_EVENTS_COMPARE0, 1).unwrap();
        assert_eq!(
            r.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
            0,
            "EVENTS_COMPARE write-1 must be no-op"
        );
    }

    #[test]
    fn events_tick_set_by_hw_cleared_by_sw() {
        let mut r = Nrf52Rtc::new_fast();
        r.write_u32(OFF_PRESCALER, 0).unwrap();
        r.write_u32(OFF_EVTENSET, EN_TICK).unwrap();
        r.write_u32(OFF_TASKS_START, 1).unwrap();
        r.tick();
        assert_eq!(
            r.read_u32(OFF_EVENTS_TICK).unwrap(),
            1,
            "HW must set EVENTS_TICK"
        );
        r.write_u32(OFF_EVENTS_TICK, 0).unwrap();
        assert_eq!(
            r.read_u32(OFF_EVENTS_TICK).unwrap(),
            0,
            "write-0 must clear EVENTS_TICK"
        );
    }

    #[test]
    fn prescaler_locked_while_running() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_PRESCALER, 0x10).unwrap();
        r.write_u32(OFF_TASKS_START, 1).unwrap();
        r.write_u32(OFF_PRESCALER, 0x100).unwrap(); // dropped
        assert_eq!(r.read_u32(OFF_PRESCALER).unwrap(), 0x10);
    }

    #[test]
    fn cc_masks_to_24_bits() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_CC0, 0xFFFF_FFFF).unwrap();
        assert_eq!(r.read_u32(OFF_CC0).unwrap(), 0x00FF_FFFF);
    }

    #[test]
    fn tick_compare_fires_event_and_irq() {
        let mut r = Nrf52Rtc::new_fast();
        r.write_u32(OFF_PRESCALER, 0).unwrap();
        r.write_u32(OFF_CC0, 7).unwrap();
        r.write_u32(OFF_EVTENSET, 1 << 16).unwrap();
        r.write_u32(OFF_INTENSET, 1 << 16).unwrap();
        r.write_u32(OFF_TASKS_START, 1).unwrap();

        let mut fires = 0;
        for _ in 0..14 {
            if r.tick().irq {
                fires += 1;
            }
        }
        assert_eq!(r.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 1);
        assert_eq!(fires, 1);
    }

    #[test]
    fn tick_tick_event_fires_when_enabled() {
        let mut r = Nrf52Rtc::new_fast();
        r.write_u32(OFF_PRESCALER, 0).unwrap();
        r.write_u32(OFF_EVTENSET, 1).unwrap();
        r.write_u32(OFF_INTENSET, 1).unwrap();
        r.write_u32(OFF_TASKS_START, 1).unwrap();

        let result = r.tick();
        assert_eq!(r.read_u32(OFF_EVENTS_TICK).unwrap(), 1);
        assert!(result.irq);
    }

    #[test]
    fn trigovrflw_jumps_counter_to_pretrigger() {
        let mut r = Nrf52Rtc::new();
        r.write_u32(OFF_TASKS_TRIGOVRFLW, 1).unwrap();
        assert_eq!(r.read_u32(OFF_COUNTER).unwrap(), 0x00FF_FFF0);
    }

    #[test]
    fn counter_wraps_at_24_bits_and_fires_ovrflw() {
        let mut r = Nrf52Rtc::new_fast();
        r.write_u32(OFF_PRESCALER, 0).unwrap();
        r.write_u32(OFF_EVTENSET, 1 << 1).unwrap(); // OVRFLW
        r.write_u32(OFF_INTENSET, 1 << 1).unwrap();
        r.write_u32(OFF_TASKS_TRIGOVRFLW, 1).unwrap(); // counter = 0x00FFFFF0
        r.write_u32(OFF_TASKS_START, 1).unwrap();

        let mut overflow_irq = false;
        for _ in 0..32 {
            if r.tick().irq {
                overflow_irq = true;
            }
        }
        assert!(overflow_irq);
        assert_eq!(r.read_u32(OFF_EVENTS_OVRFLW).unwrap(), 1);
    }

    #[cfg(feature = "event-scheduler")]
    mod scheduler_mode {
        use super::*;

        #[test]
        fn counter_read_syncs_from_cycle_clock() {
            let clock = CycleClock::default();
            let mut r = Nrf52Rtc::new_fast();
            r.attach_cycle_clock(clock.clone());
            r.write_u32(OFF_PRESCALER, 0).unwrap();
            r.write_u32(OFF_TASKS_START, 1).unwrap();
            // No INTEN/EVTEN — pure COUNTER poll.
            assert!(r.uses_scheduler());
            clock.publish(10);
            // write-path sync would also work; read-side must advance alone.
            assert_eq!(r.read_u32(OFF_COUNTER).unwrap(), 10);
            clock.publish(25);
            assert_eq!(r.read_u32(OFF_COUNTER).unwrap(), 25);
            // Idempotent at the same published cycle.
            assert_eq!(r.read_u32(OFF_COUNTER).unwrap(), 25);
        }

        #[test]
        fn without_clock_stays_on_legacy_tick_path() {
            let r = Nrf52Rtc::new();
            assert!(!r.uses_scheduler(), "no cycle clock attached → legacy walk");
        }
    }
}
