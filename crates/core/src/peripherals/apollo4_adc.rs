// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Apollo4 Plus general-purpose ADC.
//!
//! The current model slice covers software-triggered single scans of enabled
//! external-input slots, conversion into the 16-entry result FIFO, and the
//! routed millivolt input seam used by analog co-simulation. Register offsets
//! and slot fields follow the Apollo4 Plus CMSIS definitions. Source details
//! are listed in `examples/apollo4p-evb/REQUIRED_DOCS.md`; the wider
//! DMA, comparator, timer-trigger, and low-power behavior remains outside this
//! model.

use std::cell::Cell;

use crate::{Peripheral, PeripheralTickResult, SimResult};

const WINDOW: usize = 0x1000;
const FIFO_DEPTH: usize = 16;
const INPUT_CHANNELS: usize = 8;
const VREF_MV: u32 = 1190;

const CFG: u64 = 0x00;
const SWT: u64 = 0x08;
const SLOT0CFG: u64 = 0x0C;
const FIFO: u64 = 0x38;
const FIFOPR: u64 = 0x3C;
const INTSTAT: u64 = 0x204;
const INTCLR: u64 = 0x208;
const INTSET: u64 = 0x20C;

const CFG_ADCEN: u32 = 1 << 0;
const CFG_DFIFORDEN: u32 = 1 << 12;
const SWT_SOFTWARE_TRIGGER: u8 = 0x37;
const SLOT_ENABLE: u32 = 1 << 0;
const SLOT_CHANNEL_SHIFT: u32 = 8;
const SLOT_CHANNEL_MASK: u32 = 0xF << SLOT_CHANNEL_SHIFT;
const SLOT_PRECISION_SHIFT: u32 = 16;
const SLOT_PRECISION_MASK: u32 = 0x3 << SLOT_PRECISION_SHIFT;
const INT_CONVERSION_COMPLETE: u32 = 1 << 0;
const INT_SCAN_COMPLETE: u32 = 1 << 1;
const FIFO_DATA_MASK: u32 = 0xF_FFFF;

/// Apollo4 Plus general-purpose SAR ADC input and FIFO model.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Apollo4Adc {
    regs: Vec<u32>,
    /// Routed input levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; INPUT_CHANNELS],
    /// Encoded FIFO entries without the live COUNT field.
    fifo: Vec<u32>,
    /// Reads of FIFOPR can be destructive while the register file itself is
    /// read through `&self`; this cursor models that hardware side effect.
    #[serde(skip)]
    fifo_head: Cell<usize>,
}

impl Default for Apollo4Adc {
    fn default() -> Self {
        Self::new()
    }
}

impl Apollo4Adc {
    pub fn new() -> Self {
        Self {
            regs: vec![0; WINDOW / 4],
            inputs_mv: [u16::MAX; INPUT_CHANNELS],
            fifo: Vec::new(),
            fifo_head: Cell::new(0),
        }
    }

    /// The held routed input level for an external ADC channel.
    pub fn channel_input_mv(&self, channel: u8) -> Option<u16> {
        self.inputs_mv
            .get(channel as usize)
            .copied()
            .filter(|mv| *mv != u16::MAX)
    }

    fn reg(&self, offset: u64) -> u32 {
        self.regs.get((offset / 4) as usize).copied().unwrap_or(0)
    }

    fn set_reg(&mut self, offset: u64, value: u32) {
        if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
            *slot = value;
        }
    }

    fn fifo_count(&self) -> usize {
        self.fifo.len().saturating_sub(self.fifo_head.get())
    }

    fn fifo_word(&self) -> u32 {
        let head = self.fifo_head.get();
        let Some(entry) = self.fifo.get(head).copied() else {
            return 0;
        };
        let count = self.fifo_count().min(0xFF) as u32;
        (entry & FIFO_DATA_MASK) | (count << 20) | (entry & (0x7 << 28))
    }

    fn pop_fifo(&self) {
        let head = self.fifo_head.get();
        if head < self.fifo.len() {
            self.fifo_head.set(head + 1);
        }
    }

    fn quantized_sample(&self, channel: u32, precision: u32) -> u32 {
        let bits = match precision {
            2 => 10,
            3 => 8,
            _ => 12,
        };
        let full_scale = (1u32 << bits) - 1;
        let millivolts = self
            .inputs_mv
            .get(channel as usize)
            .copied()
            .filter(|mv| *mv != u16::MAX)
            .map_or(0, u32::from);
        let code = (millivolts * full_scale / VREF_MV).min(full_scale);
        // ADC FIFO DATA carries six fractional bits below the sample integer.
        code << 6
    }

    fn run_single_scan(&mut self) {
        self.fifo.clear();
        self.fifo_head.set(0);
        if self.reg(CFG) & CFG_ADCEN == 0 {
            return;
        }

        for slot in 0..8u32 {
            let config = self.reg(SLOT0CFG + u64::from(slot) * 4);
            if config & SLOT_ENABLE == 0 {
                continue;
            }
            if self.fifo.len() == FIFO_DEPTH {
                break;
            }
            let channel = (config & SLOT_CHANNEL_MASK) >> SLOT_CHANNEL_SHIFT;
            let precision = (config & SLOT_PRECISION_MASK) >> SLOT_PRECISION_SHIFT;
            let sample = self.quantized_sample(channel, precision) & FIFO_DATA_MASK;
            self.fifo.push(sample | (slot << 28));
            self.set_reg(INTSTAT, self.reg(INTSTAT) | INT_CONVERSION_COMPLETE);
        }

        if !self.fifo.is_empty() {
            self.set_reg(INTSTAT, self.reg(INTSTAT) | INT_SCAN_COMPLETE);
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset & !3 {
            FIFO => self.fifo_word(),
            FIFOPR => {
                let value = self.fifo_word();
                if self.reg(CFG) & CFG_DFIFORDEN != 0 {
                    self.pop_fifo();
                }
                value
            }
            _ => self.reg(offset),
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset & !3 {
            SWT => {
                self.set_reg(SWT, u32::from(value as u8));
                if value as u8 == SWT_SOFTWARE_TRIGGER {
                    self.run_single_scan();
                }
            }
            FIFO => self.pop_fifo(),
            INTCLR => self.set_reg(INTSTAT, self.reg(INTSTAT) & !value),
            INTSET => self.set_reg(INTSTAT, self.reg(INTSTAT) | value),
            INTSTAT => {}
            _ => self.set_reg(offset, value),
        }
    }
}

impl Peripheral for Apollo4Adc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(INPUT_CHANNELS as u8)
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

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let value = self.read_word(offset);
        Ok((value >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let current = self.read_word(aligned);
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
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}
