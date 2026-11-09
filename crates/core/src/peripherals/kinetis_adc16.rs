// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP Kinetis ADC16, the 16-bit SAR ADC on the K6x/K2x/KLx parts
//! (`ADC0` at `0x4003_B000`, `ADC1` at `0x400B_B000` on the MK64F12).
//!
//! Register map and bit fields come from the mcux-sdk CMSIS header
//! `devices/MK64F12/MK64F12.h` (`ADC_Type`: `SC1[2]` @0x00, `CFG1` @0x08,
//! `CFG2` @0x0C, `R[2]` @0x10, `CV1/CV2` @0x18/0x1C, `SC2` @0x20, `SC3`
//! @0x24, `OFS` @0x28, `PG/MG` @0x2C/0x30, `CLPD..CLP0` @0x34..0x4C, `CLMD..
//! CLM0` @0x54..0x6C) and the reset values from cmsis-svd-data
//! `NXP/MK64F12.svd` (`SC1n` 0x1F, `OFS` 0x4, `PG/MG` 0x8200, `CLPD` 0xA,
//! `CLPS` 0x20, `CLP4..CLP0` 0x200/0x100/0x80/0x40/0x20, same for CLMx).
//! Behaviour comes from the K66 Sub-Family Reference Manual Rev. 2 ch. 39
//! (the same ADC16 IP; §39.4.1 SC1n, §39.4.4 Rn, §39.5.4 conversion control,
//! §39.5.5 compare, §39.5.6 calibration, §39.5.7 offset), and the firmware
//! handshake from mcux-sdk `drivers/adc16/fsl_adc16.c`
//! (`ADC16_DoAutoCalibration`, `ADC16_SetChannelConfig`).
//!
//! Modelled:
//! - Software trigger (`SC2.ADTRG` = 0): a write to `SC1A` with `ADCH` other
//!   than 0x1F converts. Writes to `SC1B` never start one (§39.5.4.1).
//! - `CFG1.MODE` resolutions: single-ended 8/12/10/16 bits right-justified,
//!   differential (`SC1n.DIFF`) 9/13/11/16 bits in sign-extended 2's
//!   complement (§39.4.4). Single-ended code `V * 2^N / VREF`, differential
//!   `(VDADP - VDADM) * 2^(N-1) / VREF`, truncated and clamped.
//! - `CFG2.MUXSEL` picks the `ADxxa` or `ADxxb` pad for `ADCH` 4..7.
//! - `SC2.REFSEL`: 00 = VREFH (3.3 V by default), 01 = VALTH, the 1.2 V
//!   `VREF_OUT` on K6x (RM §39.1.7). Both are configurable.
//! - `ADCH` 29 converts VREFSH (full scale; `-VREFSH` when differential) and
//!   30 converts VREFSL (zero).
//! - `OFS` (2's complement, left-justified) is subtracted with the bit
//!   alignment of §39.5.7, then the result is clamped to the mode's range.
//! - The compare function (`SC2.ACFE/ACFGT/ACREN`, `CV1/CV2`) in all six
//!   modes of Table 39-12: when it is false, `COCO` stays clear and `Rn`
//!   keeps its last value.
//! - `COCO` clears on a write to its `SC1n` or a read of its `Rn`; with
//!   `AIEN` it drives the IRQ line.
//! - `SC3.ADCO` continuous conversion: started by the `SC1A` write, it
//!   re-samples once per tick until `SC1A` is rewritten or any other ADC
//!   register is written (§39.5.4.3); `SC2.ADACT` reads 1 meanwhile.
//! - Calibration: `SC3.CAL` completes at once and sets `SC1A.COCO`. With
//!   `ADTRG` = 1 it does not start and `CALF` sets. `CALF` is write 1 to
//!   clear. The converter is ideal, so calibration leaves `OFS` = 0 and the
//!   `CLPx/CLMx` registers at their reset values.
//!
//! Not modelled: hardware triggers (PDB/`ADHWT`, so `ADTRG` = 1 converts
//! nothing), the temperature sensor and bandgap channels (26/27 convert 0),
//! the `PG/MG` gain (stored only), DMA requests (`DMAEN` stored only),
//! conversion time (clock and sample-time fields are stored only, and the
//! hardware average of a held sample equals that sample), and SIM clock
//! gating.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;

/// Routable pad inputs, indexed as:
/// - 0..=3: `DADP0..3` (single-ended `ADCH` 0..3, plus side of `DAD0..3`)
/// - 4..=23: `AD4..AD23` (4..7 are the `ADxxa` pads)
/// - 24..=27: `AD4b..AD7b`
/// - 28..=31: `DADM0..3`, the minus side of `DAD0..3`
pub const INPUTS: usize = 32;
const B_PADS: usize = 24;
const DADM: usize = 28;

const DEFAULT_VREFH_MV: u32 = 3300;
const DEFAULT_VALT_MV: u32 = 1200;

// ── Register map (MK64F12.h ADC_Type) ────────────────────────────────────────
const SC1A: u64 = 0x00;
const SC1B: u64 = 0x04;
const CFG1: u64 = 0x08;
const CFG2: u64 = 0x0C;
const RA: u64 = 0x10;
const RB: u64 = 0x14;
const CV1: u64 = 0x18;
const CV2: u64 = 0x1C;
const SC2: u64 = 0x20;
const SC3: u64 = 0x24;
const OFS: u64 = 0x28;
const PG: u64 = 0x2C;
const MG: u64 = 0x30;
/// `CLPD..CLP0` @0x34..0x4C, a gap at 0x50, `CLMD..CLM0` @0x54..0x6C.
const CLPD: u64 = 0x34;
const CLMD: u64 = 0x54;
const CAL_REGS: usize = 7;
/// SVD reset values of CLxD, CLxS, CLx4, CLx3, CLx2, CLx1, CLx0.
const CAL_RESET: [u16; CAL_REGS] = [0x0A, 0x20, 0x200, 0x100, 0x80, 0x40, 0x20];
/// Field widths of the same registers (`ADC_CLPx_CLPx_MASK`).
const CAL_MASK: [u16; CAL_REGS] = [0x3F, 0x3F, 0x3FF, 0x1FF, 0xFF, 0x7F, 0x3F];

const SC1_ADCH: u32 = 0x1F;
const SC1_DIFF: u32 = 1 << 5;
const SC1_AIEN: u32 = 1 << 6;
const SC1_COCO: u32 = 1 << 7;
const SC1_STORED: u32 = SC1_ADCH | SC1_DIFF | SC1_AIEN;
const ADCH_DISABLED: u32 = 0x1F;
const ADCH_TEMP: u32 = 26;
const ADCH_BANDGAP: u32 = 27;
const ADCH_VREFSH: u32 = 29;
const ADCH_VREFSL: u32 = 30;

const CFG1_MODE_SHIFT: u32 = 2;
const CFG1_MASK: u32 = 0xFF;
const CFG2_MUXSEL: u32 = 1 << 4;
const CFG2_MASK: u32 = 0x1F;

const SC2_REFSEL: u32 = 0x3;
const SC2_ACREN: u32 = 1 << 3;
const SC2_ACFGT: u32 = 1 << 4;
const SC2_ACFE: u32 = 1 << 5;
const SC2_ADTRG: u32 = 1 << 6;
const SC2_ADACT: u32 = 1 << 7;
/// REFSEL, DMAEN, ACREN, ACFGT, ACFE, ADTRG. ADACT is read-only.
const SC2_STORED: u32 = 0x7F;

const SC3_AVG: u32 = 0x7; // AVGS + AVGE
const SC3_ADCO: u32 = 1 << 3;
const SC3_CALF: u32 = 1 << 6;
const SC3_CAL: u32 = 1 << 7;

const OFS_RESET: u16 = 0x0004;
const GAIN_RESET: u16 = 0x8200;

/// Kinetis ADC16 with the two `SC1A/SC1B` + `RA/RB` conversion slots.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct KinetisAdc16 {
    /// `SC1n` without `COCO`.
    sc1: [u32; 2],
    /// `SC1n.COCO`, a `Cell` because reading `Rn` clears it.
    coco: [Cell<bool>; 2],
    result: [u16; 2],
    cfg1: u32,
    cfg2: u32,
    cv: [u16; 2],
    sc2: u32,
    /// `AVGE/AVGS` and `ADCO`; `CAL` never reads back set (it completes at
    /// once) and `CALF` is kept apart.
    sc3: u32,
    calf: bool,
    ofs: u16,
    pg: u16,
    mg: u16,
    clp: [u16; CAL_REGS],
    clm: [u16; CAL_REGS],
    /// A software-triggered continuous chain on `SC1A` is running.
    continuous: bool,
    /// Routed pad levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; INPUTS],
    vrefh_mv: u32,
    valt_mv: u32,
}

impl Default for KinetisAdc16 {
    fn default() -> Self {
        Self::new(DEFAULT_VREFH_MV, DEFAULT_VALT_MV)
    }
}

/// What `ADCH` selects for a conversion.
enum Source {
    /// Single-ended pad (input index).
    Pad(usize),
    /// Differential pair `(plus, minus)` input indices.
    Pair(usize, usize),
    /// A reference level: `1` is full scale (VREFSH), `0` is zero
    /// (VREFSL), `-1` is negative full scale (-VREFSH, differential).
    Reference(i32),
}

impl KinetisAdc16 {
    pub fn new(vrefh_mv: u32, valt_mv: u32) -> Self {
        Self {
            sc1: [ADCH_DISABLED; 2],
            coco: [Cell::new(false), Cell::new(false)],
            result: [0; 2],
            cfg1: 0,
            cfg2: 0,
            cv: [0; 2],
            sc2: 0,
            sc3: 0,
            calf: false,
            ofs: OFS_RESET,
            pg: GAIN_RESET,
            mg: GAIN_RESET,
            clp: CAL_RESET,
            clm: CAL_RESET,
            continuous: false,
            inputs_mv: [u16::MAX; INPUTS],
            vrefh_mv: vrefh_mv.max(1),
            valt_mv: valt_mv.max(1),
        }
    }

    fn vref_mv(&self) -> u32 {
        if self.sc2 & SC2_REFSEL == 1 {
            self.valt_mv
        } else {
            self.vrefh_mv
        }
    }

    /// Result width in bits for the `CFG1.MODE` and `DIFF` setting
    /// (MK64F12.svd `CFG1.MODE` enumeration).
    fn bits(&self, diff: bool) -> u32 {
        match ((self.cfg1 >> CFG1_MODE_SHIFT) & 3, diff) {
            (0, false) => 8,
            (0, true) => 9,
            (1, false) => 12,
            (1, true) => 13,
            (2, false) => 10,
            (2, true) => 11,
            _ => 16,
        }
    }

    fn pad_mv(&self, index: usize) -> i64 {
        match self.inputs_mv[index] {
            u16::MAX => 0,
            mv => i64::from(mv),
        }
    }

    /// Decode `ADCH`/`DIFF`/`MUXSEL` (SC1n `ADCH` table, RM §39.4.1 and the
    /// §39.1.5 a/b mux). `None` converts 0: temperature sensor, bandgap and
    /// reserved selections.
    fn source(&self, sc1: u32) -> Option<Source> {
        let adch = sc1 & SC1_ADCH;
        let diff = sc1 & SC1_DIFF != 0;
        match (adch, diff) {
            (0..=3, false) => Some(Source::Pad(adch as usize)),
            (0..=3, true) => Some(Source::Pair(adch as usize, DADM + adch as usize)),
            (4..=7, false) if self.cfg2 & CFG2_MUXSEL != 0 => {
                Some(Source::Pad(B_PADS + adch as usize - 4))
            }
            (4..=23, false) => Some(Source::Pad(adch as usize)),
            (ADCH_VREFSH, false) => Some(Source::Reference(1)),
            (ADCH_VREFSH, true) => Some(Source::Reference(-1)),
            (ADCH_VREFSL, false) => Some(Source::Reference(0)),
            (ADCH_TEMP | ADCH_BANDGAP, _) => None,
            _ => None,
        }
    }

    /// The offset-corrected, clamped code for `sc1`, as the 16-bit `Rn`
    /// pattern (sign-extended when differential).
    fn sample(&self, sc1: u32) -> u16 {
        let diff = sc1 & SC1_DIFF != 0;
        let bits = self.bits(diff);
        let vref = i64::from(self.vref_mv());
        let (min, max) = if diff {
            (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1)
        } else {
            (0, (1i64 << bits) - 1)
        };
        let raw = match self.source(sc1) {
            Some(Source::Pad(index)) => (self.pad_mv(index) << bits) / vref,
            Some(Source::Pair(plus, minus)) => {
                ((self.pad_mv(plus) - self.pad_mv(minus)) << (bits - 1)) / vref
            }
            Some(Source::Reference(1)) => max,
            Some(Source::Reference(-1)) => min,
            Some(Source::Reference(_)) | None => 0,
        };
        // OFS is left-justified: its sign bit lines up with the result's
        // MSB, so it shifts right by (16 - N) when differential and by
        // (15 - N) single-ended; 16-bit single-ended has no OFS bit for D[0]
        // and shifts left by one (RM §39.5.7).
        let ofs = i64::from(self.ofs as i16);
        let offset = match (diff, bits) {
            (false, 16) => ofs << 1,
            (false, n) => ofs >> (15 - n),
            (true, n) => ofs >> (16 - n),
        };
        (raw.clamp(min, max) - offset).clamp(min, max) as u16
    }

    /// The compare condition of RM Table 39-12 for a result pattern.
    fn compare_true(&self, result: u16, diff: bool) -> bool {
        let value = |raw: u16| {
            if diff {
                i32::from(raw as i16)
            } else {
                i32::from(raw)
            }
        };
        let (r, cv1, cv2) = (value(result), value(self.cv[0]), value(self.cv[1]));
        let greater = self.sc2 & SC2_ACFGT != 0;
        match (greater, self.sc2 & SC2_ACREN != 0) {
            (false, false) => r < cv1,
            (true, false) => r >= cv1,
            (false, true) if cv1 <= cv2 => r < cv1 || r > cv2,
            (false, true) => r < cv1 && r > cv2,
            (true, true) if cv1 <= cv2 => r >= cv1 && r <= cv2,
            (true, true) => r >= cv1 || r <= cv2,
        }
    }

    /// Complete one conversion on slot `n`: transfer and set `COCO`, unless
    /// the compare function is on and false (RM §39.5.4.2).
    fn convert(&mut self, n: usize) {
        let sc1 = self.sc1[n];
        let result = self.sample(sc1);
        if self.sc2 & SC2_ACFE != 0 && !self.compare_true(result, sc1 & SC1_DIFF != 0) {
            return;
        }
        self.result[n] = result;
        self.coco[n].set(true);
    }

    fn write_sc1(&mut self, n: usize, value: u32) {
        // A write aborts the slot's conversion and clears its COCO.
        self.sc1[n] = value & SC1_STORED;
        self.coco[n].set(false);
        if n != 0 {
            return;
        }
        self.continuous = false;
        if self.sc1[0] & SC1_ADCH == ADCH_DISABLED || self.sc2 & SC2_ADTRG != 0 {
            return;
        }
        self.convert(0);
        self.continuous = self.sc3 & SC3_ADCO != 0;
    }

    fn write_sc3(&mut self, value: u32) {
        if value & SC3_CALF != 0 {
            self.calf = false;
        }
        self.sc3 = value & (SC3_AVG | SC3_ADCO);
        if value & SC3_CAL == 0 {
            return;
        }
        if self.sc2 & SC2_ADTRG != 0 {
            // §39.5.6: CAL does not set and CALF sets.
            self.calf = true;
            return;
        }
        // An ideal converter: no offset, the CLPx/CLMx keep their values.
        self.ofs = 0;
        self.coco[0].set(true);
    }

    /// `(minus side, index)` of a `CLPx`/`CLMx` offset, in register order
    /// D, S, 4, 3, 2, 1, 0.
    fn cal_index(offset: u64) -> Option<(bool, usize)> {
        match offset {
            CLPD..=0x4C => Some((false, ((offset - CLPD) / 4) as usize)),
            CLMD..=0x6C => Some((true, ((offset - CLMD) / 4) as usize)),
            _ => None,
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        let sc1 = |n: usize| self.sc1[n] | if self.coco[n].get() { SC1_COCO } else { 0 };
        match offset & !3 {
            SC1A => sc1(0),
            SC1B => sc1(1),
            CFG1 => self.cfg1,
            CFG2 => self.cfg2,
            RA => u32::from(self.result[0]),
            RB => u32::from(self.result[1]),
            CV1 => u32::from(self.cv[0]),
            CV2 => u32::from(self.cv[1]),
            SC2 => self.sc2 | if self.continuous { SC2_ADACT } else { 0 },
            SC3 => self.sc3 | if self.calf { SC3_CALF } else { 0 },
            OFS => u32::from(self.ofs),
            PG => u32::from(self.pg),
            MG => u32::from(self.mg),
            o @ (CLPD..=0x4C | CLMD..=0x6C) => match Self::cal_index(o) {
                Some((false, i)) => u32::from(self.clp[i]),
                Some((true, i)) => u32::from(self.clm[i]),
                None => 0,
            },
            _ => 0,
        }
    }

    /// Reading `Rn` clears `SC1n.COCO` (RM §39.4.1).
    fn after_read(&self, offset: u64) {
        match offset & !3 {
            RA => self.coco[0].set(false),
            RB => self.coco[1].set(false),
            _ => {}
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let offset = offset & !3;
        if !matches!(offset, SC1A | SC1B) {
            // Any other register write aborts a running conversion.
            self.continuous = false;
        }
        match offset {
            SC1A => self.write_sc1(0, value),
            SC1B => self.write_sc1(1, value),
            CFG1 => self.cfg1 = value & CFG1_MASK,
            CFG2 => self.cfg2 = value & CFG2_MASK,
            CV1 => self.cv[0] = value as u16,
            CV2 => self.cv[1] = value as u16,
            SC2 => self.sc2 = value & SC2_STORED,
            SC3 => self.write_sc3(value),
            OFS => self.ofs = value as u16,
            PG => self.pg = value as u16,
            MG => self.mg = value as u16,
            o => match Self::cal_index(o) {
                Some((false, i)) => self.clp[i] = value as u16 & CAL_MASK[i],
                Some((true, i)) => self.clm[i] = value as u16 & CAL_MASK[i],
                None => {}
            },
        }
    }

    fn irq_asserted(&self) -> bool {
        (0..2).any(|n| self.coco[n].get() && self.sc1[n] & SC1_AIEN != 0)
    }
}

impl Peripheral for KinetisAdc16 {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(INPUTS as u8)
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
        let inputs = self.inputs_mv;
        *self = Self::new(self.vrefh_mv, self.valt_mv);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let value = (self.read_word(offset) >> ((offset & 3) * 8)) as u8;
        self.after_read(offset);
        Ok(value)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let lane = offset & 3;
        // SC1n holds its fields in byte 0; a write to the reserved upper
        // bytes is not an SC1n write and must not start a conversion.
        if matches!(aligned, SC1A | SC1B) && lane != 0 {
            return Ok(());
        }
        let shift = lane * 8;
        // CALF is write 1 to clear: merge without it so the other bytes of
        // a byte write do not clear it, and never re-set CAL.
        let current = match aligned {
            SC3 => self.sc3,
            o => self.read_word(o) & !SC1_COCO,
        };
        let next = (current & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(aligned, next);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let value = self.read_word(offset);
        self.after_read(offset);
        Ok(value)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.continuous {
            self.convert(0);
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

    const MODE_8: u32 = 0 << CFG1_MODE_SHIFT;
    const MODE_12: u32 = 1 << CFG1_MODE_SHIFT;
    const MODE_10: u32 = 2 << CFG1_MODE_SHIFT;
    const MODE_16: u32 = 3 << CFG1_MODE_SHIFT;

    fn adc_with(inputs: &[(u8, u16)]) -> KinetisAdc16 {
        let mut adc = KinetisAdc16::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv));
        }
        adc
    }

    fn coco(adc: &KinetisAdc16, n: u64) -> bool {
        adc.read_u32(SC1A + 4 * n).unwrap() & SC1_COCO != 0
    }

    /// fsl_adc16 ADC16_Init: CFG1 = ADICLK | MODE, CFG2 and SC2.REFSEL by
    /// read-modify-write, SC3.ADCO cleared.
    fn init(adc: &mut KinetisAdc16, mode: u32) {
        adc.write_u32(CFG1, mode).unwrap();
        let cfg2 = adc.read_u32(CFG2).unwrap();
        adc.write_u32(CFG2, cfg2 & !0xF).unwrap();
        let sc2 = adc.read_u32(SC2).unwrap();
        adc.write_u32(SC2, sc2 & !SC2_REFSEL).unwrap();
        let sc3 = adc.read_u32(SC3).unwrap();
        adc.write_u32(SC3, sc3 & !SC3_ADCO).unwrap();
    }

    /// ADC16_SetChannelConfig(SC1A) + poll COCO + ADC16_GetChannelConversionValue.
    fn convert(adc: &mut KinetisAdc16, sc1: u32) -> u16 {
        adc.write_u32(SC1A, sc1).unwrap();
        assert!(coco(adc, 0), "COCO after SC1A {sc1:#x}");
        let value = adc.read_u32(RA).unwrap();
        assert!(!coco(adc, 0), "the RA read clears COCO");
        assert_eq!(value >> 16, 0, "Rn[31:16] reserved");
        value as u16
    }

    fn code(mv: u32, bits: u32, vref: u32) -> u16 {
        ((mv << bits) / vref).min((1 << bits) - 1) as u16
    }

    #[test]
    fn fsl_adc16_calibrates_then_converts_against_vrefh() {
        let mut adc = adc_with(&[(12, 1650), (13, 3300), (9, 812)]);
        init(&mut adc, MODE_16 | (1 << 4) /* ADLSMP */);
        assert_eq!(adc.read_u32(OFS).unwrap(), u32::from(OFS_RESET));
        // ADC16_DoAutoCalibration: SC3 |= CAL | CALF, spin on SC1A.COCO
        // while checking CALF, dummy R[0] read, then PG/MG from CLPx/CLMx.
        let sc3 = adc.read_u32(SC3).unwrap();
        adc.write_u32(SC3, sc3 | SC3_CAL | SC3_CALF).unwrap();
        assert!(coco(&adc, 0), "calibration sets SC1A.COCO");
        assert_eq!(adc.read_u32(SC3).unwrap() & (SC3_CAL | SC3_CALF), 0);
        let _ = adc.read_u32(RA).unwrap();
        assert!(!coco(&adc, 0));
        assert_eq!(adc.read_u32(OFS).unwrap(), 0, "ideal offset");
        // CLPS..CLP0 and CLMS..CLM0, skipping CLxD.
        let sum = |adc: &KinetisAdc16, base: u64| {
            (1..CAL_REGS as u64)
                .map(|i| adc.read_u32(base + 4 * i).unwrap())
                .sum::<u32>()
        };
        // 0x20 + 0x200 + 0x100 + 0x80 + 0x40 + 0x20 = 0x400.
        let (plus, minus) = (sum(&adc, CLPD), sum(&adc, CLMD));
        assert_eq!((plus, minus), (0x400, 0x400));
        adc.write_u32(PG, 0x8000 | (plus >> 1)).unwrap();
        adc.write_u32(MG, 0x8000 | (minus >> 1)).unwrap();
        // The ideal CLPx give back the SVD reset gain.
        assert_eq!(adc.read_u32(PG).unwrap(), u32::from(GAIN_RESET));

        init(&mut adc, MODE_12);
        assert_eq!(convert(&mut adc, 12), code(1650, 12, 3300));
        assert_eq!(convert(&mut adc, 12), 2048, "mid scale");
        assert_eq!(convert(&mut adc, 13), 4095, "VREFH clamps at full scale");
        assert_eq!(convert(&mut adc, 9), code(812, 12, 3300));
        // An undriven pad converts 0.
        assert_eq!(convert(&mut adc, 14), 0);
        assert!(!adc.set_adc_channel_input(INPUTS as u8, 1));
    }

    #[test]
    fn mode_sets_resolution_and_ofs_aligns_to_the_result() {
        let mut adc = adc_with(&[(8, 1234)]);
        for (mode, bits) in [(MODE_8, 8), (MODE_10, 10), (MODE_12, 12)] {
            init(&mut adc, mode);
            // Reset OFS = 4 is below the LSB of an 8..12-bit result.
            assert_eq!(convert(&mut adc, 8), code(1234, bits, 3300), "{bits}-bit");
        }
        // 16-bit single-ended: OFS[14:0] lines up with D[15:1], so the reset
        // OFS of 4 subtracts 8 counts.
        init(&mut adc, MODE_16);
        assert_eq!(convert(&mut adc, 8), code(1234, 16, 3300) - 8);
        // OFS -64 (left-justified) adds 64 >> 3 = 8 counts at 12 bits.
        adc.write_u32(OFS, (-64i16) as u16 as u32).unwrap();
        init(&mut adc, MODE_12);
        assert_eq!(convert(&mut adc, 8), code(1234, 12, 3300) + 8);
        // An offset past either rail clamps to the rail.
        adc.write_u32(OFS, 0x7FFF).unwrap();
        assert_eq!(convert(&mut adc, 8), 0);
        adc.write_u32(OFS, 0x8000).unwrap();
        assert_eq!(convert(&mut adc, 8), 4095);
    }

    #[test]
    fn differential_pairs_are_signed_and_sign_extended() {
        let dad2 = 2 | SC1_DIFF;
        let mut adc = adc_with(&[(2, 2000), (DADM as u8 + 2, 1000)]);
        adc.write_u32(OFS, 0).unwrap();
        init(&mut adc, MODE_12);
        // 13-bit differential: (P - M) * 2^12 / VREF.
        let positive = (1000u32 << 12) / 3300;
        assert_eq!(convert(&mut adc, dad2), positive as u16);
        // Swap the pair: negative, bit 12 extended through bit 15.
        adc.set_adc_channel_input(2, 1000);
        adc.set_adc_channel_input(DADM as u8 + 2, 2000);
        let negative = convert(&mut adc, dad2);
        assert_eq!(negative as i16, -(positive as i16));
        assert_eq!(negative & 0xE000, 0xE000, "sign extension");
        // Single-ended ADCH 2 reads DADP2 only.
        assert_eq!(convert(&mut adc, 2), code(1000, 12, 3300));
        // 16-bit differential, 9-bit differential.
        init(&mut adc, MODE_16);
        assert_eq!(
            convert(&mut adc, dad2) as i16,
            -((1000i32 << 15) / 3300) as i16
        );
        init(&mut adc, MODE_8);
        assert_eq!(
            convert(&mut adc, dad2) as i16,
            -((1000i32 << 8) / 3300) as i16
        );
        // -VREFSH is negative full scale; VREFSH/VREFSL are the rails.
        init(&mut adc, MODE_12);
        assert_eq!(convert(&mut adc, ADCH_VREFSH | SC1_DIFF), 0xF000);
        assert_eq!(convert(&mut adc, ADCH_VREFSH), 4095);
        assert_eq!(convert(&mut adc, ADCH_VREFSL), 0);
        // Temperature sensor and bandgap are not modelled: they convert 0.
        assert_eq!(convert(&mut adc, ADCH_TEMP), 0);
        assert_eq!(convert(&mut adc, ADCH_BANDGAP), 0);
    }

    #[test]
    fn muxsel_and_refsel_pick_the_pad_and_the_reference() {
        let mut adc = adc_with(&[(6, 600), (B_PADS as u8 + 2, 900)]);
        init(&mut adc, MODE_12);
        assert_eq!(convert(&mut adc, 6), code(600, 12, 3300), "AD6a");
        adc.write_u32(CFG2, CFG2_MUXSEL).unwrap();
        assert_eq!(convert(&mut adc, 6), code(900, 12, 3300), "AD6b");
        // MUXSEL only applies to ADCH 4..7.
        adc.set_adc_channel_input(8, 100);
        assert_eq!(convert(&mut adc, 8), code(100, 12, 3300));
        // REFSEL 01: VALTH, the 1.2 V VREF_OUT.
        adc.write_u32(SC2, 1).unwrap();
        assert_eq!(convert(&mut adc, 6), code(900, 12, 1200));
        assert_eq!(convert(&mut adc, 8), code(100, 12, 1200));
    }

    #[test]
    fn compare_function_follows_table_39_12() {
        let mut adc = adc_with(&[(10, 0)]);
        adc.write_u32(OFS, 0).unwrap();
        let at = |adc: &mut KinetisAdc16, count: u16, sc2: u32, cv1: u16, cv2: u16| {
            // mv for an exact 8-bit count at 3.3 V: count * 3300 / 256,
            // rounded up so truncation lands on `count`.
            let mv = (u32::from(count) * 3300).div_ceil(256);
            adc.set_adc_channel_input(10, mv as u16);
            adc.write_u32(CFG1, MODE_8).unwrap();
            adc.write_u32(CV1, cv1.into()).unwrap();
            adc.write_u32(CV2, cv2.into()).unwrap();
            adc.write_u32(SC2, SC2_ACFE | sc2).unwrap();
            adc.write_u32(SC1A, 10).unwrap();
            let hit = coco(adc, 0);
            let r = adc.read_u32(RA).unwrap();
            (hit, r)
        };
        let (lt, ge, out_ex, in_inc) = (0, SC2_ACFGT, SC2_ACREN, SC2_ACFGT | SC2_ACREN);
        // Less than CV1 / greater than or equal to CV1.
        assert!(at(&mut adc, 99, lt, 100, 0).0);
        assert!(!at(&mut adc, 100, lt, 100, 0).0);
        assert!(at(&mut adc, 100, ge, 100, 0).0);
        assert!(!at(&mut adc, 99, ge, 100, 0).0);
        // CV1 <= CV2: outside not inclusive / inside inclusive.
        assert!(at(&mut adc, 49, out_ex, 50, 150).0);
        assert!(at(&mut adc, 151, out_ex, 50, 150).0);
        assert!(!at(&mut adc, 150, out_ex, 50, 150).0);
        assert!(at(&mut adc, 50, in_inc, 50, 150).0);
        assert!(at(&mut adc, 150, in_inc, 50, 150).0);
        assert!(!at(&mut adc, 151, in_inc, 50, 150).0);
        // CV1 > CV2: inside not inclusive / outside inclusive.
        assert!(at(&mut adc, 100, out_ex, 150, 50).0);
        assert!(!at(&mut adc, 150, out_ex, 150, 50).0);
        assert!(at(&mut adc, 150, in_inc, 150, 50).0);
        assert!(at(&mut adc, 50, in_inc, 150, 50).0);
        assert!(!at(&mut adc, 100, in_inc, 150, 50).0);
        // A false compare leaves Rn at the last transferred result.
        let (_, kept) = at(&mut adc, 40, lt, 100, 0);
        assert_eq!(kept, 40);
        let (hit, r) = at(&mut adc, 200, lt, 100, 0);
        assert!(!hit);
        assert_eq!(r, 40, "no transfer on a false compare");
    }

    #[test]
    fn continuous_conversion_follows_the_pad_until_aborted() {
        let mut adc = adc_with(&[(12, 1000)]);
        init(&mut adc, MODE_12);
        adc.write_u32(SC3, SC3_ADCO).unwrap();
        adc.write_u32(SC1A, 12).unwrap();
        assert_ne!(adc.read_u32(SC2).unwrap() & SC2_ADACT, 0, "ADACT");
        assert_eq!(adc.read_u32(RA).unwrap() as u16, code(1000, 12, 3300));
        adc.set_adc_channel_input(12, 2000);
        adc.tick();
        assert!(coco(&adc, 0), "the next conversion completes");
        assert_eq!(adc.read_u32(RA).unwrap() as u16, code(2000, 12, 3300));
        // ADCH = 31 stops the chain.
        adc.write_u32(SC1A, ADCH_DISABLED).unwrap();
        assert_eq!(adc.read_u32(SC2).unwrap() & SC2_ADACT, 0);
        adc.set_adc_channel_input(12, 500);
        adc.tick();
        assert!(!coco(&adc, 0));
        assert_eq!(adc.read_u32(RA).unwrap() as u16, code(2000, 12, 3300));
        // Restart, then any other register write aborts it.
        adc.write_u32(SC1A, 12).unwrap();
        let _ = adc.read_u32(RA);
        adc.write_u32(CV1, 0).unwrap();
        adc.tick();
        assert!(!coco(&adc, 0), "a CV1 write aborted the chain");
        // Without ADCO an SC1A write converts once.
        adc.write_u32(SC3, 0).unwrap();
        adc.write_u32(SC1A, 12).unwrap();
        let _ = adc.read_u32(RA);
        adc.tick();
        assert!(!coco(&adc, 0));
    }

    #[test]
    fn hardware_trigger_mode_and_calibration_failure() {
        let mut adc = adc_with(&[(12, 1000)]);
        init(&mut adc, MODE_12);
        adc.write_u32(SC2, SC2_ADTRG).unwrap();
        // No PDB trigger is modelled: SC1A only stores the channel.
        adc.write_u32(SC1A, 12 | SC1_AIEN).unwrap();
        assert!(!coco(&adc, 0));
        assert_eq!(adc.read_u32(SC1A).unwrap(), 12 | SC1_AIEN);
        // CAL with ADTRG = 1: CAL does not set, CALF sets, no COCO.
        adc.write_u32(SC3, SC3_CAL).unwrap();
        assert_eq!(adc.read_u32(SC3).unwrap(), SC3_CALF);
        assert!(!coco(&adc, 0));
        assert_eq!(adc.read_u32(OFS).unwrap(), u32::from(OFS_RESET));
        // A byte write without bit 6 keeps CALF; writing 1 clears it.
        adc.write(SC3, SC3_ADCO as u8).unwrap();
        assert_eq!(adc.read_u32(SC3).unwrap(), SC3_CALF | SC3_ADCO);
        adc.write_u32(SC3, SC3_CALF).unwrap();
        assert_eq!(adc.read_u32(SC3).unwrap(), 0);
    }

    #[test]
    fn coco_with_aien_drives_the_irq_and_sc1b_never_starts() {
        let mut adc = adc_with(&[(12, 1000), (13, 2000)]);
        init(&mut adc, MODE_12);
        adc.write_u32(SC1B, 13 | SC1_AIEN).unwrap();
        assert!(!coco(&adc, 1), "SC1B is hardware-trigger only");
        assert_eq!(adc.irq_line_level(), Some(false));
        adc.write_u32(SC1A, 12).unwrap();
        assert!(coco(&adc, 0));
        assert_eq!(adc.irq_line_level(), Some(false), "COCO without AIEN");
        adc.write_u32(SC1A, 12 | SC1_AIEN).unwrap();
        assert_eq!(adc.irq_line_level(), Some(true));
        assert!(adc.tick().irq);
        // A byte read of RA clears COCO and drops the line.
        let _ = adc.read(RA + 1).unwrap();
        assert_eq!(adc.irq_line_level(), Some(false));
        // A byte write to the reserved upper bytes of SC1A starts nothing.
        adc.write(SC1A + 1, 0).unwrap();
        assert!(!coco(&adc, 0));
        // COCO is read-only.
        adc.write_u32(SC1A, ADCH_DISABLED | SC1_COCO).unwrap();
        assert_eq!(adc.read_u32(SC1A).unwrap(), ADCH_DISABLED);
    }

    #[test]
    fn reset_restores_svd_values_and_keeps_inputs() {
        let mut adc = adc_with(&[(12, 3300)]);
        adc.write_u32(CFG1, 0xFFFF_FFFF).unwrap();
        adc.write_u32(CLPD + 8, 0xFFFF).unwrap();
        assert_eq!(adc.read_u32(CLPD + 8).unwrap(), 0x3FF, "CLP4 width");
        adc.write_u32(CLMD + 24, 0xFFFF).unwrap();
        assert_eq!(adc.read_u32(CLMD + 24).unwrap(), 0x3F, "CLM0 width");
        assert_eq!(adc.read_u32(0x50).unwrap(), 0, "gap between CLP0 and CLMD");
        adc.on_system_reset();
        assert_eq!(adc.read_u32(SC1A).unwrap(), 0x1F);
        assert_eq!(adc.read_u32(SC1B).unwrap(), 0x1F);
        assert_eq!(adc.read_u32(CFG1).unwrap(), 0);
        assert_eq!(adc.read_u32(PG).unwrap(), 0x8200);
        assert_eq!(adc.read_u32(MG).unwrap(), 0x8200);
        assert_eq!(adc.read_u32(CLPD + 8).unwrap(), 0x200);
        init(&mut adc, MODE_12);
        assert_eq!(convert(&mut adc, 12), 4095);
    }
}
