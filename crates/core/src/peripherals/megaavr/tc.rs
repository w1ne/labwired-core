// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! megaAVR 0-series timers: TCA0 and TCB0..TCB3 as one block (ATmega4808/4809
//! data sheet DS40002173 §20 "TCA" and §21 "TCB"; offsets and reset values
//! from the Microchip ATmega4809 ATDF). One window, data space 0x0A00..0x0AC0,
//! because a TCB with CLKSEL = CLKTCA counts on TCA0's prescaled clock.
//!
//! Counts are derived lazily from the CPU cycle count (CLK_PER is taken as the
//! CPU clock). A prescaler divides a free-running count of CLK_PER from
//! reset, so its phase is the silicon's.
//!
//! TCA0, normal (non-split) mode: CTRLA.ENABLE and CLKSEL (DIV1..DIV1024),
//! WGMODE NORMAL and SINGLESLOPE (both count 0..PER and wrap, setting
//! INTFLAGS.OVF), CNT / PER / CMPn and their buffers. PERBUF / CMPnBUF set
//! the CTRLFSET valid bits and are copied at the next wrap (UPDATE).
//!
//! TCA0 waveform outputs WO0..WO2 in single-slope PWM (DS40002173 §20.3.3.4.3,
//! what `analogWrite` on a TCA0 pin runs): WOn is set at BOTTOM and cleared on
//! the CMPn match, so it is high for CNT < CMPn (CMPn > PER: high all period,
//! CMPn = 0: low). A CMPnBUF write takes over on the pad at the next wrap, as
//! the compare register does. With CTRLB.CMPnEN set the waveform drives the
//! line (and the pad, while it is an output and PORTMUX.TCAROUTEA routes TCA0
//! to its port; see `SystemBus::wire_megaavr_tca_output_pads`); with TCA0
//! stopped the line holds CTRLC.CMPnOV. Edges are published at their exact
//! cycle on the timer's own wake. A buffered PER reaches the pads at the first
//! register access or wake after the wrap rather than at the wrap itself.
//!
//! Not modelled, and refused when firmware asks for them: split mode, FRQ and
//! the dual-slope modes, compare outputs in NORMAL mode, TCA interrupts and
//! the event input. CTRLESET commands are refused too.
//!
//! TCB, periodic interrupt (CNTMODE INT) and 8-bit PWM (CNTMODE PWM8): the
//! counter runs 0..TOP and wraps, setting INTFLAGS.CAPT, where TOP is CCMP in
//! INT mode and CCMPL in PWM8 mode. CLKSEL DIV1, DIV2 and CLKTCA (which stops
//! while TCA0 is disabled). INTCTRL.CAPT raises the instance's vector once per
//! wrap. The other count modes, the waveform output (CTRLB.CCMPEN) and the
//! event input are refused.
//!
//! 16-bit registers go through the instance's TEMP byte as on silicon: a low
//! byte write lands in TEMP and the high byte write commits both; a low byte
//! read latches the high byte into TEMP for the following high byte read.

use crate::peripherals::esp_ledc_wave::{LedcDrive, LedcPads, LedcTimebase};
use crate::peripherals::pad_lines::LineDrive;
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::sync::Mutex;

/// TCA0 waveform output lines, in WOn order.
const TCA_OUTPUT_LINES: &[&str] = &["TCA0_WO0", "TCA0_WO1", "TCA0_WO2"];

/// Window offset of TCB n.
const TCB_BASE: [u64; 4] = [0x80, 0x90, 0xA0, 0xB0];
pub const WINDOW: u64 = 0xC0;

// TCA0 (single mode) registers.
const TCA_CTRLA: u64 = 0x00;
const TCA_CTRLB: u64 = 0x01;
const TCA_CTRLC: u64 = 0x02;
const TCA_CTRLD: u64 = 0x03;
const TCA_CTRLECLR: u64 = 0x04;
const TCA_CTRLESET: u64 = 0x05;
const TCA_CTRLFCLR: u64 = 0x06;
const TCA_CTRLFSET: u64 = 0x07;
const TCA_EVCTRL: u64 = 0x09;
const TCA_INTCTRL: u64 = 0x0A;
const TCA_INTFLAGS: u64 = 0x0B;
const TCA_DBGCTRL: u64 = 0x0E;
const TCA_TEMP: u64 = 0x0F;
const TCA_CNT: u64 = 0x20;
const TCA_PER: u64 = 0x26;
const TCA_CMP0: u64 = 0x28;
const TCA_PERBUF: u64 = 0x36;
const TCA_CMP0BUF: u64 = 0x38;
const TCA_ENABLE: u8 = 1 << 0;
const TCA_WGMODE: u8 = 0x07;
const TCA_SINGLESLOPE: u8 = 3;
const TCA_CMPEN: u8 = 0x70;
const TCA_OVF: u8 = 1 << 0;
/// CTRLFSET: PERBV bit 0, CMP0BV..CMP2BV bits 1..3.
const TCA_PERBV: u8 = 1 << 0;

// TCB registers.
const TCB_CTRLA: u64 = 0x00;
const TCB_CTRLB: u64 = 0x01;
const TCB_EVCTRL: u64 = 0x04;
const TCB_INTCTRL: u64 = 0x05;
const TCB_INTFLAGS: u64 = 0x06;
const TCB_STATUS: u64 = 0x07;
const TCB_DBGCTRL: u64 = 0x08;
const TCB_TEMP: u64 = 0x09;
const TCB_CNT: u64 = 0x0A;
const TCB_CCMP: u64 = 0x0C;
const TCB_ENABLE: u8 = 1 << 0;
const TCB_CAPT: u8 = 1 << 0;
const TCB_CNTMODE: u8 = 0x07;
const TCB_MODE_INT: u8 = 0;
const TCB_MODE_PWM8: u8 = 7;
const TCB_CCMPEN: u8 = 1 << 4;
const TCB_CLKSEL_TCA: u8 = 2;

#[derive(Debug, Default, Clone, Copy)]
struct Tca {
    ctrla: u8,
    ctrlb: u8,
    ctrlc: u8,
    ctrld: u8,
    ctrle: u8,
    ctrlf: u8,
    evctrl: u8,
    intctrl: u8,
    /// Flags latched by a settled wrap (reads also see unsettled ones).
    intflags: u8,
    dbgctrl: u8,
    temp: u8,
    per: u16,
    cmp: [u16; 3],
    perbuf: u16,
    cmpbuf: [u16; 3],
    /// Count at `anchor_tick` (prescaled ticks since reset).
    anchor_count: u16,
    anchor_tick: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct Tcb {
    ctrla: u8,
    ctrlb: u8,
    evctrl: u8,
    intctrl: u8,
    intflags: u8,
    dbgctrl: u8,
    temp: u8,
    ccmp: u16,
    anchor_count: u16,
    anchor_tick: u64,
}

#[derive(Debug, Default)]
struct State {
    tca: Tca,
    tcb: [Tcb; 4],
    /// CPU cycle of the one live scheduler wake, and its generation token.
    live_at: Option<u64>,
    generation: u32,
    pending_events: Vec<(u64, u32)>,
    /// Vectors of wraps a register write settled, not yet delivered.
    raised: Vec<u32>,
}

/// TCA0 + TCB0..3.
#[derive(Debug)]
pub struct MegaAvrTimers {
    state: Mutex<State>,
    /// TCB0..3 vector numbers (12, 13, 25, 36 on the ATmega4809).
    tcb_irqs: [u32; 4],
    clock: Option<CycleClock>,
    /// TCA0 WO0..WO2; generated only once the bus routed a pad to them.
    outputs: LedcPads,
    wired: bool,
}

/// CLK_PER cycles per TCA0 prescaled tick.
fn tca_div(ctrla: u8) -> u64 {
    [1, 2, 4, 8, 16, 64, 256, 1024][usize::from((ctrla >> 1) & 7)]
}

impl Tca {
    fn running(&self) -> bool {
        self.ctrla & TCA_ENABLE != 0
    }
    fn top(&self) -> u64 {
        u64::from(self.per)
    }
    /// Count and wraps after `n` ticks from the anchor. The first wrap is at
    /// the current PER; buffered values take over from then on.
    fn after(&self, n: u64) -> (u16, u64) {
        let start = u64::from(self.anchor_count);
        let first = if start <= self.top() {
            self.top() + 1 - start
        } else {
            0x1_0000 - start
        };
        if n < first {
            return ((start + n) as u16, 0);
        }
        let per = if self.ctrlf & TCA_PERBV != 0 {
            self.perbuf
        } else {
            self.per
        };
        let period = u64::from(per) + 1;
        let rest = n - first;
        ((rest % period) as u16, 1 + rest / period)
    }
}

impl Tcb {
    fn running(&self) -> bool {
        self.ctrla & TCB_ENABLE != 0
    }
    fn top(&self) -> u64 {
        if self.ctrlb & TCB_CNTMODE == TCB_MODE_PWM8 {
            u64::from(self.ccmp & 0xFF)
        } else {
            u64::from(self.ccmp)
        }
    }
    fn after(&self, n: u64) -> (u16, u64) {
        let start = u64::from(self.anchor_count);
        let top = self.top();
        let first = if start <= top {
            top + 1 - start
        } else {
            0x1_0000 - start
        };
        if n < first {
            return ((start + n) as u16, 0);
        }
        let rest = n - first;
        ((rest % (top + 1)) as u16, 1 + rest / (top + 1))
    }
}

impl MegaAvrTimers {
    pub fn new(tcb_irqs: [u32; 4]) -> Self {
        Self {
            state: Mutex::new(State::default()),
            tcb_irqs,
            clock: None,
            outputs: LedcPads::new(TCA_OUTPUT_LINES),
            wired: false,
        }
    }

    /// What WOc drives with the registers settled at their anchor, and
    /// whether it drives the line at all (CTRLB.CMPnEN), plus the buffered
    /// compare value's drive that takes over at the next wrap.
    fn output_drive(tca: &Tca, c: usize) -> (LedcDrive, Option<LedcDrive>, bool) {
        let enabled = tca.ctrlb & (0x10 << c) != 0;
        let ov = LedcDrive::Level(tca.ctrlc & (1 << c) != 0);
        if !tca.running() || tca.ctrlb & TCA_WGMODE != TCA_SINGLESLOPE {
            return (ov, None, enabled);
        }
        let period = u64::from(tca.per) + 1;
        let count = u64::from(tca.anchor_count);
        if count >= period {
            // Counting on past PER towards the 16-bit wrap: no BOTTOM yet.
            return (
                LedcDrive::Level(count < u64::from(tca.cmp[c])),
                None,
                enabled,
            );
        }
        let div = tca_div(tca.ctrla);
        // BOTTOM, as a tick index reduced modulo the period, so the time base
        // is the same value however many wraps the anchor has moved over.
        let origin = (tca.anchor_tick % period + period - count) % period;
        let timer = LedcTimebase {
            period,
            num: u128::from(div),
            den: 1,
            origin: origin * div,
        };
        let pwm = |cmp: u16| LedcDrive::Pwm {
            timer,
            hpoint: 0,
            duty: u64::from(cmp).min(period),
        };
        let buffered = (tca.ctrlf & (TCA_PERBV << (c + 1)) != 0).then(|| pwm(tca.cmpbuf[c]));
        (pwm(tca.cmp[c]), buffered, enabled)
    }

    /// Re-derive the output lines from the settled registers at `now`.
    fn refresh_outputs(outputs: &mut LedcPads, wired: bool, tca: &Tca, now: u64) {
        if !wired {
            return;
        }
        let lines = outputs.lines_arc();
        for c in 0..TCA_OUTPUT_LINES.len() {
            let (drive, buffered, enabled) = Self::output_drive(tca, c);
            outputs.set_drive(c, drive, now);
            if let Some(next) = buffered {
                outputs.set_drive(c, next, now);
            }
            lines.set_mode(
                c,
                if enabled {
                    LineDrive::PushPull
                } else {
                    LineDrive::Input
                },
            );
        }
    }

    /// Cycle of the next output edge (or staged compare taking over).
    fn next_output_cycle(&self) -> Option<u64> {
        if self.wired {
            self.outputs.next_change()
        } else {
            None
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    /// CLK_PER cycles per tick of TCB `t`, or `None` while it has no clock.
    fn tcb_div(s: &State, t: &Tcb) -> Option<u64> {
        if !t.running() {
            return None;
        }
        match (t.ctrla >> 1) & 3 {
            0 => Some(1),
            1 => Some(2),
            TCB_CLKSEL_TCA if s.tca.running() => Some(tca_div(s.tca.ctrla)),
            _ => None,
        }
    }

    fn tca_ticks(s: &State, now: u64) -> u64 {
        now / tca_div(s.tca.ctrla)
    }

    /// TCA0 count and wraps since its anchor, as of `now`.
    fn tca_view(s: &State, now: u64) -> (u16, u64) {
        if !s.tca.running() {
            return (s.tca.anchor_count, 0);
        }
        s.tca
            .after(Self::tca_ticks(s, now).saturating_sub(s.tca.anchor_tick))
    }

    fn tcb_view(s: &State, i: usize, now: u64) -> (u16, u64) {
        let t = &s.tcb[i];
        match Self::tcb_div(s, t) {
            Some(div) => t.after((now / div).saturating_sub(t.anchor_tick)),
            None => (t.anchor_count, 0),
        }
    }

    /// Fold every elapsed wrap into the latched state.
    fn settle(s: &mut State, now: u64) -> Vec<usize> {
        let (count, wraps) = Self::tca_view(s, now);
        if wraps > 0 {
            let tca = &mut s.tca;
            tca.intflags |= TCA_OVF;
            if tca.ctrlf & TCA_PERBV != 0 {
                tca.per = tca.perbuf;
            }
            for c in 0..3 {
                if tca.ctrlf & (TCA_PERBV << (c + 1)) != 0 {
                    tca.cmp[c] = tca.cmpbuf[c];
                }
            }
            tca.ctrlf = 0;
        }
        let tca_tick = Self::tca_ticks(s, now);
        s.tca.anchor_count = count;
        s.tca.anchor_tick = tca_tick;
        let mut rose = Vec::new();
        for i in 0..4 {
            let (count, wraps) = Self::tcb_view(s, i, now);
            let div = Self::tcb_div(s, &s.tcb[i]).unwrap_or(1);
            let t = &mut s.tcb[i];
            if wraps > 0 {
                if t.intflags & TCB_CAPT == 0 {
                    rose.push(i);
                }
                t.intflags |= TCB_CAPT;
            }
            t.anchor_count = count;
            t.anchor_tick = now / div;
        }
        rose
    }

    /// CPU cycle of the next CAPT wrap of every TCB whose interrupt is on.
    fn next_irq_cycle(s: &State) -> Option<u64> {
        (0..4)
            .filter(|&i| s.tcb[i].intctrl & TCB_CAPT != 0)
            .filter_map(|i| {
                let t = &s.tcb[i];
                let div = Self::tcb_div(s, t)?;
                let start = u64::from(t.anchor_count);
                let top = t.top();
                let to_wrap = if start <= top {
                    top + 1 - start
                } else {
                    0x1_0000 - start
                };
                Some((t.anchor_tick + to_wrap) * div)
            })
            .min()
    }

    fn rearm(&self, s: &mut State, output_edge: Option<u64>) {
        if !self.scheduler_mode() {
            return;
        }
        let Some(fire) = [Self::next_irq_cycle(s), output_edge]
            .into_iter()
            .flatten()
            .min()
        else {
            return;
        };
        let now = self.now();
        let fire = fire.max(now + 1);
        if s.live_at.is_some_and(|live| live <= fire) {
            return;
        }
        s.generation = s.generation.wrapping_add(1);
        s.live_at = Some(fire);
        s.pending_events.push((fire - now, s.generation));
    }

    fn irqs_for(&self, s: &State, rose: &[usize]) -> Vec<u32> {
        rose.iter()
            .filter(|&&i| s.tcb[i].intctrl & TCB_CAPT != 0)
            .map(|&i| self.tcb_irqs[i])
            .collect()
    }

    fn refuse(what: String) -> SimResult<()> {
        Err(SimulationError::NotImplemented(format!(
            "megaAVR timers: {what} is not modelled"
        )))
    }

    fn read_reg(&self, s: &mut State, o: u64, now: u64) -> u8 {
        if o < 0x40 {
            return Self::tca_read(s, o, now);
        }
        let Some(i) = TCB_BASE.iter().position(|&b| (b..b + 0x10).contains(&o)) else {
            return 0;
        };
        let r = o - TCB_BASE[i];
        let (count, wraps) = Self::tcb_view(s, i, now);
        let t = &mut s.tcb[i];
        match r {
            TCB_CTRLA => t.ctrla,
            TCB_CTRLB => t.ctrlb,
            TCB_EVCTRL => t.evctrl,
            TCB_INTCTRL => t.intctrl,
            TCB_INTFLAGS => t.intflags | u8::from(wraps > 0),
            TCB_STATUS => u8::from(Self::tcb_div_static(&s.tca, t).is_some()),
            TCB_DBGCTRL => t.dbgctrl,
            TCB_TEMP => t.temp,
            TCB_CNT => {
                t.temp = (count >> 8) as u8;
                count as u8
            }
            TCB_CCMP => {
                t.temp = (t.ccmp >> 8) as u8;
                t.ccmp as u8
            }
            0x0B | 0x0D => t.temp,
            _ => 0,
        }
    }

    fn tcb_div_static(tca: &Tca, t: &Tcb) -> Option<u64> {
        if !t.running() {
            return None;
        }
        match (t.ctrla >> 1) & 3 {
            0 => Some(1),
            1 => Some(2),
            TCB_CLKSEL_TCA if tca.running() => Some(tca_div(tca.ctrla)),
            _ => None,
        }
    }

    fn tca_read(s: &mut State, o: u64, now: u64) -> u8 {
        let (count, wraps) = Self::tca_view(s, now);
        let tca = &mut s.tca;
        let word = |tca: &mut Tca, v: u16| {
            tca.temp = (v >> 8) as u8;
            v as u8
        };
        match o {
            TCA_CTRLA => tca.ctrla,
            TCA_CTRLB => tca.ctrlb,
            TCA_CTRLC => tca.ctrlc,
            TCA_CTRLD => tca.ctrld,
            TCA_CTRLECLR | TCA_CTRLESET => tca.ctrle,
            TCA_CTRLFCLR | TCA_CTRLFSET => {
                if wraps > 0 {
                    0
                } else {
                    tca.ctrlf
                }
            }
            TCA_EVCTRL => tca.evctrl,
            TCA_INTCTRL => tca.intctrl,
            TCA_INTFLAGS => tca.intflags | if wraps > 0 { TCA_OVF } else { 0 },
            TCA_DBGCTRL => tca.dbgctrl,
            TCA_TEMP => tca.temp,
            TCA_CNT => word(tca, count),
            TCA_PER => {
                let per = if wraps > 0 && tca.ctrlf & TCA_PERBV != 0 {
                    tca.perbuf
                } else {
                    tca.per
                };
                word(tca, per)
            }
            0x28 | 0x2A | 0x2C => {
                let c = ((o - TCA_CMP0) / 2) as usize;
                let v = if wraps > 0 && tca.ctrlf & (TCA_PERBV << (c + 1)) != 0 {
                    tca.cmpbuf[c]
                } else {
                    tca.cmp[c]
                };
                word(tca, v)
            }
            TCA_PERBUF => word(tca, tca.perbuf),
            0x38 | 0x3A | 0x3C => {
                let c = ((o - TCA_CMP0BUF) / 2) as usize;
                word(tca, tca.cmpbuf[c])
            }
            0x21 | 0x27 | 0x29 | 0x2B | 0x2D | 0x37 | 0x39 | 0x3B | 0x3D => tca.temp,
            _ => 0,
        }
    }

    fn write_reg(&mut self, o: u64, v: u8) -> SimResult<Vec<u32>> {
        let now = self.now();
        let mut guard = self.state.lock().unwrap();
        let s = &mut *guard;
        let rose = Self::settle(s, now);
        let irqs = self.irqs_for(s, &rose);
        if o < 0x40 {
            Self::tca_write(s, o, v, now)?;
        } else if let Some(i) = TCB_BASE.iter().position(|&b| (b..b + 0x10).contains(&o)) {
            Self::tcb_write(s, i, o - TCB_BASE[i], v, now)?;
        }
        // A configuration change re-anchors every counter at `now`.
        Self::settle(s, now);
        Self::refresh_outputs(&mut self.outputs, self.wired, &s.tca, now);
        let edge = self.next_output_cycle();
        self.rearm(s, edge);
        Ok(irqs)
    }

    fn tca_write(s: &mut State, o: u64, v: u8, now: u64) -> SimResult<()> {
        let tca = &mut s.tca;
        let commit = |tca: &mut Tca| (u16::from(v) << 8) | u16::from(tca.temp);
        match o {
            TCA_CTRLA => {
                // The caller settled every counter at `now`: re-anchor them
                // on the new tick rate, TCA0's own and its CLKTCA users'.
                tca.ctrla = v & 0x8F;
                tca.anchor_tick = now / tca_div(tca.ctrla);
                let tca = *tca;
                for t in &mut s.tcb {
                    if let Some(div) = Self::tcb_div_static(&tca, t) {
                        t.anchor_tick = now / div;
                    }
                }
                Self::tca_check(&tca)?;
            }
            TCA_CTRLB => {
                tca.ctrlb = v & 0x7F;
                Self::tca_check(tca)?;
            }
            TCA_CTRLC => tca.ctrlc = v & 0x07,
            TCA_CTRLD => {
                tca.ctrld = v & 0x01;
                Self::tca_check(tca)?;
            }
            TCA_CTRLECLR => tca.ctrle &= !v,
            TCA_CTRLESET => {
                if v & 0x0C != 0 {
                    return Self::refuse(format!("TCA0.CTRLESET command {v:#04x}"));
                }
                tca.ctrle |= v & 0x03;
            }
            TCA_CTRLFCLR => tca.ctrlf &= !v,
            TCA_CTRLFSET => tca.ctrlf |= v & 0x0F,
            TCA_EVCTRL => {
                if v & 0x01 != 0 {
                    return Self::refuse("TCA0 event input".into());
                }
                tca.evctrl = v;
            }
            TCA_INTCTRL => {
                if v != 0 {
                    return Self::refuse(format!("TCA0 interrupts (INTCTRL {v:#04x})"));
                }
                tca.intctrl = v;
            }
            TCA_INTFLAGS => tca.intflags &= !v,
            TCA_DBGCTRL => tca.dbgctrl = v & 0x01,
            TCA_TEMP => tca.temp = v,
            0x20 | 0x26 | 0x28 | 0x2A | 0x2C | 0x36 | 0x38 | 0x3A | 0x3C => tca.temp = v,
            0x21 => tca.anchor_count = commit(tca),
            0x27 => tca.per = commit(tca),
            0x29 | 0x2B | 0x2D => tca.cmp[((o - 0x29) / 2) as usize] = commit(tca),
            0x37 => {
                tca.perbuf = commit(tca);
                tca.ctrlf |= TCA_PERBV;
            }
            0x39 | 0x3B | 0x3D => {
                let c = ((o - 0x39) / 2) as usize;
                tca.cmpbuf[c] = commit(tca);
                tca.ctrlf |= TCA_PERBV << (c + 1);
            }
            _ => {}
        }
        Ok(())
    }

    fn tca_check(tca: &Tca) -> SimResult<()> {
        if !tca.running() {
            return Ok(());
        }
        if tca.ctrld & 0x01 != 0 {
            return Self::refuse("TCA0 split mode".into());
        }
        if !matches!(tca.ctrlb & TCA_WGMODE, 0 | 3) {
            return Self::refuse(format!("TCA0 WGMODE {}", tca.ctrlb & TCA_WGMODE));
        }
        if tca.ctrlb & TCA_CMPEN != 0 && tca.ctrlb & TCA_WGMODE != TCA_SINGLESLOPE {
            return Self::refuse("TCA0 compare outputs in NORMAL mode (CTRLB.CMPnEN)".into());
        }
        Ok(())
    }

    fn tcb_write(s: &mut State, i: usize, r: u64, v: u8, now: u64) -> SimResult<()> {
        let tca = s.tca;
        let t = &mut s.tcb[i];
        match r {
            TCB_CTRLA => {
                t.ctrla = v & 0x57;
                t.anchor_tick = now / Self::tcb_div_static(&tca, t).unwrap_or(1);
                Self::tcb_check(i, t)?;
            }
            TCB_CTRLB => {
                t.ctrlb = v & 0x77;
                Self::tcb_check(i, t)?;
            }
            TCB_EVCTRL => {
                if v & 0x01 != 0 {
                    return Self::refuse(format!("TCB{i} event input"));
                }
                t.evctrl = v;
            }
            TCB_INTCTRL => t.intctrl = v & TCB_CAPT,
            TCB_INTFLAGS => t.intflags &= !v,
            TCB_DBGCTRL => t.dbgctrl = v & 0x01,
            TCB_TEMP | TCB_CNT | TCB_CCMP => t.temp = v,
            0x0B => t.anchor_count = (u16::from(v) << 8) | u16::from(t.temp),
            0x0D => t.ccmp = (u16::from(v) << 8) | u16::from(t.temp),
            _ => {}
        }
        Ok(())
    }

    fn tcb_check(i: usize, t: &Tcb) -> SimResult<()> {
        if !t.running() {
            return Ok(());
        }
        if !matches!(t.ctrlb & TCB_CNTMODE, TCB_MODE_INT | TCB_MODE_PWM8) {
            return Self::refuse(format!("TCB{i} CNTMODE {}", t.ctrlb & TCB_CNTMODE));
        }
        if t.ctrlb & TCB_CCMPEN != 0 {
            return Self::refuse(format!("TCB{i} waveform output (CTRLB.CCMPEN)"));
        }
        if (t.ctrla >> 1) & 3 == 3 {
            return Self::refuse(format!("TCB{i} CLKSEL 3 (reserved)"));
        }
        Ok(())
    }

    fn service(&self) -> Vec<u32> {
        let now = self.now();
        let mut guard = self.state.lock().unwrap();
        let s = &mut *guard;
        let mut irqs = std::mem::take(&mut s.raised);
        if Self::next_irq_cycle(s).is_some_and(|at| now >= at) {
            let rose = Self::settle(s, now);
            irqs.extend(self.irqs_for(s, &rose));
        }
        irqs
    }
}

impl Peripheral for MegaAvrTimers {
    fn read(&self, o: u64) -> SimResult<u8> {
        let now = self.now();
        let mut s = self.state.lock().unwrap();
        Ok(self.read_reg(&mut s, o, now))
    }

    fn peek(&self, o: u64) -> Option<u8> {
        if o >= WINDOW {
            return None;
        }
        let now = self.now();
        let mut copy = {
            let s = self.state.lock().unwrap();
            State {
                tca: s.tca,
                tcb: s.tcb,
                ..State::default()
            }
        };
        Some(self.read_reg(&mut copy, o, now))
    }

    fn write(&mut self, o: u64, v: u8) -> SimResult<()> {
        // Writes that settle a wrap raise its vector on the next tick/event;
        // the latched INTFLAGS keeps it visible meanwhile.
        let irqs = self.write_reg(o, v)?;
        if !irqs.is_empty() {
            let mut s = self.state.lock().unwrap();
            s.raised.extend(irqs);
            if self.scheduler_mode() {
                // Deliver at once through a one-cycle wake; it supersedes
                // the armed one, which `on_event` re-arms.
                s.generation = s.generation.wrapping_add(1);
                s.live_at = Some(self.now() + 1);
                let generation = s.generation;
                s.pending_events.push((1, generation));
            }
        }
        Ok(())
    }

    fn legacy_tick_active(&self) -> bool {
        let s = self.state.lock().unwrap();
        !self.scheduler_mode()
            && (!s.raised.is_empty()
                || Self::next_irq_cycle(&s).is_some()
                || self.next_output_cycle().is_some())
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        let irqs = self.service();
        if self.wired {
            // Buffered compare values the wrap applies are staged on the
            // lines already: only the edges need publishing.
            let now = self.now();
            self.outputs.advance_to(now);
        }
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        std::mem::take(&mut self.state.lock().unwrap().pending_events)
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let now = self.now();
        let mut guard = self.state.lock().unwrap();
        let s = &mut *guard;
        if s.generation != event_token {
            return crate::sched::EventResult::default();
        }
        s.live_at = None;
        let rose = Self::settle(s, now);
        let mut irqs = std::mem::take(&mut s.raised);
        irqs.extend(self.irqs_for(s, &rose));
        if self.wired {
            self.outputs.advance_to(now);
            Self::refresh_outputs(&mut self.outputs, self.wired, &s.tca, now);
        }
        let edge = self.next_output_cycle();
        self.rearm(s, edge);
        let reschedule_delay = s.pending_events.pop().map(|(delay, _)| {
            s.generation = event_token;
            delay
        });
        s.pending_events.clear();
        crate::sched::EventResult {
            explicit_irqs: irqs,
            reschedule_delay,
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_node_reset(&mut self) {
        *self.state.lock().unwrap() = State::default();
        let now = self.now();
        Self::refresh_outputs(&mut self.outputs, self.wired, &Tca::default(), now);
    }

    fn timer_output_family(&self) -> Option<&'static str> {
        Some("megaavr_tca")
    }

    fn timer_output_lines(
        &mut self,
        arm: bool,
    ) -> Option<std::sync::Arc<crate::peripherals::pad_lines::PadLines>> {
        let lines = self.outputs.lines_arc();
        if arm && !self.wired {
            self.wired = true;
            let now = self.now();
            let tca = self.state.lock().unwrap().tca;
            Self::refresh_outputs(&mut self.outputs, true, &tca, now);
        }
        Some(lines)
    }

    fn wire_lines(&self) -> Option<&crate::peripherals::pad_lines::PadLines> {
        self.outputs.wire_lines()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(cycle: u64) -> (MegaAvrTimers, CycleClock) {
        let clock = CycleClock::default();
        clock.publish(cycle);
        let mut t = MegaAvrTimers::new([12, 13, 25, 36]);
        t.attach_cycle_clock(clock.clone());
        (t, clock)
    }

    fn w16(t: &mut MegaAvrTimers, o: u64, v: u16) {
        t.write(o, v as u8).unwrap();
        t.write(o + 1, (v >> 8) as u8).unwrap();
    }

    /// ArduinoCore-megaavr `init()` on the Nano Every: TCA0 single-slope at
    /// CLK_PER/64, TCB3 in PWM8 on CLKTCA with CCMPL = 0xFF, CAPT enabled.
    fn arduino_millis(t: &mut MegaAvrTimers) {
        t.write(TCA_CTRLB, 0x03).unwrap();
        w16(t, TCA_PER, 0x00FF);
        t.write(TCA_CTRLA, 0x0B).unwrap(); // DIV64 | ENABLE
        t.write(0xB1, 0x07).unwrap(); // TCB3.CTRLB = PWM8
        w16(t, 0xBC, 0x00FF); // TCB3.CCMP
        t.write(0xB5, 0x01).unwrap(); // INTCTRL.CAPT
        t.write(0xB0, 0x04).unwrap(); // CLKSEL = CLKTCA
        t.write(0xB0, 0x05).unwrap(); // | ENABLE
    }

    #[test]
    fn tcb3_on_clktca_wraps_every_256_tca_ticks() {
        let (mut t, clock) = at(0);
        arduino_millis(&mut t);
        // 64 * 256 = 16384 CPU cycles per wrap = 1024 us at 16 MHz.
        clock.publish(64 * 100);
        assert_eq!(t.read(0xBA).unwrap(), 100, "CNTL counts TCA ticks");
        assert_eq!(t.read(0xB6).unwrap(), 0, "no wrap yet");
        clock.publish(16384 - 1);
        assert_eq!(t.tick().explicit_irqs, None);
        clock.publish(16384);
        assert_eq!(t.read(0xB6).unwrap(), 1, "CAPT visible before the wake");
        assert_eq!(t.tick().explicit_irqs, Some(vec![36]));
        assert_eq!(t.read(0xBA).unwrap(), 0);
        t.write(0xB6, 0x01).unwrap(); // the ISR clears CAPT
        assert_eq!(t.read(0xB6).unwrap(), 0);
        clock.publish(2 * 16384);
        assert_eq!(t.tick().explicit_irqs, Some(vec![36]));
    }

    #[test]
    fn clktca_stops_with_tca0() {
        let (mut t, clock) = at(0);
        arduino_millis(&mut t);
        clock.publish(64 * 10);
        t.write(TCA_CTRLA, 0x0A).unwrap(); // disable TCA0
        clock.publish(64 * 500);
        assert_eq!(t.read(0xBA).unwrap(), 10);
        assert_eq!(t.read(0xB7).unwrap(), 0, "STATUS.RUN clear");
    }

    #[test]
    fn tca0_counts_and_takes_its_buffered_period_at_the_wrap() {
        let (mut t, clock) = at(0);
        t.write(TCA_CTRLB, 0x03).unwrap();
        w16(&mut t, TCA_PER, 9);
        w16(&mut t, TCA_PERBUF, 4);
        t.write(TCA_CTRLA, 0x01).unwrap(); // DIV1 | ENABLE
        clock.publish(7);
        assert_eq!(t.read(TCA_CNT).unwrap(), 7);
        assert_eq!(t.read(TCA_INTFLAGS).unwrap(), 0);
        clock.publish(10);
        assert_eq!(t.read(TCA_CNT).unwrap(), 0, "wrapped at PER = 9");
        assert_eq!(t.read(TCA_INTFLAGS).unwrap(), TCA_OVF);
        clock.publish(15);
        assert_eq!(t.read(TCA_CNT).unwrap(), 0, "PERBUF = 4 took over");
    }

    #[test]
    fn sixteen_bit_registers_go_through_temp() {
        let (mut t, _clock) = at(0);
        t.write(0xBC, 0x34).unwrap();
        assert_eq!(
            t.read(TCB_BASE[3] + TCB_TEMP).unwrap(),
            0x34,
            "low byte waits in TEMP"
        );
        t.write(0xBD, 0x12).unwrap();
        assert_eq!(t.read(0xBC).unwrap(), 0x34);
        assert_eq!(t.read(0xBD).unwrap(), 0x12, "high byte comes from TEMP");
    }

    /// ArduinoCore-megaavr `setup_timers()` then `analogWrite(9, 128)`:
    /// single-slope, PER = 0xFF, CMP0BUF = 128, DIV64, then CTRLB.CMP0EN.
    #[test]
    fn analog_write_is_a_976_hz_half_duty_waveform_on_wo0() {
        let (mut t, clock) = at(0);
        let lines = t.timer_output_lines(true).unwrap();
        t.write(TCA_CTRLB, 0x03).unwrap();
        w16(&mut t, TCA_PER, 0x00FF);
        w16(&mut t, TCA_CMP0BUF, 0x0080);
        t.write(TCA_CTRLA, 0x0B).unwrap(); // DIV64 | ENABLE
        assert_eq!(lines.mode(0), LineDrive::Input, "CMP0EN clear: no drive");
        // The buffer took over at the first wrap (256 ticks of 64 cycles).
        clock.publish(16384 + 64);
        t.tick();
        t.write(TCA_CTRLB, 0x13).unwrap(); // | CMP0EN
        assert_eq!(lines.mode(0), LineDrive::PushPull);
        let mut high = 0;
        let mut edges = 0;
        let mut last = lines.level(0);
        for cycle in (2 * 16384..4 * 16384).step_by(64) {
            clock.publish(cycle);
            t.tick();
            let l = lines.level(0);
            high += u32::from(l);
            edges += u32::from(l != last);
            last = l;
        }
        assert_eq!(high, 2 * 128, "high for CNT < CMP0 = 128 of 256 counts");
        // High at the window start (BOTTOM), low at its end (CNT 255).
        assert_eq!(edges, 3, "fall, rise, fall over two periods");
        assert_eq!(lines.mode(1), LineDrive::Input, "WO1 not enabled");
    }

    #[test]
    fn a_new_compare_value_waits_for_the_wrap_and_a_stopped_timer_holds_cmpnov() {
        let (mut t, clock) = at(0);
        let lines = t.timer_output_lines(true).unwrap();
        t.write(TCA_CTRLB, 0x13).unwrap();
        w16(&mut t, TCA_PER, 99);
        w16(&mut t, TCA_CMP0, 50);
        t.write(TCA_CTRLA, 0x01).unwrap(); // DIV1
        clock.publish(10);
        t.tick();
        assert!(lines.level(0), "CNT 10 < 50");
        w16(&mut t, TCA_CMP0BUF, 5);
        clock.publish(20);
        t.tick();
        assert!(lines.level(0), "CMP0BUF waits for UPDATE");
        clock.publish(100 + 7);
        t.tick();
        assert!(!lines.level(0), "after the wrap CMP0 = 5: CNT 7 is past it");
        t.write(TCA_CTRLC, 0x01).unwrap(); // CMP0OV
        t.write(TCA_CTRLA, 0x00).unwrap(); // stop
        assert!(lines.level(0), "stopped: WO0 = CMP0OV");
    }

    #[test]
    fn unmodelled_modes_are_refused() {
        let (mut t, _clock) = at(0);
        t.write(TCA_CTRLB, 0x10).unwrap();
        assert!(t.write(TCA_CTRLA, 0x01).is_err(), "CMP0EN in NORMAL mode");
        let (mut t, _clock) = at(0);
        t.write(TCA_CTRLD, 0x01).unwrap();
        assert!(t.write(TCA_CTRLA, 0x01).is_err(), "split mode");
        let (mut t, _clock) = at(0);
        assert!(t.write(TCA_INTCTRL, 0x01).is_err(), "TCA0 OVF interrupt");
        let (mut t, _clock) = at(0);
        t.write(0x81, 0x02).unwrap(); // TCB0 CAPT mode
        assert!(t.write(0x80, 0x01).is_err());
    }
}
