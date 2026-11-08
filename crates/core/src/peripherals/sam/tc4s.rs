// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! SAM4S TC block (SAM4S datasheet "Timer Counter (TC)"): one instance is a
//! block of three 16-bit channels (TC0..TC2 at 0x40010000, TC3..TC5 at
//! 0x40014000), the shape Renode's `SAM_TC` maps.
//!
//! Per channel: CLKEN/CLKDIS/SWTRG (and the block's BCR.SYNC), the clock
//! enable (SR.CLKSTA) kept apart from the started/stopped state — a trigger
//! resets CV and starts the clock, but only an enabled clock counts —,
//! TIMER_CLOCK1..4 (MCK/2, /8, /32, /128, where MCK is the CPU clock) and
//! TIMER_CLOCK5 (SLCK, 32768 Hz scaled off the attached CPU clock), RA/RB/RC
//! compares, COVFS on a 0xFFFF wrap, and the read-to-clear SR flags behind a
//! level IRQ per channel (IER/IDR/IMR).
//!
//! Modes: waveform WAVSEL UP and UP_RC (with CPCSTOP/CPCDIS), and capture
//! mode used as a plain counter with or without CPCTRG. A CPCSTOP stop in
//! UP_RC holds CV at RC (not checked against silicon).
//!
//! Not modelled, refused on write: the external clocks XC0..XC2, BURST
//! gating, capture loads (LDRA/LDRB), external triggers (ETRGEDG) and
//! waveform external events (EEVTEDG), the UPDOWN waveforms, the stepper
//! mode (SMMR), quadrature decoding (BMR QDEC/QIER), the fault mode and
//! write protection. The TIOA/TIOB output actions are stored but no pad is
//! driven, so SR.MTIOA/MTIOB read 0. PMC_PCSR0 gates each channel: with its
//! peripheral clock off the channel does not count (register stores are
//! still accepted, which silicon would drop).
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;
use std::cell::Cell;

const PMC_PCSR0: u64 = 0x400e_0418;
const SLCK_HZ: u128 = 32_768;

const CCR_CLKEN: u32 = 1;
const CCR_CLKDIS: u32 = 1 << 1;
const CCR_SWTRG: u32 = 1 << 2;

const SR_COVFS: u8 = 1;
const SR_CPAS: u8 = 1 << 2;
const SR_CPBS: u8 = 1 << 3;
const SR_CPCS: u8 = 1 << 4;
const SR_CLKSTA: u32 = 1 << 16;
/// IER bits: COVFS, LOVRS, CPAS, CPBS, CPCS, LDRAS, LDRBS, ETRGS.
const INT_MASK: u8 = 0xff;

const CMR_WAVE: u32 = 1 << 15;
const CMR_CPCSTOP: u32 = 1 << 6;
const CMR_CPCDIS: u32 = 1 << 7;
const CMR_CPCTRG: u32 = 1 << 14;

#[derive(Clone, Copy, Debug)]
pub struct Sam4sTcBlock {
    pub base: u64,
    /// IRQ (and PMC peripheral id) of channel 0; channels 1/2 follow.
    pub first_irq: u32,
}

/// SAM4S "Peripheral Identifiers": TC0..TC5 are ids 23..28, and each id is
/// both the NVIC line and the PMC_PCSR0 bit.
pub const SAM4S_TC_BLOCKS: [Sam4sTcBlock; 2] = [
    Sam4sTcBlock {
        base: 0x4001_0000,
        first_irq: 23,
    },
    Sam4sTcBlock {
        base: 0x4001_4000,
        first_irq: 26,
    },
];

#[derive(Debug, Default)]
struct Channel {
    cmr: u32,
    ra: u16,
    rb: u16,
    rc: u16,
    cv: u16,
    /// SR.CLKSTA: set by CLKEN, cleared by CLKDIS and a CPCDIS compare.
    enabled: bool,
    /// Counter clock started: set by a trigger on an enabled clock, cleared
    /// by a CPCSTOP compare.
    started: bool,
    /// SR status flags (bits 0..7), cleared by an SR read.
    sr: Cell<u8>,
    imr: u8,
    /// CPU cycles (TIMER_CLOCK1..4) or CPU-cycle × SLCK units (TIMER_CLOCK5)
    /// not yet worth a whole counter clock.
    carry: u128,
}

impl Channel {
    fn wave(&self) -> bool {
        self.cmr & CMR_WAVE != 0
    }
    /// RC compare restarts the count: UP_RC in waveform, CPCTRG in capture.
    fn rc_resets(&self) -> bool {
        if self.wave() {
            (self.cmr >> 13) & 3 == 2
        } else {
            self.cmr & CMR_CPCTRG != 0
        }
    }
    fn top(&self) -> u16 {
        if self.rc_resets() && self.cv <= self.rc {
            self.rc
        } else {
            0xffff
        }
    }
    fn running(&self) -> bool {
        self.enabled && self.started
    }
    fn line(&self) -> bool {
        self.sr.get() & self.imr != 0
    }
    fn flag(&mut self, bits: u8) {
        self.sr.set(self.sr.get() | bits);
    }
    fn trigger(&mut self) {
        self.cv = 0;
        self.carry = 0;
        if self.enabled {
            self.started = true;
        }
    }
    /// The counter has just reached `cv`: raise its compares.
    fn reached(&mut self) {
        let v = self.cv;
        if self.wave() {
            if v == self.ra {
                self.flag(SR_CPAS);
            }
            if v == self.rb {
                self.flag(SR_CPBS);
            }
        }
        if v == self.rc {
            self.flag(SR_CPCS);
            if self.wave() && self.cmr & CMR_CPCDIS != 0 {
                self.enabled = false;
                self.started = false;
            } else if self.wave() && self.cmr & CMR_CPCSTOP != 0 {
                self.started = false;
            }
        }
    }
    fn advance(&mut self, mut ticks: u64) {
        while ticks > 0 && self.running() {
            let top = self.top();
            if self.cv == top {
                // The next counter clock wraps (COVFS) or is the RC trigger.
                if top == 0xffff && !(self.rc_resets() && self.rc == 0xffff) {
                    self.flag(SR_COVFS);
                }
                self.cv = 0;
                ticks -= 1;
                self.reached();
                continue;
            }
            let mut dist = top - self.cv;
            let compares = if self.wave() {
                [self.ra, self.rb, self.rc]
            } else {
                [self.rc; 3]
            };
            for t in compares {
                if t > self.cv && t <= top {
                    dist = dist.min(t - self.cv);
                }
            }
            let step = ticks.min(u64::from(dist));
            self.cv += step as u16;
            ticks -= step;
            self.reached();
        }
    }
    fn write_cmr(&mut self, value: u32) -> SimResult<()> {
        if value & 7 >= 5 {
            return Sam4sTc::refuse("external clocks XC0..XC2");
        }
        if (value >> 4) & 3 != 0 {
            return Sam4sTc::refuse("BURST clock gating");
        }
        if value & CMR_WAVE != 0 {
            if (value >> 8) & 3 != 0 {
                return Sam4sTc::refuse("waveform external events (EEVTEDG)");
            }
            if (value >> 13) & 1 != 0 {
                return Sam4sTc::refuse("UPDOWN waveforms");
            }
        } else {
            if (value >> 8) & 3 != 0 {
                return Sam4sTc::refuse("capture external triggers (ETRGEDG)");
            }
            if (value >> 16) & 0xf != 0 {
                return Sam4sTc::refuse("capture loads (LDRA/LDRB)");
            }
            if value & 0xfff0_3800 != 0 {
                return Sam4sTc::refuse("TC_CMR capture-mode reserved bits");
            }
        }
        self.cmr = value;
        Ok(())
    }
}

#[derive(Debug)]
pub struct Sam4sTc {
    block: Sam4sTcBlock,
    ch: [Channel; 3],
    bmr: u32,
    wpmr: u32,
    cpu_hz: u64,
    /// PMC_PCSR0 bits of the three channels, re-read from the bus on every
    /// bus tick while any channel's clock is enabled.
    clocked: [bool; 3],
    gate_dirty: bool,
    refusal: Option<String>,
}

impl Sam4sTc {
    pub fn new(block: Sam4sTcBlock) -> Self {
        Self {
            block,
            ch: Default::default(),
            bmr: 0,
            wpmr: 0,
            cpu_hz: 120_000_000,
            clocked: [false; 3],
            gate_dirty: true,
            refusal: None,
        }
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "sam4s TC slice: {what}"
        )))
    }
    fn reset(&mut self) {
        let cpu_hz = self.cpu_hz;
        *self = Self::new(self.block);
        self.cpu_hz = cpu_hz;
    }
    pub fn counter(&self, channel: usize) -> u16 {
        self.ch[channel].cv
    }
    fn sr_word(&self, c: &Channel, clear: bool) -> u32 {
        let flags = c.sr.get();
        if clear {
            c.sr.set(0);
        }
        u32::from(flags) | if c.enabled { SR_CLKSTA } else { 0 }
    }
    fn read_reg(&self, offset: u64, clear: bool) -> SimResult<u32> {
        if offset < 0xc0 {
            let (n, reg) = ((offset / 0x40) as usize, offset % 0x40);
            let c = &self.ch[n];
            return Ok(match reg {
                0x00 => 0, // CCR is write-only
                0x04 => c.cmr,
                0x08 | 0x0c => 0, // SMMR, RAB
                0x10 => u32::from(c.cv),
                0x14 => u32::from(c.ra),
                0x18 => u32::from(c.rb),
                0x1c => u32::from(c.rc),
                0x20 => self.sr_word(c, clear),
                0x24 | 0x28 => 0, // IER, IDR are write-only
                0x2c => u32::from(c.imr),
                _ => return Self::refuse("register outside the channel slice"),
            });
        }
        Ok(match offset {
            0xc0 => 0, // BCR is write-only
            0xc4 => self.bmr,
            0xc8..=0xdc => 0, // QDEC and fault registers: not modelled, all clear
            0xe4 => self.wpmr,
            _ => return Self::refuse("register outside the block slice"),
        })
    }
    fn write_reg(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset < 0xc0 {
            let (n, reg) = ((offset / 0x40) as usize, offset % 0x40);
            let c = &mut self.ch[n];
            match reg {
                0x00 => {
                    if value & CCR_CLKDIS != 0 {
                        c.enabled = false;
                        c.started = false;
                    } else if value & CCR_CLKEN != 0 {
                        c.enabled = true;
                    }
                    if value & CCR_SWTRG != 0 {
                        c.trigger();
                    }
                    self.gate_dirty = true;
                }
                0x04 => c.write_cmr(value)?,
                0x08 if value == 0 => {}
                0x08 => return Self::refuse("stepper motor mode (SMMR)"),
                // RA/RB are loaded by captures in capture mode; stores are ignored.
                0x14 if c.wave() => c.ra = value as u16,
                0x18 if c.wave() => c.rb = value as u16,
                0x14 | 0x18 => {}
                0x1c => c.rc = value as u16,
                0x24 => c.imr |= (value as u8) & INT_MASK,
                0x28 => c.imr &= !(value as u8),
                _ => return Self::refuse("store outside the channel slice"),
            }
            return Ok(());
        }
        match offset {
            0xc0 => {
                if value & 1 != 0 {
                    // SYNC: a software trigger on all three channels at once.
                    for c in &mut self.ch {
                        c.trigger();
                    }
                }
            }
            0xc4 if value & !0x3f == 0 => self.bmr = value,
            0xc4 => return Self::refuse("quadrature decoder (BMR QDEC)"),
            0xc8 | 0xcc | 0xd8 if value == 0 => {}
            0xc8 | 0xcc => return Self::refuse("quadrature decoder interrupts"),
            0xd8 => return Self::refuse("fault mode (FMR)"),
            0xe4 if value & 1 == 0 => self.wpmr = value & 0xffff_ff00,
            0xe4 => return Self::refuse("write protection (WPEN)"),
            _ => return Self::refuse("store outside the block slice"),
        }
        Ok(())
    }
    fn checked(&mut self, result: SimResult<()>) -> SimResult<()> {
        if let Err(e) = &result {
            self.refusal.get_or_insert_with(|| e.to_string());
        }
        result
    }
    fn counter_ticks(&self, c: &mut Channel, cycles: u64) -> u64 {
        match c.cmr & 7 {
            4 => {
                c.carry += u128::from(cycles) * SLCK_HZ;
                let hz = u128::from(self.cpu_hz);
                let ticks = c.carry / hz;
                c.carry %= hz;
                ticks as u64
            }
            sel => {
                let div = 2u128 << (2 * sel);
                c.carry += u128::from(cycles);
                let ticks = c.carry / div;
                c.carry %= div;
                ticks as u64
            }
        }
    }
}

impl Peripheral for Sam4sTc {
    fn on_system_reset(&mut self) {
        self.reset();
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let aligned = offset & !3;
        // Only the SR byte holding the flags clears them.
        let clear = offset == aligned;
        Ok((self.read_reg(aligned, clear)? >> (8 * (offset - aligned))) as u8)
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.read_reg(offset & !3, true)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        let r = Self::refuse("byte stores (TC registers are word-wide)");
        self.checked(r)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let r = self.write_reg(offset, value);
        self.checked(r)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        let aligned = offset & !3;
        let word = self.read_reg(aligned, false).ok()?;
        Some((word >> (8 * (offset - aligned))) as u8)
    }
    fn reads_can_deassert_irq(&self) -> bool {
        self.ch.iter().any(|c| c.imr != 0)
    }
    fn needs_bus_tick(&self) -> bool {
        // PMC_PCDR0 can gate a running channel off at any time and nothing
        // notifies this block, so the gate is re-read on every bus tick
        // while any channel's clock is enabled.
        self.ch.iter().any(|c| c.enabled)
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        let pcsr0 = bus.read_u32(PMC_PCSR0).unwrap_or(0);
        for (i, on) in self.clocked.iter_mut().enumerate() {
            *on = pcsr0 & (1 << (self.block.first_irq + i as u32)) != 0;
        }
        self.gate_dirty = false;
    }
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.cpu_hz = hz.max(1);
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn legacy_tick_active(&self) -> bool {
        self.ch.iter().any(|c| c.running() || c.line())
    }
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        for (i, c) in self.ch.iter().enumerate() {
            report(self.block.first_irq + i as u32, c.line());
        }
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        let mut irqs = Vec::new();
        for i in 0..3 {
            let mut c = std::mem::take(&mut self.ch[i]);
            if c.running() && self.clocked[i] && !self.gate_dirty {
                let ticks = self.counter_ticks(&mut c, cycles);
                if ticks > 0 {
                    c.advance(ticks);
                }
            }
            if c.line() {
                irqs.push(self.block.first_irq + i as u32);
            }
            self.ch[i] = c;
        }
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
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
        let ch: Vec<_> = self
            .ch
            .iter()
            .map(|c| {
                serde_json::json!({"cmr": c.cmr, "cv": c.cv, "ra": c.ra, "rb": c.rb,
                    "rc": c.rc, "sr": c.sr.get(), "imr": c.imr,
                    "enabled": c.enabled, "started": c.started})
            })
            .collect();
        serde_json::json!({"peripheral": "sam4s_tc", "base": self.block.base,
            "channels": ch, "bmr": self.bmr, "clocked": self.clocked,
            "refusal": self.refusal})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block() -> Sam4sTc {
        let mut tc = Sam4sTc::new(SAM4S_TC_BLOCKS[0]);
        tc.clocked = [true; 3];
        tc.gate_dirty = false;
        tc
    }
    fn start(tc: &mut Sam4sTc, ch: u64) {
        tc.write_u32(ch * 0x40, CCR_CLKEN | CCR_SWTRG).unwrap();
        tc.gate_dirty = false;
    }

    #[test]
    fn clken_alone_does_not_count_until_a_trigger() {
        let mut tc = block();
        tc.write_u32(0x00, CCR_CLKEN).unwrap();
        tc.gate_dirty = false;
        tc.tick_elapsed(100);
        assert_eq!(tc.counter(0), 0);
        assert_eq!(tc.read_u32(0x20).unwrap() & SR_CLKSTA, SR_CLKSTA);
        tc.write_u32(0x00, CCR_SWTRG).unwrap();
        tc.gate_dirty = false;
        tc.tick_elapsed(100); // TIMER_CLOCK1 = MCK/2
        assert_eq!(tc.counter(0), 50);
    }

    #[test]
    fn up_rc_wraps_at_rc_and_raises_cpcs_then_read_clears_it() {
        let mut tc = block();
        tc.write_u32(0x04, CMR_WAVE | (2 << 13) | 1).unwrap(); // UP_RC, MCK/8
        tc.write_u32(0x1c, 99).unwrap();
        tc.write_u32(0x14, 40).unwrap();
        tc.write_u32(0x18, 200).unwrap(); // above RC: never reached
        tc.write_u32(0x24, u32::from(SR_CPCS)).unwrap();
        start(&mut tc, 0);
        tc.tick_elapsed(8 * 99);
        assert_eq!(tc.counter(0), 99);
        let mut lines = Vec::new();
        tc.irq_line_levels(&mut |irq, level| lines.push((irq, level)));
        assert_eq!(lines[0], (23, true));
        tc.tick_elapsed(8 * 5);
        assert_eq!(tc.counter(0), 4); // 99 -> 0 is the RC trigger, not COVFS
        let sr = tc.read_u32(0x20).unwrap();
        assert_eq!(sr & 0xff, u32::from(SR_CPAS | SR_CPCS));
        assert_eq!(tc.read_u32(0x20).unwrap() & 0xff, 0);
        assert!(!tc.ch[0].line());
    }

    #[test]
    fn up_mode_overflows_and_cpcstop_stops_the_clock() {
        let mut tc = block();
        tc.write_u32(0x40 + 0x04, CMR_WAVE).unwrap(); // UP, MCK/2
        start(&mut tc, 1);
        tc.tick_elapsed(2 * 0x1_0003);
        assert_eq!(tc.counter(1), 3);
        assert_eq!(tc.read_u32(0x40 + 0x20).unwrap() & 1, 1);

        tc.write_u32(0x80 + 0x04, CMR_WAVE | CMR_CPCSTOP | 3)
            .unwrap(); // MCK/128
        tc.write_u32(0x80 + 0x1c, 10).unwrap();
        start(&mut tc, 2);
        tc.tick_elapsed(128 * 50);
        assert_eq!(tc.counter(2), 10);
        let sr = tc.read_u32(0x80 + 0x20).unwrap();
        assert_eq!(sr, SR_CLKSTA | u32::from(SR_CPCS));
    }

    #[test]
    fn cpcdis_disables_and_pmc_gate_holds_the_count() {
        let mut tc = block();
        tc.write_u32(0x04, CMR_WAVE | CMR_CPCDIS).unwrap();
        tc.write_u32(0x1c, 5).unwrap();
        start(&mut tc, 0);
        tc.tick_elapsed(100);
        assert_eq!(tc.read_u32(0x20).unwrap() & SR_CLKSTA, 0);
        // A trigger cannot restart a disabled clock.
        tc.write_u32(0x00, CCR_SWTRG).unwrap();
        tc.gate_dirty = false;
        tc.tick_elapsed(100);
        assert_eq!(tc.counter(0), 0);

        tc.write_u32(0x04, 0).unwrap(); // capture mode, free-running
        start(&mut tc, 0);
        tc.clocked[0] = false;
        tc.tick_elapsed(100);
        assert_eq!(tc.counter(0), 0);
    }

    #[test]
    fn capture_cpctrg_restarts_and_capture_registers_ignore_stores() {
        let mut tc = block();
        tc.write_u32(0x04, CMR_CPCTRG).unwrap();
        tc.write_u32(0x1c, 9).unwrap();
        tc.write_u32(0x14, 1234).unwrap();
        assert_eq!(tc.read_u32(0x14).unwrap(), 0);
        start(&mut tc, 0);
        tc.tick_elapsed(2 * 13);
        assert_eq!(tc.counter(0), 3);
        assert_eq!(tc.read_u32(0x20).unwrap() & 0xff, u32::from(SR_CPCS));
    }

    #[test]
    fn slck_counts_32768_per_second_and_sync_triggers_all() {
        let mut tc = block();
        tc.attach_cpu_hz(1_000_000);
        tc.write_u32(0x04, 4).unwrap();
        tc.write_u32(0x00, CCR_CLKEN).unwrap();
        tc.write_u32(0x40, CCR_CLKEN).unwrap();
        tc.write_u32(0xc0, 1).unwrap();
        tc.gate_dirty = false;
        tc.tick_elapsed(1_000_000);
        assert_eq!(tc.counter(0), 32_768);
        assert_eq!(tc.counter(1), 500_000u32 as u16);
    }

    #[test]
    fn unmodelled_modes_are_refused() {
        let mut tc = block();
        assert!(tc.write_u32(0x04, 5).is_err()); // XC0
        assert!(tc.write_u32(0x04, 1 << 16).is_err()); // LDRA
        assert!(tc.write_u32(0x04, CMR_WAVE | (1 << 13)).is_err()); // UPDOWN
        assert!(tc.write_u32(0xc4, 1 << 8).is_err()); // QDEN
        assert!(tc.write(0x04, 1).is_err());
    }
}
