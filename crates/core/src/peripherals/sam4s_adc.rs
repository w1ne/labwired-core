// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Microchip SAM4S 12-bit ADC (SAM4S datasheet DS11100, §42 "Analog-to-digital
//! Converter").
//!
//! Modelled: `ADC_CR.START` converting every channel enabled in `ADC_CHSR`
//! (ascending channel order, or the `ADC_SEQR1/2` user sequence when
//! `ADC_MR.USEQ` is set), 12-bit or `MR.LOWRES` 10-bit results against the
//! ADVREF reference, `ADC_CDRx` with its read-to-clear `ISR.EOCx`, `ADC_LCDR`
//! with its read-to-clear `ISR.DRDY` and the `EMR.TAG` channel number,
//! `ISR.GOVRE` and `ADC_OVER.OVREx` overruns (both read-to-clear), the
//! IER/IDR/IMR interrupt mask and a level IRQ, and `CR.SWRST`.
//! Conversions complete at the `START` write: the model has no conversion
//! clock, so `MR.PRESCAL/STARTUP/TRACKTIM` are stored only.
//! Not modelled: hardware triggers (`MR.TRGEN`), free-run mode, the window
//! comparator (`EMR/CWR`), gain/offset (`CGR/COR`), the PDC, the temperature
//! sensor channel's physics and the write protection. Each reads back what was
//! written.

use std::cell::Cell;

use crate::{Peripheral, PeripheralTickResult, SimResult};

const CHANNELS: usize = 16;
/// The SAM4S Xplained Pro and most SAM4S boards tie ADVREF to VDDIO = 3.3 V.
const DEFAULT_VREF_MV: u32 = 3300;

// ── Register map (SAM4S datasheet §42.7) ────────────────────────────────────
const CR: u64 = 0x00;
const MR: u64 = 0x04;
const SEQR1: u64 = 0x08;
const SEQR2: u64 = 0x0C;
const CHER: u64 = 0x10;
const CHDR: u64 = 0x14;
const CHSR: u64 = 0x18;
const LCDR: u64 = 0x20;
const IER: u64 = 0x24;
const IDR: u64 = 0x28;
const IMR: u64 = 0x2C;
const ISR: u64 = 0x30;
const OVER: u64 = 0x3C;
const EMR: u64 = 0x40;
const CDR0: u64 = 0x50;
const CDR_END: u64 = CDR0 + 4 * CHANNELS as u64;
const WINDOW: usize = 0x100;

const CR_SWRST: u32 = 1 << 0;
const CR_START: u32 = 1 << 1;
const MR_LOWRES: u32 = 1 << 4;
const MR_USEQ: u32 = 1 << 31;
const EMR_TAG: u32 = 1 << 24;
const ISR_DRDY: u32 = 1 << 24;
const ISR_GOVRE: u32 = 1 << 25;
const ISR_COMPE: u32 = 1 << 26;
/// IER/IDR/IMR bits that exist on this ADC: EOC0..15, DRDY, GOVRE, COMPE,
/// ENDRX, RXBUFF.
const INT_MASK: u32 = 0xFFFF | (0x1F << 24);

/// SAM4S ADC register file, sequencer and routed analog inputs.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Sam4sAdc {
    regs: Vec<u32>,
    /// Routed input levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
    vref_mv: u32,
    /// ISR and OVER clear bits on read while `read` takes `&self`.
    isr: Cell<u32>,
    over: Cell<u32>,
}

impl Default for Sam4sAdc {
    fn default() -> Self {
        Self::new(DEFAULT_VREF_MV)
    }
}

impl Sam4sAdc {
    pub fn new(vref_mv: u32) -> Self {
        Self {
            regs: vec![0; WINDOW / 4],
            inputs_mv: [u16::MAX; CHANNELS],
            vref_mv: vref_mv.max(1),
            isr: Cell::new(0),
            over: Cell::new(0),
        }
    }

    fn reg(&self, offset: u64) -> u32 {
        self.regs.get((offset / 4) as usize).copied().unwrap_or(0)
    }

    fn set_reg(&mut self, offset: u64, value: u32) {
        if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
            *slot = value;
        }
    }

    fn reset(&mut self) {
        self.regs.iter_mut().for_each(|r| *r = 0);
        self.isr.set(0);
        self.over.set(0);
    }

    fn code(&self, channel: usize) -> u32 {
        let full_scale = if self.reg(MR) & MR_LOWRES != 0 {
            0x3FF
        } else {
            0xFFF
        };
        let mv = self
            .inputs_mv
            .get(channel)
            .copied()
            .filter(|mv| *mv != u16::MAX)
            .map_or(0, u32::from);
        (mv * full_scale / self.vref_mv).min(full_scale)
    }

    /// The channels one `START` converts, in conversion order.
    fn sequence(&self) -> Vec<usize> {
        let enabled = self.reg(CHSR);
        let useq = self.reg(MR) & MR_USEQ != 0;
        (0..CHANNELS)
            .filter(|slot| enabled & (1 << slot) != 0)
            .map(|slot| {
                if !useq {
                    return slot;
                }
                // With USEQ, CHx enables sequence slot USCH(x+1), four bits
                // per slot: USCH1..8 in SEQR1, USCH9..16 in SEQR2.
                let word = if slot < 8 { SEQR1 } else { SEQR2 };
                ((self.reg(word) >> ((slot % 8) * 4)) & 0xF) as usize
            })
            .collect()
    }

    fn start(&mut self) {
        for channel in self.sequence() {
            let code = self.code(channel);
            let eoc = 1u32 << channel;
            let mut isr = self.isr.get();
            if isr & eoc != 0 {
                self.over.set(self.over.get() | eoc);
            }
            if isr & ISR_DRDY != 0 {
                isr |= ISR_GOVRE;
            }
            self.isr.set(isr | eoc | ISR_DRDY);
            self.set_reg(CDR0 + 4 * channel as u64, code);
            let tag = if self.reg(EMR) & EMR_TAG != 0 {
                (channel as u32) << 12
            } else {
                0
            };
            self.set_reg(LCDR, code | tag);
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset & !3 {
            CR | CHER | CHDR | IER | IDR => 0,
            LCDR => {
                self.isr.set(self.isr.get() & !ISR_DRDY);
                self.reg(LCDR)
            }
            ISR => {
                let value = self.isr.get();
                self.isr.set(value & !(ISR_GOVRE | ISR_COMPE));
                value
            }
            OVER => self.over.replace(0),
            o @ CDR0..CDR_END => {
                let channel = (o - CDR0) / 4;
                self.isr.set(self.isr.get() & !(1 << channel));
                self.reg(o)
            }
            o => self.reg(o),
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset & !3 {
            CR => {
                if value & CR_SWRST != 0 {
                    self.reset();
                } else if value & CR_START != 0 {
                    self.start();
                }
            }
            CHER => self.set_reg(CHSR, self.reg(CHSR) | (value & 0xFFFF)),
            CHDR => self.set_reg(CHSR, self.reg(CHSR) & !value),
            IER => self.set_reg(IMR, self.reg(IMR) | (value & INT_MASK)),
            IDR => self.set_reg(IMR, self.reg(IMR) & !value),
            // Read-only: status, results and the derived enable/mask views.
            CHSR | LCDR | IMR | ISR | OVER | CDR0..CDR_END => {}
            o => self.set_reg(o, value),
        }
    }

    fn irq_asserted(&self) -> bool {
        self.isr.get() & self.reg(IMR) != 0
    }
}

impl Peripheral for Sam4sAdc {
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

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_asserted())
    }

    fn on_system_reset(&mut self) {
        self.reset();
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        // Byte writes merge into the stored word without the read side
        // effects a firmware read would have.
        let current = match aligned {
            CR | CHER | CHDR | IER | IDR => 0,
            o => self.reg(o),
        };
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

    fn adc_with(inputs: &[(u8, u16)]) -> Sam4sAdc {
        let mut adc = Sam4sAdc::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv));
        }
        adc
    }

    #[test]
    fn start_converts_enabled_channels_in_order_and_eoc_clears_on_cdr_read() {
        let mut adc = adc_with(&[(0, 1650), (4, 3300), (9, 825)]);
        adc.write_u32(CHER, (1 << 0) | (1 << 4) | (1 << 9)).unwrap();
        adc.write_u32(CR, CR_START).unwrap();
        assert_eq!(
            adc.read_u32(ISR).unwrap() & 0xFFFF,
            (1 << 0) | (1 << 4) | (1 << 9)
        );
        assert_eq!(adc.read_u32(CDR0).unwrap(), 1650 * 4095 / 3300);
        assert_eq!(adc.read_u32(CDR0 + 16).unwrap(), 4095);
        assert_eq!(
            adc.read_u32(ISR).unwrap() & 0xFFFF,
            1 << 9,
            "CDR reads clear EOC0/EOC4"
        );
        // LCDR holds the last converted channel (CH9), with no tag by default.
        assert_eq!(adc.read_u32(LCDR).unwrap(), 825 * 4095 / 3300);
        assert_eq!(
            adc.read_u32(ISR).unwrap() & ISR_DRDY,
            0,
            "LCDR read clears DRDY"
        );
    }

    #[test]
    fn user_sequence_lowres_tag_and_overruns() {
        let mut adc = adc_with(&[(3, 3300), (7, 1000)]);
        // USCH1 = 7, USCH2 = 3; CH0/CH1 enable sequence slots 1 and 2.
        adc.write_u32(SEQR1, 7 | (3 << 4)).unwrap();
        adc.write_u32(MR, MR_USEQ | MR_LOWRES).unwrap();
        adc.write_u32(EMR, EMR_TAG).unwrap();
        adc.write_u32(CHER, 0b11).unwrap();
        adc.write_u32(CR, CR_START).unwrap();
        assert_eq!(
            adc.read_u32(LCDR).unwrap(),
            1023 | (3 << 12),
            "CH3 converts last, tagged"
        );
        assert_eq!(adc.read_u32(CDR0 + 28).unwrap(), 1000 * 1023 / 3300);
        // CH3's EOC is still set; a second START overruns it and DRDY.
        adc.write_u32(CR, CR_START).unwrap();
        assert_eq!(adc.read_u32(OVER).unwrap(), 1 << 3);
        assert_eq!(adc.read_u32(OVER).unwrap(), 0, "OVER clears on read");
        assert_ne!(adc.read_u32(ISR).unwrap() & ISR_GOVRE, 0);
        assert_eq!(
            adc.read_u32(ISR).unwrap() & ISR_GOVRE,
            0,
            "GOVRE clears on ISR read"
        );
    }

    #[test]
    fn interrupt_mask_drives_the_irq_level_and_swrst_clears_everything() {
        let mut adc = adc_with(&[(5, 500)]);
        adc.write_u32(CHER, 1 << 5).unwrap();
        adc.write_u32(IER, ISR_DRDY).unwrap();
        assert_eq!(adc.read_u32(IMR).unwrap(), ISR_DRDY);
        assert_eq!(adc.irq_line_level(), Some(false));
        adc.write_u32(CR, CR_START).unwrap();
        assert_eq!(adc.irq_line_level(), Some(true));
        assert!(adc.tick().irq);
        adc.read_u32(LCDR).unwrap();
        assert_eq!(adc.irq_line_level(), Some(false));
        adc.write_u32(CR, CR_SWRST).unwrap();
        assert_eq!(adc.read_u32(CHSR).unwrap(), 0);
        assert_eq!(adc.read_u32(IMR).unwrap(), 0);
        // Undriven inputs convert to zero.
        adc.write_u32(CHER, 1 << 6).unwrap();
        adc.write_u32(CR, CR_START).unwrap();
        assert_eq!(adc.read_u32(CDR0 + 24).unwrap(), 0);
    }
}
