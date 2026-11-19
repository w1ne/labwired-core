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
//! the CTRLFSET valid bits and are copied at the next wrap (UPDATE). Not
//! modelled, and refused when firmware asks for them: split mode, FRQ and the
//! dual-slope modes, TCA interrupts and the event input. CTRLESET commands
//! are refused too.
//!
//! TCB, periodic interrupt (CNTMODE INT) and 8-bit PWM (CNTMODE PWM8): the
//! counter runs 0..TOP and wraps, setting INTFLAGS.CAPT, where TOP is CCMP in
//! INT mode and CCMPL in PWM8 mode. CLKSEL DIV1, DIV2 and CLKTCA (which stops
//! while TCA0 is disabled). INTCTRL.CAPT raises the instance's vector once per
//! wrap. The other count modes and the event input are refused.
//!
//! Waveform outputs (what `analogWrite` drives). TCA0 single-slope PWM with
//! CTRLB.CMPnEN puts WOn on pin n of the port PORTMUX.TCAROUTEA selects: set
//! at BOTTOM, cleared at the compare match, so high while CNT < CMPn (§20.3.3.4.3);
//! CMPn = 0 holds it low and CMPn > PER high. A stopped TCA0 drives
//! CTRLC.CMPnOV. TCBn in PWM8 with CTRLB.CCMPEN drives its WO pad (default or
//! the PORTMUX.TCBROUTEA alternate): high while CNT < CCMPH (§21.3.3.1.8); a
//! stopped TCBn drives CTRLB.CCMPINIT. The override needs the pad's PORTx.DIR
//! bit, as on silicon. The levels go out on a [`PadLines`] wire with one line
//! per (route, output) pair, and only the routed line of an enabled output
//! drives; see [`OUTPUT_PADS`]. Limits:
//! - CMPnEN outside single-slope mode and CCMPEN outside PWM8 are refused, and
//!   so is either one on a bus with no port to carry it.
//! - Edges are published from the scheduler wake of the match or wrap, stamped
//!   with that cycle. A pulse shorter than one CPU instruction can merge.
//! - PORTMUX is another block: an enable or duty write reaches the pad one
//!   CPU cycle later, and a PORTMUX route write on its own at the next edge
//!   of a running waveform.
//! - After an output is disabled the pad reads its PORTx.OUT bit again at
//!   once, but a four-state logic trace keeps showing the pad as released
//!   until PORTx next moves it.
//!
//! 16-bit registers go through the instance's TEMP byte as on silicon: a low
//! byte write lands in TEMP and the high byte write commits both; a low byte
//! read latches the high byte into TEMP for the following high byte read.

use crate::peripherals::pad_lines::{LineDrive, PadLines};
use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::sync::{Arc, Mutex};

/// PORTMUX.TCAROUTEA / TCBROUTEA on the bus (data space 0x05E4 / 0x05E5).
const PORTMUX_TCAROUTEA: u64 = 0x0001_05E4;
const PORTMUX_TCBROUTEA: u64 = 0x0001_05E5;
/// PORTx on the bus: 0x0400 + 0x20 * port; OUT is offset 4.
const PORT_BASE: u64 = 0x0001_0400;
const PORT_OUT: u64 = 0x04;

/// Lines of the output wire: TCA0 WO0..WO2 on each PORTMUX.TCAROUTEA port
/// (line `3 * port + n`), then each TCB's WO on its default and its alternate
/// pad (line `18 + 2 * tcb + alt`).
pub const LINE_NAMES: [&str; 26] = [
    "TCA0_WO0", "TCA0_WO1", "TCA0_WO2", "TCA0_WO0", "TCA0_WO1", "TCA0_WO2", "TCA0_WO0", "TCA0_WO1",
    "TCA0_WO2", "TCA0_WO0", "TCA0_WO1", "TCA0_WO2", "TCA0_WO0", "TCA0_WO1", "TCA0_WO2", "TCA0_WO0",
    "TCA0_WO1", "TCA0_WO2", "TCB0_WO", "TCB0_WO", "TCB1_WO", "TCB1_WO", "TCB2_WO", "TCB2_WO",
    "TCB3_WO", "TCB3_WO",
];
const TCB_LINE0: usize = 18;

/// The pad of each line, as `(port index, pin)` with PORTA = 0 (DS40002173
/// §4.1 "I/O Multiplexing", 48-pin column): TCA0 WOn is pin n of the routed
/// port; TCB0 PA2 / PF4, TCB1 PA3 / PF5, TCB2 PC0 / PB4, TCB3 PB5 / PC1.
pub const OUTPUT_PADS: [(u8, u8); 26] = [
    (0, 0),
    (0, 1),
    (0, 2),
    (1, 0),
    (1, 1),
    (1, 2),
    (2, 0),
    (2, 1),
    (2, 2),
    (3, 0),
    (3, 1),
    (3, 2),
    (4, 0),
    (4, 1),
    (4, 2),
    (5, 0),
    (5, 1),
    (5, 2),
    (0, 2),
    (5, 4),
    (0, 3),
    (5, 5),
    (2, 0),
    (1, 4),
    (1, 5),
    (2, 1),
];

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
const TCB_CCMPINIT: u8 = 1 << 5;
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
    /// A register write may have moved a waveform output; publish it.
    outputs_dirty: bool,
}

/// TCA0 + TCB0..3.
#[derive(Debug)]
pub struct MegaAvrTimers {
    state: Mutex<State>,
    /// TCB0..3 vector numbers (12, 13, 25, 36 on the ATmega4809).
    tcb_irqs: [u32; 4],
    clock: Option<CycleClock>,
    /// The output wire, once the bus asked for it.
    lines: Option<Arc<PadLines>>,
    /// A port carries at least one line: outputs are generated.
    armed: bool,
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
            lines: None,
            armed: false,
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

    /// CPU cycle of the next CAPT wrap of every TCB whose interrupt is on,
    /// and of the next edge of every waveform output while a port carries it.
    fn next_wake_cycle(&self, s: &State) -> Option<u64> {
        let to_wrap = |start: u64, top: u64| {
            if start <= top {
                top + 1 - start
            } else {
                0x1_0000 - start
            }
        };
        let mut next: Option<u64> = None;
        let mut take = |cycle: u64| next = Some(next.map_or(cycle, |n: u64| n.min(cycle)));
        for t in &s.tcb {
            let Some(div) = Self::tcb_div(s, t) else {
                continue;
            };
            let (start, top) = (u64::from(t.anchor_count), t.top());
            let output = self.armed && t.ctrlb & TCB_CCMPEN != 0;
            if t.intctrl & TCB_CAPT != 0 || output {
                take((t.anchor_tick + to_wrap(start, top)) * div);
            }
            let duty = u64::from(t.ccmp >> 8);
            if output && start < duty && duty <= top {
                take((t.anchor_tick + duty - start) * div);
            }
        }
        let tca = &s.tca;
        if self.armed && tca.running() && tca.ctrlb & TCA_CMPEN != 0 {
            let div = tca_div(tca.ctrla);
            let (start, top) = (u64::from(tca.anchor_count), tca.top());
            take((tca.anchor_tick + to_wrap(start, top)) * div);
            for n in 0..3 {
                let cmp = u64::from(tca.cmp[n]);
                if tca.ctrlb & (0x10 << n) != 0 && start < cmp && cmp <= top {
                    take((tca.anchor_tick + cmp - start) * div);
                }
            }
        }
        next
    }

    /// The cycle to stamp edges published at `now` with: that of the wrap or
    /// match that is due, which an instruction boundary can overshoot.
    fn due_cycle(&self, s: &State, now: u64) -> u64 {
        self.next_wake_cycle(s).map_or(now, |at| at.min(now))
    }

    /// Whether any waveform output is enabled onto a pad.
    fn outputs_enabled(s: &State) -> bool {
        s.tca.ctrlb & TCA_CMPEN != 0 || s.tcb.iter().any(|t| t.ctrlb & TCB_CCMPEN != 0)
    }

    /// Put the level of every output on its routed line and release the
    /// others. `s` is settled at `now`; `at` stamps the edges.
    fn publish(&self, s: &mut State, bus: &dyn Bus, at: u64) {
        s.outputs_dirty = false;
        let Some(lines) = self.lines.as_ref().filter(|_| self.armed) else {
            return;
        };
        let route_a = usize::from(bus.read_u8(PORTMUX_TCAROUTEA).unwrap_or(0) & 0x07);
        let route_b = bus.read_u8(PORTMUX_TCBROUTEA).unwrap_or(0);
        let mut drive = [None; 26];
        let tca = &s.tca;
        for n in 0..3 {
            if tca.ctrlb & (0x10 << n) != 0 && route_a < 6 {
                drive[3 * route_a + n] = Some(if tca.running() {
                    tca.anchor_count < tca.cmp[n]
                } else {
                    tca.ctrlc & (1 << n) != 0
                });
            }
        }
        for (i, t) in s.tcb.iter().enumerate() {
            if t.ctrlb & TCB_CCMPEN != 0 {
                let alt = usize::from(route_b >> i & 1);
                drive[TCB_LINE0 + 2 * i + alt] = Some(if t.running() {
                    t.anchor_count < t.ccmp >> 8
                } else {
                    t.ctrlb & TCB_CCMPINIT != 0
                });
            }
        }
        lines.set_stamp(Some(at));
        for (line, level) in drive.into_iter().enumerate() {
            match level {
                Some(level) => lines.drive_line(line, LineDrive::PushPull, level),
                None if lines.drives(line) => {
                    // The pad goes back to its PORTx.OUT bit.
                    let (port, pin) = OUTPUT_PADS[line];
                    let out = bus
                        .read_u8(PORT_BASE + 0x20 * u64::from(port) + PORT_OUT)
                        .unwrap_or(0);
                    lines.drive_line(line, LineDrive::Input, out & (1 << pin) != 0);
                }
                None => {}
            }
        }
        lines.set_stamp(None);
    }

    fn rearm(&self, s: &mut State) {
        if !self.scheduler_mode() {
            return;
        }
        let Some(fire) = self.next_wake_cycle(s) else {
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
            self.tca_write(s, o, v, now)?;
        } else if let Some(i) = TCB_BASE.iter().position(|&b| (b..b + 0x10).contains(&o)) {
            self.tcb_write(s, i, o - TCB_BASE[i], v, now)?;
        }
        // A configuration change re-anchors every counter at `now`.
        Self::settle(s, now);
        if self.armed && (Self::outputs_enabled(s) || self.lines.as_ref().is_some_and(any_drives)) {
            s.outputs_dirty = true;
        }
        self.rearm(s);
        Ok(irqs)
    }

    fn tca_write(&self, s: &mut State, o: u64, v: u8, now: u64) -> SimResult<()> {
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
                self.tca_check(&tca)?;
            }
            TCA_CTRLB => {
                tca.ctrlb = v & 0x7F;
                self.tca_check(tca)?;
            }
            TCA_CTRLC => tca.ctrlc = v & 0x07,
            TCA_CTRLD => {
                tca.ctrld = v & 0x01;
                self.tca_check(tca)?;
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

    fn tca_check(&self, tca: &Tca) -> SimResult<()> {
        if !tca.running() {
            return Ok(());
        }
        if tca.ctrld & 0x01 != 0 {
            return Self::refuse("TCA0 split mode".into());
        }
        if !matches!(tca.ctrlb & TCA_WGMODE, 0 | 3) {
            return Self::refuse(format!("TCA0 WGMODE {}", tca.ctrlb & TCA_WGMODE));
        }
        if tca.ctrlb & TCA_CMPEN != 0 {
            if tca.ctrlb & TCA_WGMODE != 3 {
                return Self::refuse("TCA0 compare outputs (CTRLB.CMPnEN) in NORMAL mode".into());
            }
            if !self.armed {
                return Self::refuse(
                    "TCA0 compare outputs (CTRLB.CMPnEN) with no port to carry them".into(),
                );
            }
        }
        Ok(())
    }

    fn tcb_write(&self, s: &mut State, i: usize, r: u64, v: u8, now: u64) -> SimResult<()> {
        let tca = s.tca;
        let t = &mut s.tcb[i];
        match r {
            TCB_CTRLA => {
                t.ctrla = v & 0x57;
                t.anchor_tick = now / Self::tcb_div_static(&tca, t).unwrap_or(1);
                self.tcb_check(i, t)?;
            }
            TCB_CTRLB => {
                t.ctrlb = v & 0x77;
                self.tcb_check(i, t)?;
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

    fn tcb_check(&self, i: usize, t: &Tcb) -> SimResult<()> {
        if !t.running() {
            return Ok(());
        }
        if !matches!(t.ctrlb & TCB_CNTMODE, TCB_MODE_INT | TCB_MODE_PWM8) {
            return Self::refuse(format!("TCB{i} CNTMODE {}", t.ctrlb & TCB_CNTMODE));
        }
        if t.ctrlb & TCB_CCMPEN != 0 {
            if t.ctrlb & TCB_CNTMODE != TCB_MODE_PWM8 {
                return Self::refuse(format!("TCB{i} output (CTRLB.CCMPEN) outside PWM8 mode"));
            }
            if !self.armed {
                return Self::refuse(format!(
                    "TCB{i} waveform output (CTRLB.CCMPEN) with no port to carry it"
                ));
            }
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
        if self.next_wake_cycle(s).is_some_and(|at| now >= at) {
            let rose = Self::settle(s, now);
            irqs.extend(self.irqs_for(s, &rose));
        }
        irqs
    }
}

fn any_drives(lines: &Arc<PadLines>) -> bool {
    (0..LINE_NAMES.len()).any(|line| lines.drives(line))
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
        let mut s = self.state.lock().unwrap();
        if !irqs.is_empty() || s.outputs_dirty {
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
            && (!s.raised.is_empty() || s.outputs_dirty || self.next_wake_cycle(&s).is_some())
    }

    fn needs_bus_tick(&self) -> bool {
        if self.scheduler_mode() || !self.armed {
            return false;
        }
        let s = self.state.lock().unwrap();
        s.outputs_dirty || Self::outputs_enabled(&s)
    }

    /// Without the scheduler the outputs follow the counters at every tick.
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if self.scheduler_mode() {
            return;
        }
        let now = self.now();
        let mut guard = self.state.lock().unwrap();
        let s = &mut *guard;
        let due = self.due_cycle(s, now);
        let rose = Self::settle(s, now);
        let irqs = self.irqs_for(s, &rose);
        s.raised.extend(irqs);
        self.publish(s, bus, due);
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
        bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        let now = self.now();
        let mut guard = self.state.lock().unwrap();
        let s = &mut *guard;
        if s.generation != event_token {
            return crate::sched::EventResult::default();
        }
        s.live_at = None;
        let due = self.due_cycle(s, now);
        let rose = Self::settle(s, now);
        let mut irqs = std::mem::take(&mut s.raised);
        irqs.extend(self.irqs_for(s, &rose));
        self.publish(s, bus, due);
        self.rearm(s);
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
        if let Some(lines) = &self.lines {
            for line in 0..LINE_NAMES.len() {
                lines.drive_line(line, LineDrive::Input, false);
            }
        }
    }

    fn timer_output_lines(&mut self, arm: bool) -> Option<Arc<PadLines>> {
        let lines = self.lines.get_or_insert_with(|| {
            let lines = PadLines::new(&LINE_NAMES, &[false; 26]);
            for line in 0..LINE_NAMES.len() {
                lines.set_mode(line, LineDrive::Input);
            }
            Arc::new(lines)
        });
        self.armed |= arm;
        Some(lines.clone())
    }

    fn timer_output_family(&self) -> Option<&'static str> {
        Some("megaavr_tc")
    }

    fn timer_output_pads(&self) -> Vec<(usize, String, u8)> {
        OUTPUT_PADS
            .iter()
            .enumerate()
            .map(|(line, &(port, pin))| {
                let letter = char::from(b'a' + port);
                (line, format!("port{letter}"), pin)
            })
            .collect()
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

    /// The IRQs the timers raise now. Goes through `service()` so the test is
    /// the same with and without the event scheduler (`tick()` is a no-op in
    /// scheduler mode, where `on_event` delivers them).
    fn poll(t: &mut MegaAvrTimers) -> Option<Vec<u32>> {
        let irqs = t.service();
        (!irqs.is_empty()).then_some(irqs)
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
        assert_eq!(poll(&mut t), None);
        clock.publish(16384);
        assert_eq!(t.read(0xB6).unwrap(), 1, "CAPT visible before the wake");
        assert_eq!(poll(&mut t), Some(vec![36]));
        assert_eq!(t.read(0xBA).unwrap(), 0);
        t.write(0xB6, 0x01).unwrap(); // the ISR clears CAPT
        assert_eq!(t.read(0xB6).unwrap(), 0);
        clock.publish(2 * 16384);
        assert_eq!(poll(&mut t), Some(vec![36]));
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

    /// PORTMUX and the PORTx.OUT latches, as the timers read them.
    struct Mux {
        tca: u8,
        tcb: u8,
        out: [u8; 6],
        config: crate::SimulationConfig,
    }

    impl Mux {
        /// ArduinoCore-megaavr on the Nano Every: TCA0 on PORTB, TCB0 and
        /// TCB1 on their alternate pads (PF4, PF5).
        fn nano_every() -> Self {
            Self {
                tca: 1,
                tcb: 0x03,
                out: [0; 6],
                config: crate::SimulationConfig::default(),
            }
        }
    }

    impl Bus for Mux {
        fn read_u8(&self, addr: u64) -> SimResult<u8> {
            Ok(match addr {
                PORTMUX_TCAROUTEA => self.tca,
                PORTMUX_TCBROUTEA => self.tcb,
                a if a >= PORT_BASE && (a - PORT_BASE) % 0x20 == PORT_OUT => {
                    self.out[((a - PORT_BASE) / 0x20) as usize]
                }
                _ => 0,
            })
        }
        fn write_u8(&mut self, _addr: u64, _value: u8) -> SimResult<()> {
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[crate::DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &crate::SimulationConfig {
            &self.config
        }
    }

    /// Settle at `cycle` and publish, as the scheduler wake and the legacy
    /// bus tick both do.
    fn publish_at(t: &mut MegaAvrTimers, clock: &CycleClock, bus: &Mux, cycle: u64) {
        clock.publish(cycle);
        let mut guard = t.state.lock().unwrap();
        let s = &mut *guard;
        MegaAvrTimers::settle(s, cycle);
        t.publish(s, bus, cycle);
    }

    fn next_wake(t: &MegaAvrTimers) -> Option<u64> {
        t.next_wake_cycle(&t.state.lock().unwrap())
    }

    /// The lines that drive a pad, with their levels.
    fn driven(lines: &PadLines) -> Vec<(usize, bool)> {
        (0..LINE_NAMES.len())
            .filter(|&line| lines.drives(line))
            .map(|line| (line, lines.level(line)))
            .collect()
    }

    /// `analogWrite(9, 128)` on the Nano Every: D9 = PB0 = TCA0 WO0.
    #[test]
    fn tca0_single_slope_output_is_high_below_cmp_on_the_routed_port() {
        let (mut t, clock) = at(0);
        let lines = t.timer_output_lines(true).unwrap();
        let bus = Mux::nano_every();
        arduino_millis(&mut t);
        w16(&mut t, TCA_CMP0BUF, 0x80);
        t.write(TCA_CTRLB, 0x13).unwrap(); // SINGLESLOPE | CMP0EN
        publish_at(&mut t, &clock, &bus, 1);
        // CMP0 is still 0 until the buffer is taken at the wrap.
        assert_eq!(driven(&lines), vec![(3, false)], "PB0 only, low");
        assert_eq!(next_wake(&t), Some(16384), "the wrap");
        publish_at(&mut t, &clock, &bus, 16384);
        assert_eq!(driven(&lines), vec![(3, true)], "set at BOTTOM");
        assert_eq!(next_wake(&t), Some(16384 + 64 * 0x80), "the compare match");
        publish_at(&mut t, &clock, &bus, 16384 + 64 * 0x80 - 1);
        assert!(lines.level(3), "CNT = 0x7F");
        publish_at(&mut t, &clock, &bus, 16384 + 64 * 0x80);
        assert_eq!(driven(&lines), vec![(3, false)], "cleared at CNT = CMP0");
        assert_eq!(next_wake(&t), Some(2 * 16384));
    }

    #[test]
    fn tca0_output_follows_the_portmux_route_and_cmp_limits() {
        let (mut t, clock) = at(0);
        let lines = t.timer_output_lines(true).unwrap();
        let mut bus = Mux::nano_every();
        t.write(TCA_CTRLB, 0x73).unwrap(); // all three outputs
        w16(&mut t, TCA_PER, 99);
        w16(&mut t, TCA_CMP0, 0); // never high
        w16(&mut t, TCA_CMP0 + 2, 100); // above PER: always high
        w16(&mut t, TCA_CMP0 + 4, 50);
        t.write(TCA_CTRLC, 0x01).unwrap();
        publish_at(&mut t, &clock, &bus, 0);
        assert_eq!(
            driven(&lines),
            vec![(3, true), (4, false), (5, false)],
            "stopped: CTRLC.CMPnOV"
        );
        t.write(TCA_CTRLA, 0x01).unwrap(); // DIV1 | ENABLE
        bus.tca = 5; // PORTF
        publish_at(&mut t, &clock, &bus, 10);
        assert_eq!(driven(&lines), vec![(15, false), (16, true), (17, true)]);
        // Only WO2 has an edge inside the period.
        assert_eq!(next_wake(&t), Some(50));
        publish_at(&mut t, &clock, &bus, 50);
        assert_eq!(driven(&lines), vec![(15, false), (16, true), (17, false)]);
        assert_eq!(next_wake(&t), Some(100));
    }

    /// `analogWrite(3, 64)` on the Nano Every: D3 = PF5 = TCB1 WO (alternate).
    #[test]
    fn tcb_pwm8_output_is_high_below_ccmph_on_its_routed_pad() {
        let (mut t, clock) = at(0);
        let lines = t.timer_output_lines(true).unwrap();
        let mut bus = Mux::nano_every();
        arduino_millis(&mut t);
        t.write(0x91, 0x07).unwrap(); // TCB1.CTRLB = PWM8
        t.write(0x9C, 0xFF).unwrap(); // CCMPL through TEMP
        t.write(0x9D, 0x40).unwrap(); // CCMPH = 64
        t.write(0x90, 0x05).unwrap(); // CLKTCA | ENABLE
        t.write(0x91, 0x17).unwrap(); // | CCMPEN
        publish_at(&mut t, &clock, &bus, 64);
        assert_eq!(driven(&lines), vec![(21, true)], "PF5, CNT = 1");
        assert_eq!(next_wake(&t), Some(64 * 0x40));
        publish_at(&mut t, &clock, &bus, 64 * 0x40);
        assert_eq!(driven(&lines), vec![(21, false)]);
        // TCB1 is level with TCB3, whose CAPT wrap is the next wake too.
        assert_eq!(next_wake(&t), Some(16384));
        bus.tcb = 0x01; // TCB1 back on its default pad PA3
        publish_at(&mut t, &clock, &bus, 16384);
        assert_eq!(driven(&lines), vec![(20, true)]);
    }

    #[test]
    fn a_disabled_output_hands_the_pad_back_at_its_port_latch_level() {
        let (mut t, clock) = at(0);
        let lines = t.timer_output_lines(true).unwrap();
        let mut bus = Mux::nano_every();
        arduino_millis(&mut t);
        t.write(TCA_CTRLB, 0x13).unwrap();
        publish_at(&mut t, &clock, &bus, 1);
        assert!(lines.drives(3));
        bus.out[1] = 0x01; // PORTB.OUT bit 0
        t.write(TCA_CTRLB, 0x03).unwrap(); // turnOffPWM
        assert!(t.state.lock().unwrap().outputs_dirty);
        publish_at(&mut t, &clock, &bus, 2);
        assert_eq!(driven(&lines), vec![]);
        assert!(lines.level(3), "the released line reports PORTB.OUT");
        assert_eq!(next_wake(&t), Some(16384), "only the millis() wrap is left");
    }

    #[test]
    fn outputs_need_a_port_and_a_pwm_mode() {
        let (mut t, _clock) = at(0);
        t.timer_output_lines(true).unwrap();
        t.write(TCA_CTRLB, 0x10).unwrap(); // NORMAL | CMP0EN
        assert!(t.write(TCA_CTRLA, 0x01).is_err(), "CMP0EN in NORMAL mode");
        let (mut t, _clock) = at(0);
        t.timer_output_lines(true).unwrap();
        t.write(0x81, 0x10).unwrap(); // TCB0 INT mode | CCMPEN
        assert!(t.write(0x80, 0x01).is_err(), "CCMPEN outside PWM8");
        let (mut t, _clock) = at(0);
        t.timer_output_lines(false).unwrap(); // the wire exists, no pad took it
        t.write(0x81, 0x17).unwrap();
        assert!(t.write(0x80, 0x01).is_err(), "CCMPEN with no port");
    }

    #[test]
    fn unmodelled_modes_are_refused() {
        let (mut t, _clock) = at(0);
        t.write(TCA_CTRLB, 0x13).unwrap();
        assert!(t.write(TCA_CTRLA, 0x01).is_err(), "CMP0EN onto a pad");
        let (mut t, _clock) = at(0);
        assert!(t.write(TCA_INTCTRL, 0x01).is_err(), "TCA0 OVF interrupt");
        let (mut t, _clock) = at(0);
        t.write(0x81, 0x02).unwrap(); // TCB0 CAPT mode
        assert!(t.write(0x80, 0x01).is_err());
    }
}
