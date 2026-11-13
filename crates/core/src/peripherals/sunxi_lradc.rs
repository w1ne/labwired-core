// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Allwinner sunxi low-resolution ADC (LRADC / KEYADC, A10, A13, A20 and
//! later): two 6-bit channels built for resistor-ladder keys. `LRADC_CTRL`
//! @0x00, `LRADC_INTC` @0x04, `LRADC_INTS` @0x08, `LRADC_DATA0` @0x0C,
//! `LRADC_DATA1` @0x10.
//!
//! Sources:
//! - Linux v6.12 `drivers/input/keyboard/sun4i-lradc-keys.c`: the offsets;
//!   `CTRL` fields `FIRST_CONVERT_DLY` [31:24], `CHAN_SELECT` [23:22],
//!   `CONTINUE_TIME_SEL` [19:16], `KEY_MODE_SEL` [13:12], `LEVELA_B_CNT`
//!   [11:8], `HOLD_KEY_EN` 7, `HOLD_EN` 6, `LEVELB_VOL` [5:4],
//!   `SAMPLE_RATE` [3:2], `ENABLE` 0; `INTC`/`INTS` bits `CHANn_DATA_IRQ`,
//!   `KEYDOWN`, `HOLD`, `ALRDY_HOLD`, `KEYUP` at 0..4 (channel 0) and 8..12
//!   (channel 1); the ISR reads `INTS`, on `KEYDOWN` reads `DATA0 & 0x3f`
//!   and takes `voltage = val * vref / 63`, then writes `INTS` back; the
//!   open sequence `FIRST_CONVERT_DLY(2) | LEVELA_B_CNT(1) | HOLD_EN(1) |
//!   SAMPLE_RATE(0) | ENABLE(1)` with `INTC = KEYUP | KEYDOWN`; `vref` is
//!   the `vref-supply` voltage times 2/3 (A10 variant, used by A20) or 3/4
//!   (A83T R_LRADC, R329).
//! - NetBSD `sys/arch/arm/sunxi/sunxi_lradc.{h,c}`: the same fields,
//!   `LEVEL_B` codes 0x3C/0x39/0x36/0x33 for `LEVELB_VOL` 0..3, sample rates
//!   250/125/62/31 Hz, `CHAN_SELECT` 0 = channel 0, 1 = channel 1, 2 = both,
//!   key levels `voltage * 63 / vref` and `vref = supply * 2 / 3`, and the
//!   ISR that clears `INTS` by writing back what it read.
//! - Linux `arch/arm/boot/dts/allwinner/sun7i-a20.dtsi`: `lradc@1c22800`,
//!   `allwinner,sun4i-a10-lradc-keys`, 0x100 bytes, GIC SPI 31.
//! - xboot `mach-h3/driver/key-h3-lradc.c`: the same open sequence and ISR,
//!   `reference-voltage` 3.0 V by default, times 2/3.
//!
//! Modelled: while `ENABLE` is set every tick samples the channels
//! `CHAN_SELECT` names. A sample stores `code = mV * 63 / VREF` (truncated,
//! clamped at 0x3F) in `DATAn` and sets `CHANn_DATA`. A channel at or above
//! level B (`0x3C - 3 * LEVELB_VOL`) is idle; the first sample below level
//! B sets `KEYDOWN` and the first sample back at or above it sets `KEYUP`.
//! `INTS` bits clear when written 1. The IRQ level is `INTS & INTC`.
//! VREF = supply * `vref_num` / `vref_den` (3000 mV * 2 / 3 by default); a
//! routed VDDA replaces the supply.
//!
//! Not sourced, kept minimal: a sample per tick (`SAMPLE_RATE`,
//! `FIRST_CONVERT_DLY` and `CONTINUE_TIME_SEL` are stored and not timed);
//! `LEVELA_B_CNT` debounce, the hold and already-hold events, and the
//! single/continuous key modes are stored and not modelled (a key is a
//! single `KEYDOWN` and a single `KEYUP`); `CHAN_SELECT` 3 converts both
//! channels like 2; `DATAn` keeps its last sample while its channel is not
//! selected; reset values are 0.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const CHANNELS: usize = 2;
const CODE_MAX: u32 = 0x3F;

const CTRL: u64 = 0x00;
const INTC: u64 = 0x04;
const INTS: u64 = 0x08;
const DATA0: u64 = 0x0C;
const DATA1: u64 = 0x10;

const ENABLE: u32 = 1 << 0;
const LEVELB_SHIFT: u32 = 4;
const CHAN_SELECT_SHIFT: u32 = 22;
/// `FIRST_CONVERT_DLY`, `CHAN_SELECT`, `CONTINUE_TIME_SEL`, `KEY_MODE_SEL`,
/// `LEVELA_B_CNT`, `HOLD_KEY_EN`, `HOLD_EN`, `LEVELB_VOL`, `SAMPLE_RATE`,
/// `ENABLE`.
const CTRL_MASK: u32 = 0xFFCF_3FFD;
/// `DATA`, `KEYDOWN`, `HOLD`, `ALRDY_HOLD`, `KEYUP` for channels 0 and 1.
const INT_MASK: u32 = 0x1F1F;
const DATA_IRQ: u32 = 1 << 0;
const KEYDOWN_IRQ: u32 = 1 << 1;
const KEYUP_IRQ: u32 = 1 << 4;

/// sunxi LRADC: two 6-bit key-ladder channels with data, key-down and
/// key-up events.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SunxiLradc {
    ctrl: u32,
    intc: u32,
    ints: u32,
    data: [u8; CHANNELS],
    key_down: [bool; CHANNELS],
    supply_mv: u32,
    vref_num: u32,
    vref_den: u32,
    /// Routed levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
}

impl Default for SunxiLradc {
    fn default() -> Self {
        Self::new(3000, 2, 3)
    }
}

impl SunxiLradc {
    /// `supply_mv`: the `vref-supply` rail. `vref_num` / `vref_den`: the
    /// internal divider (2/3 on A10/A20, 3/4 on A83T R_LRADC and R329).
    pub fn new(supply_mv: u32, vref_num: u32, vref_den: u32) -> Self {
        Self {
            ctrl: 0,
            intc: 0,
            ints: 0,
            data: [0; CHANNELS],
            key_down: [false; CHANNELS],
            supply_mv: supply_mv.max(1),
            vref_num: vref_num.max(1),
            vref_den: vref_den.max(1),
            inputs_mv: [u16::MAX; CHANNELS],
        }
    }

    fn vref_mv(&self) -> u32 {
        (self.supply_mv * self.vref_num / self.vref_den).max(1)
    }

    fn code(&self, channel: usize) -> u8 {
        let mv = match self.inputs_mv[channel] {
            u16::MAX => 0,
            mv => u32::from(mv),
        };
        (mv * CODE_MAX / self.vref_mv()).min(CODE_MAX) as u8
    }

    fn level_b(&self) -> u8 {
        0x3C - 3 * ((self.ctrl >> LEVELB_SHIFT) & 3) as u8
    }

    fn selected(&self) -> [bool; CHANNELS] {
        match (self.ctrl >> CHAN_SELECT_SHIFT) & 3 {
            0 => [true, false],
            1 => [false, true],
            _ => [true, true],
        }
    }

    fn sample(&mut self) {
        let level_b = self.level_b();
        for (channel, selected) in self.selected().into_iter().enumerate() {
            if !selected {
                continue;
            }
            let code = self.code(channel);
            let shift = 8 * channel as u32;
            self.data[channel] = code;
            self.ints |= DATA_IRQ << shift;
            let pressed = code < level_b;
            if pressed && !self.key_down[channel] {
                self.ints |= KEYDOWN_IRQ << shift;
            } else if !pressed && self.key_down[channel] {
                self.ints |= KEYUP_IRQ << shift;
            }
            self.key_down[channel] = pressed;
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            CTRL => self.ctrl,
            INTC => self.intc,
            INTS => self.ints,
            DATA0 => u32::from(self.data[0]),
            DATA1 => u32::from(self.data[1]),
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            CTRL => {
                self.ctrl = value & CTRL_MASK;
                if self.ctrl & ENABLE == 0 {
                    // A disabled converter forgets the key state.
                    self.key_down = [false; CHANNELS];
                }
            }
            INTC => self.intc = value & INT_MASK,
            INTS => self.ints &= !(value & INT_MASK),
            _ => {}
        }
    }

    fn irq_level(&self) -> bool {
        self.ints & self.intc != 0
    }
}

impl Peripheral for SunxiLradc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        match self.inputs_mv.get_mut(usize::from(channel)) {
            Some(input) => {
                *input = millivolts;
                true
            }
            None => false,
        }
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        match self.inputs_mv.get_mut(usize::from(channel)) {
            Some(input) => {
                *input = u16::MAX;
                true
            }
            None => false,
        }
    }

    fn supports_adc_reference_voltage(&self) -> bool {
        true
    }

    /// The routed rail is the `vref-supply`; the converter divides it.
    fn set_adc_reference_voltage_v(&mut self, volts: f64) -> bool {
        if !volts.is_finite() || volts <= 0.0 {
            return false;
        }
        self.supply_mv = ((volts * 1000.0).round() as u32).max(1);
        true
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_level())
    }

    fn on_system_reset(&mut self) {
        let inputs = self.inputs_mv;
        *self = Self::new(self.supply_mv, self.vref_num, self.vref_den);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        if aligned == INTS {
            // Write-one-to-clear: only this byte's ones clear.
            self.write_word(aligned, u32::from(value) << shift);
            return Ok(());
        }
        let current = self.read_word(aligned);
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
        if self.ctrl & ENABLE != 0 {
            self.sample();
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

    const HOLD_EN: u32 = 1 << 6;
    const KEYUP0: u32 = KEYUP_IRQ;
    const KEYDOWN0: u32 = KEYDOWN_IRQ;
    const KEYDOWN1: u32 = KEYDOWN_IRQ << 8;
    const KEYUP1: u32 = KEYUP_IRQ << 8;
    const DATA1_IRQ: u32 = DATA_IRQ << 8;

    /// Linux `sun4i_lradc_open` (NetBSD adds `CHAN_SELECT`).
    fn linux_open(adc: &mut SunxiLradc, chan_select: u32) -> u32 {
        let ctrl = (2 << 24) | (chan_select << 22) | (1 << 8) | HOLD_EN | ENABLE;
        adc.write_u32(CTRL, ctrl).unwrap();
        adc.write_u32(INTC, KEYUP0 | KEYDOWN0 | KEYUP1 | KEYDOWN1)
            .unwrap();
        ctrl
    }

    /// Linux ISR: read `INTS`, the key voltage on `KEYDOWN`, write back.
    fn linux_isr(adc: &mut SunxiLradc, vref_mv: u32) -> (u32, Option<u32>) {
        let ints = adc.read_u32(INTS).unwrap();
        let voltage =
            (ints & KEYDOWN0 != 0).then(|| (adc.read_u32(DATA0).unwrap() & 0x3F) * vref_mv / 63);
        adc.write_u32(INTS, ints).unwrap();
        (ints, voltage)
    }

    #[test]
    fn reset_values_masks_and_read_only_data() {
        let mut adc = SunxiLradc::default();
        for offset in (0..0x14).step_by(4) {
            assert_eq!(adc.read_u32(offset).unwrap(), 0, "offset {offset:#x}");
        }
        adc.write_u32(CTRL, !0).unwrap();
        assert_eq!(adc.read_u32(CTRL).unwrap(), CTRL_MASK);
        adc.write_u32(INTC, !0).unwrap();
        assert_eq!(adc.read_u32(INTC).unwrap(), 0x1F1F);
        adc.write_u32(DATA0, 0x2A).unwrap();
        adc.write_u32(CTRL, 0).unwrap();
        assert_eq!(adc.read_u32(DATA0).unwrap(), 0, "DATA0 is read-only");
        assert_eq!(adc.adc_channel_count(), Some(2));
        assert!(!adc.set_adc_channel_input(2, 100), "two channels only");
    }

    #[test]
    fn samples_six_bits_against_two_thirds_of_the_supply() {
        let mut adc = SunxiLradc::default();
        for (ch, mv) in [(0u8, 1000u16), (1, 2500)] {
            assert!(adc.set_adc_channel_input(ch, mv));
        }
        linux_open(&mut adc, 2);
        adc.write_u32(INTC, 0).unwrap();
        assert_eq!(adc.read_u32(DATA0).unwrap(), 0, "no sample before a tick");
        adc.tick();
        // VREF = 3000 * 2 / 3 = 2000 mV.
        assert_eq!(adc.read_u32(DATA0).unwrap(), 1000 * 63 / 2000);
        assert_eq!(adc.read_u32(DATA1).unwrap(), 63, "clamped above VREF");
        assert_eq!(adc.read_u32(INTS).unwrap() & 0x101, 0x101, "DATA0 + DATA1");
        assert!(!adc.irq_level(), "INTC = 0 masks every event");
        // The A83T R_LRADC divides by 3/4; a routed 3.3 V rail scales it.
        let mut adc = SunxiLradc::new(3000, 3, 4);
        assert!(adc.set_adc_channel_input(0, 1000));
        assert!(adc.set_adc_reference_voltage_v(3.3));
        linux_open(&mut adc, 0);
        adc.tick();
        assert_eq!(adc.read_u32(DATA0).unwrap(), 1000 * 63 / 2475);
        assert_eq!(adc.read_u32(DATA1).unwrap(), 0, "channel 1 not selected");
    }

    #[test]
    fn key_down_and_up_cross_level_b_and_feed_the_linux_isr() {
        let mut adc = SunxiLradc::default();
        assert!(
            adc.set_adc_channel_input(0, 3000),
            "idle ladder at the rail"
        );
        linux_open(&mut adc, 0);
        adc.tick();
        assert!(!adc.irq_level(), "idle: data only, masked");
        // A key pulls channel 0 to 600 mV: code 18, below level B 0x3C.
        assert!(adc.set_adc_channel_input(0, 600));
        adc.tick();
        assert!(adc.irq_level(), "KEYDOWN raises GIC SPI 31");
        let (ints, voltage) = linux_isr(&mut adc, 2000);
        assert_eq!(ints & (KEYDOWN0 | KEYUP0), KEYDOWN0);
        assert_eq!(voltage, Some(18 * 2000 / 63));
        adc.tick();
        assert!(!adc.irq_level(), "a held key reports one KEYDOWN");
        // Release: back above level B.
        assert!(adc.set_adc_channel_input(0, 3000));
        adc.tick();
        let (ints, voltage) = linux_isr(&mut adc, 2000);
        assert_eq!(ints & (KEYDOWN0 | KEYUP0), KEYUP0);
        assert_eq!(voltage, None);
        assert!(!adc.irq_level());
    }

    #[test]
    fn levelb_vol_moves_the_key_threshold() {
        // 1830 mV is code 57 (0x39): pressed against 0x3C, idle against 0x39.
        for (levelb, pressed) in [(0u32, true), (1, false), (3, false)] {
            let mut adc = SunxiLradc::default();
            assert!(adc.set_adc_channel_input(0, 1830));
            let ctrl = linux_open(&mut adc, 0);
            adc.write_u32(CTRL, ctrl | (levelb << 4)).unwrap();
            adc.tick();
            assert_eq!(adc.read_u32(DATA0).unwrap(), 57);
            assert_eq!(
                adc.read_u32(INTS).unwrap() & KEYDOWN0 != 0,
                pressed,
                "LEVELB_VOL {levelb}"
            );
        }
    }

    #[test]
    fn chan_select_routes_samples_and_events_per_channel() {
        let mut adc = SunxiLradc::default();
        assert!(adc.set_adc_channel_input(0, 500));
        assert!(adc.set_adc_channel_input(1, 700));
        linux_open(&mut adc, 1);
        adc.tick();
        assert_eq!(adc.read_u32(DATA0).unwrap(), 0, "channel 0 not selected");
        assert_eq!(adc.read_u32(DATA1).unwrap(), 700 * 63 / 2000);
        assert_eq!(
            adc.read_u32(INTS).unwrap(),
            DATA1_IRQ | KEYDOWN1,
            "channel 1 events only"
        );
        adc.write_u32(INTS, !0).unwrap();
        // Both channels: channel 1 stays down, channel 0 goes down.
        let ctrl = adc.read_u32(CTRL).unwrap();
        adc.write_u32(CTRL, (ctrl & !(3 << 22)) | (2 << 22))
            .unwrap();
        adc.tick();
        assert_eq!(adc.read_u32(INTS).unwrap(), 0x101 | KEYDOWN0);
        assert_eq!(adc.read_u32(DATA0).unwrap(), 500 * 63 / 2000);
        // Undriven input samples 0, which is a pressed key.
        assert!(adc.clear_adc_channel_input(1));
        adc.write_u32(INTS, !0).unwrap();
        adc.tick();
        assert_eq!(adc.read_u32(DATA1).unwrap(), 0);
        assert_eq!(adc.read_u32(INTS).unwrap() & KEYUP1, 0);
    }

    #[test]
    fn disable_stops_sampling_and_forgets_the_key() {
        let mut adc = SunxiLradc::default();
        assert!(adc.set_adc_channel_input(0, 600));
        let ctrl = linux_open(&mut adc, 0);
        adc.tick();
        adc.write_u32(INTS, !0).unwrap();
        // Linux close: ENABLE = 0, INTC = 0.
        adc.write_u32(CTRL, ctrl & !ENABLE).unwrap();
        assert!(adc.set_adc_channel_input(0, 1200));
        adc.tick();
        assert_eq!(adc.read_u32(DATA0).unwrap(), 600 * 63 / 2000, "held");
        assert_eq!(adc.read_u32(INTS).unwrap(), 0, "no samples");
        // Re-open with the key still down: a fresh KEYDOWN.
        adc.write_u32(CTRL, ctrl).unwrap();
        adc.tick();
        assert_eq!(adc.read_u32(INTS).unwrap() & KEYDOWN0, KEYDOWN0);
        // Byte write-one-to-clear.
        adc.write(INTS, KEYDOWN0 as u8).unwrap();
        assert_eq!(adc.read_u32(INTS).unwrap(), DATA_IRQ);
    }

    #[test]
    fn system_reset_keeps_routed_inputs_and_supply() {
        let mut adc = SunxiLradc::default();
        assert!(adc.set_adc_channel_input(0, 1000));
        assert!(adc.set_adc_reference_voltage_v(3.3));
        linux_open(&mut adc, 0);
        adc.tick();
        adc.on_system_reset();
        assert_eq!(adc.read_u32(CTRL).unwrap(), 0);
        assert_eq!(adc.read_u32(DATA0).unwrap(), 0);
        linux_open(&mut adc, 0);
        adc.tick();
        assert_eq!(adc.read_u32(DATA0).unwrap(), 1000 * 63 / 2200);
    }
}
