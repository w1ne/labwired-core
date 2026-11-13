// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! TI CC2538 SoC ADC (SWRU319C §15 "ADC"; register layout and bit fields
//! from Contiki-NG `arch/cpu/cc2538/dev/soc-adc.h`, conversion flow and
//! scaling from its `adc.c`, `cc2538-vdd3-sensor.c` and `adc-zoul.c`).
//!
//! Modelled: the "extra" single conversion that a write to `ADCCON3` starts,
//! with `EREF` (internal reference, AIN7, AVDD5 or the AIN6-AIN7 pair),
//! `EDIV` (decimation 64/128/256/512 = 7/9/10/12 bits) and `ECH` (AIN0-7,
//! the AIN0-1/2-3/4-5/6-7 differential pairs, GND, VDD/3). The result is
//! 2's complement with its significant bits at the top of `ADCH:ADCL`
//! (`ADCL[7:2]`, `ADCH[7:0]`), so a 12-bit single-ended full scale reads
//! `2047 << 4`. `ADCCON1.EOC` rises when the conversion completes and clears
//! when `ADCH` is read. Conversions complete at the `ADCCON3` write: the model
//! has no conversion clock.
//!
//! Not modelled: sequence conversions (`ADCCON1.ST` / `STSEL`, `ADCCON2`),
//! which are stored only, the temperature-sensor channel (reads 0), the ADC
//! interrupt and the µDMA trigger. The random-number generator (`RNDL/RNDH`,
//! `ADCCON1.RCTRL`) and the analog comparator (`CMPCTL`) in the same window
//! keep the behaviour of the stub this model replaced: they read 0 and
//! ignore writes, so firmware that never converts sees no difference.

use std::cell::Cell;

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// AIN0..AIN7 are PA0..PA7.
const CHANNELS: usize = 8;
/// Contiki-NG's VDD/3 conversion scales the internal reference as 1190 mV.
const DEFAULT_INT_REF_MV: u32 = 1190;
const DEFAULT_AVDD_MV: u32 = 3300;

// ── Register map (soc-adc.h, base 0x400D7000) ───────────────────────────────
const ADCCON1: u64 = 0x00;
const ADCCON2: u64 = 0x04;
const ADCCON3: u64 = 0x08;
const ADCL: u64 = 0x0C;
const ADCH: u64 = 0x10;

const ADCCON1_EOC: u32 = 0x80;
const ADCCON1_STSEL: u32 = 0x30;

const REF_INT: u32 = 0;
const REF_EXT_AIN7: u32 = 1;
const REF_AVDD5: u32 = 2;
#[cfg(test)]
const CH_GND: u32 = 0xC;
const CH_VDD_3: u32 = 0xF;

/// CC2538 SoC ADC: extra conversions on the routed AIN0..AIN7 levels.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Cc2538Adc {
    adccon1: u32,
    adccon2: u32,
    adccon3: u32,
    /// The last result, 2's complement, significant bits at the top.
    result: u16,
    /// Routed input levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
    int_ref_mv: u32,
    avdd_mv: u32,
    /// `ADCH` reads clear EOC while `read` takes `&self`.
    eoc: Cell<bool>,
}

impl Default for Cc2538Adc {
    fn default() -> Self {
        Self::new(DEFAULT_INT_REF_MV, DEFAULT_AVDD_MV)
    }
}

impl Cc2538Adc {
    pub fn new(int_ref_mv: u32, avdd_mv: u32) -> Self {
        Self {
            adccon1: 0,
            adccon2: 0,
            adccon3: 0,
            result: 0,
            inputs_mv: [u16::MAX; CHANNELS],
            int_ref_mv: int_ref_mv.max(1),
            avdd_mv,
            eoc: Cell::new(false),
        }
    }

    fn reset(&mut self) {
        *self = Self::new(self.int_ref_mv, self.avdd_mv);
    }

    /// An AINn level in millivolts; an undriven pad reads 0 V.
    fn ain(&self, n: usize) -> i64 {
        match self.inputs_mv[n] {
            u16::MAX => 0,
            mv => i64::from(mv),
        }
    }

    /// The `ECH` input voltage: single-ended, differential or internal.
    fn input_mv(&self, ech: u32) -> i64 {
        match ech {
            0..=7 => self.ain(ech as usize),
            8..=0xB => {
                let pair = 2 * (ech as usize - 8);
                self.ain(pair) - self.ain(pair + 1)
            }
            CH_VDD_3 => i64::from(self.avdd_mv) / 3,
            // GND, the reserved 0xD and the unmodelled temperature sensor.
            _ => 0,
        }
    }

    fn reference_mv(&self, eref: u32) -> i64 {
        match eref {
            REF_INT => i64::from(self.int_ref_mv),
            REF_EXT_AIN7 => self.ain(7),
            REF_AVDD5 => i64::from(self.avdd_mv),
            _ => self.ain(6) - self.ain(7),
        }
    }

    /// One extra conversion with the `ADCCON3` settings.
    fn convert(&mut self) {
        let eref = (self.adccon3 >> 6) & 3;
        let bits = [7u32, 9, 10, 12][((self.adccon3 >> 4) & 3) as usize];
        let max = (1i64 << (bits - 1)) - 1;
        let reference = self.reference_mv(eref).max(1);
        let code = (self.input_mv(self.adccon3 & 0xF) * max / reference).clamp(-max - 1, max);
        self.result = ((code << (16 - bits)) & 0xFFFF) as u16;
        self.eoc.set(true);
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset & !3 {
            ADCCON1 => self.adccon1 | if self.eoc.get() { ADCCON1_EOC } else { 0 },
            ADCCON2 => self.adccon2,
            ADCCON3 => self.adccon3,
            ADCL => u32::from(self.result) & 0xFC,
            ADCH => {
                self.eoc.set(false);
                u32::from(self.result >> 8)
            }
            // RNDL/RNDH, CMPCTL and the rest of the window: as the stub.
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset & !3 {
            // EOC is read-only and ST starts an unmodelled sequence; RCTRL
            // completes at once, so only STSEL is kept.
            ADCCON1 => self.adccon1 = value & ADCCON1_STSEL,
            ADCCON2 => self.adccon2 = value & 0xFF,
            ADCCON3 => {
                self.adccon3 = value & 0xFF;
                self.convert();
            }
            _ => {}
        }
    }
}

impl Peripheral for Cc2538Adc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        let Some(input) = self.inputs_mv.get_mut(channel as usize) else {
            return false;
        };
        *input = millivolts;
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        let Some(input) = self.inputs_mv.get_mut(channel as usize) else {
            return false;
        };
        *input = u16::MAX;
        true
    }

    fn on_system_reset(&mut self) {
        let inputs = self.inputs_mv;
        self.reset();
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // Every register is 8 bits wide in the low byte of its word.
        if offset & 3 == 0 {
            self.write_word(offset, u32::from(value));
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
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

    const DIV_512: u32 = 3 << 4;
    const REF_AVDD: u32 = REF_AVDD5 << 6;

    fn adc_with(inputs: &[(u8, u16)]) -> Cc2538Adc {
        let mut adc = Cc2538Adc::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv));
        }
        adc
    }

    /// `adc_get()` from Contiki-NG adc.c: write ADCCON3, poll EOC, read
    /// ADCL then ADCH.
    fn adc_get(adc: &mut Cc2538Adc, channel: u32, reference: u32, div: u32) -> i16 {
        adc.write_u32(ADCCON3, reference | div | channel).unwrap();
        assert_ne!(adc.read_u32(ADCCON1).unwrap() & ADCCON1_EOC, 0);
        let low = adc.read_u32(ADCL).unwrap() & 0xFC;
        let high = adc.read_u32(ADCH).unwrap();
        assert_eq!(
            adc.read_u32(ADCCON1).unwrap() & ADCCON1_EOC,
            0,
            "the ADCH read clears EOC"
        );
        (low | (high << 8)) as u16 as i16
    }

    #[test]
    fn extra_conversion_is_msb_aligned_at_each_decimation() {
        let mut adc = adc_with(&[(6, 1650)]);
        // 1.65 V of 3.3 V is half scale at every resolution.
        for (div, bits) in [(0u32, 7u32), (1, 9), (2, 10), (3, 12)] {
            let raw = adc_get(&mut adc, 6, REF_AVDD, div << 4);
            let max = (1i32 << (bits - 1)) - 1;
            assert_eq!(i32::from(raw), (1650 * max / 3300) << (16 - bits));
            assert_eq!(
                raw & ((1 << (16 - bits)) - 1),
                0,
                "only the top {bits} bits are significant"
            );
        }
        // Full scale and above clamp to 2047 << 4 at 12 bits.
        adc.set_adc_channel_input(6, 3300);
        assert_eq!(adc_get(&mut adc, 6, REF_AVDD, DIV_512), 2047 << 4);
        // Against the 1.19 V internal reference the same pad saturates.
        assert_eq!(adc_get(&mut adc, 6, 0, DIV_512), 2047 << 4);
        // An undriven pad and the GND channel read zero.
        assert_eq!(adc_get(&mut adc, 2, REF_AVDD, DIV_512), 0);
        assert_eq!(adc_get(&mut adc, CH_GND, REF_AVDD, DIV_512), 0);
    }

    #[test]
    fn vdd3_matches_the_contiki_conversion() {
        // cc2538-vdd3-sensor.c: mV = raw * (3 * 1190) / (2047 << 4).
        let mut adc = Cc2538Adc::new(1190, 3300);
        let raw = i32::from(adc_get(&mut adc, CH_VDD_3, 0, DIV_512));
        assert_eq!(raw, (1100 * 2047 / 1190) << 4);
        let mv = raw * (3 * 1190) / (2047 << 4);
        assert!((3290..=3300).contains(&mv), "VDD reads back as {mv} mV");
    }

    #[test]
    fn differential_pairs_go_negative_and_external_references_scale() {
        let mut adc = adc_with(&[(4, 500), (5, 1500), (6, 2000), (7, 1000)]);
        // AIN4-AIN5 = -1.0 V against AVDD5 3.3 V.
        let raw = i32::from(adc_get(&mut adc, 0xA, REF_AVDD, DIV_512));
        assert_eq!(raw, (-1000 * 2047 / 3300) << 4);
        assert!(raw < 0);
        // AIN6 against AIN7 = 1.0 V as the single-ended reference: 2 V clamps.
        assert_eq!(adc_get(&mut adc, 6, REF_EXT_AIN7 << 6, DIV_512), 2047 << 4);
        // AIN0-AIN1 = 0 against the AIN6-AIN7 = 1.0 V differential reference.
        assert_eq!(adc_get(&mut adc, 0x8, 3 << 6, DIV_512), 0);
        // AIN7 = 1.0 V against that same 1.0 V reference is full scale.
        assert_eq!(adc_get(&mut adc, 7, 3 << 6, DIV_512), 2047 << 4);
        // Raising AIN7 to 1.5 V shrinks the reference to 0.5 V.
        adc.set_adc_channel_input(7, 1500);
        assert_eq!(
            i32::from(adc_get(&mut adc, 4, 3 << 6, DIV_512)),
            (500 * 2047 / 500) << 4
        );
    }

    #[test]
    fn stub_registers_stay_silent_and_reset_keeps_inputs() {
        let mut adc = adc_with(&[(0, 1000)]);
        // RNDL/RNDH/CMPCTL read 0 and ignore writes, as the stub did.
        for offset in [0x14u64, 0x18, 0x24] {
            adc.write_u32(offset, 0xAB).unwrap();
            assert_eq!(adc.read_u32(offset).unwrap(), 0);
        }
        // adc_init(): ADCCON1 |= STSEL; RCTRL never reads back.
        adc.write_u32(ADCCON1, ADCCON1_STSEL | 0x0C).unwrap();
        assert_eq!(adc.read_u32(ADCCON1).unwrap(), ADCCON1_STSEL);
        // Byte writes to ADCCON3 also start a conversion.
        adc.write(ADCCON3, (REF_AVDD | DIV_512) as u8).unwrap();
        assert_ne!(adc.read_u32(ADCCON1).unwrap() & ADCCON1_EOC, 0);
        adc.on_system_reset();
        assert_eq!(adc.read_u32(ADCCON1).unwrap(), 0);
        assert_eq!(adc.read_u32(ADCH).unwrap(), 0);
        assert_eq!(
            i32::from(adc_get(&mut adc, 0, REF_AVDD, DIV_512)),
            (1000 * 2047 / 3300) << 4
        );
    }
}
