// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! QuickLogic EOS S3 12-bit ADC (the "JTM" block at `AON_PERIPH_BASE +
//! 0x5A00`). Register layout, bit fields and the firmware handshake come from
//! QuickLogic qorc-sdk `HAL/src/eoss3_hal_adc.c` (`JTM_OUT` @0x0,
//! `JTM_STATUS` @0x4, `JTM_CONTROL` @0x8; `CONTROL.SOC` bit 0, `.SEL` bit 1,
//! `.MEAS_EN` bit 2; `STATUS.EOC` bit 0; a 12-bit `OUT`) and `eoss3_dev.h`
//! (`JTM_BASE`). The 1.4 V full scale and the EOC state sequence come from
//! Renode `Peripherals/Analog/EOSS3_ADC.cs`, which cites the EOS S3 datasheet
//! v3.3f.
//!
//! Modelled: two channels (`SEL` = 0 or 1). A write with `SOC` set samples
//! the selected channel and starts a conversion. `STATUS.EOC` reads 1 while
//! idle, then 0 on the first read after the start ("Wait for it to go low",
//! `HAL_ADC_StartConversion`), then 1 again when the result is ready
//! (`HAL_ADC_GetData`), the same three-state sequence Renode uses. `OUT` is
//! `V / 1.4 V * 4095` truncated and clamped to `[0, 4095]`, as Renode's
//! `ToADCRawValue`. Unlike Renode, which reads the input live at the `OUT`
//! read, the input is held at the `SOC` write, as a sample-and-hold would.
//!
//! Not modelled: `MEAS_EN` (battery measurement) is stored only, there is no
//! conversion clock, and the ADC interrupt in the interrupt controller is not
//! raised.

use std::cell::Cell;

use crate::{Peripheral, PeripheralTickResult, SimResult};

const CHANNELS: usize = 2;
/// EOS S3 datasheet v3.3f full scale (Renode `EOSS3_ADC.ReferenceVoltage`).
const DEFAULT_VREF_MV: u32 = 1400;
const FULL_SCALE: u32 = (1 << 12) - 1;

// ── Register map (qorc-sdk eoss3_hal_adc.c) ─────────────────────────────────
const OUT: u64 = 0x0;
const STATUS: u64 = 0x4;
const CONTROL: u64 = 0x8;

const CONTROL_SOC: u32 = 1 << 0;
const CONTROL_SEL: u32 = 1 << 1;
const CONTROL_MEAS_EN: u32 = 1 << 2;

/// The `STATUS.EOC` handshake (Renode `EOSS3_ADC.State`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum State {
    /// EOC reads 1.
    Idle,
    /// EOC reads 0 once, then moves to `SampleReady`.
    ConversionStarted,
    /// EOC reads 1 once, then moves to `Idle`.
    SampleReady,
}

/// EOS S3 ADC: SOC-started single conversions on the routed channels.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Eoss3Adc {
    control: u32,
    /// The held 12-bit result.
    result: u32,
    /// Routed input levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
    vref_mv: u32,
    /// `STATUS` reads advance the handshake while `read` takes `&self`.
    state: Cell<State>,
}

impl Default for Eoss3Adc {
    fn default() -> Self {
        Self::new(DEFAULT_VREF_MV)
    }
}

impl Eoss3Adc {
    pub fn new(vref_mv: u32) -> Self {
        Self {
            control: 0,
            result: 0,
            inputs_mv: [u16::MAX; CHANNELS],
            vref_mv: vref_mv.max(1),
            state: Cell::new(State::Idle),
        }
    }

    /// Sample the `SEL` channel; an undriven input reads 0 V.
    fn convert(&mut self) {
        let channel = usize::from(self.control & CONTROL_SEL != 0);
        let mv = match self.inputs_mv[channel] {
            u16::MAX => 0,
            mv => u32::from(mv),
        };
        self.result = (mv.min(self.vref_mv) * FULL_SCALE) / self.vref_mv;
    }

    fn eoc(&self) -> u32 {
        let (eoc, next) = match self.state.get() {
            State::Idle => (1, State::Idle),
            State::ConversionStarted => (0, State::SampleReady),
            State::SampleReady => (1, State::Idle),
        };
        self.state.set(next);
        eoc
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset & !3 {
            OUT => self.result,
            STATUS => self.eoc(),
            CONTROL => self.control,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if offset & !3 != CONTROL {
            return;
        }
        self.control = value & (CONTROL_SOC | CONTROL_SEL | CONTROL_MEAS_EN);
        // Renode's SOC write callback runs on every CONTROL write: SOC = 1
        // starts a conversion, SOC = 0 (HAL_ADC_Init, HAL_ADC_GetData)
        // returns the handshake to idle.
        if self.control & CONTROL_SOC != 0 {
            self.convert();
            self.state.set(State::ConversionStarted);
        } else {
            self.state.set(State::Idle);
        }
    }
}

impl Peripheral for Eoss3Adc {
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
        *self = Self::new(self.vref_mv);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        // A byte read of STATUS advances the handshake like a word read;
        // the upper bytes carry no state.
        let word = if offset & 3 == 0 || offset & !3 != STATUS {
            self.read_word(offset)
        } else {
            0
        };
        Ok((word >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // Every field sits in the low byte of its word.
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

    fn adc_with(inputs: &[(u8, u16)]) -> Eoss3Adc {
        let mut adc = Eoss3Adc::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv));
        }
        adc
    }

    /// qorc-sdk HAL_ADC_Init + HAL_ADC_StartConversion + HAL_ADC_GetData.
    fn hal_read(adc: &mut Eoss3Adc, channel: u32) -> u32 {
        adc.write_u32(CONTROL, 0).unwrap();
        adc.write_u32(CONTROL, channel << 1).unwrap();
        let temp = adc.read_u32(CONTROL).unwrap();
        adc.write_u32(CONTROL, temp | CONTROL_SOC).unwrap();
        // "Wait for it to go low": the first STATUS read after SOC is 0.
        assert_eq!(adc.read_u32(STATUS).unwrap(), 0, "EOC drops after SOC");
        // "Check if conversion is done".
        assert_eq!(adc.read_u32(STATUS).unwrap(), 1, "EOC rises when done");
        let data = adc.read_u32(OUT).unwrap() & 0xFFF;
        let temp = adc.read_u32(CONTROL).unwrap();
        adc.write_u32(CONTROL, temp & !CONTROL_SOC).unwrap();
        data
    }

    #[test]
    fn hal_sequence_converts_each_channel_against_1v4() {
        let mut adc = adc_with(&[(0, 700), (1, 350)]);
        assert_eq!(hal_read(&mut adc, 0), 700 * 4095 / 1400);
        assert_eq!(hal_read(&mut adc, 0), 2047, "half scale");
        assert_eq!(hal_read(&mut adc, 1), 350 * 4095 / 1400);
        // Above the 1.4 V full scale the code clamps at 4095.
        adc.set_adc_channel_input(1, 3300);
        assert_eq!(hal_read(&mut adc, 1), 4095);
        // An undriven channel reads 0.
        adc.clear_adc_channel_input(0);
        assert_eq!(hal_read(&mut adc, 0), 0);
        // Only two channels exist.
        assert!(!adc.set_adc_channel_input(2, 100));
        assert_eq!(adc.adc_channel_count(), Some(2));
    }

    #[test]
    fn eoc_follows_the_three_state_handshake_and_holds_the_sample() {
        let mut adc = adc_with(&[(0, 1000)]);
        // Idle: EOC reads 1 on every read.
        assert_eq!(adc.read_u32(STATUS).unwrap(), 1);
        assert_eq!(adc.read_u32(STATUS).unwrap(), 1);
        adc.write_u32(CONTROL, CONTROL_SOC).unwrap();
        // The input is held at SOC: a later change does not reach OUT.
        adc.set_adc_channel_input(0, 200);
        assert_eq!(adc.read_u32(STATUS).unwrap(), 0);
        assert_eq!(adc.read_u32(STATUS).unwrap(), 1);
        assert_eq!(adc.read_u32(STATUS).unwrap(), 1, "back to idle");
        assert_eq!(adc.read_u32(OUT).unwrap(), 1000 * 4095 / 1400);
        // A CONTROL write with SOC clear returns the handshake to idle.
        adc.write_u32(CONTROL, CONTROL_SOC | CONTROL_MEAS_EN)
            .unwrap();
        adc.write_u32(CONTROL, CONTROL_MEAS_EN).unwrap();
        assert_eq!(adc.read_u32(STATUS).unwrap(), 1);
        assert_eq!(adc.read_u32(CONTROL).unwrap(), CONTROL_MEAS_EN);
        // Byte writes start a conversion too; reserved bits do not stick.
        adc.write(CONTROL, 0xFF).unwrap();
        assert_eq!(adc.read_u32(CONTROL).unwrap(), 0x7);
        assert_eq!(adc.read(STATUS).unwrap(), 0);
        assert_eq!(adc.read(STATUS).unwrap(), 1);
        assert_eq!(adc.read_u32(OUT).unwrap(), 0, "SEL=1 is undriven");
    }

    #[test]
    fn reset_clears_registers_and_keeps_inputs() {
        let mut adc = adc_with(&[(1, 1400)]);
        assert_eq!(hal_read(&mut adc, 1), 4095);
        adc.write_u32(CONTROL, CONTROL_SOC | CONTROL_SEL).unwrap();
        adc.on_system_reset();
        assert_eq!(adc.read_u32(CONTROL).unwrap(), 0);
        assert_eq!(adc.read_u32(OUT).unwrap(), 0);
        assert_eq!(adc.read_u32(STATUS).unwrap(), 1);
        assert_eq!(hal_read(&mut adc, 1), 4095);
    }
}
