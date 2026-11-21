// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Philips/NXP LPC2000 10-bit A/D converter (LPC2119/2129/2194/2292/2294)
//! at `0xE003_4000`: `ADCR` @0x00, `ADDR` @0x04.
//!
//! Sources:
//! - Philips "LPC2119/2129/2194/2292/2294 User Manual", preliminary,
//!   2004-05-03 (Keil `user_manual_lpc2119_2129_2194_2292_2294.pdf`),
//!   chapter 17 "A/D Converter": Table 173 (two registers, `ADCR` reset
//!   0x0000_0001), Table 174 (`SEL` [7:0], `CLKDIV` [15:8], `BURST` 16,
//!   `CLKS` [19:17] = 11 clocks/10 bits .. 4 clocks/3 bits, `PDN` 21,
//!   `TEST` [23:22], `START` [26:24], `EDGE` 27), Table 175 (`DONE` 31,
//!   `OVERUN` 30, `CHN` [26:24], `V/V3A` [15:6]; [29:27], [23:16] and [5:0]
//!   read 0), "Interrupts" (the VIC request is asserted while `DONE` is 1;
//!   `DONE` clears when `ADDR` is read). VIC channel 18 is "A/D" (Table
//!   "Connection of interrupt sources to the VIC"). Ain0..3 = P0.27..P0.30,
//!   Ain4/5 = P2.30/P2.31, Ain6/7 = P3.29/P3.28 (144-pin pin description);
//!   the 64-pin parts bond Ain3:0 only.
//! - Renode `platforms/cpus/lpc2294.repl` (ADCR @0xE0034000, ADDR
//!   @0xE0034004) and RTEMS `bsps/arm/rtl22xx/include/lpc22xx.h` (same).
//! - Software-controlled read as LPC21xx/22xx firmware does it
//!   (udayer45-cloud/automotive-instrument-cluster-lpc2129
//!   `NodeA/adc_driver.c`): `ADCR = 0x00200400` (PDN, CLKDIV 4), set the
//!   `SEL` bit, `ADCR |= 1 << 24`, poll `ADDR` bit 31, `ADCR ^= 1 << 24`,
//!   result `(ADDR >> 6) & 0x3FF`. The `^=` idiom only stops the converter
//!   if `START` reads back as written, so `START` is stored.
//!
//! Modelled: with `PDN` = 1 and `BURST` = 0, every `ADCR` write whose
//! `START` is 001 converts at once; the channel is the least-significant 1
//! in `SEL` (all zeroes = 0x01). Every `ADCR` write clears `DONE`, then the
//! conversion sets it. With `BURST` = 1 every tick converts the next `SEL`
//! channel, lowest first, wrapping; a result that lands while `DONE` is
//! still 1 sets `OVERUN`. Reading `ADDR` clears `DONE` and `OVERUN`.
//! Code = mV * 1024 / V3A, truncated, clamped to 0x3FF; V3A defaults to
//! 3300 mV. An undriven pad converts to 0.
//!
//! Not sourced, so kept minimal: conversion time (one conversion per
//! write or per burst tick); `CLKS` in burst mode keeps the top 10 - CLKS
//! bits of the 10-bit field and zeroes the rest (the manual gives only the
//! bit count); more than one `SEL` bit in software mode converts the
//! lowest; hardware-triggered `START` 010..111 (CAP/MAT edges) converts
//! nothing, as the pin and timer edges are not wired; `TEST` modes and
//! `ADDR` writes (a test shift register) have no effect; `CHN` and the
//! result read 0 before the first conversion.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;

const CHANNELS: usize = 8;
const CODE_MAX: u32 = (1 << 10) - 1;

const ADCR: u64 = 0x00;
const ADDR: u64 = 0x04;

const ADCR_RESET: u32 = 0x0000_0001;
/// SEL, CLKDIV, BURST, CLKS, PDN, TEST, START, EDGE; bits 20 and 31:28 are
/// not defined.
const ADCR_MASK: u32 = 0x0FEF_FFFF;
const SEL_MASK: u32 = 0xFF;
const BURST: u32 = 1 << 16;
const CLKS_SHIFT: u32 = 17;
const PDN: u32 = 1 << 21;
const START_SHIFT: u32 = 24;
const START_NOW: u32 = 0b001;

const DONE: u32 = 1 << 31;
const OVERUN: u32 = 1 << 30;
const CHN_SHIFT: u32 = 24;
const RESULT_SHIFT: u32 = 6;

/// LPC2000 ADC: software-started and burst conversions over Ain0..7.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Lpc2000Adc {
    adcr: u32,
    /// `ADDR` value; a `Cell` because reading it clears `DONE` and `OVERUN`.
    addr: Cell<u32>,
    /// Next `SEL` bit position the burst scan converts.
    burst_next: u8,
    vref_mv: u32,
    /// Routed levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
}

impl Default for Lpc2000Adc {
    fn default() -> Self {
        Self::new(3300)
    }
}

impl Lpc2000Adc {
    /// `vref_mv`: V3A, the analog supply the result is a fraction of.
    pub fn new(vref_mv: u32) -> Self {
        Self {
            adcr: ADCR_RESET,
            addr: Cell::new(0),
            burst_next: 0,
            vref_mv: vref_mv.max(1),
            inputs_mv: [u16::MAX; CHANNELS],
        }
    }

    fn sel(&self) -> u32 {
        match self.adcr & SEL_MASK {
            0 => 1,
            sel => sel,
        }
    }

    fn code(&self, channel: usize) -> u32 {
        let mv = match self.inputs_mv[channel] {
            u16::MAX => 0,
            mv => u32::from(mv),
        };
        (mv * 1024 / self.vref_mv).min(CODE_MAX)
    }

    /// Converts `channel` into `ADDR` with `dropped` low bits zeroed.
    fn convert(&mut self, channel: usize, dropped: u32) {
        let code = self.code(channel) & !((1 << dropped) - 1);
        let overun = if self.addr.get() & DONE != 0 && self.adcr & BURST != 0 {
            OVERUN
        } else {
            0
        };
        self.addr
            .set(DONE | overun | ((channel as u32) << CHN_SHIFT) | (code << RESULT_SHIFT));
    }

    fn write_adcr(&mut self, value: u32) {
        let was_burst = self.adcr & BURST != 0;
        self.adcr = value & ADCR_MASK;
        self.addr.set(self.addr.get() & !DONE);
        if self.adcr & PDN == 0 {
            return;
        }
        if self.adcr & BURST != 0 {
            if !was_burst {
                self.burst_next = 0;
            }
            return;
        }
        if (self.adcr >> START_SHIFT) & 0b111 == START_NOW {
            let channel = self.sel().trailing_zeros() as usize;
            self.convert(channel, 0);
        }
    }

    /// One burst conversion: the next `SEL` channel at or above
    /// `burst_next`, wrapping to the lowest.
    fn burst_step(&mut self) {
        let sel = self.sel();
        let above = sel & !((1u32 << self.burst_next) - 1);
        let channel = if above != 0 { above } else { sel }.trailing_zeros();
        let dropped = (self.adcr >> CLKS_SHIFT) & 0b111;
        self.convert(channel as usize, dropped);
        self.burst_next = ((channel + 1) % CHANNELS as u32) as u8;
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            ADCR => self.adcr,
            ADDR => self.addr.get(),
            _ => 0,
        }
    }

    fn after_addr_read(&self) {
        self.addr.set(self.addr.get() & !(DONE | OVERUN));
    }
}

impl Peripheral for Lpc2000Adc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        match self.inputs_mv.get_mut(channel as usize) {
            Some(slot) => {
                *slot = millivolts;
                true
            }
            None => false,
        }
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        match self.inputs_mv.get_mut(channel as usize) {
            Some(slot) => {
                *slot = u16::MAX;
                true
            }
            None => false,
        }
    }

    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }

    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || volts <= 0.0 {
            return false;
        }
        self.vref_mv = ((volts * 1000.0).round() as u32).max(1);
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.addr.get() & DONE != 0)
    }

    fn on_system_reset(&mut self) {
        let (inputs, vref) = (self.inputs_mv, self.vref_mv);
        *self = Self::new(vref);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let value = (self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8;
        // The byte that holds DONE and OVERUN.
        if offset == ADDR + 3 {
            self.after_addr_read();
        }
        Ok(value)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        if aligned == ADCR {
            let shift = (offset & 3) * 8;
            let next = (self.adcr & !(0xFF << shift)) | (u32::from(value) << shift);
            self.write_adcr(next);
        }
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let value = (self.read_word(offset & !3) >> ((offset & 2) * 8)) as u16;
        if offset == ADDR + 2 {
            self.after_addr_read();
        }
        Ok(value)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let value = self.read_word(offset);
        if offset == ADDR {
            self.after_addr_read();
        }
        Ok(value)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset == ADCR {
            self.write_adcr(value);
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.adcr & (PDN | BURST) == PDN | BURST {
            self.burst_step();
        }
        PeripheralTickResult {
            irq: self.addr.get() & DONE != 0,
            ..PeripheralTickResult::default()
        }
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

    fn adc_with(inputs: &[(u8, u16)]) -> Lpc2000Adc {
        let mut adc = Lpc2000Adc::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv), "channel {ch}");
        }
        adc
    }

    fn code(mv: u32) -> u32 {
        (mv * 1024 / 3300).min(1023)
    }

    /// adc_driver.c `adc_init` + `adc_read`: PDN and CLKDIV 4, select, start,
    /// poll DONE, `ADCR ^= START`, deselect, read the result.
    fn driver_read(adc: &mut Lpc2000Adc, ch: u32) -> (u32, u32) {
        adc.write_u32(ADCR, 0x0020_0400).unwrap();
        let cr = adc.read_u32(ADCR).unwrap();
        adc.write_u32(ADCR, cr | (1 << ch)).unwrap();
        let cr = adc.read_u32(ADCR).unwrap();
        adc.write_u32(ADCR, cr | (1 << 24)).unwrap();
        assert!(adc.irq_line_level().unwrap(), "DONE requests VIC 18");
        let polled = adc.read_u32(ADDR).unwrap();
        assert_ne!(polled & DONE, 0, "Ain{ch} DONE");
        assert!(!adc.irq_line_level().unwrap(), "reading ADDR clears DONE");
        let cr = adc.read_u32(ADCR).unwrap();
        adc.write_u32(ADCR, cr ^ (1 << 24)).unwrap();
        assert_eq!(
            adc.read_u32(ADCR).unwrap() >> 24 & 7,
            0,
            "START stored, XOR stops"
        );
        let cr = adc.read_u32(ADCR).unwrap();
        adc.write_u32(ADCR, cr ^ (1 << ch)).unwrap();
        let addr = adc.read_u32(ADDR).unwrap();
        assert_eq!(addr & DONE, 0, "no conversion after the XOR stop");
        ((addr >> 6) & 0x3FF, (addr >> 24) & 7)
    }

    #[test]
    fn reset_values_and_reserved_bits() {
        let mut adc = Lpc2000Adc::default();
        assert_eq!(adc.read_u32(ADCR).unwrap(), ADCR_RESET);
        assert_eq!(adc.read_u32(ADDR).unwrap(), 0);
        adc.write_u32(ADCR, !PDN).unwrap();
        assert_eq!(adc.read_u32(ADCR).unwrap(), ADCR_MASK & !PDN);
        assert_eq!(adc.read_u32(ADDR).unwrap(), 0, "PDN = 0: nothing converts");
        assert!(!adc.irq_line_level().unwrap());
    }

    #[test]
    fn software_read_converts_each_pad_at_10_bits_against_v3a() {
        let mut adc = adc_with(&[(0, 0), (1, 1650), (5, 2000), (6, 3299), (7, 3600)]);
        assert_eq!(driver_read(&mut adc, 0), (0, 0));
        assert_eq!(driver_read(&mut adc, 1), (512, 1), "half of V3A");
        assert_eq!(driver_read(&mut adc, 5), (code(2000), 5));
        assert_eq!(driver_read(&mut adc, 6), (1023, 6), "just below V3A");
        assert_eq!(driver_read(&mut adc, 7), (1023, 7), "clamped above V3A");
        assert_eq!(driver_read(&mut adc, 3), (0, 3), "undriven pad");
        // A lower V3A raises every code.
        let mut adc = adc_with(&[(5, 2000)]);
        assert!(adc.set_adc_reference_voltage_v(3.0));
        assert_eq!(driver_read(&mut adc, 5).0, 2000 * 1024 / 3000);
    }

    #[test]
    fn sel_zero_is_ain0_and_several_bits_convert_the_lowest() {
        let mut adc = adc_with(&[(0, 1000), (2, 2000), (4, 3000)]);
        adc.write_u32(ADCR, PDN | (START_NOW << START_SHIFT))
            .unwrap();
        let addr = adc.read_u32(ADDR).unwrap();
        assert_eq!((addr >> 24) & 7, 0);
        assert_eq!((addr >> 6) & 0x3FF, code(1000));
        adc.write_u32(ADCR, PDN | 0b10100 | (START_NOW << START_SHIFT))
            .unwrap();
        let addr = adc.read_u32(ADDR).unwrap();
        assert_eq!(((addr >> 24) & 7, (addr >> 6) & 0x3FF), (2, code(2000)));
    }

    #[test]
    fn adcr_write_clears_done_and_hardware_start_converts_nothing() {
        let mut adc = adc_with(&[(5, 2000)]);
        adc.write_u32(ADCR, PDN | (1 << 5) | (START_NOW << START_SHIFT))
            .unwrap();
        assert!(adc.irq_line_level().unwrap());
        // START 010: edge on P0.16/EINT0; nothing drives it.
        adc.write_u32(ADCR, PDN | (1 << 5) | (0b010 << START_SHIFT))
            .unwrap();
        assert!(!adc.irq_line_level().unwrap(), "ADCR write clears DONE");
        assert_eq!(adc.read_u32(ADDR).unwrap() & DONE, 0);
        adc.tick();
        assert_eq!(adc.read_u32(ADDR).unwrap() & DONE, 0, "no edge, no start");
    }

    #[test]
    fn burst_scans_sel_lowest_first_wraps_and_flags_overrun() {
        let mut adc = adc_with(&[(1, 500), (5, 2000), (6, 1200)]);
        adc.write_u32(ADCR, PDN | BURST | (1 << 6) | (1 << 5) | (1 << 1))
            .unwrap();
        assert_eq!(
            adc.read_u32(ADDR).unwrap() & DONE,
            0,
            "first result on a tick"
        );
        let mut seen = Vec::new();
        for _ in 0..4 {
            adc.tick();
            let addr = adc.read_u32(ADDR).unwrap();
            assert_ne!(addr & DONE, 0);
            assert_eq!(addr & OVERUN, 0, "read every result");
            seen.push(((addr >> 24) & 7, (addr >> 6) & 0x3FF));
        }
        assert_eq!(
            seen,
            [
                (1, code(500)),
                (5, code(2000)),
                (6, code(1200)),
                (1, code(500))
            ]
        );
        // Two results without a read: OVERUN, then a read clears both flags.
        adc.tick();
        adc.tick();
        let addr = adc.read_u32(ADDR).unwrap();
        assert_eq!(addr & (DONE | OVERUN), DONE | OVERUN);
        assert_eq!((addr >> 24) & 7, 6);
        assert_eq!(adc.read_u32(ADDR).unwrap() & (DONE | OVERUN), 0);
        // CLKS 111 = 3 bits: the low 7 bits of the 10-bit field read 0.
        adc.write_u32(ADCR, PDN | BURST | (7 << CLKS_SHIFT) | (1 << 5))
            .unwrap();
        adc.tick();
        let addr = adc.read_u32(ADDR).unwrap();
        assert_eq!((addr >> 6) & 0x3FF, code(2000) & 0x380);
        // Clearing BURST stops the scan.
        adc.write_u32(ADCR, PDN | (1 << 5)).unwrap();
        adc.tick();
        assert_eq!(adc.read_u32(ADDR).unwrap() & DONE, 0);
    }

    #[test]
    fn byte_read_of_the_done_byte_clears_done() {
        let mut adc = adc_with(&[(2, 1650)]);
        adc.write_u32(ADCR, PDN | (1 << 2) | (START_NOW << START_SHIFT))
            .unwrap();
        assert_eq!(adc.read(ADDR + 1).unwrap(), (512u32 << 6 >> 8) as u8);
        assert!(adc.irq_line_level().unwrap(), "low bytes leave DONE");
        assert_eq!(adc.read(ADDR + 3).unwrap(), 0x82, "DONE | CHN 2");
        assert!(!adc.irq_line_level().unwrap());
    }

    #[test]
    fn system_reset_keeps_routed_inputs_and_vref() {
        let mut adc = adc_with(&[(4, 1000)]);
        assert!(adc.set_adc_reference_voltage_v(3.0));
        adc.write_u32(ADCR, PDN | (1 << 4) | (START_NOW << START_SHIFT))
            .unwrap();
        adc.on_system_reset();
        assert_eq!(adc.read_u32(ADCR).unwrap(), ADCR_RESET);
        assert_eq!(adc.read_u32(ADDR).unwrap(), 0);
        adc.write_u32(ADCR, PDN | (1 << 4) | (START_NOW << START_SHIFT))
            .unwrap();
        let addr = adc.read_u32(ADDR).unwrap();
        assert_eq!((addr >> 6) & 0x3FF, 1000 * 1024 / 3000);
    }
}
