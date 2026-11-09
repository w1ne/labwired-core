// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Lattice Nexus hard ADC analog core, behind a small LabWired fabric bridge.
//!
//! The ADC itself is a two-channel, 12-bit SAR hard macro. Unlike a MCU ADC,
//! the vendor IP exposes fabric signals (`adc_soc_i`, `adc0_ch_sel_i`,
//! `adc0_o`, `adc_eoc_o`, etc.), not a silicon MMIO register block. The bridge
//! below gives those signals a deterministic bus interface for board models;
//! its offsets are LabWired's wrapper contract, not a Lattice register map.
//!
//! Input slots are terminal voltages in millivolts. ADC0's dual-mode pairs
//! use P=0..7 / N=8..15 and its dedicated pair uses P/N=32/33. ADC1's pairs
//! use P=16..23 / N=24..31 and its dedicated pair uses P/N=34/35. The two
//! ADCs sample together. A conversion selects one of the eight differential
//! pairs (0..7) or the dedicated pair (0xB); internal supply and temperature
//! selections currently read zero.
//!
//! Lattice's Nexus ADC guide specifies 12 bits, up to 1 MSPS, a reference from
//! 1.0 V to 1.8 V, and calibration before accurate conversions. The model
//! keeps the reference configurable, uses the 4096/VREF transfer, and models
//! calibration/conversion waits in deterministic peripheral ticks. Tick
//! counts are simulation timing, not clock-accurate silicon timing.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const CONTROL: u64 = 0x00;
const CHANNEL_SELECT: u64 = 0x04;
const STATUS: u64 = 0x08;
const ADC0_DATA: u64 = 0x0C;
const ADC1_DATA: u64 = 0x10;

const ENABLE: u32 = 1 << 0;
const RESET_N: u32 = 1 << 1;
const CALIBRATE: u32 = 1 << 2;
const START_OF_CONVERSION: u32 = 1 << 3;
const CONVERSION_STOP: u32 = 1 << 4;
const ADC0_BIPOLAR: u32 = 1 << 5;
const ADC1_BIPOLAR: u32 = 1 << 6;
const TWOS_COMPLEMENT: u32 = 1 << 7;

const CONVERSION_ON_GOING: u32 = 1 << 0;
const END_OF_CONVERSION: u32 = 1 << 1;
const CALIBRATION_READY: u32 = 1 << 2;

const INPUT_CHANNELS: usize = 36;
const CALIBRATION_TICKS: u32 = 5_500;
const CONVERSION_TICKS: u8 = 4;
const ADC0_DEDICATED_SELECT: u8 = 0xB;
const ADC1_DEDICATED_SELECT: u8 = 0xB;

/// Nexus ADC macro behind a simulation bus bridge.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LatticeNexusAdc {
    control: u32,
    channel_select: u32,
    status: u32,
    data: [u16; 2],
    pending_data: [u16; 2],
    input_mv: Vec<Option<u16>>,
    vref_mv: u16,
    calibration_ticks: u32,
    conversion_ticks: u8,
}

impl Default for LatticeNexusAdc {
    fn default() -> Self {
        Self::new(1800)
    }
}

impl LatticeNexusAdc {
    pub fn new(vref_mv: u16) -> Self {
        Self {
            control: 0,
            channel_select: 0,
            status: 0,
            data: [0; 2],
            pending_data: [0; 2],
            input_mv: vec![None; INPUT_CHANNELS],
            vref_mv: vref_mv.clamp(1000, 1800),
            calibration_ticks: 0,
            conversion_ticks: 0,
        }
    }

    fn reset(&mut self) {
        let inputs = self.input_mv.clone();
        let vref = self.vref_mv;
        *self = Self::new(vref);
        self.input_mv = inputs;
    }

    fn input(&self, channel: usize) -> i32 {
        self.input_mv
            .get(channel)
            .and_then(|value| *value)
            .map(i32::from)
            .unwrap_or(0)
    }

    fn pair_slots(adc: usize, select: u8) -> Option<(usize, usize)> {
        if select <= 7 {
            return Some(if adc == 0 {
                (usize::from(select), 8 + usize::from(select))
            } else {
                (16 + usize::from(select), 24 + usize::from(select))
            });
        }
        match (adc, select) {
            (0, ADC0_DEDICATED_SELECT) => Some((32, 33)),
            (1, ADC1_DEDICATED_SELECT) => Some((34, 35)),
            _ => None,
        }
    }

    fn convert(&self, adc: usize, select: u8) -> u16 {
        let Some((positive, negative)) = Self::pair_slots(adc, select) else {
            return 0;
        };
        let differential_mv = self.input(positive) - self.input(negative);
        let bipolar_bit = if adc == 0 { ADC0_BIPOLAR } else { ADC1_BIPOLAR };
        let code = if self.control & bipolar_bit == 0 {
            (differential_mv * 4096 / i32::from(self.vref_mv)).clamp(0, 4095)
        } else if self.control & TWOS_COMPLEMENT != 0 {
            (differential_mv * 2048 / i32::from(self.vref_mv)).clamp(-2048, 2047) & 0x0FFF
        } else {
            // Offset binary: zero differential is mid-scale.
            (2048 + differential_mv * 2048 / i32::from(self.vref_mv)).clamp(0, 4095)
        };
        code as u16
    }

    fn read_register(&self, offset: u64) -> u32 {
        match offset {
            CONTROL => self.control,
            CHANNEL_SELECT => self.channel_select,
            STATUS => self.status,
            ADC0_DATA => u32::from(self.data[0]),
            ADC1_DATA => u32::from(self.data[1]),
            _ => 0,
        }
    }

    fn write_control(&mut self, value: u32) {
        let old = self.control;
        self.control = value & 0xFF;
        if self.control & RESET_N == 0 {
            self.reset();
            self.control = value & 0xFF;
            return;
        }
        if old & RESET_N == 0 {
            self.status = 0;
            self.calibration_ticks = 0;
            self.conversion_ticks = 0;
        }
        // `adc_en_i` gates the internal clocks but does not reset calibration
        // state. Pending calibration and conversion work resumes when enabled.
        if self.control & CALIBRATE != 0 && old & CALIBRATE == 0 && self.control & ENABLE != 0 {
            self.status &= !CALIBRATION_READY;
            self.calibration_ticks = CALIBRATION_TICKS;
        }
        if self.control & CONVERSION_STOP != 0 {
            self.conversion_ticks = 0;
            self.status &= !CONVERSION_ON_GOING;
        }
        if self.control & START_OF_CONVERSION != 0
            && old & START_OF_CONVERSION == 0
            && self.control & ENABLE != 0
            && self.control & RESET_N != 0
            && self.status & CALIBRATION_READY != 0
            && self.conversion_ticks == 0
        {
            let adc0_select = (self.channel_select & 0xF) as u8;
            let adc1_select = ((self.channel_select >> 4) & 0xF) as u8;
            self.pending_data = [self.convert(0, adc0_select), self.convert(1, adc1_select)];
            self.status &= !END_OF_CONVERSION;
            self.status |= CONVERSION_ON_GOING;
            self.conversion_ticks = CONVERSION_TICKS;
        }
    }

    fn update_byte(&mut self, offset: u64, value: u8) {
        let register = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        let mask = 0xFFu32 << shift;
        match register {
            CONTROL => {
                let next = (self.control & !mask) | (u32::from(value) << shift);
                self.write_control(next);
            }
            CHANNEL_SELECT => {
                self.channel_select = (self.channel_select & !mask) | (u32::from(value) << shift);
            }
            STATUS => self.status &= !(u32::from(value) << shift),
            _ => {}
        }
    }
}

impl Peripheral for LatticeNexusAdc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let value = self.read_register(offset & !3);
        Ok((value >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.update_byte(offset, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.control & ENABLE != 0 {
            if self.calibration_ticks > 0 {
                self.calibration_ticks -= 1;
                if self.calibration_ticks == 0 {
                    self.status |= CALIBRATION_READY;
                }
            }
            if self.conversion_ticks > 0 {
                self.conversion_ticks -= 1;
                if self.conversion_ticks == 0 {
                    self.data = self.pending_data;
                    self.status &= !CONVERSION_ON_GOING;
                    self.status |= END_OF_CONVERSION;
                }
            }
        }
        PeripheralTickResult::default()
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        let Some(input) = self.input_mv.get_mut(usize::from(channel)) else {
            return false;
        };
        *input = Some(millivolts);
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        let Some(input) = self.input_mv.get_mut(usize::from(channel)) else {
            return false;
        };
        *input = None;
        true
    }

    fn adc_channel_count(&self) -> Option<u8> {
        Some(INPUT_CHANNELS as u8)
    }

    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }

    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || !(1.0..=1.8).contains(&volts) {
            return false;
        }
        self.vref_mv = (volts * 1000.0).round() as u16;
        true
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        *self = serde_json::from_value(state)
            .map_err(|error| crate::SimulationError::Other(error.to_string()))?;
        Ok(())
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

    #[test]
    fn dedicated_pair_uses_differential_input_and_vref() {
        let mut adc = LatticeNexusAdc::new(1800);
        assert!(adc.set_adc_channel_input(32, 1200));
        assert!(adc.set_adc_channel_input(33, 300));
        assert_eq!(
            adc.convert(0, ADC0_DEDICATED_SELECT),
            (900u32 * 4096 / 1800) as u16
        );

        adc.control = ENABLE | RESET_N | ADC0_BIPOLAR | TWOS_COMPLEMENT;
        assert_eq!(adc.convert(0, ADC0_DEDICATED_SELECT), 1024);
        assert!(adc.set_adc_channel_input(33, 1500));
        assert_eq!(
            adc.convert(0, ADC0_DEDICATED_SELECT),
            (-341i32 & 0x0FFF) as u16
        );
    }

    #[test]
    fn conversion_waits_for_calibration_and_latches_both_results_together() {
        let mut adc = LatticeNexusAdc::new(1800);
        assert!(adc.set_adc_channel_input(32, 900));
        assert!(adc.set_adc_channel_input(34, 450));
        adc.write_u32(CONTROL, ENABLE | RESET_N).unwrap();
        adc.write_u32(CHANNEL_SELECT, 0xBB).unwrap();
        adc.write_u32(CONTROL, ENABLE | RESET_N | START_OF_CONVERSION)
            .unwrap();
        assert_eq!(adc.read_u32(STATUS).unwrap() & CONVERSION_ON_GOING, 0);
        adc.write_u32(CONTROL, ENABLE | RESET_N | CALIBRATE)
            .unwrap();
        for _ in 0..CALIBRATION_TICKS {
            adc.tick();
        }
        assert_ne!(adc.read_u32(STATUS).unwrap() & CALIBRATION_READY, 0);
        adc.write_u32(CONTROL, ENABLE | RESET_N | CALIBRATE | START_OF_CONVERSION)
            .unwrap();
        assert_ne!(adc.read_u32(STATUS).unwrap() & CONVERSION_ON_GOING, 0);
        for _ in 0..CONVERSION_TICKS {
            adc.tick();
        }
        assert_ne!(adc.read_u32(STATUS).unwrap() & END_OF_CONVERSION, 0);
        assert_eq!(adc.read_u32(ADC0_DATA).unwrap(), 900u32 * 4096 / 1800);
        assert_eq!(adc.read_u32(ADC1_DATA).unwrap(), 450u32 * 4096 / 1800);
    }

    #[test]
    fn disabling_adc_clock_gate_preserves_calibration_and_pauses_conversion() {
        let mut adc = LatticeNexusAdc::new(1800);
        assert!(adc.set_adc_channel_input(32, 900));
        adc.write_u32(CHANNEL_SELECT, 0xBB).unwrap();
        adc.write_u32(CONTROL, ENABLE | RESET_N | CALIBRATE)
            .unwrap();
        for _ in 0..CALIBRATION_TICKS {
            adc.tick();
        }
        assert_ne!(adc.read_u32(STATUS).unwrap() & CALIBRATION_READY, 0);

        adc.write_u32(CONTROL, ENABLE | RESET_N | CALIBRATE | START_OF_CONVERSION)
            .unwrap();
        adc.tick();
        adc.tick();
        adc.write_u32(CONTROL, RESET_N | CALIBRATE).unwrap();
        for _ in 0..10 {
            adc.tick();
        }
        assert_ne!(adc.read_u32(STATUS).unwrap() & CALIBRATION_READY, 0);
        assert_ne!(adc.read_u32(STATUS).unwrap() & CONVERSION_ON_GOING, 0);
        assert_eq!(adc.read_u32(STATUS).unwrap() & END_OF_CONVERSION, 0);
        assert_eq!(adc.read_u32(ADC0_DATA).unwrap(), 0);

        adc.write_u32(CONTROL, ENABLE | RESET_N | CALIBRATE)
            .unwrap();
        for _ in 0..2 {
            adc.tick();
        }
        assert_ne!(adc.read_u32(STATUS).unwrap() & END_OF_CONVERSION, 0);
        assert_eq!(adc.read_u32(ADC0_DATA).unwrap(), 900u32 * 4096 / 1800);
    }
}
