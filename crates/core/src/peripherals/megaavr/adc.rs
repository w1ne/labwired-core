// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! megaAVR 0-series ADC0 (ATmega4809 data sheet DS40002015, section 29;
//! register map from the ATDF `ADC` module): single-ended conversions of
//! AIN0..AIN15 (and GND) started by COMMAND.STCONV, at 10- or 8-bit
//! resolution, with SAMPNUM accumulation, against VDD or the VREF block's
//! internal ADC0 reference (VREF.CTRLA.ADC0REFSEL: 0.55/1.1/1.5/2.5/4.3 V).
//! A conversion completes on the next bus tick while CTRLA.ENABLE is set;
//! RESRDY clears on a write of 1 or on a read of RES, as on silicon.
//!
//! The code is floor(V * 2^n / Vref), clamped, times the sample count.
//! Input voltages come from the board (`input_mv`) or the host; the analog
//! input is wired to its pad whatever the PORT configuration, so no pad
//! state is consulted. Free running, window compare, event start, the
//! RESRDY/WCMP interrupts, VREFA, the temperature sensor and DACREF0 are
//! refused, not ignored. Sample timing (PRESC, SAMPCTRL, CTRLD) is stored
//! but not modelled: a conversion takes one bus tick.

use crate::{Bus, Peripheral, SimResult, SimulationError};
use std::any::Any;
use std::cell::Cell;

/// VREF.CTRLA in the 4809 data space (VREF at 0x00A0), behind the AVR
/// data-space window at 0x10000.
const VREF_CTRLA: u64 = 0x0001_00A0;
const AIN_COUNT: usize = 16;
const MUXPOS_GND: u8 = 0x1f;
const CTRLA_ENABLE: u8 = 1 << 0;
const CTRLA_FREERUN: u8 = 1 << 1;
const CTRLA_RESSEL_8BIT: u8 = 1 << 2;
const RESRDY: u8 = 1 << 0;
const STCONV: u8 = 1 << 0;

#[derive(Debug)]
pub struct MegaAvrAdc {
    ctrla: u8,
    ctrlb: u8,
    ctrlc: u8,
    ctrld: u8,
    sampctrl: u8,
    muxpos: u8,
    dbgctrl: u8,
    temp: u8,
    winlt: u16,
    winht: u16,
    calib: u8,
    res: u16,
    /// INTFLAGS; a `Cell` because reading RES clears RESRDY.
    intflags: Cell<u8>,
    pending: bool,
    /// Analog levels per AIN, millivolts (the outside world, not a register).
    inputs: [u32; AIN_COUNT],
    vdd_mv: u32,
    conversions: u64,
}

impl MegaAvrAdc {
    pub fn new(vdd_mv: u32) -> Self {
        Self {
            ctrla: 0,
            ctrlb: 0,
            ctrlc: 0,
            ctrld: 0,
            sampctrl: 0,
            muxpos: 0,
            dbgctrl: 0,
            temp: 0,
            winlt: 0,
            winht: 0,
            calib: 0,
            res: 0,
            intflags: Cell::new(0),
            pending: false,
            inputs: [0; AIN_COUNT],
            vdd_mv,
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
            "megaavr ADC0: {what}"
        )))
    }
    fn vref_mv(&self, bus: &dyn Bus) -> SimResult<u32> {
        match (self.ctrlc >> 4) & 3 {
            // INTREF: VREF.CTRLA.ADC0REFSEL[6:4].
            0 => match (bus.read_u8(VREF_CTRLA).unwrap_or(0) >> 4) & 7 {
                0 => Ok(550),
                1 => Ok(1100),
                2 => Ok(2500),
                3 => Ok(4300),
                4 => Ok(1500),
                _ => Self::refuse("reserved VREF.ADC0REFSEL"),
            },
            1 => Ok(self.vdd_mv),
            2 => Self::refuse("external reference (VREFA)"),
            _ => Self::refuse("reserved REFSEL"),
        }
    }
    fn convert(&mut self, bus: &dyn Bus) -> SimResult<bool> {
        if self.ctrla & CTRLA_ENABLE == 0 {
            return Ok(false); // the conversion stays pending
        }
        let mv = match self.muxpos {
            m if usize::from(m) < AIN_COUNT => u64::from(self.inputs[usize::from(m)]),
            MUXPOS_GND => 0,
            0x1c => return Self::refuse("DACREF0 input"),
            0x1e => return Self::refuse("temperature sensor input"),
            _ => return Self::refuse("reserved MUXPOS"),
        };
        let bits = if self.ctrla & CTRLA_RESSEL_8BIT != 0 {
            8
        } else {
            10
        };
        let vref = u64::from(self.vref_mv(bus)?);
        let code = (mv * (1 << bits) / vref).min((1 << bits) - 1);
        // SAMPNUM accumulates 2^n identical samples (the sources are static).
        self.res = (code << (self.ctrlb & 7)) as u16;
        self.intflags.set(self.intflags.get() | RESRDY);
        self.conversions += 1;
        Ok(true)
    }
    fn reg(&self, offset: u64) -> Option<u8> {
        Some(match offset {
            0x00 => self.ctrla,
            0x01 => self.ctrlb,
            0x02 => self.ctrlc,
            0x03 => self.ctrld,
            // CTRLE (WINCM), EVCTRL, INTCTRL: only zero is accepted.
            0x04 | 0x09 | 0x0a => 0,
            0x05 => self.sampctrl,
            0x06 => self.muxpos,
            0x08 => u8::from(self.pending),
            0x0b => self.intflags.get(),
            0x0c => self.dbgctrl,
            0x0d => self.temp,
            0x10 => self.res as u8,
            0x11 => (self.res >> 8) as u8,
            0x12 => self.winlt as u8,
            0x13 => (self.winlt >> 8) as u8,
            0x14 => self.winht as u8,
            0x15 => (self.winht >> 8) as u8,
            0x16 => self.calib,
            0x07 | 0x0e | 0x0f | 0x17 => 0,
            _ => return None,
        })
    }
}

impl Peripheral for MegaAvrAdc {
    fn on_system_reset(&mut self) {
        // Registers to power-on; the inputs, VDD and the conversion count are
        // the outside world and evidence, not chip state.
        let (inputs, vdd_mv, conversions) = (self.inputs, self.vdd_mv, self.conversions);
        *self = Self {
            inputs,
            conversions,
            ..Self::new(vdd_mv)
        };
    }
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let Some(v) = self.reg(offset) else {
            return Self::refuse("load outside the register map");
        };
        if matches!(offset, 0x10 | 0x11) {
            self.intflags.set(self.intflags.get() & !RESRDY);
        }
        Ok(v)
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.reg(offset)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            0x00 if value & CTRLA_FREERUN != 0 => return Self::refuse("free-running mode"),
            0x00 => self.ctrla = value & 0x85,
            0x01 if value & 7 <= 6 => self.ctrlb = value & 7,
            0x02 => self.ctrlc = value & 0x77,
            0x03 => self.ctrld = value,
            0x04 | 0x09 | 0x0a if value == 0 => {}
            0x04 => return Self::refuse("window compare"),
            0x09 => return Self::refuse("event-started conversions"),
            0x0a => return Self::refuse("RESRDY/WCMP interrupts"),
            0x05 => self.sampctrl = value & 0x1f,
            0x06 => self.muxpos = value & 0x1f,
            0x08 => {
                if value & STCONV != 0 {
                    self.intflags.set(self.intflags.get() & !RESRDY);
                    self.pending = true;
                }
            }
            0x0b => self.intflags.set(self.intflags.get() & !value),
            0x0c => self.dbgctrl = value & 1,
            0x0d => self.temp = value,
            0x12 => self.winlt = (self.winlt & 0xff00) | u16::from(value),
            0x13 => self.winlt = (self.winlt & 0x00ff) | (u16::from(value) << 8),
            0x14 => self.winht = (self.winht & 0xff00) | u16::from(value),
            0x15 => self.winht = (self.winht & 0x00ff) | (u16::from(value) << 8),
            0x16 => self.calib = value & 1,
            // RES is read-only; reserved bytes take nothing.
            0x07 | 0x0e | 0x0f | 0x10 | 0x11 | 0x17 => {}
            _ => return Self::refuse("store outside the register map"),
        }
        Ok(())
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
        serde_json::json!({"peripheral": "megaavr_adc", "ctrla": self.ctrla,
            "ctrlc": self.ctrlc, "muxpos": self.muxpos, "res": self.res,
            "conversions": self.conversions})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ArduinoCore-megaavr init(): PRESC DIV128, ENABLE (10-bit), then
    /// analogReference(VDD).
    fn arduino_default() -> MegaAvrAdc {
        let mut a = MegaAvrAdc::new(5000);
        a.write(0x02, 0x06).unwrap();
        a.write(0x00, CTRLA_ENABLE).unwrap();
        a.write(0x02, 0x16).unwrap();
        a
    }

    fn convert(a: &mut MegaAvrAdc, muxpos: u8) -> u16 {
        let bus = crate::bus::SystemBus::new();
        a.write(0x06, muxpos).unwrap();
        a.write(0x08, STCONV).unwrap();
        assert_eq!(a.read(0x0b).unwrap() & RESRDY, 0);
        assert!(a.convert(&bus).unwrap());
        assert_eq!(a.read(0x0b).unwrap() & RESRDY, RESRDY);
        let lo = a.read(0x10).unwrap();
        let hi = a.read(0x11).unwrap();
        assert_eq!(
            a.read(0x0b).unwrap() & RESRDY,
            0,
            "reading RES clears RESRDY"
        );
        u16::from(lo) | (u16::from(hi) << 8)
    }

    #[test]
    fn half_vdd_on_a0_reads_512() {
        let mut a = arduino_default();
        a.set_input_millivolts(3, 2500); // Nano Every A0 = PD3 = AIN3
        assert_eq!(convert(&mut a, 3), 512);
        assert_eq!(convert(&mut a, 2), 0, "an undriven input reads 0");
        assert_eq!(convert(&mut a, MUXPOS_GND), 0);
    }

    #[test]
    fn resolution_accumulation_and_clamp() {
        let mut a = arduino_default();
        a.set_input_millivolts(0, 6000);
        assert_eq!(convert(&mut a, 0), 1023, "above VREF clamps");
        a.set_input_millivolts(0, 2500);
        a.write(0x00, CTRLA_ENABLE | CTRLA_RESSEL_8BIT).unwrap();
        assert_eq!(convert(&mut a, 0), 128);
        a.write(0x00, CTRLA_ENABLE).unwrap();
        a.write(0x01, 2).unwrap(); // ACC4
        assert_eq!(convert(&mut a, 0), 2048);
    }

    #[test]
    fn disabled_adc_keeps_the_conversion_pending() {
        let bus = crate::bus::SystemBus::new();
        let mut a = MegaAvrAdc::new(5000);
        a.write(0x02, 0x10).unwrap();
        a.write(0x08, STCONV).unwrap();
        assert!(!a.convert(&bus).unwrap());
        assert!(a.needs_bus_tick());
        assert_eq!(a.read(0x08).unwrap(), STCONV);
    }

    #[test]
    fn modes_outside_the_model_are_refused() {
        let bus = crate::bus::SystemBus::new();
        let mut a = arduino_default();
        assert!(a.write(0x00, CTRLA_ENABLE | CTRLA_FREERUN).is_err());
        assert!(a.write(0x04, 1).is_err(), "window compare");
        assert!(a.write(0x09, 1).is_err(), "event start");
        assert!(a.write(0x0a, RESRDY).is_err(), "interrupts");
        a.write(0x06, 0x1e).unwrap();
        assert!(a.convert(&bus).is_err(), "temperature sensor");
        a.write(0x02, 0x26).unwrap(); // VREFA
        a.write(0x06, 0).unwrap();
        assert!(a.convert(&bus).is_err(), "VREFA is checked at conversion");
    }
}
