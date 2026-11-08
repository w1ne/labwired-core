// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! EFM32 Series 0 TIMER (EFM32GG reference manual "TIMER – Timer/Counter";
//! register map from Gecko SDK `efm32gg_timer.h`, TIMER_TypeDef): the 16-bit
//! counter in UP, DOWN and UP/DOWN modes, TOP with its TOPB buffer, one-shot
//! (CTRL.OSMEN), the 2^n prescaler, three compare channels with their CCVB
//! buffers, and the OF/UF/CCx flags behind IEN as one level IRQ.
//!
//! This is the block Renode maps as `Timers.Efm32Timer` at
//! 0x40010000/0x400/0x800/0xC00 on every Series-0 part (TIMER0..TIMER3).
//! Renode's model only counts and raises one IRQ; this one keeps the
//! registers and flags firmware actually reads (IF, STATUS, CCx).
//!
//! Clocking: the counter advances by CPU cycles / 2^CTRL.PRESC, i.e. it
//! assumes HFPERCLK = HFCLK (CMU_HFPERCLKDIV = 1, the reset value) and that
//! the CPU runs from HFCLK undivided. The CMU is not modelled, so the
//! HFPERCLKEN0 TIMERn gate is NOT honoured: the timer counts whether or not
//! firmware enabled its clock.
//!
//! Modes: UP wraps TOP → 0 raising OF; DOWN wraps 0 → TOP raising UF;
//! UP/DOWN turns at TOP (OF) and at 0 (UF). TOPB and CCVB load at the
//! overflow/underflow that follows the write (STATUS.TOPBV/CCVBVn track
//! it). A compare channel in OUTPUTCOMPARE or PWM mode raises CCn when CNT
//! becomes CCV; no pad is driven (ROUTE is stored only).
//!
//! Not modelled, refused on write: quadrature decoding (MODE = QDEC), the
//! CC1/TIMEROUF clock sources (CLKSEL), SYNC start/stop across timers, the
//! RISEA/FALLA input actions, input capture (CCn MODE = INPUTCAPTURE), PRS
//! inputs, and the dead-time insertion unit (DTCTRL..DTLOCK, 0x70..0x84).
//!
//! Series 1 (`new_series1`; Gecko SDK v4.4.0 `efm32pg1b_timer.h`, the same
//! TIMER_TypeDef on PG1/PG12/JG1/JG12) keeps the counter and the CTRL/CMD/
//! STATUS words but moves the rest: IF 0x0C, IFS 0x10, IFC 0x14, IEN 0x18,
//! LOCK 0x2C, ROUTEPEN 0x30, ROUTELOC0 0x34, ROUTELOC2 0x3C, four compare
//! channels CC0..CC3 at 0x60 + n * 0x10, and the DTI words from 0xA0. It
//! also raises IF.DIRCHG when UP/DOWN turns. LOCK accepts the unlock key
//! (0xCE80) and reads 0; locking it is refused. A read of IFC returns 0
//! (MSC_CTRL.IFCREADCLEAR, which would make it read-to-clear, is not
//! modelled).
use crate::{Peripheral, PeripheralTickResult, SimResult, SimulationError};
use serde::Serialize;
use std::any::Any;

// Series 0 offsets; `Reg::decode` maps both layouts onto these registers.
const CTRL: u64 = 0x00;
const CMD: u64 = 0x04;
const STATUS: u64 = 0x08;
const IEN: u64 = 0x0C;
const IF: u64 = 0x10;
const IFS: u64 = 0x14;
const IFC: u64 = 0x18;
const TOP: u64 = 0x1C;
const TOPB: u64 = 0x20;
const CNT: u64 = 0x24;
const ROUTE: u64 = 0x28;
/// CC0 at 0x30, CC1 at 0x40, CC2 at 0x50: CTRL, CCV, CCVP, CCVB.
const CC_BASE: u64 = 0x30;
/// Series 1: CC0..CC3 at 0x60..0x9F.
const S1_CC_BASE: u64 = 0x60;

/// _TIMER_LOCK_TIMERLOCKKEY_UNLOCK (efm32pg1b_timer.h).
const S1_LOCK_UNLOCK: u32 = 0xCE80;

/// A register, independent of where the layout puts it.
#[derive(Clone, Copy)]
enum Reg {
    Ctrl,
    Cmd,
    Status,
    Ien,
    If,
    Ifs,
    Ifc,
    Top,
    Topb,
    Cnt,
    /// Series 0 ROUTE; Series 1 ROUTEPEN, ROUTELOC0, ROUTELOC2.
    Route(usize),
    /// Series 1 LOCK.
    Lock,
    /// Channel, word (0 CTRL, 1 CCV, 2 CCVP, 3 CCVB).
    Cc(usize, u64),
    Dti,
    Reserved,
}

impl Reg {
    fn decode(series1: bool, offset: u64) -> Self {
        let common = match offset {
            CTRL => Some(Reg::Ctrl),
            CMD => Some(Reg::Cmd),
            STATUS => Some(Reg::Status),
            TOP => Some(Reg::Top),
            TOPB => Some(Reg::Topb),
            CNT => Some(Reg::Cnt),
            _ => None,
        };
        if let Some(r) = common {
            return r;
        }
        if series1 {
            match offset {
                0x0C => Reg::If,
                0x10 => Reg::Ifs,
                0x14 => Reg::Ifc,
                0x18 => Reg::Ien,
                0x2C => Reg::Lock,
                0x30 => Reg::Route(0),
                0x34 => Reg::Route(1),
                0x3C => Reg::Route(2),
                0x60..=0x9F => Reg::Cc(((offset - S1_CC_BASE) / 0x10) as usize, (offset & 0xF) / 4),
                0xA0..=0xBB => Reg::Dti,
                _ => Reg::Reserved,
            }
        } else {
            match offset {
                IEN => Reg::Ien,
                IF => Reg::If,
                IFS => Reg::Ifs,
                IFC => Reg::Ifc,
                ROUTE => Reg::Route(0),
                0x30..=0x5F => Reg::Cc(((offset - CC_BASE) / 0x10) as usize, (offset & 0xF) / 4),
                0x70..=0x87 => Reg::Dti,
                _ => Reg::Reserved,
            }
        }
    }
}

const CMD_START: u32 = 1;
const CMD_STOP: u32 = 1 << 1;

const IF_OF: u32 = 1;
const IF_UF: u32 = 1 << 1;
/// Series 1 only: UP/DOWN changed direction.
const IF_DIRCHG: u32 = 1 << 2;
const IF_CC0: u32 = 1 << 4;
/// OF, UF, CC0..CC2, ICBOF0..ICBOF2 (the last never raised: no capture).
const IF_MASK: u32 = 0x0773;
/// Series 1 _TIMER_IF_MASK: adds DIRCHG, CC3 and ICBOF3.
const S1_IF_MASK: u32 = 0x0FF7;

const CTRL_OSMEN: u32 = 1 << 4;
const CTRL_SYNC: u32 = 1 << 3;
/// The CTRL bits this slice stores (MODE, SYNC, OSMEN, QDM, DEBUGRUN,
/// DMACLRACT, RISEA, FALLA, X2CNT, CLKSEL, PRESC, ATI, RSSCOIST).
const CTRL_MASK: u32 = 0x3F03_2FFF;

const MODE_UP: u32 = 0;
const MODE_DOWN: u32 = 1;
const MODE_UPDOWN: u32 = 2;

#[derive(Debug, Default, Clone, Copy, Serialize)]
struct Cc {
    ctrl: u32,
    ccv: u16,
    ccvp: u16,
    ccvb: u16,
    /// STATUS.CCVBVn: CCVB written and not yet loaded.
    ccvbv: bool,
}

impl Cc {
    /// OUTPUTCOMPARE (2) and PWM (3) compare CNT against CCV.
    fn compares(&self) -> bool {
        self.ctrl & 3 >= 2
    }
}

#[derive(Debug, Serialize)]
pub struct Efm32Timer {
    irq: u32,
    /// Series-1 register layout, four compare channels and IF.DIRCHG.
    series1: bool,
    ctrl: u32,
    running: bool,
    /// STATUS.DIR: counting down.
    down: bool,
    ien: u32,
    flags: u32,
    top: u16,
    topb: u16,
    topbv: bool,
    cnt: u16,
    route: [u32; 3],
    /// Three channels on Series 0; the fourth exists on Series 1 only.
    cc: [Cc; 4],
    /// CPU cycles not yet worth a whole counter clock. Not a register.
    #[serde(skip)]
    carry: u64,
    refusal: Option<String>,
}

impl Efm32Timer {
    /// A Series-0 TIMER (three compare channels).
    pub fn new(irq: u32) -> Self {
        Self::with_layout(irq, false)
    }

    /// A Series-1 TIMER (EFM32PG1/PG12/JG1/JG12 layout, four channels).
    pub fn new_series1(irq: u32) -> Self {
        Self::with_layout(irq, true)
    }

    fn with_layout(irq: u32, series1: bool) -> Self {
        Self {
            irq,
            series1,
            ctrl: 0,
            running: false,
            down: false,
            ien: 0,
            flags: 0,
            // _TIMER_TOP_RESETVALUE / _TIMER_TOPB_RESETVALUE
            top: 0xFFFF,
            topb: 0,
            topbv: false,
            cnt: 0,
            route: [0; 3],
            cc: [Cc::default(); 4],
            carry: 0,
            refusal: None,
        }
    }

    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "efm32 TIMER slice: {what}"
        )))
    }

    pub fn counter(&self) -> u16 {
        self.cnt
    }

    fn mode(&self) -> u32 {
        self.ctrl & 3
    }

    fn if_mask(&self) -> u32 {
        if self.series1 {
            S1_IF_MASK
        } else {
            IF_MASK
        }
    }

    /// The compare channels this layout has.
    fn channels(&self) -> &[Cc] {
        &self.cc[..if self.series1 { 4 } else { 3 }]
    }

    fn channels_mut(&mut self) -> &mut [Cc] {
        let n = if self.series1 { 4 } else { 3 };
        &mut self.cc[..n]
    }

    fn line(&self) -> bool {
        self.flags & self.ien != 0
    }

    fn status(&self) -> u32 {
        let mut s = u32::from(self.running) | (u32::from(self.down) << 1);
        s |= u32::from(self.topbv) << 2;
        for (n, c) in self.channels().iter().enumerate() {
            s |= u32::from(c.ccvbv) << (8 + n);
        }
        s
    }

    fn read_reg(&self, offset: u64) -> SimResult<u32> {
        Ok(match Reg::decode(self.series1, offset) {
            Reg::Ctrl => self.ctrl,
            Reg::Cmd | Reg::Ifs | Reg::Ifc | Reg::Lock | Reg::Reserved => 0,
            Reg::Status => self.status(),
            Reg::Ien => self.ien,
            Reg::If => self.flags,
            Reg::Top => self.top.into(),
            Reg::Topb => self.topb.into(),
            Reg::Cnt => self.cnt.into(),
            Reg::Route(n) => self.route[n],
            Reg::Cc(n, word) => {
                let c = &self.cc[n];
                match word {
                    0 => c.ctrl,
                    1 => c.ccv.into(),
                    2 => c.ccvp.into(),
                    _ => c.ccvb.into(),
                }
            }
            Reg::Dti => return Self::refuse("dead-time insertion (DTCTRL..DTLOCK)"),
        })
    }

    fn write_reg(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match Reg::decode(self.series1, offset) {
            Reg::Ctrl => {
                if value & 3 == 3 {
                    return Self::refuse("quadrature decoder mode (CTRL.MODE = QDEC)");
                }
                if value & CTRL_SYNC != 0 {
                    return Self::refuse("CTRL.SYNC (start/stop with other timers)");
                }
                if (value >> 8) & 0xF != 0 {
                    return Self::refuse("CTRL.RISEA/FALLA input actions");
                }
                if (value >> 16) & 3 != 0 {
                    return Self::refuse("CTRL.CLKSEL other than PRESCHFPERCLK");
                }
                self.ctrl = value & CTRL_MASK;
                if self.mode() != MODE_UPDOWN {
                    self.down = self.mode() == MODE_DOWN;
                }
            }
            Reg::Cmd => {
                if value & CMD_START != 0 {
                    self.running = true;
                }
                if value & CMD_STOP != 0 {
                    self.running = false;
                }
            }
            Reg::Ien => self.ien = value & self.if_mask(),
            Reg::Ifs => self.flags |= value & self.if_mask(),
            Reg::Ifc => self.flags &= !value,
            Reg::Top => self.top = value as u16,
            Reg::Topb => {
                self.topb = value as u16;
                self.topbv = true;
            }
            Reg::Cnt => self.cnt = value as u16,
            Reg::Route(n) => self.route[n] = value,
            Reg::Lock => {
                if value & 0xFFFF != S1_LOCK_UNLOCK {
                    return Self::refuse("LOCK (configuration lock)");
                }
            }
            Reg::Cc(n, word) => {
                let c = &mut self.cc[n];
                match word {
                    0 => {
                        if value & 3 == 1 {
                            return Self::refuse("CCn input capture");
                        }
                        c.ctrl = value;
                    }
                    1 => c.ccv = value as u16,
                    3 => {
                        c.ccvb = value as u16;
                        c.ccvbv = true;
                    }
                    // CCVP is read-only.
                    _ => {}
                }
            }
            Reg::Dti => return Self::refuse("dead-time insertion (DTCTRL..DTLOCK)"),
            Reg::Status | Reg::If | Reg::Reserved => {}
        }
        Ok(())
    }

    fn checked(&mut self, result: SimResult<()>) -> SimResult<()> {
        if let Err(e) = &result {
            self.refusal.get_or_insert_with(|| e.to_string());
        }
        result
    }

    /// An overflow or underflow: raise `flag`, load the buffers, and stop a
    /// one-shot timer.
    fn wrap_event(&mut self, flag: u32) {
        self.flags |= flag;
        if self.topbv {
            self.top = self.topb;
            self.topbv = false;
        }
        for c in self.channels_mut() {
            c.ccvp = c.ccv;
            if c.ccvbv {
                c.ccv = c.ccvb;
                c.ccvbv = false;
            }
        }
        if self.ctrl & CTRL_OSMEN != 0 {
            self.running = false;
        }
    }

    /// UP/DOWN changes direction; Series 1 flags it as IF.DIRCHG.
    fn turn(&mut self, down: bool) {
        self.down = down;
        if self.series1 {
            self.flags |= IF_DIRCHG;
        }
    }

    /// CNT has just become its current value: raise the matching compares.
    fn compare(&mut self) {
        let mut hit = 0;
        for (n, c) in self.channels().iter().enumerate() {
            if c.compares() && c.ccv == self.cnt {
                hit |= IF_CC0 << n;
            }
        }
        self.flags |= hit;
    }

    /// Distance from CNT to the nearest compare value strictly ahead in the
    /// counting direction, capped at `limit`.
    fn next_compare(&self, limit: u16) -> u16 {
        let mut dist = limit;
        for c in self.channels().iter().filter(|c| c.compares()) {
            let d = if self.down {
                self.cnt.checked_sub(c.ccv)
            } else {
                c.ccv.checked_sub(self.cnt)
            };
            if let Some(d) = d.filter(|&d| d > 0) {
                dist = dist.min(d);
            }
        }
        dist
    }

    /// Move the counter by `ticks` counter clocks, jumping straight to the
    /// next wrap or compare instead of stepping one clock at a time.
    fn advance(&mut self, mut ticks: u64) {
        while ticks > 0 && self.running {
            // A CNT above TOP counts on to 0xFFFF before it wraps (up).
            let top = if !self.down && self.cnt > self.top {
                0xFFFF
            } else {
                self.top
            };
            match (self.mode(), self.down) {
                (MODE_UP, _) if self.cnt == top => {
                    self.cnt = 0;
                    ticks -= 1;
                    self.wrap_event(IF_OF);
                    self.compare();
                    continue;
                }
                (MODE_DOWN, _) if self.cnt == 0 => {
                    self.cnt = self.top;
                    ticks -= 1;
                    self.wrap_event(IF_UF);
                    self.compare();
                    continue;
                }
                (MODE_UPDOWN, _) if self.top == 0 => {
                    // Nothing to count between 0 and a TOP of 0.
                    return;
                }
                (MODE_UPDOWN, false) if self.cnt >= top => self.turn(true),
                (MODE_UPDOWN, true) if self.cnt == 0 => self.turn(false),
                _ => {}
            }
            let limit = if self.down { self.cnt } else { top - self.cnt };
            let step = ticks.min(u64::from(self.next_compare(limit)));
            if self.down {
                self.cnt -= step as u16;
            } else {
                self.cnt += step as u16;
            }
            ticks -= step;
            if self.mode() == MODE_UPDOWN {
                if !self.down && self.cnt == top {
                    self.turn(true);
                    self.wrap_event(IF_OF);
                } else if self.down && self.cnt == 0 {
                    self.turn(false);
                    self.wrap_event(IF_UF);
                }
            }
            self.compare();
        }
    }
}

impl Peripheral for Efm32Timer {
    fn on_system_reset(&mut self) {
        *self = Self::with_layout(self.irq, self.series1);
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let aligned = offset & !3;
        Ok((self.read_reg(aligned)? >> (8 * (offset - aligned))) as u8)
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.read_reg(offset & !3)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        let r = Self::refuse("byte stores (TIMER registers are word-wide)");
        self.checked(r)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let r = self.write_reg(offset & !3, value);
        self.checked(r)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn legacy_tick_active(&self) -> bool {
        self.running || self.line()
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        report(self.irq, self.line());
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if self.running {
            let presc = (self.ctrl >> 24) & 0xF;
            // PRESC DIV1..DIV1024; 11..15 are reserved and refused nowhere
            // by silicon docs, so clamp to DIV1024.
            let div = 1u64 << presc.min(10);
            self.carry += cycles;
            let ticks = self.carry / div;
            self.carry %= div;
            if ticks > 0 {
                self.advance(ticks);
            }
        }
        PeripheralTickResult {
            explicit_irqs: self.line().then(|| vec![self.irq]),
            ..Default::default()
        }
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer() -> Efm32Timer {
        Efm32Timer::new(2)
    }

    #[test]
    fn up_mode_wraps_at_top_raising_of_and_the_irq_follows_ien() {
        let mut t = timer();
        t.write_u32(TOP, 99).unwrap();
        t.write_u32(CMD, CMD_START).unwrap();
        t.tick_elapsed(99);
        assert_eq!(t.counter(), 99);
        assert_eq!(t.read_u32(IF).unwrap() & IF_OF, 0);
        let r = t.tick_elapsed(1);
        assert_eq!(t.counter(), 0);
        assert_eq!(t.read_u32(IF).unwrap() & IF_OF, IF_OF);
        assert!(r.explicit_irqs.is_none(), "flag latches without IEN");
        t.write_u32(IEN, IF_OF).unwrap();
        assert_eq!(t.tick_elapsed(1).explicit_irqs, Some(vec![2]));
        t.write_u32(IFC, IF_OF).unwrap();
        assert!(t.tick_elapsed(1).explicit_irqs.is_none());
        assert_eq!(t.read_u32(STATUS).unwrap() & 1, 1, "RUNNING");
        t.write_u32(CMD, CMD_STOP).unwrap();
        assert_eq!(t.read_u32(STATUS).unwrap() & 1, 0);
        let held = t.counter();
        t.tick_elapsed(500);
        assert_eq!(t.counter(), held);
    }

    #[test]
    fn prescaler_divides_cpu_cycles() {
        let mut t = timer();
        t.write_u32(CTRL, 4 << 24).unwrap(); // DIV16
        t.write_u32(CMD, CMD_START).unwrap();
        t.tick_elapsed(15);
        assert_eq!(t.counter(), 0);
        t.tick_elapsed(1);
        assert_eq!(t.counter(), 1);
        t.tick_elapsed(160);
        assert_eq!(t.counter(), 11);
    }

    #[test]
    fn down_mode_underflows_to_top_and_one_shot_stops() {
        let mut t = timer();
        t.write_u32(CTRL, MODE_DOWN | CTRL_OSMEN).unwrap();
        t.write_u32(TOP, 10).unwrap();
        t.write_u32(CNT, 3).unwrap();
        assert_eq!(t.read_u32(STATUS).unwrap() & 2, 2, "DIR = down");
        t.write_u32(CMD, CMD_START).unwrap();
        t.tick_elapsed(4);
        assert_eq!(t.counter(), 10);
        assert_eq!(t.read_u32(IF).unwrap() & (IF_OF | IF_UF), IF_UF);
        assert_eq!(t.read_u32(STATUS).unwrap() & 1, 0, "one-shot stopped");
    }

    #[test]
    fn updown_turns_at_top_and_zero() {
        let mut t = timer();
        t.write_u32(CTRL, MODE_UPDOWN).unwrap();
        t.write_u32(TOP, 5).unwrap();
        t.write_u32(CMD, CMD_START).unwrap();
        t.tick_elapsed(5);
        assert_eq!(t.counter(), 5);
        assert_eq!(t.read_u32(IF).unwrap(), IF_OF);
        t.tick_elapsed(2);
        assert_eq!(t.counter(), 3);
        assert_eq!(t.read_u32(STATUS).unwrap() & 2, 2);
        t.tick_elapsed(3);
        assert_eq!(t.counter(), 0);
        assert_eq!(t.read_u32(IF).unwrap(), IF_OF | IF_UF);
        t.tick_elapsed(1);
        assert_eq!(t.counter(), 1);
    }

    #[test]
    fn compare_flags_and_buffers_load_at_the_wrap() {
        let mut t = timer();
        t.write_u32(TOP, 1000).unwrap();
        t.write_u32(CC_BASE, 2).unwrap(); // CC0 OUTPUTCOMPARE
        t.write_u32(CC_BASE + 4, 300).unwrap();
        t.write_u32(CC_BASE + 0x10, 0).unwrap(); // CC1 OFF: never flags
        t.write_u32(CC_BASE + 0x14, 300).unwrap();
        t.write_u32(CMD, CMD_START).unwrap();
        t.tick_elapsed(299);
        assert_eq!(t.read_u32(IF).unwrap() & IF_CC0, 0);
        t.tick_elapsed(1);
        assert_eq!(t.read_u32(IF).unwrap() & 0x70, IF_CC0);
        // Buffered writes wait for the wrap.
        t.write_u32(TOPB, 500).unwrap();
        t.write_u32(CC_BASE + 0xC, 400).unwrap();
        assert_eq!(t.read_u32(STATUS).unwrap() & 0x104, 0x104);
        assert_eq!(t.read_u32(TOP).unwrap(), 1000);
        t.tick_elapsed(701);
        assert_eq!(t.counter(), 0);
        assert_eq!(t.read_u32(TOP).unwrap(), 500);
        assert_eq!(t.read_u32(CC_BASE + 4).unwrap(), 400);
        assert_eq!(t.read_u32(CC_BASE + 8).unwrap(), 300, "CCVP");
        assert_eq!(t.read_u32(STATUS).unwrap() & 0x104, 0);
        t.write_u32(IFC, 0xFFFF).unwrap();
        t.tick_elapsed(400);
        assert_eq!(t.read_u32(IF).unwrap() & IF_CC0, IF_CC0);
        t.tick_elapsed(101);
        assert_eq!(t.counter(), 0, "wraps at the new TOP");
    }

    #[test]
    fn unmodelled_features_are_refused() {
        let mut t = timer();
        assert!(t.write_u32(CTRL, 3).is_err(), "QDEC");
        assert!(t.write_u32(CTRL, 1 << 16).is_err(), "CLKSEL");
        assert!(t.write_u32(CTRL, CTRL_SYNC).is_err(), "SYNC");
        assert!(t.write_u32(CTRL, 1 << 8).is_err(), "RISEA");
        assert!(t.write_u32(CC_BASE, 1).is_err(), "input capture");
        assert!(t.write_u32(0x70, 1).is_err(), "DTI");
        assert!(t.write(0, 1).is_err(), "byte store");
    }

    // Series 1 offsets (efm32pg1b_timer.h).
    const S1_IF: u64 = 0x0C;
    const S1_IFS: u64 = 0x10;
    const S1_IFC: u64 = 0x14;
    const S1_IEN: u64 = 0x18;
    const S1_LOCK: u64 = 0x2C;
    const S1_ROUTEPEN: u64 = 0x30;
    const S1_ROUTELOC0: u64 = 0x34;

    #[test]
    fn series1_moves_the_flag_words_and_ien_gates_the_line() {
        let mut t = Efm32Timer::new_series1(10);
        t.write_u32(TOP, 9).unwrap();
        t.write_u32(CMD, CMD_START).unwrap();
        t.tick_elapsed(10);
        assert_eq!(t.read_u32(S1_IF).unwrap(), IF_OF);
        assert_eq!(t.read_u32(S1_IEN).unwrap(), 0);
        assert!(t.tick_elapsed(1).explicit_irqs.is_none());
        t.write_u32(S1_IEN, IF_OF).unwrap();
        assert_eq!(t.tick_elapsed(1).explicit_irqs, Some(vec![10]));
        // 0x0C is IF here, not IEN: writing it moves nothing.
        t.write_u32(S1_IF, 0).unwrap();
        assert_eq!(t.read_u32(S1_IF).unwrap(), IF_OF);
        t.write_u32(S1_IFC, IF_OF).unwrap();
        assert_eq!(t.read_u32(S1_IF).unwrap(), 0);
        t.write_u32(S1_IFS, 0xFFFF_FFFF).unwrap();
        assert_eq!(t.read_u32(S1_IF).unwrap(), S1_IF_MASK);
        assert_eq!(t.read_u32(S1_IFC).unwrap(), 0);
    }

    #[test]
    fn series1_has_a_fourth_compare_channel_at_0x90() {
        let mut t = Efm32Timer::new_series1(10);
        t.write_u32(TOP, 1000).unwrap();
        t.write_u32(S1_CC_BASE + 0x30, 2).unwrap(); // CC3 OUTPUTCOMPARE
        t.write_u32(S1_CC_BASE + 0x34, 250).unwrap();
        t.write_u32(CMD, CMD_START).unwrap();
        t.tick_elapsed(250);
        assert_eq!(t.read_u32(S1_IF).unwrap() & 0xF0, IF_CC0 << 3);
        t.write_u32(S1_CC_BASE + 0x3C, 400).unwrap();
        assert_eq!(t.read_u32(STATUS).unwrap() & (1 << 11), 1 << 11, "CCVBV3");
        t.tick_elapsed(751);
        assert_eq!(t.read_u32(S1_CC_BASE + 0x34).unwrap(), 400);
        assert_eq!(t.read_u32(S1_CC_BASE + 0x38).unwrap(), 250, "CCVP");
        // The Series-0 CC0 offset is ROUTEPEN here.
        t.write_u32(S1_ROUTEPEN, 0xF).unwrap();
        t.write_u32(S1_ROUTELOC0, 0x0302_0100).unwrap();
        assert_eq!(t.read_u32(S1_ROUTEPEN).unwrap(), 0xF);
        assert_eq!(t.read_u32(S1_ROUTELOC0).unwrap(), 0x0302_0100);
        assert_eq!(t.read_u32(S1_CC_BASE).unwrap(), 0);
    }

    #[test]
    fn series1_updown_raises_dirchg_and_series0_does_not() {
        for (series1, dirchg) in [(true, IF_DIRCHG), (false, 0)] {
            let mut t = Efm32Timer::with_layout(10, series1);
            t.write_u32(CTRL, MODE_UPDOWN).unwrap();
            t.write_u32(TOP, 5).unwrap();
            t.write_u32(CMD, CMD_START).unwrap();
            t.tick_elapsed(5);
            let flags = t.read_u32(if series1 { S1_IF } else { IF }).unwrap();
            assert_eq!(flags, IF_OF | dirchg);
        }
    }

    #[test]
    fn series1_lock_takes_only_the_unlock_key_and_dti_is_refused() {
        let mut t = Efm32Timer::new_series1(10);
        t.write_u32(S1_LOCK, S1_LOCK_UNLOCK).unwrap();
        assert_eq!(t.read_u32(S1_LOCK).unwrap(), 0);
        assert!(t.write_u32(S1_LOCK, 0).is_err(), "locking");
        assert!(t.write_u32(0xA0, 1).is_err(), "DTCTRL");
        // 0x70 is inside CC1 on Series 1, not DTI.
        assert!(t.write_u32(0x70, 2).is_ok());
    }
}
