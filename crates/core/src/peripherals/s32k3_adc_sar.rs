// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP S32K3 SAR ADC (`ADC_0..ADC_2` at `0x400A_0000`/`0x400A_4000`/
//! `0x400A_8000` on the S32K388, IRQs 180..182).
//!
//! Register map and bit fields come from the S32K388 device header of the
//! NXP RTD BaseNXP set (`S32K388_ADC.h`, "S32K3xx RM Rev.6 Draft A", as
//! vendored in github Jonah-Zhang1/S32K358 @fc142f34
//! `MCAL/BaseNXP_TS_T40D34M30I0R0/header/`; its `ADC_Type` equals the
//! S32K358 one in Zephyr hal_nxp @cc9b44ae) and `S32K388_COMMON.h`
//! (`ADC0_IRQn` 180). The firmware handshake is the RTD `Adc_Sar_Ip.c`
//! (hal_nxp @cc9b44ae `s32/drivers/s32k3/Adc`): `Adc_Sar_Ip_Powerdown` /
//! `Powerup` poll `MSR.ADCSTATUS` for 1 (power-down) / 0 (idle);
//! `Adc_Sar_Ip_DoCalibration` sets `CALBISTREG.TEST_EN`, polls `C_T_BUSY`
//! and checks `TEST_FAIL` (write 1 to clear); `Adc_Sar_Ip_StartConversion`
//! sets `MCR.NSTART`/`JSTART`; `Adc_Sar_Ip_GetConvData` reads `CDR[n]`
//! (word `0x100 + 4n`), checks `VALID` and write-1-clears `CEOCFR[n/32]`
//! bit `n%32`. Reset values were cross-checked against the QEMU S32K3 ADC
//! of github KJ-Sagar/qemu_s32k3 @8709eca1 (`hw/adc/s32k3_adc.c`, which
//! cites RM Rev.11 §60.6.2.1), and so was `CDR.VALID` clearing on a read
//! (§60.6.2.34 "automatically reset to 0 when the data is read").
//!
//! Channel numbers are the RTD's: precision inputs `ADCx_P0..P7` are 0..7
//! (`PCDR`), standard inputs `ADCx_S8..S31` are 32..55 (`ICDR`), external
//! (analog-mux) inputs `ADCx_X` are 64..95 (`ECDR`). Only the P and S pads
//! are routable; X inputs sit behind an external mux and convert 0.
//!
//! Modelled:
//! - `MCR.PWDN` drives `MSR.ADCSTATUS` (1 power-down, 0 idle). A start
//!   while powered down converts nothing.
//! - Normal chain: `MCR.NSTART` converts every channel enabled in
//!   `NCMR0..2` in ascending order. Each conversion fills its `CDR`
//!   (`RESULT` 00, `VALID`), sets its `CEOCFR` bit and `MSR.CHADDR`; the
//!   chain end sets `ISR.EOC` + `ISR.ECH`. One-shot (`MCR.MODE` 0) clears
//!   `NSTART` at the chain end; scan (`MODE` 1) keeps it set, mirrored in
//!   `MSR.NSTART`, and re-runs the chain once per tick.
//! - Injected chain: `MCR.JSTART` converts the `JCMR0..2` set with
//!   `RESULT` 01, then sets `ISR.JEOC` + `ISR.JECH` and self-clears.
//! - Data: `CALBISTREG.RESN` 0..3 = 14/12/10/8 bits. The N-bit code
//!   `V * 2^N / VREF` (truncated, clamped) sits MSB-first in the 15-bit
//!   right-aligned field (`code << (15 - N)`), or in all 16 bits with
//!   `MCR.WLSIDE` (`code << (16 - N)`), which is how
//!   `Adc_Sar_GetMaskedResult` unpacks it.
//! - `CDR.VALID` clears on a read. A new result over a still-valid one
//!   sets `OVERW` and replaces it only with `MCR.OWREN`; otherwise the old
//!   data stays.
//! - `ISR`, `CEOCFR0..2`, `WTISR` and `AWORR0..2` are write-1-to-clear.
//!   The IRQ line is `ISR & IMR` or any `CEOCFR & CIMR`.
//! - Calibration: `CALBISTREG.TEST_EN` completes at once, reads back clear,
//!   leaves `C_T_BUSY` and `TEST_FAIL` clear and sets `MSR.CALIBRTD`.
//!
//! Not modelled: BCTU and hardware triggers (`TRGEN`/`JTRGEN`/`BCTUEN` are
//! stored only), conversion time (`CTR`, `PSCR`, `PDEDR`, `DSDR` and the
//! clock select are stored only), averaging (a held sample averages to
//! itself), the analog watchdog comparisons (thresholds and selects are
//! stored, `WTISR`/`AWORR` never set), DMA requests, self-test, the
//! user offset/gain, and `ABORT`/`ABORTCHAIN`, which read 0 because a
//! chain never stays in flight between two bus accesses.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;

/// Channel ids 0..95: `CDR[n]` at `0x100 + 4n`.
pub const CHANNELS: usize = 96;
const DEFAULT_VREF_MV: u32 = 3300;

const MCR: u64 = 0x00;
const MSR: u64 = 0x04;
const ISR: u64 = 0x10;
const CEOCFR0: u64 = 0x14;
const IMR: u64 = 0x20;
const CIMR0: u64 = 0x24;
const WTISR: u64 = 0x30;
const NCMR0: u64 = 0xA4;
const JCMR0: u64 = 0xB4;
const CDR0: u64 = 0x100;
const CDR_END: u64 = CDR0 + 4 * (CHANNELS as u64 - 1);
const AWORR0: u64 = 0x2F0;
const AWORR2: u64 = 0x2F8;
const CALBISTREG: u64 = 0x3A0;
/// Last register (`CAL2`) and the word count of the backing store.
const LAST: u64 = 0x3B4;
const WORDS: usize = (LAST / 4) as usize + 1;

pub const MCR_PWDN: u32 = 1 << 0;
const MCR_ABORT: u32 = 1 << 6;
const MCR_ABORTCHAIN: u32 = 1 << 7;
pub const MCR_JSTART: u32 = 1 << 20;
pub const MCR_NSTART: u32 = 1 << 24;
pub const MCR_MODE: u32 = 1 << 29;
pub const MCR_WLSIDE: u32 = 1 << 30;
pub const MCR_OWREN: u32 = 1 << 31;
/// Every `MCR` field of S32K388_ADC.h; `ABORT`/`ABORTCHAIN` read 0.
const MCR_FIELDS: u32 = 0xEF73_8EE7 & !(MCR_ABORT | MCR_ABORTCHAIN);

const MSR_ADCSTATUS_POWER_DOWN: u32 = 1;
const MSR_CHADDR_SHIFT: u32 = 9;
const MSR_NSTART: u32 = 1 << 24;
pub const MSR_CALIBRTD: u32 = 1 << 31;

pub const ISR_ECH: u32 = 1 << 0;
pub const ISR_EOC: u32 = 1 << 1;
pub const ISR_JECH: u32 = 1 << 2;
pub const ISR_JEOC: u32 = 1 << 3;
const ISR_FIELDS: u32 = 0x1F;

pub const CDR_VALID: u32 = 1 << 19;
pub const CDR_OVERW: u32 = 1 << 18;
const CDR_RESULT_SHIFT: u32 = 16;

pub const CAL_TEST_EN: u32 = 1 << 0;
pub const CAL_TEST_FAIL: u32 = 1 << 3;
pub const CAL_C_T_BUSY: u32 = 1 << 15;
const CAL_RESN_SHIFT: u32 = 29;
/// `TEST_EN`, `AVG_EN`, `NR_SMPL`, `TSAMP`, `RESN` (S32K388_ADC.h).
const CAL_STORED: u32 = 0xF800_0071;

/// Valid channel bits per 32-channel group: P0..P7, S8..S31, X0..X31.
const GROUP_MASK: [u32; 3] = [0xFF, 0x00FF_FFFF, 0xFFFF_FFFF];

/// Reset values (QEMU s32k3_adc.c, RM §60.6.2.1): `CTR0..2` 0x16,
/// `THRHLR0..3` 0x7FFF_0000, self-test/AMSIO/OFSGNUSR/CAL2 as listed.
const RESET: &[(u64, u32)] = &[
    (0x60, 0x7FFF_0000),
    (0x64, 0x7FFF_0000),
    (0x68, 0x7FFF_0000),
    (0x6C, 0x7FFF_0000),
    (0x94, 0x16),
    (0x98, 0x16),
    (0x9C, 0x16),
    (0x340, 0x1818_2507),
    (0x344, 0x5),
    (0x348, 0x300),
    (0x34C, 0x0005_0000),
    (0x380, 0x0727_04C5),
    (0x388, 0x3FF9),
    (0x38C, 0x3FF9),
    (0x394, 0x0010_3FF0),
    (0x398, 0x0010_3FF0),
    (0x39C, 0x811),
    (0x3A8, 0x0004_0000),
    (0x3B4, 0x4300_8243),
];

/// Registers stored as plain read/write words.
fn plain(offset: u64) -> bool {
    matches!(
        offset,
        IMR | 0x24..=0x2C       // CIMR0..2
            | 0x34              // WTIMR
            | 0x40..=0x4C       // DMAE, DMAR0..2
            | 0x60..=0x6C       // THRHLR0..3
            | 0x80..=0x8C       // PSCR, PSR0..2
            | 0x94..=0x9C       // CTR0..2
            | 0xA4..=0xAC       // NCMR0..2
            | 0xB4..=0xBC       // JCMR0..2
            | 0xC4 | 0xC8       // DSDR, PDEDR
            | 0x2B0..=0x2B4     // CWSELRPI0..1
            | 0x2C0..=0x2C8     // CWSELRSI0..2
            | 0x2D0..=0x2DC     // CWSELREI0..3
            | 0x2E0..=0x2E8     // CWENR0..2
            | 0x340..=0x350     // STCR1..3, STBRR, STSR1
            | 0x380 | 0x388 | 0x38C | 0x394 | 0x398 | 0x39C
            | 0x3A8 | 0x3B4
    )
}

/// One `CDR`: the aligned `CDATA`, the chain type and the flags.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct Cdr {
    data: u16,
    result: u8,
    overw: bool,
    valid: Cell<bool>,
}

/// S32K3 SAR ADC.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct S32k3AdcSar {
    regs: Vec<u32>,
    mcr: u32,
    calibrated: bool,
    isr: u32,
    ceocfr: [u32; 3],
    chaddr: u32,
    cdr: Vec<Cdr>,
    /// Routed pad levels in millivolts; `u16::MAX` means undriven.
    inputs_mv: Vec<u16>,
    vref_mv: u32,
}

impl Default for S32k3AdcSar {
    fn default() -> Self {
        Self::new(DEFAULT_VREF_MV)
    }
}

/// P0..P7 (0..7) and S8..S31 (32..55) reach a pad.
fn routable(channel: usize) -> bool {
    channel < 8 || (32..56).contains(&channel)
}

impl S32k3AdcSar {
    pub fn new(vref_mv: u32) -> Self {
        let mut regs = vec![0; WORDS];
        for &(offset, value) in RESET {
            regs[(offset / 4) as usize] = value;
        }
        Self {
            regs,
            mcr: MCR_PWDN,
            calibrated: true,
            isr: 0,
            ceocfr: [0; 3],
            chaddr: 0,
            cdr: vec![Cdr::default(); CHANNELS],
            inputs_mv: vec![u16::MAX; CHANNELS],
            vref_mv: vref_mv.max(1),
        }
    }

    fn reg(&self, offset: u64) -> u32 {
        self.regs[(offset / 4) as usize]
    }

    fn powered(&self) -> bool {
        self.mcr & MCR_PWDN == 0
    }

    /// `CALBISTREG.RESN` 0..3 = 14/12/10/8 bits (RTD `ResolutionArray`).
    fn bits(&self) -> u32 {
        [14, 12, 10, 8][((self.reg(CALBISTREG) >> CAL_RESN_SHIFT) & 3) as usize]
    }

    fn sample(&self, channel: usize) -> u16 {
        let bits = self.bits();
        let max = (1i64 << bits) - 1;
        let code = match self.inputs_mv[channel] {
            u16::MAX => 0,
            mv => (i64::from(mv) << bits) / i64::from(self.vref_mv),
        }
        .clamp(0, max) as u32;
        let shift = if self.mcr & MCR_WLSIDE != 0 { 16 } else { 15 } - bits;
        (code << shift) as u16
    }

    fn convert(&mut self, channel: usize, result: u8) {
        let data = self.sample(channel);
        let cdr = &mut self.cdr[channel];
        if cdr.valid.get() {
            cdr.overw = true;
            if self.mcr & MCR_OWREN == 0 {
                // OWREN = 0: the unread result is kept.
                self.ceocfr[channel / 32] |= 1 << (channel % 32);
                self.chaddr = channel as u32;
                return;
            }
        } else {
            cdr.overw = false;
        }
        cdr.data = data;
        cdr.result = result;
        cdr.valid.set(true);
        self.ceocfr[channel / 32] |= 1 << (channel % 32);
        self.chaddr = channel as u32;
    }

    /// Convert the channels enabled in the three `xCMR` words at `cmr0`.
    fn run_chain(&mut self, cmr0: u64, result: u8) {
        for (group, mask) in GROUP_MASK.iter().enumerate() {
            let set = self.reg(cmr0 + 4 * group as u64) & mask;
            for bit in 0..32 {
                if set & (1 << bit) != 0 {
                    self.convert(group * 32 + bit, result);
                }
            }
        }
    }

    fn run_normal(&mut self) {
        self.run_chain(NCMR0, 0);
        self.isr |= ISR_EOC | ISR_ECH;
        if self.mcr & MCR_MODE == 0 {
            self.mcr &= !MCR_NSTART;
        }
    }

    fn write_mcr(&mut self, value: u32) {
        let was_running = self.mcr & MCR_NSTART != 0;
        self.mcr = value & MCR_FIELDS;
        if !self.powered() {
            // Nothing starts while powered down.
            self.mcr &= !(MCR_NSTART | MCR_JSTART);
            return;
        }
        if self.mcr & MCR_JSTART != 0 {
            self.run_chain(JCMR0, 1);
            self.isr |= ISR_JEOC | ISR_JECH;
            self.mcr &= !MCR_JSTART;
        }
        if self.mcr & MCR_NSTART != 0 && !(was_running && self.mcr & MCR_MODE != 0) {
            self.run_normal();
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            MCR => self.mcr,
            MSR => {
                let status = if self.powered() {
                    0
                } else {
                    MSR_ADCSTATUS_POWER_DOWN
                };
                let running = self.mcr & (MCR_NSTART | MCR_MODE) == MCR_NSTART | MCR_MODE;
                status
                    | (self.chaddr << MSR_CHADDR_SHIFT)
                    | if running { MSR_NSTART } else { 0 }
                    | if self.calibrated { MSR_CALIBRTD } else { 0 }
            }
            ISR => self.isr,
            o @ CEOCFR0..=0x1C => self.ceocfr[((o - CEOCFR0) / 4) as usize],
            o @ CDR0..=CDR_END => {
                let cdr = &self.cdr[((o - CDR0) / 4) as usize];
                u32::from(cdr.data)
                    | u32::from(cdr.result) << CDR_RESULT_SHIFT
                    | if cdr.overw { CDR_OVERW } else { 0 }
                    | if cdr.valid.get() { CDR_VALID } else { 0 }
            }
            WTISR | AWORR0..=AWORR2 | CALBISTREG => self.reg(offset),
            o if plain(o) => self.reg(o),
            _ => 0,
        }
    }

    /// Reading `CDR[n]` clears its `VALID`.
    fn after_read(&self, offset: u64) {
        if let o @ CDR0..=CDR_END = offset & !3 {
            self.cdr[((o - CDR0) / 4) as usize].valid.set(false);
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            MCR => self.write_mcr(value),
            ISR => self.isr &= !(value & ISR_FIELDS),
            o @ CEOCFR0..=0x1C => {
                let group = ((o - CEOCFR0) / 4) as usize;
                self.ceocfr[group] &= !value;
            }
            WTISR | AWORR0..=AWORR2 => self.regs[(offset / 4) as usize] &= !value,
            CALBISTREG => {
                let stored = value & CAL_STORED & !CAL_TEST_EN;
                self.regs[(CALBISTREG / 4) as usize] = stored;
                if value & CAL_TEST_EN != 0 && self.powered() {
                    // An ideal converter calibrates at once and passes.
                    self.calibrated = true;
                }
            }
            0x354..=0x370 => {} // STSR2..4, STDR1 are read-only
            o if plain(o) => self.regs[(o / 4) as usize] = value,
            _ => {}
        }
    }

    fn irq_asserted(&self) -> bool {
        let cimr = |g: u64| self.reg(CIMR0 + 4 * g);
        self.isr & self.reg(IMR) != 0 || (0..3).any(|g| self.ceocfr[g as usize] & cimr(g) != 0)
    }
}

impl Peripheral for S32k3AdcSar {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if !routable(channel as usize) {
            return false;
        }
        self.inputs_mv[channel as usize] = millivolts;
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        if !routable(channel as usize) {
            return false;
        }
        self.inputs_mv[channel as usize] = u16::MAX;
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_asserted())
    }

    fn on_system_reset(&mut self) {
        let inputs = std::mem::take(&mut self.inputs_mv);
        *self = Self::new(self.vref_mv);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let value = (self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8;
        self.after_read(offset);
        Ok(value)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        // A byte lane of a write-1-to-clear register clears only its bits.
        let w1c = matches!(aligned, ISR | CEOCFR0..=0x1C | WTISR | AWORR0..=AWORR2);
        let current = if w1c { 0 } else { self.read_word(aligned) };
        let next = (current & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(aligned, next);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let value = self.read_word(offset & !3);
        self.after_read(offset);
        Ok(value)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset & !3, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.powered() && self.mcr & (MCR_NSTART | MCR_MODE) == MCR_NSTART | MCR_MODE {
            self.run_normal();
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

    /// `CALBISTREG.RESN` for 12 bits (RTD `ADC_SAR_IP_RESOLUTION_12`).
    const RESN_12: u32 = 1 << CAL_RESN_SHIFT;
    const P5: u8 = 5;
    const S10: u8 = 34;

    fn powered_adc(inputs: &[(u8, u16)]) -> S32k3AdcSar {
        let mut adc = S32k3AdcSar::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv), "channel {ch}");
        }
        adc.write_u32(MCR, 0).unwrap();
        adc.write_u32(CALBISTREG, RESN_12).unwrap();
        adc
    }

    fn enable(adc: &mut S32k3AdcSar, channels: &[u8]) {
        let mut ncmr = [0u32; 3];
        for &ch in channels {
            ncmr[usize::from(ch) / 32] |= 1 << (ch % 32);
        }
        for (g, word) in ncmr.iter().enumerate() {
            adc.write_u32(NCMR0 + 4 * g as u64, *word).unwrap();
        }
    }

    /// `Adc_Sar_GetMaskedResult` for a right-aligned 12-bit result.
    fn code12(cdr: u32) -> u32 {
        (cdr & 0x7FF8) >> 3
    }

    #[test]
    fn reset_powers_down_and_matches_the_reference_values() {
        let adc = S32k3AdcSar::default();
        assert_eq!(adc.read_u32(MCR).unwrap(), MCR_PWDN);
        assert_eq!(adc.read_u32(MSR).unwrap(), MSR_CALIBRTD | 1);
        assert_eq!(adc.read_u32(0x94).unwrap(), 0x16, "CTR0");
        assert_eq!(adc.read_u32(0x60).unwrap(), 0x7FFF_0000, "THRHLR0");
        assert_eq!(adc.read_u32(0x3B4).unwrap(), 0x4300_8243, "CAL2");
        assert_eq!(adc.read_u32(0x08).unwrap(), 0, "reserved");
    }

    #[test]
    fn powered_down_start_converts_nothing() {
        let mut adc = S32k3AdcSar::default();
        adc.set_adc_channel_input(P5, 1650);
        enable(&mut adc, &[P5]);
        adc.write_u32(MCR, MCR_PWDN | MCR_NSTART).unwrap();
        assert_eq!(adc.read_u32(ISR).unwrap(), 0);
        assert_eq!(adc.read_u32(CDR0 + 4 * 5).unwrap() & CDR_VALID, 0);
        // Powerup: ADCSTATUS goes idle.
        adc.write_u32(MCR, 0).unwrap();
        assert_eq!(adc.read_u32(MSR).unwrap() & 7, 0);
    }

    #[test]
    fn one_shot_chain_fills_each_cdr_and_flags_the_chain_end() {
        let mut adc = powered_adc(&[(P5, 1650), (S10, 825)]);
        enable(&mut adc, &[P5, S10, 7]);
        adc.write_u32(MCR, MCR_NSTART).unwrap();
        assert_eq!(adc.read_u32(MCR).unwrap() & MCR_NSTART, 0, "one-shot ends");
        assert_eq!(adc.read_u32(ISR).unwrap(), ISR_EOC | ISR_ECH);
        assert_eq!(adc.read_u32(CEOCFR0).unwrap(), (1 << 5) | (1 << 7));
        assert_eq!(adc.read_u32(CEOCFR0 + 4).unwrap(), 1 << 2, "S10 = bit 2");
        assert_eq!((adc.read_u32(MSR).unwrap() >> 9) & 0x7F, 34, "CHADDR");
        let p5 = adc.read_u32(CDR0 + 4 * 5).unwrap();
        assert_ne!(p5 & CDR_VALID, 0);
        assert_eq!((p5 >> 16) & 3, 0, "normal chain");
        assert_eq!(code12(p5), 2048);
        assert_eq!(
            adc.read_u32(CDR0 + 4 * 5).unwrap() & CDR_VALID,
            0,
            "the read cleared VALID"
        );
        assert_eq!(
            code12(adc.read_u32(CDR0 + 4 * u64::from(S10)).unwrap()),
            1024
        );
        assert_eq!(
            code12(adc.read_u32(CDR0 + 4 * 7).unwrap()),
            0,
            "unrouted P7"
        );
        // w1c
        adc.write_u32(ISR, ISR_ECH).unwrap();
        assert_eq!(adc.read_u32(ISR).unwrap(), ISR_EOC);
        adc.write_u32(CEOCFR0, 1 << 5).unwrap();
        assert_eq!(adc.read_u32(CEOCFR0).unwrap(), 1 << 7);
    }

    #[test]
    fn resolution_and_alignment_follow_the_rtd_unpacking() {
        let mut adc = powered_adc(&[(P5, 1650)]);
        enable(&mut adc, &[P5]);
        for (resn, bits) in [(0u32, 14u32), (1, 12), (2, 10), (3, 8)] {
            adc.write_u32(CALBISTREG, resn << CAL_RESN_SHIFT).unwrap();
            adc.write_u32(MCR, MCR_NSTART).unwrap();
            let cdr = adc.read_u32(CDR0 + 4 * 5).unwrap();
            let right = (cdr & 0x7FFF & (0xFFFF << (15 - bits))) >> (15 - bits);
            assert_eq!(right, 1 << (bits - 1), "{bits}-bit right-aligned");
            adc.write_u32(MCR, MCR_WLSIDE | MCR_NSTART).unwrap();
            let cdr = adc.read_u32(CDR0 + 4 * 5).unwrap() & 0xFFFF;
            assert_eq!(cdr, 0x8000, "{bits}-bit left-aligned");
        }
        // Clamped at full scale.
        adc.set_adc_channel_input(P5, 4000);
        adc.write_u32(CALBISTREG, RESN_12).unwrap();
        adc.write_u32(MCR, MCR_NSTART).unwrap();
        assert_eq!(code12(adc.read_u32(CDR0 + 4 * 5).unwrap()), 4095);
    }

    #[test]
    fn unread_result_sets_overw_and_owren_decides_the_data() {
        let mut adc = powered_adc(&[(P5, 1650)]);
        enable(&mut adc, &[P5]);
        adc.write_u32(MCR, MCR_NSTART).unwrap();
        adc.set_adc_channel_input(P5, 825);
        adc.write_u32(MCR, MCR_NSTART).unwrap();
        let cdr = adc.read_u32(CDR0 + 4 * 5).unwrap();
        assert_ne!(cdr & CDR_OVERW, 0);
        assert_eq!(code12(cdr), 2048, "OWREN = 0 keeps the unread data");
        adc.write_u32(MCR, MCR_OWREN | MCR_NSTART).unwrap();
        adc.write_u32(MCR, MCR_OWREN | MCR_NSTART).unwrap();
        let cdr = adc.read_u32(CDR0 + 4 * 5).unwrap();
        assert_ne!(cdr & CDR_OVERW, 0);
        assert_eq!(code12(cdr), 1024, "OWREN = 1 replaces it");
    }

    #[test]
    fn injected_chain_tags_its_results_and_self_clears() {
        let mut adc = powered_adc(&[(S10, 1650)]);
        adc.write_u32(JCMR0 + 4, 1 << 2).unwrap();
        adc.write_u32(MCR, MCR_JSTART).unwrap();
        assert_eq!(adc.read_u32(MCR).unwrap() & MCR_JSTART, 0);
        assert_eq!(adc.read_u32(ISR).unwrap(), ISR_JEOC | ISR_JECH);
        let cdr = adc.read_u32(CDR0 + 4 * u64::from(S10)).unwrap();
        assert_eq!((cdr >> 16) & 3, 1, "injected chain");
        assert_eq!(code12(cdr), 2048);
    }

    #[test]
    fn scan_mode_reconverts_each_tick_until_nstart_clears() {
        let mut adc = powered_adc(&[(P5, 1650)]);
        enable(&mut adc, &[P5]);
        adc.write_u32(MCR, MCR_MODE | MCR_NSTART).unwrap();
        assert_ne!(adc.read_u32(MSR).unwrap() & MSR_NSTART, 0, "scan runs");
        assert_eq!(code12(adc.read_u32(CDR0 + 4 * 5).unwrap()), 2048);
        adc.set_adc_channel_input(P5, 825);
        adc.tick();
        assert_eq!(code12(adc.read_u32(CDR0 + 4 * 5).unwrap()), 1024);
        adc.write_u32(MCR, MCR_MODE).unwrap();
        assert_eq!(adc.read_u32(MSR).unwrap() & MSR_NSTART, 0);
        adc.set_adc_channel_input(P5, 0);
        adc.tick();
        assert_eq!(
            adc.read_u32(CDR0 + 4 * 5).unwrap() & CDR_VALID,
            0,
            "stopped"
        );
    }

    #[test]
    fn calibration_handshake_and_irq_masks() {
        let mut adc = powered_adc(&[(P5, 1650)]);
        adc.write_u32(CALBISTREG, RESN_12 | CAL_TEST_EN | (3 << 5) | (1 << 4))
            .unwrap();
        let cal = adc.read_u32(CALBISTREG).unwrap();
        assert_eq!(cal & (CAL_TEST_EN | CAL_C_T_BUSY | CAL_TEST_FAIL), 0);
        assert_eq!(cal, RESN_12 | (3 << 5) | (1 << 4), "fields stored");
        assert_ne!(adc.read_u32(MSR).unwrap() & MSR_CALIBRTD, 0);

        enable(&mut adc, &[P5]);
        adc.write_u32(MCR, MCR_NSTART).unwrap();
        assert_eq!(adc.irq_line_level(), Some(false), "all masked");
        adc.write_u32(CIMR0, 1 << 5).unwrap();
        assert_eq!(adc.irq_line_level(), Some(true), "CIMR0 bit 5");
        adc.write_u32(CEOCFR0, 1 << 5).unwrap();
        assert_eq!(adc.irq_line_level(), Some(false));
        adc.write_u32(IMR, ISR_ECH).unwrap();
        assert!(adc.tick().irq, "ISR.ECH & IMR.MSKECH");
        adc.write(ISR, ISR_ECH as u8).unwrap();
        assert_eq!(adc.irq_line_level(), Some(false));
    }

    #[test]
    fn only_p_and_s_pads_route() {
        let mut adc = S32k3AdcSar::default();
        for ch in [0u8, 7, 32, 55] {
            assert!(adc.set_adc_channel_input(ch, 1), "{ch}");
        }
        for ch in [8u8, 31, 56, 64, 95, 96] {
            assert!(!adc.set_adc_channel_input(ch, 1), "{ch}");
        }
    }

    #[test]
    fn reset_keeps_routed_inputs() {
        let mut adc = powered_adc(&[(P5, 1650)]);
        adc.on_system_reset();
        assert_eq!(adc.read_u32(MCR).unwrap(), MCR_PWDN);
        adc.write_u32(MCR, 0).unwrap();
        adc.write_u32(CALBISTREG, RESN_12).unwrap();
        enable(&mut adc, &[P5]);
        adc.write_u32(MCR, MCR_NSTART).unwrap();
        assert_eq!(code12(adc.read_u32(CDR0 + 4 * 5).unwrap()), 2048);
    }
}
