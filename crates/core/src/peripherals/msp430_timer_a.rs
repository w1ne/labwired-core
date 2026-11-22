// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Texas Instruments MSP430x2xx Timer_A (family user's guide SLAU144,
//! chapter 12).
//!
//! * [`Msp430TimerA`] (`msp430_timer_a`): `TACTL` at offset 0, `TACCTLx` at
//!   2 + 2x, `TAR` at 0x10 and `TACCRx` at 0x12 + 2x, for 2 to 7
//!   capture/compare blocks (Timer_A2, Timer_A3, ...).
//! * [`Msp430TimerAIv`] (`msp430_timer_a_iv`): `TAIV`. TI placed it outside
//!   the timer window (Timer0_A3: 0x012E, Timer1_A3: 0x011E), so a
//!   descriptor maps it on its own and names the same interrupt sources.
//!
//! Counter (12.2.3):
//!
//! * `MCx` 0 stop; 1 up to `TACCR0`, then to 0; 2 continuous, 0xFFFF to 0;
//!   3 up to `TACCR0` and down to 0. Up and up/down modes hold while
//!   `TACCR0` is 0.
//! * Up mode: a count above a lowered `TACCR0` rolls to 0 on the next
//!   clock. Up/down mode: counting up above a lowered `TACCR0`, the timer
//!   turns down.
//! * `IDx` divides the selected clock by 1, 2, 4 or 8. `TACLR` clears `TAR`,
//!   the divider and the up/down direction; it reads 0.
//! * `TASSELx` 2 (SMCLK) counts one CPU cycle per clock: SMCLK equals MCLK.
//!   `TASSELx` 1 (ACLK) counts at `aclk_hz` against `cpu_hz` CPU cycles per
//!   second, both from the descriptor; with no `cpu_hz` the counter holds.
//!   `TASSELx` 0 and 3 (TACLK, INCLK, outside pins) hold the counter.
//!
//! Interrupts (12.2.6):
//!
//! * `CCIFGx` is set when `TAR` counts to `TACCRx` of a block in compare
//!   mode. `TAIFG` is set when the timer counts from `TACCR0` (up mode),
//!   0xFFFF (continuous mode) or 1 (up/down mode) to 0.
//! * The flags are the interrupt sources the descriptor names
//!   (`irq_sources`: `CCIFG0` ... `CCIFGn-1`, then `TAIFG`); `CCIE` and
//!   `TAIE` are their enables. `CCIFG0` is alone on its line
//!   (`irq_lines[0]`), so acceptance clears it. The others share the `TAIV`
//!   line (`irq_lines[1]`) and stay set until firmware clears them or
//!   accesses `TAIV`.
//! * Any read or write of `TAIV` returns the highest-priority enabled
//!   pending flag (`CCIFG1` = 2, `CCIFG2` = 4, ..., `TAIFG` = 0x0A) and
//!   clears it.
//!
//! Limits:
//!
//! * Capture mode captures nothing: no capture input is modelled, `CCI`
//!   and `SCCI` read 0 and `COV` is never set.
//! * The output units drive no pad. `OUT` is storage, so output mode 0 is
//!   not visible on a pin and the PWM modes produce no waveform.
//! * SMCLK and ACLK rates do not follow the basic clock module registers.

use crate::peripherals::msp430::{Msp430Irq, SOURCES};
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::sync::Arc;

const TACTL: u64 = 0x00;
const TAR: u64 = 0x10;
const TACCR0: u64 = 0x12;

const TACLR: u16 = 0x0004;
const TAIE: u16 = 0x0002;
const TAIFG: u16 = 0x0001;
const CCIE: u16 = 0x0010;
const CCIFG: u16 = 0x0001;
const CAP: u16 = 0x0100;
/// `TACTL` bits stored: TASSELx, IDx, MCx, TAIE.
const TACTL_STORED: u16 = 0x03F2;
/// `TACCTLx` bits stored: CMx, CCISx, SCS, CAP, OUTMODx, CCIE, OUT.
const TACCTL_STORED: u16 = 0xF9F4;
/// `TAIV` value of `TAIFG`.
const TAIV_TAIFG: u16 = 0x0A;

/// The interrupt sources and request lines of one Timer_A.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerARequest {
    /// `CCIFG0` ... `CCIFGn-1`, then `TAIFG`.
    pub sources: Vec<u32>,
    /// The `TACCR0` line, then the `TAIV` line.
    pub lines: [u32; 2],
}

/// The clock rates the counter uses for ACLK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerAClocks {
    pub aclk_hz: u64,
    /// CPU cycles per second; 0 holds a counter on ACLK.
    pub cpu_hz: u64,
}

#[derive(Debug)]
pub struct Msp430TimerA {
    blocks: usize,
    request: Option<TimerARequest>,
    clocks: TimerAClocks,
    irq: Arc<Msp430Irq>,
    ctl: u16,
    cctl: Vec<u16>,
    ccr: Vec<u16>,
    tar: u16,
    /// Up/down mode is counting down.
    down: bool,
    /// Clocks counted toward the next `IDx` division.
    divider: u64,
    /// ACLK accumulator, in units of 1/`cpu_hz` ACLK periods.
    aclk_phase: u64,
}

impl Msp430TimerA {
    /// A Timer_A with `blocks` capture/compare blocks (2 to 7).
    pub fn new(blocks: usize, request: Option<TimerARequest>, clocks: TimerAClocks) -> Self {
        let blocks = blocks.clamp(2, 7);
        Self {
            blocks,
            request: request.filter(|r| {
                r.sources.len() == blocks + 1 && r.sources.iter().all(|s| *s < SOURCES)
            }),
            clocks,
            irq: Msp430Irq::new(),
            ctl: 0,
            cctl: vec![0; blocks],
            ccr: vec![0; blocks],
            tar: 0,
            down: false,
            divider: 0,
            aclk_phase: 0,
        }
    }

    /// The source behind `CCIFGx` (`x` < blocks) or `TAIFG` (`x` = blocks).
    fn source(&self, x: usize) -> Option<u32> {
        self.request.as_ref().map(|r| r.sources[x])
    }

    fn flag(&self, x: usize) -> bool {
        self.source(x).is_some_and(|s| self.irq.flag(s))
    }

    fn set_flag(&self, x: usize, on: bool) {
        if let Some(s) = self.source(x) {
            self.irq.set_flag(s, on);
        }
    }

    fn sync_enables(&self) {
        for x in 0..self.blocks {
            if let Some(s) = self.source(x) {
                self.irq.set_enable(s, self.cctl[x] & CCIE != 0);
            }
        }
        if let Some(s) = self.source(self.blocks) {
            self.irq.set_enable(s, self.ctl & TAIE != 0);
        }
    }

    fn mode(&self) -> u16 {
        (self.ctl >> 4) & 3
    }

    fn running(&self) -> bool {
        let clocked = match (self.ctl >> 8) & 3 {
            1 => self.clocks.cpu_hz != 0 && self.clocks.aclk_hz != 0,
            2 => true,
            _ => false,
        };
        match self.mode() {
            0 => false,
            2 => clocked,
            _ => clocked && self.ccr[0] != 0,
        }
    }

    fn word(&self, offset: u64) -> u16 {
        let offset = offset & !1;
        let ccr_end = TACCR0 + 2 * self.blocks as u64;
        match offset {
            TACTL => self.ctl | u16::from(self.flag(self.blocks)),
            2..=0x0E if ((offset - 2) / 2) < self.blocks as u64 => {
                let x = ((offset - 2) / 2) as usize;
                self.cctl[x] | u16::from(self.flag(x))
            }
            TAR => self.tar,
            _ if (TACCR0..ccr_end).contains(&offset) => self.ccr[((offset - TACCR0) / 2) as usize],
            _ => 0,
        }
    }

    fn set_word(&mut self, offset: u64, value: u16) {
        let offset = offset & !1;
        let ccr_end = TACCR0 + 2 * self.blocks as u64;
        match offset {
            TACTL => {
                self.ctl = value & TACTL_STORED;
                self.set_flag(self.blocks, value & TAIFG != 0);
                if value & TACLR != 0 {
                    self.tar = 0;
                    self.divider = 0;
                    self.aclk_phase = 0;
                    self.down = false;
                }
            }
            2..=0x0E if ((offset - 2) / 2) < self.blocks as u64 => {
                let x = ((offset - 2) / 2) as usize;
                self.cctl[x] = value & TACCTL_STORED;
                self.set_flag(x, value & CCIFG != 0);
            }
            TAR => self.tar = value,
            _ if (TACCR0..ccr_end).contains(&offset) => {
                self.ccr[((offset - TACCR0) / 2) as usize] = value
            }
            _ => {}
        }
        self.sync_enables();
    }

    /// Timer clocks in `cycles` CPU cycles, after the clock select and `IDx`.
    fn clocks_in(&mut self, cycles: u64) -> u64 {
        let source = match (self.ctl >> 8) & 3 {
            2 => cycles,
            1 if self.clocks.cpu_hz != 0 => {
                let phase = self.aclk_phase + cycles * self.clocks.aclk_hz;
                self.aclk_phase = phase % self.clocks.cpu_hz;
                phase / self.clocks.cpu_hz
            }
            _ => 0,
        };
        let div = 1u64 << ((self.ctl >> 6) & 3);
        let total = self.divider + source;
        self.divider = total % div;
        total / div
    }

    /// `TAR` arrived at a new value: set `CCIFGx` of every compare block
    /// whose `TACCRx` it equals.
    fn arrive(&self) {
        for x in 0..self.blocks {
            if self.cctl[x] & CAP == 0 && self.ccr[x] == self.tar {
                self.set_flag(x, true);
            }
        }
    }

    /// The nearest compare value strictly between `from` and `to`, in the
    /// counting direction, or `to`.
    fn next_stop(&self, from: u16, to: u16, up: bool) -> u16 {
        self.ccr
            .iter()
            .zip(&self.cctl)
            .filter(|(_, c)| **c & CAP == 0)
            .map(|(v, _)| *v)
            .filter(|v| {
                if up {
                    *v > from && *v < to
                } else {
                    *v < from && *v > to
                }
            })
            .fold(to, |best, v| if up { best.min(v) } else { best.max(v) })
    }

    /// Count `clocks` timer clocks, jumping between events.
    fn advance(&mut self, mut clocks: u64) {
        while clocks > 0 {
            let ccr0 = self.ccr[0];
            let (limit, up) = match self.mode() {
                1 | 3 if ccr0 == 0 => return,
                1 if self.tar >= ccr0 => {
                    self.tar = 0;
                    clocks -= 1;
                    self.set_flag(self.blocks, true);
                    self.arrive();
                    continue;
                }
                1 => (ccr0, true),
                2 if self.tar == 0xFFFF => {
                    self.tar = 0;
                    clocks -= 1;
                    self.set_flag(self.blocks, true);
                    self.arrive();
                    continue;
                }
                2 => (0xFFFF, true),
                3 => {
                    if !self.down && self.tar >= ccr0 {
                        self.down = true;
                    }
                    if self.down && self.tar == 0 {
                        self.down = false;
                    }
                    if self.down {
                        (0, false)
                    } else {
                        (ccr0, true)
                    }
                }
                _ => return,
            };
            let stop = self.next_stop(self.tar, limit, up);
            let span = u64::from(stop.abs_diff(self.tar));
            if clocks < span {
                self.tar = if up {
                    self.tar + clocks as u16
                } else {
                    self.tar - clocks as u16
                };
                return;
            }
            clocks -= span;
            self.tar = stop;
            self.arrive();
            if self.mode() == 3 && !up && self.tar == 0 {
                self.set_flag(self.blocks, true);
            }
        }
    }
}

impl Peripheral for Msp430TimerA {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.word(offset) >> (8 * (offset & 1))) as u8)
    }

    /// A byte store writes the low byte and clears the upper one.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset & 1 == 0 {
            self.set_word(offset, u16::from(value));
        }
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(self.word(offset))
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.set_word(offset, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn legacy_tick_active(&self) -> bool {
        self.running()
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if self.running() {
            let clocks = self.clocks_in(cycles);
            self.advance(clocks);
        }
        PeripheralTickResult::default()
    }

    fn attach_msp430_irq(&mut self, irq: &Arc<Msp430Irq>) {
        if let Some(r) = &self.request {
            irq.route(r.sources[0], r.lines[0], true);
            for s in &r.sources[1..] {
                irq.route(*s, r.lines[1], false);
            }
        }
        self.irq = irq.clone();
        self.on_system_reset();
    }

    fn on_system_reset(&mut self) {
        self.ctl = 0;
        self.cctl.fill(0);
        self.ccr.fill(0);
        self.tar = 0;
        self.down = false;
        self.divider = 0;
        self.aclk_phase = 0;
        for x in 0..=self.blocks {
            self.set_flag(x, false);
        }
        self.sync_enables();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "tactl": self.word(TACTL),
            "tar": self.tar,
            "tacctl": (0..self.blocks).map(|x| self.word(2 + 2 * x as u64)).collect::<Vec<_>>(),
            "taccr": self.ccr,
            "down": self.down,
        })
    }
}

/// `TAIV` of one Timer_A: the highest-priority enabled pending flag of
/// `CCIFG1` ... `CCIFGn-1` and `TAIFG`, cleared by any access.
#[derive(Debug)]
pub struct Msp430TimerAIv {
    /// `CCIFG1` ... `CCIFGn-1`, then `TAIFG`, highest priority first.
    sources: Vec<u32>,
    irq: Arc<Msp430Irq>,
}

impl Msp430TimerAIv {
    pub fn new(sources: Vec<u32>) -> Self {
        Self {
            sources,
            irq: Msp430Irq::new(),
        }
    }

    /// The highest-priority enabled pending source and its `TAIV` value.
    fn pending(&self) -> Option<(u32, u16)> {
        let last = self.sources.len().checked_sub(1)?;
        self.sources.iter().enumerate().find_map(|(i, s)| {
            (self.irq.flag(*s) && self.irq.enabled(*s)).then(|| {
                let value = if i == last {
                    TAIV_TAIFG
                } else {
                    2 * (i as u16 + 1)
                };
                (*s, value)
            })
        })
    }

    /// The access that clears the flag `TAIV` reports.
    fn take(&self) -> u16 {
        match self.pending() {
            Some((s, value)) => {
                self.irq.set_flag(s, false);
                value
            }
            None => 0,
        }
    }
}

impl Peripheral for Msp430TimerAIv {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // The upper byte is always 0; only the low byte access clears.
        Ok(if offset & 1 == 0 {
            self.take() as u8
        } else {
            0
        })
    }

    fn write(&mut self, offset: u64, _value: u8) -> SimResult<()> {
        if offset & 1 == 0 {
            self.take();
        }
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(if offset & 1 == 0 { self.take() } else { 0 })
    }

    fn write_u16(&mut self, offset: u64, _value: u16) -> SimResult<()> {
        self.write(offset, 0)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some(if offset & 1 == 0 {
            self.pending().map_or(0, |(_, v)| v as u8)
        } else {
            0
        })
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn attach_msp430_irq(&mut self, irq: &Arc<Msp430Irq>) {
        self.irq = irq.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MC_UP: u16 = 0x0010;
    const MC_CONT: u16 = 0x0020;
    const MC_UPDOWN: u16 = 0x0030;
    const SMCLK: u16 = 0x0200;
    const ACLK: u16 = 0x0100;
    const ID_8: u16 = 0x00C0;

    fn timer(irq: &Arc<Msp430Irq>) -> (Msp430TimerA, Msp430TimerAIv) {
        let request = TimerARequest {
            sources: vec![26, 27, 28, 29],
            lines: [6, 7],
        };
        let clocks = TimerAClocks {
            aclk_hz: 32_768,
            cpu_hz: 1_048_576,
        };
        let mut t = Msp430TimerA::new(3, Some(request), clocks);
        t.attach_msp430_irq(irq);
        let mut iv = Msp430TimerAIv::new(vec![27, 28, 29]);
        iv.attach_msp430_irq(irq);
        (t, iv)
    }

    #[test]
    fn up_mode_sets_ccifg0_at_taccr0_and_taifg_on_the_roll_to_zero() {
        let irq = Msp430Irq::new();
        let (mut t, _) = timer(&irq);
        t.write_u16(TACCR0, 99).unwrap();
        t.write_u16(2, CCIE).unwrap();
        t.write_u16(TACTL, SMCLK | MC_UP | TACLR).unwrap();
        assert!(t.legacy_tick_active());
        t.tick_elapsed(98);
        assert_eq!(t.read_u16(TAR).unwrap(), 98);
        assert_eq!(irq.pending(), 0);
        t.tick_elapsed(1);
        assert_eq!(t.read_u16(TAR).unwrap(), 99);
        assert_eq!(t.read_u16(2).unwrap() & CCIFG, CCIFG);
        assert_eq!(irq.pending(), 1 << 6);
        assert_eq!(t.read_u16(TACTL).unwrap() & TAIFG, 0);
        t.tick_elapsed(1);
        assert_eq!(t.read_u16(TAR).unwrap(), 0);
        assert_eq!(t.read_u16(TACTL).unwrap() & TAIFG, TAIFG);
        // TAIE is clear: TAIFG requests nothing.
        assert_eq!(irq.pending(), 1 << 6);
        // One period of 100 clocks later the flag is set again.
        t.write_u16(2, CCIE).unwrap();
        t.tick_elapsed(99);
        assert_eq!(t.read_u16(2).unwrap() & CCIFG, CCIFG);
    }

    #[test]
    fn acceptance_of_the_taccr0_line_clears_ccifg0() {
        let irq = Msp430Irq::new();
        let (mut t, _) = timer(&irq);
        t.write_u16(TACCR0, 9).unwrap();
        t.write_u16(2, CCIE).unwrap();
        t.write_u16(TACTL, SMCLK | MC_UP).unwrap();
        t.tick_elapsed(9);
        assert_eq!(irq.pending(), 1 << 6);
        irq.accept(6);
        assert_eq!(irq.pending(), 0);
        assert_eq!(t.read_u16(2).unwrap() & CCIFG, 0);
    }

    #[test]
    fn continuous_mode_compares_every_block_and_taiv_reports_by_priority() {
        let irq = Msp430Irq::new();
        let (mut t, iv) = timer(&irq);
        t.write_u16(4, CCIE).unwrap();
        t.write_u16(6, CCIE).unwrap();
        t.write_u16(TACCR0 + 2, 0x1000).unwrap();
        t.write_u16(TACCR0 + 4, 0x0800).unwrap();
        t.write_u16(TACTL, SMCLK | MC_CONT | TAIE | TACLR).unwrap();
        t.tick_elapsed(0x10000);
        assert_eq!(t.read_u16(TAR).unwrap(), 0);
        assert_eq!(irq.pending(), 1 << 7);
        // The shared line: acceptance clears nothing; TAIV clears one flag
        // per access, highest priority first.
        irq.accept(7);
        assert_eq!(iv.peek(0), Some(2));
        assert_eq!(iv.read_u16(0).unwrap(), 2);
        assert_eq!(iv.read_u16(0).unwrap(), 4);
        assert_eq!(iv.read_u16(0).unwrap(), TAIV_TAIFG);
        assert_eq!(iv.read_u16(0).unwrap(), 0);
        assert_eq!(irq.pending(), 0);
    }

    #[test]
    fn taiv_skips_a_pending_flag_whose_enable_is_clear() {
        let irq = Msp430Irq::new();
        let (mut t, iv) = timer(&irq);
        t.write_u16(4, CCIFG).unwrap();
        t.write_u16(TACTL, TAIE | TAIFG).unwrap();
        assert_eq!(iv.read_u16(0).unwrap(), TAIV_TAIFG);
        assert_eq!(t.read_u16(4).unwrap() & CCIFG, CCIFG);
        assert_eq!(iv.read_u16(0).unwrap(), 0);
    }

    #[test]
    fn up_down_mode_turns_at_taccr0_and_sets_taifg_at_zero() {
        let irq = Msp430Irq::new();
        let (mut t, _) = timer(&irq);
        t.write_u16(TACCR0, 4).unwrap();
        t.write_u16(TACCR0 + 2, 2).unwrap();
        t.write_u16(TACTL, SMCLK | MC_UPDOWN | TACLR).unwrap();
        let mut seen = Vec::new();
        for _ in 0..9 {
            t.tick_elapsed(1);
            seen.push(t.read_u16(TAR).unwrap());
        }
        assert_eq!(seen, [1, 2, 3, 4, 3, 2, 1, 0, 1]);
        assert_eq!(t.read_u16(TACTL).unwrap() & TAIFG, TAIFG);
        assert_eq!(t.read_u16(2).unwrap() & CCIFG, CCIFG);
        assert_eq!(t.read_u16(4).unwrap() & CCIFG, CCIFG);
    }

    #[test]
    fn up_mode_rolls_to_zero_above_a_lowered_taccr0_and_holds_at_zero() {
        let irq = Msp430Irq::new();
        let (mut t, _) = timer(&irq);
        t.write_u16(TACCR0, 1000).unwrap();
        t.write_u16(TACTL, SMCLK | MC_UP).unwrap();
        t.tick_elapsed(500);
        t.write_u16(TACCR0, 100).unwrap();
        t.tick_elapsed(1);
        assert_eq!(t.read_u16(TAR).unwrap(), 0);
        t.write_u16(TACCR0, 0).unwrap();
        assert!(!t.legacy_tick_active());
    }

    #[test]
    fn the_input_divider_and_aclk_scale_the_count() {
        let irq = Msp430Irq::new();
        let (mut t, _) = timer(&irq);
        t.write_u16(TACTL, SMCLK | ID_8 | MC_CONT).unwrap();
        t.tick_elapsed(7);
        assert_eq!(t.read_u16(TAR).unwrap(), 0);
        t.tick_elapsed(1);
        assert_eq!(t.read_u16(TAR).unwrap(), 1);
        // ACLK = cpu_hz / 32: one count per 32 CPU cycles.
        t.write_u16(TACTL, ACLK | MC_CONT | TACLR).unwrap();
        t.tick_elapsed(31);
        assert_eq!(t.read_u16(TAR).unwrap(), 0);
        t.tick_elapsed(32 * 10 + 1);
        assert_eq!(t.read_u16(TAR).unwrap(), 11);
    }

    #[test]
    fn taclr_reads_zero_and_a_puc_clears_the_flags() {
        let irq = Msp430Irq::new();
        let (mut t, _) = timer(&irq);
        t.write_u16(TACTL, SMCLK | MC_CONT | TACLR | TAIE).unwrap();
        assert_eq!(t.read_u16(TACTL).unwrap() & TACLR, 0);
        t.write_u16(2, CCIE | CCIFG).unwrap();
        assert_eq!(irq.pending(), 1 << 6);
        t.on_system_reset();
        assert_eq!(irq.pending(), 0);
        assert_eq!(t.read_u16(TACTL).unwrap(), 0);
        assert!(!t.legacy_tick_active());
    }
}
