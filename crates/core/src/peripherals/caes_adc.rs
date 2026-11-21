// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CAES (Frontgrade) UT32M0R500 delta-sigma ADC signal processing block at
//! `0x4000_F000`.
//!
//! Two sources:
//! - Renode `Peripherals/Analog/CAES_ADC.cs` (renode-infrastructure
//!   @9e55716d, MIT): the register map (`SPB_CFG_0` @0x00, `SPB_CFG_1`
//!   @0x04, 16 `SECHAN_CFG` @0x10, 8 `DIFFCHAN_CFG` @0x50, `TEMPCHAN_CFG`
//!   @0x70, `TIM_CTRL` @0x74, `SEQ_CTRL` @0x78, `STAB_CTRL` @0x84,
//!   `INT_STATUS` @0x8C, 18 `DATA` words @0x90), every bit position and reset
//!   value, the 1.5 V single-ended full scale, the MSB flip of single-ended
//!   codes, the differential transfer `(Vp - Vn + 1.5 V) * 4095 / 3 V`, and
//!   the config-to-data channel mapping (differential pair k lands in data
//!   word 2k, the temperature channel in word 16). The Renode `.repl` puts the
//!   block at 0x4000_F000 on NVIC line 21.
//! - Frontgrade (CAES) application notes "UT32M0R500 ADC Users Guide"
//!   (rev 1.0.1, sha1 2878cceb) and "UT32M0R500 Enable the ADC" (rev 1.0.0,
//!   sha1 e7a3b788): AIN0..AIN15 are 16 dedicated inputs, one per
//!   single-ended channel, paired for differential (§2.0); PGA gains 0.5, 1,
//!   2, 4, 8, 16 V/V divide the 1.5 V single-ended full scale (Tables 3, 4);
//!   `ADC_TRIGGER` converts every enabled channel in one sweep (§3.0);
//!   `CHNL_INTR_PEND` and `CONV_COMPL_COMB` (bit 31) set for polled
//!   firmware without the ADC interrupts enabled (errata 8.2); reading
//!   `INT_STATUS` or a channel's `DATA` clears its pending bit; the reader
//!   flips bit 11 of `DATA_OUT` back to offset binary ("Code 4: ADC Read");
//!   `ADC_IRQn` = 21.
//!
//! Modelled:
//! - `ADC_TRIGGER` with `ADC_EN` converts every enabled channel at once,
//!   inputs held at the write. Config channel i (0..15) lands in data word i;
//!   differential pair k (config 16 + k) in data word 2k, overwriting a
//!   single-ended result there, as Renode orders it.
//! - Single-ended codes: `min(V * G, 1.5 V) * 4095 / 1.5 V`, truncated, with
//!   bit 11 flipped (two's complement around mid-scale). `G` is the channel's
//!   `GAIN` when `EN_PGA` is set and 1 otherwise.
//! - Differential codes: `(min(Vp G, 1.5 V) - min(Vn G, 1.5 V) + 1.5 V) *
//!   4095 / 3 V`, offset binary with no flip (Renode).
//! - Conversions run whether or not the interrupts are enabled (errata 8.2;
//!   Renode only fills `DATA` when `ADC_INTR_EN && CONV_COMPL_IEN`, which
//!   would leave the vendor's polled quick-start reading zero). The IRQ line
//!   is `ADC_INTR_EN && CONV_COMPL_IEN && CHNL_INTR_PEND != 0`.
//! - `INT_STATUS` reads clear the byte lanes they read; a `DATA` read clears
//!   the pending bit of the config channel that wrote it.
//! - `ADC_REGDEF` resets every register, results included; routed inputs stay.
//!
//! Not modelled: the temperature sensor (config 24 converts a zero
//! differential input, mid-scale `0x7FF`, Renode's value with no sample
//! source), conversion time (`TIM_CTRL`/`SEQ_CTRL` are stored), continuous
//! sweep (`ADC_SINGLESWEEP` stored, each trigger sweeps once), the `DDF2`
//! filter choice, and the error flags (`DATA_ERROR`, `COI_OVER`,
//! `SINC4_OVER`, `DSM_OVL_FLAG`, `TRIG_UNDER` read 0).

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;

/// AIN0..AIN15.
const INPUTS: usize = 16;
/// 16 single-ended + 8 differential + temperature.
const CONFIG_CHANNELS: usize = 25;
const DIFF_OFFSET: usize = 16;
const TEMP_CONFIG: usize = 24;
/// 16 single-ended words, temperature in word 16 (and 17).
const DATA_WORDS: usize = 18;
const FULL_SCALE: u64 = 0xFFF;
/// Single-ended full scale at unity gain, microvolts.
const MAX_UV: u64 = 1_500_000;

// ── Register map (Renode CAES_ADC.cs) ──────────────────────────────────────
const SPB_CFG_0: u64 = 0x00;
const SPB_CFG_1: u64 = 0x04;
const CHAN_CFG: u64 = 0x10;
const TIM_CTRL: u64 = 0x74;
const SEQ_CTRL: u64 = 0x78;
const STAB_CTRL: u64 = 0x84;
const INT_STATUS: u64 = 0x8C;
const DATA: u64 = 0x90;

const CFG0_ADC_EN: u32 = 1 << 0;
const CFG0_EN_PGA: u32 = 1 << 1;
const CFG0_ADC_INTR_EN: u32 = 1 << 24;
const CFG0_CONV_COMPL_IEN: u32 = 1 << 31;
/// ADC_EN, EN_PGA, EN_DSM, EN_AAF, DDF2_CLK_EN, EN_REFP, EN_REFC,
/// EN_BIASGEN, EN_CLKGEN, ADC_SINGLESWEEP, ODB, ADC_INTR_EN and the five
/// interrupt enables [31:27].
const CFG0_MASK: u32 = 0xF900_E6BF;

const CFG1_TRIGGER: u32 = 1 << 0;
const CFG1_RST_CCONV: u32 = 1 << 7;
const CFG1_REGDEF: u32 = 1 << 31;

/// EN [0], DDF2 [3], GAIN [6:4].
const CHAN_MASK: u32 = 0x79;
const CHAN_RESET: u32 = 0x10;
const CHAN_EN: u32 = 1 << 0;
/// ADC_DSMOSR [7:0], ADC_OSCDIV [9:8].
const TIM_MASK: u32 = 0x3FF;
const TIM_RESET: u32 = 0x64;
const SEQ_MASK: u32 = 0x3F;
/// DSM_OVL_CNT [6:0], DSM_OVL_RST [12:8]; DSM_OVL_FLAG never sets.
const STAB_MASK: u32 = 0x1F7F;
const STAB_RESET: u32 = 0x81E;

const PENDING_MASK: u32 = (1 << CONFIG_CHANNELS) - 1;
const CONV_COMPL_COMB: u32 = 1 << 31;

/// UT32M0R500 ADC: one `ADC_TRIGGER` converts every enabled channel.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct CaesAdc {
    cfg0: u32,
    cfg1: u32,
    chan_cfg: [u32; CONFIG_CHANNELS],
    tim_ctrl: u32,
    seq_ctrl: u32,
    stab_ctrl: u32,
    /// `CHNL_INTR_PEND` [24:0] and `CONV_COMPL_COMB` [31], read to clear.
    status: Cell<u32>,
    data: [u32; DATA_WORDS],
    /// Routed AIN levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; INPUTS],
}

impl Default for CaesAdc {
    fn default() -> Self {
        Self::new()
    }
}

impl CaesAdc {
    pub fn new() -> Self {
        Self {
            cfg0: 0,
            cfg1: 0,
            chan_cfg: [CHAN_RESET; CONFIG_CHANNELS],
            tim_ctrl: TIM_RESET,
            seq_ctrl: 0,
            stab_ctrl: STAB_RESET,
            status: Cell::new(0),
            data: [0; DATA_WORDS],
            inputs_mv: [u16::MAX; INPUTS],
        }
    }

    fn reset_registers(&mut self) {
        let inputs = self.inputs_mv;
        *self = Self::new();
        self.inputs_mv = inputs;
    }

    /// The channel gain as `numerator / 2`: GAIN 0 = 0.5, 1 = 1, 2 = 2,
    /// 3 = 4, 4 = 8, 5..7 = 16 V/V.
    fn gain_halves(&self, config: usize) -> u64 {
        if self.cfg0 & CFG0_EN_PGA == 0 {
            return 2;
        }
        match (self.chan_cfg[config] >> 4) & 7 {
            0 => 1,
            g @ 1..=4 => 1 << g,
            _ => 32,
        }
    }

    /// The AIN level after the PGA, clamped to the 1.5 V input range.
    fn input_uv(&self, ain: usize, config: usize) -> u64 {
        let mv = match self.inputs_mv.get(ain) {
            Some(&mv) if mv != u16::MAX => u64::from(mv),
            _ => 0,
        };
        (mv * 1000 * self.gain_halves(config) / 2).min(MAX_UV)
    }

    fn single_ended(&self, config: usize) -> u32 {
        (self.input_uv(config, config) * FULL_SCALE / MAX_UV) as u32 ^ 0x800
    }

    fn differential(&self, positive_uv: u64, negative_uv: u64) -> u32 {
        ((positive_uv + MAX_UV - negative_uv) * FULL_SCALE / (2 * MAX_UV)) as u32
    }

    fn sweep(&mut self) {
        if self.cfg0 & CFG0_ADC_EN == 0 {
            return;
        }
        let mut status = self.status.get();
        for config in 0..CONFIG_CHANNELS {
            if self.chan_cfg[config] & CHAN_EN == 0 {
                continue;
            }
            let (word, code) = if config < DIFF_OFFSET {
                (config, self.single_ended(config))
            } else if config == TEMP_CONFIG {
                (DIFF_OFFSET, self.differential(0, 0))
            } else {
                let pair = 2 * (config - DIFF_OFFSET);
                let positive = self.input_uv(pair, config);
                let negative = self.input_uv(pair + 1, config);
                (pair, self.differential(positive, negative))
            };
            self.data[word] = code;
            status |= (1 << config) | CONV_COMPL_COMB;
        }
        self.status.set(status);
    }

    /// The config channel that owns data word `word` (Renode
    /// `DataToConfigChannel`).
    fn data_owner(&self, word: usize) -> usize {
        let diff = DIFF_OFFSET + word / 2;
        if word >= DIFF_OFFSET || self.chan_cfg[diff] & CHAN_EN != 0 {
            diff
        } else {
            word
        }
    }

    fn data_word(offset: u64) -> Option<usize> {
        let word = offset.checked_sub(DATA)? / 4;
        (word < DATA_WORDS as u64).then_some(word as usize)
    }

    fn chan_index(offset: u64) -> Option<usize> {
        let index = offset.checked_sub(CHAN_CFG)? / 4;
        (index < CONFIG_CHANNELS as u64).then_some(index as usize)
    }

    fn read_word(&self, offset: u64) -> u32 {
        let offset = offset & !3;
        if let Some(index) = Self::chan_index(offset) {
            return self.chan_cfg[index];
        }
        if let Some(word) = Self::data_word(offset) {
            return self.data[word];
        }
        match offset {
            SPB_CFG_0 => self.cfg0,
            SPB_CFG_1 => self.cfg1,
            TIM_CTRL => self.tim_ctrl,
            SEQ_CTRL => self.seq_ctrl,
            STAB_CTRL => self.stab_ctrl,
            INT_STATUS => self.status.get(),
            _ => 0,
        }
    }

    /// Read side effects for the byte lanes in `lanes` of the word at `offset`.
    fn after_read(&self, offset: u64, lanes: u32) {
        let offset = offset & !3;
        if offset == INT_STATUS {
            self.status.set(self.status.get() & !lanes);
        } else if let Some(word) = Self::data_word(offset) {
            if lanes & 0xFF != 0 {
                let owner = self.data_owner(word);
                self.status.set(self.status.get() & !(1 << owner));
            }
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let offset = offset & !3;
        if let Some(index) = Self::chan_index(offset) {
            self.chan_cfg[index] = value & CHAN_MASK;
            return;
        }
        match offset {
            SPB_CFG_0 => self.cfg0 = value & CFG0_MASK,
            SPB_CFG_1 => {
                if value & CFG1_REGDEF != 0 {
                    self.reset_registers();
                    return;
                }
                self.cfg1 = value & CFG1_RST_CCONV;
                if value & CFG1_TRIGGER != 0 {
                    self.sweep();
                }
            }
            TIM_CTRL => self.tim_ctrl = value & TIM_MASK,
            SEQ_CTRL => self.seq_ctrl = value & SEQ_MASK,
            STAB_CTRL => self.stab_ctrl = value & STAB_MASK,
            _ => {}
        }
    }

    fn irq_asserted(&self) -> bool {
        let enables = CFG0_ADC_INTR_EN | CFG0_CONV_COMPL_IEN;
        self.cfg0 & enables == enables && self.status.get() & PENDING_MASK != 0
    }
}

impl Peripheral for CaesAdc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(INPUTS as u8)
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
        self.reset_registers();
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let shift = (offset & 3) * 8;
        let value = (self.read_word(offset) >> shift) as u8;
        self.after_read(offset, 0xFF << shift);
        Ok(value)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        // TRIGGER and REGDEF are not stored, so a byte write to another
        // SPB_CFG_1 byte never starts a sweep.
        let current = self.read_word(aligned);
        let next = (current & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(aligned, next);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let value = self.read_word(offset);
        self.after_read(offset, u32::MAX);
        Ok(value)
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

    fn adc_with(inputs: &[(u8, u16)]) -> CaesAdc {
        let mut adc = CaesAdc::new();
        for &(channel, mv) in inputs {
            assert!(adc.set_adc_channel_input(channel, mv));
        }
        adc
    }

    /// Vendor quick start: REGDEF, SINGLESWEEP, ADC_EN, channel EN with
    /// GAIN, TRIGGER, poll INT_STATUS, read DATA_OUT[11:0].
    fn sweep(adc: &mut CaesAdc, cfg0: u32, channels: &[(usize, u32)]) {
        adc.write_u32(SPB_CFG_1, CFG1_REGDEF).unwrap();
        adc.write_u32(SPB_CFG_0, cfg0 | (1 << 13) | CFG0_ADC_EN)
            .unwrap();
        for &(config, gain) in channels {
            adc.write_u32(CHAN_CFG + 4 * config as u64, (gain << 4) | CHAN_EN)
                .unwrap();
        }
        adc.write_u32(SPB_CFG_1, CFG1_TRIGGER).unwrap();
    }

    fn data_out(adc: &CaesAdc, word: usize) -> u32 {
        adc.read_u32(DATA + 4 * word as u64).unwrap() & 0xFFF
    }

    /// The vendor reader: flip bit 11 back to offset binary.
    fn offset_binary(code: u32) -> u32 {
        code ^ 0x800
    }

    #[test]
    fn single_ended_codes_flip_the_msb_and_scale_with_the_pga() {
        let mut adc = adc_with(&[(0, 0), (1, 750), (2, 1500), (3, 2000), (15, 375)]);
        sweep(&mut adc, 0, &[(0, 1), (1, 1), (2, 1), (3, 1), (15, 1)]);
        assert_eq!(data_out(&adc, 0), 0x800, "0 V is the two's complement zero");
        assert_eq!(offset_binary(data_out(&adc, 1)), 750 * 4095 / 1500);
        assert_eq!(data_out(&adc, 2), 0x7FF, "1.5 V is full scale");
        assert_eq!(data_out(&adc, 3), 0x7FF, "above 1.5 V clamps");
        assert_eq!(offset_binary(data_out(&adc, 15)), 375 * 4095 / 1500);

        // PGA: 0.5 V/V doubles the range, x4 quarters it, x16 for 5..7.
        let mut adc = adc_with(&[(0, 2790), (1, 300), (2, 50), (3, 50), (4, 50)]);
        sweep(
            &mut adc,
            CFG0_EN_PGA,
            &[(0, 0), (1, 3), (2, 5), (3, 6), (4, 7)],
        );
        assert_eq!(offset_binary(data_out(&adc, 0)), 2790 * 4095 / 3000);
        assert_eq!(offset_binary(data_out(&adc, 1)), 1200 * 4095 / 1500);
        for word in 2..=4 {
            assert_eq!(offset_binary(data_out(&adc, word)), 800 * 4095 / 1500);
        }
        // Without EN_PGA the GAIN field is ignored.
        let mut adc = adc_with(&[(0, 2790)]);
        sweep(&mut adc, 0, &[(0, 0)]);
        assert_eq!(data_out(&adc, 0), 0x7FF);
    }

    #[test]
    fn differential_pairs_and_temperature_land_in_even_words() {
        let mut adc = adc_with(&[(2, 1200), (3, 300), (4, 100), (5, 900)]);
        // DIFFCHAN 1 = AIN2/AIN3 -> word 2, DIFFCHAN 2 = AIN4/AIN5 -> word 4.
        sweep(&mut adc, 0, &[(2, 1), (17, 1), (18, 1), (TEMP_CONFIG, 1)]);
        assert_eq!(data_out(&adc, 2), (900 + 1500) * 4095 / 3000);
        assert_eq!(data_out(&adc, 4), (1500 - 800) * 4095 / 3000);
        assert_eq!(data_out(&adc, 16), 0x7FF, "temperature: zero differential");
        // The differential result replaced the single-ended one in word 2.
        assert_ne!(data_out(&adc, 2), (1200 * 4095 / 1500) ^ 0x800);
        // Full-scale differential both ways.
        let mut adc = adc_with(&[(0, 1500), (1, 0)]);
        sweep(&mut adc, 0, &[(16, 1)]);
        assert_eq!(data_out(&adc, 0), 4095);
        let mut adc = adc_with(&[(0, 0), (1, 1500)]);
        sweep(&mut adc, 0, &[(16, 1)]);
        assert_eq!(data_out(&adc, 0), 0);
    }

    #[test]
    fn status_pends_per_channel_and_clears_on_read() {
        let mut adc = adc_with(&[(0, 100), (5, 200)]);
        // Polled, no interrupt enables (errata 8.2).
        sweep(&mut adc, 0, &[(0, 1), (5, 1)]);
        assert!(!adc.irq_asserted());
        assert_eq!(adc.status.get(), CONV_COMPL_COMB | 1 | (1 << 5));
        // The DATA read clears its channel's pending bit only.
        data_out(&adc, 0);
        assert_eq!(adc.status.get(), CONV_COMPL_COMB | (1 << 5));
        // INT_STATUS reads clear the lanes they read: byte 3 is COMB.
        assert_eq!(adc.read(INT_STATUS + 3).unwrap(), 0x80);
        assert_eq!(adc.status.get(), 1 << 5);
        assert_eq!(adc.read_u32(INT_STATUS).unwrap(), 1 << 5);
        assert_eq!(adc.read_u32(INT_STATUS).unwrap(), 0);

        // IRQ needs both enables and a pending channel.
        sweep(&mut adc, CFG0_CONV_COMPL_IEN, &[(5, 1)]);
        assert!(!adc.irq_asserted(), "ADC_INTR_EN gates the line");
        adc.write_u32(SPB_CFG_0, adc.cfg0 | CFG0_ADC_INTR_EN)
            .unwrap();
        assert!(adc.irq_asserted());
        assert!(adc.tick().irq);
        data_out(&adc, 5);
        assert!(!adc.irq_asserted(), "the DATA read acknowledges");
        assert_eq!(adc.status.get(), CONV_COMPL_COMB);
    }

    #[test]
    fn trigger_needs_adc_en_and_holds_inputs() {
        let mut adc = adc_with(&[(0, 600)]);
        adc.write_u32(CHAN_CFG, (1 << 4) | CHAN_EN).unwrap();
        adc.write_u32(SPB_CFG_1, CFG1_TRIGGER).unwrap();
        assert_eq!(adc.status.get(), 0, "ADC_EN clear: no sweep");
        assert_eq!(data_out(&adc, 0), 0);
        adc.write_u32(SPB_CFG_0, CFG0_ADC_EN).unwrap();
        adc.write_u32(SPB_CFG_1, CFG1_TRIGGER).unwrap();
        let held = data_out(&adc, 0);
        assert_eq!(offset_binary(held), 600 * 4095 / 1500);
        adc.set_adc_channel_input(0, 1200);
        assert_eq!(
            data_out(&adc, 0),
            held,
            "sample and hold until the next trigger"
        );
        // An undriven AIN converts as 0 V.
        assert!(adc.clear_adc_channel_input(0));
        adc.write_u32(SPB_CFG_1, CFG1_TRIGGER).unwrap();
        assert_eq!(data_out(&adc, 0), 0x800);
        assert!(!adc.set_adc_channel_input(16, 100), "only AIN0..15 route");
        assert_eq!(adc.adc_channel_count(), Some(16));
        // TRIGGER self-clears; a byte write to another CFG_1 byte does not sweep.
        assert_eq!(adc.read_u32(SPB_CFG_1).unwrap(), 0);
        adc.read_u32(INT_STATUS).unwrap();
        adc.write(SPB_CFG_1 + 1, 0).unwrap();
        assert_eq!(adc.status.get(), 0);
    }

    #[test]
    fn reset_values_masks_and_regdef() {
        let mut adc = adc_with(&[(0, 900)]);
        assert_eq!(adc.read_u32(CHAN_CFG).unwrap(), 0x10);
        assert_eq!(adc.read_u32(0x70).unwrap(), 0x10, "TEMPCHAN_CFG");
        assert_eq!(adc.read_u32(TIM_CTRL).unwrap(), 0x64);
        assert_eq!(adc.read_u32(STAB_CTRL).unwrap(), 0x81E);
        for (offset, mask) in [
            (SPB_CFG_0, CFG0_MASK),
            (CHAN_CFG + 4 * 7, CHAN_MASK),
            (0x50, CHAN_MASK),
            (TIM_CTRL, TIM_MASK),
            (SEQ_CTRL, SEQ_MASK),
            (STAB_CTRL, STAB_MASK),
        ] {
            adc.write_u32(offset, u32::MAX).unwrap();
            assert_eq!(adc.read_u32(offset).unwrap(), mask, "{offset:#x}");
        }
        adc.write_u32(SPB_CFG_1, CFG1_TRIGGER).unwrap();
        assert_ne!(adc.status.get(), 0);
        adc.write_u32(SPB_CFG_1, CFG1_REGDEF).unwrap();
        assert_eq!(adc.read_u32(SPB_CFG_0).unwrap(), 0);
        assert_eq!(adc.read_u32(TIM_CTRL).unwrap(), 0x64);
        assert_eq!(adc.status.get(), 0);
        assert_eq!(data_out(&adc, 0), 0, "REGDEF clears the results");
        adc.on_system_reset();
        sweep(&mut adc, 0, &[(0, 1)]);
        assert_eq!(offset_binary(data_out(&adc, 0)), 900 * 4095 / 1500);
    }
}
