// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Opt-in SAM TCC slice, SAMD51 (DS60001507F 49) and SAMD21 (DS40001882 31;
//! the register map is the same): an up/down counter at the prescaled generic
//! clock, PER top, compare matches on CC0..CCn, the CTRLB commands
//! (RETRIGGER, STOP, UPDATE, READSYNC) and buffered PERBUF/CCBUF (SAMD21
//! PERB/CCB) copied at the UPDATE condition (counter wrap) unless CTRLB.LUPD
//! holds them. Interrupts follow the instance: the SAMD51 has line 0 for
//! OVF/TRG/CNT/ERR/UFS/DFS/faults and line 1 + x for MCx, the SAMD21 one line
//! per instance for all of them.
//!
//! The SAMD51 rate comes from the live clock tree. The SAMD21 GCLK.CLKCTRL is
//! an indirect register declared as a plain bank, so on the SAMD21 the generic
//! clock is not checked: the TCC counts at the CPU clock (GCLK0, which is what
//! ArduinoCore-samd's analogWrite selects) over the prescaler, and only while
//! its PM.APBCMASK bit is set.
//!
//! Waveform outputs WO[x] (driven by CC[x % channels], WEXCTRL.OTMX = 0) are
//! published edge-exact onto pads routed to them (the bus routes the SAMD21
//! instances' PMUX E/F pads; SAMD51 TCC pads are not routed yet), for
//! up-counting single-slope
//! NPWM: WO high from BOTTOM until the CC match, WAVE.POLx and DRVCTRL.INVENx
//! invert. NFRQ and down-counting outputs are not modelled and drive low (or
//! high, inverted). MFRQ, dual-slope, dithering, faults, capture, output
//! matrix, dead time, patterns, events and DMA are refused or not modelled.
use super::clock_tree;
use crate::peripherals::esp_ledc_wave::{LedcDrive, LedcPads, LedcTimebase};
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;

/// Line names of a TCC's waveform-output wire, in WO order; an instance with
/// fewer outputs uses the first `outputs` of them.
pub const TCC_OUTPUT_LINES: [&str; 8] = ["WO0", "WO1", "WO2", "WO3", "WO4", "WO5", "WO6", "WO7"];

const MCLK: u64 = 0x40000800;
/// SAMD21 PM.APBCMASK (CMSIS `pm.h`); TCC0..2 are bits 8..10.
const SAMD21_PM_APBCMASK: u64 = 0x40000420;
const SWRST: u32 = 1;
const ENABLE: u32 = 1 << 1;
const OVF: u32 = 1;
const MC0: u32 = 1 << 16;
const LUPD: u8 = 1 << 1;
/// STATUS.PERBUFV; CCBUFVx is bit 16 + x.
const PERBUFV: u32 = 1 << 7;
const NPWM: u32 = 2;

#[derive(Clone, Copy, Debug)]
pub struct TccInstance {
    pub base: u64,
    /// First NVIC line (`TCCn_0_IRQn`); on the SAMD51 MCx follow on
    /// `irq + 1 + x`, on the SAMD21 (`single_irq`) everything shares it.
    pub irq: u32,
    pub single_irq: bool,
    pub channels: usize,
    /// Waveform outputs WO0..WO(n-1).
    pub outputs: usize,
    /// Counter width in bits.
    pub width: u32,
    /// GCLK peripheral channel; `None` on the SAMD21 (counts at the CPU clock).
    pub gclk_channel: Option<u64>,
    /// (APB mask register address, bit)
    pub bus_clock: (u64, u32),
}

/// TCC0..TCC4 on the SAMD51J19A (ATDF instance parameters).
pub const TCC_INSTANCES: [TccInstance; 5] = [
    TccInstance {
        base: 0x41016000,
        irq: 85,
        single_irq: false,
        channels: 6,
        outputs: 8,
        width: 24,
        gclk_channel: Some(25),
        bus_clock: (MCLK + 0x18, 11),
    },
    TccInstance {
        base: 0x41018000,
        irq: 92,
        single_irq: false,
        channels: 4,
        outputs: 8,
        width: 24,
        gclk_channel: Some(25),
        bus_clock: (MCLK + 0x18, 12),
    },
    TccInstance {
        base: 0x42000c00,
        irq: 97,
        single_irq: false,
        channels: 3,
        outputs: 3,
        width: 16,
        gclk_channel: Some(29),
        bus_clock: (MCLK + 0x1c, 3),
    },
    TccInstance {
        base: 0x42001000,
        irq: 101,
        single_irq: false,
        channels: 2,
        outputs: 2,
        width: 16,
        gclk_channel: Some(29),
        bus_clock: (MCLK + 0x1c, 4),
    },
    TccInstance {
        base: 0x43001000,
        irq: 104,
        single_irq: false,
        channels: 2,
        outputs: 2,
        width: 16,
        gclk_channel: Some(38),
        bus_clock: (MCLK + 0x20, 4),
    },
];

/// TCC0..TCC2 on the SAMD21 (CMSIS `samd21g18a.h`, `instance/tccN.h`).
pub const SAMD21_TCC_INSTANCES: [TccInstance; 3] = [
    TccInstance {
        base: 0x42002000,
        irq: 15,
        single_irq: true,
        channels: 4,
        outputs: 8,
        width: 24,
        gclk_channel: None,
        bus_clock: (SAMD21_PM_APBCMASK, 8),
    },
    TccInstance {
        base: 0x42002400,
        irq: 16,
        single_irq: true,
        channels: 2,
        outputs: 4,
        width: 24,
        gclk_channel: None,
        bus_clock: (SAMD21_PM_APBCMASK, 9),
    },
    TccInstance {
        base: 0x42002800,
        irq: 17,
        single_irq: true,
        channels: 2,
        outputs: 2,
        width: 16,
        gclk_channel: None,
        bus_clock: (SAMD21_PM_APBCMASK, 10),
    },
];

#[derive(Debug)]
pub struct SamTcc {
    instance: TccInstance,
    ctrla: u32,
    ctrlb: u8,
    inten: u32,
    intflag: u32,
    count: u32,
    per: u32,
    cc: [u32; 6],
    wave: u32,
    drvctrl: u32,
    /// PERBUF/CCBUF and their valid bits (STATUS layout: PERBUFV, CCBUFVx).
    perbuf: u32,
    ccbuf: [u32; 6],
    bufv: u32,
    rate: Option<(u64, u64)>,
    rate_dirty: bool,
    carry: u64,
    refusal: Option<String>,
    /// Waveform outputs; generated only once the bus routed a pad to them.
    outputs: LedcPads,
    wired: bool,
    /// A wrap applied buffered values; the outputs follow on this tick.
    buffers_applied: bool,
    clock: Option<crate::CycleClock>,
    /// Cycles seen by `tick_elapsed`: the time base without a cycle clock.
    elapsed: u64,
}

impl SamTcc {
    pub fn new(instance: TccInstance) -> Self {
        let max = Self::mask(instance.width);
        Self {
            instance,
            ctrla: 0,
            ctrlb: 0,
            inten: 0,
            intflag: 0,
            count: 0,
            per: max,
            cc: [0; 6],
            wave: 0,
            drvctrl: 0,
            perbuf: max,
            ccbuf: [0; 6],
            bufv: 0,
            rate: None,
            rate_dirty: true,
            carry: 0,
            refusal: None,
            outputs: LedcPads::new(&TCC_OUTPUT_LINES[..instance.outputs]),
            wired: false,
            buffers_applied: false,
            clock: None,
            elapsed: 0,
        }
    }
    fn reset(&mut self) {
        // SWRST resets registers; the output wire and its routing survive.
        let outputs = std::mem::replace(&mut self.outputs, LedcPads::new(&[]));
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
    /// UPDATE condition: valid buffers take effect.
    fn update(&mut self) {
        for x in 0..self.instance.channels {
            if self.bufv & (1 << (16 + x)) != 0 {
                self.cc[x] = self.ccbuf[x];
            }
        }
        if self.bufv & PERBUFV != 0 {
            self.per = self.perbuf;
        }
        if self.bufv != 0 {
            self.buffers_applied = true;
        }
        self.bufv = 0;
    }
    /// What WO[wo] drives now, from the registers and the resolved count rate.
    fn output_drive(&self, wo: usize) -> LedcDrive {
        let ch = wo % self.instance.channels;
        let invert = ((self.wave >> (16 + ch)) & 1 != 0) ^ ((self.drvctrl >> (16 + wo)) & 1 != 0);
        if self.wave & 7 != NPWM || self.ctrlb & 1 != 0 {
            return LedcDrive::Level(invert); // not modelled: low (inverted: high)
        }
        let period = u64::from(self.per) + 1;
        let duty = u64::from(self.cc[ch]).min(period);
        // Inverted: low from BOTTOM until the match, high for the rest.
        let (hpoint, duty) = if invert {
            (duty, period - duty)
        } else {
            (0, duty)
        };
        match (self.enabled(), self.rate) {
            (true, Some((tcc, cpu))) if tcc > 0 => {
                let now = self.now();
                // The counter's BOTTOM for the period in progress.
                let into = (u128::from(self.count) * u128::from(cpu) + u128::from(self.carry))
                    / u128::from(tcc);
                let origin = now.saturating_sub(into.min(u128::from(u64::MAX)) as u64);
                LedcDrive::Pwm {
                    timer: LedcTimebase {
                        period,
                        num: u128::from(cpu),
                        den: u128::from(tcc),
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
        for wo in 0..self.instance.outputs {
            let drive = self.output_drive(wo);
            self.outputs.set_drive(wo, drive, now);
        }
    }
    fn mask(width: u32) -> u32 {
        (1u32 << width) - 1
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "SAM TCC slice: {what}"
        )))
    }
    fn enabled(&self) -> bool {
        self.ctrla & ENABLE != 0
    }
    fn prescaler(&self) -> u64 {
        [1, 2, 4, 8, 16, 64, 256, 1024][((self.ctrla >> 8) & 7) as usize]
    }
    fn interrupt_mask(&self) -> u32 {
        // OVF..DFS and the fault flags, plus MC0..MC(n-1).
        0xfc0f | (((1 << self.instance.channels) - 1) << 16)
    }
    /// Lines asserted now: 0 for the non-compare flags, 1 + x for MCx (one
    /// line for everything on a `single_irq` instance).
    fn pending(&self) -> Vec<u32> {
        let active = self.intflag & self.inten;
        if self.instance.single_irq {
            return if active != 0 {
                vec![self.instance.irq]
            } else {
                Vec::new()
            };
        }
        let mut lines = Vec::new();
        if active & 0xffff != 0 {
            lines.push(self.instance.irq);
        }
        for x in 0..self.instance.channels {
            if active & (MC0 << x) != 0 {
                lines.push(self.instance.irq + 1 + x as u32);
            }
        }
        lines
    }
    fn advance(&mut self, mut ticks: u64) {
        let down = self.ctrlb & 1 != 0;
        let channels = self.instance.channels;
        while ticks > 0 {
            let top = u64::from(self.per);
            let here = u64::from(self.count);
            let span = if down {
                here + 1
            } else {
                top.saturating_sub(here) + 1
            };
            let step = ticks.min(span);
            for x in 0..channels {
                let cc = u64::from(self.cc[x]);
                let hit = if down {
                    cc <= here && here - cc < step
                } else {
                    cc > here && cc - here <= step
                };
                if hit {
                    self.intflag |= MC0 << x;
                }
            }
            if step == span {
                self.intflag |= OVF;
                if self.ctrlb & LUPD == 0 {
                    self.update();
                }
                self.count = if down { self.per } else { 0 };
                if self.ctrlb & 4 != 0 {
                    self.ctrla &= !ENABLE; // ONESHOT
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
        let x = |o: u64| ((o - 0x44) / 4) as usize;
        let xb = |o: u64| ((o - 0x70) / 4) as usize;
        Ok(match offset {
            0x00 => self.ctrla,
            0x04 | 0x05 => u32::from(self.ctrlb),
            0x08 => 0, // SYNCBUSY: writes complete immediately
            0x0c | 0x10 | 0x14 | 0x1e | 0x20 | 0x38 => 0,
            0x18 => self.drvctrl,
            0x24 | 0x28 => self.inten,
            0x2c => self.intflag,
            0x30 => (u32::from(!self.enabled()) << 1) | self.bufv, // STOP, buffer-valid bits
            0x34 => self.count,
            0x3c => self.wave,
            0x40 => self.per,
            0x44..=0x58 if x(offset) < self.instance.channels && offset % 4 == 0 => {
                self.cc[x(offset)]
            }
            0x6c => self.perbuf,
            0x70..=0x84 if xb(offset) < self.instance.channels && offset % 4 == 0 => {
                self.ccbuf[xb(offset)]
            }
            _ => return Self::refuse("register"),
        })
    }
    fn write_reg(&mut self, offset: u64, value: u32, size: u32) -> SimResult<()> {
        let max = Self::mask(self.instance.width);
        let x = ((offset.wrapping_sub(0x44)) / 4) as usize;
        let xb = ((offset.wrapping_sub(0x70)) / 4) as usize;
        match (offset, size) {
            (0x00, 4) => {
                if value & SWRST != 0 {
                    self.reset();
                    return Ok(());
                }
                // RESOLUTION (dithering), ALOCK, MSYNC, DMAOS and capture refused.
                if value & !0x3f0a != 0 || (value >> 5) & 3 != 0 {
                    return Self::refuse("CTRLA dithering, capture, lock or DMA");
                }
                if self.enabled() && (value ^ self.ctrla) & !ENABLE != 0 {
                    return Self::refuse("CTRLA is enable-protected");
                }
                self.ctrla = value;
                self.rate_dirty = true;
            }
            (0x04, 1) | (0x05, 1) => {
                let flags = (value as u8) & 0x07;
                if offset == 0x05 {
                    self.ctrlb |= flags;
                } else {
                    self.ctrlb &= !flags;
                }
                match (value as u8) >> 5 {
                    0 | 4 => {} // NONE, READSYNC: COUNT is always current
                    1 => {
                        self.count = if self.ctrlb & 1 != 0 { self.per } else { 0 };
                        self.ctrla |= ENABLE;
                    }
                    2 => self.ctrla &= !ENABLE,
                    3 => self.update(),
                    _ => return Self::refuse("CTRLB DMAOS command"),
                }
            }
            (0x0c | 0x10 | 0x14 | 0x20 | 0x38, _) if value == 0 => {}
            // DRVCTRL INVENx only: non-recoverable fault states and filters refused.
            (0x18, 4) if self.enabled() => return Self::refuse("DRVCTRL is enable-protected"),
            (0x18, 4) if value & !0x00ff_0000 == 0 => self.drvctrl = value,
            (0x1e, 1) if value & !1 == 0 => {}
            (0x24, 4) => self.inten &= !value,
            (0x28, 4) => self.inten |= value & self.interrupt_mask(),
            (0x2c, 4) => self.intflag &= !value,
            (0x30, 4) => {}
            (0x34, 4) => self.count = value & max,
            (0x3c, 4) if self.enabled() => return Self::refuse("WAVE is enable-protected"),
            // WAVEGEN NFRQ or NPWM and the POLx bits; RAMP2, dual-slope, MFRQ,
            // CIPEREN/CICCEN and SWAP are refused.
            (0x3c, 4) if (value & 7 == 0 || value & 7 == NPWM) && value & !0x000f_0007 == 0 => {
                self.wave = value
            }
            (0x40, 4) => self.per = value & max,
            (0x44..=0x58, 4) if x < self.instance.channels && offset % 4 == 0 => {
                self.cc[x] = value & max
            }
            (0x6c, 4) => {
                self.perbuf = value & max;
                self.bufv |= PERBUFV;
            }
            (0x70..=0x84, 4) if xb < self.instance.channels && offset % 4 == 0 => {
                self.ccbuf[xb] = value & max;
                self.bufv |= 1 << (16 + xb);
            }
            _ => return Self::refuse("store outside the counter slice"),
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

impl Peripheral for SamTcc {
    fn on_system_reset(&mut self) {
        self.reset();
        self.refresh_outputs();
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let aligned = match offset {
            0x04 | 0x05 | 0x1e => offset,
            _ => offset & !3,
        };
        Ok((self.read_reg(aligned)? >> (8 * (offset - aligned))) as u8)
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.read_reg(offset)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let r = match offset {
            0x04 | 0x05 | 0x1e => self.write_reg(offset, u32::from(value), 1),
            // A byte lane of a 32-bit register: read-modify-write the word.
            _ => {
                let aligned = offset & !3;
                let shift = 8 * (offset - aligned);
                match self.read_reg(aligned) {
                    Ok(word) => {
                        let word = (word & !(0xff << shift)) | (u32::from(value) << shift);
                        // W1C/W1S registers take only the written lane.
                        let word = if matches!(aligned, 0x24 | 0x28 | 0x2c) {
                            u32::from(value) << shift
                        } else {
                            word
                        };
                        self.write_reg(aligned, word, 4)
                    }
                    Err(e) => Err(e),
                }
            }
        };
        self.checked(r)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let r = self.write_reg(offset, value, 4);
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
        let clocked = bus.read_u32(mask).is_ok_and(|m| m & (1 << bit) != 0);
        self.rate = match self.instance.gclk_channel {
            Some(channel) => {
                let tcc = clock_tree::peripheral_hz(bus, channel);
                let cpu = clock_tree::cpu_hz(bus);
                match (clocked, tcc, cpu) {
                    (true, Some(tcc), Some(cpu)) if cpu > 0 => Some((tcc / self.prescaler(), cpu)),
                    _ => None,
                }
            }
            // SAMD21: the generic clock is GCLK0, the CPU clock.
            None => clocked.then(|| (1, self.prescaler())),
        };
        self.rate_dirty = false;
        self.refresh_outputs();
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.clock = Some(clock);
    }
    fn timer_output_family(&self) -> Option<&'static str> {
        Some(if self.instance.gclk_channel.is_none() {
            "samd21_tcc"
        } else {
            "samd51_tcc"
        })
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
        (self.enabled() && self.rate.is_some()) || !self.pending().is_empty()
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        let pending = self.pending();
        let lines = if self.instance.single_irq {
            0
        } else {
            self.instance.channels as u32
        };
        for line in 0..=lines {
            let irq = self.instance.irq + line;
            report(irq, pending.contains(&irq));
        }
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.elapsed += cycles;
        if self.enabled() && !self.rate_dirty {
            if let Some((tcc, cpu)) = self.rate {
                self.carry += cycles * tcc;
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
        let pending = self.pending();
        PeripheralTickResult {
            explicit_irqs: (!pending.is_empty()).then_some(pending),
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
        let outputs: Vec<bool> = (0..self.instance.outputs)
            .map(|w| self.outputs.level(w))
            .collect();
        serde_json::json!({"peripheral": self.timer_output_family(), "base": self.instance.base,
            "ctrla": self.ctrla, "count": self.count, "per": self.per, "cc": self.cc,
            "wave": self.wave, "inten": self.inten, "intflag": self.intflag, "bufv": self.bufv,
            "rate": self.rate, "refusal": self.refusal, "outputs": outputs, "wired": self.wired})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_match_raises_its_own_line() {
        let mut t = SamTcc::new(TCC_INSTANCES[0]);
        t.write_u32(0x44 + 4, 50).unwrap(); // CC1
        t.write_u32(0x28, MC0 << 1).unwrap();
        t.write_u32(0, ENABLE).unwrap();
        t.rate = Some((1, 1));
        t.rate_dirty = false;
        t.tick_elapsed(49);
        assert!(t.pending().is_empty());
        t.tick_elapsed(1);
        assert_eq!(t.pending(), vec![85 + 2], "MC1 on TCC0_2");
        t.write_u32(0x2c, MC0 << 1).unwrap();
        assert!(t.pending().is_empty());
        t.write_u32(0x28, OVF).unwrap();
        t.tick_elapsed(u64::from(SamTcc::mask(24)));
        assert_eq!(t.pending(), vec![85], "OVF on TCC0_0");
    }

    #[test]
    fn dithering_and_waveforms_are_refused() {
        let mut t = SamTcc::new(TCC_INSTANCES[2]);
        assert!(t.write_u32(0, 1 << 5).is_err(), "RESOLUTION");
        assert!(t.write_u32(0x3c, 1).is_err(), "MFRQ");
        assert!(t.write_u32(0x3c, 4).is_err(), "DSCRITICAL");
        assert!(
            t.write_u32(0x44 + 4 * 3, 1).is_err(),
            "TCC2 has three channels"
        );
        assert_eq!(t.read_u32(0x40).unwrap(), 0xffff, "16-bit PER");
    }

    fn high_fraction(t: &mut SamTcc, wo: usize, from: u64, to: u64) -> f64 {
        let lines = t.timer_output_lines(true).unwrap();
        let mut high = 0u64;
        for _ in from..to {
            t.tick_elapsed(1);
            high += u64::from(lines.level(wo));
        }
        high as f64 / (to - from) as f64
    }

    /// The register sequence of ArduinoCore-samd analogWrite on a TCC pin.
    fn arduino_analog_write(t: &mut SamTcc, ch: usize, value: u32, per: u32) {
        t.write_u32(0, 0).unwrap();
        t.write_u32(0x3c, NPWM).unwrap();
        t.write_u32(0x44 + 4 * ch as u64, value).unwrap();
        t.write_u32(0x40, per).unwrap();
        t.write_u32(0, ENABLE).unwrap();
        t.rate = Some((1, 1)); // one count per cycle
        t.rate_dirty = false;
        t.refresh_outputs();
    }

    #[test]
    fn samd21_npwm_drives_wo_high_until_the_match() {
        let mut t = SamTcc::new(SAMD21_TCC_INSTANCES[2]);
        assert_eq!(t.timer_output_family(), Some("samd21_tcc"));
        t.timer_output_lines(true);
        arduino_analog_write(&mut t, 1, 25, 99);
        let start = t.now();
        let f = high_fraction(&mut t, 1, start, start + 1000);
        assert!((f - 0.25).abs() < 0.011, "WO1 duty {f}");
        // WO0's CC0 is 0: low all period.
        assert!(!t.outputs.level(0));
        // POL1 inverts it (WAVE is enable-protected: disable first).
        t.write_u32(0, 0).unwrap();
        t.write_u32(0x3c, NPWM | 1 << 17).unwrap();
        t.write_u32(0, ENABLE).unwrap();
        let start = t.now();
        let f = high_fraction(&mut t, 1, start, start + 1000);
        assert!((f - 0.75).abs() < 0.011, "POL1 WO1 duty {f}");
    }

    #[test]
    fn tcc0_wo_beyond_the_channels_follows_cc_modulo() {
        let mut t = SamTcc::new(SAMD21_TCC_INSTANCES[0]);
        t.timer_output_lines(true);
        arduino_analog_write(&mut t, 3, 50, 99);
        let start = t.now();
        // WO7 = CC3 (WEXCTRL.OTMX = 0).
        let f = high_fraction(&mut t, 7, start, start + 1000);
        assert!((f - 0.5).abs() < 0.011, "WO7 duty {f}");
    }

    #[test]
    fn ccb_waits_for_the_wrap_and_lupd_holds_it() {
        let mut t = SamTcc::new(SAMD21_TCC_INSTANCES[2]);
        t.timer_output_lines(true);
        arduino_analog_write(&mut t, 1, 25, 99);
        // Second analogWrite: CTRLBSET.LUPD, CCB1, CTRLBCLR.LUPD.
        t.write(0x05, LUPD).unwrap();
        t.write_u32(0x74, 75).unwrap();
        assert_eq!(t.read_u32(0x30).unwrap() & (1 << 17), 1 << 17, "CCBUFV1");
        t.tick_elapsed(300);
        assert_eq!(t.cc[1], 25, "LUPD holds the buffer across wraps");
        t.write(0x04, LUPD).unwrap();
        t.tick_elapsed(100);
        assert_eq!(t.cc[1], 75, "applied at the next wrap");
        assert_eq!(t.read_u32(0x30).unwrap() & (1 << 17), 0);
        let start = t.now();
        let f = high_fraction(&mut t, 1, start, start + 1000);
        assert!((f - 0.75).abs() < 0.011, "WO1 duty {f}");
    }

    #[test]
    fn samd21_raises_every_flag_on_one_line() {
        let mut t = SamTcc::new(SAMD21_TCC_INSTANCES[1]);
        t.write_u32(0x44 + 4, 10).unwrap();
        t.write_u32(0x28, MC0 << 1 | OVF).unwrap();
        t.write_u32(0x40, 99).unwrap();
        t.write_u32(0, ENABLE).unwrap();
        t.rate = Some((1, 1));
        t.rate_dirty = false;
        t.tick_elapsed(10);
        assert_eq!(t.pending(), vec![16], "MC1 on TCC1_IRQn");
        t.write_u32(0x2c, MC0 << 1).unwrap();
        t.tick_elapsed(90);
        assert_eq!(t.pending(), vec![16], "OVF on the same line");
        let mut lines = Vec::new();
        t.irq_line_levels(&mut |irq, level| lines.push((irq, level)));
        assert_eq!(lines, vec![(16, true)]);
    }

    #[test]
    fn samd21_counts_only_with_its_apb_clock() {
        use crate::bus::SystemBus;
        let mut t = SamTcc::new(SAMD21_TCC_INSTANCES[2]);
        t.write_u32(0, ENABLE | 2 << 8).unwrap(); // DIV4
        let mut bus = SystemBus::new();
        t.tick_with_bus(&mut bus);
        assert_eq!(t.rate, None, "PM.APBCMASK unreadable: no clock");
    }
}
