// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CC2538 SOC_ADC block (`cc2538_soc_adc`) at 0x400D_7000: the ADC (SWRU319C
//! §15, Table 15-1), the random-number generator (§16, Table 16-1) and the
//! analog comparator (§17, Table 17-1) share this window. Renode's
//! `cc2538.repl` leaves it unmodelled.
//!
//! | Offset | Name | Fields | Reset |
//! | --- | --- | --- | --- |
//! | 0x00 | ADCCON1 | EOC 7 (RO, cleared by reading ADCH), ST 6, STSEL 5:4, RCTRL 3:2, reserved RW 1:0 | 0x33 |
//! | 0x04 | ADCCON2 | SREF 7:6, SDIV 5:4, SCH 3:0 (sequence) | 0x10 |
//! | 0x08 | ADCCON3 | EREF 7:6, EDIV 5:4, ECH 3:0 (single conversion) | 0x00 |
//! | 0x0C | ADCL | ADC result \[7:2\] (RO) | 0x00 |
//! | 0x10 | ADCH | ADC result \[15:8\] (RO) | 0x00 |
//! | 0x14 | RNDL | LFSR \[7:0\] | 0xFF |
//! | 0x18 | RNDH | LFSR \[15:8\]; a write is a CRC16 input byte | 0xFF |
//! | 0x24 | CMPCTL | EN 1, OUTPUT 0 (RO) | 0x01 |
//!
//! **ADC.** A write to ADCCON3 performs one conversion of ECH (§15.2.3) and
//! pulses the ADC interrupt (§15.2.8, IRQ 14). With STSEL = 11, writing ST = 1
//! runs the ADCCON2 sequence (§15.2.2): AIN0..SCH for SCH ≤ 7, AIN0-AIN1..SCH
//! for 8 ≤ SCH ≤ 11, SCH alone for SCH ≥ 12; ST clears and SCH then reads one
//! more than the last channel (the written value for SCH ≥ 12, §15.2.5).
//! Sequence conversions raise no interrupt. Every conversion sets EOC.
//!
//! The result is the §15.2.5 two's-complement code, MSB-aligned in
//! ADCH:ADCL: with `b` = 7/9/10/12 ENOB bits for decimation 64/128/256/512,
//! `code = round(Vconv / Vref * (2^(b-1) - 1))`, saturated to the signed
//! `b`-bit range, and the register pair holds `code << (16 - b)`. A 12-bit
//! VREF-level input reads 2047 in the 12 MSBs, as §15.2.5 states.
//!
//! Inputs: AIN0..AIN7 are driven through `set_adc_channel_input` (0 mV until
//! driven). Channels 8..11 are AIN(2n) - AIN(2n+1); 12 is GND; 15 is AVDD5/3.
//! References: internal 1.19 V (the constant Contiki-NG's `vdd3-sensor.c`
//! converts with), AVDD5 (the board rail, 3.0 V unless the system routes a
//! reference voltage), AIN7, or AIN6 - AIN7. Channel 14 (temperature sensor)
//! converts a fixed 25 °C level, chosen so the 12-bit internal-reference code
//! is 1422, the 25 °C offset in Contiki-NG's `cc2538-temp-sensor.c`.
//!
//! **RNG** (§16.2). RNDH:RNDL is the 16-bit LFSR with polynomial
//! x^16 + x^15 + x^2 + 1 (Figure 16-1): a shift moves bit 15 out and, when
//! the feedback (bit 15 XOR the input bit) is 1, XORs 0x8005 into the shifted
//! value. Seeds 0x0000 and 0x8003 are fixed points, as the §16.2.2 NOTE says.
//! Writing RCTRL = 01 clocks the LFSR once (13 shifts with a zero input bit)
//! and RCTRL reads back 00. A write to RNDL moves the low byte to the high
//! byte and loads the written byte; a write to RNDH feeds the byte MSB-first
//! as CRC16 input (§16.2.3).
//!
//! **Comparator** (§17.1). With EN set, OUTPUT is 1 when the PA5 (AIN5)
//! input is above PA4 (AIN4). With EN clear it reads its reset value, 1.
//!
//! NOT modelled: conversion time (TCONV = (decimation + 16) × 0.25 µs,
//! §15.2.7; every conversion completes inside the register write), the
//! STSEL = 01 free-running and STSEL = 10 timer-triggered sequence starts,
//! the IOC_Pxx_SEL check that skips inactive sequence channels, ADC DMA
//! triggers (§15.2.9), the CCTEST_TR0.ADCTM / RFCORE ATEST gating of the
//! temperature sensor, a varying temperature, the CSP-driven RNG clocking
//! (RCTRL = 00), and the comparator's override of the PA5 edge detector and
//! its IRQ (41).

use std::cell::Cell;

use crate::{Peripheral, SimResult};

const ADCCON1: u64 = 0x00;
const ADCCON2: u64 = 0x04;
const ADCCON3: u64 = 0x08;
const ADCL: u64 = 0x0C;
const ADCH: u64 = 0x10;
const RNDL: u64 = 0x14;
const RNDH: u64 = 0x18;
const CMPCTL: u64 = 0x24;

const ADCCON1_EOC: u32 = 1 << 7;
const ADCCON1_ST: u32 = 1 << 6;
const ADCCON1_STSEL: u32 = 0x30;
const ADCCON1_RCTRL: u32 = 0x0C;
/// RCTRL = 01: clock the LFSR once.
const RCTRL_CLOCK_ONCE: u32 = 0x04;
/// Writable ADCCON1 bits: STSEL, RCTRL and the RW reserved bits 1:0.
const ADCCON1_RW: u32 = ADCCON1_STSEL | ADCCON1_RCTRL | 0x03;
const ADCCON1_RESET: u32 = 0x33;
const ADCCON2_RESET: u32 = 0x10;
const CMPCTL_EN: u32 = 1 << 1;

/// Internal reference, mV: the 1.19 V Contiki-NG's `vdd3-sensor.c` converts
/// with (`raw * 3 * 1190 / (2047 << 4)`).
pub const INTERNAL_REF_MV: u32 = 1190;
/// AVDD5 when nothing routes a reference voltage: the CC2538DK 3.0 V rail.
pub const DEFAULT_AVDD_MV: u32 = 3000;
/// Temperature-sensor level at 25 °C, in µV: 1422/2047 of the internal
/// reference, so the 12-bit internal-reference code is 1422.
const TEMP_SENSOR_25C_UV: i64 = 1422 * INTERNAL_REF_MV as i64 * 1000 / 2047;

/// Figure 16-1: x^16 + x^15 + x^2 + 1.
const LFSR_POLY: u16 = 0x8005;
/// "13× unrolling" (§16.2.1).
const RNG_CLOCK_SHIFTS: u32 = 13;

const AIN_CHANNELS: usize = 8;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cc2538SocAdc {
    adccon1: u32,
    adccon2: u32,
    adccon3: u32,
    /// ADCH:ADCL, MSB-aligned two's-complement.
    result: u16,
    /// ADCCON1.EOC: set by a conversion, cleared by a read of ADCH.
    eoc: Cell<bool>,
    lfsr: u16,
    cmpctl_en: bool,
    /// AIN0..AIN7, mV.
    ain_mv: [u32; AIN_CHANNELS],
    avdd_mv: u32,
    irq_pending: bool,
}

impl Default for Cc2538SocAdc {
    fn default() -> Self {
        Self::new()
    }
}

/// One LFSR shift with `in_bit` (Figure 16-1).
fn lfsr_shift(lfsr: u16, in_bit: bool) -> u16 {
    let feedback = (lfsr & 0x8000 != 0) ^ in_bit;
    let shifted = lfsr << 1;
    if feedback {
        shifted ^ LFSR_POLY
    } else {
        shifted
    }
}

/// ENOB bits for a DIV field (§15.3: 64/128/256/512 → 7/9/10/12).
fn div_bits(div: u32) -> u32 {
    match div & 0x3 {
        0 => 7,
        1 => 9,
        2 => 10,
        _ => 12,
    }
}

impl Cc2538SocAdc {
    pub fn new() -> Self {
        Self {
            adccon1: ADCCON1_RESET,
            adccon2: ADCCON2_RESET,
            adccon3: 0,
            result: 0,
            eoc: Cell::new(false),
            lfsr: 0xFFFF,
            cmpctl_en: false,
            ain_mv: [0; AIN_CHANNELS],
            avdd_mv: DEFAULT_AVDD_MV,
            irq_pending: false,
        }
    }

    fn reset_registers(&mut self) {
        // Analog inputs and the rail are the outside world, not registers.
        let (ain_mv, avdd_mv) = (self.ain_mv, self.avdd_mv);
        *self = Self::new();
        self.ain_mv = ain_mv;
        self.avdd_mv = avdd_mv;
    }

    fn ain_uv(&self, n: usize) -> i64 {
        i64::from(self.ain_mv[n]) * 1000
    }

    /// The converted voltage of `channel`, µV (§15.2.1).
    fn channel_uv(&self, channel: u32) -> i64 {
        match channel & 0xF {
            c @ 0..=7 => self.ain_uv(c as usize),
            c @ 8..=11 => {
                let pair = 2 * (c as usize - 8);
                self.ain_uv(pair) - self.ain_uv(pair + 1)
            }
            14 => TEMP_SENSOR_25C_UV,
            15 => i64::from(self.avdd_mv) * 1000 / 3,
            // 12 = GND, 13 = reserved.
            _ => 0,
        }
    }

    /// The selected positive reference, µV (§15.2.6).
    fn reference_uv(&self, sref: u32) -> i64 {
        match sref & 0x3 {
            0 => i64::from(INTERNAL_REF_MV) * 1000,
            1 => self.ain_uv(7),
            2 => i64::from(self.avdd_mv) * 1000,
            _ => self.ain_uv(6) - self.ain_uv(7),
        }
    }

    /// One conversion with an ADCCONx-coded control byte (REF 7:6, DIV 5:4,
    /// CH 3:0); latches ADCH:ADCL and sets EOC.
    fn convert(&mut self, control: u32) {
        let bits = div_bits(control >> 4);
        let full = (1i64 << (bits - 1)) - 1;
        let vin = self.channel_uv(control);
        let vref = self.reference_uv(control >> 6);
        let code = if vref > 0 {
            // Round half away from zero.
            let scaled = vin * full;
            let half = vref / 2;
            if scaled >= 0 {
                (scaled + half) / vref
            } else {
                (scaled - half) / vref
            }
        } else {
            // No usable reference: the converter saturates by sign.
            vin.signum() * (full + 1)
        };
        let code = code.clamp(-(full + 1), full);
        self.result = ((code as i16 as u16) << (16 - bits)) & 0xFFFC;
        self.eoc.set(true);
    }

    /// §15.2.2: the ST-started conversion sequence.
    fn run_sequence(&mut self) {
        let sch = self.adccon2 & 0xF;
        let ctrl = self.adccon2 & 0xF0;
        let channels: Vec<u32> = match sch {
            0..=7 => (0..=sch).collect(),
            8..=11 => (8..=sch).collect(),
            _ => vec![sch],
        };
        for ch in channels {
            self.convert(ctrl | ch);
        }
        if sch < 12 {
            self.adccon2 = ctrl | (sch + 1);
        }
    }

    fn comparator_output(&self) -> bool {
        !self.cmpctl_en || self.ain_mv[5] > self.ain_mv[4]
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            ADCCON1 => self.adccon1 | if self.eoc.get() { ADCCON1_EOC } else { 0 },
            ADCCON2 => self.adccon2,
            ADCCON3 => self.adccon3,
            ADCL => u32::from(self.result & 0xFC),
            ADCH => {
                self.eoc.set(false);
                u32::from(self.result >> 8)
            }
            RNDL => u32::from(self.lfsr & 0xFF),
            RNDH => u32::from(self.lfsr >> 8),
            CMPCTL => {
                (if self.cmpctl_en { CMPCTL_EN } else { 0 }) | u32::from(self.comparator_output())
            }
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let value = value & 0xFF;
        match offset {
            ADCCON1 => {
                self.adccon1 = value & ADCCON1_RW;
                if self.adccon1 & ADCCON1_RCTRL == RCTRL_CLOCK_ONCE {
                    for _ in 0..RNG_CLOCK_SHIFTS {
                        self.lfsr = lfsr_shift(self.lfsr, false);
                    }
                    // "automatically returns to 00 when the operation completes"
                    self.adccon1 &= !ADCCON1_RCTRL;
                }
                if value & ADCCON1_ST != 0 && self.adccon1 & ADCCON1_STSEL == ADCCON1_STSEL {
                    // ST reads 1 until the sequence completes; it completes here.
                    self.run_sequence();
                }
            }
            ADCCON2 => self.adccon2 = value,
            ADCCON3 => {
                self.adccon3 = value;
                self.convert(value);
                self.irq_pending = true;
            }
            RNDL => self.lfsr = (self.lfsr << 8) | value as u16,
            RNDH => {
                for bit in (0..8).rev() {
                    self.lfsr = lfsr_shift(self.lfsr, value & (1 << bit) != 0);
                }
            }
            CMPCTL => self.cmpctl_en = value & CMPCTL_EN != 0,
            _ => {}
        }
    }
}

impl Peripheral for Cc2538SocAdc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Every field is in byte 0; upper lanes are reserved and a read of
        // them has no side effect.
        if offset & 3 != 0 {
            return Ok(0);
        }
        Ok(self.read_reg(offset) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_reg(offset, u32::from(value));
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        match self.ain_mv.get_mut(channel as usize) {
            Some(slot) => {
                *slot = u32::from(millivolts);
                true
            }
            None => false,
        }
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        self.set_adc_channel_input(channel, 0)
    }

    fn adc_channel_count(&self) -> Option<u8> {
        Some(AIN_CHANNELS as u8)
    }

    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }

    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || volts <= 0.0 {
            return false;
        }
        self.avdd_mv = (volts * 1000.0).round() as u32;
        true
    }

    /// The ADC interrupt is a pulse per ADCCON3 conversion; there is no
    /// status flag to clear.
    fn tick_elapsed(&mut self, _cycles: u64) -> crate::PeripheralTickResult {
        crate::PeripheralTickResult {
            irq: std::mem::take(&mut self.irq_pending),
            ..Default::default()
        }
    }

    /// Only an undelivered conversion pulse needs the walk; it is raised by
    /// this block's own MMIO, after which the bus re-checks this predicate.
    fn legacy_tick_active(&self) -> bool {
        self.irq_pending
    }

    fn on_system_reset(&mut self) {
        self.reset_registers();
    }

    fn on_node_reset(&mut self) {
        self.reset_registers();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        *self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Cc2538SocAdc snapshot decode: {error}"))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADCCONx control byte: REF, DIV, CH.
    fn ctl(reference: u32, div: u32, ch: u32) -> u32 {
        (reference << 6) | (div << 4) | ch
    }

    /// Contiki-NG `adc_get`: the 16-bit MSB-aligned result.
    fn single(adc: &mut Cc2538SocAdc, control: u32) -> i16 {
        adc.write_u32(ADCCON3, control).unwrap();
        assert_ne!(adc.read_u32(ADCCON1).unwrap() & ADCCON1_EOC, 0);
        let lo = adc.read_u32(ADCL).unwrap() & 0xFC;
        let hi = adc.read_u32(ADCH).unwrap();
        assert_eq!(adc.read_u32(ADCCON1).unwrap() & ADCCON1_EOC, 0);
        ((hi << 8) | lo) as u16 as i16
    }

    #[test]
    fn reset_values_follow_tables_15_16_17() {
        let adc = Cc2538SocAdc::new();
        assert_eq!(adc.read_u32(ADCCON1).unwrap(), 0x33);
        assert_eq!(adc.read_u32(ADCCON2).unwrap(), 0x10);
        assert_eq!(adc.read_u32(ADCCON3).unwrap(), 0);
        assert_eq!(adc.read_u32(ADCL).unwrap(), 0);
        assert_eq!(adc.read_u32(ADCH).unwrap(), 0);
        assert_eq!(adc.read_u32(RNDL).unwrap(), 0xFF);
        assert_eq!(adc.read_u32(RNDH).unwrap(), 0xFF);
        assert_eq!(adc.read_u32(CMPCTL).unwrap(), 0x01);
        assert_eq!(adc.read_u32(0x1C).unwrap(), 0);
    }

    #[test]
    fn single_conversion_codes_follow_15_2_5() {
        let mut adc = Cc2538SocAdc::new();
        // VDD/3 = 1000 mV against 1.19 V, 12 bits: round(1000/1190*2047) = 1720.
        assert_eq!(single(&mut adc, ctl(0, 3, 15)) >> 4, 1720);
        // GND.
        assert_eq!(single(&mut adc, ctl(0, 3, 12)), 0);
        // Temperature sensor at 25 °C: Contiki-NG's 1422 offset.
        assert_eq!(single(&mut adc, ctl(0, 3, 14)) >> 4, 1422);
        // Input at VREF reads the 12-bit maximum, 2047; above it saturates.
        adc.set_adc_channel_input(2, 3000);
        assert_eq!(single(&mut adc, ctl(2, 3, 2)) >> 4, 2047);
        adc.set_adc_channel_input(2, 3300);
        assert_eq!(single(&mut adc, ctl(2, 3, 2)) >> 4, 2047);
        // 7-bit decimation (64): AVDD/2 against AVDD → round(0.5*63) = 32.
        adc.set_adc_channel_input(2, 1500);
        assert_eq!(single(&mut adc, ctl(2, 0, 2)) >> 9, 32);
        // Differential AIN0-AIN1 is negatively signed.
        adc.set_adc_channel_input(0, 0);
        adc.set_adc_channel_input(1, 3000);
        assert_eq!(single(&mut adc, ctl(2, 3, 8)) >> 4, -2047);
        // External AIN7 reference.
        adc.set_adc_channel_input(7, 2000);
        adc.set_adc_channel_input(3, 1000);
        assert_eq!(single(&mut adc, ctl(1, 3, 3)) >> 4, 1024);
    }

    #[test]
    fn single_conversion_pulses_the_irq_once() {
        let mut adc = Cc2538SocAdc::new();
        assert!(!adc.legacy_tick_active());
        adc.write_u32(ADCCON3, ctl(0, 3, 15)).unwrap();
        assert!(adc.legacy_tick_active());
        assert!(adc.tick_elapsed(1).irq);
        assert!(!adc.tick_elapsed(1).irq);
        assert!(!adc.legacy_tick_active());
    }

    #[test]
    fn st_sequence_runs_only_with_stsel_11_and_raises_no_irq() {
        let mut adc = Cc2538SocAdc::new();
        adc.set_adc_channel_input(0, 500);
        adc.set_adc_channel_input(1, 1000);
        adc.set_adc_channel_input(2, 1190);
        // AIN0..AIN2, internal ref, 12 bits.
        adc.write_u32(ADCCON2, ctl(0, 3, 2)).unwrap();
        // STSEL = 01: no ST start.
        adc.write_u32(ADCCON1, 0x10 | ADCCON1_ST).unwrap();
        assert_eq!(adc.read_u32(ADCCON1).unwrap() & ADCCON1_EOC, 0);
        adc.write_u32(ADCCON1, 0x30 | ADCCON1_ST).unwrap();
        let con1 = adc.read_u32(ADCCON1).unwrap();
        assert_eq!(
            con1 & ADCCON1_ST,
            0,
            "ST clears when the sequence completes"
        );
        assert_ne!(con1 & ADCCON1_EOC, 0);
        // The last channel's result; SCH reads last + 1.
        assert_eq!(adc.read_u32(ADCH).unwrap(), 2047 >> 4);
        assert_eq!(adc.read_u32(ADCCON2).unwrap() & 0xF, 3);
        assert!(!adc.tick_elapsed(1).irq);
        // SCH >= 12 reads back unchanged.
        adc.write_u32(ADCCON2, ctl(0, 3, 15)).unwrap();
        adc.write_u32(ADCCON1, 0x30 | ADCCON1_ST).unwrap();
        assert_eq!(adc.read_u32(ADCCON2).unwrap() & 0xF, 15);
    }

    #[test]
    fn lfsr_fixed_points_and_clock_once() {
        for seed in [0x0000u16, 0x8003] {
            let mut adc = Cc2538SocAdc::new();
            adc.write_u32(RNDL, u32::from(seed >> 8)).unwrap();
            adc.write_u32(RNDL, u32::from(seed & 0xFF)).unwrap();
            adc.write_u32(ADCCON1, 0x30 | RCTRL_CLOCK_ONCE).unwrap();
            let value = adc.read_u32(RNDL).unwrap() | (adc.read_u32(RNDH).unwrap() << 8);
            assert_eq!(value as u16, seed, "§16.2.2 NOTE fixed point");
        }
        let mut adc = Cc2538SocAdc::new();
        adc.write_u32(RNDL, 0x12).unwrap();
        adc.write_u32(RNDL, 0x34).unwrap();
        assert_eq!(adc.read_u32(RNDH).unwrap(), 0x12);
        assert_eq!(adc.read_u32(RNDL).unwrap(), 0x34);
        adc.write_u32(ADCCON1, 0x30 | RCTRL_CLOCK_ONCE).unwrap();
        assert_eq!(
            adc.read_u32(ADCCON1).unwrap() & ADCCON1_RCTRL,
            0,
            "RCTRL returns to 00"
        );
        let mut expected = 0x1234u16;
        for _ in 0..13 {
            expected = lfsr_shift(expected, false);
        }
        let value = adc.read_u32(RNDL).unwrap() | (adc.read_u32(RNDH).unwrap() << 8);
        assert_eq!(value as u16, expected);
        assert_ne!(expected, 0x1234);
        // RCTRL = 11 (stopped) does not clock.
        adc.write_u32(ADCCON1, 0x3C).unwrap();
        let again = adc.read_u32(RNDL).unwrap() | (adc.read_u32(RNDH).unwrap() << 8);
        assert_eq!(again, value);
    }

    #[test]
    fn rndh_writes_compute_crc16_0x8005_msb_first() {
        // x^16 + x^15 + x^2 + 1, MSB first, init 0: CRC-16/UMTS "123456789" = 0xFEE8.
        let mut adc = Cc2538SocAdc::new();
        adc.write_u32(RNDL, 0).unwrap();
        adc.write_u32(RNDL, 0).unwrap();
        for byte in b"123456789" {
            adc.write_u32(RNDH, u32::from(*byte)).unwrap();
        }
        let crc = adc.read_u32(RNDL).unwrap() | (adc.read_u32(RNDH).unwrap() << 8);
        assert_eq!(crc, 0xFEE8);
    }

    #[test]
    fn comparator_follows_pa5_over_pa4_when_enabled() {
        let mut adc = Cc2538SocAdc::new();
        adc.set_adc_channel_input(4, 1000);
        adc.set_adc_channel_input(5, 500);
        assert_eq!(adc.read_u32(CMPCTL).unwrap(), 0x01);
        adc.write_u32(CMPCTL, 0xFF).unwrap();
        assert_eq!(adc.read_u32(CMPCTL).unwrap(), 0x02);
        adc.set_adc_channel_input(5, 1500);
        assert_eq!(adc.read_u32(CMPCTL).unwrap(), 0x03);
    }

    #[test]
    fn reset_keeps_inputs_and_snapshot_round_trips() {
        let mut adc = Cc2538SocAdc::new();
        adc.set_adc_channel_input(3, 700);
        adc.write_u32(ADCCON3, ctl(0, 3, 3)).unwrap();
        adc.write_u32(RNDL, 0x55).unwrap();
        let saved = adc.snapshot();
        adc.on_system_reset();
        assert_eq!(adc.read_u32(ADCCON1).unwrap(), 0x33);
        assert_eq!(adc.read_u32(RNDL).unwrap(), 0xFF);
        assert_eq!(single(&mut adc, ctl(0, 3, 3)) >> 4, 1204);
        adc.restore(saved).unwrap();
        assert_eq!(adc.read_u32(RNDL).unwrap(), 0x55);
        assert_ne!(adc.read_u32(ADCCON1).unwrap() & ADCCON1_EOC, 0);
    }

    #[test]
    fn byte_reads_of_upper_lanes_do_not_clear_eoc() {
        let mut adc = Cc2538SocAdc::new();
        adc.write(ADCCON3 as u64, ctl(0, 3, 15) as u8).unwrap();
        adc.read(ADCH + 1).unwrap();
        assert_ne!(adc.read(ADCCON1).unwrap() as u32 & ADCCON1_EOC, 0);
        adc.read(ADCH).unwrap();
        assert_eq!(adc.read(ADCCON1).unwrap() as u32 & ADCCON1_EOC, 0);
    }
}
