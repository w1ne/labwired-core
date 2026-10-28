// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Opt-in SAMD51 TCC slice (DS60001507F 49): an up/down counter at the
//! prescaled generic clock resolved from the live clock tree, PER top, compare
//! matches on CC0..CCn, the CTRLB commands (RETRIGGER, STOP, READSYNC) and the
//! SAMD51 interrupt layout: line 0 carries OVF/TRG/CNT/ERR/UFS/DFS/faults and
//! line 1 + x carries MCx. Waveform outputs, buffered updates (CCBUF/PERBUF),
//! dithering, faults, capture, patterns, events and DMA are refused or not
//! modelled.
use super::clock_tree;
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};

const MCLK: u64 = 0x40000800;
const SWRST: u32 = 1;
const ENABLE: u32 = 1 << 1;
const OVF: u32 = 1;
const MC0: u32 = 1 << 16;

#[derive(Clone, Copy, Debug)]
pub struct TccInstance {
    pub base: u64,
    /// First NVIC line (`TCCn_0_IRQn`); MCx follow on `irq + 1 + x`.
    pub irq: u32,
    pub channels: usize,
    /// Counter width in bits.
    pub width: u32,
    pub gclk_channel: u64,
    /// (MCLK mask register offset, bit)
    pub bus_clock: (u64, u32),
}

/// TCC0..TCC4 on the SAMD51J19A (ATDF instance parameters).
pub const TCC_INSTANCES: [TccInstance; 5] = [
    TccInstance {
        base: 0x41016000,
        irq: 85,
        channels: 6,
        width: 24,
        gclk_channel: 25,
        bus_clock: (0x18, 11),
    },
    TccInstance {
        base: 0x41018000,
        irq: 92,
        channels: 4,
        width: 24,
        gclk_channel: 25,
        bus_clock: (0x18, 12),
    },
    TccInstance {
        base: 0x42000c00,
        irq: 97,
        channels: 3,
        width: 16,
        gclk_channel: 29,
        bus_clock: (0x1c, 3),
    },
    TccInstance {
        base: 0x42001000,
        irq: 101,
        channels: 2,
        width: 16,
        gclk_channel: 29,
        bus_clock: (0x1c, 4),
    },
    TccInstance {
        base: 0x43001000,
        irq: 104,
        channels: 2,
        width: 16,
        gclk_channel: 38,
        bus_clock: (0x20, 4),
    },
];

#[derive(Debug)]
pub struct Samd51Tcc {
    instance: TccInstance,
    ctrla: u32,
    ctrlb: u8,
    inten: u32,
    intflag: u32,
    count: u32,
    per: u32,
    cc: [u32; 6],
    wave: u32,
    rate: Option<(u64, u64)>,
    rate_dirty: bool,
    carry: u64,
    refusal: Option<String>,
}

impl Samd51Tcc {
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
            rate: None,
            rate_dirty: true,
            carry: 0,
            refusal: None,
        }
    }
    fn mask(width: u32) -> u32 {
        (1u32 << width) - 1
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd51 TCC slice: {what}"
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
    /// Lines asserted now: 0 for the non-compare flags, 1 + x for MCx.
    fn pending(&self) -> Vec<u32> {
        let active = self.intflag & self.inten;
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
                self.count = if down { top as u32 } else { 0 };
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
        Ok(match offset {
            0x00 => self.ctrla,
            0x04 | 0x05 => u32::from(self.ctrlb),
            0x08 => 0, // SYNCBUSY: writes complete immediately
            0x0c | 0x10 | 0x14 | 0x18 | 0x1e | 0x20 | 0x38 => 0,
            0x24 | 0x28 => self.inten,
            0x2c => self.intflag,
            0x30 => u32::from(!self.enabled()) << 1, // STATUS.STOP
            0x34 => self.count,
            0x3c => self.wave,
            0x40 => self.per,
            0x44..=0x58 if x(offset) < self.instance.channels && offset % 4 == 0 => {
                self.cc[x(offset)]
            }
            _ => return Self::refuse("register"),
        })
    }
    fn write_reg(&mut self, offset: u64, value: u32, size: u32) -> SimResult<()> {
        let max = Self::mask(self.instance.width);
        let x = ((offset.wrapping_sub(0x44)) / 4) as usize;
        match (offset, size) {
            (0x00, 4) => {
                if value & SWRST != 0 {
                    *self = Self::new(self.instance);
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
                    _ => return Self::refuse("CTRLB UPDATE/DMAOS command"),
                }
            }
            (0x0c | 0x10 | 0x14 | 0x18 | 0x20 | 0x38, _) if value == 0 => {}
            (0x1e, 1) if value & !1 == 0 => {}
            (0x24, 4) => self.inten &= !value,
            (0x28, 4) => self.inten |= value & self.interrupt_mask(),
            (0x2c, 4) => self.intflag &= !value,
            (0x30, 4) => {}
            (0x34, 4) => self.count = value & max,
            (0x3c, 4) if self.enabled() => return Self::refuse("WAVE is enable-protected"),
            // NFRQ only: waveform generation is not modelled.
            (0x3c, 4) if value == 0 => self.wave = 0,
            (0x40, 4) => self.per = value & max,
            (0x44..=0x58, 4) if x < self.instance.channels && offset % 4 == 0 => {
                self.cc[x] = value & max
            }
            _ => return Self::refuse("store outside the counter slice"),
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

impl Peripheral for Samd51Tcc {
    fn on_system_reset(&mut self) {
        *self = Self::new(self.instance);
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
        let clocked = bus.read_u32(MCLK + mask).is_ok_and(|m| m & (1 << bit) != 0);
        let tcc = clock_tree::peripheral_hz(bus, self.instance.gclk_channel);
        let cpu = clock_tree::cpu_hz(bus);
        self.rate = match (clocked, tcc, cpu) {
            (true, Some(tcc), Some(cpu)) if cpu > 0 => Some((tcc / self.prescaler(), cpu)),
            _ => None,
        };
        self.rate_dirty = false;
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn legacy_tick_active(&self) -> bool {
        (self.enabled() && self.rate.is_some()) || !self.pending().is_empty()
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        let pending = self.pending();
        for line in 0..=self.instance.channels as u32 {
            let irq = self.instance.irq + line;
            report(irq, pending.contains(&irq));
        }
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        if self.enabled() && !self.rate_dirty {
            if let Some((tcc, cpu)) = self.rate {
                self.carry += cycles * tcc;
                let ticks = self.carry / cpu;
                self.carry %= cpu;
                if ticks > 0 {
                    self.advance(ticks);
                }
            }
        }
        let pending = self.pending();
        PeripheralTickResult {
            explicit_irqs: (!pending.is_empty()).then_some(pending),
            ..Default::default()
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "samd51_tcc", "base": self.instance.base, "ctrla": self.ctrla,
            "count": self.count, "per": self.per, "cc": self.cc, "inten": self.inten, "intflag": self.intflag,
            "rate": self.rate, "refusal": self.refusal})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_match_raises_its_own_line() {
        let mut t = Samd51Tcc::new(TCC_INSTANCES[0]);
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
        t.tick_elapsed(u64::from(Samd51Tcc::mask(24)));
        assert_eq!(t.pending(), vec![85], "OVF on TCC0_0");
    }

    #[test]
    fn dithering_and_waveforms_are_refused() {
        let mut t = Samd51Tcc::new(TCC_INSTANCES[2]);
        assert!(t.write_u32(0, 1 << 5).is_err(), "RESOLUTION");
        assert!(t.write_u32(0x3c, 2).is_err(), "NPWM");
        assert!(
            t.write_u32(0x44 + 4 * 3, 1).is_err(),
            "TCC2 has three channels"
        );
        assert_eq!(t.read_u32(0x40).unwrap(), 0xffff, "16-bit PER");
    }
}
