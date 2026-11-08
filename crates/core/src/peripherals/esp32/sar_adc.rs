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
//!
//! ## Touch sensor (TRM §30, `soc/esp32/sens_reg.h`, `hal/touch_sensor_ll.h`)
//!
//! The ten capacitive touch channels also live in SENS. Arduino `touchRead()`
//! (IDF `touch_pad_read_raw_data`) returns the IIR filter's raw slot, which an
//! esp_timer callback fills by busy-waiting on `TOUCH_MEAS_DONE` and then
//! reading `SAR_TOUCH_OUTn`. Before this model the SENS window was plain
//! storage there: `TOUCH_MEAS_DONE` never rose and the first callback hung.
//!
//! * `SAR_TOUCH_CTRL1` (0x58) `TOUCH_MEAS_DELAY` [15:0] is the measurement
//!   window in 8 MHz RTC_FAST cycles; `TOUCH_OUT_SEL` [24] picks the trigger
//!   direction (0 = below threshold).
//! * `SAR_TOUCH_THRES1..5` (0x5C..0x6C) and `SAR_TOUCH_OUT1..5` (0x70..0x80)
//!   pack two channels per word: the even hardware channel in [31:16], the
//!   odd one in [15:0].
//! * `SAR_TOUCH_CTRL2` (0x84): `TOUCH_MEAS_DONE` [10] (RO) and
//!   `TOUCH_MEAS_EN` [9:0] (RO, per-channel trigger status).
//! * `SAR_TOUCH_ENABLE` (0x8C) `TOUCH_PAD_WORKEN` [9:0] enables a channel.
//!
//! Hardware channels 8 and 9 are swapped against the software pad numbers
//! (`touch_ll_num_wrap`): Arduino `T8` is hardware channel 9. The stimulus
//! channels `t0`..`t9` use the software (Arduino) numbering.
//!
//! The count is the number of charge/discharge cycles in the window,
//! `t_meas * I / (2 * C * dV)`. The charge slope and the DREFH/DREFL rails are
//! RTC_IO fields, outside this window, so the model uses the IDF defaults
//! (slope 7, 2.7 V / 0.5 V). The charge current [`TOUCH_CHARGE_A`] is NOT a
//! TRM figure: it is a model constant chosen so a bare 20 pF pad reads ~70 at
//! Arduino's 0x1000-cycle window and a finger (+100 pF) reads ~12. The point is
//! the direction and the dependence on capacitance and window, not
//! silicon-exact counts. The touch interrupt (RTC_CNTL `rtc_touch`) is not
//! raised, so `touchAttachInterrupt()` does not fire.
//!
//! ## DAC (`soc/esp32/sens_reg.h`, `hal/dac_ll.h`)
//!
//! The classic DACs are split across two blocks. Their power and 8-bit code
//! sit in RTC_IO `PAD_DAC1/2` (`peripherals::esp_rtc_io`), whose pad-stage
//! level the bus hands this block as ADC input probes 8 / 9
//! (`SystemBus::wire_esp32_dac_adc`). Which source feeds the DAC is muxed
//! here: `SAR_DAC_CTRL2` (0x9C) `DAC_CW_EN1` [24] / `DAC_CW_EN2` [25] (reset
//! 1) pick the cosine generator instead of the `PDACn_DAC` code. IDF
//! `dac_ll_update_output_value` (Arduino `dacWrite()`) clears the bit. The
//! cosine generator is not modelled, so with `CW_ENn` set the DAC reads as
//! not driven.
//!
//! The muxed DAC1 / DAC2 level is this block's analog output 0 / 1
//! ([`Peripheral::analog_output_probe`]), so a board jumper names it as
//! `sens_sar_adc:0` / `sens_sar_adc:1`. It is read in two places:
//!
//! * ADC2_CH8 / ADC2_CH9 are GPIO25 / GPIO26, the DAC pads themselves, so a
//!   SAR2 conversion of a pad the DAC drives reads what it drives, ahead of
//!   any injected level. Stock Arduino `analogRead(25)` never sees this,
//!   because `__adcAttachPin` powers the DAC on that pin down first, as it
//!   does on silicon.
//! * ADC1_CH0..7 take a board wire (`analog_links: { <ch>: "<peripheral>:<output>" }`,
//!   bound by [`Peripheral::bind_adc_channel_probe`]): a jumper from GPIO25
//!   to GPIO34 is `6: "sens_sar_adc:0"`. A wired SAR1 channel reads the source
//!   while it drives, and the injected level or the ramp while it does not.

use crate::sim_input::{InputChannel, SimInput, SimInputError};
use crate::{AnalogProbe, Peripheral, PeripheralTickResult, SimResult};
use std::borrow::Cow;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

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
/// tracks the SELECTED channel, not a constant. Used when no external analog
/// source has injected a level via [`Esp32SarAdc::set_channel_input`].
fn channel_sample(channel: u32) -> u32 {
    (0x100 + channel * 0x111) & 0x0FFF
}

const NO_INJECT: u16 = 0xFFFF;
const CHANNEL_SLOTS: usize = 16;

/// `SAR_DAC_CTRL2`: `DAC_CW_EN1` [24] / `DAC_CW_EN2` [25], reset 1 each.
const SAR_DAC_CTRL2: u64 = 0x9C;
const DAC_CW_EN: [u32; 2] = [1 << 24, 1 << 25];
/// The SAR2 channels on the DAC1 / DAC2 pads (GPIO25 / GPIO26), and the ADC
/// input probes the bus binds the DAC pad stages to.
const SAR2_DAC_CHANNELS: [u8; 2] = [8, 9];
/// SAR1 (ADC1) has CH0..CH7.
const SAR1_CHANNELS: usize = 8;

const SAR_TOUCH_CTRL1: u64 = 0x58;
const SAR_TOUCH_THRES1: u64 = 0x5C;
const SAR_TOUCH_OUT1: u64 = 0x70;
const SAR_TOUCH_OUT5: u64 = 0x80;
const SAR_TOUCH_CTRL2: u64 = 0x84;
const SAR_TOUCH_ENABLE: u64 = 0x8C;

/// `TOUCH_MEAS_DELAY` [15:0] of `SAR_TOUCH_CTRL1`, in 8 MHz cycles.
const TOUCH_MEAS_DELAY_MASK: u32 = 0xFFFF;
/// `TOUCH_OUT_SEL` [24] of `SAR_TOUCH_CTRL1`: 1 = trigger above threshold.
const TOUCH_OUT_SEL: u32 = 1 << 24;
/// `TOUCH_MEAS_DONE` [10] of `SAR_TOUCH_CTRL2` (RO).
const TOUCH_MEAS_DONE: u32 = 1 << 10;
/// `TOUCH_MEAS_EN` [9:0] of `SAR_TOUCH_CTRL2` (RO trigger status).
const TOUCH_MEAS_EN_MASK: u32 = 0x3FF;
/// `TOUCH_MEAS_EN_CLR` [30] of `SAR_TOUCH_CTRL2` (WO).
const TOUCH_MEAS_EN_CLR: u32 = 1 << 30;
/// `TOUCH_PAD_WORKEN` [9:0] of `SAR_TOUCH_ENABLE`.
const TOUCH_WORKEN_MASK: u32 = 0x3FF;

const TOUCH_CHANNELS: usize = 10;
const TOUCH_RTC_FAST_HZ: f64 = 8_000_000.0;
/// Model constant (see the module docs), not a TRM figure.
const TOUCH_CHARGE_A: f64 = 12e-6;
/// DREFH 2.7 V - DREFL 0.5 V, the IDF default rails.
const TOUCH_SWING_V: f64 = 2.2;
/// A bare pad with no finger on it.
const TOUCH_BARE_PAD_PF: f64 = 20.0;

/// Software pad number (Arduino `Tn`) -> hardware channel. Pads 8 and 9 are
/// swapped (`touch_ll_num_wrap`).
fn touch_hw_channel(pad: usize) -> usize {
    match pad {
        8 => 9,
        9 => 8,
        p => p,
    }
}

const fn touch_input(key: &'static str, label: &'static str) -> InputChannel {
    InputChannel {
        key: Cow::Borrowed(key),
        label: Cow::Borrowed(label),
        unit: Cow::Borrowed("pF"),
        min: 1.0,
        max: 10_000.0,
        default: Some(TOUCH_BARE_PAD_PF),
    }
}

/// Pad capacitance per software touch pad, Arduino numbering.
const TOUCH_INPUTS: &[InputChannel] = &[
    touch_input("t0", "T0 (GPIO4) capacitance"),
    touch_input("t1", "T1 (GPIO0) capacitance"),
    touch_input("t2", "T2 (GPIO2) capacitance"),
    touch_input("t3", "T3 (GPIO15) capacitance"),
    touch_input("t4", "T4 (GPIO13) capacitance"),
    touch_input("t5", "T5 (GPIO12) capacitance"),
    touch_input("t6", "T6 (GPIO14) capacitance"),
    touch_input("t7", "T7 (GPIO27) capacitance"),
    touch_input("t8", "T8 (GPIO33) capacitance"),
    touch_input("t9", "T9 (GPIO32) capacitance"),
];

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
    /// Per-channel 12-bit counts from external analog sources (kits).
    channel_inputs: [u16; CHANNEL_SLOTS],
    /// Touch pad capacitance in pF, indexed by HARDWARE channel.
    touch_pf: [f64; TOUCH_CHANNELS],
    /// DAC1 / DAC2 pad stage (RTC_IO), as bound to [`SAR2_DAC_CHANNELS`].
    dac_pads: [Option<AnalogProbe>; 2],
    /// `SAR_DAC_CTRL2`, shared with the DAC output probes this block hands out.
    dac_ctrl2: Arc<AtomicU32>,
    /// Board wires onto ADC1_CH0..7.
    sar1_links: [Option<AnalogProbe>; SAR1_CHANNELS],
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
        let dac_ctrl2 = DAC_CW_EN[0] | DAC_CW_EN[1];
        regs[(SAR_DAC_CTRL2 / 4) as usize] = dac_ctrl2;
        Self {
            regs,
            sar1: SarUnit::default(),
            sar2: SarUnit::default(),
            channel_inputs: [NO_INJECT; CHANNEL_SLOTS],
            touch_pf: [TOUCH_BARE_PAD_PF; TOUCH_CHANNELS],
            dac_pads: [None, None],
            dac_ctrl2: Arc::new(AtomicU32::new(dac_ctrl2)),
            sar1_links: Default::default(),
        }
    }

    /// Set the capacitance on software touch pad `pad` (Arduino `Tn`).
    pub fn set_touch_capacitance_pf(&mut self, pad: usize, pf: f64) {
        if pad < TOUCH_CHANNELS && pf.is_finite() && pf > 0.0 {
            self.touch_pf[touch_hw_channel(pad)] = pf;
        }
    }

    /// The count hardware channel `hw` latches into `SAR_TOUCH_OUTn`. A
    /// channel that is not in `TOUCH_PAD_WORKEN` is never measured and keeps
    /// its reset value of 0.
    fn touch_count(&self, hw: usize) -> u32 {
        if self.reg(SAR_TOUCH_ENABLE) & TOUCH_WORKEN_MASK & (1 << hw) == 0 {
            return 0;
        }
        let window_s =
            (self.reg(SAR_TOUCH_CTRL1) & TOUCH_MEAS_DELAY_MASK) as f64 / TOUCH_RTC_FAST_HZ;
        let cycle_s = 2.0 * self.touch_pf[hw] * 1e-12 * TOUCH_SWING_V / TOUCH_CHARGE_A;
        ((window_s / cycle_s).round() as u32).min(0xFFFF)
    }

    /// The 16-bit threshold field of hardware channel `hw`.
    fn touch_threshold(&self, hw: usize) -> u32 {
        let word = self.reg(SAR_TOUCH_THRES1 + (hw as u64 / 2) * 4);
        if hw % 2 == 0 {
            word >> 16
        } else {
            word & 0xFFFF
        }
    }

    /// `SAR_TOUCH_CTRL2` as firmware reads it: the stored control bits, plus
    /// `TOUCH_MEAS_DONE` once a channel is enabled, plus the per-channel
    /// trigger status. A threshold of 0 (the IDF "never touched" value) never
    /// triggers below.
    fn touch_ctrl2_read(&self) -> u32 {
        let worken = self.reg(SAR_TOUCH_ENABLE) & TOUCH_WORKEN_MASK;
        let above = self.reg(SAR_TOUCH_CTRL1) & TOUCH_OUT_SEL != 0;
        let mut status = 0;
        for hw in 0..TOUCH_CHANNELS {
            if worken & (1 << hw) == 0 {
                continue;
            }
            let (count, thresh) = (self.touch_count(hw), self.touch_threshold(hw));
            if (above && count > thresh) || (!above && count < thresh) {
                status |= 1 << hw;
            }
        }
        let done = if worken != 0 { TOUCH_MEAS_DONE } else { 0 };
        (self.reg(SAR_TOUCH_CTRL2) & !(TOUCH_MEAS_DONE | TOUCH_MEAS_EN_MASK)) | done | status
    }

    /// Inject a millivolt reading for a SAR channel. Next conversion on that
    /// channel returns the equivalent 12-bit count (3.3 V Vref). Used by
    /// [`SystemBus::seed_adc_channel`] for analog kits on classic ESP32.
    pub fn set_channel_input(&mut self, channel: u8, millivolts: u16) {
        if (channel as usize) < self.channel_inputs.len() {
            let count = ((millivolts as u32 * 4095) / 3300).min(4095) as u16;
            self.channel_inputs[channel as usize] = count;
        }
    }

    pub fn channel_input_count(&self, channel: u8) -> u16 {
        self.channel_inputs
            .get(channel as usize)
            .copied()
            .unwrap_or(NO_INJECT)
    }

    fn reg(&self, off: u64) -> u32 {
        *self.regs.get((off / 4) as usize).unwrap_or(&0)
    }

    /// DAC `dac`'s output (see the module docs): the pad stage's level unless
    /// the cosine generator owns the DAC.
    fn dac_output(ctrl2: &AtomicU32, dac: usize, pad: &AnalogProbe) -> Option<u16> {
        if ctrl2.load(Ordering::Relaxed) & DAC_CW_EN[dac] != 0 {
            return None;
        }
        pad()
    }

    /// The level a conversion of `channel` on SAR2 (`sar2`) or SAR1 reads
    /// from an analog source wired to it, while that source drives.
    fn wired_mv(&self, channel: u32, sar2: bool) -> Option<u16> {
        if sar2 {
            let dac = SAR2_DAC_CHANNELS
                .iter()
                .position(|&c| u32::from(c) == channel)?;
            Self::dac_output(&self.dac_ctrl2, dac, self.dac_pads[dac].as_ref()?)
        } else {
            self.sar1_links.get(channel as usize)?.as_ref()?()
        }
    }

    /// Run a conversion for the given `start_word` and resolution, returning the
    /// latched unit state, or `None` if no channel was selected. `sar2` picks
    /// the unit (see [`Self::wired_mv`]).
    fn convert(&self, start_word: u32, width_bits: u32, sar2: bool) -> Option<SarUnit> {
        let en_pad = (start_word >> EN_PAD_SHIFT) & EN_PAD_MASK;
        if en_pad == 0 {
            return None;
        }
        // The IDF selects exactly one channel via a one-hot EN_PAD bitmap.
        let channel = en_pad.trailing_zeros();
        let full = if let Some(mv) = self.wired_mv(channel, sar2) {
            (u32::from(mv) * 4095 / 3300).min(4095)
        } else {
            let idx = channel as usize;
            if idx < self.channel_inputs.len() && self.channel_inputs[idx] != NO_INJECT {
                self.channel_inputs[idx] as u32
            } else {
                channel_sample(channel)
            }
        };
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
    /// CH0..CH9. SAR1 has CH0..CH7 and SAR2 CH0..CH9, and this model keeps one
    /// input table for both units, so the wider unit's range is what a
    /// conversion can read.
    fn adc_channel_count(&self) -> Option<u8> {
        Some(10)
    }

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
            o @ SAR_TOUCH_OUT1..=SAR_TOUCH_OUT5 => {
                let even = ((o - SAR_TOUCH_OUT1) / 4 * 2) as usize;
                (self.touch_count(even) << 16) | self.touch_count(even + 1)
            }
            SAR_TOUCH_CTRL2 => self.touch_ctrl2_read(),
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
                    let ctrl_off = if off == SAR_MEAS_START1 {
                        SAR_READ_CTRL
                    } else {
                        SAR_READ_CTRL2
                    };
                    let bits = 9
                        + ((self.regs[(ctrl_off / 4) as usize] >> SAMPLE_BIT_SHIFT)
                            & SAMPLE_BIT_MASK);
                    if let Some(converted) = self.convert(value, bits, off == SAR_MEAS_START2) {
                        if off == SAR_MEAS_START1 {
                            self.sar1 = converted;
                        } else {
                            self.sar2 = converted;
                        }
                    }
                }
            }
            SAR_DAC_CTRL2 => {
                self.regs[(SAR_DAC_CTRL2 / 4) as usize] = value;
                self.dac_ctrl2.store(value, Ordering::Relaxed);
            }
            // The OUT words are RO counts.
            SAR_TOUCH_OUT1..=SAR_TOUCH_OUT5 => {}
            // DONE and the trigger status are RO; MEAS_EN_CLR is a strobe (the
            // status is recomputed from the counts on every read).
            SAR_TOUCH_CTRL2 => {
                self.regs[(SAR_TOUCH_CTRL2 / 4) as usize] =
                    value & !(TOUCH_MEAS_DONE | TOUCH_MEAS_EN_MASK | TOUCH_MEAS_EN_CLR);
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

    /// Inputs 0..7 are board wires onto ADC1_CH0..7; 8 / 9 are the DAC1 /
    /// DAC2 pad stages (see the module docs).
    fn bind_adc_channel_probe(&mut self, channel: u8, probe: AnalogProbe) -> bool {
        let channel = usize::from(channel);
        if let Some(link) = self.sar1_links.get_mut(channel) {
            *link = Some(probe);
        } else if let Some(dac) = SAR2_DAC_CHANNELS
            .iter()
            .position(|&c| usize::from(c) == channel)
        {
            self.dac_pads[dac] = Some(probe);
        } else {
            return false;
        }
        true
    }

    /// Outputs 0 / 1: DAC1 / DAC2 after the source mux, once the bus has
    /// bound their pad stages.
    fn analog_output_probe(&self, channel: u8) -> Option<AnalogProbe> {
        let dac = usize::from(channel);
        let pad = self.dac_pads.get(dac)?.clone()?;
        let ctrl2 = Arc::clone(&self.dac_ctrl2);
        Some(Arc::new(move || Self::dac_output(&ctrl2, dac, &pad)))
    }

    /// The touch pads are this peripheral's own inputs (`t0`..`t9`, pF).
    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn SimInput) -> bool,
    ) -> bool {
        f(self)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl SimInput for Esp32SarAdc {
    fn input_channels(&self) -> &[InputChannel] {
        TOUCH_INPUTS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        let pad = TOUCH_INPUTS
            .iter()
            .position(|c| c.key == key)
            .ok_or_else(|| SimInputError::UnknownChannel(key.to_string()))?;
        self.set_touch_capacitance_pf(pad, value);
        Ok(())
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

    fn sar2_oneshot(a: &mut Esp32SarAdc, channel: u32) -> u32 {
        let en_pad = (1u32 << channel) & EN_PAD_MASK;
        a.write_u32(
            SAR_MEAS_START2,
            (en_pad << EN_PAD_SHIFT) | (1 << 18) | MEAS_START_SAR,
        )
        .unwrap();
        a.read_u32(SAR_MEAS_START2).unwrap() & MEAS_DATA_MASK
    }

    #[test]
    fn sar2_dac_pads_read_the_dac_once_cw_is_off() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        // u32::MAX = the DAC is powered down.
        let level = Arc::new(AtomicU32::new(u32::MAX));
        let probe_level = Arc::clone(&level);
        let probe: crate::AnalogProbe = Arc::new(move || {
            let mv = probe_level.load(Ordering::Relaxed);
            (mv != u32::MAX).then_some(mv as u16)
        });
        let mut a = Esp32SarAdc::new();
        assert!(a.analog_output_probe(0).is_none(), "no pad stage bound yet");
        assert!(!a.bind_adc_channel_probe(10, Arc::clone(&probe)));
        assert!(a.bind_adc_channel_probe(8, probe));
        let dac1 = a.analog_output_probe(0).expect("DAC1 output");
        assert!(a.analog_output_probe(1).is_none());
        // CW_EN1/2 reset to 1.
        assert_eq!(a.read_u32(SAR_DAC_CTRL2).unwrap(), 0x0300_0000);
        level.store(1650, Ordering::Relaxed);
        // The cosine generator owns the DAC: not modelled, the old ramp.
        assert_eq!(sar2_oneshot(&mut a, 8), channel_sample(8));
        assert_eq!(dac1(), None);
        // dac_ll_update_output_value clears CW_EN1.
        a.write_u32(SAR_DAC_CTRL2, DAC_CW_EN[1]).unwrap();
        assert_eq!(sar2_oneshot(&mut a, 8), 1650 * 4095 / 3300);
        assert_eq!(dac1(), Some(1650));
        level.store(3300, Ordering::Relaxed);
        assert_eq!(sar2_oneshot(&mut a, 8), 4095);
        // The DAC drives the pad over an injected kit level.
        a.set_channel_input(8, 100);
        assert_eq!(sar2_oneshot(&mut a, 8), 4095);
        // Powered down: the injected level again.
        level.store(u32::MAX, Ordering::Relaxed);
        assert_eq!(sar2_oneshot(&mut a, 8), 100 * 4095 / 3300);
        // SAR1 has no channel on the DAC pads.
        level.store(3300, Ordering::Relaxed);
        a.write_u32(SAR_MEAS_START1, sar1_oneshot(8)).unwrap();
        assert_eq!(
            a.read_u32(SAR_MEAS_START1).unwrap() & MEAS_DATA_MASK,
            100 * 4095 / 3300
        );
        // CH9 has no DAC bound here: the ramp.
        assert_eq!(sar2_oneshot(&mut a, 9), channel_sample(9));
    }

    #[test]
    fn a_jumper_from_the_dac_reaches_an_adc1_channel() {
        use std::sync::Arc;
        let pad: AnalogProbe = Arc::new(|| Some(1000));
        let mut a = Esp32SarAdc::new();
        assert!(a.bind_adc_channel_probe(8, pad));
        // GPIO25 -> GPIO34 (ADC1_CH6), as `analog_links: { 6: "sens_sar_adc:0" }`.
        let dac1 = a.analog_output_probe(0).unwrap();
        assert!(a.bind_adc_channel_probe(6, dac1));
        a.write_u32(SAR_MEAS_START1, sar1_oneshot(6)).unwrap();
        // CW_EN1 still at reset: the DAC is the cosine generator's, not driven.
        assert_eq!(
            a.read_u32(SAR_MEAS_START1).unwrap() & MEAS_DATA_MASK,
            channel_sample(6)
        );
        a.write_u32(SAR_DAC_CTRL2, 0).unwrap();
        a.write_u32(SAR_MEAS_START1, sar1_oneshot(6)).unwrap();
        assert_eq!(
            a.read_u32(SAR_MEAS_START1).unwrap() & MEAS_DATA_MASK,
            1000 * 4095 / 3300
        );
        // A neighbouring channel is not wired.
        a.write_u32(SAR_MEAS_START1, sar1_oneshot(7)).unwrap();
        assert_eq!(
            a.read_u32(SAR_MEAS_START1).unwrap() & MEAS_DATA_MASK,
            channel_sample(7)
        );
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

    /// What the IDF does for `touch_pad_config(pad)` in timer mode, with
    /// Arduino's 0x1000-cycle window.
    fn enable_touch(a: &mut Esp32SarAdc, hw: u32) {
        a.write_u32(SAR_TOUCH_CTRL1, 0x1000).unwrap();
        let en = a.read_u32(SAR_TOUCH_ENABLE).unwrap();
        a.write_u32(SAR_TOUCH_ENABLE, en | (1 << hw)).unwrap();
    }

    #[test]
    fn touch_done_and_counts_stay_zero_until_a_pad_is_enabled() {
        let mut a = Esp32SarAdc::new();
        a.write_u32(SAR_TOUCH_CTRL1, 0x1000).unwrap();
        assert_eq!(a.read_u32(SAR_TOUCH_CTRL2).unwrap() & TOUCH_MEAS_DONE, 0);
        assert_eq!(a.read_u32(SAR_TOUCH_OUT1).unwrap(), 0);
    }

    #[test]
    fn touch_count_drops_when_a_finger_adds_capacitance() {
        let mut a = Esp32SarAdc::new();
        enable_touch(&mut a, 0);
        assert_ne!(a.read_u32(SAR_TOUCH_CTRL2).unwrap() & TOUCH_MEAS_DONE, 0);
        // T0 is the even channel of OUT1: bits [31:16].
        let bare = a.read_u32(SAR_TOUCH_OUT1).unwrap() >> 16;
        assert_eq!(bare, 70, "20 pF bare pad at a 0x1000-cycle window");
        a.set_input("t0", 120.0).unwrap();
        let touched = a.read_u32(SAR_TOUCH_OUT1).unwrap() >> 16;
        assert_eq!(touched, 12, "finger on T0");
        // T1 (odd, [15:0]) is not enabled and stays 0.
        assert_eq!(a.read_u32(SAR_TOUCH_OUT1).unwrap() & 0xFFFF, 0);
    }

    #[test]
    fn touch_count_scales_with_the_measurement_window() {
        let mut a = Esp32SarAdc::new();
        enable_touch(&mut a, 3);
        let short = a.read_u32(SAR_TOUCH_OUT1 + 4).unwrap() & 0xFFFF;
        a.write_u32(SAR_TOUCH_CTRL1, 0x2000).unwrap();
        let long = a.read_u32(SAR_TOUCH_OUT1 + 4).unwrap() & 0xFFFF;
        assert_eq!(long, short * 2);
    }

    #[test]
    fn touch_pads_8_and_9_are_swapped_in_hardware() {
        let mut a = Esp32SarAdc::new();
        enable_touch(&mut a, 8);
        enable_touch(&mut a, 9);
        a.set_input("t8", 120.0).unwrap();
        let out5 = a.read_u32(SAR_TOUCH_OUT5).unwrap();
        // Hardware channel 9 is the odd half of OUT5, and that is Arduino T8.
        assert_eq!(out5 & 0xFFFF, 12);
        assert_eq!(out5 >> 16, 70);
    }

    #[test]
    fn touch_trigger_status_follows_threshold_and_direction() {
        let mut a = Esp32SarAdc::new();
        enable_touch(&mut a, 0);
        // Threshold 0 (IDF "never touched") never triggers below.
        assert_eq!(a.read_u32(SAR_TOUCH_CTRL2).unwrap() & TOUCH_MEAS_EN_MASK, 0);
        a.write_u32(SAR_TOUCH_THRES1, 40 << 16).unwrap();
        assert_eq!(a.read_u32(SAR_TOUCH_CTRL2).unwrap() & 1, 0, "70 >= 40");
        a.set_input("t0", 120.0).unwrap();
        assert_eq!(a.read_u32(SAR_TOUCH_CTRL2).unwrap() & 1, 1, "12 < 40");
        a.write_u32(SAR_TOUCH_CTRL1, 0x1000 | TOUCH_OUT_SEL)
            .unwrap();
        assert_eq!(a.read_u32(SAR_TOUCH_CTRL2).unwrap() & 1, 0, "above mode");
        // DONE / status writes do not stick.
        a.write_u32(SAR_TOUCH_CTRL2, 0x7FF).unwrap();
        assert_eq!(a.read_u32(SAR_TOUCH_CTRL2).unwrap() & TOUCH_MEAS_EN_MASK, 0);
    }

    #[test]
    fn touch_input_rejects_unknown_and_out_of_range() {
        let mut a = Esp32SarAdc::new();
        assert!(a.set_input("t10", 50.0).is_err());
        assert!(a.set_input("t0", 0.0).is_err());
    }

    #[test]
    fn other_registers_are_register_backed() {
        let mut a = Esp32SarAdc::new();
        a.write_u32(0x08, 0xDEAD_BEEF).unwrap();
        assert_eq!(a.read_u32(0x08).unwrap(), 0xDEAD_BEEF);
    }
}
