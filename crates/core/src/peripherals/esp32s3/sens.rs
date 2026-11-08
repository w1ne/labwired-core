// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 SENS block (RTC-domain SAR-ADC / touch / TSENS controller) —
//! digital twin.
//!
//! Base address `DR_REG_SENS_BASE = 0x6000_8800`, architected span 0x000..0x200
//! (last register `SENS_SAR_SARDATE` @ 0x1FC). This is the *RTC-domain* sensor
//! controller the IDF/Arduino oneshot ADC driver reaches through
//! `adc_oneshot_ll_*` — DISTINCT from the digital APB SAR-ADC controller
//! modeled in `sar_adc.rs` (`DR_REG_APB_SARADC_BASE = 0x6004_0000`).
//!
//! ## Register file
//!
//! All 71 architected registers of the ESP32-S3 SVD `SENS` block are modeled
//! as a fixed register file: each register is seeded with its SVD reset value
//! and a write applies the register's writable-bit mask
//! (`stored = (stored & !wmask) | (value & wmask)`) — read-only registers and
//! reserved bits read back their reset value, never arbitrary written data.
//! Offsets outside the architected map (the 0x118..0x1FC hole and everything
//! at/above 0x200) read as zero and ignore writes, NOT round-trip, so the SVD
//! behavioral coverage probe cannot mistake this model for generic storage.
//!
//! Reset values and write masks are sourced from the ESP32-S3 SVD (and
//! cross-checked against `soc/esp32s3/register/soc/sens_reg.h`); they are NOT
//! validated against silicon dumps.
//!
//! ## Behavioral model: SAR oneshot done/data — and nothing else
//!
//! The only behavior layered on top of the register file is the SAR1/SAR2
//! software-oneshot completion the IDF oneshot driver busy-polls
//! (`hal/adc_ll.h`, `adc_oneshot_ll_start` / `..._get_event` /
//! `..._get_raw_result`):
//!
//! | reg (offset)               | field                | behavior            |
//! |----------------------------|----------------------|---------------------|
//! | SAR_MEAS1_CTRL2 (0x0C)     | `MEAS1_START_SAR`    | R/W, bit 17: 0→1    |
//! |                            |                      | starts a conversion |
//! |                            | `MEAS1_DONE_SAR`     | RO, bit 16: set on  |
//! |                            |                      | start, cleared when |
//! |                            |                      | START_SAR written 0 |
//! |                            | `MEAS1_DATA_SAR`     | RO, bits [15:0]:    |
//! |                            |                      | conversion result   |
//! | SAR_MEAS2_CTRL2 (0x30)     | same fields for SAR2 | same                |
//!
//! A conversion completes IMMEDIATELY: writing `MEAS*_START_SAR = 1` sets the
//! read-only `MEAS*_DONE_SAR` bit and latches a fixed, plausible mid-scale
//! 12-bit reading (0x800) in `MEAS*_DATA_SAR`, so the driver's done-poll
//! always makes progress and `analogRead()` returns a stable value. Writing
//! `START_SAR = 0` (the driver does 0-then-1 to launch) clears the done bit;
//! the last data value is retained, as on real silicon.
//!
//! The ADC1 launch path also busy-polls `SAR_SLAVE_ADDR1.MEAS_STATUS != 0`
//! before starting; those status bits are read-only and stay 0 here (idle),
//! so the poll exits on the first iteration.
//!
//! Touch, TSENS, RTC-I2C arbitration, the ULP coprocessor interrupts and the
//! hall sensor are register-file-only (no invented behavior).
//!
//! ## ESP32-S2 (`esp32s2_sens`, base `0x3F40_8800`)
//!
//! The S2 SENS block shares 0x000..0x058 with the S3 field for field (the
//! oneshot START/DONE/DATA bits, `SAR1_EN_PAD`, and `SAR_SLAVE_ADDR1.MEAS_STATUS`
//! at [29:22]). Past 0x05C the S2 has its own touch/DAC/COCPU layout and ends at
//! `SENS_SARDATE` @ 0x148. [`Esp32s3Sens::new_esp32s2`] swaps in that map
//! ([`spec_s2`], from the ESP32-S2 SVD shipped with arduino-esp32
//! `tools/ide-debug/svd/esp32s2.svd`). The oneshot behavior is the same code.
//! IDF's boot-time `adc_hw_calibration` constructor busy-polls `MEAS1_DONE_SAR`
//! on the S2, so without this block stock Arduino never reaches `setup()` once
//! `analogRead()` is linked. The S2's RTC SAR is 13-bit, so its codes are
//! too: mid-scale 0x1000 when nothing is injected, and `millivolts * 8191 /
//! 3300` for an injected channel.

use crate::{Peripheral, SimResult};

const SAR_READER1_CTRL: u64 = 0x000;
const SAR_READER1_STATUS: u64 = 0x004;
const SAR_MEAS1_CTRL1: u64 = 0x008;
/// `SENS_SAR_MEAS1_CTRL2` — SAR1 oneshot: DATA[15:0] RO, DONE bit16 RO,
/// START_SAR bit17 R/W, START_FORCE bit18 R/W (`soc/sens_reg.h`).
const SAR_MEAS1_CTRL2: u64 = 0x00C;
const SAR_MEAS1_MUX: u64 = 0x010;
const SAR_ATTEN1: u64 = 0x014;
const SAR_AMP_CTRL1: u64 = 0x018;
const SAR_AMP_CTRL2: u64 = 0x01C;
const SAR_AMP_CTRL3: u64 = 0x020;
const SAR_READER2_CTRL: u64 = 0x024;
const SAR_READER2_STATUS: u64 = 0x028;
const SAR_MEAS2_CTRL1: u64 = 0x02C;
/// `SENS_SAR_MEAS2_CTRL2` — SAR2 oneshot, same field layout as MEAS1_CTRL2.
const SAR_MEAS2_CTRL2: u64 = 0x030;
const SAR_MEAS2_MUX: u64 = 0x034;
const SAR_ATTEN2: u64 = 0x038;
const SAR_POWER_XPD_SAR: u64 = 0x03C;
const SAR_SLAVE_ADDR1: u64 = 0x040;
const SAR_SLAVE_ADDR4: u64 = 0x04C;
const SAR_TSENS_CTRL: u64 = 0x050;
const SAR_TSENS_CTRL2: u64 = 0x054;
const SAR_I2C_CTRL: u64 = 0x058;
const SAR_TOUCH_CONF: u64 = 0x05C;
const SAR_TOUCH_DENOISE: u64 = 0x060;
const SAR_TOUCH_THRES1: u64 = 0x064;
const SAR_TOUCH_THRES14: u64 = 0x098;
const SAR_TOUCH_CHN_ST: u64 = 0x09C;
const SAR_TOUCH_STATUS0: u64 = 0x0A0;
const SAR_TOUCH_STATUS16: u64 = 0x0E0;
const SAR_COCPU_STATE: u64 = 0x0E4;
const SAR_COCPU_INT_RAW: u64 = 0x0E8;
const SAR_COCPU_INT_ENA: u64 = 0x0EC;
const SAR_COCPU_INT_ST: u64 = 0x0F0;
const SAR_COCPU_INT_CLR: u64 = 0x0F4;
const SAR_COCPU_DEBUG: u64 = 0x0F8;
const SAR_HALL_CTRL: u64 = 0x0FC;
const SAR_NOUSE: u64 = 0x100;
const SAR_PERI_CLK_GATE_CONF: u64 = 0x104;
const SAR_PERI_RESET_CONF: u64 = 0x108;
const SAR_COCPU_INT_ENA_W1TS: u64 = 0x10C;
const SAR_COCPU_INT_ENA_W1TC: u64 = 0x110;
const SAR_DEBUG_CONF: u64 = 0x114;
/// `SENS_SAR_SARDATE` (0x1FC) — version stamp, last architected register.
const SAR_SARDATE: u64 = 0x1FC;

/// `SENS_MEAS*_START_SAR` (bitpos 17) — software oneshot launch.
const MEAS_START_SAR: u32 = 1 << 17;
/// `SENS_MEAS*_DONE_SAR` (bitpos 16, RO) — conversion-complete flag.
const MEAS_DONE_SAR: u32 = 1 << 16;
/// `SENS_MEAS*_DATA_SAR` (bits [15:0], RO) — conversion result.
const MEAS_DATA_MASK: u32 = 0x0000_FFFF;
/// Fixed plausible reading latched on an oneshot when no analog source is
/// injected on the selected channel: mid-scale of the 12-bit SAR range.
/// Deterministic — LabWired runs must be reproducible.
const MEAS_DATA_FIXED: u32 = 0x800;
/// Full-scale code of the S3's 12-bit RTC SAR.
const FULL_SCALE_12: u32 = 4095;
/// The S2's RTC SAR is 13-bit (`SOC_ADC_RTC_MAX_BITWIDTH` = 13): full scale
/// 8191, mid-scale 0x1000.
const FULL_SCALE_13: u32 = 8191;
const MEAS_DATA_FIXED_13: u32 = 0x1000;
/// `SENS_SAR1_EN_PAD` (bits [30:19] of `SAR_MEAS1_CTRL2`) — the 12-bit channel
/// enable mask esp-hal sets to `1 << channel` before launching a oneshot.
const SAR1_EN_PAD_S: u32 = 19;
const SAR1_EN_PAD_M: u32 = 0x0FFF;

/// One word past the last architected register (`SAR_SARDATE` @ 0x1FC).
const NWORDS: usize = 0x200 / 4;

/// `(reset value, writable-bit mask)` for one architected register, or `None`
/// for a hole in the register map (reads 0, ignores writes).
type RegSpec = fn(usize) -> Option<(u32, u32)>;

/// `(reset value, writable-bit mask)` for the architected register at word
/// index `word` (offset `word * 4`), exactly per the ESP32-S3 SVD `SENS`
/// block; `None` = hole in the register map (reads 0, ignores writes).
/// `wmask == 0` = read-only register (writes ignored, reset value sticks).
const fn spec(word: usize) -> Option<(u32, u32)> {
    match (word as u64) * 4 {
        SAR_READER1_CTRL => Some((0x2004_0002, 0x37FC_00FF)),
        SAR_READER1_STATUS => Some((0x0000_0000, 0x0000_0000)), // RO
        SAR_MEAS1_CTRL1 => Some((0x0000_0000, 0xFF00_0000)),
        SAR_MEAS1_CTRL2 => Some((0x0000_0000, 0xFFFE_0000)), // [16:0] RO
        SAR_MEAS1_MUX => Some((0x0000_0000, 0x8000_0000)),
        SAR_ATTEN1 => Some((0xFFFF_FFFF, 0xFFFF_FFFF)),
        SAR_AMP_CTRL1 => Some((0x000A_000A, 0xFFFF_FFFF)),
        SAR_AMP_CTRL2 => Some((0x000A_0000, 0xFFFF_007F)),
        SAR_AMP_CTRL3 => Some((0x0073_38F3, 0x0FFF_FFFF)),
        SAR_READER2_CTRL => Some((0x4005_0002, 0x67FF_00FF)),
        SAR_READER2_STATUS => Some((0x0000_0000, 0x0000_0000)), // RO
        SAR_MEAS2_CTRL1 => Some((0x0702_0200, 0xFFFF_FFF8)),
        SAR_MEAS2_CTRL2 => Some((0x0000_0000, 0xFFFE_0000)), // [16:0] RO
        SAR_MEAS2_MUX => Some((0x0000_0000, 0xF000_0000)),
        SAR_ATTEN2 => Some((0xFFFF_FFFF, 0xFFFF_FFFF)),
        SAR_POWER_XPD_SAR => Some((0x0000_0000, 0xE000_0000)), // MEAS_STATUS RO
        SAR_SLAVE_ADDR1..=SAR_SLAVE_ADDR4 => Some((0x0000_0000, 0x003F_FFFF)),
        SAR_TSENS_CTRL => Some((0x0001_9000, 0x01FF_F000)), // TSENS_OUT RO
        SAR_TSENS_CTRL2 => Some((0x0000_4002, 0x0000_7FFF)),
        SAR_I2C_CTRL => Some((0x0000_0000, 0x3FFF_FFFF)),
        SAR_TOUCH_CONF => Some((0xFFF0_7FFF, 0xFFF3_FFFF)),
        SAR_TOUCH_DENOISE => Some((0x0000_0000, 0x0000_0000)), // RO per SVD
        SAR_TOUCH_THRES1..=SAR_TOUCH_THRES14 => Some((0x0000_0000, 0x003F_FFFF)),
        SAR_TOUCH_CHN_ST => Some((0x0000_0000, 0x3FFF_8000)),
        SAR_TOUCH_STATUS0..=SAR_TOUCH_STATUS16 => Some((0x0000_0000, 0x0000_0000)), // RO
        SAR_COCPU_STATE => Some((0x0000_0000, 0x0200_0000)),
        SAR_COCPU_INT_RAW => Some((0x0000_0000, 0x0000_0000)), // RO
        SAR_COCPU_INT_ENA => Some((0x0000_0000, 0x0000_0FFF)),
        SAR_COCPU_INT_ST => Some((0x0000_0000, 0x0000_0000)), // RO
        SAR_COCPU_INT_CLR => Some((0x0000_0000, 0x0000_0FFF)),
        SAR_COCPU_DEBUG => Some((0x0000_0000, 0x0000_0000)), // RO
        SAR_HALL_CTRL => Some((0xA000_0000, 0xF000_0000)),
        SAR_NOUSE => Some((0x0000_0000, 0xFFFF_FFFF)),
        SAR_PERI_CLK_GATE_CONF => Some((0x0000_0000, 0xE800_0000)),
        SAR_PERI_RESET_CONF => Some((0x0000_0000, 0x6A00_0000)),
        SAR_COCPU_INT_ENA_W1TS => Some((0x0000_0000, 0x0000_0FFF)),
        SAR_COCPU_INT_ENA_W1TC => Some((0x0000_0000, 0x0000_0FFF)),
        SAR_DEBUG_CONF => Some((0x0000_0000, 0x0000_001F)),
        SAR_SARDATE => Some((0x0210_1180, 0x0FFF_FFFF)),
        _ => None,
    }
}

/// The ESP32-S2 SENS map, per the ESP32-S2 SVD `SENS` block. 0x000..0x058
/// carries the S3's offsets; the S2 differs there only in `SAR_MEAS1_CTRL1`
/// (writable from bit 22) and `SAR_TSENS_CTRL2` (17 writable bits).
const fn spec_s2(word: usize) -> Option<(u32, u32)> {
    match (word as u64) * 4 {
        SAR_MEAS1_CTRL1 => Some((0x0000_0000, 0xFFC0_0000)),
        SAR_TSENS_CTRL2 => Some((0x0000_4002, 0x0001_FFFF)),
        off @ SAR_READER1_CTRL..=SAR_TOUCH_CONF => spec((off / 4) as usize),
        0x060..=0x094 => Some((0x0000_0000, 0x003F_FFFF)), // TOUCH_THRES1..14
        0x0D4 => Some((0x0000_0000, 0x3FFF_8000)),         // TOUCH_CHN_ST
        0x0D8..=0x118 => Some((0x0000_0000, 0x0000_0000)), // TOUCH_STATUS* (RO)
        0x11C => Some((0x0000_0000, 0x0FFF_FFFF)),         // DAC_CTRL1
        0x120 => Some((0x0300_0000, 0x03FF_FFFF)),         // DAC_CTRL2
        0x124 => Some((0x0000_0000, 0x0200_0000)),         // COCPU_STATE
        0x128 => Some((0x0000_0000, 0x0000_0000)),         // COCPU_INT_RAW (RO)
        0x12C => Some((0x0000_0000, 0x0000_01FF)),         // COCPU_INT_ENA
        0x130 => Some((0x0000_0000, 0x0000_0000)),         // COCPU_INT_ST (RO)
        0x134 => Some((0x0000_0000, 0x0000_01FF)),         // COCPU_INT_CLR
        0x138 => Some((0x0000_0000, 0x0000_0000)),         // COCPU_DEBUG (RO)
        0x13C => Some((0xA000_0000, 0xF000_0000)),         // HALL_CTRL
        0x140 => Some((0x0000_0000, 0xFFFF_FFFF)),         // NOUSE
        0x144 => Some((0x0000_0000, 0xC000_0000)),         // IO_MUX_CONF
        0x148 => Some((0x0190_6140, 0x0FFF_FFFF)),         // SARDATE
        _ => None,
    }
}

pub struct Esp32s3Sens {
    /// Register file for the architected map (word-indexed; holes stay 0 and
    /// are never read back — `spec()` gates both directions).
    regs: [u32; NWORDS],
    /// Per-ADC1-channel injected 12-bit counts (CH0..9 = GPIO1..10). `0xFFFF` =
    /// "no injection" → fall back to `MEAS_DATA_FIXED`. A potentiometer/analog
    /// source writes its wiper here so `analogRead()` returns a controllable,
    /// position-driven value instead of the fixed mid-scale placeholder. This is
    /// the SENS path esp-hal's ESP32-S3 ADC oneshot actually reads.
    channel_inputs: [u16; 10],
    /// Register map: [`spec`] (S3) or [`spec_s2`] (S2).
    spec: RegSpec,
    /// Full-scale code (S3 12-bit 4095, S2 13-bit 8191) and the mid-scale
    /// placeholder latched when nothing is injected.
    full_scale: u32,
    fixed: u32,
}

impl std::fmt::Debug for Esp32s3Sens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3Sens(meas1_ctrl2=0x{:08x}, meas2_ctrl2=0x{:08x})",
            self.reg(SAR_MEAS1_CTRL2),
            self.reg(SAR_MEAS2_CTRL2),
        )
    }
}

impl Default for Esp32s3Sens {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32s3Sens {
    pub fn new() -> Self {
        Self::with_spec(spec, FULL_SCALE_12, MEAS_DATA_FIXED)
    }

    /// The ESP32-S2 SENS block at `0x3F40_8800` (see the module header):
    /// the S2 register map and 13-bit conversion codes.
    pub fn new_esp32s2() -> Self {
        Self::with_spec(spec_s2, FULL_SCALE_13, MEAS_DATA_FIXED_13)
    }

    fn with_spec(spec: RegSpec, full_scale: u32, fixed: u32) -> Self {
        let mut regs = [0u32; NWORDS];
        for (w, r) in regs.iter_mut().enumerate() {
            if let Some((reset, _)) = spec(w) {
                *r = reset;
            }
        }
        Self {
            regs,
            channel_inputs: [0xFFFF; 10],
            spec,
            full_scale,
            fixed,
        }
    }

    /// Inject a millivolt reading for an ADC1 channel (CH0..9 = GPIO1..10). The
    /// next SENS oneshot that selects this channel returns the equivalent count
    /// (3.3 V Vref; 12-bit on the S3, 13-bit on the S2) instead of the fixed
    /// mid-scale placeholder.
    /// Read back the injected 12-bit count for a channel (`0xFFFF` = nothing
    /// injected). The read-back counterpart of [`Self::set_channel_input`], so
    /// a stimulus test can assert what the next conversion will return without
    /// running one.
    pub fn channel_input_count(&self, channel: u8) -> u16 {
        self.channel_inputs
            .get(channel as usize)
            .copied()
            .unwrap_or(0xFFFF)
    }

    pub fn set_channel_input(&mut self, channel: u8, millivolts: u16) {
        if (channel as usize) < self.channel_inputs.len() {
            let count = ((millivolts as u32 * self.full_scale) / 3300).min(self.full_scale) as u16;
            self.channel_inputs[channel as usize] = count;
        }
    }

    fn reg(&self, off: u64) -> u32 {
        let w = (off / 4) as usize;
        if w < NWORDS && (self.spec)(w).is_some() {
            self.regs[w]
        } else {
            0
        }
    }

    /// Masked store into an architected register; no-op on holes and on
    /// fully read-only registers (`wmask == 0`).
    fn set_reg_masked(&mut self, off: u64, value: u32) {
        let w = (off / 4) as usize;
        if w < NWORDS {
            if let Some((_, wmask)) = (self.spec)(w) {
                self.regs[w] = (self.regs[w] & !wmask) | (value & wmask);
            }
        }
    }

    /// Raw store (full word) — used by the oneshot FSM to update the RO
    /// DONE/DATA bits after the write mask has already been applied.
    fn set_reg_raw(&mut self, off: u64, value: u32) {
        let w = (off / 4) as usize;
        if w < NWORDS {
            self.regs[w] = value;
        }
    }

    /// SAR oneshot FSM on `SAR_MEAS1_CTRL2` / `SAR_MEAS2_CTRL2`: the masked
    /// store keeps the RO DONE/DATA bits, then `START_SAR` written 1 →
    /// conversion completes immediately (DONE set, fixed mid-scale DATA);
    /// `START_SAR` written 0 → DONE clears, last DATA retained.
    fn write_meas_ctrl2(&mut self, off: u64, value: u32) {
        self.set_reg_masked(off, value);
        let stored = self.reg(off);
        let updated = if value & MEAS_START_SAR != 0 {
            (stored & !MEAS_DATA_MASK) | MEAS_DONE_SAR | self.meas_data(off, stored)
        } else {
            stored & !MEAS_DONE_SAR
        };
        self.set_reg_raw(off, updated);
    }

    /// The 12-bit result to latch for a oneshot on `off`: the injected value for
    /// the selected ADC1 channel if one is set, else the fixed mid-scale sample.
    /// Only ADC1 (`SAR_MEAS1_CTRL2`) carries per-channel injection today; ADC2
    /// keeps the fixed placeholder.
    fn meas_data(&self, off: u64, stored: u32) -> u32 {
        if off != SAR_MEAS1_CTRL2 {
            return self.fixed;
        }
        let en_pad = (stored >> SAR1_EN_PAD_S) & SAR1_EN_PAD_M;
        if en_pad == 0 {
            return self.fixed;
        }
        let channel = en_pad.trailing_zeros() as usize;
        match self.channel_inputs.get(channel) {
            Some(&c) if c != 0xFFFF => (c as u32) & MEAS_DATA_MASK,
            _ => self.fixed,
        }
    }
}

impl Peripheral for Esp32s3Sens {
    /// SAR ADC1 CH0..CH9 (GPIO1..GPIO10), the slots `channel_inputs` holds.
    fn adc_channel_count(&self) -> Option<u8> {
        Some(10)
    }

    // Inert walk: SENS register bank; no free-running state. tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3)?;
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.reg(offset & !3))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let shift = (offset & 3) * 8;
        // Read-modify-write the affected word, then re-dispatch through the
        // u32 path so the oneshot side-effects fire consistently.
        let base = self.reg(word_off);
        let merged = (base & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_u32(word_off, merged)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            o @ (SAR_MEAS1_CTRL2 | SAR_MEAS2_CTRL2) => self.write_meas_ctrl2(o, value),
            // Everything else: masked store into the architected register;
            // RO registers and holes ignore writes entirely.
            o => self.set_reg_masked(o, value),
        }
        Ok(())
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

    #[test]
    fn reset_defaults_seeded() {
        let s = Esp32s3Sens::new();
        assert_eq!(s.read_u32(SAR_READER1_CTRL).unwrap(), 0x2004_0002);
        assert_eq!(s.read_u32(SAR_ATTEN1).unwrap(), 0xFFFF_FFFF);
        assert_eq!(s.read_u32(SAR_AMP_CTRL1).unwrap(), 0x000A_000A);
        assert_eq!(s.read_u32(SAR_AMP_CTRL3).unwrap(), 0x0073_38F3);
        assert_eq!(s.read_u32(SAR_READER2_CTRL).unwrap(), 0x4005_0002);
        assert_eq!(s.read_u32(SAR_MEAS2_CTRL1).unwrap(), 0x0702_0200);
        assert_eq!(s.read_u32(SAR_ATTEN2).unwrap(), 0xFFFF_FFFF);
        assert_eq!(s.read_u32(SAR_TSENS_CTRL).unwrap(), 0x0001_9000);
        assert_eq!(s.read_u32(SAR_TSENS_CTRL2).unwrap(), 0x0000_4002);
        assert_eq!(s.read_u32(SAR_TOUCH_CONF).unwrap(), 0xFFF0_7FFF);
        assert_eq!(s.read_u32(SAR_HALL_CTRL).unwrap(), 0xA000_0000);
        assert_eq!(s.read_u32(SAR_SARDATE).unwrap(), 0x0210_1180);
    }

    #[test]
    fn config_registers_store_under_write_mask() {
        let mut s = Esp32s3Sens::new();
        // SAR_ATTEN1 is fully writable.
        s.write_u32(SAR_ATTEN1, 0x1234_5678).unwrap();
        assert_eq!(s.read_u32(SAR_ATTEN1).unwrap(), 0x1234_5678);
        // SAR_MEAS1_MUX: only bit 31 is writable; the rest keeps reset (0).
        s.write_u32(SAR_MEAS1_MUX, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(SAR_MEAS1_MUX).unwrap(), 0x8000_0000);
        // SAR_TSENS_CTRL: TSENS_OUT [11:0] is RO — those bits keep reset 0
        // while the writable [24:12] window stores.
        s.write_u32(SAR_TSENS_CTRL, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(SAR_TSENS_CTRL).unwrap(), 0x01FF_F000);
        // SAR_SLAVE_ADDR1: [21:0] writable, MEAS_STATUS [29:22] RO stays 0 —
        // the adc_oneshot_ll_start busy-poll on it must exit immediately.
        s.write_u32(SAR_SLAVE_ADDR1, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(SAR_SLAVE_ADDR1).unwrap(), 0x003F_FFFF);
        // SAR_TOUCH_THRES range shares one mask.
        s.write_u32(SAR_TOUCH_THRES14, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(SAR_TOUCH_THRES14).unwrap(), 0x003F_FFFF);
    }

    #[test]
    fn read_only_registers_ignore_writes() {
        let mut s = Esp32s3Sens::new();
        for off in [
            SAR_READER1_STATUS,
            SAR_READER2_STATUS,
            SAR_TOUCH_DENOISE,
            SAR_TOUCH_STATUS0,
            SAR_TOUCH_STATUS16,
            SAR_COCPU_INT_RAW,
            SAR_COCPU_INT_ST,
            SAR_COCPU_DEBUG,
        ] {
            s.write_u32(off, 0xFFFF_FFFF).unwrap();
            assert_eq!(s.read_u32(off).unwrap(), 0, "RO reg at {off:#x}");
        }
    }

    #[test]
    fn unmapped_offsets_read_zero_and_ignore_writes() {
        let mut s = Esp32s3Sens::new();
        // The 0x118..0x1FC hole and offsets at/above 0x200 must NOT
        // round-trip — the coverage probe's baseline depends on it.
        for off in [0x118u64, 0x150, 0x1F8, 0x200, 0x3FC] {
            s.write_u32(off, 0xDEAD_BEEF).unwrap();
            assert_eq!(s.read_u32(off).unwrap(), 0, "hole at {off:#x}");
        }
    }

    #[test]
    fn sar1_oneshot_start_sets_done_and_data() {
        let mut s = Esp32s3Sens::new();
        // adc_oneshot_ll_start(ADC_UNIT_1): start_sar = 0, then 1.
        s.write_u32(SAR_MEAS1_CTRL2, 0).unwrap();
        assert_eq!(
            s.read_u32(SAR_MEAS1_CTRL2).unwrap() & MEAS_DONE_SAR,
            0,
            "DONE clear while idle"
        );
        s.write_u32(SAR_MEAS1_CTRL2, MEAS_START_SAR).unwrap();
        let v = s.read_u32(SAR_MEAS1_CTRL2).unwrap();
        assert_eq!(v & MEAS_DONE_SAR, MEAS_DONE_SAR, "DONE set after start");
        assert_eq!(v & MEAS_DATA_MASK, MEAS_DATA_FIXED, "mid-scale reading");
        assert_eq!(v & MEAS_START_SAR, MEAS_START_SAR, "START retains (R/W)");
        // Restart: start=0 clears DONE (data retained), start=1 completes again.
        s.write_u32(SAR_MEAS1_CTRL2, 0).unwrap();
        let v = s.read_u32(SAR_MEAS1_CTRL2).unwrap();
        assert_eq!(v & MEAS_DONE_SAR, 0, "DONE clears on start=0");
        assert_eq!(v & MEAS_DATA_MASK, MEAS_DATA_FIXED, "DATA retained");
        s.write_u32(SAR_MEAS1_CTRL2, MEAS_START_SAR).unwrap();
        assert_eq!(
            s.read_u32(SAR_MEAS1_CTRL2).unwrap() & MEAS_DONE_SAR,
            MEAS_DONE_SAR
        );
    }

    #[test]
    fn sar2_oneshot_start_sets_done_and_data() {
        let mut s = Esp32s3Sens::new();
        s.write_u32(SAR_MEAS2_CTRL2, 0).unwrap();
        s.write_u32(SAR_MEAS2_CTRL2, MEAS_START_SAR).unwrap();
        let v = s.read_u32(SAR_MEAS2_CTRL2).unwrap();
        assert_eq!(v & MEAS_DONE_SAR, MEAS_DONE_SAR, "SAR2 DONE set");
        assert_eq!(v & MEAS_DATA_MASK, MEAS_DATA_FIXED, "SAR2 mid-scale data");
        // The done/data of SAR1 are untouched.
        assert_eq!(s.read_u32(SAR_MEAS1_CTRL2).unwrap(), 0);
    }

    #[test]
    fn meas_ctrl2_data_and_done_are_read_only_to_software() {
        let mut s = Esp32s3Sens::new();
        // A direct write cannot forge DONE or DATA ([16:0] are outside the
        // write mask): only the FSM sets them.
        s.write_u32(SAR_MEAS1_CTRL2, MEAS_DONE_SAR | 0xFFF).unwrap();
        assert_eq!(s.read_u32(SAR_MEAS1_CTRL2).unwrap(), 0);
        // Upper config bits ([31:19]) store normally without firing the FSM.
        s.write_u32(SAR_MEAS1_CTRL2, 0xFFF8_0000).unwrap();
        let v = s.read_u32(SAR_MEAS1_CTRL2).unwrap();
        assert_eq!(v, 0xFFF8_0000, "config bits store, DONE/DATA stay 0");
    }

    #[test]
    fn esp32s2_map_resets_and_masks_follow_its_svd() {
        let mut s = Esp32s3Sens::new_esp32s2();
        assert_eq!(s.read_u32(SAR_READER1_CTRL).unwrap(), 0x2004_0002);
        assert_eq!(s.read_u32(SAR_MEAS2_CTRL1).unwrap(), 0x0702_0200);
        assert_eq!(s.read_u32(0x120).unwrap(), 0x0300_0000, "DAC_CTRL2");
        assert_eq!(s.read_u32(0x13C).unwrap(), 0xA000_0000, "HALL_CTRL");
        assert_eq!(s.read_u32(0x148).unwrap(), 0x0190_6140, "SARDATE");
        // The S3's SARDATE offset is a hole on the S2, and vice versa.
        assert_eq!(s.read_u32(SAR_SARDATE).unwrap(), 0);
        assert_eq!(Esp32s3Sens::new().read_u32(0x148).unwrap(), 0);
        // S2-specific masks inside the shared range.
        s.write_u32(SAR_MEAS1_CTRL1, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(SAR_MEAS1_CTRL1).unwrap(), 0xFFC0_0000);
        s.write_u32(SAR_TSENS_CTRL2, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(SAR_TSENS_CTRL2).unwrap(), 0x0001_FFFF);
        // MEAS_STATUS [29:22] stays 0, so adc_oneshot_ll_start's poll exits.
        s.write_u32(SAR_SLAVE_ADDR1, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(SAR_SLAVE_ADDR1).unwrap(), 0x003F_FFFF);
    }

    #[test]
    fn esp32s2_oneshot_completes_like_the_s3() {
        let mut s = Esp32s3Sens::new_esp32s2();
        s.write_u32(SAR_MEAS1_CTRL2, 1 << (SAR1_EN_PAD_S + 3))
            .unwrap();
        s.write_u32(SAR_MEAS1_CTRL2, (1 << (SAR1_EN_PAD_S + 3)) | MEAS_START_SAR)
            .unwrap();
        let v = s.read_u32(SAR_MEAS1_CTRL2).unwrap();
        assert_eq!(v & MEAS_DONE_SAR, MEAS_DONE_SAR);
        assert_eq!(v & MEAS_DATA_MASK, MEAS_DATA_FIXED_13, "13-bit mid-scale");
        s.set_channel_input(3, 1650);
        s.write_u32(SAR_MEAS1_CTRL2, 1 << (SAR1_EN_PAD_S + 3))
            .unwrap();
        s.write_u32(SAR_MEAS1_CTRL2, (1 << (SAR1_EN_PAD_S + 3)) | MEAS_START_SAR)
            .unwrap();
        // 1650 mV on the 13-bit scale: 1650 * 8191 / 3300 = 4095.
        assert_eq!(s.read_u32(SAR_MEAS1_CTRL2).unwrap() & MEAS_DATA_MASK, 4095);
        assert_eq!(s.channel_input_count(3), 4095);
        // Full scale clamps at 8191, not the S3's 4095.
        s.set_channel_input(3, 3300);
        assert_eq!(s.channel_input_count(3), 8191);
    }

    #[test]
    fn byte_writes_merge_and_fire_oneshot() {
        let mut s = Esp32s3Sens::new();
        // Byte 2 of MEAS2_CTRL2 carries START_SAR (bit 17 = bit 1 of byte 2).
        s.write(SAR_MEAS2_CTRL2 + 2, (MEAS_START_SAR >> 16) as u8)
            .unwrap();
        let v = s.read_u32(SAR_MEAS2_CTRL2).unwrap();
        assert_eq!(v & MEAS_DONE_SAR, MEAS_DONE_SAR, "byte path fires FSM");
        assert_eq!(v & MEAS_DATA_MASK, MEAS_DATA_FIXED);
        // Byte reads see the same word.
        assert_eq!(s.read(SAR_MEAS2_CTRL2).unwrap(), 0x00);
        assert_eq!(s.read(SAR_MEAS2_CTRL2 + 1).unwrap(), 0x08);
    }
}
