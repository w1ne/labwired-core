// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 PWM — 8 independent slices (datasheet §4.5, base `0x40050000`);
//! the RP2350's 12-slice block is the same model under [`PwmLayout::Rp2350`].
//!
//! Each slice is a 16-bit counter with its own clock divider, wrap value and two
//! compare channels (A/B). Register block per slice is 5 words at stride `0x14`:
//! `CSR`, `DIV`, `CTR`, `CC`, `TOP`. The global registers `EN`, `INTR`, `INTE`,
//! `INTF`, `INTS` sit above the slice array.
//!
//! ## What silicon does, and what this models
//!
//! * **Counter + wrap.** An enabled slice counts up once per divided clock. On
//!   reaching `TOP` it wraps to 0 and latches its `INTR` bit. That wrap
//!   interrupt is the whole point of the block for RTOS/tick users, and it is
//!   modelled exactly.
//! * **Fractional divider.** `DIV` is 8.4 fixed point (`INT[11:4]`, `FRAC[3:0]`)
//!   and a value below `0x010` behaves as 1.0. Modelled with a 4-bit phase
//!   accumulator, so a divider of e.g. 2.5 really does advance the counter on
//!   alternate pairs of ticks rather than being rounded away.
//! * **Phase-correct mode** (`CSR.PH_CORRECT`, bit 1). The counter ramps up to
//!   `TOP` then back down to 0, halving the output frequency; the wrap
//!   interrupt fires at the *bottom* of the ramp, as on silicon.
//! * **`PH_ADV` / `PH_RET`** (bits 7/6) nudge the counter forward/back by one
//!   and self-clear — pico-sdk uses them to phase-align slices.
//! * **`EN` (0xA0) aliases the per-slice `CSR.EN` bits**, which is what makes an
//!   atomic multi-slice start possible. Writing it is exactly equivalent to
//!   writing bit 0 of each slice's `CSR`, and reading it collects them back.
//!
//! ## Deliberately not modelled
//!
//! `DIVMODE` (`CSR[5:4]`) selects what advances the counter: free-running (0) or
//! gated / edge-counted from the slice's B input pin (1..3). Only free-running
//! advances here — there is no B-pin routing into this model, so a gated slice
//! genuinely has no clock and standing still is the honest behaviour, not a
//! shortcut.
//!
//! ## Channel outputs
//!
//! Each slice drives two outputs, A and B (datasheet §4.5.2): a channel is high
//! while `CTR < CC` (its half of `CC`), then `CSR.A_INV` / `B_INV` (bits 2/3)
//! invert it. So `CC = 0` is low all period and `CC > TOP` high all period, in
//! both counting modes. Once the bus routes a pad to the block
//! ([`Peripheral::timer_output_lines`]), every level change is published on the
//! 16-line wire [`LINE_NAMES`] at the cycle the counter step made it, so a pad
//! IO_BANK0 hands to `FUNCSEL = 4` (PWM) reads the waveform rather than the SIO
//! latch. The wire is inert until then.

use std::sync::Arc;

use crate::peripherals::pad_lines::PadLines;
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

/// Most PWM slices on any supported part (RP2350: 12; RP2040: 8).
const MAX_SLICES: usize = 12;
/// Bytes per slice register block (`CSR`/`DIV`/`CTR`/`CC`/`TOP`).
const SLICE_STRIDE: u64 = 0x14;

/// Which silicon's PWM map the block serves.
///
/// RP2040 (§4.5.3): 8 slices, `EN` at 0xA0 then `INTR`, `INTE`, `INTF`,
/// `INTS`, one wrap interrupt (`PWM_IRQ_WRAP`, NVIC 4). RP2350 (pico-sdk
/// rp2350 `pwm.h`): 12 slices, so the globals start at 0xF0 — `EN`, `INTR`,
/// then two enable/force/status groups `IRQ0_*` and `IRQ1_*` driving
/// `PWM_IRQ_WRAP_0` / `_1` (NVIC 8 / 9). The slice block is identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PwmLayout {
    #[default]
    Rp2040,
    Rp2350,
}

impl PwmLayout {
    const fn slices(self) -> usize {
        match self {
            PwmLayout::Rp2040 => 8,
            PwmLayout::Rp2350 => 12,
        }
    }

    /// Offset of `EN`, the first word above the slice array.
    const fn en(self) -> u64 {
        SLICE_STRIDE * self.slices() as u64
    }

    /// NVIC vector of each `INTE`/`INTF`/`INTS` group.
    const fn wrap_irqs(self) -> &'static [u32] {
        match self {
            // RP2040 datasheet §2.3.2.
            PwmLayout::Rp2040 => &[4],
            // pico-sdk rp2350 intctrl.h: PWM_IRQ_WRAP_0 = 8, _1 = 9.
            PwmLayout::Rp2350 => &[8, 9],
        }
    }
}

// CSR bits.
const CSR_EN: u32 = 1 << 0;
const CSR_PH_CORRECT: u32 = 1 << 1;
/// `A_INV`; `B_INV` is the next bit up.
const CSR_A_INV: u32 = 1 << 2;
const CSR_PH_RET: u32 = 1 << 6;
const CSR_PH_ADV: u32 = 1 << 7;
const CSR_DIVMODE: u32 = 0b11 << 4;
/// Writable CSR bits; PH_ADV/PH_RET are self-clearing and never read back set.
const CSR_MASK: u32 = CSR_EN | CSR_PH_CORRECT | CSR_DIVMODE | CSR_A_INV | (CSR_A_INV << 1);

/// The output wire, slice-major: line `2 * slice + channel` (A = 0, B = 1).
/// That is the order IO_BANK0 FUNCSEL 4 maps the pads onto: GPn carries
/// slice `(n >> 1) & 7`, channel `n & 1`, so GP0..GP31 are line `n & 15`
/// (RP2350B GP32..47 reach slices 8..11 as line `16 + (n & 7)`). A block
/// publishes the first `2 * slices` names.
pub static LINE_NAMES: [&str; 2 * MAX_SLICES] = [
    "PWM0_A", "PWM0_B", "PWM1_A", "PWM1_B", "PWM2_A", "PWM2_B", "PWM3_A", "PWM3_B", "PWM4_A",
    "PWM4_B", "PWM5_A", "PWM5_B", "PWM6_A", "PWM6_B", "PWM7_A", "PWM7_B", "PWM8_A", "PWM8_B",
    "PWM9_A", "PWM9_B", "PWM10_A", "PWM10_B", "PWM11_A", "PWM11_B",
];

#[derive(Debug, Default, Clone)]
struct Slice {
    csr: u32,
    /// 8.4 fixed-point clock divider.
    div: u32,
    /// 16-bit counter.
    ctr: u16,
    /// Compare values: A in `[15:0]`, B in `[31:16]`.
    cc: u32,
    /// Wrap value.
    top: u16,
    /// Fractional-divider phase accumulator (4 bits of `DIV.FRAC`).
    frac_acc: u32,
    /// Phase-correct ramp direction: `true` while counting down.
    counting_down: bool,
}

impl Slice {
    fn new() -> Self {
        Self {
            // Reset values per datasheet: TOP is all-ones, DIV is 1.0.
            div: 0x0000_0010,
            top: 0xFFFF,
            ..Default::default()
        }
    }

    fn enabled(&self) -> bool {
        self.csr & CSR_EN != 0
    }

    /// Free-running is the only divider mode with an internal clock source.
    fn free_running(&self) -> bool {
        self.csr & CSR_DIVMODE == 0
    }

    /// Advance the fractional divider; `true` when the counter should step.
    ///
    /// `DIV` is 8.4 fixed point. Silicon treats an integer part of 0 as 1.0, so
    /// a zeroed `DIV` still counts rather than freezing.
    fn divider_fires(&mut self) -> bool {
        let step = (self.div & 0xFFF).max(0x010);
        self.frac_acc += 0x10;
        if self.frac_acc >= step {
            self.frac_acc -= step;
            true
        } else {
            false
        }
    }

    /// One divided-clock edge. Returns `true` if the slice wrapped (which
    /// latches its `INTR` bit).
    fn step(&mut self) -> bool {
        if self.csr & CSR_PH_CORRECT != 0 {
            // Up to TOP, then back down to 0, wrapping at the bottom
            // (datasheet §4.5.2.1). The turnaround must not linger at TOP or at
            // 0: the phase-correct period is 2*TOP, so each endpoint is
            // occupied for exactly one step, and the counter reverses on the
            // same edge that reaches it.
            if self.counting_down || self.ctr >= self.top {
                self.counting_down = true;
                self.ctr = self.ctr.saturating_sub(1);
            } else {
                self.ctr += 1;
            }
            if self.counting_down && self.ctr == 0 {
                self.counting_down = false;
                return true;
            }
            false
        } else if self.ctr >= self.top {
            self.ctr = 0;
            true
        } else {
            self.ctr += 1;
            false
        }
    }

    /// Level of output `ch` (0 = A, 1 = B): high while `CTR < CC`, then the
    /// channel's `_INV` bit.
    fn output(&self, ch: usize) -> bool {
        let cc = (self.cc >> (16 * ch)) as u16;
        (self.ctr < cc) ^ (self.csr & (CSR_A_INV << ch) != 0)
    }

    /// Advance the counter by `fires` divider ticks. Returns true if the
    /// slice wrapped at least once (`INTR` latch).
    fn advance_steps(&mut self, fires: u64) -> bool {
        if fires == 0 {
            return false;
        }
        let period = u64::from(self.top) + 1;
        if u64::from(self.ctr) >= u64::from(self.top) {
            let rest = fires - 1;
            self.ctr = (rest % period) as u16;
            return true;
        }
        let first_wrap = u64::from(self.top - self.ctr) + 1;
        if fires < first_wrap {
            self.ctr = self.ctr.saturating_add(fires as u16);
            false
        } else {
            let rest = fires - first_wrap;
            self.ctr = (rest % period) as u16;
            true
        }
    }

    fn cycles_per_step(&self) -> u64 {
        u64::from((self.div & 0xFFF).max(0x010) / 0x10).max(1)
    }

    fn cycles_until_next_step(&self) -> u64 {
        let step = (self.div & 0xFFF).max(0x010);
        if self.frac_acc >= step {
            1
        } else {
            u64::from((step - self.frac_acc).div_ceil(0x10)).max(1)
        }
    }

    /// Counter steps until wrap or an A/B compare match (an output edge).
    fn steps_until_next_edge(&self) -> u64 {
        if self.csr & CSR_PH_CORRECT != 0 {
            return 1;
        }
        let to_wrap = if self.ctr >= self.top {
            1
        } else {
            u64::from(self.top - self.ctr) + 1
        };
        let mut best = to_wrap;
        for ch in 0..2 {
            let cc = (self.cc >> (16 * ch)) as u16;
            if self.ctr < cc {
                best = best.min(u64::from(cc - self.ctr));
            }
        }
        best.max(1)
    }

    fn cycles_until_next_edge(&self) -> u64 {
        let steps = self.steps_until_next_edge();
        let first = self.cycles_until_next_step();
        if steps == 1 {
            first
        } else {
            first + (steps - 1) * self.cycles_per_step()
        }
    }
}

#[derive(Debug)]
pub struct Rp2040Pwm {
    layout: PwmLayout,
    slices: [Slice; MAX_SLICES],
    /// `INTR` — latched wrap interrupt, one bit per slice (write-1-clear).
    intr: u16,
    /// `INTE` / `INTF` per interrupt group (`IRQ0_*`, `IRQ1_*` on RP2350).
    inte: [u16; 2],
    intf: [u16; 2],
    /// Bus-published cycle clock (event-scheduler). When present the model is
    /// walk-independent: free-running counters ride scheduled events.
    clock: Option<CycleClock>,
    /// Bumped each arm so stale slice events die on arrival.
    arm_seq: u32,
    last_sync: u64,
    /// The A/B output wire ([`LINE_NAMES`]), created when the bus routes a pad
    /// to it.
    lines: Option<Arc<PadLines>>,
    /// A pad can carry the wire: publish every output change on it.
    wired: bool,
}

impl Default for Rp2040Pwm {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040Pwm {
    pub fn new() -> Self {
        Self {
            layout: PwmLayout::Rp2040,
            slices: core::array::from_fn(|_| Slice::new()),
            intr: 0,
            inte: [0; 2],
            intf: [0; 2],
            clock: None,
            arm_seq: 0,
            last_sync: 0,
            lines: None,
            wired: false,
        }
    }

    /// Serve the given silicon's map: slice count, globals and wrap IRQs.
    pub fn with_layout(mut self, layout: PwmLayout) -> Self {
        self.layout = layout;
        self
    }

    fn slice_count(&self) -> usize {
        self.layout.slices()
    }

    /// Put every slice's A/B level on the wire. `at` is the cycle the change
    /// happened (scheduler mode); without one the tap stamps it.
    fn publish(&self, at: Option<u64>) {
        for i in 0..self.slice_count() {
            self.publish_slice(i, at);
        }
    }

    fn publish_slice(&self, i: usize, at: Option<u64>) {
        let Some(lines) = self.lines.as_deref().filter(|_| self.wired) else {
            return;
        };
        for ch in 0..2 {
            let level = self.slices[i].output(ch);
            match at {
                Some(at) => lines.set_line_at(2 * i + ch, level, at),
                None => lines.set_line(2 * i + ch, level),
            }
        }
    }

    /// `INTS` of interrupt group `g`.
    fn ints(&self, g: usize) -> u16 {
        (self.intr | self.intf[g]) & self.inte[g]
    }

    fn any_ints(&self) -> bool {
        (0..self.layout.wrap_irqs().len()).any(|g| self.ints(g) != 0)
    }

    /// `EN` collects bit 0 of every slice's `CSR`.
    fn en_bits(&self) -> u32 {
        (0..self.slice_count()).fold(0u32, |acc, i| acc | ((self.slices[i].csr & CSR_EN) << i))
    }

    crate::cycle_clock::scheduler_mode!();

    fn free_running_active(&self) -> bool {
        self.slices[..self.slice_count()]
            .iter()
            .any(|s| s.enabled() && s.free_running())
    }

    fn needs_scheduler_wake(&self) -> bool {
        self.free_running_active() || self.any_ints()
    }

    fn cycles_until_next_wake(&self) -> Option<u64> {
        if self.any_ints() {
            return Some(1);
        }
        let mut best: Option<u64> = None;
        for s in &self.slices[..self.slice_count()] {
            if s.enabled() && s.free_running() {
                let d = s.cycles_until_next_edge();
                best = Some(best.map_or(d, |b| b.min(d)));
            }
        }
        best
    }

    /// Advance free-running slices by `n` CPU cycles and publish the resulting
    /// levels. Used so a scheduler wake can cover a whole PWM edge instead of
    /// firing every cycle (which pinned idle-ff after `analogWrite`).
    fn advance_cycles(&mut self, n: u64, at: Option<u64>) -> PeripheralTickResult {
        if n == 0 {
            return self.step_one(at);
        }
        for i in 0..self.slice_count() {
            if !self.slices[i].enabled() || !self.slices[i].free_running() {
                continue;
            }
            if self.slices[i].csr & CSR_PH_CORRECT != 0 {
                for _ in 0..n {
                    if self.slices[i].divider_fires() && self.slices[i].step() {
                        self.intr |= 1 << i;
                    }
                }
                self.publish_slice(i, at);
                continue;
            }
            // Exact 8.4 DIV: each CPU cycle adds 0x10 to the accumulator.
            let step = u64::from((self.slices[i].div & 0xFFF).max(0x010));
            let new_acc = u64::from(self.slices[i].frac_acc) + n * 0x10;
            let fires = new_acc / step;
            self.slices[i].frac_acc = (new_acc % step) as u32;
            if self.slices[i].advance_steps(fires) {
                self.intr |= 1 << i;
            }
            if fires > 0 {
                self.publish_slice(i, at);
            }
        }
        let irqs: Vec<u32> = self
            .layout
            .wrap_irqs()
            .iter()
            .enumerate()
            .filter(|&(g, _)| self.ints(g) != 0)
            .map(|(_, &irq)| irq)
            .collect();
        let explicit_irqs = (!irqs.is_empty()).then_some(irqs);
        PeripheralTickResult {
            explicit_irqs,
            ..Default::default()
        }
    }

    /// One peripheral-tick of free-running counter work + level IRQ vector,
    /// at cycle `at` when the scheduler knows it.
    fn step_one(&mut self, at: Option<u64>) -> PeripheralTickResult {
        for i in 0..self.slice_count() {
            let s = &mut self.slices[i];
            if !s.enabled() || !s.free_running() {
                continue;
            }
            if s.divider_fires() {
                if s.step() {
                    self.intr |= 1 << i;
                }
                self.publish_slice(i, at);
            }
        }
        let irqs: Vec<u32> = self
            .layout
            .wrap_irqs()
            .iter()
            .enumerate()
            .filter(|&(g, _)| self.ints(g) != 0)
            .map(|(_, &irq)| irq)
            .collect();
        let explicit_irqs = (!irqs.is_empty()).then_some(irqs);
        PeripheralTickResult {
            explicit_irqs,
            ..Default::default()
        }
    }
}

impl Peripheral for Rp2040Pwm {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let en = self.layout.en();
        if offset < en {
            let s = &self.slices[(offset / SLICE_STRIDE) as usize];
            return Ok(match offset % SLICE_STRIDE {
                0x00 => s.csr,
                0x04 => s.div,
                0x08 => s.ctr as u32,
                0x0C => s.cc,
                0x10 => s.top as u32,
                _ => 0,
            });
        }
        let groups = self.layout.wrap_irqs().len() as u64;
        Ok(match offset - en {
            0x0 => self.en_bits(),
            0x4 => self.intr as u32,
            // INTE/INTF/INTS, then IRQ1_* on the RP2350.
            rel @ 0x8.. if rel < 0x8 + 0xC * groups => {
                let g = ((rel - 0x8) / 0xC) as usize;
                match (rel - 0x8) % 0xC {
                    0x0 => self.inte[g] as u32,
                    0x4 => self.intf[g] as u32,
                    _ => self.ints(g) as u32,
                }
            }
            _ => {
                crate::census_reg!("rp2040.pwm:Rp2040Pwm", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let en = self.layout.en();
        let n = self.slice_count();
        if offset < en {
            let idx = (offset / SLICE_STRIDE) as usize;
            let s = &mut self.slices[idx];
            match offset % SLICE_STRIDE {
                0x00 => {
                    // PH_ADV / PH_RET nudge the counter one step and self-clear.
                    if value & CSR_PH_ADV != 0 {
                        s.ctr = s.ctr.wrapping_add(1);
                    }
                    if value & CSR_PH_RET != 0 {
                        s.ctr = s.ctr.wrapping_sub(1);
                    }
                    s.csr = value & CSR_MASK;
                }
                0x04 => s.div = value & 0xFFF,
                0x08 => s.ctr = value as u16,
                0x0C => s.cc = value,
                0x10 => s.top = value as u16,
                _ => {}
            }
            self.publish(self.clock.as_ref().map(CycleClock::now));
            return Ok(());
        }
        let groups = self.layout.wrap_irqs().len() as u64;
        match offset - en {
            // Aliases CSR.EN across all slices — the atomic multi-slice start.
            0x0 => {
                for (i, s) in self.slices[..n].iter_mut().enumerate() {
                    if value & (1 << i) != 0 {
                        s.csr |= CSR_EN;
                    } else {
                        s.csr &= !CSR_EN;
                    }
                }
                self.publish(self.clock.as_ref().map(CycleClock::now));
            }
            0x4 => self.intr &= !(value as u16),
            rel @ 0x8.. if rel < 0x8 + 0xC * groups => {
                let g = ((rel - 0x8) / 0xC) as usize;
                let mask = ((1u32 << n) - 1) as u16;
                match (rel - 0x8) % 0xC {
                    0x0 => self.inte[g] = value as u16 & mask,
                    0x4 => self.intf[g] = value as u16 & mask,
                    _ => {}
                }
            }
            _ => {
                crate::census_reg!("rp2040.pwm:Rp2040Pwm", offset, "write");
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
        self.step_one(None)
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

    /// The A/B output wire ([`LINE_NAMES`]); `arm` starts publishing on it,
    /// once the bus has bound a pad that can carry it.
    fn timer_output_lines(&mut self, arm: bool) -> Option<Arc<PadLines>> {
        let lines = self
            .lines
            .get_or_insert_with(|| {
                let n = 2 * self.layout.slices();
                Arc::new(PadLines::new(
                    &LINE_NAMES[..n],
                    &[false; 2 * MAX_SLICES][..n],
                ))
            })
            .clone();
        if arm && !self.wired {
            self.wired = true;
            self.publish(self.clock.as_ref().map(CycleClock::now));
        }
        Some(lines)
    }

    fn wire_lines(&self) -> Option<&PadLines> {
        self.lines.as_deref()
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() || !self.needs_scheduler_wake() {
            return Vec::new();
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        // Do not move `last_sync` here. `on_event` advances by
        // `now - last_sync`; bumping the watermark without applying
        // those cycles dropped the enable-to-first-step interval and
        // stretched the first PWM period by one cycle.
        let delay = self.cycles_until_next_wake().unwrap_or(1).saturating_sub(1);
        vec![(delay, self.arm_seq)]
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
        let now = sched.now();
        let elapsed = now.saturating_sub(self.last_sync).max(1);
        self.last_sync = now;
        let res = self.advance_cycles(elapsed, Some(now));
        crate::sched::EventResult {
            explicit_irqs: res.explicit_irqs.unwrap_or_default(),
            reschedule_delay: self.cycles_until_next_wake(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RP2040 globals.
    const EN: u64 = 0xA0;
    const INTR: u64 = 0xA4;
    const INTE: u64 = 0xA8;
    const PWM_IRQ_WRAP: u32 = 4;

    fn slice_reg(idx: u64, reg: u64) -> u64 {
        idx * SLICE_STRIDE + reg
    }

    /// A disabled slice does not count — the reset state must be inert.
    #[test]
    fn disabled_slice_does_not_count() {
        let mut p = Rp2040Pwm::new();
        for _ in 0..50 {
            p.tick();
        }
        assert_eq!(p.read_u32(slice_reg(0, 0x08)).unwrap(), 0);
    }

    /// Enabled slice counts to TOP, wraps, and latches its INTR bit.
    #[test]
    fn free_running_slice_reports_cycles_until_wrap() {
        let mut p = Rp2040Pwm::new();
        p.write_u32(slice_reg(0, 0x10), 4).unwrap(); // TOP = 4
        p.write_u32(slice_reg(0, 0x00), CSR_EN).unwrap();
        // CTR=0, TOP=4, div=1.0 → wrap in 5 steps / 5 cycles.
        assert_eq!(p.slices[0].cycles_until_next_edge(), 5);
        let res = p.advance_cycles(5, Some(10));
        assert_eq!(p.read_u32(slice_reg(0, 0x08)).unwrap(), 0);
        assert_eq!(p.read_u32(INTR).unwrap(), 1);
        let _ = res;
    }

    #[test]
    fn slice_wraps_at_top_and_latches_intr() {
        let mut p = Rp2040Pwm::new();
        p.write_u32(slice_reg(0, 0x10), 4).unwrap(); // TOP = 4
        p.write_u32(slice_reg(0, 0x00), CSR_EN).unwrap();

        // 4 ticks reach TOP without wrapping.
        for _ in 0..4 {
            p.tick();
        }
        assert_eq!(p.read_u32(slice_reg(0, 0x08)).unwrap(), 4);
        assert_eq!(p.read_u32(INTR).unwrap(), 0, "no wrap yet");

        p.tick(); // 5th step wraps
        assert_eq!(p.read_u32(slice_reg(0, 0x08)).unwrap(), 0);
        assert_eq!(p.read_u32(INTR).unwrap(), 1, "slice 0 wrap latched");
    }

    /// The wrap IRQ is only delivered once enabled, and is held until the raw
    /// flag is acknowledged.
    #[test]
    fn wrap_irq_gated_by_inte_and_cleared_by_intr_write() {
        let mut p = Rp2040Pwm::new();
        p.write_u32(slice_reg(0, 0x10), 1).unwrap();
        p.write_u32(slice_reg(0, 0x00), CSR_EN).unwrap();

        // Wrap with INTE clear: raw latches, no IRQ.
        let mut fired = false;
        for _ in 0..4 {
            fired |= p.tick().explicit_irqs.is_some();
        }
        assert_ne!(p.read_u32(INTR).unwrap(), 0);
        assert!(!fired, "masked wrap must not raise PWM_IRQ_WRAP");

        p.write_u32(INTE, 0x1).unwrap();
        let r = p.tick();
        assert_eq!(r.explicit_irqs, Some(vec![PWM_IRQ_WRAP]));

        // Acknowledge; the line drops.
        p.write_u32(INTR, 0xFF).unwrap();
        p.write_u32(slice_reg(0, 0x00), 0).unwrap(); // stop counting
        assert_eq!(p.tick().explicit_irqs, None);
    }

    /// `EN` really is an alias of the per-slice CSR.EN bits, in both directions.
    #[test]
    fn en_register_aliases_slice_csr_enable() {
        let mut p = Rp2040Pwm::new();
        p.write_u32(EN, 0b1010_0001).unwrap();
        assert_eq!(p.read_u32(EN).unwrap(), 0b1010_0001);
        assert_eq!(p.read_u32(slice_reg(0, 0x00)).unwrap() & CSR_EN, CSR_EN);
        assert_eq!(p.read_u32(slice_reg(1, 0x00)).unwrap() & CSR_EN, 0);
        assert_eq!(p.read_u32(slice_reg(7, 0x00)).unwrap() & CSR_EN, CSR_EN);

        // Enabling via CSR shows up in EN.
        p.write_u32(slice_reg(1, 0x00), CSR_EN).unwrap();
        assert_eq!(p.read_u32(EN).unwrap(), 0b1010_0011);
    }

    /// An integer divider really divides: DIV=2.0 halves the count rate.
    #[test]
    fn integer_divider_halves_the_count_rate() {
        let mut p = Rp2040Pwm::new();
        p.write_u32(slice_reg(0, 0x10), 0xFFFF).unwrap();
        p.write_u32(slice_reg(0, 0x04), 0x20).unwrap(); // 2.0
        p.write_u32(slice_reg(0, 0x00), CSR_EN).unwrap();
        for _ in 0..20 {
            p.tick();
        }
        assert_eq!(p.read_u32(slice_reg(0, 0x08)).unwrap(), 10);
    }

    /// A fractional divider is not rounded away: 2.5 yields 8 steps in 20 ticks.
    #[test]
    fn fractional_divider_is_honoured() {
        let mut p = Rp2040Pwm::new();
        p.write_u32(slice_reg(0, 0x10), 0xFFFF).unwrap();
        p.write_u32(slice_reg(0, 0x04), 0x28).unwrap(); // 2.5
        p.write_u32(slice_reg(0, 0x00), CSR_EN).unwrap();
        for _ in 0..20 {
            p.tick();
        }
        assert_eq!(p.read_u32(slice_reg(0, 0x08)).unwrap(), 8);
    }

    /// Phase-correct ramps up then down and only wraps at the bottom.
    #[test]
    fn phase_correct_ramps_down_and_wraps_at_bottom() {
        let mut p = Rp2040Pwm::new();
        p.write_u32(slice_reg(0, 0x10), 3).unwrap(); // TOP = 3
        p.write_u32(slice_reg(0, 0x00), CSR_EN | CSR_PH_CORRECT)
            .unwrap();

        let mut seen = Vec::new();
        for _ in 0..10 {
            p.tick();
            seen.push(p.read_u32(slice_reg(0, 0x08)).unwrap());
        }
        // Up 1,2,3, reverse, down 2,1,0, then straight back up: period 2*TOP,
        // with neither endpoint held for an extra step.
        assert_eq!(&seen[..8], &[1, 2, 3, 2, 1, 0, 1, 2]);
        assert_eq!(p.read_u32(INTR).unwrap(), 1, "wrap latched at the bottom");
    }

    /// A gated slice has no clock source in this model and must not advance —
    /// standing still is the honest result, not a silent free-run.
    #[test]
    fn gated_divmode_does_not_free_run() {
        let mut p = Rp2040Pwm::new();
        p.write_u32(slice_reg(0, 0x10), 0xFFFF).unwrap();
        p.write_u32(slice_reg(0, 0x00), CSR_EN | (1 << 4)).unwrap();
        for _ in 0..50 {
            p.tick();
        }
        assert_eq!(p.read_u32(slice_reg(0, 0x08)).unwrap(), 0);
    }

    /// PH_ADV / PH_RET nudge the counter and do not read back set.
    #[test]
    fn phase_advance_and_retard_are_self_clearing() {
        let mut p = Rp2040Pwm::new();
        p.write_u32(slice_reg(0, 0x00), CSR_EN | CSR_PH_ADV)
            .unwrap();
        assert_eq!(p.read_u32(slice_reg(0, 0x08)).unwrap(), 1);
        assert_eq!(p.read_u32(slice_reg(0, 0x00)).unwrap() & CSR_PH_ADV, 0);
        p.write_u32(slice_reg(0, 0x00), CSR_EN | CSR_PH_RET)
            .unwrap();
        assert_eq!(p.read_u32(slice_reg(0, 0x08)).unwrap(), 0);
    }

    /// Compare values round-trip so firmware computing duty cycles reads back
    /// exactly what it wrote.
    #[test]
    fn compare_and_top_round_trip() {
        let mut p = Rp2040Pwm::new();
        p.write_u32(slice_reg(3, 0x0C), 0x1234_5678).unwrap();
        p.write_u32(slice_reg(3, 0x10), 0xBEEF).unwrap();
        assert_eq!(p.read_u32(slice_reg(3, 0x0C)).unwrap(), 0x1234_5678);
        assert_eq!(p.read_u32(slice_reg(3, 0x10)).unwrap(), 0xBEEF);
        // Independent slices.
        assert_eq!(p.read_u32(slice_reg(4, 0x0C)).unwrap(), 0);
    }

    /// RP2350 map: 12 slices, so `EN` is at 0xF0 (0xA0 is slice 8's CSR),
    /// `INTR` 0xF4, `IRQ0_INTE` 0xF8 -> NVIC 8, `IRQ1_INTE` 0x104 -> NVIC 9,
    /// and the wire carries 24 lines.
    #[test]
    fn rp2350_layout_has_twelve_slices_and_two_wrap_irqs() {
        let mut p = Rp2040Pwm::new().with_layout(PwmLayout::Rp2350);
        p.write_u32(0xA0, CSR_EN).unwrap();
        assert_eq!(p.read_u32(0xF0).unwrap(), 1 << 8, "0xA0 is CH8_CSR");
        p.write_u32(0xF0, 0xFFF).unwrap();
        assert_eq!(p.read_u32(0xF0).unwrap(), 0xFFF);
        assert_eq!(p.read_u32(slice_reg(11, 0x00)).unwrap() & CSR_EN, CSR_EN);
        p.write_u32(0xF0, 0).unwrap();
        // Slice 11 wraps; IRQ1 routes it, IRQ0 does not.
        p.write_u32(slice_reg(11, 0x10), 1).unwrap();
        p.write_u32(slice_reg(11, 0x00), CSR_EN).unwrap();
        p.write_u32(0x104, 1 << 11).unwrap();
        let mut irqs = Vec::new();
        for _ in 0..3 {
            irqs.extend(p.tick().explicit_irqs.unwrap_or_default());
        }
        assert_eq!(p.read_u32(0xF4).unwrap(), 1 << 11, "INTR");
        assert_eq!(p.read_u32(0x10C).unwrap(), 1 << 11, "IRQ1_INTS");
        assert_eq!(p.read_u32(0x100).unwrap(), 0, "IRQ0_INTS");
        assert!(irqs.iter().all(|&i| i == 9) && !irqs.is_empty());
        p.write_u32(0xF4, 1 << 11).unwrap();
        p.write_u32(slice_reg(11, 0x00), 0).unwrap();
        assert_eq!(p.read_u32(0x10C).unwrap(), 0);
        let lines = p.timer_output_lines(false).unwrap();
        assert_eq!(lines.names().len(), 24);
        assert_eq!(lines.names()[23], "PWM11_B");
    }

    /// Reset values match the datasheet: TOP all-ones, DIV = 1.0.
    #[test]
    fn reset_values_match_datasheet() {
        let p = Rp2040Pwm::new();
        for i in 0..8u64 {
            assert_eq!(p.read_u32(slice_reg(i, 0x10)).unwrap(), 0xFFFF);
            assert_eq!(p.read_u32(slice_reg(i, 0x04)).unwrap(), 0x010);
            assert_eq!(p.read_u32(slice_reg(i, 0x00)).unwrap(), 0);
        }
    }
}
