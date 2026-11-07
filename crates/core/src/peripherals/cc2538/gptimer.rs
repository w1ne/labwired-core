//! TI CC2538 general-purpose timers GPTIMER0-3 (`0x4003_0000` + n × `0x1000`).
//!
//! Source: SWRU319C §11 "General-Purpose Timers" (the GPTM register map:
//! `CFG` @ `0x000`, `TAMR`/`TBMR` @ `0x004`/`0x008`, `CTL` @ `0x00C`, `SYNC`
//! @ `0x010`, `IMR`/`RIS`/`MIS`/`ICR` @ `0x018`-`0x024`, `TnILR` @
//! `0x028`/`0x02C`, `TnMATCHR` @ `0x030`/`0x034`, `TnPR` @ `0x038`/`0x03C`,
//! `TnPMR` @ `0x040`/`0x044`, `TnR` @ `0x048`/`0x04C`, `TnV` @
//! `0x050`/`0x054`, `RTCPD` @ `0x058`, `TnPS` @ `0x05C`/`0x060`, `TnPV` @
//! `0x064`/`0x068`, `PP` @ `0xFC0`), and Table 5-2 for the lines (GPTimer0A-2B
//! = IRQ 19-24, GPTimer3A/3B = IRQ 35/36 in the regular map).
//!
//! Modelled:
//! - `CFG` = 0 (one 32-bit timer, A) and `CFG` = 4 (two 16-bit timers A and
//!   B, each with the 8-bit `TnPR` as a true prescaler, ÷ (`TnPR` + 1));
//! - one-shot and periodic modes (`TnMR` = 1/2), counting down (from
//!   `TnILR` to 0, then reload) or up (`TnCDIR`: from 0 to `TnILR`, then 0);
//!   a one-shot clears its `TnEN` at the time-out and holds the reload value
//!   (`TnILR` down, 0 up), so setting `TnEN` again runs it once more;
//! - the time-out (`TnTORIS`) and match (`TnMRIS`, gated by `TnMIE`) raw
//!   status, `MIS` = `RIS` & `IMR`, `ICR` write-1-to-clear. Status is derived
//!   from the cycle count when read, so a polled timer needs no walk;
//! - interrupts: A on the block's own line, B on `config.irq_b`. A wake is
//!   armed only for an unmasked event; an `IMR`/`ICR` write that leaves `MIS`
//!   set pends the line again on the next cycle. One live scheduler wake per
//!   half (a generation token), as `rx_cmt`;
//! - `TnILR` writes load a down-counter at once (`TnILD` = 0) and leave an
//!   up-counter alone; `TnV` writes load the counter; a `TnMR` write while
//!   the half is disabled loads its start value (0 up, `TnILR` down);
//!   `TnEN` 0 → 1 resumes from the held count.
//!
//! The timer clock is the CPU clock: one count (before the prescaler) per bus
//! cycle, which holds while `IO_DIV` = `SYS_DIV` (the reset state, Table 7-3).
//!
//! NOT modelled: RTC mode (`CFG` = 1 runs as the 32-bit timer), capture and
//! PWM modes (`TnMR` = 3 and `TnAMS` count as periodic; no edge events,
//! `CnMRIS`/`CnERIS` never set, no CCP pins), `TnSNAPS`, `TnWOT`/`SYNC`
//! (stored only), `TnSTALL`, the ADC trigger (`TnOTE`), `TnILD` = 1 deferred
//! reload, `TnPLO`/`TnMRSU`, the prescaler extending an up-counter (it always
//! divides), and the level re-pend of an interrupt left set when its handler
//! returns without clearing it. `TnPS`/`TnPV` report the prescaler position
//! derived from the cycle count.

use super::{lane_merge, lane_read};
use crate::{CycleClock, Peripheral, SimResult};

const CFG: u64 = 0x000;
const TAMR: u64 = 0x004;
const TBMR: u64 = 0x008;
const CTL: u64 = 0x00C;
const SYNC: u64 = 0x010;
const IMR: u64 = 0x018;
const RIS: u64 = 0x01C;
const MIS: u64 = 0x020;
const ICR: u64 = 0x024;
const TAILR: u64 = 0x028;
const TBILR: u64 = 0x02C;
const TAMATCHR: u64 = 0x030;
const TBMATCHR: u64 = 0x034;
const TAPR: u64 = 0x038;
const TBPR: u64 = 0x03C;
const TAPMR: u64 = 0x040;
const TBPMR: u64 = 0x044;
const TAR: u64 = 0x048;
const TBR: u64 = 0x04C;
const TAV: u64 = 0x050;
const TBV: u64 = 0x054;
const RTCPD: u64 = 0x058;
const TAPS: u64 = 0x05C;
const TBPS: u64 = 0x060;
const TAPV: u64 = 0x064;
const TBPV: u64 = 0x068;
const PP: u64 = 0xFC0;

/// `CFG` = 4: two 16-bit timers.
const CFG_16BIT: u32 = 0x4;
/// `TnMR.TnMR` = 1: one-shot.
const MR_ONE_SHOT: u32 = 0x1;
/// `TnMR.TnCDIR`: count up.
const MR_CDIR: u32 = 1 << 4;
/// `TnMR.TnMIE`: match interrupt enable.
const MR_MIE: u32 = 1 << 5;
/// `TnMR.TnILD`: `TnILR` takes effect at the next time-out.
const MR_ILD: u32 = 1 << 8;
const MR_MASK: u32 = 0xFFF;
/// `CTL.TnEN` for A (bit 0) and B (bit 8).
const CTL_EN: [u32; 2] = [1 << 0, 1 << 8];
const CTL_MASK: u32 = 0x6F6F;
/// `IMR`/`RIS` time-out (`TnTO*`) and match (`TnM*`) bits per half.
const INT_TO: [u32; 2] = [1 << 0, 1 << 8];
const INT_M: [u32; 2] = [1 << 4, 1 << 11];
/// Every `IMR`/`RIS` bit belonging to a half (A also owns `RTCIM`, bit 3).
const INT_HALF: [u32; 2] = [0x001F, 0x0F00];
const IMR_MASK: u32 = 0x0F1F;
/// `RTCPD` reset value (read-only RTC predivider snapshot).
const RTCPD_RESET: u32 = 0x7FFF;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Half {
    mr: u32,
    ilr: u32,
    matchr: u32,
    pr: u32,
    pmr: u32,
    /// `TnEN` as last applied; a finished one-shot clears it on the next
    /// register access (`stopped` covers the reads in between).
    en: bool,
    /// Count at `anchor_tick` (half ticks, i.e. cycles ÷ prescale, since 0).
    anchor_count: u32,
    anchor_tick: u64,
    /// Edges up to and including this tick are already folded into `ris`.
    seen_tick: u64,
    /// `MIS` left set by a register write: pend the line on the next cycle.
    kick: bool,
    /// CPU cycle of the next interrupt delivery (edge or kick).
    armed: Option<u64>,
    /// CPU cycle of this half's one live scheduler wake; only an earlier
    /// delivery queues a new wake under a new generation.
    live_at: Option<u64>,
    generation: u32,
}

impl Half {
    fn new(max: u32) -> Self {
        Self {
            mr: 0,
            ilr: max,
            matchr: max,
            pr: 0,
            pmr: 0,
            en: false,
            anchor_count: max,
            anchor_tick: 0,
            seen_tick: 0,
            kick: false,
            armed: None,
            live_at: None,
            generation: 0,
        }
    }

    fn up(&self) -> bool {
        self.mr & MR_CDIR != 0
    }

    fn one_shot(&self) -> bool {
        self.mr & 0x3 == MR_ONE_SHOT
    }
}

/// Scheduler token: half in bit 0, generation above it.
fn token(h: usize, generation: u32) -> u32 {
    (generation << 1) | h as u32
}

/// One GPTIMER block (timers A and B).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cc2538GpTimer {
    cfg: u32,
    /// `CTL` without the `TnEN` bits (those live in each half).
    ctl: u32,
    sync: u32,
    imr: u32,
    /// Raw status folded in so far; `ris_now` adds the edges since.
    ris: u32,
    halves: [Half; 2],
    /// NVIC line of timer B (timer A uses the block's own `irq:`).
    irq_b: Option<u32>,
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl Cc2538GpTimer {
    pub fn new(irq_b: Option<u32>) -> Self {
        Self {
            cfg: 0,
            ctl: 0,
            sync: 0,
            imr: 0,
            ris: 0,
            halves: [Half::new(u32::MAX), Half::new(0xFFFF)],
            irq_b,
            pending_events: Vec::new(),
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now_cycle(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn split(&self) -> bool {
        self.cfg & 0x7 == CFG_16BIT
    }

    /// Counter width mask of half `h`.
    fn max(&self, h: usize) -> u64 {
        if self.split() || h == 1 {
            0xFFFF
        } else {
            0xFFFF_FFFF
        }
    }

    /// Cycles per count: the prescaler applies to the 16-bit timers only.
    fn div(&self, h: usize) -> u64 {
        if self.split() {
            u64::from(self.halves[h].pr & 0xFF) + 1
        } else {
            1
        }
    }

    fn ilr(&self, h: usize) -> u64 {
        u64::from(self.halves[h].ilr) & self.max(h)
    }

    fn matchr(&self, h: usize) -> u64 {
        u64::from(self.halves[h].matchr) & self.max(h)
    }

    fn now_tick(&self, h: usize) -> u64 {
        self.now_cycle() / self.div(h)
    }

    /// Half `h` counts: enabled, present in this configuration, clocked.
    fn running(&self, h: usize) -> bool {
        self.halves[h].en && (h == 0 || self.split()) && self.clock.is_some()
    }

    /// Ticks since the anchor.
    fn rel_now(&self, h: usize) -> u64 {
        self.now_tick(h).saturating_sub(self.halves[h].anchor_tick)
    }

    /// The count a time-out enters: `TnILR` counting up, 0 counting down.
    fn timeout_value(&self, h: usize) -> u64 {
        if self.halves[h].up() {
            self.ilr(h)
        } else {
            0
        }
    }

    /// The count after a one-shot time-out (and a fresh start): 0 counting
    /// up, `TnILR` counting down.
    fn reload_value(&self, h: usize) -> u32 {
        if self.halves[h].up() {
            0
        } else {
            self.ilr(h) as u32
        }
    }

    /// Ticks after the anchor at which the counter first enters `v`.
    fn first_entry(&self, h: usize, v: u64) -> Option<u64> {
        let start = u64::from(self.halves[h].anchor_count);
        let ilr = self.ilr(h);
        let period = ilr + 1;
        if self.halves[h].up() {
            if start <= ilr {
                if v > ilr {
                    return None;
                }
                let d = (v + period - start) % period;
                Some(if d == 0 { period } else { d })
            } else if v > start {
                Some(v - start)
            } else if v <= ilr {
                // Above `TnILR`: run to the width limit, wrap to 0.
                Some(self.max(h) + 1 - start + v)
            } else {
                None
            }
        } else if v < start {
            Some(start - v)
        } else if v <= ilr {
            // Down to 0, reload `TnILR` on the next count.
            Some(start + 1 + (ilr - v))
        } else {
            None
        }
    }

    /// The tick (after the anchor) at which a one-shot stops.
    fn stop_rel(&self, h: usize) -> Option<u64> {
        if self.halves[h].one_shot() {
            self.first_entry(h, self.timeout_value(h))
        } else {
            None
        }
    }

    /// The counter `n` ticks after the anchor, ignoring a one-shot stop.
    fn count_after(&self, h: usize, n: u64) -> u64 {
        let start = u64::from(self.halves[h].anchor_count);
        let ilr = self.ilr(h);
        let period = ilr + 1;
        if self.halves[h].up() {
            if start <= ilr {
                (start + n) % period
            } else if n < self.max(h) + 1 - start {
                start + n
            } else {
                (n - (self.max(h) + 1 - start)) % period
            }
        } else if n <= start {
            start - n
        } else {
            ilr - ((n - start - 1) % period)
        }
    }

    /// A one-shot that has reached its time-out.
    fn stopped(&self, h: usize) -> bool {
        self.running(h) && self.stop_rel(h).is_some_and(|s| self.rel_now(h) >= s)
    }

    /// The count now (`TnR`/`TnV`).
    fn count(&self, h: usize) -> u32 {
        if !self.running(h) {
            return self.halves[h].anchor_count;
        }
        if self.stopped(h) {
            return self.reload_value(h);
        }
        self.count_after(h, self.rel_now(h)) as u32
    }

    /// Whether the counter enters `v` in the relative ticks `(a, b]`.
    fn edge_between(&self, h: usize, v: u64, a: u64, b: u64) -> bool {
        let Some(first) = self.first_entry(h, v) else {
            return false;
        };
        let b = self.stop_rel(h).map_or(b, |s| b.min(s));
        if first > b {
            return false;
        }
        if first > a {
            return true;
        }
        if v > self.ilr(h) {
            return false; // passed once, outside the reload period
        }
        let period = self.ilr(h) + 1;
        first + ((a - first) / period + 1) * period <= b
    }

    /// The first relative tick after `a` at which the counter enters `v`.
    fn next_entry_after(&self, h: usize, v: u64, a: u64) -> Option<u64> {
        let first = self.first_entry(h, v)?;
        let next = if first > a {
            first
        } else if v > self.ilr(h) {
            return None;
        } else {
            let period = self.ilr(h) + 1;
            first + ((a - first) / period + 1) * period
        };
        match self.stop_rel(h) {
            Some(s) if next > s => None,
            _ => Some(next),
        }
    }

    /// `RIS` bits half `h` raised since its last fold.
    fn unseen_ris(&self, h: usize) -> u32 {
        if !self.running(h) {
            return 0;
        }
        let half = &self.halves[h];
        let a = half.seen_tick.saturating_sub(half.anchor_tick);
        let b = self.rel_now(h);
        if a >= b {
            return 0;
        }
        let mut bits = 0;
        if self.edge_between(h, self.timeout_value(h), a, b) {
            bits |= INT_TO[h];
        }
        if half.mr & MR_MIE != 0 && self.edge_between(h, self.matchr(h), a, b) {
            bits |= INT_M[h];
        }
        bits
    }

    fn ris_now(&self) -> u32 {
        self.ris | self.unseen_ris(0) | self.unseen_ris(1)
    }

    /// Fold the edges so far into `ris` and retire a finished one-shot.
    fn settle(&mut self, h: usize) {
        if !self.running(h) {
            return;
        }
        self.ris |= self.unseen_ris(h);
        let now = self.now_tick(h);
        if self.stopped(h) {
            let reload = self.reload_value(h);
            let half = &mut self.halves[h];
            half.en = false;
            half.anchor_count = reload;
            half.anchor_tick = now;
        }
        self.halves[h].seen_tick = now;
    }

    /// Settle, hold the count, apply `f`, then restart counting from here.
    fn reanchor(&mut self, h: usize, f: impl FnOnce(&mut Self)) {
        self.settle(h);
        self.halves[h].anchor_count = self.count(h);
        f(self);
        let now = self.now_tick(h);
        self.halves[h].anchor_tick = now;
        self.halves[h].seen_tick = now;
        self.rearm(h);
    }

    /// After an `IMR`/`ICR` write: a half whose `MIS` is still set pends its
    /// line again.
    fn status_written(&mut self) {
        for (h, &int_half) in INT_HALF.iter().enumerate() {
            self.halves[h].kick = self.ris & self.imr & int_half != 0;
            self.rearm(h);
        }
    }

    /// CPU cycle of the next interrupt delivery of half `h`.
    fn next_delivery(&self, h: usize) -> Option<u64> {
        self.clock.as_ref()?;
        let now = self.now_cycle();
        if self.halves[h].kick {
            return Some(now + 1);
        }
        if !self.running(h) {
            return None;
        }
        let rel = self.rel_now(h);
        let mut next: Option<u64> = None;
        if self.imr & INT_TO[h] != 0 {
            next = self.next_entry_after(h, self.timeout_value(h), rel);
        }
        if self.imr & INT_M[h] != 0 && self.halves[h].mr & MR_MIE != 0 {
            if let Some(m) = self.next_entry_after(h, self.matchr(h), rel) {
                next = Some(next.map_or(m, |t| t.min(m)));
            }
        }
        next.map(|rel| ((self.halves[h].anchor_tick + rel) * self.div(h)).max(now + 1))
    }

    fn rearm(&mut self, h: usize) {
        let at = self.next_delivery(h);
        self.halves[h].armed = at;
        let Some(fire) = at else { return };
        if self.scheduler_mode() {
            if self.halves[h].live_at.is_some_and(|live| live <= fire) {
                return;
            }
            let half = &mut self.halves[h];
            half.generation = half.generation.wrapping_add(1);
            half.live_at = Some(fire);
            let generation = half.generation;
            let now = self.now_cycle();
            self.pending_events.push((fire - now, token(h, generation)));
        }
    }

    /// Deliver half `h`'s armed edge or kick; true if its line pends.
    fn fire(&mut self, h: usize) -> bool {
        self.halves[h].armed = None;
        self.settle(h);
        self.halves[h].kick = false;
        let pend = self.ris & self.imr & INT_HALF[h] != 0;
        self.rearm(h);
        pend
    }

    fn write_cfg(&mut self, value: u32) {
        for h in 0..2 {
            self.settle(h);
            self.halves[h].anchor_count = self.count(h);
        }
        self.cfg = value & 0x7;
        for h in 0..2 {
            let max = self.max(h) as u32;
            let now = self.now_tick(h);
            let half = &mut self.halves[h];
            half.anchor_count &= max;
            half.anchor_tick = now;
            half.seen_tick = now;
            self.rearm(h);
        }
    }

    fn write_ctl(&mut self, value: u32) {
        let value = value & CTL_MASK;
        self.ctl = value & !(CTL_EN[0] | CTL_EN[1]);
        for (h, &ctl_en) in CTL_EN.iter().enumerate() {
            self.settle(h);
            let en = value & ctl_en != 0;
            if en != self.halves[h].en {
                self.reanchor(h, |t| t.halves[h].en = en);
            }
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        let en_bits = (0..2)
            .filter(|&h| self.halves[h].en && !self.stopped(h))
            .fold(0, |acc, h| acc | CTL_EN[h]);
        match offset {
            CFG => self.cfg,
            TAMR => self.halves[0].mr,
            TBMR => self.halves[1].mr,
            CTL => self.ctl | en_bits,
            SYNC => self.sync,
            IMR => self.imr,
            RIS => self.ris_now(),
            MIS => self.ris_now() & self.imr,
            ICR => 0,
            TAILR => self.halves[0].ilr,
            TBILR => self.halves[1].ilr,
            TAMATCHR => self.halves[0].matchr,
            TBMATCHR => self.halves[1].matchr,
            TAPR => self.halves[0].pr,
            TBPR => self.halves[1].pr,
            TAPMR => self.halves[0].pmr,
            TBPMR => self.halves[1].pmr,
            TAR | TAV => self.count(0),
            TBR | TBV => self.count(1),
            RTCPD => RTCPD_RESET,
            TAPS | TAPV => self.prescale(0),
            TBPS | TBPV => self.prescale(1),
            // PP.SIZE = 0: 16/32-bit timers.
            PP => 0,
            _ => {
                crate::census_reg!("cc2538:Cc2538GpTimer", offset, "read");
                0
            }
        }
    }

    /// The prescaler position, from the cycle count.
    fn prescale(&self, h: usize) -> u32 {
        let div = self.div(h);
        if div == 1 || !self.running(h) {
            return 0;
        }
        let phase = self.now_cycle() % div;
        (if self.halves[h].up() {
            phase
        } else {
            div - 1 - phase
        }) as u32
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            CFG => self.write_cfg(value),
            TAMR | TBMR => {
                let h = usize::from(offset == TBMR);
                self.reanchor(h, |t| {
                    t.halves[h].mr = value & MR_MASK;
                    if !t.halves[h].en {
                        t.halves[h].anchor_count = t.reload_value(h);
                    }
                });
            }
            CTL => self.write_ctl(value),
            SYNC => self.sync = value & 0xFF,
            IMR => {
                self.settle(0);
                self.settle(1);
                self.imr = value & IMR_MASK;
                self.status_written();
            }
            ICR => {
                self.settle(0);
                self.settle(1);
                self.ris &= !(value & IMR_MASK);
                self.status_written();
            }
            TAILR | TBILR => {
                let h = usize::from(offset == TBILR);
                self.reanchor(h, |t| {
                    t.halves[h].ilr = value;
                    let half = &t.halves[h];
                    if !half.up() && half.mr & MR_ILD == 0 {
                        t.halves[h].anchor_count = t.ilr(h) as u32;
                    }
                });
            }
            TAMATCHR | TBMATCHR => {
                let h = usize::from(offset == TBMATCHR);
                self.reanchor(h, |t| t.halves[h].matchr = value);
            }
            TAPR | TBPR => {
                let h = usize::from(offset == TBPR);
                self.reanchor(h, |t| t.halves[h].pr = value & 0xFF);
            }
            TAPMR => self.halves[0].pmr = value & 0xFF,
            TBPMR => self.halves[1].pmr = value & 0xFF,
            TAV | TBV => {
                let h = usize::from(offset == TBV);
                self.reanchor(h, |t| t.halves[h].anchor_count = value & t.max(h) as u32);
            }
            RIS | MIS | TAR | TBR | RTCPD | TAPS | TBPS | TAPV | TBPV | PP => {}
            _ => {
                crate::census_reg!("cc2538:Cc2538GpTimer", offset, "write");
            }
        }
    }
}

impl Default for Cc2538GpTimer {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Peripheral for Cc2538GpTimer {
    /// Counts and status are read lazily from the cycle clock; only an armed
    /// interrupt needs the walk, and only when there is no event scheduler.
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && self.halves.iter().any(|h| h.armed.is_some())
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
        let mut pend = [false; 2];
        for (h, pended) in pend.iter_mut().enumerate() {
            while self.halves[h]
                .armed
                .is_some_and(|at| self.now_cycle() >= at)
            {
                *pended |= self.fire(h);
            }
        }
        let (own, explicit) = self.route(pend);
        crate::PeripheralTickResult {
            irq: own,
            explicit_irqs: (!explicit.is_empty()).then_some(explicit),
            ..Default::default()
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane_read(self.read_reg(offset & !3), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // `ICR` reads 0, so a byte store clears only its own lane's bits.
        let word = self.read_reg(offset & !3);
        self.write_reg(offset & !3, lane_merge(word, offset, value));
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
        let h = (event_token & 1) as usize;
        if token(h, self.halves[h].generation) != event_token {
            // Superseded by an earlier wake.
            return crate::sched::EventResult::default();
        }
        self.halves[h].live_at = None;
        let now = self.now_cycle();
        let Some(due) = self.halves[h].armed else {
            return crate::sched::EventResult::default();
        };
        if due > now {
            // The delivery moved later since this wake was queued: follow it.
            self.halves[h].live_at = Some(due);
            return crate::sched::EventResult {
                reschedule_delay: Some(due - now),
                ..Default::default()
            };
        }
        let mut pend = [false; 2];
        pend[h] = self.fire(h);
        // The next delivery `fire` armed goes back as a reschedule of this
        // token, and the generation returns to it to match.
        let reschedule_delay = self.pending_events.pop().map(|(delay, _)| {
            self.halves[h].generation = event_token >> 1;
            delay
        });
        self.pending_events.clear();
        let (own, explicit) = self.route(pend);
        crate::sched::EventResult {
            raise_own_irq: own,
            explicit_irqs: explicit,
            reschedule_delay,
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_node_reset(&mut self) {
        let clock = self.clock.take();
        *self = Self::new(self.irq_b);
        self.clock = clock;
    }

    snapshot_impl!(Cc2538GpTimer, "Cc2538GpTimer", keep[clock]);
}

impl Cc2538GpTimer {
    /// Timer A pends the block's own line; timer B pends `irq_b` (or the own
    /// line when the descriptor gives none).
    fn route(&self, pend: [bool; 2]) -> (bool, Vec<u32>) {
        match (pend[1], self.irq_b) {
            (true, Some(irq)) => (pend[0], vec![irq]),
            (b, _) => (pend[0] || b, Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer_at(cycle: u64) -> (Cc2538GpTimer, CycleClock) {
        let clock = CycleClock::default();
        clock.publish(cycle);
        let mut t = Cc2538GpTimer::new(Some(20));
        t.attach_cycle_clock(clock.clone());
        (t, clock)
    }

    fn w(t: &mut Cc2538GpTimer, off: u64, v: u32) {
        t.write_u32(off, v).unwrap();
    }

    fn r(t: &Cc2538GpTimer, off: u64) -> u32 {
        t.read_u32(off).unwrap()
    }

    /// Advance to `cycle`, delivering every armed interrupt on the way the
    /// way the build's time base does; returns the halves that pended.
    fn walk_to(t: &mut Cc2538GpTimer, clock: &CycleClock, cycle: u64) -> Vec<usize> {
        let mut out = Vec::new();
        loop {
            let next = (0..2)
                .filter_map(|h| t.halves[h].armed.map(|at| (at, h)))
                .filter(|&(at, _)| at <= cycle)
                .min();
            let Some((at, h)) = next else { break };
            clock.publish(at);
            if t.scheduler_mode() {
                if t.fire(h) {
                    out.push(h);
                }
            } else {
                let res = t.tick_elapsed(1);
                if res.irq {
                    out.push(0);
                }
                if res.explicit_irqs.is_some_and(|v| v.contains(&20)) {
                    out.push(1);
                }
            }
        }
        clock.publish(cycle);
        out
    }

    #[test]
    fn registers_reset_to_their_documented_values() {
        let t = Cc2538GpTimer::default();
        assert_eq!(r(&t, CFG), 0);
        assert_eq!(r(&t, CTL), 0);
        assert_eq!(r(&t, TAILR), 0xFFFF_FFFF);
        assert_eq!(r(&t, TBILR), 0xFFFF);
        assert_eq!(r(&t, TAMATCHR), 0xFFFF_FFFF);
        assert_eq!(r(&t, TAR), 0xFFFF_FFFF);
        assert_eq!(r(&t, TBV), 0xFFFF);
        assert_eq!(r(&t, RTCPD), RTCPD_RESET);
        assert_eq!(r(&t, RIS), 0);
    }

    #[test]
    fn periodic_down_counter_times_out_every_ilr_plus_one_cycles() {
        let (mut t, clock) = timer_at(1_000);
        w(&mut t, TAMR, 0x2); // periodic, down
        w(&mut t, TAILR, 99);
        assert_eq!(r(&t, TAR), 99, "an ILR write loads a down-counter");
        w(&mut t, CTL, 0x1);
        clock.publish(1_040);
        assert_eq!(r(&t, TAV), 59);
        clock.publish(1_098);
        assert_eq!(r(&t, TAR), 1);
        assert_eq!(r(&t, RIS), 0);
        clock.publish(1_099);
        assert_eq!(r(&t, TAR), 0);
        // The time-out is the entry into 0; polled without IMR, the status
        // comes from the cycle count with no walk.
        assert!(t.halves[0].armed.is_none());
        assert_eq!(r(&t, RIS), 0x1);
        clock.publish(1_100);
        assert_eq!(r(&t, TAR), 99, "reloads on the next count");
        w(&mut t, ICR, 0x1);
        assert_eq!(r(&t, RIS), 0);
        clock.publish(1_198);
        assert_eq!(r(&t, RIS), 0);
        clock.publish(1_199);
        assert_eq!(r(&t, RIS), 0x1);
        assert_eq!(r(&t, MIS), 0, "masked");
    }

    #[test]
    fn split_one_shot_up_counter_with_prescaler_matches_then_stops() {
        let (mut t, clock) = timer_at(0);
        w(&mut t, CFG, CFG_16BIT);
        w(&mut t, TBMR, MR_ONE_SHOT | MR_CDIR | MR_MIE);
        assert_eq!(r(&t, TBR), 0, "an up-counter starts at 0");
        w(&mut t, TBPR, 15); // ÷16
        w(&mut t, TBILR, 1_000);
        w(&mut t, TBMATCHR, 500);
        w(&mut t, CTL, CTL_EN[1]);
        clock.publish(16 * 500 - 1);
        assert_eq!(r(&t, TBR), 499);
        assert_eq!(r(&t, RIS), 0);
        clock.publish(16 * 500);
        assert_eq!(r(&t, RIS), INT_M[1]);
        assert_eq!(r(&t, CTL), CTL_EN[1]);
        clock.publish(16 * 1_000);
        assert_eq!(r(&t, RIS), INT_M[1] | INT_TO[1]);
        assert_eq!(r(&t, CTL), 0, "a one-shot clears TnEN at the time-out");
        assert_eq!(r(&t, TBR), 0, "and holds the reload value");
        clock.publish(16 * 5_000);
        w(&mut t, ICR, 0xFFFF);
        assert_eq!(r(&t, RIS), 0, "no further edges once stopped");
        // TnEN again runs it once more from 0.
        w(&mut t, CTL, CTL_EN[1]);
        clock.publish(16 * 6_000);
        assert_eq!(r(&t, RIS), INT_M[1] | INT_TO[1]);
        // Timer A is not disturbed (16-bit, never enabled).
        assert_eq!(r(&t, TAR), 0xFFFF);
    }

    #[test]
    fn unmasked_time_out_pends_a_on_its_own_line_and_b_on_irq_b() {
        let (mut t, clock) = timer_at(0);
        w(&mut t, CFG, CFG_16BIT);
        w(&mut t, TAMR, 0x2);
        w(&mut t, TAILR, 49);
        w(&mut t, TBMR, 0x2);
        w(&mut t, TBILR, 79);
        w(&mut t, IMR, INT_TO[0] | INT_TO[1]);
        w(&mut t, CTL, CTL_EN[0] | CTL_EN[1]);
        assert_eq!(t.halves[0].armed, Some(49));
        assert_eq!(t.halves[1].armed, Some(79));
        assert_eq!(walk_to(&mut t, &clock, 78), vec![0]);
        w(&mut t, ICR, INT_TO[0]);
        assert_eq!(walk_to(&mut t, &clock, 100), vec![1, 0]);
    }

    #[test]
    fn status_left_set_by_an_imr_or_icr_write_pends_again() {
        let (mut t, clock) = timer_at(0);
        w(&mut t, TAMR, 0x2);
        w(&mut t, TAILR, 9);
        w(&mut t, CTL, 0x1);
        clock.publish(25); // two time-outs, polled
        assert_eq!(r(&t, RIS), 0x1);
        w(&mut t, IMR, INT_TO[0]);
        assert!(t.halves[0].kick);
        assert_eq!(t.halves[0].armed, Some(26));
        assert_eq!(walk_to(&mut t, &clock, 26), vec![0]);
        w(&mut t, ICR, INT_TO[0]);
        assert!(!t.halves[0].kick, "cleared status does not pend");
        assert_eq!(t.halves[0].armed, Some(29));
    }

    #[test]
    fn rewriting_a_later_match_keeps_one_live_wake() {
        let (mut t, _clock) = timer_at(0);
        w(&mut t, TAMR, 0x2 | MR_CDIR | MR_MIE);
        w(&mut t, IMR, INT_M[0]);
        w(&mut t, TAMATCHR, 1_000);
        w(&mut t, CTL, 0x1);
        let _ = t.take_scheduled_events();
        for m in 1_001..1_020 {
            w(&mut t, TAMATCHR, m);
        }
        let events = t.take_scheduled_events();
        assert!(events.is_empty(), "later matches reuse the live wake");
        assert_eq!(t.halves[0].armed, Some(1_019));
        w(&mut t, TAMATCHR, 500);
        let events = t.take_scheduled_events();
        if t.scheduler_mode() {
            assert_eq!(events.len(), 1, "an earlier match supersedes it");
        }
    }

    #[test]
    fn tav_writes_load_and_up_counter_wraps_past_ilr() {
        let (mut t, clock) = timer_at(0);
        w(&mut t, TAMR, 0x2 | MR_CDIR);
        w(&mut t, TAILR, 100);
        w(&mut t, CTL, 0x1);
        clock.publish(10);
        assert_eq!(r(&t, TAR), 10);
        w(&mut t, TAV, 0xFFFF_FFF0);
        clock.publish(10 + 0x10);
        assert_eq!(r(&t, TAR), 0, "above ILR: run to the width limit, wrap");
        assert_eq!(r(&t, RIS), 0);
        clock.publish(10 + 0x10 + 100);
        assert_eq!(r(&t, RIS), 0x1);
        // Disabling holds the count.
        w(&mut t, CTL, 0);
        let held = r(&t, TAR);
        clock.publish(10_000);
        assert_eq!(r(&t, TAR), held);
    }

    #[test]
    fn byte_store_to_icr_clears_only_its_lane() {
        let (mut t, clock) = timer_at(0);
        w(&mut t, CFG, CFG_16BIT);
        w(&mut t, TAMR, 0x2);
        w(&mut t, TAILR, 9);
        w(&mut t, TBMR, 0x2);
        w(&mut t, TBILR, 9);
        w(&mut t, CTL, CTL_EN[0] | CTL_EN[1]);
        clock.publish(10);
        assert_eq!(r(&t, RIS), INT_TO[0] | INT_TO[1]);
        t.write(ICR + 1, 0x01).unwrap();
        assert_eq!(r(&t, RIS), INT_TO[0]);
    }

    #[test]
    fn without_a_clock_nothing_counts_or_arms() {
        let mut t = Cc2538GpTimer::default();
        w(&mut t, TAMR, 0x2);
        w(&mut t, TAILR, 9);
        w(&mut t, IMR, 0x1);
        w(&mut t, CTL, 0x1);
        assert_eq!(r(&t, TAR), 9);
        assert!(t.take_scheduled_events().is_empty());
        assert!(!t.legacy_tick_active());
    }
}
