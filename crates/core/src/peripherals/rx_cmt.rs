// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RX compare match timer (CMT), one unit = `CMSTRn` + two 16-bit
//! channels.
//!
//! Layout (RX261 Group User's Manual: Hardware, R01UH1045, "Compare Match
//! Timer"; RDP `iodefine.h` `st_cmt`/`st_cmt0`): `CMSTR0` +0x00 (STR0 bit0,
//! STR1 bit1), channel 0 `CMCR` +0x02 / `CMCNT` +0x04 / `CMCOR` +0x06,
//! channel 1 `CMCR` +0x08 / `CMCNT` +0x0A / `CMCOR` +0x0C. Unit 0 (CMT0/1)
//! is at 0x00088000, unit 1 (CMT2/3) at 0x00088010 with the same layout.
//!
//! Modelled:
//! - each started channel counts up at PCLKB / 8, 32, 128 or 512
//!   (`CMCR.CKS`), derived lazily from the bus cycle count at `clock_hz`;
//! - when `CMCNT` reaches `CMCOR` the next count clears it to 0 and raises
//!   compare match CMIn, an ICU interrupt request on the channel's vector,
//!   if `CMCR.CMIE` is set. A count above `CMCOR` runs up to 0xFFFF and
//!   wraps to 0 first, without a request;
//! - `CMCNT`/`CMCOR` writes take effect at once; stopping a channel freezes
//!   its count.
//!
//! The requests leave as `explicit_irqs` holding ICU vector numbers (RX261:
//! CMI0 28, CMI1 29, CMI2 30, CMI3 31), which reach [`crate::cpu::Rx`]'s
//! ICU through `set_exception_pending`.
//!
//! NOT modelled: DTC activation by CMI, and any skew when firmware moves
//! PCLKB away from `pclkb_hz`.

use crate::{CycleClock, Peripheral, SimResult};

const CMSTR: u64 = 0x00;
const CMIE: u16 = 1 << 6;

/// EK-RX261 under Zephyr: ICLK 64 MHz.
pub const DEFAULT_CLOCK_HZ: u64 = 64_000_000;
/// EK-RX261 under Zephyr: PCLKB 32 MHz (the CMT runs at PCLKB / 8 = 4 MHz,
/// `&cmt { clock-frequency = <DT_FREQ_M(4)>; }`).
pub const DEFAULT_PCLKB_HZ: u64 = 32_000_000;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Channel {
    cmcr: u16,
    cmcor: u16,
    /// Count at `anchor_tick` (count-source ticks since cycle 0).
    anchor_count: u16,
    anchor_tick: u64,
    running: bool,
    /// Count-source tick of the next compare-match clear.
    armed: Option<u64>,
    /// CPU cycle of this channel's one live scheduler wake. A re-arm whose
    /// match is no earlier keeps it (the wake then reschedules itself to the
    /// match); only an earlier match queues a new wake under a new
    /// generation, and the superseded one dies on arrival. One wake per
    /// re-arm would pile up past the scheduler's per-peripheral ceiling
    /// under Zephyr's tickless `sys_clock_set_timeout`.
    live_at: Option<u64>,
    generation: u32,
}

impl Channel {
    fn divider(&self) -> u64 {
        [8, 32, 128, 512][usize::from(self.cmcr & 3)]
    }
}

/// One CMT unit (two channels).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RxCmt {
    clock_hz: u64,
    pclkb_hz: u64,
    vectors: [u32; 2],
    ch: [Channel; 2],
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

/// Scheduler token: channel in bit 0, generation above it.
fn token(ch: usize, generation: u32) -> u32 {
    (generation << 1) | ch as u32
}

impl RxCmt {
    pub fn new(clock_hz: u64, pclkb_hz: u64, vectors: [u32; 2]) -> Self {
        Self {
            clock_hz: clock_hz.max(1),
            pclkb_hz: pclkb_hz.max(1),
            vectors,
            ch: [Channel::default(), Channel::default()],
            pending_events: Vec::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now_cycle(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn source_hz(&self, ch: usize) -> u64 {
        (self.pclkb_hz / self.ch[ch].divider()).max(1)
    }

    fn ticks_at(&self, ch: usize, cycle: u64) -> u64 {
        (u128::from(cycle) * u128::from(self.source_hz(ch)) / u128::from(self.clock_hz)) as u64
    }

    /// First CPU cycle at which `ticks` count-source ticks have elapsed.
    fn cycle_of_tick(&self, ch: usize, ticks: u64) -> u64 {
        let hz = u128::from(self.source_hz(ch));
        ((u128::from(ticks) * u128::from(self.clock_hz)).div_ceil(hz)) as u64
    }

    /// The counter `n` count-source ticks after the anchor.
    fn count_after(&self, ch: usize, n: u64) -> u16 {
        let c = &self.ch[ch];
        let period = u64::from(c.cmcor) + 1;
        let start = u64::from(c.anchor_count);
        if start <= u64::from(c.cmcor) {
            ((start + n) % period) as u16
        } else {
            // Above CMCOR: run to 0xFFFF, wrap to 0, then match normally.
            let to_wrap = 0x1_0000 - start;
            if n < to_wrap {
                (start + n) as u16
            } else {
                ((n - to_wrap) % period) as u16
            }
        }
    }

    fn count(&self, ch: usize) -> u16 {
        let c = &self.ch[ch];
        if !c.running || self.clock.is_none() {
            return c.anchor_count;
        }
        let n = self
            .ticks_at(ch, self.now_cycle())
            .saturating_sub(c.anchor_tick);
        self.count_after(ch, n)
    }

    /// Ticks from the anchor to the next compare-match clear.
    fn ticks_to_match(&self, ch: usize) -> u64 {
        let c = &self.ch[ch];
        let start = u64::from(c.anchor_count);
        let cor = u64::from(c.cmcor);
        if start <= cor {
            cor + 1 - start
        } else {
            (0x1_0000 - start) + cor + 1
        }
    }

    fn rearm(&mut self, ch: usize) {
        self.ch[ch].armed = None;
        if !self.ch[ch].running || self.clock.is_none() {
            return;
        }
        let tick = self.ch[ch].anchor_tick + self.ticks_to_match(ch);
        self.ch[ch].armed = Some(tick);
        if self.scheduler_mode() {
            let now = self.now_cycle();
            let fire = self.cycle_of_tick(ch, tick).max(now + 1);
            if self.ch[ch].live_at.is_some_and(|live| live <= fire) {
                return;
            }
            self.ch[ch].generation = self.ch[ch].generation.wrapping_add(1);
            self.ch[ch].live_at = Some(fire);
            let generation = self.ch[ch].generation;
            self.pending_events
                .push((fire - now, token(ch, generation)));
        }
    }

    /// Freeze the count, apply `f`, re-anchor and re-arm.
    fn reanchor_with(&mut self, ch: usize, f: impl FnOnce(&mut Channel)) {
        let count = self.count(ch);
        f(&mut self.ch[ch]);
        self.ch[ch].anchor_count = count;
        self.ch[ch].anchor_tick = if self.clock.is_some() {
            self.ticks_at(ch, self.now_cycle())
        } else {
            0
        };
        self.rearm(ch);
    }

    /// Take the armed compare match of `ch`: the counter is 0 on that edge.
    fn fire(&mut self, ch: usize) -> Option<u32> {
        let tick = self.ch[ch].armed.take()?;
        self.ch[ch].anchor_count = 0;
        self.ch[ch].anchor_tick = tick;
        self.rearm(ch);
        (self.ch[ch].cmcr & CMIE != 0).then_some(self.vectors[ch])
    }

    fn armed_cycle(&self, ch: usize) -> Option<u64> {
        self.ch[ch].armed.map(|t| self.cycle_of_tick(ch, t))
    }

    fn read_u16_reg(&self, off: u64) -> u16 {
        match off {
            CMSTR => u16::from(self.ch[0].running) | (u16::from(self.ch[1].running) << 1),
            0x02 | 0x08 => self.ch[((off - 2) / 6) as usize].cmcr,
            0x04 | 0x0A => self.count(((off - 4) / 6) as usize),
            0x06 | 0x0C => self.ch[((off - 6) / 6) as usize].cmcor,
            _ => 0,
        }
    }

    fn write_u16_reg(&mut self, off: u64, v: u16) {
        match off {
            CMSTR => {
                for ch in 0..2 {
                    let on = v & (1 << ch) != 0;
                    if on != self.ch[ch].running {
                        self.reanchor_with(ch, |c| c.running = on);
                    }
                }
            }
            0x02 | 0x08 => {
                let ch = ((off - 2) / 6) as usize;
                self.reanchor_with(ch, |c| c.cmcr = v & 0x00C3);
            }
            0x04 | 0x0A => {
                let ch = ((off - 4) / 6) as usize;
                let count = v;
                self.reanchor_with(ch, |_| {});
                self.ch[ch].anchor_count = count;
                self.rearm(ch);
            }
            0x06 | 0x0C => {
                let ch = ((off - 6) / 6) as usize;
                self.reanchor_with(ch, |c| c.cmcor = v);
            }
            _ => {}
        }
    }
}

impl Default for RxCmt {
    fn default() -> Self {
        Self::new(DEFAULT_CLOCK_HZ, DEFAULT_PCLKB_HZ, [28, 29])
    }
}

impl Peripheral for RxCmt {
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && self.ch.iter().any(|c| c.armed.is_some())
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
        for ch in 0..2 {
            while self
                .armed_cycle(ch)
                .is_some_and(|at| self.now_cycle() >= at)
            {
                irqs.extend(self.fire(ch));
            }
        }
        crate::PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let off = offset & 0xF;
        let word = self.read_u16_reg(off & !1);
        Ok(if off & 1 == 0 {
            word as u8
        } else {
            (word >> 8) as u8
        })
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(self.read_u16_reg(offset & 0xE))
    }

    /// Byte writes merge into the 16-bit register (the RX bus allows them).
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = offset & 0xF;
        let word = self.read_u16_reg(off & !1);
        let merged = if off & 1 == 0 {
            (word & 0xFF00) | u16::from(value)
        } else {
            (word & 0x00FF) | (u16::from(value) << 8)
        };
        self.write_u16_reg(off & !1, merged);
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write_u16_reg(offset & 0xE, value);
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
        let ch = (event_token & 1) as usize;
        if token(ch, self.ch[ch].generation) != event_token {
            // Superseded by an earlier wake.
            return crate::sched::EventResult::default();
        }
        self.ch[ch].live_at = None;
        let now = self.now_cycle();
        let Some(due) = self.armed_cycle(ch) else {
            return crate::sched::EventResult::default();
        };
        if due > now {
            // The match moved later since this wake was queued: follow it.
            self.ch[ch].live_at = Some(due);
            return crate::sched::EventResult {
                reschedule_delay: Some(due - now),
                ..Default::default()
            };
        }
        let irq = self.fire(ch);
        // The next match `fire` armed goes back as a reschedule of this
        // token, and the generation returns to it to match.
        let reschedule_delay = self.pending_events.pop().map(|(delay, _)| {
            self.ch[ch].generation = event_token >> 1;
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
        *self = Self::new(self.clock_hz, self.pclkb_hz, self.vectors);
        self.clock = clock;
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: RxCmt = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("RxCmt snapshot decode: {error}"))
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

    /// CPU at 64 MHz, PCLKB 32 MHz: CKS=0 counts every 16 CPU cycles.
    fn cmt_at(cycle: u64) -> (RxCmt, CycleClock) {
        let clock = CycleClock::default();
        clock.publish(cycle);
        let mut t = RxCmt::default();
        t.attach_cycle_clock(clock.clone());
        (t, clock)
    }

    /// Advance to `cycle`, delivering every armed match on the way the way
    /// the build's time base does (legacy walk or scheduled event).
    fn walk_to(t: &mut RxCmt, clock: &CycleClock, cycle: u64) -> Vec<u32> {
        let mut out = Vec::new();
        loop {
            let next = (0..2)
                .filter_map(|ch| t.armed_cycle(ch).map(|at| (at, ch)))
                .filter(|&(at, _)| at <= cycle)
                .min();
            let Some((at, ch)) = next else { break };
            clock.publish(at);
            if t.scheduler_mode() {
                out.extend(t.fire(ch));
            } else {
                out.extend(t.tick_elapsed(1).explicit_irqs.unwrap_or_default());
            }
        }
        clock.publish(cycle);
        out
    }

    #[test]
    fn counts_at_pclkb_over_cks_and_clears_on_match() {
        let (mut t, clock) = cmt_at(0);
        t.write_u16(0x02, 0x00C0).unwrap(); // CKS=0, CMIE
        t.write_u16(0x06, 9).unwrap(); // CMCOR = 9: period 10 counts
        t.write_u16(CMSTR, 1).unwrap();
        clock.publish(16 * 5);
        assert_eq!(t.read_u16(0x04).unwrap(), 5);
        // The tenth count clears to 0 and raises CMI0 (vector 28).
        let irqs = walk_to(&mut t, &clock, 16 * 10);
        assert_eq!(irqs, vec![28]);
        assert_eq!(t.read_u16(0x04).unwrap(), 0);
        // Periodic.
        assert_eq!(walk_to(&mut t, &clock, 16 * 20), vec![28]);
    }

    #[test]
    fn cmie_clear_still_matches_but_raises_nothing() {
        let (mut t, clock) = cmt_at(0);
        t.write_u16(0x08, 0x0080).unwrap(); // channel 1, CMIE off
        t.write_u16(0x0C, 3).unwrap();
        t.write_u16(CMSTR, 2).unwrap();
        assert!(walk_to(&mut t, &clock, 16 * 9).is_empty());
        assert_eq!(t.read_u16(0x0A).unwrap(), 9 % 4);
        assert_eq!(t.read_u16(CMSTR).unwrap(), 2);
    }

    #[test]
    fn stop_freezes_and_a_count_above_cmcor_wraps_first() {
        let (mut t, clock) = cmt_at(0);
        t.write_u16(0x02, 0x00C1).unwrap(); // CKS=1: /32, 64 CPU cycles
        t.write_u16(0x06, 0xFFFF).unwrap();
        t.write_u16(CMSTR, 1).unwrap();
        clock.publish(64 * 7);
        t.write_u16(CMSTR, 0).unwrap();
        clock.publish(64 * 100);
        assert_eq!(t.read_u16(0x04).unwrap(), 7);
        // Count 0xFFFE above a new CMCOR of 4: wraps through 0xFFFF to 0
        // without a request, then matches after five more counts.
        t.write_u16(0x04, 0xFFFE).unwrap();
        t.write_u16(0x06, 4).unwrap();
        t.write_u16(CMSTR, 1).unwrap();
        let start = clock.now();
        let irqs = walk_to(&mut t, &clock, start + 64 * 6);
        assert!(irqs.is_empty(), "{irqs:?}");
        let irqs = walk_to(&mut t, &clock, start + 64 * 8);
        assert_eq!(irqs, vec![28]);
    }

    #[test]
    fn scheduler_mode_queues_the_match_cycle() {
        let (mut t, _clock) = cmt_at(100);
        t.write_u16(0x02, 0x00C0).unwrap();
        t.write_u16(0x06, 249).unwrap();
        t.take_scheduled_events();
        t.write_u16(CMSTR, 1).unwrap();
        let events = t.take_scheduled_events();
        if t.scheduler_mode() {
            // Anchored at tick 6 (cycle 100 / 16): match after 250 counts.
            let (delay, tok) = *events.last().unwrap();
            assert_eq!(delay, (6 + 250) * 16 - 100);
            assert_eq!(tok & 1, 0);
        } else {
            assert!(events.is_empty());
            assert!(t.legacy_tick_active());
        }
    }

    #[test]
    fn later_compare_rewrites_keep_one_live_wake() {
        // Zephyr's tickless sys_clock_set_timeout rewrites CMCOR on every
        // timeout change; each later match must reuse the live wake.
        let (mut t, _clock) = cmt_at(0);
        t.write_u16(0x02, 0x00C0).unwrap();
        t.write_u16(0x06, 100).unwrap();
        t.write_u16(CMSTR, 1).unwrap();
        let first = t.take_scheduled_events();
        for cor in 200..220 {
            t.write_u16(0x06, cor).unwrap();
        }
        let later = t.take_scheduled_events();
        if t.scheduler_mode() {
            assert_eq!(first.len(), 1);
            assert!(later.is_empty(), "{later:?}");
            // An earlier match supersedes the live wake with a new one.
            t.write_u16(0x06, 50).unwrap();
            let earlier = t.take_scheduled_events();
            assert_eq!(earlier.len(), 1);
            assert_ne!(earlier[0].1, first[0].1);
            assert_eq!(t.armed_cycle(0), Some(t.cycle_of_tick(0, 51)));
        } else {
            assert!(first.is_empty() && later.is_empty());
            assert_eq!(t.armed_cycle(0), Some(t.cycle_of_tick(0, 220)));
        }
    }
}
