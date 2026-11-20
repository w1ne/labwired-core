// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Analog pad levels for the dsPIC33F/PIC24 ADC1 inputs (AN0-AN15).
//!
//! ADC1 itself lives in the PIC24 core's data-space SFR map (`cpu::pic24`),
//! so the generic analog seam — [`Peripheral::set_adc_channel_input`], which
//! `adc.<peripheral>.<channel>_volts` and `board.analog.<pad>_volts` routes
//! drive — has no bus-side converter to land on. This model is that landing
//! point: it holds the pin level of each ANx pad and the converter reference,
//! and the core pulls them through the I/O mirror window
//! (`cpu::pic24::PIC24_ADC_PAD_BASE`) when they change.
//!
//! The window is not silicon. Its 16-bit layout:
//!
//! | offset | content |
//! |--------|---------|
//! | [`OFF_GENERATION`] | bumps on every change below |
//! | [`OFF_VREF_MV`] | positive full-scale reference, mV (AVDD by default) |
//! | [`OFF_HELD`] | bit `n` set: AN`n` carries a routed level |
//! | [`OFF_CHANNEL_MV`] + 2·`n` | AN`n` level, mV |
//!
//! The PIC16F87XA A/D module is CPU-internal as well (`cpu::pic16`), and its
//! core pulls AN0-AN7 from the same window at `cpu::pic16::PIC16_ADC_PAD_BASE`;
//! there [`OFF_VREF_MV`] is VDD. The PIC18F4520 A/D module (`cpu::pic18`)
//! pulls AN0-AN12 the same way at `cpu::pic18::PIC18_ADC_PAD_BASE`.
//!
//! A released channel (never routed, or cleared) leaves the core's harness
//! code (`Pic24::set_adc_analog`) in charge of that input.

use crate::{Peripheral, SimResult};

/// AN0-AN15: the CH0SA/AD1PCFG input space of the 10-bit PIC24 ADC1. The
/// package decides which of them exist: AN0-AN8 on the dsPIC33FJ32MC20x
/// (DS70283H Table 1-1), AN0-AN5 and AN10-AN12 on the PIC24F16KA102 family
/// (DS39927C Table 1-2).
pub const CHANNELS: usize = 16;
/// Bumps on every level or reference change.
pub const OFF_GENERATION: u64 = 0x00;
/// Positive full-scale reference in millivolts.
pub const OFF_VREF_MV: u64 = 0x02;
/// Bitmask of channels that carry a routed level.
pub const OFF_HELD: u64 = 0x04;
/// AN0 level in millivolts; AN`n` is at `OFF_CHANNEL_MV + 2 * n`.
pub const OFF_CHANNEL_MV: u64 = 0x06;
/// Bytes this model answers.
pub const WINDOW_BYTES: u64 = OFF_CHANNEL_MV + 2 * CHANNELS as u64;

/// Routed analog levels for one PIC24 ADC1.
#[derive(Debug, Clone)]
pub struct Pic24AdcPads {
    /// AN`n` bonded out on this package (bit `n`).
    present: u16,
    generation: u16,
    vref_mv: u16,
    held: u16,
    channel_mv: [u16; CHANNELS],
}

impl Pic24AdcPads {
    /// `present` masks the package's ANx inputs; `vref_mv` is the AVDD
    /// reference the converter uses with VCFG = 000.
    pub fn new(present: u16, vref_mv: u16) -> Self {
        Self {
            present,
            generation: 0,
            vref_mv: vref_mv.max(1),
            held: 0,
            channel_mv: [0; CHANNELS],
        }
    }

    fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn reg(&self, offset: u64) -> u16 {
        match offset & !1 {
            OFF_GENERATION => self.generation,
            OFF_VREF_MV => self.vref_mv,
            OFF_HELD => self.held,
            o if (OFF_CHANNEL_MV..WINDOW_BYTES).contains(&o) => {
                self.channel_mv[((o - OFF_CHANNEL_MV) / 2) as usize]
            }
            _ => 0,
        }
    }

    fn has_channel(&self, channel: u8) -> bool {
        usize::from(channel) < CHANNELS && self.present & (1 << channel) != 0
    }
}

impl Peripheral for Pic24AdcPads {
    /// Levels move only through the analog seam; no tick behaviour.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = self.reg(offset);
        Ok(if offset & 1 == 0 {
            (reg & 0xFF) as u8
        } else {
            (reg >> 8) as u8
        })
    }

    /// Read-only from the bus: the window is a harness seam, not an SFR.
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    /// AN0 up to the highest bonded-out input: a route to AN6 on the 28-pin
    /// part is refused when the session is built, not dropped every step.
    fn adc_channel_count(&self) -> Option<u8> {
        Some((16 - self.present.leading_zeros()) as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if !self.has_channel(channel) {
            return false;
        }
        let idx = usize::from(channel);
        if self.held & (1 << channel) == 0 || self.channel_mv[idx] != millivolts {
            self.held |= 1 << channel;
            self.channel_mv[idx] = millivolts;
            self.bump();
        }
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        if !self.has_channel(channel) {
            return false;
        }
        if self.held & (1 << channel) != 0 {
            self.held &= !(1 << channel);
            self.channel_mv[usize::from(channel)] = 0;
            self.bump();
        }
        true
    }

    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }

    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || volts <= 0.0 || volts * 1000.0 > f64::from(u16::MAX) {
            return false;
        }
        let mv = ((volts * 1000.0).round() as u16).max(1);
        if mv != self.vref_mv {
            self.vref_mv = mv;
            self.bump();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u16(pads: &Pic24AdcPads, offset: u64) -> u16 {
        u16::from(pads.read(offset).unwrap()) | (u16::from(pads.read(offset + 1).unwrap()) << 8)
    }

    #[test]
    fn routed_levels_publish_through_the_window_and_bump_the_generation() {
        // 28-pin package: AN0-AN5 only.
        let mut pads = Pic24AdcPads::new(0x003F, 3300);
        assert_eq!(read_u16(&pads, OFF_VREF_MV), 3300);
        assert_eq!(read_u16(&pads, OFF_HELD), 0);
        assert!(pads.set_adc_channel_input(4, 1234));
        assert_eq!(read_u16(&pads, OFF_GENERATION), 1);
        assert_eq!(read_u16(&pads, OFF_HELD), 1 << 4);
        assert_eq!(read_u16(&pads, OFF_CHANNEL_MV + 8), 1234);
        // The same level again is not a change.
        assert!(pads.set_adc_channel_input(4, 1234));
        assert_eq!(read_u16(&pads, OFF_GENERATION), 1);
        assert!(pads.set_adc_reference_voltage_v(2.5));
        assert_eq!(read_u16(&pads, OFF_VREF_MV), 2500);
        assert_eq!(read_u16(&pads, OFF_GENERATION), 2);
        assert!(pads.clear_adc_channel_input(4));
        assert_eq!(read_u16(&pads, OFF_HELD), 0);
        assert_eq!(read_u16(&pads, OFF_GENERATION), 3);
    }

    #[test]
    fn refuses_channels_the_package_does_not_bond_out() {
        let mut pads = Pic24AdcPads::new(0x003F, 3300);
        assert_eq!(pads.adc_channel_count(), Some(6), "AN0-AN5");
        assert_eq!(Pic24AdcPads::new(0x01FF, 3300).adc_channel_count(), Some(9));
        assert!(!pads.set_adc_channel_input(6, 100), "AN6 is RC0: no PORTC");
        assert!(!pads.set_adc_channel_input(9, 100), "no AN9 on this family");
        assert!(!pads.set_adc_channel_input(16, 100), "past AN15");
        assert!(!pads.set_adc_reference_voltage_v(0.0));
        assert_eq!(read_u16(&pads, OFF_GENERATION), 0);
    }

    #[test]
    fn sparse_upper_channels_route_on_the_pic24f_ka102() {
        // DS39927C Table 1-2: AN0-AN5 and AN10-AN12; AN6-AN9 do not exist.
        let mut pads = Pic24AdcPads::new(0x1C3F, 3300);
        assert_eq!(pads.adc_channel_count(), Some(13), "AN0 up to AN12");
        assert!(!pads.set_adc_channel_input(9, 100), "no AN9 on the KA102");
        assert!(pads.set_adc_channel_input(12, 2048));
        assert_eq!(read_u16(&pads, OFF_HELD), 1 << 12);
        assert_eq!(read_u16(&pads, OFF_CHANNEL_MV + 24), 2048);
        assert_eq!(WINDOW_BYTES, 38);
    }
}
