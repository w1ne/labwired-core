// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Opt-in SAMD51 DAC slice (DS60001507F 47): channel 0 conversions from DATA0
//! writes while the DAC is enabled (CTRLA.ENABLE, DACCTRL0.ENABLE) and its
//! generic clock (GCLK PCHCTRL[42]) runs; STATUS.READY0 reports a started
//! channel. Each conversion is recorded with its engine cycle as the channel's
//! output code. With the board's references declared (`vrefa_mv`, the VREFA
//! pad, and `vddana_mv`), `output_millivolts` gives the nominal level of a code,
//! DATA / 4095 x VREF for the REFSEL selected (DS60001507F 47.6.2): no gain,
//! offset or linearity error, settling or load. Refresh, dithering, the second
//! channel, differential mode, events, interrupts and DATABUF are not modelled
//! and their non-default settings are refused.
use crate::{Bus, Peripheral, SimResult, SimulationError};
use std::any::Any;

const GCLK_PCHCTRL_DAC: u64 = 0x40001c80 + 4 * 42;
/// Conversions kept for inspection; later ones are counted only.
const KEPT: usize = 1 << 16;

#[derive(Debug, Default)]
pub struct Samd51Dac {
    ctrla: u8,
    ctrlb: u8,
    dacctrl: [u16; 2],
    data0: u16,
    clocked: bool,
    clock: Option<crate::CycleClock>,
    elapsed: u64,
    conversions: Vec<(u64, u16)>,
    total: u64,
    /// Board references in millivolts: the VREFA pad and VDDANA.
    vrefa_mv: Option<f64>,
    vddana_mv: Option<f64>,
}

impl Samd51Dac {
    pub fn new() -> Self {
        Self::default()
    }
    /// The board's VREFA pad and VDDANA levels.
    pub fn with_references(mut self, vrefa_mv: Option<f64>, vddana_mv: Option<f64>) -> Self {
        self.vrefa_mv = vrefa_mv;
        self.vddana_mv = vddana_mv;
        self
    }
    /// Nominal output level of `code` for the selected reference: VREFA
    /// unbuffered or buffered (REFSEL 0, 2) or VDDANA (1). `None` for the
    /// internal reference (3), whose SUPC bandgap is not modelled, or an
    /// undeclared board reference.
    pub fn output_millivolts(&self, code: u16) -> Option<f64> {
        let vref = match (self.ctrlb >> 1) & 3 {
            0 | 2 => self.vrefa_mv?,
            1 => self.vddana_mv?,
            _ => return None,
        };
        Some(f64::from(code & 0xfff) * vref / 4095.0)
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd51 DAC slice: {what}"
        )))
    }
    fn enabled(&self) -> bool {
        self.ctrla & 2 != 0
    }
    fn ready0(&self) -> bool {
        self.enabled() && self.dacctrl[0] & 2 != 0 && self.clocked
    }
    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(self.elapsed, |c| c.now())
    }
    /// Recorded conversions `(cycle, code)` of channel 0, oldest first.
    pub fn conversions(&self) -> &[(u64, u16)] {
        &self.conversions
    }
    pub fn total(&self) -> u64 {
        self.total
    }
    fn read_reg(&self, offset: u64) -> SimResult<u16> {
        Ok(match offset {
            0x00 => u16::from(self.ctrla),
            0x01 => u16::from(self.ctrlb),
            0x02 => 0,
            0x04..=0x06 => 0,
            0x07 => u16::from(self.ready0()), // STATUS.READY0; EOC not modelled
            0x08..=0x0b => 0,                 // SYNCBUSY: writes complete at once
            0x0c => self.dacctrl[0],
            0x0e => self.dacctrl[1],
            0x18 => 0,
            0x1c => self.data0,
            _ => return Self::refuse("register"),
        })
    }
    fn write_reg(&mut self, offset: u64, value: u16) -> SimResult<()> {
        match offset {
            0x00 if value & 1 != 0 => {
                let (clock, elapsed) = (self.clock.take(), self.elapsed);
                let conversions = std::mem::take(&mut self.conversions);
                let total = self.total;
                // SWRST resets registers; the board's references are wiring.
                let (vrefa_mv, vddana_mv) = (self.vrefa_mv, self.vddana_mv);
                *self = Self {
                    clock,
                    elapsed,
                    conversions,
                    total,
                    vrefa_mv,
                    vddana_mv,
                    ..Self::default()
                };
            }
            0x00 if value & !0x02 == 0 => self.ctrla = value as u8,
            0x01 if self.enabled() => return Self::refuse("CTRLB is enable-protected"),
            // REFSEL (bits 2:1); DIFF refused.
            0x01 if value & !0x06 == 0 => self.ctrlb = value as u8,
            0x02 | 0x04 | 0x05 | 0x06 | 0x18 if value == 0 => {}
            0x0c | 0x0e if self.enabled() => return Self::refuse("DACCTRL is enable-protected"),
            // LEFTADJ, ENABLE, CCTRL, FEXT, RUNSTDBY, DITHER, REFRESH, OSR.
            0x0c => self.dacctrl[0] = value,
            0x0e if value & 2 == 0 => self.dacctrl[1] = value,
            0x10 => {
                self.data0 = value;
                if self.ready0() {
                    self.total += 1;
                    if self.conversions.len() < KEPT {
                        let at = self.now();
                        self.conversions.push((at, value & 0x0fff));
                    }
                }
            }
            _ => return Self::refuse("store outside channel 0 conversions"),
        }
        Ok(())
    }
}

impl Peripheral for Samd51Dac {
    fn read(&self, offset: u64) -> SimResult<u8> {
        if offset < 0x08 || offset == 0x18 {
            return Ok(self.read_reg(offset)? as u8); // 8-bit registers
        }
        let base = offset & !1;
        Ok((self.read_reg(base)? >> (8 * (offset - base))) as u8)
    }
    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        self.read_reg(offset)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            0x00..=0x07 | 0x18 => self.write_reg(offset, u16::from(value)),
            _ => Self::refuse("byte store to a 16-bit register"),
        }
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write_reg(offset, value)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            0x0c | 0x10 if value >> 16 == 0 => self.write_reg(offset, value as u16),
            _ => Self::refuse("word store"),
        }
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.clock = Some(clock);
    }
    fn needs_bus_tick(&self) -> bool {
        self.enabled()
    }
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.clocked = bus
            .read_u32(GCLK_PCHCTRL_DAC)
            .is_ok_and(|c| c & (1 << 6) != 0);
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn legacy_tick_active(&self) -> bool {
        false
    }
    fn tick_elapsed(&mut self, cycles: u64) -> crate::PeripheralTickResult {
        self.elapsed += cycles;
        crate::PeripheralTickResult::default()
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "samd51_dac", "ctrla": self.ctrla, "ctrlb": self.ctrlb,
            "dacctrl": self.dacctrl, "ready0": self.ready0(), "conversions": self.total})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_level_follows_the_selected_reference() {
        let mut dac = Samd51Dac::new().with_references(Some(3300.0), Some(3300.0));
        // REFSEL 0 (VREFA unbuffered, the CODAL default): full scale is VREFA.
        assert_eq!(dac.output_millivolts(4095), Some(3300.0));
        assert!((dac.output_millivolts(2048).unwrap() - 1650.4).abs() < 0.1);
        dac.write(0, 1).unwrap(); // SWRST keeps the board's references
        assert_eq!(dac.output_millivolts(4095), Some(3300.0));
        dac.ctrlb = 3 << 1; // INTREF: the SUPC bandgap is not modelled
        assert_eq!(dac.output_millivolts(4095), None);
        assert_eq!(
            Samd51Dac::new().output_millivolts(1),
            None,
            "no declared reference"
        );
    }

    #[test]
    fn conversions_need_enable_channel_and_clock() {
        let mut d = Samd51Dac::new();
        d.write_u16(0x0c, 0x0002).unwrap(); // DACCTRL0.ENABLE
        d.write_u16(0x10, 100).unwrap();
        assert_eq!(d.total(), 0, "DAC disabled");
        d.write(0, 2).unwrap();
        d.write_u16(0x10, 100).unwrap();
        assert_eq!(d.total(), 0, "no generic clock");
        assert_eq!(d.read(0x07).unwrap(), 0);
        d.clocked = true;
        assert_eq!(d.read(0x07).unwrap() & 1, 1, "READY0");
        d.write_u16(0x10, 0x0abc).unwrap();
        assert_eq!(d.conversions(), &[(0, 0x0abc)]);
        assert!(d.write_u16(0x0c, 0).is_err(), "DACCTRL while enabled");
        assert!(
            Samd51Dac::new().write(0x01, 1).is_err(),
            "differential mode"
        );
    }
}
