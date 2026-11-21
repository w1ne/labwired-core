// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Rockchip SARADC, second register generation ("v2": RV1103/RV1106, RK3588
//! and later). `CONV_CON` @0x000, `T_PD_SOC` @0x004, `T_AS_SOC` @0x008,
//! `T_DAS_SOC` @0x00C, `T_SEL_SOC` @0x010, `END_INT_EN` @0x104,
//! `END_INT_ST` @0x110, `DATAn` @0x120 + 4n.
//!
//! Sources:
//! - Luckfox SDK @824b817f kernel `drivers/iio/adc/rockchip_saradc.c`
//!   (sha1 84f00b0c): the offsets; `CONV_CON` channel [3:0], `START` bit 4,
//!   `SINGLE_MODE` bit 5; `END_INT_EN` bit 0. `rockchip_saradc_start_v2`
//!   writes `T_DAS_SOC = 0xc`, `T_PD_SOC = 0x20`, `END_INT_EN = 1 << 16 | 1`
//!   and `CONV_CON = val << 16 | val` with `val = START | SINGLE_MODE | chn`.
//!   `rockchip_saradc_read_v2` writes 1 to `END_INT_ST` ("Clear irq") and
//!   reads `DATA_BASE + chn * 4`; the ISR masks the value to `realbits`.
//!   `rv1106_saradc_data`: two channels, 10 bits. The scale is
//!   `vref_mV / 2^realbits` (`IIO_VAL_FRACTIONAL_LOG2`).
//! - Linux v6.12 `drivers/iio/adc/rockchip_saradc.c` (sha1 51514dbd): the
//!   same v2 offsets and bits; the `CONV_CON` write mask is
//!   `(START | SINGLE_MODE | CONV_CHANNELS) << 16`, so bits [31:16] are a
//!   write enable for bits [15:0]. `rk3588_saradc_data`: 8 channels, 12 bits.
//! - Luckfox SDK @824b817f U-Boot `drivers/adc/rockchip-saradc-v2.c`
//!   (sha1 4de9405a): `struct rockchip_saradc_regs`, which gives the same
//!   offsets for `conv_con`, `t_pd_soc`, `t_as_soc`, `t_das_soc`,
//!   `t_sel_soc`, `end_int_en`, `end_int_st` and `data0..15`, and the same
//!   start and read sequence.
//! - Luckfox SDK @824b817f `rv1106.dtsi`: `saradc@ff3c0000`,
//!   `rockchip,rv1106-saradc`, 0x200 bytes, GIC SPI 62.
//!   `rv1103-luckfox-pico-ipc.dtsi`: `vref-supply = <&vcc_1v8>`.
//!
//! Modelled: a `CONV_CON` write that leaves `START` set arms one conversion
//! of the channel in bits [3:0]. The next tick stores
//! `code = mV * 2^bits / VREF` (truncated, clamped at full scale) in that
//! channel's `DATAn` and sets `END_INT_ST` bit 0. Writing 1 to `END_INT_ST`
//! bit 0 clears it. The IRQ level is `END_INT_ST & END_INT_EN`. `CONV_CON`
//! and `END_INT_EN` take 32-bit writes with the write enable in the upper
//! half. The four `T_*` registers are stored and not timed.
//!
//! The vendor `val << 16 | val` start enables only the bits it sets, so it
//! cannot clear a channel bit that an earlier start left set. Both vendor
//! drivers reset the controller (`saradc-apb`) before each start; the kernel
//! comment gives the reason ("If read other chn at anytime, then chn1 will
//! error, assert controller as a workaround"). The write enable is the
//! model's explanation of that symptom, not a vendor statement: without the
//! reset, channel 0 after channel 1 converts channel 1 again. `on_system_reset` stands for the CRU reset. The upstream start
//! enables the whole channel field and needs no reset.
//!
//! Not sourced, kept minimal: `START` clears when the conversion ends (the
//! drivers never read `CONV_CON` back); a `START` without `SINGLE_MODE`
//! also converts once (continuous conversion is not modelled); a channel
//! number at or above the channel count converts nothing and raises no
//! event; the comparators (`HIGH_COMPn`, `LOW_COMPn`, `DEBOUNCE`,
//! `HT/LT/MT_INT_*`), `ST_CON`, `STATUS` and `AUTO_CH_EN` read 0 and ignore
//! writes; reset values are 0; byte writes to `CONV_CON` and `END_INT_EN`
//! are ignored, because a byte cannot carry its own write enable.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const MAX_CHANNELS: usize = 16;

const CONV_CON: u64 = 0x000;
const T_PD_SOC: u64 = 0x004;
const T_SEL_SOC: u64 = 0x010;
const END_INT_EN: u64 = 0x104;
const END_INT_ST: u64 = 0x110;
const DATA_BASE: u64 = 0x120;
const DATA_END: u64 = DATA_BASE + 4 * MAX_CHANNELS as u64;

const CHANNEL_MASK: u32 = 0xF;
const START: u32 = 1 << 4;
const SINGLE_MODE: u32 = 1 << 5;
const CONV_CON_MASK: u32 = CHANNEL_MASK | START | SINGLE_MODE;
const END_INT: u32 = 1 << 0;

/// Rockchip SARADC v2: single conversions with an end-of-conversion event.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RockchipSaradcV2 {
    conv_con: u32,
    /// `T_PD_SOC`, `T_AS_SOC`, `T_DAS_SOC`, `T_SEL_SOC`.
    timing: [u32; 4],
    end_int_en: u32,
    end_int_st: u32,
    data: [u16; MAX_CHANNELS],
    /// A conversion armed by `START`, finished by the next tick.
    pending: bool,
    channels: u8,
    bits: u8,
    vref_mv: u32,
    /// Routed levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; MAX_CHANNELS],
}

impl Default for RockchipSaradcV2 {
    /// The RV1103/RV1106 converter on a 1.8 V reference.
    fn default() -> Self {
        Self::new(2, 10, 1800)
    }
}

impl RockchipSaradcV2 {
    /// `channels`: 2 on RV1103/RV1106, 8 on RK3588. `bits`: 10 on
    /// RV1103/RV1106, 12 on RK3588. `vref_mv`: the `vref-supply` rail.
    pub fn new(channels: u8, bits: u8, vref_mv: u32) -> Self {
        Self {
            conv_con: 0,
            timing: [0; 4],
            end_int_en: 0,
            end_int_st: 0,
            data: [0; MAX_CHANNELS],
            pending: false,
            channels: channels.clamp(1, MAX_CHANNELS as u8),
            bits: bits.clamp(1, 16),
            vref_mv: vref_mv.max(1),
            inputs_mv: [u16::MAX; MAX_CHANNELS],
        }
    }

    fn full_scale(&self) -> u32 {
        (1 << self.bits) - 1
    }

    fn code(&self, channel: usize) -> u16 {
        let mv = match self.inputs_mv[channel] {
            u16::MAX => 0,
            mv => u64::from(mv),
        };
        let code = (mv << self.bits) / u64::from(self.vref_mv);
        code.min(u64::from(self.full_scale())) as u16
    }

    fn convert(&mut self) {
        self.pending = false;
        self.conv_con &= !START;
        let channel = (self.conv_con & CHANNEL_MASK) as usize;
        if channel < usize::from(self.channels) {
            self.data[channel] = self.code(channel);
            self.end_int_st |= END_INT;
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            CONV_CON => self.conv_con,
            T_PD_SOC..=T_SEL_SOC => self.timing[(offset - T_PD_SOC) as usize / 4],
            END_INT_EN => self.end_int_en,
            END_INT_ST => self.end_int_st,
            DATA_BASE..DATA_END => u32::from(self.data[(offset - DATA_BASE) as usize / 4]),
            _ => 0,
        }
    }

    /// Bits [31:16] of `value` enable the write of the same bit in [15:0].
    fn masked(current: u32, value: u32, implemented: u32) -> u32 {
        let enable = (value >> 16) & implemented;
        (current & !enable) | (value & enable)
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            CONV_CON => {
                self.conv_con = Self::masked(self.conv_con, value, CONV_CON_MASK);
                self.pending = self.conv_con & START != 0;
            }
            T_PD_SOC..=T_SEL_SOC => self.timing[(offset - T_PD_SOC) as usize / 4] = value,
            END_INT_EN => self.end_int_en = Self::masked(self.end_int_en, value, END_INT),
            END_INT_ST => self.end_int_st &= !(value & END_INT),
            _ => {}
        }
    }

    fn irq_level(&self) -> bool {
        self.end_int_st & self.end_int_en != 0
    }
}

impl Peripheral for RockchipSaradcV2 {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(self.channels)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        if channel >= self.channels {
            return false;
        }
        self.inputs_mv[usize::from(channel)] = millivolts;
        true
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        if channel >= self.channels {
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
        let inputs = self.inputs_mv;
        *self = Self::new(self.channels, self.bits, self.vref_mv);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        match aligned {
            // The write enable is in the other half of the word.
            CONV_CON | END_INT_EN => {}
            // Write-one-to-clear: only this byte's ones clear.
            END_INT_ST => self.write_word(aligned, u32::from(value) << shift),
            _ => {
                let current = self.read_word(aligned);
                let next = (current & !(0xFF << shift)) | (u32::from(value) << shift);
                self.write_word(aligned, next);
            }
        }
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
        if self.pending {
            self.convert();
        }
        PeripheralTickResult {
            irq: self.irq_level(),
            ..PeripheralTickResult::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T_DAS_SOC: u64 = 0x00C;

    /// `rockchip_saradc_start_v2` (Luckfox SDK kernel and U-Boot).
    fn driver_start(adc: &mut RockchipSaradcV2, channel: u32) {
        adc.write_u32(T_DAS_SOC, 0xC).unwrap();
        adc.write_u32(T_PD_SOC, 0x20).unwrap();
        adc.write_u32(END_INT_EN, END_INT << 16 | END_INT).unwrap();
        let val = START | SINGLE_MODE | channel;
        adc.write_u32(CONV_CON, val << 16 | val).unwrap();
    }

    /// `rockchip_saradc_read_v2`: clear the event, read the channel's data.
    fn driver_read(adc: &mut RockchipSaradcV2, channel: u32) -> u32 {
        adc.write_u32(END_INT_ST, 1).unwrap();
        adc.read_u32(DATA_BASE + 4 * u64::from(channel)).unwrap()
    }

    #[test]
    fn reset_values_write_enables_and_read_only_data() {
        let mut adc = RockchipSaradcV2::default();
        for offset in (0..0x164).step_by(4) {
            assert_eq!(adc.read_u32(offset).unwrap(), 0, "offset {offset:#x}");
        }
        // No write enable: nothing lands.
        adc.write_u32(CONV_CON, 0xFFFF).unwrap();
        assert_eq!(adc.read_u32(CONV_CON).unwrap(), 0);
        adc.write_u32(END_INT_EN, 1).unwrap();
        assert_eq!(adc.read_u32(END_INT_EN).unwrap(), 0);
        // Enable only SINGLE_MODE and the channel: START stays clear.
        adc.write_u32(CONV_CON, (SINGLE_MODE | CHANNEL_MASK) << 16 | 0xFFFF)
            .unwrap();
        assert_eq!(adc.read_u32(CONV_CON).unwrap(), SINGLE_MODE | CHANNEL_MASK);
        adc.tick();
        assert_eq!(adc.read_u32(END_INT_ST).unwrap(), 0, "no START, no event");
        adc.write_u32(END_INT_EN, 0xFFFF_FFFF).unwrap();
        assert_eq!(adc.read_u32(END_INT_EN).unwrap(), END_INT);
        adc.write_u32(DATA_BASE, 0x155).unwrap();
        assert_eq!(adc.read_u32(DATA_BASE).unwrap(), 0, "DATA0 is read-only");
        adc.write_u32(T_PD_SOC, 0x20).unwrap();
        adc.write_u32(T_DAS_SOC, 0xC).unwrap();
        assert_eq!(adc.read_u32(T_PD_SOC).unwrap(), 0x20);
        assert_eq!(adc.read_u32(T_DAS_SOC).unwrap(), 0xC);
        assert_eq!(adc.adc_channel_count(), Some(2));
        assert!(!adc.set_adc_channel_input(2, 100), "two channels only");
    }

    #[test]
    fn driver_sequence_converts_ten_bits_against_the_reference() {
        let mut adc = RockchipSaradcV2::default();
        assert!(adc.set_adc_channel_input(0, 900));
        assert!(adc.set_adc_channel_input(1, 450));
        driver_start(&mut adc, 0);
        assert!(!adc.irq_level(), "the conversion ends on the next tick");
        assert_eq!(adc.read_u32(DATA_BASE).unwrap(), 0);
        assert!(adc.tick().irq, "END_INT raises GIC SPI 62");
        assert_eq!(adc.read_u32(END_INT_ST).unwrap(), END_INT);
        assert_eq!(
            adc.read_u32(CONV_CON).unwrap(),
            SINGLE_MODE,
            "START cleared"
        );
        assert_eq!(driver_read(&mut adc, 0), 900 * 1024 / 1800);
        assert!(!adc.irq_level(), "the read clears the event");
        assert!(!adc.tick().irq, "one conversion per START");
        assert_eq!(adc.read_u32(DATA_BASE + 4).unwrap(), 0, "channel 1 idle");

        driver_start(&mut adc, 1);
        adc.tick();
        assert_eq!(driver_read(&mut adc, 1), 450 * 1024 / 1800);
        assert_eq!(adc.read_u32(DATA_BASE).unwrap(), 512, "DATA0 is held");
    }

    /// Linux v6.12 `rockchip_saradc_start_v2`: the write enable covers the
    /// whole channel field.
    fn upstream_start(adc: &mut RockchipSaradcV2, channel: u32) {
        adc.write_u32(END_INT_EN, END_INT << 16 | END_INT).unwrap();
        let val = START | SINGLE_MODE | channel;
        adc.write_u32(CONV_CON, CONV_CON_MASK << 16 | val).unwrap();
    }

    #[test]
    fn vendor_start_needs_the_controller_reset_to_go_back_to_channel_0() {
        let mut adc = RockchipSaradcV2::default();
        assert!(adc.set_adc_channel_input(0, 900));
        assert!(adc.set_adc_channel_input(1, 450));
        driver_start(&mut adc, 1);
        adc.tick();
        assert_eq!(driver_read(&mut adc, 1), 256);
        // No reset: the vendor write cannot clear channel bit 0.
        assert!(adc.set_adc_channel_input(1, 225));
        driver_start(&mut adc, 0);
        adc.tick();
        assert_eq!(adc.read_u32(CONV_CON).unwrap(), SINGLE_MODE | 1);
        assert_eq!(driver_read(&mut adc, 0), 0, "channel 0 did not convert");
        assert_eq!(adc.read_u32(DATA_BASE + 4).unwrap(), 128, "channel 1 did");
        // The vendor sequence: reset the controller, then start.
        adc.on_system_reset();
        driver_start(&mut adc, 0);
        adc.tick();
        assert_eq!(driver_read(&mut adc, 0), 512);
        // The upstream write enable needs no reset.
        upstream_start(&mut adc, 1);
        adc.tick();
        upstream_start(&mut adc, 0);
        assert!(adc.set_adc_channel_input(0, 450));
        adc.tick();
        assert_eq!(adc.read_u32(CONV_CON).unwrap(), SINGLE_MODE);
        assert_eq!(driver_read(&mut adc, 0), 256);
    }

    #[test]
    fn full_scale_clamps_and_an_undriven_pad_reads_zero() {
        let mut adc = RockchipSaradcV2::default();
        assert!(adc.set_adc_channel_input(0, 1800));
        driver_start(&mut adc, 0);
        adc.tick();
        assert_eq!(driver_read(&mut adc, 0), 1023, "VREF is full scale");
        assert!(adc.set_adc_channel_input(0, 3300));
        driver_start(&mut adc, 0);
        adc.tick();
        assert_eq!(driver_read(&mut adc, 0), 1023, "over-range clamps");
        assert!(adc.clear_adc_channel_input(0));
        driver_start(&mut adc, 0);
        adc.tick();
        assert_eq!(driver_read(&mut adc, 0), 0);
    }

    #[test]
    fn end_event_is_masked_by_end_int_en_and_clears_by_writing_one() {
        let mut adc = RockchipSaradcV2::default();
        assert!(adc.set_adc_channel_input(1, 600));
        let val = START | SINGLE_MODE | 1;
        adc.write_u32(CONV_CON, val << 16 | val).unwrap();
        assert!(!adc.tick().irq, "END_INT_EN = 0 masks the request");
        assert_eq!(adc.read_u32(END_INT_ST).unwrap(), END_INT, "still latched");
        adc.write_u32(END_INT_ST, 0).unwrap();
        assert_eq!(adc.read_u32(END_INT_ST).unwrap(), END_INT, "0 keeps it");
        adc.write_u32(END_INT_EN, END_INT << 16 | END_INT).unwrap();
        assert!(adc.irq_level());
        adc.write(END_INT_ST, 1).unwrap();
        assert_eq!(adc.read_u32(END_INT_ST).unwrap(), 0, "byte write clears");
        // Disable through the write enable.
        adc.write_u32(END_INT_EN, END_INT << 16).unwrap();
        assert_eq!(adc.read_u32(END_INT_EN).unwrap(), 0);
        // Byte writes cannot carry a write enable.
        adc.write(CONV_CON, (START | 1) as u8).unwrap();
        adc.write(END_INT_EN, 1).unwrap();
        assert_eq!(adc.read_u32(CONV_CON).unwrap(), SINGLE_MODE | 1);
        assert_eq!(adc.read_u32(END_INT_EN).unwrap(), 0);
    }

    #[test]
    fn an_absent_channel_converts_nothing() {
        let mut adc = RockchipSaradcV2::default();
        driver_start(&mut adc, 5);
        assert!(!adc.tick().irq);
        assert_eq!(adc.read_u32(END_INT_ST).unwrap(), 0);
        assert_eq!(adc.read_u32(CONV_CON).unwrap(), SINGLE_MODE | 5);
    }

    #[test]
    fn rk3588_shape_has_eight_twelve_bit_channels() {
        let mut adc = RockchipSaradcV2::new(8, 12, 1800);
        assert_eq!(adc.adc_channel_count(), Some(8));
        assert!(adc.set_adc_channel_input(7, 900));
        driver_start(&mut adc, 7);
        adc.tick();
        assert_eq!(driver_read(&mut adc, 7), 2048);
        assert!(adc.set_adc_channel_input(7, 1800));
        driver_start(&mut adc, 7);
        adc.tick();
        assert_eq!(driver_read(&mut adc, 7), 4095);
    }

    #[test]
    fn system_reset_keeps_routed_inputs_and_the_reference() {
        let mut adc = RockchipSaradcV2::default();
        assert!(adc.set_adc_channel_input(0, 900));
        assert!(adc.set_adc_reference_voltage_v(3.3));
        driver_start(&mut adc, 0);
        adc.on_system_reset();
        assert_eq!(adc.read_u32(CONV_CON).unwrap(), 0);
        assert_eq!(adc.read_u32(END_INT_EN).unwrap(), 0);
        assert_eq!(adc.read_u32(T_PD_SOC).unwrap(), 0);
        assert!(!adc.tick().irq, "the armed conversion is gone");
        driver_start(&mut adc, 0);
        adc.tick();
        assert_eq!(driver_read(&mut adc, 0), 900 * 1024 / 3300);
    }
}
