// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 PWM — 8 independent slices (datasheet §4.5, base `0x40050000`).
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
//! shortcut. `A_INV` / `B_INV` and the compare outputs affect pin levels, which
//! this block does not drive; `CC` is stored and read back faithfully so
//! firmware computing duty cycles sees its own values.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

/// Number of PWM slices on the RP2040.
const SLICES: usize = 8;
/// Bytes per slice register block (`CSR`/`DIV`/`CTR`/`CC`/`TOP`).
const SLICE_STRIDE: u64 = 0x14;
/// First address above the slice array.
const SLICE_END: u64 = SLICE_STRIDE * SLICES as u64; // 0xA0

const EN: u64 = 0xA0;
const INTR: u64 = 0xA4;
const INTE: u64 = 0xA8;
const INTF: u64 = 0xAC;
const INTS: u64 = 0xB0;

// CSR bits.
const CSR_EN: u32 = 1 << 0;
const CSR_PH_CORRECT: u32 = 1 << 1;
const CSR_PH_RET: u32 = 1 << 6;
const CSR_PH_ADV: u32 = 1 << 7;
const CSR_DIVMODE: u32 = 0b11 << 4;
/// Writable CSR bits; PH_ADV/PH_RET are self-clearing and never read back set.
const CSR_MASK: u32 = CSR_EN | CSR_PH_CORRECT | CSR_DIVMODE | (1 << 2) | (1 << 3);

/// NVIC vector for `PWM_IRQ_WRAP` (RP2040 datasheet §2.3.2).
const PWM_IRQ_WRAP: u32 = 4;

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
}

#[derive(Debug)]
pub struct Rp2040Pwm {
    slices: [Slice; SLICES],
    /// `INTR` — latched wrap interrupt, one bit per slice (write-1-clear).
    intr: u8,
    inte: u8,
    intf: u8,
    /// Bus-published cycle clock (event-scheduler). When present the model is
    /// walk-independent: free-running counters ride scheduled events.
    clock: Option<CycleClock>,
    /// Bumped each arm so stale slice events die on arrival.
    arm_seq: u32,
}

impl Default for Rp2040Pwm {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040Pwm {
    pub fn new() -> Self {
        Self {
            slices: core::array::from_fn(|_| Slice::new()),
            intr: 0,
            inte: 0,
            intf: 0,
            clock: None,
            arm_seq: 0,
        }
    }

    fn ints(&self) -> u8 {
        (self.intr | self.intf) & self.inte
    }

    /// `EN` collects bit 0 of every slice's `CSR`.
    fn en_bits(&self) -> u32 {
        (0..SLICES).fold(0u32, |acc, i| acc | ((self.slices[i].csr & CSR_EN) << i))
    }

    crate::cycle_clock::scheduler_mode!();

    fn free_running_active(&self) -> bool {
        self.slices.iter().any(|s| s.enabled() && s.free_running())
    }

    fn needs_scheduler_wake(&self) -> bool {
        self.free_running_active() || self.ints() != 0
    }

    /// One peripheral-tick of free-running counter work + level IRQ vector.
    fn step_one(&mut self) -> PeripheralTickResult {
        for i in 0..SLICES {
            let s = &mut self.slices[i];
            if !s.enabled() || !s.free_running() {
                continue;
            }
            if s.divider_fires() && s.step() {
                self.intr |= 1 << i;
            }
        }
        let explicit_irqs = (self.ints() != 0).then(|| vec![PWM_IRQ_WRAP]);
        PeripheralTickResult {
            explicit_irqs,
            ..Default::default()
        }
    }
}

impl Peripheral for Rp2040Pwm {
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset < SLICE_END {
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
        Ok(match offset {
            EN => self.en_bits(),
            INTR => self.intr as u32,
            INTE => self.inte as u32,
            INTF => self.intf as u32,
            INTS => self.ints() as u32,
            _ => {
                crate::census_reg!("rp2040.pwm:Rp2040Pwm", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset < SLICE_END {
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
            return Ok(());
        }
        match offset {
            // Aliases CSR.EN across all slices — the atomic multi-slice start.
            EN => {
                for (i, s) in self.slices.iter_mut().enumerate() {
                    if value & (1 << i) != 0 {
                        s.csr |= CSR_EN;
                    } else {
                        s.csr &= !CSR_EN;
                    }
                }
            }
            INTR => self.intr &= !(value as u8),
            INTE => self.inte = value as u8,
            INTF => self.intf = value as u8,
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

    /// Reset values match the datasheet: TOP all-ones, DIV = 1.0.
    #[test]
    fn reset_values_match_datasheet() {
        let p = Rp2040Pwm::new();
        for i in 0..SLICES as u64 {
            assert_eq!(p.read_u32(slice_reg(i, 0x10)).unwrap(), 0xFFFF);
            assert_eq!(p.read_u32(slice_reg(i, 0x04)).unwrap(), 0x010);
            assert_eq!(p.read_u32(slice_reg(i, 0x00)).unwrap(), 0);
        }
    }
}
