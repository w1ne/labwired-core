// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ATmega328P ADC input side: the voltage on each of ADC0..ADC7.
//!
//! The AVR interpreter owns ADMUX/ADCSRA/ADCL/ADCH in its data-space IO map,
//! so the converter itself lives on the CPU. What the CPU cannot own is the
//! outside world: a potentiometer or NTC on A0 drives its level through the
//! generic [`Peripheral::set_adc_channel_input`] seam, which reaches bus
//! peripherals only. This model is that bus-side half. It parks outside the
//! 16-bit data space (like `avr_gpio`) and answers the CPU's conversion with
//! the millivolts on the selected channel, two bytes little-endian per channel.

use crate::{Peripheral, SimResult};

/// ADC0..ADC7. ADC6/ADC7 exist only on the TQFP/QFN part (Nano), not the
/// DIP-28 on the Uno; the converter still has the mux positions.
pub const AVR_ADC_CHANNELS: usize = 8;

/// An input nothing drives. Mid-rail at 5 V AVcc converts to 512, which is
/// what every conversion returned before inputs were modelled, so a sketch
/// that reads a floating pin sees the same value it always did.
pub const AVR_ADC_UNDRIVEN_MV: u16 = 2500;

#[derive(Debug)]
pub struct AvrAdcInputs {
    millivolts: [u16; AVR_ADC_CHANNELS],
}

impl Default for AvrAdcInputs {
    fn default() -> Self {
        Self::new()
    }
}

impl AvrAdcInputs {
    pub fn new() -> Self {
        Self {
            millivolts: [AVR_ADC_UNDRIVEN_MV; AVR_ADC_CHANNELS],
        }
    }
}

impl Peripheral for AvrAdcInputs {
    /// Held levels moved only by `set_adc_channel_input`; no tick behaviour.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let channel = (offset / 2) as usize;
        let Some(mv) = self.millivolts.get(channel) else {
            return Ok(0);
        };
        Ok(if offset % 2 == 0 {
            (*mv & 0xFF) as u8
        } else {
            (*mv >> 8) as u8
        })
    }

    /// Firmware has no register here; the window is read-only to the CPU.
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        match self.millivolts.get_mut(channel as usize) {
            Some(slot) => {
                *slot = millivolts;
                true
            }
            None => false,
        }
    }

    fn adc_channel_count(&self) -> Option<u8> {
        Some(AVR_ADC_CHANNELS as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undriven_channels_sit_mid_rail() {
        let adc = AvrAdcInputs::new();
        assert_eq!(adc.read(0).unwrap(), (AVR_ADC_UNDRIVEN_MV & 0xFF) as u8);
        assert_eq!(adc.read(1).unwrap(), (AVR_ADC_UNDRIVEN_MV >> 8) as u8);
    }

    #[test]
    fn a_driven_channel_reads_back_little_endian_and_others_do_not_move() {
        let mut adc = AvrAdcInputs::new();
        assert!(adc.set_adc_channel_input(3, 4321));
        assert_eq!(adc.read(6).unwrap(), (4321u16 & 0xFF) as u8);
        assert_eq!(adc.read(7).unwrap(), (4321u16 >> 8) as u8);
        assert_eq!(adc.read(0).unwrap(), (AVR_ADC_UNDRIVEN_MV & 0xFF) as u8);
    }

    #[test]
    fn a_channel_the_mux_does_not_have_is_refused() {
        let mut adc = AvrAdcInputs::new();
        assert!(!adc.set_adc_channel_input(8, 1000));
        assert_eq!(adc.adc_channel_count(), Some(8));
    }
}
