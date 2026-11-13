// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas DA1459x (DA14592) general purpose ADC at `0x5004_0900`.
//!
//! Two sources:
//! - Renode `Peripherals/Analog/RenesasDA14_GPADC.cs` (renode-infrastructure
//!   @cdf0cd3d, MIT): the register map (`GP_ADC_CTRL_REG` @0x00, `CTRL2`
//!   @0x04, `CTRL3` @0x08, `SEL` @0x0C, `OFFP` @0x10, `OFFN` @0x14,
//!   `CLEAR_INT` @0x1C, `RESULT` @0x20), every bit position and reset value,
//!   the attenuator full scale `0.9 V * (GP_ADC_ATTN + 1)`, `GP_ADC_INT` set
//!   at the end of a conversion and `GP_ADC_MUTE` reading mid-scale
//!   (`0x200 << 6`).
//! - The Renesas SDK10 DA1459x `adc_sample` ELF Renode's own platform test
//!   runs (`renesas-da1459x-adc_sample.elf`, sha1 54c1b646). Its DWARF gives
//!   `GPADC_IRQn` = 25, the `GPADC_Type` member order (same offsets as
//!   Renode), and the mux: `HW_GPADC_INP_P1_0` = 0, `P1_1` = 1, `P1_2` = 2,
//!   `P0_10` = 3, `DIE_TEMP` = 4, `VDCDC` = 5, `VBAT` = 6, `VDDD` = 7,
//!   `VSSA` = 8, `VDDIO` = 9, `P1_5` = 10, `P1_6` = 11, `P1_9` = 12,
//!   `P1_11` = 13. Its `hw_gpadc_adc_measure` sets `GP_ADC_START` and spins
//!   until it reads 0; `hw_gpadc_get_value` reads `RESULT & 0xFFFF` and
//!   shifts it right by `6 - GP_ADC_CONV_NRS`, so `RESULT` is a 16-bit
//!   MSB-aligned word.
//!
//! Modelled:
//! - Channels are the `GP_ADC_SEL_P` mux values. The eight pads (0..3,
//!   10..13) take routed inputs. `VSSA` (8) converts ground. The other
//!   internal sources (die temperature, supplies) are not modelled and read 0.
//! - Single-ended codes are `V / FS * 1023` truncated and clamped, with
//!   `FS = 900 mV * (GP_ADC_ATTN + 1)`, stored `<< 6` in `RESULT`. A
//!   noiseless input gives `2^n` identical samples for `GP_ADC_CONV_NRS` = n,
//!   whose MSB-aligned sum is the same word, so oversampling adds no bits.
//! - `GP_ADC_START` with `GP_ADC_EN` set converts at once: the input is held
//!   at the write, `START` reads 0 afterwards and `GP_ADC_INT` is set.
//!   Without `GP_ADC_EN` the write is ignored.
//! - `GP_ADC_MUTE` converts mid-scale, `0x8000`.
//! - `GP_ADC_INT` is read-only; any write to `CLEAR_INT` clears it. The IRQ
//!   line is `GP_ADC_INT && GP_ADC_MINT`.
//!
//! Not modelled: differential mode (`GP_ADC_SE` = 0 sets `INT` and leaves
//! `RESULT` at 0, since no source gives the differential transfer), the
//! offset/chopper/sign trims (`OFFP`/`OFFN`, `CHOP`, `SIGN` are stored
//! only), continuous mode and DMA (`CONT`, `DMA_EN` stored, each `START`
//! converts once) and the conversion time.

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// `GP_ADC_SEL_P` values that select a pad, in mux order.
const PAD_CHANNELS: [u8; 8] = [0, 1, 2, 3, 10, 11, 12, 13];
const CHANNELS: usize = 14;
const FULL_SCALE: u32 = (1 << 10) - 1;
/// Attenuator step: `HW_GPADC_INPUT_VOLTAGE_UP_TO_0V9` .. `UP_TO_3V6`.
const ATTN_STEP_MV: u32 = 900;

// ── Register map (Renode RenesasDA14_GPADC.cs, SDK GPADC_Type) ──────────────
const CTRL: u64 = 0x00;
const CTRL2: u64 = 0x04;
const CTRL3: u64 = 0x08;
const SEL: u64 = 0x0C;
const OFFP: u64 = 0x10;
const OFFN: u64 = 0x14;
const CLEAR_INT: u64 = 0x1C;
const RESULT: u64 = 0x20;

const CTRL_EN: u32 = 1 << 0;
const CTRL_START: u32 = 1 << 1;
const CTRL_INT: u32 = 1 << 4;
const CTRL_MINT: u32 = 1 << 5;
const CTRL_SE: u32 = 1 << 6;
const CTRL_MUTE: u32 = 1 << 7;
/// EN, CONT, DMA_EN, MINT, SE, MUTE, SIGN, CHOP, LDO_HOLD, DIE_TEMP_EN.
/// `START` self-clears and `INT` is read-only, so neither is stored.
const CTRL_STORED: u32 = 0x17ED;

/// ATTN [1:0], I20U, CONV_NRS [8:6], SMPL_TIME [12:9], STORE_DEL [15:13].
const CTRL2_MASK: u32 = 0xFFC7;
const CTRL2_RESET: u32 = 0x0200;
const CTRL3_MASK: u32 = 0xFFFF;
const CTRL3_RESET: u32 = 0x0040;
const SEL_MASK: u32 = 0xFF;
const OFF_MASK: u32 = 0x3FF;
const OFFP_RESET: u32 = 0x0200;

/// DA1459x GPADC: one `GP_ADC_START` converts the `SEL_P` channel.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RenesasDa14Gpadc {
    ctrl: u32,
    int: bool,
    ctrl2: u32,
    ctrl3: u32,
    sel: u32,
    offp: u32,
    offn: u32,
    result: u32,
    /// Routed levels in millivolts by mux value. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
}

impl Default for RenesasDa14Gpadc {
    fn default() -> Self {
        Self::new()
    }
}

impl RenesasDa14Gpadc {
    pub fn new() -> Self {
        Self {
            ctrl: 0,
            int: false,
            ctrl2: CTRL2_RESET,
            ctrl3: CTRL3_RESET,
            sel: 0,
            offp: OFFP_RESET,
            offn: 0,
            result: 0,
            inputs_mv: [u16::MAX; CHANNELS],
        }
    }

    fn full_scale_mv(&self) -> u32 {
        ATTN_STEP_MV * ((self.ctrl2 & 3) + 1)
    }

    fn channel_mv(&self, channel: u8) -> u32 {
        match self.inputs_mv.get(channel as usize) {
            Some(&mv) if mv != u16::MAX => u32::from(mv),
            _ => 0,
        }
    }

    fn convert(&mut self) {
        self.result = if self.ctrl & CTRL_MUTE != 0 {
            0x200 << 6
        } else if self.ctrl & CTRL_SE == 0 {
            0
        } else {
            let fs = self.full_scale_mv();
            let mv = self.channel_mv(((self.sel >> 4) & 0xF) as u8);
            (mv.min(fs) * FULL_SCALE / fs) << 6
        };
        self.int = true;
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset & !3 {
            CTRL => self.ctrl | if self.int { CTRL_INT } else { 0 },
            CTRL2 => self.ctrl2,
            CTRL3 => self.ctrl3,
            SEL => self.sel,
            OFFP => self.offp,
            OFFN => self.offn,
            RESULT => self.result,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset & !3 {
            CTRL => {
                self.ctrl = value & CTRL_STORED;
                if value & CTRL_START != 0 && self.ctrl & CTRL_EN != 0 {
                    self.convert();
                }
            }
            CTRL2 => self.ctrl2 = value & CTRL2_MASK,
            CTRL3 => self.ctrl3 = value & CTRL3_MASK,
            SEL => self.sel = value & SEL_MASK,
            OFFP => self.offp = value & OFF_MASK,
            OFFN => self.offn = value & OFF_MASK,
            CLEAR_INT => self.int = false,
            _ => {}
        }
    }

    fn irq_asserted(&self) -> bool {
        self.int && self.ctrl & CTRL_MINT != 0
    }
}

impl Peripheral for RenesasDa14Gpadc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if !PAD_CHANNELS.contains(&channel) {
            return false;
        }
        self.inputs_mv[channel as usize] = millivolts;
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        if !PAD_CHANNELS.contains(&channel) {
            return false;
        }
        self.inputs_mv[channel as usize] = u16::MAX;
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_asserted())
    }

    fn on_system_reset(&mut self) {
        let inputs = self.inputs_mv;
        *self = Self::new();
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        if aligned == CLEAR_INT {
            self.write_word(aligned, u32::from(value) << shift);
            return Ok(());
        }
        // START is not stored, so a byte write to another CTRL byte never
        // restarts a conversion.
        let current = self.read_word(aligned) & !(CTRL_START | CTRL_INT);
        let next = (current & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(aligned, next);
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

    /// `HW_GPADC_INP_VSSA`: analog ground.
    const VSSA: u8 = 8;

    fn adc_with(inputs: &[(u8, u16)]) -> RenesasDa14Gpadc {
        let mut adc = RenesasDa14Gpadc::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv), "channel {ch}");
        }
        adc
    }

    /// SDK hw_gpadc_configure (single-ended, ATTN, CONV_NRS) then enable.
    fn configure(adc: &mut RenesasDa14Gpadc, attn: u32, conv_nrs: u32) {
        adc.write_u32(CTRL, CTRL_SE).unwrap();
        let ctrl2 = adc.read_u32(CTRL2).unwrap() & !(3 | (7 << 6));
        adc.write_u32(CTRL2, ctrl2 | attn | (conv_nrs << 6))
            .unwrap();
        let ctrl = adc.read_u32(CTRL).unwrap();
        adc.write_u32(CTRL, ctrl | CTRL_EN).unwrap();
    }

    /// SDK hw_gpadc_adc_measure + hw_gpadc_get_value (no trim correction).
    fn measure(adc: &mut RenesasDa14Gpadc, sel_p: u32) -> u32 {
        adc.write_u32(SEL, sel_p << 4).unwrap();
        adc.write_u32(CLEAR_INT, 1).unwrap();
        let ctrl = adc.read_u32(CTRL).unwrap();
        adc.write_u32(CTRL, ctrl | CTRL_START).unwrap();
        assert_eq!(adc.read_u32(CTRL).unwrap() & CTRL_START, 0, "START clears");
        assert_ne!(adc.read_u32(CTRL).unwrap() & CTRL_INT, 0, "INT set");
        let raw = adc.read_u32(RESULT).unwrap() & 0xFFFF;
        let nrs = (adc.read_u32(CTRL2).unwrap() >> 6) & 7;
        let shift = if nrs == 7 { 0 } else { 6 - nrs };
        raw >> shift
    }

    #[test]
    fn sdk_measure_converts_each_pad_against_the_attenuated_full_scale() {
        let mut adc = adc_with(&[
            (0, 450),
            (1, 900),
            (2, 1000),
            (3, 2000),
            (10, 100),
            (11, 3000),
            (12, 3600),
            (13, 3700),
        ]);
        // ATTN = 0: 0.9 V full scale, one sample.
        configure(&mut adc, 0, 0);
        assert_eq!(measure(&mut adc, 0), 450 * 1023 / 900, "P1_0 half scale");
        assert_eq!(measure(&mut adc, 1), 1023, "P1_1 at full scale");
        assert_eq!(measure(&mut adc, 2), 1023, "P1_2 clamps above 0.9 V");
        assert_eq!(measure(&mut adc, 10), 100 * 1023 / 900, "P1_5");
        // ATTN = 3: 3.6 V full scale resolves every pad.
        configure(&mut adc, 3, 0);
        assert_eq!(measure(&mut adc, 2), 1000 * 1023 / 3600);
        assert_eq!(measure(&mut adc, 3), 2000 * 1023 / 3600, "P0_10");
        assert_eq!(measure(&mut adc, 11), 3000 * 1023 / 3600, "P1_6");
        assert_eq!(measure(&mut adc, 12), 1023, "P1_9 at 3.6 V");
        assert_eq!(measure(&mut adc, 13), 1023, "P1_11 clamps");
        // ATTN = 1 and 2: 1.8 V and 2.7 V.
        configure(&mut adc, 1, 0);
        assert_eq!(measure(&mut adc, 2), 1000 * 1023 / 1800);
        configure(&mut adc, 2, 0);
        assert_eq!(measure(&mut adc, 3), 2000 * 1023 / 2700);
    }

    #[test]
    fn result_is_msb_aligned_and_oversampling_scales_the_sdk_value() {
        let mut adc = adc_with(&[(1, 1234)]);
        let code = 1234 * 1023 / 3600;
        for nrs in 0..=6 {
            configure(&mut adc, 3, nrs);
            // hw_gpadc_get_value shifts by 6 - n: 10 + n bits.
            assert_eq!(measure(&mut adc, 1), code << nrs, "CONV_NRS {nrs}");
            assert_eq!(adc.read_u32(RESULT).unwrap(), code << 6);
        }
    }

    #[test]
    fn internal_channels_mute_differential_and_enable() {
        let mut adc = adc_with(&[(0, 600)]);
        // Only the eight pads route.
        for ch in [4, 5, 6, 7, 8, 9, 14] {
            assert!(!adc.set_adc_channel_input(ch, 500), "channel {ch}");
        }
        assert_eq!(adc.adc_channel_count(), Some(14));
        // START without GP_ADC_EN does nothing.
        adc.write_u32(SEL, 0).unwrap();
        adc.write_u32(CTRL, CTRL_SE | CTRL_START).unwrap();
        assert_eq!(adc.read_u32(CTRL).unwrap(), CTRL_SE);
        assert_eq!(adc.read_u32(RESULT).unwrap(), 0);
        configure(&mut adc, 0, 0);
        assert_eq!(measure(&mut adc, 0), 600 * 1023 / 900);
        // VSSA is ground; the unmodelled internal sources read 0, as does an
        // undriven pad.
        assert_eq!(measure(&mut adc, u32::from(VSSA)), 0);
        assert_eq!(measure(&mut adc, 6), 0, "VBAT not modelled");
        assert_eq!(measure(&mut adc, 3), 0, "undriven P0_10");
        // MUTE: input tied to half the reference.
        let ctrl = adc.read_u32(CTRL).unwrap();
        adc.write_u32(CTRL, ctrl | CTRL_MUTE).unwrap();
        measure(&mut adc, 0);
        assert_eq!(adc.read_u32(RESULT).unwrap(), 0x8000);
        // Differential (SE = 0) is not modelled: INT, RESULT 0.
        adc.write_u32(CTRL, CTRL_EN).unwrap();
        measure(&mut adc, 0);
        assert_eq!(adc.read_u32(RESULT).unwrap(), 0);
        // The sample is held at START.
        configure(&mut adc, 0, 0);
        measure(&mut adc, 0);
        adc.set_adc_channel_input(0, 100);
        assert_eq!(adc.read_u32(RESULT).unwrap(), (600 * 1023 / 900) << 6);
    }

    #[test]
    fn int_clear_mask_irq_and_reset() {
        let mut adc = adc_with(&[(12, 300)]);
        configure(&mut adc, 0, 0);
        measure(&mut adc, 12);
        // INT without MINT does not raise the line.
        assert_eq!(adc.irq_line_level(), Some(false));
        let ctrl = adc.read_u32(CTRL).unwrap();
        adc.write_u32(CTRL, ctrl | CTRL_MINT).unwrap();
        assert_eq!(adc.irq_line_level(), Some(true));
        assert!(adc.tick().irq);
        // INT is read-only: writing CTRL without it keeps it set.
        let ctrl = adc.read_u32(CTRL).unwrap() & !CTRL_INT;
        adc.write_u32(CTRL, ctrl).unwrap();
        assert_ne!(adc.read_u32(CTRL).unwrap() & CTRL_INT, 0);
        // A byte write to CTRL's high byte neither clears INT nor restarts.
        adc.set_adc_channel_input(12, 800);
        adc.write(CTRL + 1, 0).unwrap();
        assert_ne!(adc.read_u32(CTRL).unwrap() & CTRL_INT, 0);
        assert_eq!(adc.read_u32(RESULT).unwrap(), (300 * 1023 / 900) << 6);
        // Any write to CLEAR_INT clears it, and CLEAR_INT reads 0.
        adc.write(CLEAR_INT, 0).unwrap();
        assert_eq!(adc.read_u32(CTRL).unwrap() & CTRL_INT, 0);
        assert_eq!(adc.irq_line_level(), Some(false));
        assert_eq!(adc.read_u32(CLEAR_INT).unwrap(), 0);
        // Register masks and reset values (Renode).
        adc.write_u32(CTRL2, 0xFFFF_FFFF).unwrap();
        adc.write_u32(CTRL3, 0xFFFF_FFFF).unwrap();
        adc.write_u32(SEL, 0xFFFF_FFFF).unwrap();
        adc.write_u32(OFFP, 0xFFFF_FFFF).unwrap();
        adc.write_u32(OFFN, 0xFFFF_FFFF).unwrap();
        assert_eq!(adc.read_u32(CTRL2).unwrap(), CTRL2_MASK);
        assert_eq!(adc.read_u32(CTRL3).unwrap(), 0xFFFF);
        assert_eq!(adc.read_u32(SEL).unwrap(), 0xFF);
        assert_eq!(adc.read_u32(OFFP).unwrap(), 0x3FF);
        assert_eq!(adc.read_u32(OFFN).unwrap(), 0x3FF);
        adc.on_system_reset();
        assert_eq!(adc.read_u32(CTRL).unwrap(), 0);
        assert_eq!(adc.read_u32(CTRL2).unwrap(), 0x200);
        assert_eq!(adc.read_u32(CTRL3).unwrap(), 0x40);
        assert_eq!(adc.read_u32(OFFP).unwrap(), 0x200);
        assert_eq!(adc.read_u32(OFFN).unwrap(), 0);
        assert_eq!(adc.read_u32(RESULT).unwrap(), 0);
        configure(&mut adc, 0, 0);
        assert_eq!(measure(&mut adc, 12), 800 * 1023 / 900, "inputs survive");
    }
}
