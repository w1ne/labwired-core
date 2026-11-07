// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Deterministic SAM D21 ADC slice for external single-ended AIN0..AIN19.
//!
//! It models software-triggered conversions while CTRLA.ENABLE is set, reads
//! the selected AIN only when its PORT pad is handed to peripheral function
//! B, supports the 12/10/8-bit resolutions and INT1V/INTVCC0/INTVCC1
//! references, and raises RESRDY when a conversion completes on the next bus
//! tick. Sampling time, averaging, correction, freerun, differential mode,
//! window monitoring, event triggers, DMA and generic-clock gating are outside
//! this slice. Inputs are ordinary LabWired ADC inputs, so any analog netlist
//! can drive them through the shared `board.analog.<pad>_volts` route.

use crate::{Bus, Peripheral, SimResult, SimulationError};
use std::any::Any;
use std::cell::Cell;

const ADC_CHANNELS: usize = 20;
const ENABLE: u8 = 1 << 1;
const RESRDY: u8 = 1;
const START: u8 = 1 << 1;
const PORT_GROUP_A: u64 = 0x4100_4400;
const PORT_GROUP_B: u64 = 0x4100_4480;

/// `(AIN, port group, pad)` external input muxes, from the SAM D21 G18A
/// function-B table. AIN16..19 are the PA08..PA11 external pads.
const AIN_PADS: [(u8, u8, u8); ADC_CHANNELS] = [
    (0, 0, 2),
    (1, 0, 3),
    (2, 1, 8),
    (3, 1, 9),
    (4, 0, 4),
    (5, 0, 5),
    (6, 0, 6),
    (7, 0, 7),
    (8, 1, 0),
    (9, 1, 1),
    (10, 1, 2),
    (11, 1, 3),
    (12, 1, 4),
    (13, 1, 5),
    (14, 1, 6),
    (15, 1, 7),
    (16, 0, 8),
    (17, 0, 9),
    (18, 0, 10),
    (19, 0, 11),
];

#[derive(Debug)]
pub struct Samd21Adc {
    ctrla: u8,
    refctrl: u8,
    avgctrl: u8,
    sampctrl: u8,
    ctrlb: u16,
    winctrl: u8,
    inputctrl: u32,
    evctrl: u8,
    inten: u8,
    intflag: Cell<u8>,
    result: u16,
    inputs_mv: [Option<u16>; ADC_CHANNELS],
    vddana_mv: u32,
    pending: bool,
    conversions: u64,
}

impl Default for Samd21Adc {
    fn default() -> Self {
        Self::new()
    }
}

impl Samd21Adc {
    pub fn new() -> Self {
        Self {
            ctrla: 0,
            refctrl: 0,
            avgctrl: 0,
            sampctrl: 0,
            ctrlb: 0,
            winctrl: 0,
            inputctrl: 0,
            evctrl: 0,
            inten: 0,
            intflag: Cell::new(0),
            result: 0,
            inputs_mv: [None; ADC_CHANNELS],
            vddana_mv: 3300,
            pending: false,
            conversions: 0,
        }
    }

    pub fn channel_input_mv(&self, channel: u8) -> Option<u16> {
        self.inputs_mv.get(usize::from(channel)).copied().flatten()
    }

    pub fn conversions(&self) -> u64 {
        self.conversions
    }

    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd21 ADC slice: {what}"
        )))
    }

    fn resolution_bits(&self) -> SimResult<u32> {
        match (self.ctrlb >> 4) & 0x3 {
            0 => Ok(12),
            2 => Ok(10),
            3 => Ok(8),
            _ => Self::refuse("16-bit averaging resolution"),
        }
    }

    fn reference_mv(&self) -> SimResult<u32> {
        match self.refctrl & 0x0f {
            0 => Ok(1000),
            1 => Ok(self.vddana_mv * 100 / 148),
            2 => Ok(self.vddana_mv / 2),
            _ => Self::refuse("external or reserved ADC reference"),
        }
    }

    fn convert(&mut self, bus: &dyn Bus) -> SimResult<bool> {
        if self.ctrla & ENABLE == 0 {
            return Ok(false);
        }
        let channel = (self.inputctrl & 0x1f) as usize;
        let Some(&(_, port, pin)) = AIN_PADS.get(channel) else {
            return Self::refuse("MUXPOS outside external AIN0..AIN19");
        };
        let group = if port == 0 {
            PORT_GROUP_A
        } else {
            PORT_GROUP_B
        };
        let pincfg = bus.read_u8(group + 0x40 + u64::from(pin)).unwrap_or(0);
        let pmux = bus.read_u8(group + 0x30 + u64::from(pin / 2)).unwrap_or(0);
        let function = if pin & 1 == 0 { pmux & 0x0f } else { pmux >> 4 };
        let millivolts = if pincfg & 1 != 0 && function == 1 {
            u32::from(self.inputs_mv[channel].unwrap_or(0))
        } else {
            0
        };
        let reference_mv = self.reference_mv()?;
        let bits = self.resolution_bits()?;
        let full_scale = (1u32 << bits) - 1;
        self.result = (millivolts * full_scale / reference_mv).min(full_scale) as u16;
        self.intflag.set(self.intflag.get() | RESRDY);
        self.conversions += 1;
        Ok(true)
    }

    fn read_reg(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            0x00 => u32::from(self.ctrla),
            0x01 => u32::from(self.refctrl),
            0x02 => u32::from(self.avgctrl),
            0x03 => u32::from(self.sampctrl),
            0x04 => u32::from(self.ctrlb),
            0x06 => 0, // reserved
            0x08 => u32::from(self.winctrl),
            0x0c => 0, // SWTRIG read-as-zero
            0x10 => self.inputctrl,
            0x14 => u32::from(self.evctrl),
            0x16 | 0x17 => u32::from(self.inten),
            0x18 => u32::from(self.intflag.get()),
            0x19 => 0, // synchronisation is immediate in this model
            0x1a => {
                self.intflag.set(self.intflag.get() & !RESRDY);
                u32::from(self.result)
            }
            0x1c | 0x1e | 0x20 | 0x24 | 0x2c | 0x2e => 0,
            _ => return Self::refuse("register read outside the conversion slice"),
        })
    }

    fn write_reg(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            0x00 if value & 1 != 0 => {
                let (inputs_mv, vddana_mv, conversions) =
                    (self.inputs_mv, self.vddana_mv, self.conversions);
                *self = Self {
                    inputs_mv,
                    vddana_mv,
                    conversions,
                    ..Self::new()
                };
            }
            0x00 if value & !0x06 == 0 => self.ctrla = value as u8,
            0x01 if value & !0x8f == 0 => self.refctrl = value as u8,
            0x02 if value == 0 => self.avgctrl = 0,
            0x03 if value & !0x3f == 0 => self.sampctrl = value as u8,
            // 12/10/8-bit single-ended, right-adjusted, no correction/freerun.
            0x04 if value & !0x0730 == 0 && (value & (3 << 4)) != (1 << 4) => {
                self.ctrlb = value as u16;
            }
            0x06 if value == 0 => self.winctrl = 0,
            0x08 if value == 0 => self.winctrl = 0,
            0x0c if value & !u32::from(START) == 0 => {
                if value & u32::from(START) != 0 {
                    self.intflag.set(self.intflag.get() & !RESRDY);
                    self.pending = true;
                }
            }
            // Single-ended inputs against GND; gain, scan and offset remain 0.
            0x10 if value & !0x1f == 0x1800 && value & 0x1f < ADC_CHANNELS as u32 => {
                self.inputctrl = value;
            }
            0x14 if value == 0 => self.evctrl = 0,
            0x16 => self.inten &= !(value as u8 & 0x0f),
            0x17 => self.inten |= value as u8 & 0x0f,
            0x18 => self.intflag.set(self.intflag.get() & !(value as u8 & 0x0f)),
            0x1c | 0x1e | 0x20 | 0x24 | 0x2c | 0x2e if value == 0 => {}
            _ => return Self::refuse("store outside the conversion slice"),
        }
        Ok(())
    }

    fn byte_register(offset: u64) -> SimResult<(u64, u64)> {
        let base = match offset {
            0x00..=0x03 | 0x08 | 0x0c | 0x14 | 0x16..=0x19 => offset,
            0x04..=0x05 => 0x04,
            0x10..=0x13 => 0x10,
            0x1a..=0x1b => 0x1a,
            0x1c..=0x1d => 0x1c,
            0x1e..=0x1f => 0x1e,
            0x20..=0x23 => 0x20,
            0x24..=0x27 => 0x24,
            0x2c..=0x2d => 0x2c,
            0x2e..=0x2f => 0x2e,
            _ => return Self::refuse("byte read outside the conversion slice"),
        };
        Ok((base, offset - base))
    }
}

impl Peripheral for Samd21Adc {
    fn on_system_reset(&mut self) {
        let (inputs_mv, vddana_mv, conversions) =
            (self.inputs_mv, self.vddana_mv, self.conversions);
        *self = Self {
            inputs_mv,
            vddana_mv,
            conversions,
            ..Self::new()
        };
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let (base, shift) = Self::byte_register(offset)?;
        Ok((self.read_reg(base)? >> (8 * shift)) as u8)
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(self.read_reg(offset)? as u16)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        self.read_reg(offset)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            0x05 => self.write_reg(
                0x04,
                u32::from(self.ctrlb & 0x00ff) | (u32::from(value) << 8),
            ),
            0x11..=0x13 => {
                let shift = 8 * (offset - 0x10);
                let mask = !(0xffu32 << shift);
                self.write_reg(0x10, (self.inputctrl & mask) | (u32::from(value) << shift))
            }
            _ => self.write_reg(offset, u32::from(value)),
        }
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        match offset {
            0x04 => self.write_reg(offset, u32::from(value)),
            0x10 => {
                let merged = (self.inputctrl & 0xffff_0000) | u32::from(value);
                self.write_reg(offset, merged)
            }
            _ => self.write_reg(offset, u32::from(value)),
        }
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset, value)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let (base, shift) = Self::byte_register(offset).ok()?;
        let value = match base {
            0x00 => u32::from(self.ctrla),
            0x01 => u32::from(self.refctrl),
            0x02 => u32::from(self.avgctrl),
            0x03 => u32::from(self.sampctrl),
            0x04 => u32::from(self.ctrlb),
            0x08 => u32::from(self.winctrl),
            0x0c => 0,
            0x10 => self.inputctrl,
            0x14 => u32::from(self.evctrl),
            0x16 | 0x17 => u32::from(self.inten),
            0x18 => u32::from(self.intflag.get()),
            0x19 => 0,
            0x1a => u32::from(self.result),
            0x1c | 0x1e | 0x20 | 0x24 | 0x2c | 0x2e => 0,
            _ => return None,
        };
        Some((value >> (8 * shift)) as u8)
    }

    fn needs_bus_tick(&self) -> bool {
        self.pending
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if matches!(self.convert(bus), Ok(true)) {
            self.pending = false;
        }
    }

    fn adc_channel_count(&self) -> Option<u8> {
        Some(ADC_CHANNELS as u8)
    }

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

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.inten & self.intflag.get() & RESRDY != 0)
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "samd21_adc",
            "ctrla": self.ctrla,
            "refctrl": self.refctrl,
            "ctrlb": self.ctrlb,
            "inputctrl": self.inputctrl,
            "intflag": self.intflag.get(),
            "result": self.result,
            "conversions": self.conversions,
        })
    }
}
