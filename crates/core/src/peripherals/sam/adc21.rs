// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Opt-in SAMD21 ADC slice (DS40001882 33; register map from CMSIS
//! `component/adc.h`): single-ended software-triggered conversions of AINn
//! against GND at 8/10/12-bit resolution, with gain 1x/2x/4x/8x/16x/DIV2 and
//! the internal references (REFSEL INT1V = 1.0 V, INTVCC0 = VDDANA / 1.48,
//! INTVCC1 = VDDANA / 2). A conversion started by SWTRIG.START completes on the
//! next bus tick, and only while the ADC is enabled, its APB clock runs
//! (PM.APBCMASK bit 16) and the input's pad is handed to the ADC (PMUX
//! function B). The SAMD21 GCLK.CLKCTRL is an indirect register declared as a
//! plain bank, so the generic clock is not checked. The code is
//! floor(V * gain * 2^n / Vref), clamped. Input voltages are set by the host
//! or the board (`input_mv`); nothing is sampled from a modelled analog front
//! end (no settling, noise, gain/offset correction, averaging, windowing,
//! scan, events, external references or differential mode — those are
//! refused, not ignored).
use crate::{Bus, Peripheral, SimResult, SimulationError};
use std::any::Any;

/// PM.APBCMASK; the ADC is bit 16 (CMSIS `PM_APBCMASK_ADC_Pos`).
const PM_APBCMASK: u64 = 0x40000420;
const APBC_ADC: u32 = 1 << 16;
/// PORT group A; group B follows at +0x80.
const PORT: u64 = 0x41004400;
const START: u8 = 1 << 1;
const RESRDY: u8 = 1;
/// MUXNEG = GND (0x18) and IOGND (0x19) are the single-ended negatives.
const MUXNEG_GND: u32 = 0x18;
const MUXNEG_IOGND: u32 = 0x19;
const AIN_COUNT: usize = 20;

/// (AIN, port, pin) for function B on the ATSAMD21G18A, from CMSIS
/// `pio/samd21g18a.h` (`PIN_PxyyB_ADC_AINn`). AIN8/9 and AIN12..15 are not
/// bonded on the 48-pin G package.
pub const SAMD21G18A_AIN_PADS: &[(u8, u8, u8)] = &[
    (0, 0, 2),
    (1, 0, 3),
    (2, 1, 8),
    (3, 1, 9),
    (4, 0, 4),
    (5, 0, 5),
    (6, 0, 6),
    (7, 0, 7),
    (10, 1, 2),
    (11, 1, 3),
    (16, 0, 8),
    (17, 0, 9),
    (18, 0, 10),
    (19, 0, 11),
];

#[derive(Debug)]
pub struct Samd21Adc {
    ctrla: u8,
    refctrl: u8,
    avgctrl: u8,
    sampctrl: u8,
    ctrlb: u16,
    inputctrl: u32,
    inten: u8,
    intflag: u8,
    result: u16,
    calib: u16,
    dbgctrl: u8,
    pending: bool,
    /// Analog levels per AIN, millivolts (the outside world, not a register).
    inputs: [u32; AIN_COUNT],
    vddana_mv: u32,
    conversions: u64,
}

impl Default for Samd21Adc {
    fn default() -> Self {
        Self::new()
    }
}

impl Samd21Adc {
    pub fn new() -> Self {
        Self {
            ctrla: 0,
            refctrl: 0,
            avgctrl: 0,
            sampctrl: 0,
            ctrlb: 0,
            inputctrl: 0,
            inten: 0,
            intflag: 0,
            result: 0,
            calib: 0,
            dbgctrl: 0,
            pending: false,
            inputs: [0; AIN_COUNT],
            vddana_mv: 3300,
            conversions: 0,
        }
    }
    /// Drive AIN `ain` with `millivolts`.
    pub fn set_input_millivolts(&mut self, ain: usize, millivolts: u32) {
        if ain < AIN_COUNT {
            self.inputs[ain] = millivolts;
        }
    }
    pub fn conversions(&self) -> u64 {
        self.conversions
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd21 ADC slice: {what}"
        )))
    }
    fn reset_registers(&mut self) {
        let (inputs, vddana_mv, conversions) = (self.inputs, self.vddana_mv, self.conversions);
        *self = Self {
            inputs,
            vddana_mv,
            conversions,
            ..Self::new()
        };
    }
    fn enabled(&self) -> bool {
        self.ctrla & 2 != 0
    }
    fn bits(&self) -> SimResult<u32> {
        match (self.ctrlb >> 4) & 3 {
            0 => Ok(12),
            2 => Ok(10),
            3 => Ok(8),
            _ => Self::refuse("16-bit (averaging) resolution"),
        }
    }
    /// Gain as (numerator, denominator).
    fn gain(&self) -> SimResult<(u64, u64)> {
        match (self.inputctrl >> 24) & 0xf {
            g @ 0..=4 => Ok((1 << g, 1)),
            0xf => Ok((1, 2)),
            _ => Self::refuse("reserved GAIN"),
        }
    }
    fn vref_mv(&self) -> SimResult<u32> {
        match self.refctrl & 0xf {
            0 => Ok(1000),
            1 => Ok(self.vddana_mv * 100 / 148),
            2 => Ok(self.vddana_mv / 2),
            _ => Self::refuse("external references (VREFA/VREFB)"),
        }
    }
    fn convert(&mut self, bus: &dyn Bus) -> SimResult<bool> {
        let clocked = bus.read_u32(PM_APBCMASK).is_ok_and(|m| m & APBC_ADC != 0);
        if !self.enabled() || !clocked {
            return Ok(false); // the conversion stays pending
        }
        let ain = (self.inputctrl & 0x1f) as u8;
        let Some(&(_, port, pin)) = SAMD21G18A_AIN_PADS.iter().find(|i| i.0 == ain) else {
            return Self::refuse("MUXPOS outside the bonded pad inputs");
        };
        let group = PORT + 0x80 * u64::from(port);
        let pincfg = bus.read_u8(group + 0x40 + u64::from(pin)).unwrap_or(0);
        let pmux = bus.read_u8(group + 0x30 + u64::from(pin / 2)).unwrap_or(0);
        let function = if pin & 1 != 0 { pmux >> 4 } else { pmux & 0xf };
        let vref = u64::from(self.vref_mv()?);
        let (gn, gd) = self.gain()?;
        let bits = self.bits()?;
        // A pad not handed to the ADC carries no signal to the input.
        let mv = if pincfg & 1 != 0 && function == 1 {
            u64::from(self.inputs[usize::from(ain)])
        } else {
            0
        };
        let full = (1u64 << bits) - 1;
        self.result = (mv * gn * (1 << bits) / (gd * vref)).min(full) as u16;
        self.intflag |= RESRDY;
        self.conversions += 1;
        Ok(true)
    }
    /// Register base and width for an offset, per CMSIS `component/adc.h`.
    fn layout(offset: u64) -> Option<(u64, u64)> {
        Some(match offset {
            0x00..=0x03 | 0x08 | 0x0c | 0x14 | 0x16..=0x19 | 0x2a => (offset, 1),
            0x04..=0x05 => (0x04, 2),
            0x10..=0x13 => (0x10, 4),
            0x1a..=0x1b => (0x1a, 2),
            0x1c..=0x1d => (0x1c, 2),
            0x20..=0x21 => (0x20, 2),
            0x24..=0x25 => (0x24, 2),
            0x26..=0x27 => (0x26, 2),
            0x28..=0x29 => (0x28, 2),
            _ => return None,
        })
    }
    fn read_reg(&self, base: u64) -> u32 {
        match base {
            0x00 => u32::from(self.ctrla),
            0x01 => u32::from(self.refctrl),
            0x02 => u32::from(self.avgctrl),
            0x03 => u32::from(self.sampctrl),
            0x04 => u32::from(self.ctrlb),
            0x10 => self.inputctrl,
            0x16 | 0x17 => u32::from(self.inten),
            0x18 => u32::from(self.intflag),
            0x1a => u32::from(self.result),
            0x28 => u32::from(self.calib),
            0x2a => u32::from(self.dbgctrl),
            // STATUS.SYNCBUSY: register writes synchronise at once. SWTRIG,
            // WINCTRL, EVCTRL, the window and correction registers hold only
            // their reset zeros (non-zero writes are refused).
            _ => 0,
        }
    }
    fn write_reg(&mut self, base: u64, value: u32) -> SimResult<()> {
        match base {
            0x00 if value & 1 != 0 => self.reset_registers(),
            // ENABLE, RUNSTDBY.
            0x00 if value & !0x06 == 0 => self.ctrla = value as u8,
            // REFSEL and REFCOMP; the reference is checked at conversion.
            0x01 if value & !0x8f == 0 => self.refctrl = value as u8,
            // One sample, no adjust.
            0x02 if value == 0 => self.avgctrl = 0,
            0x03 => self.sampctrl = value as u8 & 0x3f,
            // RESSEL and PRESCALER only (no differential, left adjust,
            // freerun or correction).
            0x04 if value & !0x0730 == 0 => self.ctrlb = value as u16,
            0x08 | 0x14 | 0x1c | 0x20 | 0x24 | 0x26 if value == 0 => {}
            0x0c if value & !u32::from(START) == 0 => {
                if value & u32::from(START) != 0 {
                    self.intflag &= !RESRDY;
                    self.pending = true;
                }
            }
            // MUXPOS, MUXNEG = GND/IOGND, GAIN; no scan or offset.
            0x10 if value & 0x00ff_0000 == 0
                && value & 0xf0e0_e0e0 == 0
                && matches!((value >> 8) & 0x1f, MUXNEG_GND | MUXNEG_IOGND) =>
            {
                self.inputctrl = value
            }
            0x16 => self.inten &= !(value as u8),
            0x17 => self.inten |= value as u8 & 0x0f,
            0x18 => self.intflag &= !(value as u8),
            // LINEARITY_CAL[7:0], BIAS_CAL[10:8].
            0x28 => self.calib = value as u16 & 0x07ff,
            0x2a => self.dbgctrl = value as u8 & 1,
            _ => return Self::refuse("store outside the conversion slice"),
        }
        Ok(())
    }
    fn write_sized(&mut self, offset: u64, value: u32, size: u64) -> SimResult<()> {
        let Some((base, width)) = Self::layout(offset) else {
            return Self::refuse("store to a reserved offset");
        };
        if offset == base && size == width {
            return self.write_reg(base, value);
        }
        if offset - base + size > width {
            return Self::refuse("store spanning two registers");
        }
        // A narrower store into a wider register merges with its other bytes.
        let shift = 8 * (offset - base);
        let mask = (u32::MAX >> (32 - 8 * size)) << shift;
        let merged = (self.read_reg(base) & !mask) | ((value << shift) & mask);
        self.write_reg(base, merged)
    }
    fn read_sized(&self, offset: u64, size: u64) -> SimResult<u32> {
        let Some((base, width)) = Self::layout(offset) else {
            return Self::refuse("load from a reserved offset");
        };
        if offset - base + size > width {
            // A wide load over several narrow registers reads each byte from
            // its own register (reserved bytes read 0), as the APB bridge does:
            // a 32-bit read at 0x00 returns CTRLA..SAMPCTRL.
            let mut v = 0u32;
            for i in 0..size {
                if let Some((b, _)) = Self::layout(offset + i) {
                    v |= ((self.read_reg(b) >> (8 * (offset + i - b))) & 0xff) << (8 * i);
                }
            }
            return Ok(v);
        }
        let v = self.read_reg(base) >> (8 * (offset - base));
        Ok(if size >= 4 {
            v
        } else {
            v & ((1 << (8 * size)) - 1)
        })
    }
}

impl Peripheral for Samd21Adc {
    fn on_system_reset(&mut self) {
        // Registers to power-on; the analog inputs and the conversion count
        // are the outside world and evidence, not chip state.
        self.reset_registers();
    }
    /// No `tick()` work: state moves on MMIO and the bus tick only.
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.read_sized(offset, 1)? as u8)
    }
    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(self.read_sized(offset, 2)? as u16)
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.read_sized(offset, 4)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_sized(offset, u32::from(value), 1)
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write_sized(offset, u32::from(value), 2)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_sized(offset, value, 4)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn needs_bus_tick(&self) -> bool {
        self.pending
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if let Ok(true) = self.convert(bus) {
            self.pending = false;
        }
    }
    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if usize::from(channel) >= AIN_COUNT {
            return false;
        }
        self.inputs[usize::from(channel)] = u32::from(millivolts);
        true
    }
    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        self.set_adc_channel_input(channel, 0)
    }
    fn adc_channel_count(&self) -> Option<u8> {
        Some(AIN_COUNT as u8)
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "samd21_adc", "ctrla": self.ctrla,
            "refctrl": self.refctrl, "ctrlb": self.ctrlb, "inputctrl": self.inputctrl,
            "result": self.result, "conversions": self.conversions})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ArduinoCore-samd init(): 10-bit, MUXNEG GND, AR_DEFAULT = GAIN DIV2
    /// against INTVCC1, so full scale is VDDANA.
    fn arduino_default() -> Samd21Adc {
        let mut a = Samd21Adc::new();
        a.write_u16(0x04, 0x0700 | (2 << 4)).unwrap(); // DIV512, 10-bit
        a.write(0x03, 0x3f).unwrap();
        a.write_u32(0x10, 0x18 << 8).unwrap();
        a.write(0x02, 0).unwrap();
        a.write_u32(0x10, (0xf << 24) | (0x18 << 8)).unwrap();
        a.write(0x01, 2).unwrap();
        a
    }

    #[test]
    fn arduino_default_full_scale_is_vddana() {
        let mut a = arduino_default();
        a.set_input_millivolts(0, 1650);
        assert_eq!(a.vref_mv().unwrap(), 1650);
        assert_eq!(a.gain().unwrap(), (1, 2));
        assert_eq!(a.bits().unwrap(), 10);
        // 1650 mV * 1/2 * 1024 / 1650 mV = 512.
        let (gn, gd) = a.gain().unwrap();
        assert_eq!(1650 * gn * 1024 / (gd * 1650), 512);
    }

    #[test]
    fn a_word_load_over_byte_registers_reads_each_byte() {
        let mut a = Samd21Adc::new();
        a.write(0x01, 0x02).unwrap(); // REFCTRL INTVCC0
        a.write(0x03, 0x05).unwrap(); // SAMPCTRL
        assert_eq!(a.read_u32(0x00).unwrap(), 0x0500_0200);
        // CTRLB (16-bit) then two reserved bytes.
        a.write_u16(0x04, 0x0020).unwrap(); // RESSEL 8-bit
        assert_eq!(a.read_u32(0x04).unwrap(), 0x0020);
    }

    #[test]
    fn modes_outside_the_slice_are_refused() {
        let mut a = arduino_default();
        assert!(a.write_u16(0x04, 1).is_err(), "differential");
        assert!(a.write_u16(0x04, 1 << 2).is_err(), "freerun");
        assert!(a.write_u32(0x10, 0x0000_0005).is_err(), "MUXNEG = AIN0");
        assert!(a.write_u32(0x10, 0x0001_1805).is_err(), "INPUTSCAN");
        assert!(a.write(0x02, 1).is_err(), "averaging");
        a.write(0x01, 3).unwrap();
        assert!(a.vref_mv().is_err(), "VREFA is checked at conversion");
    }

    #[test]
    fn byte_store_into_inputctrl_merges() {
        let mut a = arduino_default();
        a.write(0x10, 0x12).unwrap(); // MUXPOS = AIN18 by byte
        assert_eq!(a.read_u32(0x10).unwrap(), 0x0f00_1812);
        assert_eq!(a.read(0x13).unwrap(), 0x0f);
    }

    #[test]
    fn swtrig_start_clears_resrdy_and_pends() {
        let mut a = arduino_default();
        a.intflag = RESRDY;
        a.write(0x0c, START).unwrap();
        assert_eq!(a.read(0x18).unwrap() & RESRDY, 0);
        assert!(a.needs_bus_tick());
        a.write(0x00, 1).unwrap(); // SWRST
        assert_eq!(a.read_u16(0x04).unwrap(), 0);
    }
}
