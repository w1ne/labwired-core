// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Bounded input conversion model for the nRF51 single-sample ADC.
//!
//! The register names and field encodings follow Nordic's vendor `nrf51.svd`
//! (SHA-256 `2929d85924068b02e429b2363bfed7a632978b13158180f1dc16cae57e9ce090`).
//! That SVD is vendor metadata, not an independent silicon validation. This
//! model covers held analog input channels, PSEL, the 8/9/10-bit result width,
//! the VBG reference, input 2/3 and 1/3 prescaling, START/END, and polling.
//! It does not claim conversion timing, external-reference pin routing, or the
//! supply-as-input paths.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const OFF_TASKS_START: u64 = 0x000;
const OFF_TASKS_STOP: u64 = 0x004;
const OFF_EVENTS_END: u64 = 0x100;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_BUSY: u64 = 0x400;
const OFF_ENABLE: u64 = 0x500;
const OFF_CONFIG: u64 = 0x504;
const OFF_RESULT: u64 = 0x508;

const EVENT_END: u32 = 1;
const INT_END: u32 = 1;
const VBG_MV: u64 = 1200;

#[derive(Debug)]
pub struct Nrf51Adc {
    events_end: u32,
    inten: u32,
    enable: u32,
    config: u32,
    result: u16,
    busy: bool,
    pending: bool,
    inputs_mv: [Option<u16>; 8],
    external_reference_v: Option<f64>,
}

impl Default for Nrf51Adc {
    fn default() -> Self {
        Self {
            events_end: 0,
            inten: 0,
            enable: 0,
            config: 0x18,
            result: 0,
            busy: false,
            pending: false,
            inputs_mv: [None; 8],
            external_reference_v: None,
        }
    }
}

impl Nrf51Adc {
    pub fn new() -> Self {
        Self::default()
    }

    fn input_selection(&self) -> Option<(u64, u64)> {
        match (self.config >> 2) & 0x7 {
            0 => Some((1, 1)),
            1 => Some((2, 3)),
            2 => Some((1, 3)),
            // Supply-as-input modes need a board supply model, which is outside
            // this peripheral's current contract.
            _ => None,
        }
    }

    fn reference_mv(&self) -> Option<u64> {
        match (self.config >> 5) & 0x3 {
            0 => Some(VBG_MV),
            1 => self
                .external_reference_v
                .filter(|volts| volts.is_finite() && *volts > 0.0)
                .map(|volts| (volts * 1000.0 + 0.5).floor() as u64),
            // VDD references need a board supply value, which this bounded
            // model does not infer from an unrelated chip's GPIO rail.
            _ => None,
        }
    }

    fn selected_input_mv(&self) -> Option<u64> {
        let psel = ((self.config >> 8) & 0xff) as u8;
        if psel == 0 || !psel.is_power_of_two() {
            return None;
        }
        let channel = psel.trailing_zeros() as usize;
        self.inputs_mv.get(channel)?.map(u64::from)
    }

    fn convert(&self) -> u16 {
        let Some((prescale_numerator, prescale_denominator)) = self.input_selection() else {
            return 0;
        };
        let Some(input_mv) = self.selected_input_mv() else {
            return 0;
        };
        let Some(reference_mv) = self.reference_mv().filter(|mv| *mv != 0) else {
            return 0;
        };

        let bits = match self.config & 0x3 {
            0 => 8,
            1 => 9,
            _ => 10,
        };
        let full_scale = 1u64 << bits;
        let code = input_mv
            .saturating_mul(prescale_numerator)
            .saturating_mul(full_scale)
            / prescale_denominator
            / reference_mv;
        code.min(full_scale - 1) as u16
    }
}

impl Peripheral for Nrf51Adc {
    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        let Some(input) = self.inputs_mv.get_mut(usize::from(channel)) else {
            return false;
        };
        *input = Some(millivolts);
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        let Some(input) = self.inputs_mv.get_mut(usize::from(channel)) else {
            return false;
        };
        *input = None;
        true
    }

    fn adc_channel_count(&self) -> Option<u8> {
        Some(8)
    }

    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }

    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || volts <= 0.0 {
            return false;
        }
        self.external_reference_v = Some(volts);
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.events_end != 0 && self.inten & INT_END != 0)
    }

    fn on_system_reset(&mut self) {
        self.events_end = 0;
        self.inten = 0;
        self.enable = 0;
        self.config = 0x18;
        self.result = 0;
        self.busy = false;
        self.pending = false;
    }

    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            OFF_TASKS_START | OFF_TASKS_STOP => 0,
            OFF_EVENTS_END => self.events_end,
            OFF_INTENSET | OFF_INTENCLR => self.inten,
            OFF_BUSY => u32::from(self.busy),
            OFF_ENABLE => self.enable,
            OFF_CONFIG => self.config,
            OFF_RESULT => u32::from(self.result),
            _ => {
                crate::census_reg!("nrf51.adc:Nrf51Adc", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            OFF_TASKS_START if value != 0 && self.enable == 1 && !self.pending => {
                self.busy = true;
                self.pending = true;
            }
            OFF_TASKS_STOP if value != 0 => {
                self.busy = false;
                self.pending = false;
            }
            OFF_EVENTS_END if value == 0 => self.events_end = 0,
            OFF_INTENSET => self.inten |= value & INT_END,
            OFF_INTENCLR => self.inten &= !(value & INT_END),
            OFF_ENABLE => self.enable = value & 0x3,
            OFF_CONFIG => self.config = value & 0x3ffff,
            OFF_TASKS_START | OFF_TASKS_STOP | OFF_EVENTS_END | OFF_BUSY | OFF_RESULT => {}
            _ => crate::census_reg!("nrf51.adc:Nrf51Adc", offset, "write"),
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if !self.pending {
            return PeripheralTickResult::default();
        }
        self.result = self.convert();
        self.busy = false;
        self.pending = false;
        let fired = self.events_end == 0;
        self.events_end = EVENT_END;
        PeripheralTickResult {
            irq: self.inten & INT_END != 0,
            fired_events: if fired {
                vec![OFF_EVENTS_END as u32]
            } else {
                Vec::new()
            },
            ..PeripheralTickResult::default()
        }
    }
}
