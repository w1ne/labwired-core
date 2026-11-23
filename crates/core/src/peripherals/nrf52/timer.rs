// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 TIMER peripheral.
//!
//! Source: nRF52840 Product Specification v1.7 §6.30 (TIMER), tables
//! 158–161.
//!
//! Models TIMER0..TIMER4 in Timer mode (`MODE=0`): runs on a 16 MHz base
//! clock divided by `2^PRESCALER` (1..=9), counter width set by BITMODE
//! (8/16/24/32 bits), CC[0..num_cc-1] comparators each raising
//! EVENTS_COMPARE[i] when matched and pending the peripheral's NVIC IRQ
//! when the corresponding INTEN bit is set. SHORTS provides auto-CLEAR
//! and auto-STOP on compare match (PS table 161).
//!
//! Instance-specific CC count (PS §6.30, table 152):
//!   TIMER0/1/2 — 4 CC registers (CC[0..3])
//!   TIMER3/4   — 6 CC registers (CC[0..5])
//! Accesses to CC[i] where i >= num_cc are silently ignored on write and
//! return 0 on read. SHORTS, INTEN, and TASKS_CAPTURE are similarly masked
//! to the active CC count.
//!
//! EVENTS_* semantics: hardware-generated only. Writes of 1 are ignored;
//! only writes of 0 clear the event register. HW sets events via compare
//! match in tick().
//!
//! Counter mode (`MODE=1` / `MODE=2`) increments only on TASKS_COUNT and
//! is supported by the register surface but not auto-driven by `tick()`.
//!
//! Clocking: the TIMER base clock is 16 MHz on silicon (PS §6.30:
//! `fTIMER = 16 MHz / 2^PRESCALER`; Renode `NRF52840_Timer.cs`
//! `InitialFrequency = 16000000`, `Divider = 1 << PRESCALER`), but the
//! nRF52 Cortex-M4 runs at 64 MHz. A descriptor declares the base clock with
//! `config: { peripheral_hz: 16_000_000 }`; the model then converts CPU
//! cycles (`attach_cpu_hz`) to base ticks with an exact integer residue.
//! Without the key, or on a bus with no `cpu_hz`, one CPU cycle is one base
//! tick (the former behaviour, 4x fast on a 64 MHz part).

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

// ── Register offsets (PS §6.30.13, table 159) ────────────────────────────────

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_TASKS_COUNT: u64 = 0x008;
const OFF_TASKS_CLEAR: u64 = 0x00C;
const OFF_TASKS_SHUTDOWN: u64 = 0x010;
const OFF_TASKS_CAPTURE0: u64 = 0x040;
const OFF_TASKS_CAPTURE5: u64 = 0x054;
const OFF_EVENTS_COMPARE0: u64 = 0x140;
const OFF_EVENTS_COMPARE5: u64 = 0x154;
const OFF_SHORTS: u64 = 0x200;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_MODE: u64 = 0x504;
const OFF_BITMODE: u64 = 0x508;
const OFF_PRESCALER: u64 = 0x510;
const OFF_CC0: u64 = 0x540;
const OFF_CC5: u64 = 0x554;

// SHORTS bits (PS table 161): COMPARE[i]_CLEAR at bit i, COMPARE[i]_STOP at bit i+8.
const SHORT_COMPARE_CLEAR_SHIFT: u32 = 0;
const SHORT_COMPARE_STOP_SHIFT: u32 = 8;

// INTEN bits: COMPARE[i] at bit i+16 (PS table 160).
const INTEN_COMPARE_SHIFT: u32 = 16;

// MODE values (PS table 161): 0=Timer, 1=Counter, 2=LowPowerCounter.
const MODE_TIMER: u32 = 0;

#[derive(Debug)]
pub struct Nrf52Timer {
    /// Number of CC/EVENTS_COMPARE/TASKS_CAPTURE channels present on this
    /// instance. TIMER0/1/2 = 4; TIMER3/4 = 6. Default: 4.
    num_cc: usize,
    events_compare: [u32; 6],
    shorts: u32,
    inten: u32,
    mode: u32,
    bitmode: u32,
    prescaler: u32,
    cc: [u32; 6],

    // Dynamic state — driven by tick() / scheduler sync.
    running: bool,
    counter: u32,
    prescaler_accum: u32,

    /// Base clock from `config: { peripheral_hz }`. `None` = CPU clock.
    peripheral_hz: Option<u64>,
    /// Core clock from `attach_cpu_hz`. Zero on a hand-built bus.
    cpu_hz: u64,
    /// CPU-cycle to base-tick residue, in units of `peripheral_hz / cpu_hz`
    /// base ticks: `base = (clock_frac + cycles * peripheral_hz) / cpu_hz`.
    clock_frac: u64,

    /// Bus-published cycle clock (event-scheduler builds). When present the
    /// model is walk-independent: counter advances lazily via `sync_to` and
    /// compare matches ride scheduled events.
    clock: Option<CycleClock>,
    /// CPU cycle of the last `sync_to` / advance.
    anchor: u64,
    /// Bumped when the armed instant CHANGES, so stale compare events die on
    /// arrival. Deliberately NOT bumped on a re-arm that targets the cycle
    /// already in flight — see `take_scheduled_events`.
    arm_seq: u32,
    /// True while a compare event is live in the scheduler.
    scheduled: bool,
    /// Absolute CPU cycle the live wake targets, or `None` when nothing is
    /// armed. Paired with `arm_seq` to recognise a redundant re-arm, and to
    /// keep a wake that is already earlier than a new target (it re-arms on
    /// arrival) instead of stacking a second one.
    armed_target: Option<u64>,
}

impl Default for Nrf52Timer {
    fn default() -> Self {
        Self {
            num_cc: 4,
            events_compare: [0u32; 6],
            shorts: 0,
            inten: 0,
            mode: 0,
            bitmode: 0,
            prescaler: 0,
            cc: [0u32; 6],
            running: false,
            counter: 0,
            prescaler_accum: 0,
            peripheral_hz: None,
            cpu_hz: 0,
            clock_frac: 0,
            clock: None,
            anchor: 0,
            arm_seq: 0,
            scheduled: false,
            armed_target: None,
        }
    }
}

impl Nrf52Timer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with an explicit CC count. Use `num_cc: 6` for TIMER3/4.
    pub fn new_with_cc(num_cc: usize) -> Self {
        Self {
            num_cc: num_cc.clamp(1, 6),
            ..Self::default()
        }
    }

    /// Declare the TIMER base clock in Hz (`config: { peripheral_hz }`).
    pub fn set_peripheral_hz(&mut self, hz: u64) {
        self.peripheral_hz = Some(hz.max(1));
    }

    /// `(base_hz, cpu_hz)` when the base clock differs from the CPU clock.
    fn clock_ratio(&self) -> Option<(u64, u64)> {
        match self.peripheral_hz {
            Some(base) if self.cpu_hz != 0 && base != self.cpu_hz => Some((base, self.cpu_hz)),
            _ => None,
        }
    }

    /// Convert elapsed CPU cycles to base-clock ticks, keeping the residue.
    fn cpu_cycles_to_base_ticks(&mut self, cycles: u64) -> u64 {
        let Some((base, cpu)) = self.clock_ratio() else {
            return cycles;
        };
        let total = u128::from(self.clock_frac) + u128::from(cycles) * u128::from(base);
        self.clock_frac = (total % u128::from(cpu)) as u64;
        (total / u128::from(cpu)) as u64
    }

    /// CPU cycles until `ticks` (>= 1) more base-clock ticks have elapsed.
    fn base_ticks_to_cpu_cycles(&self, ticks: u64) -> u64 {
        let Some((base, cpu)) = self.clock_ratio() else {
            return ticks;
        };
        let need =
            (u128::from(ticks) * u128::from(cpu)).saturating_sub(u128::from(self.clock_frac));
        need.div_ceil(u128::from(base)).min(u128::from(u64::MAX)) as u64
    }

    /// SHORTS writable mask: COMPARE[0..num_cc)_CLEAR (bits 0..num_cc) +
    /// COMPARE[0..num_cc)_STOP (bits 8..8+num_cc).
    fn shorts_mask(&self) -> u32 {
        let low = (1u32 << self.num_cc) - 1;
        low | (low << 8)
    }

    /// INTEN writable mask: COMPARE[0..num_cc) at bits 16..16+num_cc.
    fn inten_mask(&self) -> u32 {
        let bits = (1u32 << self.num_cc) - 1;
        bits << INTEN_COMPARE_SHIFT
    }

    /// Mask the counter to the active BITMODE width.
    fn counter_mask(&self) -> u32 {
        match self.bitmode & 0x3 {
            0 => 0x0000_FFFF, // 16-bit (reset)
            1 => 0x0000_00FF, // 8-bit
            2 => 0x00FF_FFFF, // 24-bit
            3 => 0xFFFF_FFFF, // 32-bit
            _ => unreachable!(),
        }
    }

    fn scheduler_mode(&self) -> bool {
        self.clock.is_some()
    }

    /// Advance the timer by `cycles` CPU cycles (one legacy `tick()` call is
    /// one cycle). Collects compare matches into `fired` / `irq`.
    fn advance_cycles(&mut self, cycles: u64) -> PeripheralTickResult {
        if !self.running || self.mode != MODE_TIMER || cycles == 0 {
            return PeripheralTickResult::default();
        }
        let cycles = self.cpu_cycles_to_base_ticks(cycles);
        if cycles == 0 {
            return PeripheralTickResult::default();
        }
        let divider = 1u32 << (self.prescaler & 0xF);
        let mask = self.counter_mask();
        let mut irq = false;
        let mut fired_events = Vec::new();
        let mut remaining = cycles;
        while remaining > 0 {
            let need = (divider - self.prescaler_accum) as u64;
            if remaining < need {
                self.prescaler_accum = self.prescaler_accum.wrapping_add(remaining as u32);
                break;
            }
            remaining -= need;
            self.prescaler_accum = 0;
            self.counter = self.counter.wrapping_add(1) & mask;
            for i in 0..self.num_cc {
                if self.counter == (self.cc[i] & mask) {
                    self.events_compare[i] = 1;
                    fired_events.push(OFF_EVENTS_COMPARE0 as u32 + 4 * i as u32);
                    if (self.inten >> (INTEN_COMPARE_SHIFT + i as u32)) & 1 != 0 {
                        irq = true;
                    }
                    if (self.shorts >> (SHORT_COMPARE_CLEAR_SHIFT + i as u32)) & 1 != 0 {
                        self.counter = 0;
                    }
                    if (self.shorts >> (SHORT_COMPARE_STOP_SHIFT + i as u32)) & 1 != 0 {
                        self.running = false;
                    }
                }
            }
            if !self.running {
                break;
            }
        }
        PeripheralTickResult {
            irq,
            cycles: 1,
            fired_events,
            ..Default::default()
        }
    }

    /// CPU cycles until the next compare match (prescaler-aware). None if
    /// stopped / no finite compare.
    fn cycles_until_next_compare(&self) -> Option<u64> {
        if !self.running || self.mode != MODE_TIMER {
            return None;
        }
        let divider = 1u32 << (self.prescaler & 0xF);
        let mask = self.counter_mask();
        let mut best: Option<u64> = None;
        for i in 0..self.num_cc {
            let target = self.cc[i] & mask;
            let cur = self.counter & mask;
            // Steps of counter until match (at least 1 if already equal —
            // next wrap-around match, since match fires on the increment
            // that lands on CC).
            let steps = if target > cur {
                (target - cur) as u64
            } else {
                // equal or behind: full period to re-hit
                (mask as u64 + 1) - (cur as u64) + (target as u64)
            };
            // Base ticks: finish current prescaler quantum, then (steps-1)
            // full quanta, then the final quantum that increments onto target.
            let first = (divider - self.prescaler_accum) as u64;
            let cycles = self.base_ticks_to_cpu_cycles(first + (steps - 1) * divider as u64);
            best = Some(best.map_or(cycles, |b| b.min(cycles)));
        }
        best
    }
}

impl Peripheral for Nrf52Timer {
    /// Not in the per-cycle walk while idle. `tick()` above early-returns a
    /// default `PeripheralTickResult` in exactly this state, so skipping the
    /// visit removes dispatch and never an effect — byte-identical.
    ///
    /// A stopped timer, or one in COUNTER mode (which advances only on TASKS_COUNT, an MMIO write), has nothing to advance per cycle.
    ///
    /// Paired with `legacy_tick_dynamic() -> true` because this condition can
    /// change during the model's own tick; the bus also re-arms via
    /// `refresh_legacy_tick_index()` on every MMIO write, which is what makes
    /// the wake path (a firmware write to the start/trigger task) safe.
    fn legacy_tick_active(&self) -> bool {
        self.running && self.mode == MODE_TIMER
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let val = match offset {
            OFF_TASKS_START | OFF_TASKS_STOP | OFF_TASKS_COUNT | OFF_TASKS_CLEAR
            | OFF_TASKS_SHUTDOWN => 0,

            // TASKS_CAPTURE[0..5] — read-as-zero.
            OFF_TASKS_CAPTURE0..=OFF_TASKS_CAPTURE5 if offset.is_multiple_of(4) => 0,

            // EVENTS_COMPARE[i]: return 0 for i >= num_cc (register absent).
            OFF_EVENTS_COMPARE0..=OFF_EVENTS_COMPARE5 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_EVENTS_COMPARE0) / 4) as usize;
                if i < self.num_cc {
                    self.events_compare[i]
                } else {
                    0
                }
            }

            // SHORTS readback is masked to valid bits for this instance.
            OFF_SHORTS => self.shorts & self.shorts_mask(),

            // INTENSET/INTENCLR readback masked to valid compare bits.
            OFF_INTENSET | OFF_INTENCLR => self.inten & self.inten_mask(),

            OFF_MODE => self.mode,
            OFF_BITMODE => self.bitmode,
            OFF_PRESCALER => self.prescaler,

            // CC[i]: return 0 for i >= num_cc.
            OFF_CC0..=OFF_CC5 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_CC0) / 4) as usize;
                if i < self.num_cc {
                    self.cc[i]
                } else {
                    0
                }
            }

            _ => {
                crate::census_reg!("nrf52.timer:Nrf52Timer", offset, "read");
                0
            }
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START
                if value & 1 != 0 => {
                    self.running = true;
                }
            OFF_TASKS_STOP | OFF_TASKS_SHUTDOWN
                if value & 1 != 0 => {
                    self.running = false;
                }
            OFF_TASKS_COUNT
                // Counter mode advance — gated by MODE != Timer.
                if value & 1 != 0 && self.mode != MODE_TIMER => {
                    self.counter = (self.counter.wrapping_add(1)) & self.counter_mask();
                }
            OFF_TASKS_CLEAR
                if value & 1 != 0 => {
                    self.counter = 0;
                    self.prescaler_accum = 0;
                }
            // TASKS_CAPTURE[i]: only valid for i < num_cc.
            OFF_TASKS_CAPTURE0..=OFF_TASKS_CAPTURE5 if offset.is_multiple_of(4)
                && value & 1 != 0 => {
                    let i = ((offset - OFF_TASKS_CAPTURE0) / 4) as usize;
                    if i < self.num_cc {
                        self.cc[i] = self.counter;
                    }
                }

            // EVENTS_COMPARE: hardware-generated; SW may only clear (write 0).
            // Writes of 1 are silently ignored — the HW sets these on compare
            // match in tick(). Only writes of 0 that fall within num_cc clear.
            OFF_EVENTS_COMPARE0..=OFF_EVENTS_COMPARE5 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_EVENTS_COMPARE0) / 4) as usize;
                if i < self.num_cc && value == 0 {
                    self.events_compare[i] = 0;
                }
            }

            // SHORTS: accept only bits valid for this instance.
            OFF_SHORTS => self.shorts = value & self.shorts_mask(),

            // INTENSET/INTENCLR: mask to valid compare bits.
            OFF_INTENSET => self.inten |= value & self.inten_mask(),
            OFF_INTENCLR => self.inten &= !value,

            OFF_MODE => self.mode = value & 0x3,
            OFF_BITMODE => self.bitmode = value & 0x3,
            OFF_PRESCALER => self.prescaler = value & 0xF,

            // CC[i]: only valid for i < num_cc.
            // Silicon stores the full 32-bit value; the BITMODE mask is applied
            // only at compare time (counter & bitmode_mask == cc & bitmode_mask),
            // not at register write time. Masking at write would cause readback
            // to differ from silicon for the upper bytes when BITMODE < 32.
            OFF_CC0..=OFF_CC5 if offset.is_multiple_of(4) => {
                let i = ((offset - OFF_CC0) / 4) as usize;
                if i < self.num_cc {
                    self.cc[i] = value;
                }
            }

            _ => { crate::census_reg!("nrf52.timer:Nrf52Timer", offset, "write"); }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Legacy / feature-off path. Scheduler mode skips the walk.
        self.advance_cycles(1)
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

    fn attach_cpu_hz(&mut self, hz: u64) {
        self.cpu_hz = hz;
    }

    fn sync_to(&mut self, now_cycle: u64) {
        if !self.scheduler_mode() {
            return;
        }
        if now_cycle <= self.anchor {
            return;
        }
        let delta = now_cycle - self.anchor;
        self.anchor = now_cycle;
        let _ = self.advance_cycles(delta);
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() || !self.running || self.mode != MODE_TIMER {
            self.scheduled = false;
            self.armed_target = None;
            return Vec::new();
        }
        let Some(d) = self.cycles_until_next_compare() else {
            self.scheduled = false;
            self.armed_target = None;
            return Vec::new();
        };
        // `d` counts from `anchor`, so this is the absolute cycle the compare
        // lands on — the same instant the scheduler derives as its deadline.
        let target = self.anchor.saturating_add(d);

        // Re-arm with a FRESH token only when the instant actually moved (a CC
        // rewrite, a prescaler change, a restart); a stale wake must then die on
        // arrival, which is what the token check in `on_event` is for.
        //
        // Re-arming the instant already in flight must reuse the token instead.
        // This drain runs after EVERY MMIO write to the peripheral, and reading
        // the nRF52 counter is defined as a write (strobe `TASKS_CAPTURE[i]`,
        // then read `CC[i]` — there is no COUNTER register). A fresh token every
        // time makes the scheduler's dedup key `(peripheral_idx, event_token,
        // deadline)` unique on every poll, so dedup can never fire and each poll
        // leaves another live wake behind. With a far compare those never drain
        // and the peripheral walks into `MAX_LIVE_EVENTS_PER_PERIPHERAL` — the
        // unbounded-heap class the scheduler docs describe. Holding the token
        // steady keeps the key identical, so layer-1 dedup collapses the repeat.
        //
        // A LATER instant needs no new wake at all. The wake already in flight
        // arrives first; `on_event` then advances to it, finds no compare yet,
        // and re-arms for the current target under the same token. Arming a
        // fresh one instead left the earlier wake resident until it fired and
        // died: CODAL's micro:bit V2 retargets TIMER3's compare later over and
        // over between firings, and those stale wakes walked the timer past
        // `MAX_LIVE_EVENTS_PER_PERIPHERAL` (9 live, ceiling 8). Only an EARLIER
        // instant takes a fresh token, so at most the superseded later wake
        // stays resident beside the live one.
        if let (true, Some(live)) = (self.scheduled, self.armed_target) {
            if target > live {
                return Vec::new();
            }
        }
        if !(self.scheduled && self.armed_target == Some(target)) {
            self.arm_seq = self.arm_seq.wrapping_add(1);
        }
        self.scheduled = true;
        self.armed_target = Some(target);
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
        // Advance to the event's cycle; compare latch happens inside advance.
        // Keep `arm_seq` unchanged so `reschedule_delay` reuses this token
        // (Machine reschedules with the same event_token).
        let now = sched.now();
        let res = if now > self.anchor {
            let delta = now - self.anchor;
            self.anchor = now;
            self.advance_cycles(delta)
        } else {
            PeripheralTickResult::default()
        };
        self.scheduled = self.running && self.mode == MODE_TIMER;
        let next = self.cycles_until_next_compare();
        // `reschedule_delay` re-arms under the SAME token, so record the instant
        // it targets. Without this, `armed_target` would still name the compare
        // that just fired and the next drain would count as a change, bumping
        // the token and killing the reschedule it had just handed back.
        self.armed_target = if self.scheduled {
            next.map(|d| now.saturating_add(d))
        } else {
            None
        };
        crate::sched::EventResult {
            raise_own_irq: res.irq,
            fired_events: res.fired_events,
            reschedule_delay: next.map(|d| d.saturating_sub(1)),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmode_round_trips() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        assert_eq!(t.read_u32(OFF_BITMODE).unwrap(), 3);
        t.write_u32(OFF_BITMODE, 0xFFFF_FFFF).unwrap();
        assert_eq!(t.read_u32(OFF_BITMODE).unwrap(), 0x3);
    }

    #[test]
    fn prescaler_masks_to_4_bits() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_PRESCALER, 0xFFFF_FFFF).unwrap();
        assert_eq!(t.read_u32(OFF_PRESCALER).unwrap(), 0xF);
    }

    #[test]
    fn cc_array_full_width() {
        // Default instance has 4 CCs (TIMER0/1/2). Use new_with_cc(6) to
        // exercise the full 6-CC path (TIMER3/4).
        let mut t = Nrf52Timer::new_with_cc(6);
        // BITMODE=3 (32-bit) so the full value survives CC masking.
        t.write_u32(OFF_BITMODE, 3).unwrap();
        for i in 0..6u64 {
            t.write_u32(OFF_CC0 + i * 4, 0xDEAD_0000 | i as u32)
                .unwrap();
        }
        for i in 0..6u64 {
            assert_eq!(t.read_u32(OFF_CC0 + i * 4).unwrap(), 0xDEAD_0000 | i as u32);
        }
    }

    #[test]
    fn cc_above_num_cc_reads_zero() {
        // TIMER0/1/2 have 4 CCs; CC[4] and CC[5] are absent.
        let mut t = Nrf52Timer::new(); // default num_cc=4
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_CC0 + 4 * 4, 0xDEAD_BEEF).unwrap(); // CC[4] — ignored
        t.write_u32(OFF_CC0 + 5 * 4, 0xCAFE_BABE).unwrap(); // CC[5] — ignored
        assert_eq!(t.read_u32(OFF_CC0 + 4 * 4).unwrap(), 0); // absent
        assert_eq!(t.read_u32(OFF_CC0 + 5 * 4).unwrap(), 0); // absent
    }

    #[test]
    fn shorts_masked_to_num_cc() {
        let mut t4 = Nrf52Timer::new(); // 4 CC
        t4.write_u32(OFF_SHORTS, 0x3F3F).unwrap();
        // Only bits 0..4 (CLEAR) + 8..12 (STOP) survive: 0x0F0F.
        assert_eq!(t4.read_u32(OFF_SHORTS).unwrap(), 0x0F0F);

        let mut t6 = Nrf52Timer::new_with_cc(6); // 6 CC
        t6.write_u32(OFF_SHORTS, 0x3F3F).unwrap();
        assert_eq!(t6.read_u32(OFF_SHORTS).unwrap(), 0x3F3F);
    }

    #[test]
    fn inten_masked_to_num_cc() {
        let mut t4 = Nrf52Timer::new(); // 4 CC
        t4.write_u32(OFF_INTENSET, 0x003F_0000).unwrap();
        // Only bits 16..20 survive: 0x000F_0000.
        assert_eq!(t4.read_u32(OFF_INTENSET).unwrap(), 0x000F_0000);

        let mut t6 = Nrf52Timer::new_with_cc(6); // 6 CC
        t6.write_u32(OFF_INTENSET, 0x003F_0000).unwrap();
        assert_eq!(t6.read_u32(OFF_INTENSET).unwrap(), 0x003F_0000);
    }

    #[test]
    fn events_write_one_ignored() {
        // Silicon: EVENTS_* are hardware-generated; SW write of 1 is a no-op.
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_EVENTS_COMPARE0, 1).unwrap();
        assert_eq!(
            t.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
            0,
            "write-1 must be ignored"
        );
        // Write 0 is the clear path (from firmware ISR ack).
        // Seed it via tick() compare match, then clear.
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 0).unwrap();
        t.write_u32(OFF_CC0, 1).unwrap();
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        t.tick();
        assert_eq!(
            t.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
            1,
            "tick compare must set event"
        );
        t.write_u32(OFF_EVENTS_COMPARE0, 0).unwrap();
        assert_eq!(
            t.read_u32(OFF_EVENTS_COMPARE0).unwrap(),
            0,
            "write-0 must clear event"
        );
    }

    #[test]
    fn intenset_intenclr_alias_inten() {
        // Compare-interrupt bits are 16..16+num_cc. Use the correct bit positions
        // (PS §6.30.13, table 160): COMPARE[0..3] at bits 16..19 for a 4-CC instance.
        let mut t = Nrf52Timer::new(); // num_cc=4
        let bits = 0b0111_u32 << 16; // COMPARE[0..2] → bits 16, 17, 18
        t.write_u32(OFF_INTENSET, bits).unwrap();
        assert_eq!(t.read_u32(OFF_INTENSET).unwrap(), bits);
        assert_eq!(t.read_u32(OFF_INTENCLR).unwrap(), bits);
        t.write_u32(OFF_INTENCLR, 0b0010_u32 << 16).unwrap(); // clear bit 17
        assert_eq!(t.read_u32(OFF_INTENSET).unwrap(), 0b0101_u32 << 16);
    }

    #[test]
    fn tasks_read_as_zero_even_after_write() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        assert_eq!(t.read_u32(OFF_TASKS_START).unwrap(), 0);
    }

    #[test]
    fn tick_advances_counter_through_prescaler() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap(); // 32-bit
        t.write_u32(OFF_PRESCALER, 0).unwrap(); // divider = 1
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        // PRESCALER=0 → 1:1, expect counter==10.
        assert_eq!(t.counter, 10);
    }

    #[test]
    fn cc_match_sets_event_and_pends_irq() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 0).unwrap();
        t.write_u32(OFF_CC0, 5).unwrap();
        t.write_u32(OFF_INTENSET, 1 << 16).unwrap(); // COMPARE[0]
        t.write_u32(OFF_TASKS_START, 1).unwrap();

        let mut irq_count = 0;
        for _ in 0..5 {
            if t.tick().irq {
                irq_count += 1;
            }
        }
        // After 5 ticks at PRESCALER=0: counter advanced 0→5 → CC[0] match.
        assert_eq!(t.counter, 5);
        assert_eq!(t.read_u32(OFF_EVENTS_COMPARE0).unwrap(), 1);
        assert_eq!(irq_count, 1);
    }

    #[test]
    fn shorts_compare0_clear_resets_counter() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 0).unwrap();
        t.write_u32(OFF_CC0, 3).unwrap();
        t.write_u32(OFF_SHORTS, 1).unwrap(); // COMPARE[0]_CLEAR
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        // Sequence: 1,2,3(→0),1,2,3(→0),1,2,3(→0),1 → counter==1 after 10 ticks.
        assert_eq!(t.counter, 1);
    }

    #[test]
    fn shorts_compare0_stop_halts_timer() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 0).unwrap();
        t.write_u32(OFF_CC0, 3).unwrap();
        t.write_u32(OFF_SHORTS, 1 << 8).unwrap(); // COMPARE[0]_STOP
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        // Stopped at counter==3 after 3 ticks; remaining 7 ticks no-op.
        assert_eq!(t.counter, 3);
        assert!(!t.running);
    }

    #[test]
    fn prescaler_divider_slows_counter() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 3).unwrap(); // divider = 8
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..32 {
            t.tick();
        }
        // 32 ticks / 8 = 4 increments.
        assert_eq!(t.counter, 4);
    }

    #[test]
    fn capture_records_current_counter() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 0).unwrap();
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..7 {
            t.tick();
        }
        t.write_u32(OFF_TASKS_CAPTURE0, 1).unwrap();
        assert_eq!(t.read_u32(OFF_CC0).unwrap(), 7);
    }

    #[test]
    fn tasks_clear_resets_counter_and_prescaler() {
        let mut t = Nrf52Timer::new();
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 0).unwrap();
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..10 {
            t.tick();
        }
        t.write_u32(OFF_TASKS_CLEAR, 1).unwrap();
        assert_eq!(t.counter, 0);
        assert_eq!(t.prescaler_accum, 0);
    }

    /// nRF52: 64 MHz CPU, 16 MHz TIMER base clock. Four CPU cycles make one
    /// base tick; PRESCALER divides the base clock, not the CPU clock.
    #[test]
    fn peripheral_hz_divides_the_cpu_clock() {
        let mut t = Nrf52Timer::new();
        t.attach_cpu_hz(64_000_000);
        t.set_peripheral_hz(16_000_000);
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_PRESCALER, 4).unwrap(); // 1 MHz
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        for _ in 0..(64 * 10 - 1) {
            t.tick();
        }
        assert_eq!(t.counter, 9, "one cycle short of 10 us");
        t.tick();
        assert_eq!(t.counter, 10, "640 CPU cycles at 64 MHz = 10 us at 1 MHz");
    }

    #[test]
    fn next_compare_is_counted_in_cpu_cycles() {
        let mut t = Nrf52Timer::new();
        t.attach_cpu_hz(64_000_000);
        t.set_peripheral_hz(16_000_000);
        t.write_u32(OFF_BITMODE, 3).unwrap();
        t.write_u32(OFF_CC0, 10).unwrap();
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        assert_eq!(t.cycles_until_next_compare(), Some(40));
        // One CPU cycle leaves a quarter base tick in the residue.
        let _ = t.advance_cycles(1);
        assert_eq!(t.cycles_until_next_compare(), Some(39));
        let r = t.advance_cycles(39);
        assert_eq!(t.counter, 10);
        assert_eq!(
            t.events_compare[0], 1,
            "compare lands on the predicted cycle"
        );
        assert!(!r.irq, "INTEN clear");
    }

    /// No `peripheral_hz`, or no `cpu_hz` on the bus: one cycle per base tick.
    #[test]
    fn clock_ratio_needs_both_clocks() {
        let mut t = Nrf52Timer::new();
        t.set_peripheral_hz(16_000_000);
        t.write_u32(OFF_TASKS_START, 1).unwrap();
        t.tick();
        assert_eq!(t.counter, 1, "cpu_hz 0: ratio off");
        let mut u = Nrf52Timer::new();
        u.attach_cpu_hz(64_000_000);
        u.write_u32(OFF_TASKS_START, 1).unwrap();
        u.tick();
        assert_eq!(u.counter, 1, "no peripheral_hz: ratio off");
    }
}
