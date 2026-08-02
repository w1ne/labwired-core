// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Classic ESP32 (Xtensa LX6) SAR ADC — behavioral one-shot conversion engine.
//!
//! Unlike the C3/S3, the classic ESP32 has no `APB_SARADC` block: the one-shot
//! ("RTC controller") ADC path the IDF `adc1_get_raw` / `adc2_get_raw` drivers
//! use lives in the **SENS** peripheral (`DR_REG_SENS_BASE = 0x3FF4_8800`). This
//! model implements the minimum of that register file needed to make a genuine,
//! channel- and width-dependent conversion observable — mirroring
//! `peripherals::esp32c3::apb_saradc` but on the SENS register layout.
//!
//! ## One-shot handshake (offsets + fields from the ESP32 TRM §29.4 /
//! `soc/esp32/sens_reg.h`)
//!
//! SAR1 (`adc1_get_raw`):
//!   1. Program `SAR_READ_CTRL` (0x00): `SAR1_SAMPLE_BIT` [17:16] selects the
//!      resolution (00=9-bit … 11=12-bit).
//!   2. Program `SAR_MEAS_START1` (0x54): `SAR1_EN_PAD` [30:19] is the one-hot
//!      channel-enable bitmap, `MEAS1_START_FORCE` [18] hands control to
//!      software, `MEAS1_START_SAR` [17] triggers a conversion.
//!   3. Busy-poll `MEAS1_DONE_SAR` [16] (RO) until set.
//!   4. Read the sample from `MEAS1_DATA_SAR` [15:0] (RO).
//!
//! SAR2 (`adc2_get_raw`) is the same shape via `SAR_READ_CTRL2` (0x90) /
//! `SAR_MEAS_START2` (0x94).
//!
//! ## Conversion model — deterministic, channel- and width-dependent
//!
//! There is no real analog front-end, so each channel gets a FIXED 12-bit
//! source code via [`channel_sample`] (an injective ramp). On a `START_SAR`
//! strobe the model decodes the selected channel from the one-hot `EN_PAD`
//! bitmap, scales the 12-bit code down to the configured resolution (a faithful
//! model of a lower-resolution SAR: `code >> (12 - bits)`), latches it into the
//! unit's `DATA_SAR` field and raises `DONE_SAR`. Because the result is a
//! function of BOTH the selected channel and the configured width — reading
//! channel 3 at 12 bits differs from channel 5, and the same channel at 9 bits
//! is the 12-bit value shifted right by 3 — a declarative register file (which
//! returns a constant `DATA` and never raises the RO `DONE` bit) cannot
//! reproduce this behavior.

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// SENS block base on the classic ESP32 (`DR_REG_SENS_BASE`).
pub const SENS_BASE: u32 = 0x3FF4_8800;
/// Window covering the SAR control + measurement registers (max offset 0x98).
pub const SENS_SIZE: u64 = 0x100;

const SAR_READ_CTRL: u64 = 0x00;
const SAR_MEAS_START1: u64 = 0x54;
const SAR_READ_CTRL2: u64 = 0x90;
const SAR_MEAS_START2: u64 = 0x94;

/// `MEAS{1,2}_START_SAR` (bit 17) — conversion trigger.
const MEAS_START_SAR: u32 = 1 << 17;
/// `MEAS{1,2}_DONE_SAR` (bit 16, RO) — conversion complete.
const MEAS_DONE_SAR: u32 = 1 << 16;
/// `MEAS{1,2}_DATA_SAR` field (bits [15:0], RO) — latched sample.
const MEAS_DATA_MASK: u32 = 0xFFFF;
/// `SAR{1,2}_EN_PAD` one-hot channel bitmap (bits [30:19]).
const EN_PAD_SHIFT: u32 = 19;
const EN_PAD_MASK: u32 = 0xFFF;
/// Writable bits of a `MEAS_START` register ([31:17]); [16:0] are RO
/// (`DONE_SAR` + `DATA_SAR`), recomputed on read from the latched result.
const MEAS_START_WRITABLE: u32 = 0xFFFE_0000;
/// `SAR{1,2}_SAMPLE_BIT` resolution selector (bits [17:16] of the READ_CTRL
/// registers): 00=9-bit … 11=12-bit.
const SAMPLE_BIT_SHIFT: u32 = 16;
const SAMPLE_BIT_MASK: u32 = 0x3;

/// Deterministic fixed 12-bit source code for `channel`. The ramp is injective
/// over the ESP32's ADC channel range (0..11) so a reader can prove the result
/// tracks the SELECTED channel, not a constant.
fn channel_sample(channel: u32) -> u32 {
    (0x100 + channel * 0x111) & 0x0FFF
}

/// One SAR unit's latched conversion state.
#[derive(Default)]
struct SarUnit {
    done: bool,
    data: u32,
}

pub struct Esp32SarAdc {
    /// Register-backed storage for the whole window (word indexed). Holds the
    /// writable bits of every offset (calibration, force flags, etc.) so the
    /// IDF's bring-up writes round-trip; the RO DONE/DATA fields are overlaid
    /// on read from `sar1`/`sar2`.
    regs: Vec<u32>,
    sar1: SarUnit,
    sar2: SarUnit,
}

impl Default for Esp32SarAdc {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Esp32SarAdc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32SarAdc(sar1: done={} data=0x{:03x}, sar2: done={} data=0x{:03x})",
            self.sar1.done, self.sar1.data, self.sar2.done, self.sar2.data
        )
    }
}

impl Esp32SarAdc {
    pub fn new() -> Self {
        let mut regs = vec![0u32; (SENS_SIZE / 4) as usize];
        // Reset value of SAR{1,2}_SAMPLE_BIT is 0b11 (12-bit) — TRM default.
        let default_width = SAMPLE_BIT_MASK << SAMPLE_BIT_SHIFT;
        regs[(SAR_READ_CTRL / 4) as usize] = default_width;
        regs[(SAR_READ_CTRL2 / 4) as usize] = default_width;
        Self {
            regs,
            sar1: SarUnit::default(),
            sar2: SarUnit::default(),
        }
    }

    fn reg(&self, off: u64) -> u32 {
        *self.regs.get((off / 4) as usize).unwrap_or(&0)
    }

    /// Run a conversion for the given `start_word` and resolution, returning the
    /// latched unit state, or `None` if no channel was selected.
    fn convert(start_word: u32, width_bits: u32) -> Option<SarUnit> {
        let en_pad = (start_word >> EN_PAD_SHIFT) & EN_PAD_MASK;
        if en_pad == 0 {
            return None;
        }
        // The IDF selects exactly one channel via a one-hot EN_PAD bitmap.
        let channel = en_pad.trailing_zeros();
        let full = channel_sample(channel);
        // Faithful lower-resolution SAR: drop the low (12 - width) bits.
        let data = full >> (12 - width_bits);
        Some(SarUnit {
            done: true,
            data: data & MEAS_DATA_MASK,
        })
    }

    /// Combine the writable bits stored in `regs` with the unit's RO DONE/DATA.
    fn meas_read(&self, off: u64, unit: &SarUnit) -> u32 {
        let writable = self.reg(off) & MEAS_START_WRITABLE;
        let done = if unit.done { MEAS_DONE_SAR } else { 0 };
        writable | done | (unit.data & MEAS_DATA_MASK)
    }
}

impl Peripheral for Esp32SarAdc {
    // Inert walk: conversions complete at the MEAS_START write (result latched there); tick() is an explicit no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_u32(offset & !3)?;
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let sh = (offset & 3) * 8;
        let cur = self.read_u32(aligned)?;
        self.write_u32(aligned, (cur & !(0xFFu32 << sh)) | ((value as u32) << sh))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset & !3 {
            SAR_MEAS_START1 => self.meas_read(SAR_MEAS_START1, &self.sar1),
            SAR_MEAS_START2 => self.meas_read(SAR_MEAS_START2, &self.sar2),
            o => self.reg(o),
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            off @ (SAR_MEAS_START1 | SAR_MEAS_START2) => {
                // Persist only the writable bits; DONE/DATA are RO overlays.
                if let Some(slot) = self.regs.get_mut((off / 4) as usize) {
                    *slot = value & MEAS_START_WRITABLE;
                }
                if value & MEAS_START_SAR != 0 {
                    let (ctrl_off, unit) = if off == SAR_MEAS_START1 {
                        (SAR_READ_CTRL, &mut self.sar1)
                    } else {
                        (SAR_READ_CTRL2, &mut self.sar2)
                    };
                    let bits = 9
                        + ((self.regs[(ctrl_off / 4) as usize] >> SAMPLE_BIT_SHIFT)
                            & SAMPLE_BIT_MASK);
                    if let Some(converted) = Self::convert(value, bits) {
                        *unit = converted;
                    }
                }
            }
            o => {
                if let Some(slot) = self.regs.get_mut((o / 4) as usize) {
                    *slot = value;
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a SAR1 one-shot of `channel`: one-hot EN_PAD + START_FORCE +
    /// START_SAR.
    fn sar1_oneshot(channel: u32) -> u32 {
        let en_pad = (1u32 << channel) & EN_PAD_MASK;
        (en_pad << EN_PAD_SHIFT) | (1 << 18) | MEAS_START_SAR
    }

    fn set_width(adc: &mut Esp32SarAdc, ctrl: u64, sample_bit: u32) {
        adc.write_u32(ctrl, (sample_bit & SAMPLE_BIT_MASK) << SAMPLE_BIT_SHIFT)
            .unwrap();
    }

    #[test]
    fn done_not_set_before_conversion() {
        let a = Esp32SarAdc::new();
        assert_eq!(a.read_u32(SAR_MEAS_START1).unwrap() & MEAS_DONE_SAR, 0);
    }

    #[test]
    fn oneshot_sets_done_and_returns_channel_sample() {
        let mut a = Esp32SarAdc::new();
        set_width(&mut a, SAR_READ_CTRL, 3); // 12-bit
        a.write_u32(SAR_MEAS_START1, sar1_oneshot(3)).unwrap();
        let v = a.read_u32(SAR_MEAS_START1).unwrap();
        assert_ne!(v & MEAS_DONE_SAR, 0, "DONE raised");
        assert_eq!(v & MEAS_DATA_MASK, channel_sample(3), "ch3 12-bit sample");
    }

    #[test]
    fn result_tracks_selected_channel() {
        // Headline truthfulness property: distinct channels give distinct,
        // predictable codes.
        let mut a = Esp32SarAdc::new();
        set_width(&mut a, SAR_READ_CTRL, 3);
        a.write_u32(SAR_MEAS_START1, sar1_oneshot(3)).unwrap();
        let d3 = a.read_u32(SAR_MEAS_START1).unwrap() & MEAS_DATA_MASK;
        a.write_u32(SAR_MEAS_START1, sar1_oneshot(5)).unwrap();
        let d5 = a.read_u32(SAR_MEAS_START1).unwrap() & MEAS_DATA_MASK;
        assert_eq!(d3, channel_sample(3));
        assert_eq!(d5, channel_sample(5));
        assert_ne!(d3, d5, "different channels yield different results");
    }

    #[test]
    fn result_tracks_configured_width() {
        // The same channel at 9-bit resolution is the 12-bit code >> 3.
        let mut a = Esp32SarAdc::new();
        set_width(&mut a, SAR_READ_CTRL, 3); // 12-bit
        a.write_u32(SAR_MEAS_START1, sar1_oneshot(7)).unwrap();
        let d12 = a.read_u32(SAR_MEAS_START1).unwrap() & MEAS_DATA_MASK;

        set_width(&mut a, SAR_READ_CTRL, 0); // 9-bit
        a.write_u32(SAR_MEAS_START1, sar1_oneshot(7)).unwrap();
        let d9 = a.read_u32(SAR_MEAS_START1).unwrap() & MEAS_DATA_MASK;

        assert_eq!(d12, channel_sample(7));
        assert_eq!(d9, d12 >> 3, "9-bit result is 12-bit value >> 3");
    }

    #[test]
    fn sar2_unit_is_independent() {
        let mut a = Esp32SarAdc::new();
        set_width(&mut a, SAR_READ_CTRL2, 3);
        let en_pad = (1u32 << 2) & EN_PAD_MASK;
        a.write_u32(
            SAR_MEAS_START2,
            (en_pad << EN_PAD_SHIFT) | (1 << 18) | MEAS_START_SAR,
        )
        .unwrap();
        let v2 = a.read_u32(SAR_MEAS_START2).unwrap();
        assert_ne!(v2 & MEAS_DONE_SAR, 0);
        assert_eq!(v2 & MEAS_DATA_MASK, channel_sample(2));
        // SAR1 untouched.
        assert_eq!(a.read_u32(SAR_MEAS_START1).unwrap() & MEAS_DONE_SAR, 0);
    }

    #[test]
    fn no_channel_selected_does_not_convert() {
        let mut a = Esp32SarAdc::new();
        // START_SAR set but EN_PAD bitmap empty.
        a.write_u32(SAR_MEAS_START1, (1 << 18) | MEAS_START_SAR)
            .unwrap();
        assert_eq!(a.read_u32(SAR_MEAS_START1).unwrap() & MEAS_DONE_SAR, 0);
    }

    #[test]
    fn other_registers_are_register_backed() {
        let mut a = Esp32SarAdc::new();
        a.write_u32(0x08, 0xDEAD_BEEF).unwrap();
        assert_eq!(a.read_u32(0x08).unwrap(), 0xDEAD_BEEF);
    }
}
