// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Opt-in SAMD51 TC slice (DS60001507F 48): 8- and 16-bit counters with the
//! NFRQ/MFRQ/NPWM/MPWM wave generators, compare-match and overflow interrupts,
//! the CTRLB commands and READSYNC. The counter advances from CPU cycles by the
//! nominal ratio f_TC / f_CPU resolved from the live clock tree; it stops when
//! its bus clock, generic clock channel or source is not running. CCBUF/PERBUF
//! are copied at the UPDATE condition (counter wrap).
//!
//! Waveform outputs WO0/WO1 are published edge-exact onto pads routed to PMUX
//! function E, for up-counting NPWM (WOx high from BOTTOM until the CCx match)
//! and MPWM WO1 (TOP = CC0); DRVCTRL.INVENx inverts. Every other output (NFRQ,
//! MFRQ, MPWM WO0, down-counting) is not modelled and drives low. Capture,
//! events, one-shot retrigger timing, 32-bit pairing and DMA are refused or not
//! modelled.
use super::clock_tree;
use crate::peripherals::esp_ledc_wave::{LedcDrive, LedcPads, LedcTimebase};
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;

/// Line names of a TC's waveform-output wire, in WO order.
pub const TC_OUTPUT_LINES: &[&str] = &["WO0", "WO1"];

const MCLK: u64 = 0x40000800;
const SWRST: u32 = 1;
const ENABLE: u32 = 1 << 1;
const OVF: u8 = 1;
const MC0: u8 = 1 << 4;

#[derive(Clone, Copy, Debug)]
pub struct TcInstance {
    pub base: u64,
    pub irq: u32,
    pub gclk_channel: u64,
    /// (MCLK mask register offset, bit)
    pub bus_clock: (u64, u32),
    /// DMAC trigger source of the overflow request (`TCn_DMAC_ID_OVF`).
    pub dma_ovf: u32,
}

/// TC0..TC5 on the SAMD51J19A (ATDF instance parameters and DMAC trigger
/// sources); G-variant parts have TC0..TC3.
pub const TC_INSTANCES: [TcInstance; 6] = [
    TcInstance {
        base: 0x40003800,
        irq: 107,
        gclk_channel: 9,
        bus_clock: (0x14, 14),
        dma_ovf: 44,
    },
    TcInstance {
        base: 0x40003c00,
        irq: 108,
        gclk_channel: 9,
        bus_clock: (0x14, 15),
        dma_ovf: 47,
    },
    TcInstance {
        base: 0x4101a000,
        irq: 109,
        gclk_channel: 26,
        bus_clock: (0x18, 13),
        dma_ovf: 50,
    },
    TcInstance {
        base: 0x4101c000,
        irq: 110,
        gclk_channel: 26,
        bus_clock: (0x18, 14),
        dma_ovf: 53,
    },
    TcInstance {
        base: 0x42001400,
        irq: 111,
        gclk_channel: 30,
        bus_clock: (0x1c, 5),
        dma_ovf: 56,
    },
    TcInstance {
        base: 0x42001800,
        irq: 112,
        gclk_channel: 30,
        bus_clock: (0x1c, 6),
        dma_ovf: 59,
    },
];

#[derive(Debug)]
pub struct Samd51Tc {
    instance: TcInstance,
    ctrla: u32,
    ctrlb: u8,
    inten: u8,
    intflag: u8,
    wave: u8,
    drvctrl: u8,
    dbgctrl: u8,
    count: u32,
    per: u8,
    cc: [u32; 2],
    /// CCBUF0/1 and PERBUF with their valid bits (STATUS CCBUFV0/1, PERBUFV).
    ccbuf: [u32; 2],
    perbuf: u8,
    bufv: u8,
    // Resolved clock ratio (f_TC, f_CPU) and the fractional carry between ticks.
    rate: Option<(u64, u64)>,
    rate_dirty: bool,
    carry: u64,
    refusal: Option<String>,
    /// Waveform outputs; generated only once the bus routed a pad to them.
    outputs: LedcPads,
    wired: bool,
    /// A wrap applied buffered values; the outputs follow on this tick.
    buffers_applied: bool,
    /// The overflow DMA request: set at each wrap, cleared when the DMAC
    /// serves it. One request is pending at most, as on silicon.
    ovf_request: bool,
    clock: Option<crate::CycleClock>,
    /// Cycles seen by `tick_elapsed`: the time base without a cycle clock.
    elapsed: u64,
}

impl Samd51Tc {
    pub fn new(instance: TcInstance) -> Self {
        Self {
            instance,
            ctrla: 0,
            ctrlb: 0,
            inten: 0,
            intflag: 0,
            wave: 0,
            drvctrl: 0,
            dbgctrl: 0,
            count: 0,
            per: 0xff,
            cc: [0; 2],
            ccbuf: [0; 2],
            perbuf: 0xff,
            bufv: 0,
            rate: None,
            rate_dirty: true,
            carry: 0,
            refusal: None,
            outputs: LedcPads::new(TC_OUTPUT_LINES),
            wired: false,
            buffers_applied: false,
            ovf_request: false,
            clock: None,
            elapsed: 0,
        }
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd51 TC slice: {what}"
        )))
    }
    fn mode(&self) -> u32 {
        (self.ctrla >> 2) & 3 // 0 COUNT16, 1 COUNT8, 2 COUNT32
    }
    fn enabled(&self) -> bool {
        self.ctrla & ENABLE != 0
    }
    fn width_mask(&self) -> u32 {
        if self.mode() == 1 {
            0xff
        } else {
            0xffff
        }
    }
    fn prescaler(&self) -> u64 {
        [1, 2, 4, 8, 16, 64, 256, 1024][((self.ctrla >> 8) & 7) as usize]
    }
    /// TOP: MAX for NFRQ/NPWM, CC0 for MFRQ/MPWM; PER replaces MAX in 8-bit mode.
    fn top(&self) -> u32 {
        match self.wave & 3 {
            1 | 3 => self.cc[0],
            _ if self.mode() == 1 => u32::from(self.per),
            _ => self.width_mask(),
        }
    }
    fn reset(&mut self) {
        // SWRST resets registers; the output wire and its routing survive.
        let outputs = std::mem::replace(&mut self.outputs, LedcPads::new(TC_OUTPUT_LINES));
        let (wired, clock, elapsed) = (self.wired, self.clock.take(), self.elapsed);
        *self = Self::new(self.instance);
        self.outputs = outputs;
        self.wired = wired;
        self.clock = clock;
        self.elapsed = elapsed;
    }
    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(self.elapsed, |c| c.now())
    }
    /// UPDATE condition: buffered CC/PER take effect.
    fn update(&mut self) {
        for i in 0..2 {
            if self.bufv & (0x10 << i) != 0 {
                self.cc[i] = self.ccbuf[i];
            }
        }
        if self.bufv & 0x08 != 0 {
            self.per = self.perbuf;
        }
        self.bufv = 0;
    }
    /// What WOx drives now, from the registers and the resolved count rate.
    fn output_drive(&self, ch: usize) -> LedcDrive {
        let invert = self.drvctrl & (1 << ch) != 0;
        let up = self.ctrlb & 1 == 0;
        let (period, duty) = match self.wave & 3 {
            2 => (u64::from(self.top()) + 1, u64::from(self.cc[ch])),
            3 if ch == 1 => (u64::from(self.cc[0]) + 1, u64::from(self.cc[1])),
            _ => return LedcDrive::Level(invert), // not modelled: low (inverted: high)
        };
        if !up {
            return LedcDrive::Level(invert);
        }
        let duty = duty.min(period);
        // Inverted: low from BOTTOM until the match, high for the rest.
        let (hpoint, duty) = if invert {
            (duty, period - duty)
        } else {
            (0, duty)
        };
        match (self.enabled(), self.rate) {
            (true, Some((tc, cpu))) if tc > 0 => {
                let now = self.now();
                // The counter's BOTTOM for the period in progress.
                let into = (u128::from(self.count) * u128::from(cpu) + u128::from(self.carry))
                    / u128::from(tc);
                let origin = now.saturating_sub(into.min(u128::from(u64::MAX)) as u64);
                LedcDrive::Pwm {
                    timer: LedcTimebase {
                        period,
                        num: u128::from(cpu),
                        den: u128::from(tc),
                        origin,
                    },
                    hpoint,
                    duty,
                }
            }
            // Stopped or unclocked: the output holds the level at the current count.
            _ => {
                let v = u64::from(self.count) % period;
                LedcDrive::Level((v + period - hpoint) % period < duty)
            }
        }
    }
    fn refresh_outputs(&mut self) {
        if !self.wired {
            return;
        }
        let now = self.now();
        for ch in 0..2 {
            let drive = self.output_drive(ch);
            self.outputs.set_drive(ch, drive, now);
        }
    }
    pub fn count(&self) -> u32 {
        self.count
    }
    fn line(&self) -> bool {
        self.intflag & self.inten != 0
    }
    /// Advance by `ticks` counter clocks, raising overflow and compare matches.
    fn advance(&mut self, mut ticks: u64) {
        let down = self.ctrlb & 1 != 0;
        while ticks > 0 {
            // TOP is re-read each span: an UPDATE at the wrap may change CC0/PER.
            let top = u64::from(self.top());
            let here = u64::from(self.count);
            // Distance to the next wrap; matches are checked inside that span.
            let span = if down {
                here + 1
            } else {
                top.saturating_sub(here) + 1
            };
            let step = ticks.min(span);
            for (i, cc) in self.cc.iter().enumerate() {
                let cc = u64::from(*cc);
                let hit = if down {
                    cc <= here && here - cc < step
                } else {
                    cc > here && cc - here <= step
                };
                if hit {
                    self.intflag |= MC0 << i;
                }
            }
            if step == span {
                self.intflag |= OVF;
                self.ovf_request = true;
                self.count = if down { top as u32 } else { 0 };
                if self.bufv != 0 {
                    self.update();
                    self.buffers_applied = true;
                }
                if self.ctrlb & 4 != 0 {
                    // ONESHOT: stop at the wrap.
                    self.ctrla &= !ENABLE;
                    return;
                }
            } else {
                self.count = if down {
                    (here - step) as u32
                } else {
                    (here + step) as u32
                };
            }
            ticks -= step;
        }
    }
    fn read_reg(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            0x00 => self.ctrla,
            0x04 | 0x05 => u32::from(self.ctrlb),
            0x06 => 0,
            0x08 | 0x09 => u32::from(self.inten),
            0x0a => u32::from(self.intflag),
            0x0b => u32::from(!self.enabled()) | u32::from(self.bufv), // STATUS.STOP, *BUFV
            0x0c => u32::from(self.wave),
            0x0d => u32::from(self.drvctrl),
            0x0f => u32::from(self.dbgctrl),
            0x10 => 0, // SYNCBUSY: writes complete immediately
            0x14 => self.count,
            0x1b if self.mode() == 1 => u32::from(self.per),
            0x1c | 0x1d if self.mode() == 1 => self.cc[(offset - 0x1c) as usize],
            0x1c if self.mode() == 0 => self.cc[0],
            0x1e if self.mode() == 0 => self.cc[1],
            0x2f if self.mode() == 1 => u32::from(self.perbuf),
            0x30 | 0x31 if self.mode() == 1 => self.ccbuf[(offset - 0x30) as usize],
            0x30 if self.mode() == 0 => self.ccbuf[0],
            0x32 if self.mode() == 0 => self.ccbuf[1],
            _ => return Self::refuse("register outside the 8/16-bit slice"),
        })
    }
    fn write_reg(&mut self, offset: u64, value: u32, size: u32) -> SimResult<()> {
        match (offset, size) {
            (0x00, 4) => {
                if value & SWRST != 0 {
                    self.reset();
                    return Ok(());
                }
                let protected = !(ENABLE | SWRST);
                if self.enabled() && (value ^ self.ctrla) & protected != 0 {
                    return Self::refuse("CTRLA is enable-protected");
                }
                // MODE (COUNT8/16), PRESCSYNC, RUNSTDBY, ONDEMAND, PRESCALER; no capture/DMA/ALOCK.
                if value & !0x0000_07ff != 0 || (value >> 2) & 3 > 1 {
                    return Self::refuse("CTRLA capture, lock or 32-bit mode");
                }
                self.ctrla = value;
                self.rate_dirty = true;
            }
            (0x04, 1) | (0x05, 1) => {
                let set = offset == 0x05;
                let flags = (value as u8) & 0x07;
                let command = (value as u8) >> 5;
                if set {
                    self.ctrlb |= flags;
                } else {
                    self.ctrlb &= !flags;
                }
                match command {
                    0 | 4 => {} // NONE, READSYNC: COUNT is always current
                    1 => {
                        // RETRIGGER: restart from the counting direction's origin.
                        self.count = if self.ctrlb & 1 != 0 { self.top() } else { 0 };
                        self.ctrla |= ENABLE;
                    }
                    2 => self.ctrla &= !ENABLE, // STOP
                    _ => return Self::refuse("CTRLB UPDATE/DMAOS command"),
                }
            }
            (0x06, 2) if value == 0 => {}
            (0x0b, 1) => self.bufv &= !(value as u8 & 0x38), // STATUS: clear buffer-valid bits
            (0x08, 1) => self.inten &= !(value as u8),
            (0x09, 1) => self.inten |= (value as u8) & 0x33,
            (0x0a, 1) => self.intflag &= !(value as u8),
            (0x0c, 1) if self.enabled() => return Self::refuse("WAVE is enable-protected"),
            (0x0c, 1) => self.wave = (value as u8) & 3,
            (0x0d, 1) => self.drvctrl = (value as u8) & 3,
            (0x0f, 1) => self.dbgctrl = (value as u8) & 1,
            (0x14, 2) | (0x14, 1) => self.count = value & self.width_mask(),
            (0x1b, 1) if self.mode() == 1 => self.per = value as u8,
            (0x1c, 1) | (0x1d, 1) if self.mode() == 1 => {
                self.cc[(offset - 0x1c) as usize] = value & 0xff
            }
            (0x1c, 2) if self.mode() == 0 => self.cc[0] = value & 0xffff,
            (0x1e, 2) if self.mode() == 0 => self.cc[1] = value & 0xffff,
            (0x2f, 1) if self.mode() == 1 => {
                self.perbuf = value as u8;
                self.bufv |= 0x08;
            }
            (0x30, 1) | (0x31, 1) if self.mode() == 1 => {
                self.ccbuf[(offset - 0x30) as usize] = value & 0xff;
                self.bufv |= 0x10 << (offset - 0x30);
            }
            (0x30, 2) | (0x32, 2) if self.mode() == 0 => {
                let i = ((offset - 0x30) / 2) as usize;
                self.ccbuf[i] = value & 0xffff;
                self.bufv |= 0x10 << i;
            }
            _ => return Self::refuse("store outside the 8/16-bit slice"),
        }
        Ok(())
    }
    fn checked(&mut self, result: SimResult<()>) -> SimResult<()> {
        match &result {
            Err(e) => {
                self.refusal.get_or_insert_with(|| e.to_string());
            }
            Ok(()) => self.refresh_outputs(),
        }
        result
    }
}

impl Samd51Tc {
    /// The overflow DMA request for trigger `source` (this TC's OVF trigger),
    /// consumed on read; `None` for any other source. DMAC wiring is a
    /// later stage; the request is latched so it can be polled.
    pub fn take_dma_trigger(&mut self, source: u32) -> Option<bool> {
        (source == self.instance.dma_ovf).then(|| std::mem::take(&mut self.ovf_request))
    }
}

impl Peripheral for Samd51Tc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let aligned = match offset {
            0x14..=0x17 => 0x14,
            0x1c..=0x1d if self.mode() == 0 => 0x1c,
            0x1e..=0x1f if self.mode() == 0 => 0x1e,
            0x30..=0x31 if self.mode() == 0 => 0x30,
            0x32..=0x33 if self.mode() == 0 => 0x32,
            0x00..=0x03 | 0x10..=0x13 => offset & !3,
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
            // 16-bit COUNT/CC registers stored with a word access keep their low half.
            0x14 | 0x1c | 0x30 if self.mode() == 0 && value >> 16 == 0 => {
                self.write_reg(offset, value, 2)
            }
            _ => self.write_reg(offset, value, 4),
        };
        self.checked(r)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn needs_bus_tick(&self) -> bool {
        self.enabled() && self.rate_dirty
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        let (mask, bit) = self.instance.bus_clock;
        let clocked = bus.read_u32(MCLK + mask).is_ok_and(|m| m & (1 << bit) != 0);
        let tc = clock_tree::peripheral_hz(bus, self.instance.gclk_channel);
        let cpu = clock_tree::cpu_hz(bus);
        self.rate = match (clocked, tc, cpu) {
            (true, Some(tc), Some(cpu)) if cpu > 0 => Some((tc / self.prescaler(), cpu)),
            _ => None,
        };
        self.rate_dirty = false;
        self.refresh_outputs();
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.clock = Some(clock);
    }
    fn timer_output_family(&self) -> Option<&'static str> {
        Some("samd51_tc")
    }
    fn timer_output_lines(
        &mut self,
        arm: bool,
    ) -> Option<std::sync::Arc<crate::peripherals::pad_lines::PadLines>> {
        let lines = self.outputs.lines_arc();
        if arm && !self.wired {
            self.wired = true;
            self.refresh_outputs();
        }
        Some(lines)
    }
    fn wire_lines(&self) -> Option<&crate::peripherals::pad_lines::PadLines> {
        self.outputs.wire_lines()
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn legacy_tick_active(&self) -> bool {
        (self.enabled() && self.rate.is_some()) || self.line()
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        report(self.instance.irq, self.line());
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.elapsed += cycles;
        if self.enabled() && !self.rate_dirty {
            if let Some((tc, cpu)) = self.rate {
                self.carry += cycles * tc;
                let ticks = self.carry / cpu;
                self.carry %= cpu;
                if ticks > 0 {
                    let was_enabled = self.enabled();
                    self.advance(ticks);
                    // Buffered values or a one-shot stop change the waveform.
                    if std::mem::take(&mut self.buffers_applied) || was_enabled != self.enabled() {
                        self.refresh_outputs();
                    }
                }
            }
        }
        if self.wired {
            let now = self.now();
            self.outputs.advance_to(now);
        }
        PeripheralTickResult {
            explicit_irqs: self.line().then(|| vec![self.instance.irq]),
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
        serde_json::json!({"peripheral": "samd51_tc", "base": self.instance.base, "ctrla": self.ctrla,
            "count": self.count, "cc": self.cc, "intflag": self.intflag, "inten": self.inten,
            "rate": self.rate, "refusal": self.refusal, "bufv": self.bufv,
            "outputs": [self.outputs.level(0), self.outputs.level(1)], "wired": self.wired})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(tc: &mut Samd51Tc, rate: (u64, u64)) {
        tc.rate = Some(rate);
        tc.rate_dirty = false;
    }

    #[test]
    fn count16_compare_and_overflow_from_cpu_cycles() {
        let mut tc = Samd51Tc::new(TC_INSTANCES[0]);
        tc.write_u32(0, 3 << 8).unwrap(); // COUNT16, DIV8
        tc.write_u16(0x1c, 100).unwrap();
        tc.write(0x09, MC0).unwrap();
        tc.write_u32(0, 3 << 8 | ENABLE).unwrap();
        running(&mut tc, (1_000_000, 120_000_000));
        tc.tick_elapsed(120 * 99);
        assert_eq!(tc.count(), 99);
        assert!(!tc.line());
        tc.tick_elapsed(120);
        assert_eq!(tc.count(), 100);
        assert!(tc.line());
        tc.write(0x0a, MC0).unwrap();
        assert!(!tc.line());
        tc.tick_elapsed(120 * (0x10000 - 100));
        assert_eq!(tc.count(), 0);
        assert_eq!(tc.read(0x0a).unwrap() & OVF, OVF);
    }

    /// Fraction of cycles in `from..to` that WOx spends high, read off the wire.
    fn high_fraction(tc: &mut Samd51Tc, ch: usize, from: u64, to: u64) -> f64 {
        let lines = tc.timer_output_lines(true).unwrap();
        let mut high = 0u64;
        for t in from..to {
            tc.tick_elapsed(1);
            assert_eq!(tc.now(), t + 1);
            high += u64::from(lines.level(ch));
        }
        high as f64 / (to - from) as f64
    }

    #[test]
    fn mpwm_wo1_follows_the_buffered_duty_from_the_update() {
        let mut tc = Samd51Tc::new(TC_INSTANCES[4]);
        tc.timer_output_lines(true);
        tc.write_u32(0, 0).unwrap(); // COUNT16, DIV1
        tc.write(0x0c, 3).unwrap(); // MPWM
        tc.write_u16(0x30, 99).unwrap(); // CCBUF0: TOP
        tc.write_u16(0x32, 25).unwrap(); // CCBUF1: duty
        assert_eq!(tc.read(0x0b).unwrap() & 0x30, 0x30, "both buffers valid");
        tc.write_u32(0, ENABLE).unwrap();
        running(&mut tc, (1, 1)); // one count per cycle
        tc.refresh_outputs();
        // The first wrap (TOP = CC0 = 0) applies the buffers.
        tc.tick_elapsed(1);
        assert_eq!(tc.cc, [99, 25]);
        assert_eq!(tc.read(0x0b).unwrap() & 0x30, 0);
        let start = tc.now();
        let f = high_fraction(&mut tc, 1, start, start + 1000);
        assert!((f - 0.25).abs() < 0.011, "WO1 duty {f}");
        // WO0 under MPWM is not modelled: it stays low.
        assert!(!tc.outputs.level(0));
        // INVEN1 inverts WO1.
        tc.write(0x0d, 2).unwrap();
        let start = tc.now();
        let f = high_fraction(&mut tc, 1, start, start + 1000);
        assert!((f - 0.75).abs() < 0.011, "inverted WO1 duty {f}");
    }

    #[test]
    fn a_stopped_counter_holds_its_level() {
        let mut tc = Samd51Tc::new(TC_INSTANCES[4]);
        tc.timer_output_lines(true);
        tc.write(0x0c, 2).unwrap(); // NPWM: TOP = MAX
        tc.write_u16(0x1c, 0x8000).unwrap();
        // Disabled at count 0 < CC0: WO0 high, and it stays high.
        let start = tc.now();
        assert_eq!(high_fraction(&mut tc, 0, start, start + 100), 1.0);
    }

    #[test]
    fn stopped_without_a_resolved_clock_and_refusals() {
        let mut tc = Samd51Tc::new(TC_INSTANCES[2]);
        tc.write_u32(0, ENABLE).unwrap();
        tc.tick_elapsed(1_000_000);
        assert_eq!(tc.count(), 0, "no clock resolved: the counter does not run");
        assert!(
            tc.write_u32(0, ENABLE | 3 << 8).is_err(),
            "enable-protected prescaler"
        );
        assert!(tc.write_u32(0, 2 << 2).is_err(), "32-bit pairing");
        assert!(tc.write_u16(0x06, 1).is_err(), "events");
        tc.write_u32(0, SWRST).unwrap();
        assert_eq!(tc.read_u32(0).unwrap(), 0);
    }
}
