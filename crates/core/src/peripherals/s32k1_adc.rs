// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP S32K1 ADC, the 12-bit SAR ADC with 16 `SC1n`/`Rn` slots on the
//! S32K11x/S32K14x parts (`ADC0` at `0x4003_B000` on the S32K118).
//!
//! Register map and bit fields come from the S32K118 device header vendored
//! in Zephyr hal_nxp @cc9b44ae (`s32/drivers/s32k1/BaseNXP/header/
//! S32K118_ADC.h`: `SC1[16]` @0x00, `CFG1` @0x40, `CFG2` @0x44, `R[16]`
//! @0x48, `CV[2]` @0x88, `SC2` @0x90, `SC3` @0x94, `BASE_OFS..CLP9_OFS`
//! @0x98..0xE8, plus the field masks). Reset values and the `ADCH`, `MODE`
//! and `REFSEL` enumerations come from the S32K144 SVD of the same S32K1 IP
//! (github kjetilkjeka/s32k144.rs @444b0e87 `S32K144.svd`: `SC1n` 0x1F,
//! `CFG2` 0xC, `BASE_OFS` 0x40, `XOFS` 0x30, `YOFS` 0x37, `G` 0x2F0, `UG`
//! 0x4, `CLPS` 0x2E, `CLP3..CLP0` 0x180/0xB8/0x5C/0x2E, `CLPX_OFS` 0x440,
//! `CLP9_OFS` 0x240, the rest 0). The firmware handshake is the NXP
//! s32k118_cookbook @a75b5d9d `S32K118_Project_ADC/src/ADC.c` and
//! `S32K118_Project_ADC_Cal/src/ADC.c` ("For SW trigger mode, SC1[0] is
//! used": write `SC1[0].ADCH`, poll `COCO`, read `R[0]`; `SC3.CAL`, then poll
//! `SC1[0].COCO`).
//!
//! Modelled:
//! - Software trigger (`SC2.ADTRG` = 0): a write to `SC1[0]` with `ADCH`
//!   other than 0x1F converts. Writes to `SC1[1..15]` only store, since those
//!   slots are for the hardware trigger handler.
//! - `CFG1.MODE` resolutions 8/12/10 bits, right-justified in `Rn.D`, code
//!   `V * 2^N / VREF`, truncated and clamped. `MODE` 11 is reserved and
//!   converts at 12 bits.
//! - `ADCH` 0..15 convert external channels `ADC0_SE0..SE15`. 29 converts
//!   VREFSH (full scale) and 30 converts VREFSL (zero). Every other
//!   selection (external 18/19 on bigger parts, internal channels, the
//!   temperature sensor, the bandgap) converts 0.
//! - `SC2.REFSEL`: 00 = VREFH (5 V by default, the S32K118EVB supply the
//!   cookbook scales by: `5000 * R / 0xFFF`), 01 = VALTH (5 V by default).
//!   Both are configurable.
//! - The compare function (`SC2.ACFE/ACFGT/ACREN`, `CV1/CV2`), the six ADC16
//!   modes (see `kinetis_adc16::compare_true`): when it is false, `COCO`
//!   stays clear and `Rn` keeps its last value.
//! - `COCO` clears on a write to its `SC1n` or a read of its `Rn`. With
//!   `AIEN` it drives the IRQ line.
//! - `SC3.ADCO` continuous conversion: started by the `SC1[0]` write, it
//!   re-samples once per tick until `SC1[0]`, `CFG1`, `CFG2`, `SC2` or `SC3`
//!   is written. `SC2.ADACT` reads 1 meanwhile.
//! - Calibration: `SC3.CAL` completes at once, reads back clear and sets
//!   `SC1[0].COCO`, which is what the cookbook polls. The converter is ideal,
//!   so calibration leaves the `OFS`, `G` and `CLPx` registers as they are.
//!
//! Not modelled: hardware triggers (PDB, the trigger latches, so `ADTRG` = 1
//! converts nothing and `TRGPRNUM/TRGSTLAT/TRGSTERR` read 0), the offset and
//! gain arithmetic (`OFS`, `USR_OFS`, `G`, `UG`, `CLPx` are stored only, with
//! the header masks), DMA requests (`DMAEN` stored only), conversion time
//! (clock and sample-time fields are stored only, and the hardware average
//! of a held sample equals that sample), and PCC clock gating.

use crate::peripherals::kinetis_adc16::compare_true;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;

/// Routable pad inputs: `ADC0_SE0..SE15`, input index = `ADCH`.
pub const INPUTS: usize = 16;
const SLOTS: usize = 16;

const DEFAULT_VREFH_MV: u32 = 5000;
const DEFAULT_VALT_MV: u32 = 5000;

// Offsets (S32K118_ADC.h `ADC_Type`).
const SC1_END: u64 = 0x3C;
const CFG1: u64 = 0x40;
const CFG2: u64 = 0x44;
const R0: u64 = 0x48;
const R_END: u64 = 0x84;
const CV1: u64 = 0x88;
const CV2: u64 = 0x8C;
const SC2: u64 = 0x90;
const SC3: u64 = 0x94;
/// `BASE_OFS` @0x98 through `CLP9_OFS` @0xE8, one word each.
const TRIM0: u64 = 0x98;
const TRIM_END: u64 = 0xE8;
const TRIM_REGS: usize = 21;

/// Reset values of `BASE_OFS, OFS, USR_OFS, XOFS, YOFS, G, UG, CLPS, CLP3,
/// CLP2, CLP1, CLP0, CLPX, CLP9, CLPS_OFS, CLP3_OFS, CLP2_OFS, CLP1_OFS,
/// CLP0_OFS, CLPX_OFS, CLP9_OFS` (S32K144.svd).
const TRIM_RESET: [u16; TRIM_REGS] = [
    0x40, 0, 0, 0x30, 0x37, 0x2F0, 0x4, 0x2E, 0x180, 0xB8, 0x5C, 0x2E, 0, 0, 0, 0, 0, 0, 0, 0x440,
    0x240,
];
/// Field masks in the same order (S32K118_ADC.h `*_MASK`).
const TRIM_MASK: [u16; TRIM_REGS] = [
    0xFF, 0xFFFF, 0xFF, 0x3F, 0xFF, 0x7FF, 0x3FF, 0x7F, 0x3FF, 0x3FF, 0x1FF, 0xFF, 0x7F, 0x7F, 0xF,
    0xF, 0xF, 0xF, 0xF, 0xFFF, 0xFFF,
];

const SC1_ADCH: u32 = 0x1F;
const SC1_AIEN: u32 = 1 << 6;
const SC1_COCO: u32 = 1 << 7;
const SC1_STORED: u32 = SC1_ADCH | SC1_AIEN;
const ADCH_DISABLED: u32 = 0x1F;
const ADCH_VREFSH: u32 = 29;
const ADCH_VREFSL: u32 = 30;

const CFG1_MODE_SHIFT: u32 = 2;
/// `ADICLK`, `MODE`, `ADIV`; `CLRLTRG` (bit 8) is write-only.
const CFG1_MASK: u32 = 0x6F;
const CFG2_MASK: u32 = 0xFF;
const CFG2_RESET: u32 = 0x0C;

const SC2_REFSEL: u32 = 0x3;
const SC2_ACREN: u32 = 1 << 3;
const SC2_ACFGT: u32 = 1 << 4;
const SC2_ACFE: u32 = 1 << 5;
const SC2_ADTRG: u32 = 1 << 6;
const SC2_ADACT: u32 = 1 << 7;
/// `REFSEL`, `DMAEN`, `ACREN`, `ACFGT`, `ACFE`, `ADTRG`.
const SC2_STORED: u32 = 0x7F;

const SC3_AVG: u32 = 0x7; // AVGS + AVGE
const SC3_ADCO: u32 = 1 << 3;
const SC3_CAL: u32 = 1 << 7;

/// S32K1 ADC with 16 `SC1n` + `Rn` slots.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct S32k1Adc {
    /// `SC1n` without `COCO`.
    sc1: [u32; SLOTS],
    /// `SC1n.COCO`, a `Cell` because reading `Rn` clears it.
    coco: [Cell<bool>; SLOTS],
    result: [u16; SLOTS],
    cfg1: u32,
    cfg2: u32,
    cv: [u16; 2],
    sc2: u32,
    /// `AVGE/AVGS` and `ADCO`; `CAL` never reads back set.
    sc3: u32,
    trim: [u16; TRIM_REGS],
    /// A software-triggered continuous chain on `SC1[0]` is running.
    continuous: bool,
    /// Routed pad levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; INPUTS],
    vrefh_mv: u32,
    valt_mv: u32,
}

impl Default for S32k1Adc {
    fn default() -> Self {
        Self::new(DEFAULT_VREFH_MV, DEFAULT_VALT_MV)
    }
}

impl S32k1Adc {
    pub fn new(vrefh_mv: u32, valt_mv: u32) -> Self {
        Self {
            sc1: [ADCH_DISABLED; SLOTS],
            coco: std::array::from_fn(|_| Cell::new(false)),
            result: [0; SLOTS],
            cfg1: 0,
            cfg2: CFG2_RESET,
            cv: [0; 2],
            sc2: 0,
            sc3: 0,
            trim: TRIM_RESET,
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

    /// Result width for `CFG1.MODE` (S32K144.svd: 00 8-bit, 01 12-bit,
    /// 10 10-bit; 11 reserved).
    fn bits(&self) -> u32 {
        match (self.cfg1 >> CFG1_MODE_SHIFT) & 3 {
            0 => 8,
            2 => 10,
            _ => 12,
        }
    }

    /// The clamped code for `ADCH`, right-justified.
    fn sample(&self, adch: u32) -> u16 {
        let bits = self.bits();
        let max = (1i64 << bits) - 1;
        let raw = match adch {
            0..=15 => match self.inputs_mv[adch as usize] {
                u16::MAX => 0,
                mv => (i64::from(mv) << bits) / i64::from(self.vref_mv()),
            },
            ADCH_VREFSH => max,
            ADCH_VREFSL => 0,
            _ => 0,
        };
        raw.clamp(0, max) as u16
    }

    /// Complete one conversion on slot `n`: transfer and set `COCO`, unless
    /// the compare function is on and false.
    fn convert(&mut self, n: usize) {
        let result = self.sample(self.sc1[n] & SC1_ADCH);
        if self.sc2 & SC2_ACFE != 0
            && !compare_true(
                i32::from(result),
                i32::from(self.cv[0]),
                i32::from(self.cv[1]),
                self.sc2 & SC2_ACFGT != 0,
                self.sc2 & SC2_ACREN != 0,
            )
        {
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
        self.sc3 = value & (SC3_AVG | SC3_ADCO);
        if value & SC3_CAL != 0 {
            // An ideal converter: the trim registers keep their values.
            self.coco[0].set(true);
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset & !3 {
            o @ 0..=SC1_END => {
                let n = (o / 4) as usize;
                self.sc1[n] | if self.coco[n].get() { SC1_COCO } else { 0 }
            }
            CFG1 => self.cfg1,
            CFG2 => self.cfg2,
            o @ R0..=R_END => u32::from(self.result[((o - R0) / 4) as usize]),
            CV1 => u32::from(self.cv[0]),
            CV2 => u32::from(self.cv[1]),
            SC2 => self.sc2 | if self.continuous { SC2_ADACT } else { 0 },
            SC3 => self.sc3,
            o @ TRIM0..=TRIM_END => u32::from(self.trim[((o - TRIM0) / 4) as usize]),
            _ => 0,
        }
    }

    /// Reading `Rn` clears `SC1n.COCO`.
    fn after_read(&self, offset: u64) {
        if let o @ R0..=R_END = offset & !3 {
            self.coco[((o - R0) / 4) as usize].set(false);
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let offset = offset & !3;
        if matches!(offset, CFG1 | CFG2 | SC2 | SC3) {
            // A configuration write aborts a running conversion.
            self.continuous = false;
        }
        match offset {
            o @ 0..=SC1_END => self.write_sc1((o / 4) as usize, value),
            CFG1 => self.cfg1 = value & CFG1_MASK,
            CFG2 => self.cfg2 = value & CFG2_MASK,
            CV1 => self.cv[0] = value as u16,
            CV2 => self.cv[1] = value as u16,
            SC2 => self.sc2 = value & SC2_STORED,
            SC3 => self.write_sc3(value),
            o @ TRIM0..=TRIM_END => {
                let i = ((o - TRIM0) / 4) as usize;
                self.trim[i] = value as u16 & TRIM_MASK[i];
            }
            _ => {}
        }
    }

    fn irq_asserted(&self) -> bool {
        (0..SLOTS).any(|n| self.coco[n].get() && self.sc1[n] & SC1_AIEN != 0)
    }
}

impl Peripheral for S32k1Adc {
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
        if aligned <= SC1_END && lane != 0 {
            return Ok(());
        }
        let shift = lane * 8;
        let current = match aligned {
            o if o <= SC1_END => self.read_word(o) & !SC1_COCO,
            o => self.read_word(o),
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

    fn adc_with(inputs: &[(u8, u16)]) -> S32k1Adc {
        let mut adc = S32k1Adc::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv));
        }
        adc
    }

    /// The cookbook `convertAdcChan` / `adc_complete` / `read_adc_chx`
    /// sequence; returns the `R[0]` code.
    fn convert(adc: &mut S32k1Adc, adch: u32) -> u32 {
        adc.write_u32(0, adch).unwrap();
        assert_ne!(
            adc.read_u32(0).unwrap() & SC1_COCO,
            0,
            "ADCH {adch} sets COCO"
        );
        let code = adc.read_u32(R0).unwrap();
        assert_eq!(
            adc.read_u32(0).unwrap() & SC1_COCO,
            0,
            "the R[0] read clears COCO"
        );
        code
    }

    #[test]
    fn reset_values_match_the_svd() {
        let adc = S32k1Adc::default();
        for n in 0..SLOTS as u64 {
            assert_eq!(adc.read_u32(4 * n).unwrap(), 0x1F, "SC1[{n}]");
        }
        assert_eq!(adc.read_u32(CFG1).unwrap(), 0);
        assert_eq!(adc.read_u32(CFG2).unwrap(), 0x0C);
        let expect = [
            (0x98, 0x40),
            (0xA4, 0x30),
            (0xA8, 0x37),
            (0xAC, 0x2F0),
            (0xB0, 0x4),
            (0xB4, 0x2E),
            (0xB8, 0x180),
            (0xBC, 0xB8),
            (0xC0, 0x5C),
            (0xC4, 0x2E),
            (0xE4, 0x440),
            (0xE8, 0x240),
        ];
        for (offset, value) in expect {
            assert_eq!(adc.read_u32(offset).unwrap(), value, "reset @{offset:#x}");
        }
    }

    #[test]
    fn software_trigger_converts_slot_zero_at_each_resolution() {
        // 1.25 V against VREFH 5 V.
        let mut adc = adc_with(&[(3, 1250)]);
        for (mode, code) in [(MODE_12, 1024), (MODE_10, 256), (MODE_8, 64)] {
            adc.write_u32(CFG1, mode).unwrap();
            assert_eq!(convert(&mut adc, 3), code, "MODE {mode:#x}");
        }
        // Clamped at full scale, VREFSH/VREFSL, and an undriven pad.
        adc.write_u32(CFG1, MODE_12).unwrap();
        adc.set_adc_channel_input(4, 6000);
        assert_eq!(convert(&mut adc, 4), 4095);
        assert_eq!(convert(&mut adc, ADCH_VREFSH), 4095);
        assert_eq!(convert(&mut adc, ADCH_VREFSL), 0);
        assert_eq!(convert(&mut adc, 7), 0);
        // Internal channels, the temperature sensor and bandgap convert 0.
        for adch in [21, 26, 27] {
            assert_eq!(convert(&mut adc, adch), 0, "ADCH {adch}");
        }
    }

    #[test]
    fn refsel_valth_scales_against_the_alternate_reference() {
        let mut adc = S32k1Adc::new(5000, 3300);
        adc.set_adc_channel_input(0, 1650);
        adc.write_u32(CFG1, MODE_12).unwrap();
        assert_eq!(convert(&mut adc, 0), 1351);
        adc.write_u32(SC2, 1).unwrap();
        assert_eq!(convert(&mut adc, 0), 2048);
    }

    #[test]
    fn disabled_channel_hardware_trigger_and_other_slots_do_not_convert() {
        let mut adc = adc_with(&[(2, 2500)]);
        adc.write_u32(CFG1, MODE_12).unwrap();
        adc.write_u32(0, ADCH_DISABLED).unwrap();
        assert_eq!(adc.read_u32(0).unwrap() & SC1_COCO, 0, "ADCH 0x1F is off");
        // SC1[1] belongs to the hardware trigger handler.
        adc.write_u32(4, 2).unwrap();
        assert_eq!(adc.read_u32(4).unwrap(), 2);
        assert_eq!(adc.read_u32(R0 + 4).unwrap(), 0);
        // ADTRG = 1 waits for a PDB trigger, which is not modelled.
        adc.write_u32(SC2, SC2_ADTRG).unwrap();
        adc.write_u32(0, 2).unwrap();
        assert_eq!(adc.read_u32(0).unwrap() & SC1_COCO, 0, "ADTRG = 1");
        // A byte write to a reserved SC1 lane starts nothing.
        adc.write_u32(SC2, 0).unwrap();
        adc.write(1, 0).unwrap();
        assert_eq!(adc.read_u32(0).unwrap() & SC1_COCO, 0, "reserved byte lane");
        // A byte write to the ADCH lane does.
        adc.write(0, 2).unwrap();
        assert_eq!(adc.read_u32(R0).unwrap(), 2048);
    }

    #[test]
    fn calibration_completes_with_slot_zero_coco_and_keeps_trims() {
        let mut adc = S32k1Adc::default();
        // ADC_calibration_init: zero the CLPx, user offset/gain, then CAL.
        for offset in (0xB4..=0xCC).step_by(4) {
            adc.write_u32(offset, 0).unwrap();
        }
        adc.write_u32(0xA0, 0x1FF).unwrap();
        adc.write_u32(0xB0, 0xFFFF).unwrap();
        adc.write_u32(SC3, SC3_CAL | 0x7).unwrap();
        assert_ne!(adc.read_u32(0).unwrap() & SC1_COCO, 0, "CAL ends in COCO");
        assert_eq!(adc.read_u32(SC3).unwrap(), 0x7, "CAL reads back clear");
        assert_eq!(adc.read_u32(0xA0).unwrap(), 0xFF, "USR_OFS mask");
        assert_eq!(adc.read_u32(0xB0).unwrap(), 0x3FF, "UG mask");
        assert_eq!(adc.read_u32(0xB8).unwrap(), 0, "CLP3 stays as written");
    }

    #[test]
    fn compare_function_gates_the_transfer() {
        let mut adc = adc_with(&[(1, 2500)]);
        adc.write_u32(CFG1, MODE_12).unwrap();
        assert_eq!(convert(&mut adc, 1), 2048);
        // Less than CV1 = 1000: false, so COCO stays clear and R[0] holds.
        adc.write_u32(CV1, 1000).unwrap();
        adc.write_u32(SC2, SC2_ACFE).unwrap();
        adc.set_adc_channel_input(1, 3750);
        adc.write_u32(0, 1).unwrap();
        assert_eq!(adc.read_u32(0).unwrap() & SC1_COCO, 0);
        assert_eq!(adc.read_u32(R0).unwrap(), 2048, "R[0] keeps its value");
        // Greater or equal: true.
        adc.write_u32(SC2, SC2_ACFE | SC2_ACFGT).unwrap();
        assert_eq!(convert(&mut adc, 1), 3072);
        // Inside the range [1000, 2000] with ACFGT + ACREN: 3072 is out.
        adc.write_u32(CV2, 2000).unwrap();
        adc.write_u32(SC2, SC2_ACFE | SC2_ACFGT | SC2_ACREN)
            .unwrap();
        adc.write_u32(0, 1).unwrap();
        assert_eq!(adc.read_u32(0).unwrap() & SC1_COCO, 0);
    }

    #[test]
    fn continuous_conversion_follows_the_pad_until_a_config_write() {
        let mut adc = adc_with(&[(5, 1000)]);
        adc.write_u32(CFG1, MODE_12).unwrap();
        adc.write_u32(SC3, SC3_ADCO).unwrap();
        adc.write_u32(0, 5).unwrap();
        assert_eq!(adc.read_u32(R0).unwrap(), 819);
        assert_ne!(
            adc.read_u32(SC2).unwrap() & SC2_ADACT,
            0,
            "ADACT while running"
        );
        adc.set_adc_channel_input(5, 4000);
        adc.tick();
        assert_ne!(adc.read_u32(0).unwrap() & SC1_COCO, 0);
        assert_eq!(adc.read_u32(R0).unwrap(), 3276);
        // A CV write does not abort; an SC3 write does.
        adc.write_u32(CV1, 0).unwrap();
        assert_ne!(adc.read_u32(SC2).unwrap() & SC2_ADACT, 0);
        adc.write_u32(SC3, SC3_ADCO).unwrap();
        assert_eq!(adc.read_u32(SC2).unwrap() & SC2_ADACT, 0, "aborted");
        adc.set_adc_channel_input(5, 0);
        adc.tick();
        assert_eq!(adc.read_u32(R0).unwrap(), 3276, "no more samples");
    }

    #[test]
    fn aien_drives_the_irq_line_until_the_result_is_read() {
        let mut adc = adc_with(&[(0, 2500)]);
        adc.write_u32(CFG1, MODE_12).unwrap();
        adc.write_u32(0, SC1_AIEN).unwrap();
        assert_eq!(adc.irq_line_level(), Some(true));
        assert!(adc.tick().irq);
        adc.read_u32(R0).unwrap();
        assert_eq!(adc.irq_line_level(), Some(false));
        // Without AIEN no interrupt.
        adc.write_u32(0, 0).unwrap();
        assert_eq!(adc.irq_line_level(), Some(false));
    }

    #[test]
    fn reset_keeps_routed_inputs() {
        let mut adc = adc_with(&[(9, 2500)]);
        adc.write_u32(CFG1, MODE_8).unwrap();
        adc.on_system_reset();
        assert_eq!(adc.read_u32(CFG1).unwrap(), 0);
        adc.write_u32(CFG1, MODE_12).unwrap();
        assert_eq!(convert(&mut adc, 9), 2048);
    }
}
