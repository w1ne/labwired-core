// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Alif Semiconductor ADC12 (Ensemble E1..E8, E1C, Balletto B1): the 12-bit
//! SAR converter with hardware averaging, a channel sequencer and two digital
//! threshold comparators. `ADC_START_SRC` @0x00, `ADC_COMP_THRESH_A/B`
//! @0x04/0x08, `ADC_CLK_DIVISOR` @0x0C, `ADC_INTERRUPT` @0x10,
//! `ADC_INTERRUPT_MASK` @0x14, `ADC_SAMPLE_WIDTH` @0x18, `ADC_AVG_NUM` @0x20,
//! `ADC_SHIFT_CONTROL` @0x24, `ADC_CONTROL` @0x30, `ADC_SEQUENCER_CTRL`
//! @0x34, `ADC_REG1` @0x38, `ADC_SEL` @0x3C, `ADC_PERIPH` @0x40,
//! `ADC_SAMPLE_REG_n` @0x50 + 4n (n = 0..8).
//!
//! Sources:
//! - `alifsemi/alif_ensemble-cmsis-dfp`, `Device/soc/AE822FA0E5597/include/
//!   rtss_he/soc.h`: `ADC_Type` (the offsets above), `ADC120_BASE`
//!   0x4902_0000, `ADC121_BASE` 0x4902_1000, `ADC122_BASE` 0x4902_2000,
//!   `ADC24_BASE` 0x4902_7000; `ADC12n_DONE0/DONE1/CMPA/CMPB_IRQn` 151..162.
//! - The same tree's `drivers/include/adc.h` and `drivers/source/adc.c`:
//!   `ADC_START_SRC`: `ENA` bit 7 (single shot), `CONT` bit 6, external
//!   trigger enables [5:0]; `ADC_CONTROL`: single-shot start bit 0,
//!   `SAMPLE_INT_CLR` bit 2, comparator mode [17:16] (above / below /
//!   between A and B for CMPA; above / below / outside for CMPB);
//!   `ADC_SEQUENCER_CTRL`: channel masks [8:0], initial channel [15:12],
//!   fixed (1) or rotate (0) scan bit 16; `ADC_SHIFT_CONTROL`: shift count,
//!   right (1) or left (0) at bit 16; `ADC_INTERRUPT` bits DONE0 (averaged
//!   sample ready) 0, DONE1 (all samples taken) 1, CMPA 2, CMPB 3, cleared
//!   by writing ones; `ADC_INTERRUPT_MASK` 0xF masks all. The DONE1 handler
//!   reads `ADC_SAMPLE_REG_[ADC_SEL]` in single-shot mode; the DONE0 handler
//!   (continuous) reads the channel it prefetched from `ADC_SEL` at the
//!   previous DONE0, starting from the initial channel, so `ADC_SEL` is the
//!   channel the sequencer converts next.
//! - `Alif_CMSIS/Source/Driver_ADC.c`: `ADC_Start` = unmask, then
//!   `START_SRC |= ENA; CONTROL |= 1` (single shot) or `START_SRC |= CONT`;
//!   `ADC_Stop` clears the same bits.
//! - `alifsemi/zephyr_alif` `drivers/adc/adc_alif.c`: the same offsets and
//!   bits, `adc_check_busy` = `ADC_CONTROL` bit 1, masks written as
//!   `~channels & 0x1FF` (1 = channel excluded), a read refused when the
//!   initial channel is masked, the sign bit at 11 + log2(avg) ("ADC12
//!   starts at bit 11 and grows with averaging (max bit 19)"): the sample
//!   register holds the sum of `ADC_AVG_NUM` conversions before the shift.
//!   `dts/arm/alif/ensemble/common/e1.dtsi`: `adc0@49020000` (0x1000),
//!   `avg_sample_num` 256, `shift_n_bits` 8, `RIGHT_SHIFT`, so the default
//!   result is the 12-bit code.
//! - `alifsemi/zephyr_alif` `boards/alif/common/ensemble-e8-appkit-pinctrl.dtsi`
//!   (`ANA_S0..S17` on P0_0..P2_1) and `alifsemi/sdk-alif`
//!   `doc/appnotes/source/adc.rst` (six external inputs per instance,
//!   channel 6 the temperature sensor, channel 7 the internal VREF).
//!
//! Modelled: with `START_SRC.ENA` set, a write that raises `CONTROL` bit 0
//! (or sets `ENA` while bit 0 is set) converts the channel at the sequencer
//! position once and sets DONE0 and DONE1. With `START_SRC.CONT` set every
//! tick converts the channel at the sequencer position and sets DONE0; in
//! rotate mode the position then moves to the next unmasked channel
//! (ascending, wrapping) and DONE1 is set when it wraps. Setting `CONT`
//! restarts the sequencer at the initial channel. `ADC_SEL` reads the
//! position. A conversion with every channel masked converts nothing.
//! Sample = (code * N) shifted, with N = `ADC_AVG_NUM` (0 or 1 is one
//! sample) and code = mV * 4095 / VREF truncated and clamped. Each sample is
//! compared with `THRESH_A`/`THRESH_B` per `CONTROL` [17:16] and sets CMPA /
//! CMPB. `CONTROL` bit 1 (busy) reads 0. The IRQ level is
//! `INTERRUPT & !INTERRUPT_MASK`.
//!
//! Not sourced, kept minimal: the reference voltage (`vref_mv`, 1800 by
//! default: no public Ensemble datasheet states it); reset values (0); the
//! comparator's strictness (above = greater than, below = less than,
//! between = inclusive); the four NVIC lines are one level here; external
//! triggers arm but never fire; `SAMPLE_INT_CLR`, the clock divisor, sample
//! width, `ADC_REG1` and `ADC_PERIPH` (PGA, differential, comparator analog
//! enables) are stored with no effect; channels 6 and 7 (temperature,
//! VREF) read whatever is routed to them, 0 by default. The 24-bit ADC24
//! uses the same `ADC_Type` but is a different converter and is not this
//! model.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const CHANNELS: usize = 9;
const CODE_MAX: u32 = (1 << 12) - 1;

const START_SRC: u64 = 0x00;
const THRESH_A: u64 = 0x04;
const THRESH_B: u64 = 0x08;
const CLK_DIVISOR: u64 = 0x0C;
const INTERRUPT: u64 = 0x10;
const INTERRUPT_MASK: u64 = 0x14;
const SAMPLE_WIDTH: u64 = 0x18;
const AVG_NUM: u64 = 0x20;
const SHIFT_CONTROL: u64 = 0x24;
const CONTROL: u64 = 0x30;
const SEQUENCER_CTRL: u64 = 0x34;
const REG1: u64 = 0x38;
const SEL: u64 = 0x3C;
const PERIPH: u64 = 0x40;
const SAMPLE_REG0: u64 = 0x50;

const START_ENA: u32 = 1 << 7;
const START_CONT: u32 = 1 << 6;
const START_MASK: u32 = 0xFF;
const SINGLE_SHOT: u32 = 1 << 0;
const BUSY: u32 = 1 << 1;
/// Single-shot start, `SAMPLE_INT_CLR`, comparator mode [17:16].
const CONTROL_MASK: u32 = 0x0003_0005;
const DONE0: u32 = 1 << 0;
const DONE1: u32 = 1 << 1;
const CMPA: u32 = 1 << 2;
const CMPB: u32 = 1 << 3;
const IRQ_BITS: u32 = 0xF;
const MASK_ALL_CHANNELS: u32 = 0x1FF;
const INIT_POS: u32 = 12;
const FIXED_SCAN: u32 = 1 << 16;
const SEQUENCER_MASK: u32 = FIXED_SCAN | (0xF << INIT_POS) | MASK_ALL_CHANNELS;
const SHIFT_RIGHT: u32 = 1 << 16;

/// Alif ADC12: single-shot and continuous conversions with averaging, a
/// fixed or rotating sequencer and threshold comparators.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct AlifAdc12 {
    start_src: u32,
    thresh_a: u32,
    thresh_b: u32,
    clk_divisor: u32,
    interrupt: u32,
    interrupt_mask: u32,
    sample_width: u32,
    avg_num: u32,
    shift_control: u32,
    control: u32,
    sequencer: u32,
    reg1: u32,
    periph: u32,
    /// The channel the sequencer converts next (`ADC_SEL`).
    position: u8,
    samples: [u32; CHANNELS],
    vref_mv: u32,
    /// Routed levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
}

impl Default for AlifAdc12 {
    fn default() -> Self {
        Self::new(1800)
    }
}

impl AlifAdc12 {
    /// `vref_mv`: the reference the 12-bit code is a fraction of.
    pub fn new(vref_mv: u32) -> Self {
        Self {
            start_src: 0,
            thresh_a: 0,
            thresh_b: 0,
            clk_divisor: 0,
            interrupt: 0,
            interrupt_mask: 0,
            sample_width: 0,
            avg_num: 0,
            shift_control: 0,
            control: 0,
            sequencer: 0,
            reg1: 0,
            periph: 0,
            position: 0,
            samples: [0; CHANNELS],
            vref_mv: vref_mv.max(1),
            inputs_mv: [u16::MAX; CHANNELS],
        }
    }

    fn init_channel(&self) -> u8 {
        (((self.sequencer >> INIT_POS) & 0xF) as u8).min(CHANNELS as u8 - 1)
    }

    fn enabled(&self, channel: u8) -> bool {
        self.sequencer & (1 << channel) == 0
    }

    fn code(&self, channel: usize) -> u32 {
        let mv = match self.inputs_mv[channel] {
            u16::MAX => 0,
            mv => u32::from(mv),
        };
        (mv * CODE_MAX / self.vref_mv).min(CODE_MAX)
    }

    /// The averaging accumulator after the output shift.
    fn sample(&self, channel: usize) -> u32 {
        let sum = u64::from(self.code(channel)) * u64::from(self.avg_num.max(1));
        let shift = self.shift_control & 0x1F;
        let shifted = if self.shift_control & SHIFT_RIGHT != 0 {
            sum >> shift
        } else {
            sum << shift
        };
        shifted as u32
    }

    fn compare(&mut self, sample: u32) {
        let (a, b) = (self.thresh_a, self.thresh_b);
        let (cmpa, cmpb) = match (self.control >> 16) & 0b11 {
            0 => (sample > a, sample > b),
            1 => (sample < a, sample < b),
            2 => {
                let between = a.min(b) <= sample && sample <= a.max(b);
                (between, !between)
            }
            _ => (false, false),
        };
        if cmpa {
            self.interrupt |= CMPA;
        }
        if cmpb {
            self.interrupt |= CMPB;
        }
    }

    /// Converts the channel at the sequencer position. Returns false when
    /// that channel is masked (nothing converts).
    fn convert(&mut self) -> bool {
        let channel = self.position;
        if !self.enabled(channel) {
            return false;
        }
        let sample = self.sample(usize::from(channel));
        self.samples[usize::from(channel)] = sample;
        self.interrupt |= DONE0;
        self.compare(sample);
        true
    }

    /// Rotate mode: the next unmasked channel above the position, wrapping.
    /// Returns true when the sequence wrapped.
    fn advance(&mut self) -> bool {
        if self.sequencer & FIXED_SCAN != 0 {
            return true;
        }
        let n = CHANNELS as u8;
        for step in 1..=n {
            let next = (self.position + step) % n;
            if self.enabled(next) {
                let wrapped = next <= self.position;
                self.position = next;
                return wrapped;
            }
        }
        true
    }

    fn single_shot_armed(&self) -> bool {
        self.start_src & START_ENA != 0 && self.control & SINGLE_SHOT != 0
    }

    fn after_start_write(&mut self, was_armed: bool, was_continuous: bool) {
        if self.start_src & START_CONT != 0 && !was_continuous {
            self.position = self.init_channel();
        }
        if self.single_shot_armed() && !was_armed {
            if self.start_src & START_CONT == 0 {
                self.position = self.init_channel();
            }
            if self.convert() {
                self.interrupt |= DONE1;
            }
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            START_SRC => self.start_src,
            THRESH_A => self.thresh_a,
            THRESH_B => self.thresh_b,
            CLK_DIVISOR => self.clk_divisor,
            INTERRUPT => self.interrupt,
            INTERRUPT_MASK => self.interrupt_mask,
            SAMPLE_WIDTH => self.sample_width,
            AVG_NUM => self.avg_num,
            SHIFT_CONTROL => self.shift_control,
            CONTROL => self.control & !BUSY,
            SEQUENCER_CTRL => self.sequencer,
            REG1 => self.reg1,
            SEL => u32::from(self.position),
            PERIPH => self.periph,
            _ => match offset.checked_sub(SAMPLE_REG0).map(|o| (o / 4) as usize) {
                Some(n) if n < CHANNELS => self.samples[n],
                _ => 0,
            },
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let (was_armed, was_continuous) =
            (self.single_shot_armed(), self.start_src & START_CONT != 0);
        match offset {
            START_SRC => self.start_src = value & START_MASK,
            THRESH_A => self.thresh_a = value,
            THRESH_B => self.thresh_b = value,
            CLK_DIVISOR => self.clk_divisor = value,
            INTERRUPT => self.interrupt &= !(value & IRQ_BITS),
            INTERRUPT_MASK => self.interrupt_mask = value & IRQ_BITS,
            SAMPLE_WIDTH => self.sample_width = value,
            AVG_NUM => self.avg_num = value & 0x1FF,
            SHIFT_CONTROL => self.shift_control = value & (SHIFT_RIGHT | 0xFFFF),
            CONTROL => self.control = value & CONTROL_MASK,
            SEQUENCER_CTRL => {
                self.sequencer = value & SEQUENCER_MASK;
                if self.start_src & START_CONT == 0 {
                    self.position = self.init_channel();
                }
            }
            REG1 => self.reg1 = value,
            PERIPH => self.periph = value,
            _ => {}
        }
        if matches!(offset, START_SRC | CONTROL) {
            self.after_start_write(was_armed, was_continuous);
        }
    }

    fn irq_level(&self) -> bool {
        self.interrupt & !self.interrupt_mask & IRQ_BITS != 0
    }
}

impl Peripheral for AlifAdc12 {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if usize::from(channel) >= CHANNELS {
            return false;
        }
        self.inputs_mv[usize::from(channel)] = millivolts;
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        if usize::from(channel) >= CHANNELS {
            return false;
        }
        self.inputs_mv[usize::from(channel)] = u16::MAX;
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
        Some(self.irq_level())
    }

    fn on_system_reset(&mut self) {
        let (inputs, vref) = (self.inputs_mv, self.vref_mv);
        *self = Self::new(vref);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        if aligned == INTERRUPT {
            // Write-one-to-clear: only this byte's ones clear.
            self.write_word(aligned, u32::from(value) << shift);
            return Ok(());
        }
        let current = self.read_word(aligned);
        let next = (current & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(aligned, next);
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok((self.read_word(offset & !3) >> ((offset & 2) * 8)) as u16)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.start_src & START_CONT != 0 && self.convert() && self.advance() {
            self.interrupt |= DONE1;
        }
        PeripheralTickResult {
            irq: self.irq_level(),
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

    fn adc_with(inputs: &[(u8, u16)]) -> AlifAdc12 {
        let mut adc = AlifAdc12::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv), "channel {ch}");
        }
        adc
    }

    fn code(mv: u32) -> u32 {
        (mv * 4095 / 1800).min(4095)
    }

    /// Zephyr `adc_hw_init` with the e1.dtsi defaults (avg 256, shift 8
    /// right) and `adc_channel_select` for `channel`, fixed scan.
    fn zephyr_init(adc: &mut AlifAdc12, channel: u32) {
        adc.write_u32(SAMPLE_WIDTH, 16).unwrap();
        adc.write_u32(CLK_DIVISOR, 2).unwrap();
        adc.write_u32(AVG_NUM, 256).unwrap();
        adc.write_u32(SHIFT_CONTROL, 8 | SHIFT_RIGHT).unwrap();
        adc.write_u32(THRESH_A, 1).unwrap();
        adc.write_u32(THRESH_B, 1).unwrap();
        adc.write_u32(CONTROL, 0).unwrap();
        let seq = adc.read_u32(SEQUENCER_CTRL).unwrap();
        adc.write_u32(SEQUENCER_CTRL, seq | FIXED_SCAN).unwrap();
        adc.write_u32(INTERRUPT_MASK, !(DONE0 | DONE1) & 0xF)
            .unwrap();
        let seq = adc.read_u32(SEQUENCER_CTRL).unwrap() & !(0xF << INIT_POS);
        adc.write_u32(SEQUENCER_CTRL, seq | (channel << INIT_POS))
            .unwrap();
    }

    /// Zephyr `adc_start_read` + single-shot start + the DONE1 ISR + stop.
    fn zephyr_read(adc: &mut AlifAdc12, channel: u32) -> u32 {
        let seq = adc.read_u32(SEQUENCER_CTRL).unwrap() & !MASK_ALL_CHANNELS;
        adc.write_u32(SEQUENCER_CTRL, seq | (!(1 << channel) & MASK_ALL_CHANNELS))
            .unwrap();
        let init = (adc.read_u32(SEQUENCER_CTRL).unwrap() & 0xF000) >> 12;
        assert_eq!(adc.read_u32(SEQUENCER_CTRL).unwrap() & (1 << init), 0);
        adc.write_u32(INTERRUPT_MASK, !(DONE0 | DONE1) & 0xF)
            .unwrap();
        let src = adc.read_u32(START_SRC).unwrap();
        adc.write_u32(START_SRC, src | START_ENA).unwrap();
        let ctl = adc.read_u32(CONTROL).unwrap();
        adc.write_u32(CONTROL, ctl | SINGLE_SHOT).unwrap();
        assert!(adc.irq_level(), "DONE requests the NVIC line");
        assert_eq!(adc.read_u32(INTERRUPT).unwrap() & (DONE0 | DONE1), 3);
        adc.write_u32(INTERRUPT, DONE0).unwrap();
        adc.write_u32(INTERRUPT, DONE1).unwrap();
        let sel = adc.read_u32(SEL).unwrap();
        let value = adc.read_u32(SAMPLE_REG0 + 4 * u64::from(sel)).unwrap();
        // adc_context_on_complete -> disable_adc.
        let src = adc.read_u32(START_SRC).unwrap();
        adc.write_u32(START_SRC, src & !START_ENA).unwrap();
        let ctl = adc.read_u32(CONTROL).unwrap();
        adc.write_u32(CONTROL, ctl & !SINGLE_SHOT).unwrap();
        value
    }

    #[test]
    fn reset_state_and_register_masks() {
        let mut adc = AlifAdc12::default();
        for off in (0..0x74).step_by(4) {
            assert_eq!(adc.read_u32(off).unwrap(), 0, "offset {off:#x}");
        }
        adc.write_u32(START_SRC, u32::MAX).unwrap();
        assert_eq!(adc.read_u32(START_SRC).unwrap(), 0xFF);
        adc.write_u32(START_SRC, 0).unwrap();
        adc.write_u32(INTERRUPT_MASK, u32::MAX).unwrap();
        assert_eq!(adc.read_u32(INTERRUPT_MASK).unwrap(), 0xF);
        adc.write_u32(SEQUENCER_CTRL, u32::MAX).unwrap();
        assert_eq!(adc.read_u32(SEQUENCER_CTRL).unwrap(), 0x1_F1FF);
        adc.write_u32(CONTROL, u32::MAX).unwrap();
        assert_eq!(adc.read_u32(CONTROL).unwrap() & BUSY, 0, "never busy");
        adc.write_u32(SEL, 5).unwrap();
        assert_eq!(adc.read_u32(SEL).unwrap(), 8, "SEL is read-only");
        assert_eq!(adc.read_u32(0x74).unwrap(), 0, "past ADC_SAMPLE_REG_8");
    }

    #[test]
    fn zephyr_single_shot_reads_twelve_bits_against_vref() {
        let mut adc = adc_with(&[(0, 1650), (3, 640), (5, 2790)]);
        zephyr_init(&mut adc, 0);
        assert_eq!(zephyr_read(&mut adc, 0), code(1650));
        zephyr_init(&mut adc, 3);
        assert_eq!(zephyr_read(&mut adc, 3), code(640));
        zephyr_init(&mut adc, 5);
        assert_eq!(zephyr_read(&mut adc, 5), 4095, "above VREF clamps");
        zephyr_init(&mut adc, 1);
        assert_eq!(zephyr_read(&mut adc, 1), 0, "undriven reads 0");
        assert!(!adc.irq_level(), "the ISR cleared DONE0 and DONE1");
    }

    #[test]
    fn single_shot_needs_ena_and_a_rising_start() {
        let mut adc = adc_with(&[(2, 900)]);
        zephyr_init(&mut adc, 2);
        adc.write_u32(SEQUENCER_CTRL, FIXED_SCAN | (2 << INIT_POS))
            .unwrap();
        adc.write_u32(CONTROL, SINGLE_SHOT).unwrap();
        assert_eq!(adc.read_u32(INTERRUPT).unwrap(), 0, "no ENA, no conversion");
        adc.write_u32(START_SRC, START_ENA).unwrap();
        // Zephyr's thresholds (1) put the sample above A and B as well.
        assert_eq!(
            adc.read_u32(INTERRUPT).unwrap(),
            DONE0 | DONE1 | CMPA | CMPB,
            "ENA after bit 0"
        );
        assert_eq!(adc.read_u32(SAMPLE_REG0 + 8).unwrap(), code(900));
        adc.write_u32(INTERRUPT, 0xF).unwrap();
        adc.write_u32(CONTROL, SINGLE_SHOT).unwrap();
        adc.tick();
        assert_eq!(
            adc.read_u32(INTERRUPT).unwrap(),
            0,
            "bit 0 still set: no new shot"
        );
        // A masked initial channel converts nothing.
        adc.write_u32(CONTROL, 0).unwrap();
        adc.write_u32(SEQUENCER_CTRL, FIXED_SCAN | (2 << INIT_POS) | (1 << 2))
            .unwrap();
        adc.write_u32(CONTROL, SINGLE_SHOT).unwrap();
        assert_eq!(adc.read_u32(INTERRUPT).unwrap(), 0);
    }

    #[test]
    fn averaging_accumulates_and_the_shift_scales_it() {
        let mut adc = adc_with(&[(4, 1000)]);
        let c = code(1000);
        for (avg, shift, expected) in [
            (256, 8 | SHIFT_RIGHT, c),
            (256, 0, c * 256),
            (16, 0, c * 16),
            (16, 4 | SHIFT_RIGHT, c),
            (0, 0, c),
            (1, 2, c << 2),
        ] {
            adc.write_u32(AVG_NUM, avg).unwrap();
            adc.write_u32(SHIFT_CONTROL, shift).unwrap();
            adc.write_u32(SEQUENCER_CTRL, FIXED_SCAN | (4 << INIT_POS))
                .unwrap();
            adc.write_u32(START_SRC, START_ENA).unwrap();
            adc.write_u32(CONTROL, SINGLE_SHOT).unwrap();
            assert_eq!(
                adc.read_u32(SAMPLE_REG0 + 16).unwrap(),
                expected,
                "avg {avg}, shift {shift:#x}"
            );
            adc.write_u32(CONTROL, 0).unwrap();
        }
    }

    #[test]
    fn continuous_rotate_scan_follows_adc_sel() {
        let mut adc = adc_with(&[(1, 300), (3, 600), (6, 1200)]);
        adc.write_u32(AVG_NUM, 256).unwrap();
        adc.write_u32(SHIFT_CONTROL, 8 | SHIFT_RIGHT).unwrap();
        // CMSIS: mask all but 1, 3 and 6, initial channel 3, rotate mode.
        let mask = MASK_ALL_CHANNELS & !((1 << 1) | (1 << 3) | (1 << 6));
        adc.write_u32(SEQUENCER_CTRL, (3 << INIT_POS) | mask)
            .unwrap();
        adc.write_u32(START_SRC, START_CONT).unwrap();
        // The CMSIS DONE0 handler: read the prefetched channel, prefetch SEL.
        let mut read_channel = 3u32;
        let mut seen = Vec::new();
        for _ in 0..6 {
            adc.tick();
            assert_ne!(adc.read_u32(INTERRUPT).unwrap() & DONE0, 0);
            adc.write_u32(INTERRUPT, DONE0).unwrap();
            let raw = adc
                .read_u32(SAMPLE_REG0 + 4 * u64::from(read_channel))
                .unwrap();
            seen.push((read_channel, raw));
            read_channel = adc.read_u32(SEL).unwrap();
        }
        let (c1, c3, c6) = (code(300), code(600), code(1200));
        assert_eq!(
            seen,
            vec![(3, c3), (6, c6), (1, c1), (3, c3), (6, c6), (1, c1)],
            "3 -> 6 -> 1 -> 3, wrapping"
        );
        assert_ne!(
            adc.read_u32(INTERRUPT).unwrap() & DONE1,
            0,
            "the wrap sets DONE1"
        );
        // ADC_Stop: CONT off, conversions stop.
        adc.write_u32(START_SRC, 0).unwrap();
        adc.set_adc_channel_input(3, 0);
        adc.tick();
        assert_eq!(adc.read_u32(SAMPLE_REG0 + 12).unwrap(), c3);
    }

    #[test]
    fn continuous_fixed_scan_stays_on_the_initial_channel() {
        let mut adc = adc_with(&[(5, 700)]);
        adc.write_u32(SEQUENCER_CTRL, FIXED_SCAN | (5 << INIT_POS))
            .unwrap();
        adc.write_u32(START_SRC, START_CONT).unwrap();
        for _ in 0..3 {
            adc.tick();
            assert_eq!(adc.read_u32(SEL).unwrap(), 5);
        }
        assert_eq!(adc.read_u32(SAMPLE_REG0 + 20).unwrap(), code(700));
        adc.set_adc_channel_input(5, 1400);
        adc.tick();
        assert_eq!(adc.read_u32(SAMPLE_REG0 + 20).unwrap(), code(1400));
    }

    #[test]
    fn comparator_modes_set_cmpa_and_cmpb() {
        let mut adc = adc_with(&[(0, 900)]);
        let c = code(900);
        adc.write_u32(THRESH_A, c - 100).unwrap();
        adc.write_u32(THRESH_B, c + 100).unwrap();
        for (mode, expected) in [
            (0, CMPA), // above A, not above B
            (1, CMPB), // not below A, below B
            (2, CMPA), // between A and B, not outside
        ] {
            adc.write_u32(INTERRUPT, 0xF).unwrap();
            adc.write_u32(START_SRC, START_ENA).unwrap();
            adc.write_u32(CONTROL, mode << 16).unwrap();
            adc.write_u32(SEQUENCER_CTRL, FIXED_SCAN).unwrap();
            adc.write_u32(CONTROL, (mode << 16) | SINGLE_SHOT).unwrap();
            assert_eq!(
                adc.read_u32(INTERRUPT).unwrap() & (CMPA | CMPB),
                expected,
                "mode {mode}"
            );
        }
        adc.write_u32(THRESH_B, c - 50).unwrap();
        adc.write_u32(INTERRUPT, 0xF).unwrap();
        adc.write_u32(CONTROL, 2 << 16).unwrap();
        adc.write_u32(CONTROL, (2 << 16) | SINGLE_SHOT).unwrap();
        assert_eq!(
            adc.read_u32(INTERRUPT).unwrap() & (CMPA | CMPB),
            CMPB,
            "outside A..B"
        );
    }

    #[test]
    fn interrupt_mask_gates_the_line_and_reset_keeps_inputs() {
        let mut adc = adc_with(&[(0, 500)]);
        adc.write_u32(INTERRUPT_MASK, 0xF).unwrap();
        adc.write_u32(START_SRC, START_ENA).unwrap();
        adc.write_u32(CONTROL, SINGLE_SHOT).unwrap();
        assert_eq!(adc.read_u32(INTERRUPT).unwrap() & 3, 3);
        assert!(!adc.irq_level(), "masked");
        adc.write_u32(INTERRUPT_MASK, !DONE1 & 0xF).unwrap();
        assert!(adc.irq_level());
        adc.write(INTERRUPT, DONE1 as u8).unwrap();
        assert!(!adc.irq_level(), "byte write clears DONE1");
        assert_eq!(adc.read_u32(INTERRUPT).unwrap() & DONE0, DONE0);
        adc.on_system_reset();
        assert_eq!(adc.read_u32(INTERRUPT).unwrap(), 0);
        adc.write_u32(START_SRC, START_ENA).unwrap();
        adc.write_u32(CONTROL, SINGLE_SHOT).unwrap();
        assert_eq!(
            adc.read_u32(SAMPLE_REG0).unwrap(),
            code(500),
            "inputs survive reset"
        );
    }

    #[test]
    fn routed_reference_rescales() {
        let mut adc = adc_with(&[(0, 1650)]);
        assert!(adc.set_adc_reference_voltage_v(3.3));
        adc.write_u32(START_SRC, START_ENA).unwrap();
        adc.write_u32(CONTROL, SINGLE_SHOT).unwrap();
        assert_eq!(adc.read_u32(SAMPLE_REG0).unwrap(), 1650 * 4095 / 3300);
        assert!(!adc.set_adc_channel_input(9, 1));
    }
}
