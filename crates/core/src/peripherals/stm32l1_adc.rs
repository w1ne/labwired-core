// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32L1 ADC1 at `0x4001_2400` (IRQ 18), with its common registers
//! (`ADC_CSR`, `ADC_CCR`) at `+0x300` in the same 1 KB window.
//!
//! The L1 block is the F2/F4 ADC IP behind a different register map, so it is
//! its own model rather than another `AdcRegisterLayout` in `adc.rs`: that
//! file is on the drift watch list of every silicon-validated STM32 board.
//!
//! Sources:
//! - `STM32L151.svd` v1.4 (PlatformIO ststm32 19.7.1 `misc/svd`): ADC base
//!   and IRQ 18, every register offset (`SR` 0x00, `CR1` 0x04, `CR2` 0x08,
//!   `SMPR1..3` 0x0C..0x14, `JOFR1..4` 0x18..0x24, `HTR` 0x28, `LTR` 0x2C,
//!   `SQR1..5` 0x30..0x40, `JSQR` 0x44, `JDR1..4` 0x48..0x54, `DR` 0x58,
//!   `SMPR0` 0x5C, `CSR` 0x300, `CCR` 0x304), the field positions, and the
//!   reset values (all 0 except `HTR` 0xFFF).
//! - STM32CubeL1 HAL (STM32duino 2.12 `stm32l1xx_hal_adc.c/.h`):
//!   `ADC_IS_ENABLE` polls `SR.ADONS` after `CR2.ADON`; flags are cleared by
//!   writing their complement to `SR` (`__HAL_ADC_CLEAR_FLAG` writes
//!   `~flag`, so the status bits are write-0-to-clear); `HAL_ADC_Start` sets
//!   `CR2.SWSTART`; `PollForConversion` waits on `SR.EOC`; `CR2.EOCS` picks
//!   EOC per conversion or per sequence; `ADC_SQR5_RK` puts rank 1 in
//!   `SQR5[4:0]`; `ADC_SQR1_L_SHIFT` puts the length at `SQR1[24:20]`;
//!   `ADC_JSQR_RK_JL` puts injected rank 1 at slot `4 - JL`; `CR2.CFG`
//!   (`ChannelsBank`) selects bank B; `ADC_CHANNEL_0..3, 6..12` are "different
//!   in bank A and bank B" and the rest are common to both; channel 16 is the
//!   temperature sensor, 17 VREFINT (gated by `CCR.TSVREFE`).
//! - STM32duino variants `L151ZCT_L152ZCT_L162ZCT/PeripheralPins.c`
//!   (`PinMap_ADC`): the pad of each bank-A input `ADC_IN0..31` and each
//!   bank-B input `ADC_IN0b..12b`.
//!
//! Modelled:
//! - `ADON` powers the converter and `ADONS` follows it at once. `SWSTART`
//!   with `ADONS` converts the regular group and reads back 0; without
//!   `ADON` it converts nothing. `JSWSTART` likewise for the injected group.
//! - Regular group: `SQ1` alone without `CR1.SCAN`; `SQ1..SQ(L+1)` with it.
//!   With `EOCS` = 0 the whole sequence converts on the trigger, `DR` holds
//!   the last rank and `EOC` rises once. With `EOCS` = 1 each rank raises
//!   `EOC` and the next rank converts when `DR` is read, the order a reader
//!   that keeps up observes. Reading `DR` clears `EOC`.
//! - `CR1.DISCEN`: each trigger converts the next `DISCNUM` + 1 ranks of the
//!   sequence; the last subgroup may be shorter, then it restarts at `SQ1`.
//! - `CR2.CONT`: every tick restarts the finished sequence until `ADON` or
//!   `CONT` is cleared. With `EOCS` = 1 a result that lands on an unread
//!   `EOC` sets `OVR`.
//! - `CR1.RES` 12/10/8/6 bits, code = `mV * 4095 / VREF` truncated (the
//!   shared STM32 transfer function in `adc.rs`), shifted down for the
//!   lower resolutions; `CR2.ALIGN` left-aligns into `[15:16-N]`, except
//!   6 bits, which lands in `[7:2]`.
//! - Injected group: `JL` + 1 conversions from `JSQ(4 - JL)` to `JSQ4` into
//!   `JDR1..`, each the code minus its `JOFRx`, sign-extended; left-aligned
//!   it is shifted by 3 (the F1/F4 rule in `adc.rs`, same IP). `JEOC` rises
//!   at the end of the group.
//! - Analog watchdog: `AWDEN`/`JAWDEN`, `AWDSGL`/`AWDCH`; `AWD` rises when a
//!   guarded conversion lies outside `[LTR, HTR]`.
//! - `IRQ` 18 = `EOC & EOCIE | JEOC & JEOCIE | AWD & AWDIE | OVR & OVRIE`.
//! - Channels: input ids 0..31 are `ADC_IN0..31` (bank A, and the shared
//!   channels); 32 + n is `ADC_INnb` for the bank-B channels. With `CR2.CFG`
//!   a bank-B channel number converts its `ADC_INnb` pad. An undriven input,
//!   including the temperature sensor (16) and VREFINT (17), converts 0.
//! - `ADC_CSR` mirrors `SR[6:0]`; `ADC_CCR` stores `ADCPRE` and `TSVREFE`.
//!
//! Not modelled (stored only, with the SVD masks): external triggers
//! (`EXTEN`/`JEXTEN`), `JAUTO` and `JDISCEN`, DMA requests (`DMA`/`DDS`),
//! power-down and delay (`PDD`, `PDI`, `DELS`), sample times (`SMPR0..3`),
//! conversion time, `RCNR`/`JCNR` (always ready), and the temperature
//! sensor and VREFINT levels (they convert only what is routed to them).

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;

/// Input ids: `ADC_IN0..31`, then `ADC_IN0b..31b` at 32 + n.
const INPUTS: usize = 64;
const BANK_B: usize = 32;
/// `ADC_CHANNEL_n` that differ between bank A and bank B (HAL comments).
const BANKED: u32 = 0x1FCF; // 0..3, 6..12

const SR: u64 = 0x00;
const CR1: u64 = 0x04;
const CR2: u64 = 0x08;
const SMPR1: u64 = 0x0C;
const SMPR3: u64 = 0x14;
const JOFR1: u64 = 0x18;
const JOFR4: u64 = 0x24;
const HTR: u64 = 0x28;
const LTR: u64 = 0x2C;
const SQR1: u64 = 0x30;
const SQR5: u64 = 0x40;
const JSQR: u64 = 0x44;
const JDR1: u64 = 0x48;
const JDR4: u64 = 0x54;
const DR: u64 = 0x58;
const SMPR0: u64 = 0x5C;
const CSR: u64 = 0x300;
const CCR: u64 = 0x304;

pub const SR_AWD: u32 = 1 << 0;
pub const SR_EOC: u32 = 1 << 1;
pub const SR_JEOC: u32 = 1 << 2;
pub const SR_JSTRT: u32 = 1 << 3;
pub const SR_STRT: u32 = 1 << 4;
pub const SR_OVR: u32 = 1 << 5;
pub const SR_ADONS: u32 = 1 << 6;
/// The write-0-to-clear status bits.
const SR_FLAGS: u32 = 0x3F;

const CR1_EOCIE: u32 = 1 << 5;
const CR1_AWDIE: u32 = 1 << 6;
const CR1_JEOCIE: u32 = 1 << 7;
const CR1_SCAN: u32 = 1 << 8;
const CR1_AWDSGL: u32 = 1 << 9;
const CR1_DISCEN: u32 = 1 << 11;
const CR1_JAWDEN: u32 = 1 << 22;
const CR1_AWDEN: u32 = 1 << 23;
const CR1_OVRIE: u32 = 1 << 26;
const CR1_MASK: u32 = 0x07C3_FFFF;

pub const CR2_ADON: u32 = 1 << 0;
pub const CR2_CONT: u32 = 1 << 1;
pub const CR2_CFG: u32 = 1 << 2;
pub const CR2_EOCS: u32 = 1 << 10;
pub const CR2_ALIGN: u32 = 1 << 11;
pub const CR2_JSWSTART: u32 = 1 << 22;
pub const CR2_SWSTART: u32 = 1 << 30;
/// Every CR2 field except the two start commands, which read 0.
const CR2_MASK: u32 = 0x3F3F_0F77;

const SMPR_MASK: u32 = 0x3FFF_FFFF;
const SMPR0_MASK: u32 = 0x3F;
const SQR1_MASK: u32 = 0x01FF_FFFF;
const SQR_MASK: u32 = 0x3FFF_FFFF;
const JSQR_MASK: u32 = 0x003F_FFFF;
const CCR_MASK: u32 = 0x0083_0000;
/// Regular sequence length cap: `SQ1..SQ28`.
const MAX_RANKS: u8 = 28;

/// STM32L1 ADC1 plus its common registers.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Stm32l1Adc {
    /// `SR` status flags; a `Cell` because reading `DR` clears `EOC`.
    sr: Cell<u32>,
    cr1: u32,
    cr2: u32,
    /// `SMPR0..SMPR3`.
    smpr: [u32; 4],
    jofr: [u32; 4],
    htr: u32,
    ltr: u32,
    /// `SQR1..SQR5`.
    sqr: [u32; 5],
    jsqr: u32,
    jdr: [u32; 4],
    dr: Cell<u32>,
    ccr: u32,
    /// Next regular rank (0-based) still to convert in the current burst.
    next_rank: Cell<u8>,
    /// End (exclusive) of the current burst.
    burst_end: Cell<u8>,
    /// First rank of the next `DISCEN` subgroup.
    disc_rank: u8,
    vref_mv: u32,
    /// Routed levels in millivolts, `[bank A/common, bank B]` (32 each, the
    /// largest array serde derives). `u16::MAX` means undriven.
    inputs_mv: [[u16; BANK_B]; 2],
}

impl Default for Stm32l1Adc {
    fn default() -> Self {
        Self::new(3300)
    }
}

impl Stm32l1Adc {
    /// `vref_mv`: VREF+ (3.3 V by default, the STM32 convention in `adc.rs`).
    pub fn new(vref_mv: u32) -> Self {
        Self {
            sr: Cell::new(0),
            cr1: 0,
            cr2: 0,
            smpr: [0; 4],
            jofr: [0; 4],
            htr: 0xFFF,
            ltr: 0,
            sqr: [0; 5],
            jsqr: 0,
            jdr: [0; 4],
            dr: Cell::new(0),
            ccr: 0,
            next_rank: Cell::new(0),
            burst_end: Cell::new(0),
            disc_rank: 0,
            vref_mv: vref_mv.max(1),
            inputs_mv: [[u16::MAX; BANK_B]; 2],
        }
    }

    fn bits(&self) -> u32 {
        12 - 2 * ((self.cr1 >> 24) & 3)
    }

    /// The input id a channel number converts under the current bank.
    fn input(&self, channel: u32) -> usize {
        let channel = (channel & 0x1F) as usize;
        if self.cr2 & CR2_CFG != 0 && BANKED & (1 << channel) != 0 {
            BANK_B + channel
        } else {
            channel
        }
    }

    /// 12-bit code of a channel number.
    fn code12(&self, channel: u32) -> u32 {
        let input = self.input(channel);
        match self.inputs_mv[input / BANK_B][input % BANK_B] {
            u16::MAX => 0,
            mv => (u32::from(mv) * 4095 / self.vref_mv).min(4095),
        }
    }

    /// The analog watchdog verdict for one conversion of `channel`.
    fn watchdog(&self, channel: u32, code: u32, injected: bool) -> bool {
        let enabled = if injected {
            self.cr1 & CR1_JAWDEN != 0
        } else {
            self.cr1 & CR1_AWDEN != 0
        };
        let guarded = self.cr1 & CR1_AWDSGL == 0 || self.cr1 & 0x1F == channel & 0x1F;
        let code12 = code << (12 - self.bits());
        enabled && guarded && (code12 > self.htr || code12 < self.ltr)
    }

    fn rank_channel(&self, rank: u8) -> u32 {
        let reg = self.sqr[4 - usize::from(rank / 6)];
        (reg >> (5 * u32::from(rank % 6))) & 0x1F
    }

    fn sequence_len(&self) -> u8 {
        if self.cr1 & CR1_SCAN == 0 {
            1
        } else {
            (((self.sqr[0] >> 20) & 0x1F) as u8 + 1).min(MAX_RANKS)
        }
    }

    /// Convert one regular rank into `DR` and raise `EOC`.
    fn convert_rank(&self, rank: u8) {
        let channel = self.rank_channel(rank);
        let bits = self.bits();
        let code = self.code12(channel) >> (12 - bits);
        let data = if self.cr2 & CR2_ALIGN == 0 {
            code
        } else if bits == 6 {
            code << 2
        } else {
            code << (16 - bits)
        };
        let mut sr = self.sr.get();
        if sr & SR_EOC != 0 && self.cr2 & CR2_EOCS != 0 {
            sr |= SR_OVR;
        }
        if self.watchdog(channel, code, false) {
            sr |= SR_AWD;
        }
        self.dr.set(data);
        self.sr.set(sr | SR_EOC | SR_STRT);
    }

    /// One regular trigger: pick the burst, then convert all of it
    /// (`EOCS` = 0) or its first rank (`EOCS` = 1).
    fn start_regular(&mut self) {
        if self.cr2 & CR2_ADON == 0 {
            return;
        }
        let len = self.sequence_len();
        let (start, end) = if self.cr1 & CR1_DISCEN != 0 {
            let n = ((self.cr1 >> 13) & 7) as u8 + 1;
            let start = if self.disc_rank >= len {
                0
            } else {
                self.disc_rank
            };
            let end = (start + n).min(len);
            self.disc_rank = if end >= len { 0 } else { end };
            (start, end)
        } else {
            (0, len)
        };
        if self.cr2 & CR2_EOCS == 0 {
            for rank in start..end {
                // EOC rises once, at the end of the sequence.
                self.sr.set(self.sr.get() & !SR_EOC);
                self.convert_rank(rank);
            }
            self.next_rank.set(end);
        } else {
            self.convert_rank(start);
            self.next_rank.set(start + 1);
        }
        self.burst_end.set(end);
    }

    /// The `DR` read: clear `EOC`, then convert the next rank of an
    /// `EOCS` = 1 burst.
    fn after_dr_read(&self) {
        self.sr.set(self.sr.get() & !SR_EOC);
        let next = self.next_rank.get();
        if self.cr2 & CR2_EOCS != 0 && next < self.burst_end.get() && self.cr2 & CR2_ADON != 0 {
            self.convert_rank(next);
            self.next_rank.set(next + 1);
        }
    }

    fn start_injected(&mut self) {
        if self.cr2 & CR2_ADON == 0 {
            return;
        }
        let len = ((self.jsqr >> 20) & 3) as usize + 1;
        let mut sr = self.sr.get();
        for (k, slot) in (4 - len..4).enumerate() {
            let channel = (self.jsqr >> (5 * slot)) & 0x1F;
            let bits = self.bits();
            let code = self.code12(channel) >> (12 - bits);
            if self.watchdog(channel, code, true) {
                sr |= SR_AWD;
            }
            let signed = code as i32 - (self.jofr[k] & 0xFFF) as i32;
            let aligned = if self.cr2 & CR2_ALIGN != 0 {
                signed << 3
            } else {
                signed
            };
            self.jdr[k] = u32::from(aligned as i16 as u16);
        }
        self.sr.set(sr | SR_JEOC | SR_JSTRT);
    }

    fn status(&self) -> u32 {
        let adons = if self.cr2 & CR2_ADON != 0 {
            SR_ADONS
        } else {
            0
        };
        self.sr.get() | adons
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            SR => self.status(),
            CR1 => self.cr1,
            CR2 => self.cr2,
            SMPR1..=SMPR3 => self.smpr[((offset - SMPR1) / 4 + 1) as usize],
            JOFR1..=JOFR4 => self.jofr[((offset - JOFR1) / 4) as usize],
            HTR => self.htr,
            LTR => self.ltr,
            SQR1..=SQR5 => self.sqr[((offset - SQR1) / 4) as usize],
            JSQR => self.jsqr,
            JDR1..=JDR4 => self.jdr[((offset - JDR1) / 4) as usize],
            DR => self.dr.get(),
            SMPR0 => self.smpr[0],
            CSR => self.status() & 0x7F,
            CCR => self.ccr,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            SR => self.sr.set(self.sr.get() & value & SR_FLAGS),
            CR1 => self.cr1 = value & CR1_MASK,
            CR2 => {
                self.cr2 = value & CR2_MASK;
                if self.cr2 & CR2_ADON == 0 {
                    self.next_rank.set(0);
                    self.burst_end.set(0);
                }
                if value & CR2_JSWSTART != 0 {
                    self.start_injected();
                }
                if value & CR2_SWSTART != 0 {
                    self.start_regular();
                }
            }
            SMPR1..=SMPR3 => self.smpr[((offset - SMPR1) / 4 + 1) as usize] = value & SMPR_MASK,
            JOFR1..=JOFR4 => self.jofr[((offset - JOFR1) / 4) as usize] = value & 0xFFF,
            HTR => self.htr = value & 0xFFF,
            LTR => self.ltr = value & 0xFFF,
            SQR1 => self.sqr[0] = value & SQR1_MASK,
            o if (SQR1 + 4..=SQR5).contains(&o) => {
                self.sqr[((o - SQR1) / 4) as usize] = value & SQR_MASK
            }
            JSQR => self.jsqr = value & JSQR_MASK,
            SMPR0 => self.smpr[0] = value & SMPR0_MASK,
            CCR => self.ccr = value & CCR_MASK,
            _ => {}
        }
    }

    fn irq_asserted(&self) -> bool {
        let sr = self.sr.get();
        (sr & SR_EOC != 0 && self.cr1 & CR1_EOCIE != 0)
            || (sr & SR_JEOC != 0 && self.cr1 & CR1_JEOCIE != 0)
            || (sr & SR_AWD != 0 && self.cr1 & CR1_AWDIE != 0)
            || (sr & SR_OVR != 0 && self.cr1 & CR1_OVRIE != 0)
    }

    fn routable(channel: u8) -> bool {
        let channel = usize::from(channel);
        channel < BANK_B || (channel < INPUTS && BANKED & (1 << (channel - BANK_B)) != 0)
    }
}

impl Peripheral for Stm32l1Adc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(INPUTS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if !Self::routable(channel) {
            return false;
        }
        let input = usize::from(channel);
        self.inputs_mv[input / BANK_B][input % BANK_B] = millivolts;
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        if !Self::routable(channel) {
            return false;
        }
        let input = usize::from(channel);
        self.inputs_mv[input / BANK_B][input % BANK_B] = u16::MAX;
        true
    }

    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }

    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || volts <= 0.0 {
            return false;
        }
        self.vref_mv = ((volts * 1000.0).round() as u32).max(1);
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_asserted())
    }

    fn on_system_reset(&mut self) {
        let (inputs, vref) = (self.inputs_mv, self.vref_mv);
        *self = Self::new(vref);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let value = (self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8;
        if offset == DR {
            self.after_dr_read();
        }
        Ok(value)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        // A byte write to SR leaves the flags of the other bytes alone.
        let current = match aligned {
            SR => self.sr.get() | !(0xFF << shift),
            o => self.read_word(o),
        };
        let next = (current & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(aligned, next);
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let value = (self.read_word(offset & !3) >> ((offset & 2) * 8)) as u16;
        if offset == DR {
            self.after_dr_read();
        }
        Ok(value)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let value = self.read_word(offset);
        if offset == DR {
            self.after_dr_read();
        }
        Ok(value)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let idle = self.next_rank.get() >= self.burst_end.get();
        if self.cr2 & (CR2_ADON | CR2_CONT) == CR2_ADON | CR2_CONT
            && self.burst_end.get() > 0
            && idle
        {
            self.start_regular();
        }
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

    const SCAN: u32 = CR1_SCAN;

    fn adc_with(inputs: &[(u8, u16)]) -> Stm32l1Adc {
        let mut adc = Stm32l1Adc::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv), "input {ch}");
        }
        adc
    }

    fn code(mv: u32) -> u32 {
        (mv * 4095 / 3300).min(4095)
    }

    /// HAL `ADC_Enable` + `ConfigChannel` (rank 1) + `Start` +
    /// `PollForConversion` + `GetValue`.
    fn hal_read(adc: &mut Stm32l1Adc, channel: u32) -> Option<u32> {
        let cr2 = adc.read_u32(CR2).unwrap();
        adc.write_u32(CR2, cr2 | CR2_ADON).unwrap();
        assert_ne!(
            adc.read_u32(SR).unwrap() & SR_ADONS,
            0,
            "ADONS follows ADON"
        );
        adc.write_u32(SQR5, channel).unwrap();
        adc.write_u32(SR, !(SR_EOC | SR_OVR)).unwrap();
        let cr2 = adc.read_u32(CR2).unwrap();
        adc.write_u32(CR2, cr2 | CR2_SWSTART).unwrap();
        if adc.read_u32(SR).unwrap() & SR_EOC == 0 {
            return None;
        }
        adc.write_u32(SR, !(SR_STRT | SR_EOC)).unwrap();
        Some(adc.read_u32(DR).unwrap())
    }

    #[test]
    fn reset_values_match_the_svd() {
        let adc = Stm32l1Adc::default();
        for offset in (0..=0x5C).step_by(4) {
            let expect = if offset == HTR { 0xFFF } else { 0 };
            assert_eq!(adc.read_u32(offset).unwrap(), expect, "@{offset:#x}");
        }
        assert_eq!(adc.read_u32(CSR).unwrap(), 0);
        assert_eq!(adc.read_u32(CCR).unwrap(), 0);
    }

    #[test]
    fn hal_single_conversion_converts_each_pad_at_12_bits() {
        let mut adc = adc_with(&[(0, 0), (5, 1650), (21, 3299), (31, 5000), (17, 1224)]);
        assert_eq!(hal_read(&mut adc, 0), Some(0));
        assert_eq!(hal_read(&mut adc, 5), Some(code(1650)), "half scale");
        assert_eq!(hal_read(&mut adc, 21), Some(4093), "just below VREF");
        assert_eq!(hal_read(&mut adc, 31), Some(4095), "clamped above VREF");
        assert_eq!(hal_read(&mut adc, 17), Some(code(1224)), "routed VREFINT");
        assert_eq!(hal_read(&mut adc, 7), Some(0), "undriven pad");
        assert_eq!(
            adc.read_u32(CR2).unwrap() & CR2_SWSTART,
            0,
            "SWSTART reads 0"
        );
    }

    #[test]
    fn swstart_without_adon_converts_nothing() {
        let mut adc = adc_with(&[(3, 1000)]);
        adc.write_u32(SQR5, 3).unwrap();
        adc.write_u32(CR2, CR2_SWSTART | CR2_JSWSTART).unwrap();
        assert_eq!(adc.read_u32(SR).unwrap(), 0);
        assert_eq!(adc.read_u32(DR).unwrap(), 0);
    }

    #[test]
    fn dr_read_clears_eoc_and_status_is_write_zero_to_clear() {
        let mut adc = adc_with(&[(4, 1000)]);
        adc.write_u32(CR2, CR2_ADON).unwrap();
        adc.write_u32(SQR5, 4).unwrap();
        adc.write_u32(CR2, CR2_ADON | CR2_SWSTART).unwrap();
        assert_eq!(adc.read_u32(SR).unwrap(), SR_ADONS | SR_STRT | SR_EOC);
        assert_eq!(
            adc.read_u32(CSR).unwrap(),
            SR_ADONS | SR_STRT | SR_EOC,
            "CSR mirrors SR"
        );
        // Writing 1s leaves the flags; writing 0 to STRT clears it alone.
        adc.write_u32(SR, u32::MAX).unwrap();
        assert_eq!(adc.read_u32(SR).unwrap() & SR_FLAGS, SR_STRT | SR_EOC);
        adc.write_u32(SR, !SR_STRT).unwrap();
        assert_eq!(adc.read_u32(SR).unwrap() & SR_FLAGS, SR_EOC);
        assert_eq!(adc.read_u32(DR).unwrap(), code(1000));
        assert_eq!(adc.read_u32(SR).unwrap() & SR_EOC, 0, "DR read clears EOC");
    }

    #[test]
    fn resolution_and_alignment() {
        let mut adc = adc_with(&[(2, 2000)]);
        let c12 = code(2000);
        for (res, bits) in [(0, 12), (1, 10), (2, 8), (3, 6)] {
            adc.write_u32(CR1, res << 24).unwrap();
            adc.write_u32(CR2, CR2_ADON).unwrap();
            let right = hal_read(&mut adc, 2).unwrap();
            assert_eq!(right, c12 >> (12 - bits), "RES {res}");
            adc.write_u32(CR2, CR2_ADON | CR2_ALIGN).unwrap();
            adc.write_u32(SQR5, 2).unwrap();
            adc.write_u32(CR2, CR2_ADON | CR2_ALIGN | CR2_SWSTART)
                .unwrap();
            let left = adc.read_u32(DR).unwrap();
            let expect = if bits == 6 {
                right << 2
            } else {
                right << (16 - bits)
            };
            assert_eq!(left, expect, "RES {res} left-aligned");
        }
    }

    fn set_sequence(adc: &mut Stm32l1Adc, channels: &[u32]) {
        let mut sqr = [0u32; 5];
        for (rank, &ch) in channels.iter().enumerate() {
            sqr[4 - rank / 6] |= ch << (5 * (rank % 6));
        }
        sqr[0] |= (channels.len() as u32 - 1) << 20;
        for (k, v) in sqr.iter().enumerate() {
            adc.write_u32(SQR1 + 4 * k as u64, *v).unwrap();
        }
    }

    #[test]
    fn scan_with_eocs_steps_one_rank_per_dr_read() {
        let mut adc = adc_with(&[(1, 500), (9, 1500), (25, 2500)]);
        // SQ1..SQ8 crosses from SQR5 into SQR4.
        let seq = [1, 9, 25, 1, 9, 25, 9, 1];
        set_sequence(&mut adc, &seq);
        adc.write_u32(CR1, SCAN).unwrap();
        adc.write_u32(CR2, CR2_ADON | CR2_EOCS | CR2_SWSTART)
            .unwrap();
        let codes: Vec<u32> = seq
            .iter()
            .map(|_| {
                assert_ne!(adc.read_u32(SR).unwrap() & SR_EOC, 0);
                adc.read_u32(DR).unwrap()
            })
            .collect();
        let mv = |c: u32| match c {
            1 => 500,
            9 => 1500,
            _ => 2500,
        };
        let want: Vec<u32> = seq.iter().map(|&c| code(mv(c))).collect();
        assert_eq!(codes, want);
        assert_eq!(
            adc.read_u32(SR).unwrap() & (SR_EOC | SR_OVR),
            0,
            "sequence done"
        );
    }

    #[test]
    fn scan_with_eoc_per_sequence_leaves_the_last_rank_in_dr() {
        let mut adc = adc_with(&[(1, 500), (9, 1500)]);
        set_sequence(&mut adc, &[9, 1]);
        adc.write_u32(CR1, SCAN).unwrap();
        adc.write_u32(CR2, CR2_ADON | CR2_SWSTART).unwrap();
        assert_eq!(adc.read_u32(DR).unwrap(), code(500));
        assert_eq!(adc.read_u32(SR).unwrap() & SR_EOC, 0, "one EOC, cleared");
        // Without SCAN only SQ1 converts.
        adc.write_u32(CR1, 0).unwrap();
        adc.write_u32(CR2, CR2_ADON | CR2_SWSTART).unwrap();
        assert_eq!(adc.read_u32(DR).unwrap(), code(1500));
    }

    #[test]
    fn discontinuous_mode_converts_discnum_plus_one_ranks_per_trigger() {
        let mut adc = adc_with(&[(1, 500), (2, 1000), (3, 1500)]);
        set_sequence(&mut adc, &[1, 2, 3]);
        adc.write_u32(CR1, SCAN | CR1_DISCEN | (1 << 13)).unwrap();
        let mut got = Vec::new();
        for _ in 0..3 {
            adc.write_u32(CR2, CR2_ADON | CR2_EOCS | CR2_SWSTART)
                .unwrap();
            let mut burst = Vec::new();
            while adc.read_u32(SR).unwrap() & SR_EOC != 0 {
                burst.push(adc.read_u32(DR).unwrap());
            }
            got.push(burst);
        }
        let (a, b, c) = (code(500), code(1000), code(1500));
        assert_eq!(got, vec![vec![a, b], vec![c], vec![a, b]]);
    }

    #[test]
    fn continuous_mode_reconverts_and_flags_overrun() {
        let mut adc = adc_with(&[(6, 1000)]);
        adc.write_u32(SQR5, 6).unwrap();
        adc.write_u32(CR1, CR1_OVRIE).unwrap();
        adc.write_u32(CR2, CR2_ADON | CR2_CONT | CR2_EOCS | CR2_SWSTART)
            .unwrap();
        assert_eq!(adc.read_u32(DR).unwrap(), code(1000));
        adc.set_adc_channel_input(6, 2000);
        adc.tick();
        assert_eq!(
            adc.read_u32(DR).unwrap(),
            code(2000),
            "CONT follows the input"
        );
        adc.tick();
        assert!(!adc.irq_line_level().unwrap());
        adc.tick();
        assert_ne!(adc.read_u32(SR).unwrap() & SR_OVR, 0, "unread EOC overruns");
        assert!(adc.irq_line_level().unwrap(), "OVRIE");
        adc.write_u32(CR2, CR2_ADON).unwrap();
        adc.write_u32(SR, 0).unwrap();
        adc.tick();
        assert_eq!(
            adc.read_u32(SR).unwrap() & SR_FLAGS,
            0,
            "CONT cleared stops"
        );
    }

    #[test]
    fn bank_b_selects_the_b_pad_for_banked_channels_only() {
        // PB2 = ADC_IN0b (input 32), PA0 = ADC_IN0; IN13 is common.
        let mut adc = adc_with(&[(0, 600), (32, 2400), (13, 1200)]);
        assert!(
            !adc.set_adc_channel_input(32 + 4, 1),
            "IN4 has no bank-B pad"
        );
        assert!(!adc.set_adc_channel_input(32 + 13, 1), "IN13 is common");
        assert_eq!(hal_read(&mut adc, 0), Some(code(600)));
        adc.write_u32(CR2, CR2_ADON | CR2_CFG).unwrap();
        assert_eq!(hal_read(&mut adc, 0), Some(code(2400)), "bank B");
        adc.write_u32(CR2, CR2_ADON | CR2_CFG).unwrap();
        assert_eq!(hal_read(&mut adc, 13), Some(code(1200)), "common channel");
    }

    #[test]
    fn injected_group_converts_from_jsq_4_minus_jl_minus_offsets() {
        let mut adc = adc_with(&[(1, 500), (2, 1000)]);
        adc.write_u32(CR2, CR2_ADON).unwrap();
        adc.write_u32(JOFR1, 100).unwrap();
        adc.write_u32(JOFR1 + 4, 4000).unwrap();
        // JL = 1: JSQ3 = 1, JSQ4 = 2.
        adc.write_u32(JSQR, (1 << 20) | (1 << 10) | (2 << 15))
            .unwrap();
        adc.write_u32(CR1, CR1_JEOCIE).unwrap();
        adc.write_u32(CR2, CR2_ADON | CR2_JSWSTART).unwrap();
        assert_eq!(
            adc.read_u32(SR).unwrap() & (SR_JEOC | SR_JSTRT),
            SR_JEOC | SR_JSTRT
        );
        assert!(adc.irq_line_level().unwrap(), "JEOCIE");
        assert_eq!(adc.read_u32(JDR1).unwrap(), code(500) - 100);
        let neg = (code(1000) as i32 - 4000) as i16 as u16;
        assert_eq!(
            adc.read_u32(JDR1 + 4).unwrap(),
            u32::from(neg),
            "sign-extended"
        );
        assert_eq!(adc.read_u32(CR2).unwrap() & CR2_JSWSTART, 0);
        assert_eq!(
            adc.read_u32(SR).unwrap() & SR_EOC,
            0,
            "regular EOC untouched"
        );
    }

    #[test]
    fn analog_watchdog_guards_one_channel_or_all() {
        let mut adc = adc_with(&[(2, 1000), (3, 3000)]);
        adc.write_u32(HTR, code(2000)).unwrap();
        adc.write_u32(LTR, code(500)).unwrap();
        adc.write_u32(CR1, CR1_AWDEN | CR1_AWDIE | CR1_AWDSGL | 2)
            .unwrap();
        hal_read(&mut adc, 3);
        assert_eq!(adc.read_u32(SR).unwrap() & SR_AWD, 0, "IN3 not guarded");
        hal_read(&mut adc, 2);
        assert_eq!(
            adc.read_u32(SR).unwrap() & SR_AWD,
            0,
            "IN2 inside the window"
        );
        adc.write_u32(CR1, CR1_AWDEN | CR1_AWDIE).unwrap();
        hal_read(&mut adc, 3);
        assert_ne!(
            adc.read_u32(SR).unwrap() & SR_AWD,
            0,
            "all channels guarded"
        );
        assert!(adc.irq_line_level().unwrap());
    }

    #[test]
    fn routed_reference_rescales_the_code_and_reset_keeps_inputs() {
        let mut adc = adc_with(&[(5, 1500)]);
        assert!(adc.set_adc_reference_voltage_v(3.0));
        assert_eq!(hal_read(&mut adc, 5), Some(1500 * 4095 / 3000));
        adc.on_system_reset();
        assert_eq!(adc.read_u32(CR2).unwrap(), 0);
        assert_eq!(hal_read(&mut adc, 5), Some(1500 * 4095 / 3000));
    }
}
