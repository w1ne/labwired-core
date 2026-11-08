// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! SAM D21 RTC (DS40001882 "RTC – Real-Time Counter") at 0x4000_1400, IRQ 3:
//! MODE0 (32-bit counter, COMP0, clear on match), MODE1 (16-bit counter,
//! PER, COMP0/COMP1) and MODE2 (clock/calendar, 24- and 12-hour, ALARM0 with
//! MASK.SEL, clear on match). Register map cross-checked against
//! `tests/fixtures/real_world/atsamd21g18a.svd`.
//!
//! Counter timing follows the datasheet's next-cycle rule: a match with
//! COMPn/ALARM0 raises its flag on the CLK_RTC_CNT edge that leaves the
//! matching value. With CTRL.MATCHCLR that same edge clears the counter (the
//! calendar to 0x0000_0000) and raises OVF too ("INTFLAG.CMP0 and
//! INTFLAG.OVF will both be set simultaneously"). MODE1 wraps after PER;
//! MODE2 rolls the calendar over (leap year = YEAR % 4 == 0) and raises OVF
//! when YEAR wraps from 63.
//!
//! Clocking: CLK_RTC_CNT = GCLK_RTC / CTRL.PRESCALER, converted from CPU
//! cycles with the attached `cpu_hz`. GCLK routing is NOT modelled (the SAM D21
//! GCLK here is a declarative register bank), so GCLK_RTC is a fixed
//! frequency, default 1024 Hz — the 32.768 kHz oscillator divided by 32, which
//! is what the Arduino RTCZero library and ASF's RTC drivers configure (with
//! PRESCALER DIV1024 that gives the 1 Hz calendar). `config: { gclk_hz: … }`
//! overrides it. PM.APBAMASK.RTC (bit 5, on at reset) gates counting.
//!
//! Not modelled, refused on write: EVCTRL events, FREQCORR, reserved
//! prescaler codes and MASK.SEL 7. Writes synchronise immediately:
//! STATUS.SYNCBUSY reads 0, SYNCRDY never sets, READREQ.RREQ completes at once
//! and COUNT/CLOCK always read current.
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;

pub const RTC21_BASE: u64 = 0x4000_1400;
pub const RTC21_IRQ: u32 = 3;
/// GCLK_RTC assumed when the chip yaml declares none: OSC32K / 32.
pub const RTC21_DEFAULT_GCLK_HZ: u64 = 1024;

const PM_APBAMASK: u64 = 0x4000_0418;
const APBAMASK_RTC: u32 = 1 << 5;
const SWRST: u16 = 1;
const ENABLE: u16 = 1 << 1;
const CLKREP: u16 = 1 << 6;
const MATCHCLR: u16 = 1 << 7;
const CMP0: u8 = 1;
const OVF: u8 = 1 << 7;
/// INTFLAG bits per mode: CMP0/ALARM0 (+CMP1 in MODE1), SYNCRDY, OVF.
const INT_MASK: [u8; 3] = [0xc1, 0xc3, 0xc1];
/// MODE2 ALARM0 compare masks for MASK.SEL 1..=6 (SS .. YYMMDDHHMMSS).
const SEL_MASK: [u32; 7] = [
    0,
    0x0000_003f,
    0x0000_0fff,
    0x0001_ffff,
    0x003f_ffff,
    0x03ff_ffff,
    0xffff_ffff,
];

#[derive(Debug)]
pub struct Samd21Rtc {
    ctrl: u16,
    readreq: u16,
    dbgctrl: u8,
    inten: u8,
    intflag: u8,
    /// MODE0 COUNT, MODE1 COUNT (low half) or MODE2 CLOCK.
    count: u32,
    per: u16,
    /// MODE0 COMP0 / MODE2 ALARM0 in `comp[0]`; MODE1 COMP0/COMP1.
    comp: [u32; 2],
    mask_sel: u8,
    gclk_hz: u64,
    cpu_hz: u64,
    apb_clocked: bool,
    gate_dirty: bool,
    /// `cycles * gclk_hz` not yet worth a whole CLK_RTC_CNT edge.
    carry: u128,
    refusal: Option<String>,
}

impl Samd21Rtc {
    pub fn new(gclk_hz: u64) -> Self {
        Self {
            ctrl: 0,
            readreq: 0x0010,
            dbgctrl: 0,
            inten: 0,
            intflag: 0,
            count: 0,
            per: 0,
            comp: [0; 2],
            mask_sel: 0,
            gclk_hz: gclk_hz.max(1),
            cpu_hz: 1_000_000,
            apb_clocked: true,
            gate_dirty: true,
            carry: 0,
            refusal: None,
        }
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd21 RTC: {what}"
        )))
    }
    fn mode(&self) -> usize {
        ((self.ctrl >> 2) & 3) as usize
    }
    fn enabled(&self) -> bool {
        self.ctrl & ENABLE != 0
    }
    fn running(&self) -> bool {
        self.enabled() && self.apb_clocked
    }
    fn line(&self) -> bool {
        self.intflag & self.inten != 0
    }
    pub fn count(&self) -> u32 {
        self.count
    }
    fn reset(&mut self) {
        *self = Self {
            cpu_hz: self.cpu_hz,
            ..Self::new(self.gclk_hz)
        };
    }

    /// MODE0/MODE1: jump straight to the next edge that leaves a match value
    /// or the top, so a long idle costs one step per event.
    fn advance_counter(&mut self, mut ticks: u64) {
        let mode1 = self.mode() == 1;
        let wmax = if mode1 { 0xffff } else { u64::from(u32::MAX) };
        let top = if mode1 { u64::from(self.per) } else { wmax };
        let ncomp = if mode1 { 2 } else { 1 };
        let clear_on_match = !mode1 && self.ctrl & MATCHCLR != 0;
        while ticks > 0 {
            let here = u64::from(self.count);
            let mut targets = vec![top, wmax];
            targets.extend(self.comp[..ncomp].iter().map(|&c| u64::from(c)));
            let dist = targets
                .iter()
                .filter(|&&t| t >= here)
                .map(|&t| t - here)
                .min()
                .unwrap_or(0);
            if ticks <= dist {
                self.count = (here + ticks) as u32;
                return;
            }
            // `dist` quiet edges, then the event edge leaving `at`.
            ticks -= dist + 1;
            let at = here + dist;
            let mut cleared = false;
            for i in 0..ncomp {
                if u64::from(self.comp[i]) == at {
                    self.intflag |= CMP0 << i;
                    cleared |= i == 0 && clear_on_match;
                }
            }
            if cleared || at == top || at == wmax {
                self.intflag |= OVF;
                self.count = 0;
            } else {
                self.count = (at + 1) as u32;
            }
        }
    }

    /// MODE2: one calendar second (CLK_RTC_CNT edge) at a time.
    fn advance_clock(&mut self, ticks: u64) {
        let sel = SEL_MASK[usize::from(self.mask_sel)];
        for _ in 0..ticks {
            if sel != 0 && (self.count ^ self.comp[0]) & sel == 0 {
                self.intflag |= CMP0;
                if self.ctrl & MATCHCLR != 0 {
                    self.intflag |= OVF;
                    self.count = 0;
                    continue;
                }
            }
            self.step_second();
        }
    }
    fn step_second(&mut self) {
        let c = self.count;
        let mut sec = c & 0x3f;
        let mut min = (c >> 6) & 0x3f;
        let mut hour = (c >> 12) & 0x1f;
        let mut day = (c >> 17) & 0x1f;
        let mut month = (c >> 22) & 0xf;
        let mut year = c >> 26;
        let twelve = self.ctrl & CLKREP != 0;
        sec += 1;
        if sec >= 60 {
            sec = 0;
            min += 1;
        }
        if min >= 60 {
            min = 0;
            let mut day_done = false;
            if twelve {
                // HOUR[4] = PM, HOUR[3:0] = 1..12; 11 PM rolls the day.
                let (pm, h) = (hour & 0x10 != 0, hour & 0xf);
                hour = match (pm, h) {
                    (_, 12) => (hour & 0x10) | 1,
                    (false, 11) => 0x10 | 12,
                    (true, 11) => {
                        day_done = true;
                        12
                    }
                    _ => (hour & 0x10) | (h + 1),
                };
            } else {
                hour += 1;
                if hour >= 24 {
                    hour = 0;
                    day_done = true;
                }
            }
            if day_done {
                day += 1;
                let dim = match month {
                    2 if year % 4 == 0 => 29,
                    2 => 28,
                    4 | 6 | 9 | 11 => 30,
                    _ => 31,
                };
                if day > dim {
                    day = 1;
                    month += 1;
                    if month > 12 {
                        month = 1;
                        year += 1;
                        if year > 63 {
                            year = 0;
                            self.intflag |= OVF;
                        }
                    }
                }
            }
        }
        self.count = sec | min << 6 | hour << 12 | day << 17 | month << 22 | year << 26;
    }
    fn advance(&mut self, ticks: u64) {
        match self.mode() {
            2 => self.advance_clock(ticks),
            _ => self.advance_counter(ticks),
        }
    }

    fn read_reg(&self, offset: u64) -> SimResult<u32> {
        let mode = self.mode();
        Ok(match offset {
            0x00 => u32::from(self.ctrl),
            0x02 => u32::from(self.readreq),
            0x04 => 0, // EVCTRL
            0x06 | 0x07 => u32::from(self.inten),
            0x08 => u32::from(self.intflag),
            0x0a => 0, // STATUS.SYNCBUSY: writes synchronise at once
            0x0b => u32::from(self.dbgctrl),
            0x0c => 0, // FREQCORR
            0x10 => self.count,
            0x14 if mode == 1 => u32::from(self.per),
            0x18 => self.comp[0],
            0x1a if mode == 1 => self.comp[1],
            0x1c if mode == 2 => u32::from(self.mask_sel),
            _ => return Self::refuse("register outside the mode's map"),
        })
    }
    fn write_ctrl(&mut self, value: u16) -> SimResult<()> {
        if value & SWRST != 0 {
            self.reset();
            return Ok(());
        }
        let protected = !(ENABLE | SWRST);
        if self.enabled() && (value ^ self.ctrl) & protected != 0 {
            return Self::refuse("CTRL is enable-protected");
        }
        let mode = (value >> 2) & 3;
        let allowed: u16 = match mode {
            0 => 0x0f8e,
            1 => 0x0f0e,
            2 => 0x0fce,
            _ => return Self::refuse("CTRL.MODE 3 is reserved"),
        };
        if value & !allowed != 0 {
            return Self::refuse("CTRL bits outside the mode");
        }
        if (value >> 8) & 0xf > 10 {
            return Self::refuse("reserved CTRL.PRESCALER code");
        }
        if mode as usize != self.mode() {
            self.intflag = 0;
        }
        self.ctrl = value;
        self.gate_dirty = true;
        Ok(())
    }
    /// Store `value` (`size` bytes) at `offset`; narrower stores into a wider
    /// COUNT/CLOCK/COMP/ALARM merge into the current value.
    fn write_reg(&mut self, offset: u64, value: u32, size: u32) -> SimResult<()> {
        let mode = self.mode();
        let merge = |old: u32, base: u64| -> u32 {
            let shift = 8 * (offset - base) as u32;
            let m = if size >= 4 {
                u32::MAX
            } else {
                ((1u32 << (8 * size)) - 1) << shift
            };
            (old & !m) | ((value << shift) & m)
        };
        match (offset, size) {
            (0x00, 2) | (0x00, 4) if value >> 16 == 0 => self.write_ctrl(value as u16)?,
            (0x00, 1) => self.write_ctrl((self.ctrl & 0xff00) | value as u16)?,
            (0x01, 1) => self.write_ctrl((self.ctrl & 0x00ff) | (value as u16) << 8)?,
            (0x02, 2) => self.readreq = (value as u16) & 0x403f,
            (0x03, 1) => self.readreq = (self.readreq & 0xff) | ((value as u16) << 8) & 0x4000,
            (0x04, 2) if value == 0 => {}
            (0x04, _) | (0x05, _) => return Self::refuse("EVCTRL events"),
            (0x06, 1) => self.inten &= !(value as u8),
            (0x07, 1) => self.inten |= (value as u8) & INT_MASK[mode],
            (0x08, 1) => self.intflag &= !(value as u8),
            (0x0a, 1) => {}
            (0x0b, 1) => self.dbgctrl = (value as u8) & 1,
            (0x0c, 1) if value == 0 => {}
            (0x0c, 1) => return Self::refuse("FREQCORR"),
            (0x10..=0x13, _) if mode != 1 => self.count = merge(self.count, 0x10),
            (0x10..=0x11, _) if mode == 1 => self.count = merge(self.count, 0x10) & 0xffff,
            (0x14..=0x15, _) if mode == 1 => self.per = merge(u32::from(self.per), 0x14) as u16,
            (0x18..=0x1b, _) if mode != 1 => self.comp[0] = merge(self.comp[0], 0x18),
            (0x18..=0x19, _) if mode == 1 => self.comp[0] = merge(self.comp[0], 0x18) & 0xffff,
            (0x1a..=0x1b, _) if mode == 1 => self.comp[1] = merge(self.comp[1], 0x1a) & 0xffff,
            (0x1c, 1) if mode == 2 && value & 7 != 7 => self.mask_sel = (value & 7) as u8,
            (0x1c, 1) if mode == 2 => return Self::refuse("reserved MASK.SEL 7"),
            _ => return Self::refuse("store outside the mode's map"),
        }
        Ok(())
    }
    fn checked(&mut self, result: SimResult<()>) -> SimResult<()> {
        if let Err(e) = &result {
            self.refusal.get_or_insert_with(|| e.to_string());
        }
        result
    }
}

impl Peripheral for Samd21Rtc {
    fn on_system_reset(&mut self) {
        self.reset();
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let mode1 = self.mode() == 1;
        let aligned = match offset {
            0x00..=0x01 => 0x00,
            0x02..=0x03 => 0x02,
            0x04..=0x05 => 0x04,
            0x06 | 0x07 => offset,
            0x10..=0x11 if mode1 => 0x10,
            0x14..=0x15 if mode1 => 0x14,
            0x18..=0x19 if mode1 => 0x18,
            0x1a..=0x1b if mode1 => 0x1a,
            0x10..=0x13 => 0x10,
            0x18..=0x1b => 0x18,
            _ => offset,
        };
        Ok((self.read_reg(aligned)? >> (8 * (offset - aligned))) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let r = self.write_reg(offset, u32::from(value), 1);
        self.checked(r)
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        let r = self.write_reg(offset, u32::from(value), 2);
        self.checked(r)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let r = match offset {
            // A word store to a 16-bit register keeps its low half.
            0x00 | 0x02 | 0x04 if value >> 16 == 0 => self.write_reg(offset, value, 2),
            _ => self.write_reg(offset, value, 4),
        };
        self.checked(r)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.cpu_hz = hz.max(1);
    }
    fn needs_bus_tick(&self) -> bool {
        self.enabled() && (self.gate_dirty || !self.apb_clocked)
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.apb_clocked = bus
            .read_u32(PM_APBAMASK)
            .is_ok_and(|m| m & APBAMASK_RTC != 0);
        self.gate_dirty = false;
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn legacy_tick_active(&self) -> bool {
        self.running() || self.line()
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        report(RTC21_IRQ, self.line());
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if self.running() && !self.gate_dirty {
            self.carry += u128::from(cycles) * u128::from(self.gclk_hz);
            let div = u128::from(self.cpu_hz) << ((self.ctrl >> 8) & 0xf);
            let ticks = (self.carry / div) as u64;
            self.carry %= div;
            if ticks > 0 {
                self.advance(ticks);
            }
        }
        PeripheralTickResult {
            explicit_irqs: self.line().then(|| vec![RTC21_IRQ]),
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
        serde_json::json!({"peripheral": "samd21_rtc", "ctrl": self.ctrl,
            "count": self.count, "per": self.per, "comp": self.comp,
            "mask_sel": self.mask_sel, "intflag": self.intflag, "inten": self.inten,
            "gclk_hz": self.gclk_hz, "apb_clocked": self.apb_clocked,
            "refusal": self.refusal})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1024 Hz GCLK on a 1024 Hz core: one CPU cycle per DIV1 edge.
    fn rtc() -> Samd21Rtc {
        let mut r = Samd21Rtc::new(1024);
        r.attach_cpu_hz(1024);
        r
    }
    fn go(r: &mut Samd21Rtc, ctrl: u16) {
        r.write_u16(0x00, ctrl | ENABLE).unwrap();
        r.gate_dirty = false;
    }

    #[test]
    fn mode0_compare_matchclr_and_prescaler() {
        let mut r = rtc();
        r.write_u16(0x00, MATCHCLR | 1 << 8).unwrap(); // MODE0, DIV2
        r.write_u32(0x18, 5).unwrap();
        r.write(0x07, CMP0 | OVF).unwrap();
        go(&mut r, MATCHCLR | 1 << 8);
        r.tick_elapsed(2 * 5);
        assert_eq!(r.count(), 5);
        assert_eq!(r.read(0x08).unwrap(), 0, "flag on the edge leaving COMP0");
        r.tick_elapsed(2);
        assert_eq!(r.count(), 0, "cleared on match");
        assert_eq!(r.read(0x08).unwrap(), CMP0 | OVF);
        assert!(r.line());
        r.write(0x08, CMP0 | OVF).unwrap();
        // 3 more periods of 6 edges each in one long tick.
        r.tick_elapsed(2 * 18 + 2 * 3);
        assert_eq!(r.count(), 3);
        assert_eq!(r.read(0x08).unwrap(), CMP0 | OVF);
    }

    #[test]
    fn mode0_wraps_at_max_without_matchclr() {
        let mut r = rtc();
        r.write_u16(0x00, 0).unwrap();
        r.write_u32(0x10, u32::MAX - 1).unwrap();
        r.write_u32(0x18, 1).unwrap();
        go(&mut r, 0);
        r.tick_elapsed(2);
        assert_eq!(r.count(), 0);
        assert_eq!(r.read(0x08).unwrap(), OVF);
        r.tick_elapsed(2);
        assert_eq!(r.count(), 2);
        assert_eq!(r.read(0x08).unwrap(), OVF | CMP0, "CMP0 leaving 1");
    }

    #[test]
    fn mode1_per_and_two_compares() {
        let mut r = rtc();
        let ctrl = 1 << 2; // MODE1, DIV1
        r.write_u16(0x00, ctrl).unwrap();
        r.write_u16(0x14, 9).unwrap();
        r.write_u16(0x18, 3).unwrap();
        r.write_u16(0x1a, 7).unwrap();
        go(&mut r, ctrl);
        r.tick_elapsed(4);
        assert_eq!(r.read(0x08).unwrap(), CMP0);
        r.tick_elapsed(4);
        assert_eq!(r.read(0x08).unwrap(), CMP0 | 2);
        r.tick_elapsed(2);
        assert_eq!(r.read_u16(0x10).unwrap(), 0, "wrap after PER");
        assert_eq!(r.read(0x08).unwrap(), CMP0 | 2 | OVF);
    }

    #[test]
    fn mode2_calendar_rollover_alarm_and_twelve_hour() {
        let mut r = rtc();
        let ctrl = 2 << 2; // MODE2, DIV1
        r.write_u16(0x00, ctrl).unwrap();
        // 2063-12-31 23:59:58 (YEAR 63).
        let t = |y: u32, mo: u32, d: u32, h: u32, mi: u32, s: u32| {
            s | mi << 6 | h << 12 | d << 17 | mo << 22 | y << 26
        };
        r.write_u32(0x10, t(63, 12, 31, 23, 59, 58)).unwrap();
        r.write_u32(0x18, t(0, 1, 1, 0, 0, 0)).unwrap();
        r.write(0x1c, 6).unwrap(); // YYMMDDHHMMSS
        go(&mut r, ctrl);
        r.tick_elapsed(2);
        assert_eq!(r.count(), t(0, 1, 1, 0, 0, 0));
        assert_eq!(r.read(0x08).unwrap(), OVF, "YEAR 63 -> 0");
        r.tick_elapsed(1);
        assert_eq!(r.read(0x08).unwrap(), OVF | CMP0, "ALARM0 leaving match");
        // Leap February (YEAR 4).
        r.write_u16(0x00, ctrl).unwrap();
        r.write_u32(0x10, t(4, 2, 28, 23, 59, 59)).unwrap();
        go(&mut r, ctrl);
        r.tick_elapsed(1);
        assert_eq!(r.count(), t(4, 2, 29, 0, 0, 0));
        // 12-hour: 11:59:59 PM -> 12:00:00 AM next day; 12:59:59 -> 1 PM.
        r.write_u16(0x00, ctrl).unwrap();
        let ctrl12 = ctrl | CLKREP;
        r.write_u16(0x00, ctrl12).unwrap();
        r.write_u32(0x10, t(5, 3, 31, 0x10 | 11, 59, 59)).unwrap();
        go(&mut r, ctrl12);
        r.tick_elapsed(1);
        assert_eq!(r.count(), t(5, 4, 1, 12, 0, 0));
        r.write_u16(0x00, ctrl12).unwrap();
        r.write_u32(0x10, t(5, 4, 1, 0x10 | 12, 59, 59)).unwrap();
        go(&mut r, ctrl12);
        r.tick_elapsed(1);
        assert_eq!(r.count(), t(5, 4, 1, 0x10 | 1, 0, 0));
    }

    #[test]
    fn gate_and_refusals() {
        let mut r = rtc();
        go(&mut r, 0);
        r.apb_clocked = false;
        r.tick_elapsed(100);
        assert_eq!(r.count(), 0, "APBAMASK.RTC off: no counting");
        assert!(
            r.write_u16(0x00, ENABLE | 1 << 8).is_err(),
            "enable-protected"
        );
        assert!(r.write_u16(0x04, 1).is_err(), "events");
        assert!(r.write(0x0c, 1).is_err(), "FREQCORR");
        r.write_u16(0x00, 0).unwrap();
        assert!(r.write_u16(0x00, 3 << 2).is_err(), "MODE 3");
        assert!(r.write_u16(0x00, 11 << 8).is_err(), "PRESCALER 11");
        assert!(r.snapshot()["refusal"].is_string(), "first refusal kept");
        r.write_u16(0x00, SWRST).unwrap();
        assert_eq!(r.read_u16(0x02).unwrap(), 0x0010, "READREQ reset");
    }
}
