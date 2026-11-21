// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Microchip SAM E70/S70/V70/V71 Analog Front-End Controller (AFEC).
//!
//! Sources: the register map, bit fields and the read-to-clear notes are
//! Microchip's `component/afec.h` (Zephyr hal_atmel @8f95c79f,
//! `asf/sam/include/same70b`); every reset value is 0 in cmsis-svd-data
//! `Atmel/ATSAME70Q21B.svd`; the firmware handshake (CHER + IER + CR.START,
//! then CSELR and the CDR read in the ISR) is Zephyr v3.7.0
//! `drivers/adc/adc_sam_afec.c`.
//!
//! Modelled: `AFEC_CR.START` converting every channel enabled in
//! `AFEC_CHSR` (ascending order, or the `AFEC_SEQ1R/2R` user sequence when
//! `MR.USEQ` is set), the `EMR.RES` 12/13/14/15/16-bit results against
//! VREFP, the per-channel `AFEC_CDR` selected by `AFEC_CSELR` with its
//! read-to-clear `ISR.EOCx`, `AFEC_LCDR` with its read-to-clear `ISR.DRDY`
//! and the `EMR.TAG` channel number in `CHNB`, `ISR.GOVRE` and
//! `AFEC_OVER.OVREx` overruns (both read-to-clear), the IER/IDR/IMR mask and
//! a level IRQ, `CR.SWRST`, the per-channel `AFEC_COCR` behind `CSELR`, and
//! the `WPMR` key ("ADC").
//! Conversions complete at the `START` write: the model has no conversion
//! clock, so `MR.PRESCAL/STARTUP/TRACKTIM/TRANSFER` are stored only.
//! Not modelled: hardware triggers (`MR.TRGEN`), free-run mode, the window
//! comparator, gain (`CGR`), analog offset (`COCR`, which Zephyr sets to
//! VREFP/2 for an ideal single-ended mid-point), differential and signed
//! conversions (`DIFFR`, `EMR.SIGNMODE`), dual sample-and-hold, the
//! temperature sensor and error correction, and the write protection itself.
//! Each reads back what was written.

use std::cell::Cell;

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// AD0..AD11 (`AFEC_ISR.EOC0..11`, `AFEC_CSELR.CSEL` 4 bits).
const CHANNELS: usize = 12;
/// VREFP default; boards override it with `config.vref_mv`.
const DEFAULT_VREF_MV: u32 = 3300;

// ── Register map (afec.h `afec_registers_t`) ────────────────────────────────
const CR: u64 = 0x00;
const MR: u64 = 0x04;
const EMR: u64 = 0x08;
const SEQ1R: u64 = 0x0C;
const SEQ2R: u64 = 0x10;
const CHER: u64 = 0x14;
const CHDR: u64 = 0x18;
const CHSR: u64 = 0x1C;
const LCDR: u64 = 0x20;
const IER: u64 = 0x24;
const IDR: u64 = 0x28;
const IMR: u64 = 0x2C;
const ISR: u64 = 0x30;
const OVER: u64 = 0x4C;
const CSELR: u64 = 0x64;
const CDR: u64 = 0x68;
const COCR: u64 = 0x6C;
const WPMR: u64 = 0xE4;
const WPSR: u64 = 0xE8;
const WINDOW: usize = 0x100;

const CR_SWRST: u32 = 1 << 0;
const CR_START: u32 = 1 << 1;
const MR_USEQ: u32 = 1 << 31;
const EMR_RES_SHIFT: u32 = 16;
const EMR_TAG: u32 = 1 << 24;
const LCDR_CHNB_SHIFT: u32 = 24;
const ISR_DRDY: u32 = 1 << 24;
const ISR_GOVRE: u32 = 1 << 25;
const ISR_COMPE: u32 = 1 << 26;
const ISR_TEMPCHG: u32 = 1 << 30;
const EOC_MASK: u32 = (1 << CHANNELS) - 1;
/// IER/IDR/IMR bits that exist: EOC0..11, DRDY, GOVRE, COMPE, TEMPCHG.
const INT_MASK: u32 = EOC_MASK | ISR_DRDY | ISR_GOVRE | ISR_COMPE | ISR_TEMPCHG;
const CSEL_MASK: u32 = 0xF;
const COCR_AOFF_MASK: u32 = 0x3FF;
/// `AFEC_WPMR_WPKEY_PASSWD` ("ADC"), bits 31:8.
const WPKEY: u32 = 0x41_4443;

/// SAM AFEC register file, sequencer and routed analog inputs.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SamAfec {
    regs: Vec<u32>,
    cdr: [u32; CHANNELS],
    cocr: [u32; CHANNELS],
    /// Routed input levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
    vref_mv: u32,
    /// ISR and OVER clear bits on read while `read` takes `&self`.
    isr: Cell<u32>,
    over: Cell<u32>,
}

impl Default for SamAfec {
    fn default() -> Self {
        Self::new(DEFAULT_VREF_MV)
    }
}

impl SamAfec {
    pub fn new(vref_mv: u32) -> Self {
        Self {
            regs: vec![0; WINDOW / 4],
            cdr: [0; CHANNELS],
            cocr: [0; CHANNELS],
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

    fn selected(&self) -> usize {
        (self.reg(CSELR) & CSEL_MASK) as usize
    }

    fn reset(&mut self) {
        self.regs.iter_mut().for_each(|r| *r = 0);
        self.cdr = [0; CHANNELS];
        self.cocr = [0; CHANNELS];
        self.isr.set(0);
        self.over.set(0);
    }

    /// Result width from `EMR.RES`: 0 = 12-bit, 2..5 = 13..16-bit with
    /// averaging (`AFEC_EMR_RES_OSR4..OSR256`); 1, 6 and 7 are not
    /// enumerated in afec.h and convert at 12 bits.
    fn resolution_bits(&self) -> u32 {
        match (self.reg(EMR) >> EMR_RES_SHIFT) & 0x7 {
            r @ 2..=5 => 11 + r,
            _ => 12,
        }
    }

    fn code(&self, channel: usize) -> u32 {
        let full_scale = (1u32 << self.resolution_bits()) - 1;
        let mv = self
            .inputs_mv
            .get(channel)
            .copied()
            .filter(|mv| *mv != u16::MAX)
            .map_or(0, u32::from);
        (u64::from(mv) * u64::from(full_scale) / u64::from(self.vref_mv)).min(u64::from(full_scale))
            as u32
    }

    /// The channels one `START` converts, in conversion order.
    fn sequence(&self) -> Vec<usize> {
        let enabled = self.reg(CHSR);
        let useq = self.reg(MR) & MR_USEQ != 0;
        (0..CHANNELS)
            .filter(|slot| enabled & (1 << slot) != 0)
            .filter_map(|slot| {
                if !useq {
                    return Some(slot);
                }
                // With USEQ, CHx enables sequence slot USCH(x+1), four bits
                // per slot: USCH1..8 in SEQ1R, USCH9..12 in SEQ2R.
                let word = if slot < 8 { SEQ1R } else { SEQ2R };
                let channel = ((self.reg(word) >> ((slot % 8) * 4)) & 0xF) as usize;
                (channel < CHANNELS).then_some(channel)
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
            self.cdr[channel] = code;
            let tag = if self.reg(EMR) & EMR_TAG != 0 {
                (channel as u32) << LCDR_CHNB_SHIFT
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
                self.isr.set(value & !(ISR_GOVRE | ISR_COMPE | ISR_TEMPCHG));
                value
            }
            OVER => self.over.replace(0),
            CDR => {
                let channel = self.selected();
                match self.cdr.get(channel) {
                    Some(code) => {
                        self.isr.set(self.isr.get() & !(1 << channel));
                        *code
                    }
                    None => 0,
                }
            }
            COCR => self.cocr.get(self.selected()).copied().unwrap_or(0),
            // WPKEY always reads as 0.
            WPMR => self.reg(WPMR) & 1,
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
            CHER => self.set_reg(CHSR, self.reg(CHSR) | (value & EOC_MASK)),
            CHDR => self.set_reg(CHSR, self.reg(CHSR) & !value),
            IER => self.set_reg(IMR, self.reg(IMR) | (value & INT_MASK)),
            IDR => self.set_reg(IMR, self.reg(IMR) & !value),
            CSELR => self.set_reg(CSELR, value & CSEL_MASK),
            COCR => {
                let channel = self.selected();
                if let Some(slot) = self.cocr.get_mut(channel) {
                    *slot = value & COCR_AOFF_MASK;
                }
            }
            // A wrong key aborts the WPEN write.
            WPMR => {
                if value >> 8 == WPKEY {
                    self.set_reg(WPMR, value & 1);
                }
            }
            // Read-only: status, results and the derived enable/mask views.
            CHSR | LCDR | IMR | ISR | OVER | CDR | WPSR => {}
            o => self.set_reg(o, value),
        }
    }

    fn irq_asserted(&self) -> bool {
        self.isr.get() & self.reg(IMR) != 0
    }
}

impl Peripheral for SamAfec {
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
            CR | CHER | CHDR | IER | IDR | WPMR => 0,
            COCR => self.cocr.get(self.selected()).copied().unwrap_or(0),
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

    fn afec_with(inputs: &[(u8, u16)]) -> SamAfec {
        let mut afec = SamAfec::default();
        for &(ch, mv) in inputs {
            assert!(afec.set_adc_channel_input(ch, mv));
        }
        afec
    }

    fn read_cdr(afec: &mut SamAfec, channel: u32) -> u32 {
        afec.write_u32(CSELR, channel).unwrap();
        afec.read_u32(CDR).unwrap()
    }

    #[test]
    fn zephyr_flow_converts_one_channel_and_the_cdr_read_clears_its_eoc() {
        let mut afec = afec_with(&[(0, 1650), (8, 3300)]);
        // adc_sam_init: SWRST, MR (ONE, STARTUP_SUT96), COCR = 512 per channel.
        afec.write_u32(CR, CR_SWRST).unwrap();
        afec.write_u32(MR, (1 << 23) | (5 << 16)).unwrap();
        for ch in 0..CHANNELS as u32 {
            afec.write_u32(CSELR, ch).unwrap();
            afec.write_u32(COCR, 512).unwrap();
        }
        afec.write_u32(CSELR, 3).unwrap();
        assert_eq!(afec.read_u32(COCR).unwrap(), 512, "COCR is per channel");
        // adc_sam_start_conversion for channel 0.
        afec.write_u32(CHDR, 0xFFF).unwrap();
        afec.write_u32(IDR, 0xFFF).unwrap();
        afec.write_u32(CHER, 1).unwrap();
        afec.write_u32(IER, 1).unwrap();
        afec.write_u32(CR, CR_START).unwrap();
        assert_eq!(afec.read_u32(ISR).unwrap() & EOC_MASK, 1);
        assert_eq!(afec.irq_line_level(), Some(true));
        // adc_sam_isr: CSELR = channel, read CDR.
        assert_eq!(read_cdr(&mut afec, 0), 1650 * 4095 / 3300);
        assert_eq!(afec.read_u32(ISR).unwrap() & EOC_MASK, 0);
        assert_eq!(afec.irq_line_level(), Some(false));
        // CH8 was not enabled: no EOC8 and no result.
        assert_eq!(read_cdr(&mut afec, 8), 0);
    }

    #[test]
    fn the_cdr_read_returns_the_csel_channel_only() {
        let mut afec = afec_with(&[(2, 1000), (9, 3000)]);
        afec.write_u32(CHER, (1 << 2) | (1 << 9)).unwrap();
        afec.write_u32(CR, CR_START).unwrap();
        assert_eq!(afec.read_u32(ISR).unwrap() & EOC_MASK, (1 << 2) | (1 << 9));
        assert_eq!(read_cdr(&mut afec, 9), 3000 * 4095 / 3300);
        assert_eq!(
            afec.read_u32(ISR).unwrap() & EOC_MASK,
            1 << 2,
            "reading CDR with CSEL = 9 clears only EOC9"
        );
        assert_eq!(read_cdr(&mut afec, 2), 1000 * 4095 / 3300);
        // LCDR holds the last converted channel (AD9), untagged by default.
        assert_eq!(afec.read_u32(LCDR).unwrap(), 3000 * 4095 / 3300);
        assert_eq!(afec.read_u32(ISR).unwrap() & ISR_DRDY, 0);
    }

    #[test]
    fn emr_res_selects_12_to_16_bit_results() {
        let mut afec = afec_with(&[(4, 1100)]);
        afec.write_u32(CHER, 1 << 4).unwrap();
        for (res, bits) in [
            (0u32, 12u32),
            (1, 12),
            (2, 13),
            (3, 14),
            (4, 15),
            (5, 16),
            (6, 12),
        ] {
            afec.write_u32(EMR, res << EMR_RES_SHIFT).unwrap();
            afec.write_u32(CR, CR_START).unwrap();
            let full = (1u32 << bits) - 1;
            assert_eq!(read_cdr(&mut afec, 4), 1100 * full / 3300, "RES = {res}");
        }
        // Over-range inputs clamp at full scale.
        afec.set_adc_channel_input(4, 3600);
        afec.write_u32(EMR, 5 << EMR_RES_SHIFT).unwrap();
        afec.write_u32(CR, CR_START).unwrap();
        assert_eq!(read_cdr(&mut afec, 4), 0xFFFF);
    }

    #[test]
    fn user_sequence_tag_and_overruns() {
        let mut afec = afec_with(&[(3, 3300), (7, 1000)]);
        // USCH1 = 7, USCH2 = 3; CH0/CH1 enable sequence slots 1 and 2.
        afec.write_u32(SEQ1R, 7 | (3 << 4)).unwrap();
        afec.write_u32(MR, MR_USEQ).unwrap();
        afec.write_u32(EMR, EMR_TAG).unwrap();
        afec.write_u32(CHER, 0b11).unwrap();
        afec.write_u32(CR, CR_START).unwrap();
        assert_eq!(
            afec.read_u32(LCDR).unwrap(),
            4095 | (3 << LCDR_CHNB_SHIFT),
            "AD3 converts last, tagged in CHNB"
        );
        assert_eq!(read_cdr(&mut afec, 7), 1000 * 4095 / 3300);
        // AD3's EOC is still set; a second START overruns it, and DRDY.
        afec.write_u32(CR, CR_START).unwrap();
        assert_eq!(afec.read_u32(OVER).unwrap(), 1 << 3);
        assert_eq!(afec.read_u32(OVER).unwrap(), 0, "OVER clears on read");
        assert_ne!(afec.read_u32(ISR).unwrap() & ISR_GOVRE, 0);
        assert_eq!(
            afec.read_u32(ISR).unwrap() & ISR_GOVRE,
            0,
            "GOVRE clears on ISR read"
        );
    }

    #[test]
    fn swrst_clears_state_and_the_wpmr_key_gates_wpen() {
        let mut afec = afec_with(&[(5, 500)]);
        afec.write_u32(CHER, 1 << 5).unwrap();
        afec.write_u32(IER, ISR_DRDY | (1 << 31)).unwrap();
        assert_eq!(
            afec.read_u32(IMR).unwrap(),
            ISR_DRDY,
            "bit 31 does not exist"
        );
        afec.write_u32(CR, CR_START).unwrap();
        assert!(afec.tick().irq);
        afec.read_u32(LCDR).unwrap();
        assert!(!afec.tick().irq, "the LCDR read drops DRDY");
        afec.write_u32(WPMR, 1).unwrap();
        assert_eq!(afec.read_u32(WPMR).unwrap(), 0, "wrong key aborts WPEN");
        afec.write_u32(WPMR, (WPKEY << 8) | 1).unwrap();
        assert_eq!(afec.read_u32(WPMR).unwrap(), 1, "WPKEY reads as 0");
        afec.write_u32(CR, CR_SWRST).unwrap();
        assert_eq!(afec.read_u32(CHSR).unwrap(), 0);
        assert_eq!(afec.read_u32(IMR).unwrap(), 0);
        assert_eq!(read_cdr(&mut afec, 5), 0);
        // Undriven inputs convert to zero.
        afec.write_u32(CHER, 1 << 6).unwrap();
        afec.write_u32(CR, CR_START).unwrap();
        assert_eq!(read_cdr(&mut afec, 6), 0);
        assert!(afec.clear_adc_channel_input(5));
        assert!(!afec.set_adc_channel_input(12, 1), "AD0..AD11 only");
    }
}
