// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Frontgrade Gaisler GR716 on-chip ADC: eight control units at
//! `0x8040_0000 + n * 0x1000` sharing the analog MUX over pads ADC0..ADC7.
//!
//! Sources:
//! - GR716-DS-UM v1.29 (May 2019) §2 Table 25 (ADC interface 0..7 at
//!   0x80400000..0x80407FFF, 0x100 bytes each, DAC interfaces from
//!   0x80408000), §2 interrupt table (ADC0..6 on lines 28..34), §12.1
//!   (11-bit SAR, two converters behind eight control units), §12.2
//!   (single-ended 0 V to 2.5 V, differential -2 V to 2 V on channel pairs
//!   0/2/4/6, pre-amplifier x1/x2/x4 or bypass, single-ended with the
//!   amplifier needs the negative pin grounded, oversampling accumulates
//!   AO + 1 samples without dividing) and §12.3 Tables 93..102: ACFG @0x00
//!   (AC [31:16] scaler, AE 15 enable, AS 14 start, high until end of
//!   conversion, AH 8, AL 7, AI [5:2] input, AM 0 single-ended), ASAMPC
//!   @0x04 (AE [17:16], AO [15:0]), ASEQC @0x08, ASYNC @0x0C, ASTS @0x10
//!   (bit 31 conversion valid, AD [18:0] accumulated result, all ones on
//!   oversampling overflow), AINT @0x14 (IS 3, IH 2, IL 1, IE 0,
//!   write-1-to-clear), AMASK @0x18 (MS/MH/ML/ME), PACFG @0x1C (AB 2
//!   bypass, AG [1:0] gain 0/6/12/12 dB), AHT @0x20, ALT @0x24, sequence
//!   memory @0x2C..0x38. Read through the manualslib mirror of the PDF.
//! - Phoenix-RTOS phoenix-rtos-devices @566e2554 `multi/grlib-multi/adc.c`:
//!   the same offsets; `adc_configure` writes ACFG = scaler << 16 |
//!   unit << 2 | mode | AE, PACFG = AB, AMASK = IE; `adc_convert` sets AS,
//!   waits for ASTS bit 31 and reads `ASTS & 0x7FFFF`; the ISR clears
//!   AINT.IE by writing it.
//! - GR716-MINI user manual rev 1.1 (download.gaisler.com): ADC0..ADC7 are
//!   pads GPIO37..GPIO44.
//!
//! Modelled: ACFG.AS with AE converts at once and AS reads back 0; a
//! single-ended (AM = 1) conversion of pad AI = 0..7 sums AO + 1 samples
//! into ASTS.AD, saturating at 0x7FFFF, sets ASTS bit 31 and AINT.IE; with
//! the amplifier in the path (AB = 0) the sample is scaled by AG's gain.
//! Each sample is mV * 2048 / VREF, clamped to 2047 (VREF 2500 mV, the
//! single-ended range, configurable). An undriven pad samples 0. The IRQ
//! line is the OR of AINT & AMASK over the units.
//!
//! Not sourced, so kept minimal: reset values (0: the PDF reset column is
//! not legible); differential mode (AM = 0) and the temperature / core
//! voltage inputs (AI = 8/9) convert nothing, since the result coding is
//! not stated; the sequencer (ASEQC, ASYNC), the sample memory and the
//! level detectors (AHT/ALT, AH/AL, IH/IL) are stored with no behaviour;
//! the per-unit interrupt lines (one peripheral has one line); conversion
//! time; when ASTS bit 31 clears (at the next start).

use crate::{Peripheral, PeripheralTickResult, SimResult};

const UNITS: usize = 8;
const PADS: usize = 8;
const UNIT_STRIDE: u64 = 0x1000;
const SAMPLE_MAX: u32 = (1 << 11) - 1;
const AD_MAX: u32 = 0x7FFFF;

const ACFG: u64 = 0x00;
const ASAMPC: u64 = 0x04;
const ASEQC: u64 = 0x08;
const ASYNC: u64 = 0x0C;
const ASTS: u64 = 0x10;
const AINT: u64 = 0x14;
const AMASK: u64 = 0x18;
const PACFG: u64 = 0x1C;
const AHT: u64 = 0x20;
const ALT: u64 = 0x24;

const AE: u32 = 1 << 15;
const AS: u32 = 1 << 14;
const AM: u32 = 1 << 0;
const ASTS_VALID: u32 = 1 << 31;
const INT_BITS: u32 = 0xF;
const IE: u32 = 1 << 0;
const AB: u32 = 1 << 2;

#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Unit {
    acfg: u32,
    asampc: u32,
    aseqc: u32,
    async_: u32,
    asts: u32,
    aint: u32,
    amask: u32,
    pacfg: u32,
    aht: u32,
    alt: u32,
}

/// GR716 ADC: eight control units over one shared pad MUX.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Gr716Adc {
    units: [Unit; UNITS],
    vref_mv: u32,
    /// Routed pad levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; PADS],
}

impl Default for Gr716Adc {
    fn default() -> Self {
        Self::new(2500)
    }
}

impl Gr716Adc {
    /// `vref_mv`: single-ended full scale (GR716-DS-UM §12.2: 0 V to 2.5 V).
    pub fn new(vref_mv: u32) -> Self {
        Self {
            units: [Unit::default(); UNITS],
            vref_mv: vref_mv.max(1),
            inputs_mv: [u16::MAX; PADS],
        }
    }

    fn sample(&self, pad: usize, pacfg: u32) -> u32 {
        let mv = match self.inputs_mv[pad] {
            u16::MAX => 0,
            mv => u32::from(mv),
        };
        // AG b00/b01/b10/b11 = 0/6/12/12 dB; bypass (AB) has no gain.
        let gain = if pacfg & AB != 0 {
            1
        } else {
            [1, 2, 4, 4][(pacfg & 3) as usize]
        };
        (mv * gain * 2048 / self.vref_mv).min(SAMPLE_MAX)
    }

    fn convert(&mut self, n: usize) {
        let unit = self.units[n];
        let pad = ((unit.acfg >> 2) & 0xF) as usize;
        self.units[n].asts = 0;
        if unit.acfg & AM == 0 || pad >= PADS {
            return;
        }
        let samples = (unit.asampc & 0xFFFF) + 1;
        let sum = self.sample(pad, unit.pacfg) * samples;
        self.units[n].asts = ASTS_VALID | sum.min(AD_MAX);
        self.units[n].aint |= IE;
    }

    fn read_word(&self, offset: u64) -> u32 {
        let n = (offset / UNIT_STRIDE) as usize;
        let Some(u) = self.units.get(n) else {
            return 0;
        };
        match offset % UNIT_STRIDE {
            ACFG => u.acfg,
            ASAMPC => u.asampc,
            ASEQC => u.aseqc,
            ASYNC => u.async_,
            ASTS => u.asts,
            AINT => u.aint,
            AMASK => u.amask,
            PACFG => u.pacfg,
            AHT => u.aht,
            ALT => u.alt,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let n = (offset / UNIT_STRIDE) as usize;
        if n >= UNITS {
            return;
        }
        let u = &mut self.units[n];
        match offset % UNIT_STRIDE {
            ACFG => {
                // ACFG bits 13:9 and 6 are reserved.
                u.acfg = value & 0xFFFF_C1BD & !AS;
                if value & AS != 0 && value & AE != 0 {
                    self.convert(n);
                }
            }
            ASAMPC => u.asampc = value & 0x3_FFFF,
            ASEQC => u.aseqc = value & 0xB800_FFFF,
            ASYNC => u.async_ = value,
            AINT => u.aint &= !(value & INT_BITS),
            AMASK => u.amask = value & INT_BITS,
            PACFG => u.pacfg = value & 0x7,
            AHT => u.aht = value,
            ALT => u.alt = value,
            _ => {}
        }
    }

    fn irq_asserted(&self) -> bool {
        self.units.iter().any(|u| u.aint & u.amask & INT_BITS != 0)
    }
}

impl Peripheral for Gr716Adc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(PADS as u8)
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

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_asserted())
    }

    fn on_system_reset(&mut self) {
        let inputs = self.inputs_mv;
        *self = Self::new(self.vref_mv);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        // AINT is write-1-to-clear: the other bytes write 0.
        let current = if aligned % UNIT_STRIDE == AINT {
            0
        } else {
            self.read_word(aligned)
        };
        self.write_word(
            aligned,
            (current & !(0xFF << shift)) | (u32::from(value) << shift),
        );
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
        PeripheralTickResult {
            irq: self.irq_asserted(),
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

    fn adc_with(inputs: &[(u8, u16)]) -> Gr716Adc {
        let mut adc = Gr716Adc::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv), "pad {ch}");
        }
        adc
    }

    fn code(mv: u32) -> u32 {
        (mv * 2048 / 2500).min(2047)
    }

    /// Phoenix-RTOS `adc_configure` (scaler for 200 kSps at 50 MHz, the
    /// unit's own pad, single mode) then `adc_convert`.
    fn phoenix_read(adc: &mut Gr716Adc, unit: u64, pad: u32) -> u32 {
        let base = unit * UNIT_STRIDE;
        adc.write_u32(base + ACFG, (12 << 16) | (pad << 2) | AM | AE)
            .unwrap();
        adc.write_u32(base + PACFG, AB).unwrap();
        adc.write_u32(base + AMASK, IE).unwrap();
        let cfg = adc.read_u32(base + ACFG).unwrap();
        adc.write_u32(base + ACFG, cfg | AS).unwrap();
        let sts = adc.read_u32(base + ASTS).unwrap();
        assert_ne!(sts & ASTS_VALID, 0, "unit {unit}: conversion valid");
        assert!(adc.irq_asserted(), "unit {unit}: masked IE drives the line");
        adc.write_u32(base + AINT, IE).unwrap();
        assert!(!adc.irq_asserted(), "unit {unit}: AINT.IE write-1-clears");
        sts & 0x7FFFF
    }

    #[test]
    fn every_unit_converts_its_pad_against_the_2v5_single_ended_range() {
        let mut adc = adc_with(&[(0, 0), (3, 1250), (5, 2499), (7, 3300)]);
        assert_eq!(phoenix_read(&mut adc, 0, 0), 0);
        assert_eq!(phoenix_read(&mut adc, 3, 3), 1024);
        assert_eq!(phoenix_read(&mut adc, 5, 5), code(2499));
        // Above 2.5 V the 11-bit sample saturates.
        assert_eq!(phoenix_read(&mut adc, 7, 7), 2047);
        // An undriven pad samples 0.
        assert_eq!(phoenix_read(&mut adc, 1, 1), 0);
    }

    #[test]
    fn any_unit_reaches_any_pad_through_ai() {
        let mut adc = adc_with(&[(2, 800), (6, 1900)]);
        assert_eq!(phoenix_read(&mut adc, 4, 2), code(800));
        assert_eq!(phoenix_read(&mut adc, 4, 6), code(1900));
        assert_eq!(phoenix_read(&mut adc, 0, 6), code(1900));
    }

    #[test]
    fn start_needs_ae_and_as_self_clears() {
        let mut adc = adc_with(&[(1, 1000)]);
        adc.write_u32(ACFG, (1 << 2) | AM | AS).unwrap();
        assert_eq!(adc.read_u32(ASTS).unwrap(), 0, "no conversion without AE");
        assert_eq!(adc.read_u32(AINT).unwrap(), 0);
        adc.write_u32(ACFG, (1 << 2) | AM | AE | AS).unwrap();
        assert_eq!(adc.read_u32(ACFG).unwrap() & AS, 0, "AS reads 0 at EOC");
        assert_eq!(adc.read_u32(ASTS).unwrap(), ASTS_VALID | code(1000));
        assert_eq!(adc.read_u32(AINT).unwrap(), IE);
        assert!(!adc.irq_asserted(), "IE is masked by default");
    }

    #[test]
    fn oversampling_accumulates_ao_plus_one_samples_and_saturates() {
        let mut adc = adc_with(&[(4, 1000)]);
        adc.write_u32(ASAMPC, 3).unwrap();
        adc.write_u32(PACFG, AB).unwrap();
        adc.write_u32(ACFG, (4 << 2) | AM | AE | AS).unwrap();
        assert_eq!(adc.read_u32(ASTS).unwrap() & 0x7FFFF, 4 * code(1000));
        adc.write_u32(ASAMPC, 0xFFFF).unwrap();
        adc.write_u32(ACFG, (4 << 2) | AM | AE | AS).unwrap();
        assert_eq!(
            adc.read_u32(ASTS).unwrap() & 0x7FFFF,
            AD_MAX,
            "overflow fills AD with ones"
        );
    }

    #[test]
    fn preamplifier_gain_applies_unless_bypassed() {
        let mut adc = adc_with(&[(0, 500)]);
        for (pacfg, gain) in [(AB, 1), (0, 1), (1, 2), (2, 4), (3, 4), (AB | 2, 1)] {
            adc.write_u32(PACFG, pacfg).unwrap();
            adc.write_u32(ACFG, AM | AE | AS).unwrap();
            assert_eq!(
                adc.read_u32(ASTS).unwrap() & 0x7FFFF,
                code(500 * gain),
                "PACFG {pacfg:#x}"
            );
        }
    }

    #[test]
    fn differential_and_internal_inputs_convert_nothing() {
        let mut adc = adc_with(&[(0, 1000), (1, 200)]);
        adc.write_u32(ACFG, AE | AS).unwrap();
        assert_eq!(adc.read_u32(ASTS).unwrap(), 0, "AM = 0 differential");
        adc.write_u32(ACFG, (8 << 2) | AE | AS).unwrap();
        assert_eq!(adc.read_u32(ASTS).unwrap(), 0, "AI = 8 temperature");
        assert_eq!(adc.read_u32(AINT).unwrap(), 0);
    }

    #[test]
    fn units_keep_separate_registers_and_reset_keeps_inputs() {
        let mut adc = adc_with(&[(5, 2000)]);
        adc.write_u32(7 * UNIT_STRIDE + ACFG, (5 << 2) | AM | AE | AS)
            .unwrap();
        assert_eq!(adc.read_u32(ASTS).unwrap(), 0, "unit 0 untouched");
        assert_ne!(adc.read_u32(7 * UNIT_STRIDE + ASTS).unwrap(), 0);
        adc.on_system_reset();
        assert_eq!(adc.read_u32(7 * UNIT_STRIDE + ASTS).unwrap(), 0);
        adc.write_u32(ACFG, (5 << 2) | AM | AE | AS).unwrap();
        assert_eq!(adc.read_u32(ASTS).unwrap(), ASTS_VALID | code(2000));
    }

    #[test]
    fn byte_write_to_aint_clears_only_that_lane() {
        let mut adc = adc_with(&[(0, 100)]);
        adc.write_u32(ACFG, AM | AE | AS).unwrap();
        adc.write(AINT + 1, 0xFF).unwrap();
        assert_eq!(adc.read_u32(AINT).unwrap(), IE, "upper lane clears nothing");
        adc.write(AINT, IE as u8).unwrap();
        assert_eq!(adc.read_u32(AINT).unwrap(), 0);
    }
}
