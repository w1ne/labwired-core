// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RZ "ADC_C" 12-bit A/D converter (RZ/G2L, RZ/V2L, RZ/G2UL,
//! RZ/A3UL, RZ/G3S): `ADM0`..`ADM3` @0x00..0x0C, `ADINT` @0x20, `ADSTS`
//! @0x24, `ADIVC` @0x28, `ADFIL` @0x2C, `ADCRn` @0x30 + 4n.
//!
//! Sources:
//! - Zephyr `hal_renesas` @445939f6, RZ/G FSP
//!   `R9A07G044L/iodefines/adc_c_iodefine.h` (the register block: `ADM0`..
//!   `ADM3`, 16 reserved bytes, `ADINT`, `ADSTS`, `ADIVC`, `ADFIL`,
//!   `ADCR0`..`ADCR7`) and `iobitmasks/adc_c_iobitmask.h` (`ADM0`: `ADCE` 0,
//!   `ADBSY` 1, `PWDWNB` 2, `SRESB` 15; `ADM1`: `TRG` 0, `TRGIN` 1, `MS` 2,
//!   `RPS` 3, `BS` 4, `EGA` [13:12], `TRGEN` [21:16]; `ADM2.CHSEL` [7:0];
//!   `ADM3`: `ADSMP` [15:0], `ADCMP` [23:16], `ADIL` [31:24]; `ADINT`:
//!   `INTEN` [7:0], `CSEEN` 16, `INTS` 31; `ADSTS`: `INTST` [7:0], `CSEST`
//!   16, `TRGS` 31; `ADIVC.DIVADC` [8:0]; `ADFIL`: `FILONOFF` 0, `FILNUM`
//!   [5:4]; `ADCRn` [11:0]). The R9A08G045S (RZ/G3S) header has the same
//!   block with a 9-bit `CHSEL`/`INTST` (channel 8 is the TSU), hence the
//!   `channels` option.
//! - The same tree's `src/rzg/r_adc_c/r_adc_c.c`: open = `ADM0 = SRESB`,
//!   clear `ADSTS` by writing `TRGS | INTST`, `ADM1`/`ADM3`/`ADIVC`/`ADFIL`,
//!   `ADM0 |= PWDWNB`; scan start = `ADM0 = SRESB | PWDWNB | ADCE`, refused
//!   while `ADCE` reads 1 ("another scan is still in progress (software
//!   trigger)"), so a single scan clears `ADCE` when it ends; `ADM1.RPS` is
//!   the repeat (continuous) mode; stop writes `ADCE` = 0 and waits for it;
//!   `R_ADC_C_StatusGet` reads `ADBSY`; `R_ADC_C_Read` reads `ADCRn`; the
//!   scan-end ISR clears `ADSTS` by writing ones; `ADINT.INTEN` enables the
//!   highest scanned channel only.
//! - Linux v6.12 `drivers/iio/adc/rzg2l_adc.c`: the same offsets and bits,
//!   `RZG2L_ADC_MAX_CHANNELS` 8, `ADCR_AD_MASK` [11:0]; one read = `ADM1`
//!   select mode, `ADM2.CHSEL = BIT(ch)`, `ADINT = CSEEN | BIT(ch)`,
//!   `ADM0 |= ADCE`, the ISR reads `ADSTS.INTST` and each flagged `ADCRn`,
//!   then writes `ADSTS` back to clear it; `CSEST` is the channel-select
//!   error. `r9a07g044.dtsi`: `adc@10059000`, 0x400 bytes, GIC SPI 347.
//! - Zephyr `dts/arm/renesas/rz/rzg/r9a07g044.dtsi` @c85db27d: `adc@40059000`
//!   (the Cortex-M33 view), `renesas,rz-adc-c`, `vref-mv = <1800>`,
//!   `channel-available-mask = <0xff>`.
//!
//! Modelled: with `SRESB` and `PWDWNB` set and `ADM1.TRG` = 0 (software
//! trigger), a write that sets `ADCE` converts every `CHSEL` channel at once,
//! lowest first, into `ADCRn` and sets `INTST` bit n for each. Without
//! `RPS` the scan then ends and `ADCE` reads 0; with `RPS` `ADCE` stays 1 and
//! every tick converts the selection again until firmware clears `ADCE`.
//! `ADBSY` reads 0 (a scan ends inside the write or tick). `ADSTS` bits clear
//! when written 1. The IRQ level is `INTST & INTEN`, or `CSEST & CSEEN`.
//! Code = mV * 4095 / VREF, truncated, clamped at 0xFFF; VREF = `vref_mv`
//! (1800 by default) or the routed reference.
//!
//! Not sourced, kept minimal: an `ADCE` start with `CHSEL` = 0 sets `CSEST`
//! and converts nothing (the bit name and the Linux ISR suggest it; the
//! hardware manual was not available); hardware triggers (`TRG` = 1) arm but
//! never fire, as `ADC_TRG` and the timer sources are not wired; select and
//! scan mode (`MS`) and 1/4-buffer mode (`BS`) convert the same way; the
//! `INTS` pulse/level choice, `ADM3` timing, `ADIVC` and `ADFIL` are stored
//! and have no effect; reset values are 0.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const MAX_CHANNELS: usize = 9;
const CODE_MAX: u32 = (1 << 12) - 1;

const ADM0: u64 = 0x00;
const ADM1: u64 = 0x04;
const ADM2: u64 = 0x08;
const ADM3: u64 = 0x0C;
const ADINT: u64 = 0x20;
const ADSTS: u64 = 0x24;
const ADIVC: u64 = 0x28;
const ADFIL: u64 = 0x2C;
const ADCR0: u64 = 0x30;

const ADCE: u32 = 1 << 0;
const PWDWNB: u32 = 1 << 2;
const SRESB: u32 = 1 << 15;
/// `ADCE`, `PWDWNB`, `SRESB`; `ADBSY` (bit 1) is read-only.
const ADM0_MASK: u32 = ADCE | PWDWNB | SRESB;
const TRG: u32 = 1 << 0;
const RPS: u32 = 1 << 3;
/// `TRG`, `TRGIN`, `MS`, `RPS`, `BS`, `EGA`, `TRGEN`.
const ADM1_MASK: u32 = 0x003F_301F;
const CSEEN: u32 = 1 << 16;
const INTS: u32 = 1 << 31;
const CSEST: u32 = 1 << 16;
const TRGS: u32 = 1 << 31;
const ADIVC_MASK: u32 = 0x1FF;
const ADFIL_MASK: u32 = 0x31;

/// RZ ADC_C: software-started single and repeat scans over `channels`
/// inputs.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RenesasAdcC {
    channels: u8,
    adm0: u32,
    adm1: u32,
    adm2: u32,
    adm3: u32,
    adint: u32,
    adsts: u32,
    adivc: u32,
    adfil: u32,
    results: [u16; MAX_CHANNELS],
    vref_mv: u32,
    /// Routed levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; MAX_CHANNELS],
}

impl Default for RenesasAdcC {
    fn default() -> Self {
        Self::new(8, 1800)
    }
}

impl RenesasAdcC {
    /// `channels`: analog inputs (8 on RZ/G2L, 9 with the RZ/G3S TSU input),
    /// clamped to 1..=9. `vref_mv`: the reference the result is a fraction of.
    pub fn new(channels: u8, vref_mv: u32) -> Self {
        Self {
            channels: channels.clamp(1, MAX_CHANNELS as u8),
            adm0: 0,
            adm1: 0,
            adm2: 0,
            adm3: 0,
            adint: 0,
            adsts: 0,
            adivc: 0,
            adfil: 0,
            results: [0; MAX_CHANNELS],
            vref_mv: vref_mv.max(1),
            inputs_mv: [u16::MAX; MAX_CHANNELS],
        }
    }

    fn channel_mask(&self) -> u32 {
        (1u32 << self.channels) - 1
    }

    fn code(&self, channel: usize) -> u16 {
        let mv = match self.inputs_mv[channel] {
            u16::MAX => 0,
            mv => u32::from(mv),
        };
        (mv * CODE_MAX / self.vref_mv).min(CODE_MAX) as u16
    }

    /// Converts every selected channel, lowest first. Returns false when
    /// nothing is selected (the channel-select error).
    fn scan(&mut self) -> bool {
        let sel = self.adm2 & self.channel_mask();
        if sel == 0 {
            self.adsts |= CSEST;
            return false;
        }
        for channel in 0..usize::from(self.channels) {
            if sel & (1 << channel) != 0 {
                self.results[channel] = self.code(channel);
                self.adsts |= 1 << channel;
            }
        }
        true
    }

    fn write_adm0(&mut self, value: u32) {
        let was_running = self.adm0 & ADCE != 0;
        self.adm0 = value & ADM0_MASK;
        let ready = self.adm0 & (SRESB | PWDWNB) == SRESB | PWDWNB;
        if was_running || self.adm0 & ADCE == 0 || !ready || self.adm1 & TRG != 0 {
            return;
        }
        // A software start; a single scan (or a select error) ends here.
        if !self.scan() || self.adm1 & RPS == 0 {
            self.adm0 &= !ADCE;
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            ADM0 => self.adm0,
            ADM1 => self.adm1,
            ADM2 => self.adm2,
            ADM3 => self.adm3,
            ADINT => self.adint,
            ADSTS => self.adsts,
            ADIVC => self.adivc,
            ADFIL => self.adfil,
            _ => match offset.checked_sub(ADCR0).map(|o| (o / 4) as usize) {
                Some(n) if n < usize::from(self.channels) => u32::from(self.results[n]),
                _ => 0,
            },
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let channels = self.channel_mask();
        match offset {
            ADM0 => self.write_adm0(value),
            ADM1 => self.adm1 = value & ADM1_MASK,
            ADM2 => self.adm2 = value & channels,
            ADM3 => self.adm3 = value,
            ADINT => self.adint = value & (channels | CSEEN | INTS),
            ADSTS => self.adsts &= !(value & (channels | CSEST | TRGS)),
            ADIVC => self.adivc = value & ADIVC_MASK,
            ADFIL => self.adfil = value & ADFIL_MASK,
            _ => {}
        }
    }

    fn irq_level(&self) -> bool {
        self.adsts & self.adint & self.channel_mask() != 0
            || (self.adsts & CSEST != 0 && self.adint & CSEEN != 0)
    }
}

impl Peripheral for RenesasAdcC {
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
        let (channels, inputs, vref) = (self.channels, self.inputs_mv, self.vref_mv);
        *self = Self::new(channels, vref);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        if aligned == ADSTS {
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
        let repeat = self.adm0 & (ADCE | PWDWNB | SRESB) == ADCE | PWDWNB | SRESB
            && self.adm1 & (TRG | RPS) == RPS;
        if repeat && !self.scan() {
            self.adm0 &= !ADCE;
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

    const MS: u32 = 1 << 2;

    fn adc_with(inputs: &[(u8, u16)]) -> RenesasAdcC {
        let mut adc = RenesasAdcC::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv), "channel {ch}");
        }
        adc
    }

    fn code(mv: u32) -> u32 {
        (mv * 4095 / 1800).min(4095)
    }

    /// FSP `r_adc_c_open_sub` with a software single scan in select mode.
    fn fsp_open(adc: &mut RenesasAdcC, adm1: u32) {
        adc.write_u32(ADM0, SRESB).unwrap();
        adc.write_u32(ADSTS, TRGS | 0xFF).unwrap();
        adc.write_u32(ADM1, adm1).unwrap();
        adc.write_u32(ADM3, (0x0E << 16) | 0x578).unwrap();
        adc.write_u32(ADIVC, 4).unwrap();
        adc.write_u32(ADFIL, 0).unwrap();
        adc.write_u32(ADM0, SRESB | PWDWNB).unwrap();
    }

    /// Linux `rzg2l_adc_conversion` + ISR for one channel.
    fn linux_read(adc: &mut RenesasAdcC, ch: u32) -> u32 {
        assert_eq!(adc.read_u32(ADM0).unwrap() & 0b10, 0, "ADBSY");
        let adm1 = adc.read_u32(ADM1).unwrap();
        adc.write_u32(ADM1, (adm1 & !(0x3000 | 0x10 | TRG)) | MS)
            .unwrap();
        let adm2 = adc.read_u32(ADM2).unwrap();
        adc.write_u32(ADM2, (adm2 & !0xFF) | (1 << ch)).unwrap();
        let adint = adc.read_u32(ADINT).unwrap();
        adc.write_u32(ADINT, (adint & !(INTS | 0xFF)) | CSEEN | (1 << ch))
            .unwrap();
        let adm0 = adc.read_u32(ADM0).unwrap();
        adc.write_u32(ADM0, adm0 | ADCE).unwrap();
        assert!(adc.irq_level(), "ch{ch}: scan end requests SPI 347");
        let sts = adc.read_u32(ADSTS).unwrap();
        assert_eq!(sts & 0xFF, 1 << ch, "INTST flags the channel");
        let value = adc.read_u32(ADCR0 + 4 * u64::from(ch)).unwrap() & 0xFFF;
        adc.write_u32(ADSTS, sts).unwrap();
        assert!(!adc.irq_level(), "writing ADSTS back clears it");
        value
    }

    #[test]
    fn reset_values_masks_and_read_only_results() {
        let mut adc = RenesasAdcC::default();
        for offset in (0..0x50).step_by(4) {
            assert_eq!(adc.read_u32(offset).unwrap(), 0, "offset {offset:#x}");
        }
        adc.write_u32(ADM0, !ADCE).unwrap();
        assert_eq!(adc.read_u32(ADM0).unwrap(), PWDWNB | SRESB, "ADBSY reads 0");
        adc.write_u32(ADM1, !0).unwrap();
        assert_eq!(adc.read_u32(ADM1).unwrap(), ADM1_MASK);
        adc.write_u32(ADM2, !0).unwrap();
        assert_eq!(adc.read_u32(ADM2).unwrap(), 0xFF, "8 channels");
        adc.write_u32(ADIVC, !0).unwrap();
        assert_eq!(adc.read_u32(ADIVC).unwrap(), 0x1FF);
        adc.write_u32(ADFIL, !0).unwrap();
        assert_eq!(adc.read_u32(ADFIL).unwrap(), 0x31);
        adc.write_u32(ADCR0, 0xABC).unwrap();
        assert_eq!(adc.read_u32(ADCR0).unwrap(), 0, "ADCRn is read-only");
        assert_eq!(adc.read_u32(ADCR0 + 4 * 8).unwrap(), 0, "no ADCR8");
    }

    #[test]
    fn linux_read_converts_each_input_at_12_bits_against_vref() {
        let mut adc = adc_with(&[(0, 0), (1, 900), (4, 1234), (6, 1799), (7, 2500)]);
        fsp_open(&mut adc, MS);
        assert_eq!(linux_read(&mut adc, 0), 0);
        assert_eq!(linux_read(&mut adc, 1), 2047, "half of VREF");
        assert_eq!(linux_read(&mut adc, 4), code(1234));
        assert_eq!(linux_read(&mut adc, 6), code(1799), "just below VREF");
        assert_eq!(linux_read(&mut adc, 7), 4095, "clamped above VREF");
        assert_eq!(linux_read(&mut adc, 3), 0, "undriven input");
        assert_eq!(adc.read_u32(ADM0).unwrap() & ADCE, 0, "single scan ended");
        // A higher reference lowers every code.
        let mut adc = adc_with(&[(4, 1234)]);
        assert!(adc.set_adc_reference_voltage_v(3.3));
        fsp_open(&mut adc, MS);
        assert_eq!(linux_read(&mut adc, 4), 1234 * 4095 / 3300);
    }

    #[test]
    fn fsp_scan_converts_every_selected_channel_and_irqs_on_the_highest() {
        let mut adc = adc_with(&[(2, 500), (5, 1000), (6, 1500)]);
        fsp_open(&mut adc, 0);
        adc.write_u32(ADM2, (1 << 2) | (1 << 5) | (1 << 6)).unwrap();
        adc.write_u32(ADINT, 1 << 6).unwrap();
        assert_eq!(adc.read_u32(ADM0).unwrap() & ADCE, 0, "ScanStart may go");
        adc.write_u32(ADM0, SRESB | PWDWNB | ADCE).unwrap();
        assert_eq!(adc.read_u32(ADM0).unwrap() & ADCE, 0);
        assert_eq!(adc.read_u32(ADSTS).unwrap(), 0b0110_0100);
        assert!(adc.irq_level());
        for (ch, mv) in [(2, 500), (5, 1000), (6, 1500)] {
            assert_eq!(
                u32::from(adc.read_u16(ADCR0 + 4 * ch).unwrap()),
                code(mv),
                "ADCR{ch}"
            );
        }
        // Only INTEN bits raise the line: clear INTST6, keep 2 and 5.
        adc.write_u32(ADSTS, 1 << 6).unwrap();
        assert_eq!(adc.read_u32(ADSTS).unwrap(), 0b0010_0100);
        assert!(!adc.irq_level(), "INTEN covers channel 6 only");
    }

    #[test]
    fn repeat_mode_rescans_every_tick_until_adce_clears() {
        let mut adc = adc_with(&[(5, 1000)]);
        fsp_open(&mut adc, RPS);
        adc.write_u32(ADM2, 1 << 5).unwrap();
        adc.write_u32(ADM0, SRESB | PWDWNB | ADCE).unwrap();
        assert_ne!(adc.read_u32(ADM0).unwrap() & ADCE, 0, "repeat keeps ADCE");
        assert_eq!(u32::from(adc.read_u16(ADCR0 + 20).unwrap()), code(1000));
        assert!(adc.set_adc_channel_input(5, 1500));
        adc.tick();
        assert_eq!(u32::from(adc.read_u16(ADCR0 + 20).unwrap()), code(1500));
        adc.write_u32(ADM0, SRESB | PWDWNB).unwrap();
        assert!(adc.set_adc_channel_input(5, 200));
        adc.tick();
        assert_eq!(
            u32::from(adc.read_u16(ADCR0 + 20).unwrap()),
            code(1500),
            "stopped"
        );
    }

    #[test]
    fn no_conversion_in_reset_power_down_or_hardware_trigger() {
        let mut adc = adc_with(&[(1, 900)]);
        adc.write_u32(ADM2, 1 << 1).unwrap();
        adc.write_u32(ADM0, PWDWNB | ADCE).unwrap();
        assert_eq!(adc.read_u32(ADCR0 + 4).unwrap(), 0, "SRESB = 0: in reset");
        adc.write_u32(ADM0, 0).unwrap();
        adc.write_u32(ADM0, SRESB | ADCE).unwrap();
        assert_eq!(adc.read_u32(ADCR0 + 4).unwrap(), 0, "PWDWNB = 0");
        adc.write_u32(ADM0, 0).unwrap();
        adc.write_u32(ADM1, TRG).unwrap();
        adc.write_u32(ADM0, SRESB | PWDWNB | ADCE).unwrap();
        adc.tick();
        assert_eq!(adc.read_u32(ADCR0 + 4).unwrap(), 0, "ADC_TRG never fires");
        assert_ne!(adc.read_u32(ADM0).unwrap() & ADCE, 0, "armed");
    }

    #[test]
    fn empty_chsel_flags_the_select_error() {
        let mut adc = RenesasAdcC::default();
        fsp_open(&mut adc, MS);
        adc.write_u32(ADINT, CSEEN).unwrap();
        adc.write_u32(ADM0, SRESB | PWDWNB | ADCE).unwrap();
        assert_eq!(adc.read_u32(ADSTS).unwrap(), CSEST);
        assert_eq!(adc.read_u32(ADM0).unwrap() & ADCE, 0);
        assert!(adc.irq_level(), "CSEEN routes CSEST");
        adc.write(ADSTS + 2, 0x01).unwrap();
        assert_eq!(adc.read_u32(ADSTS).unwrap(), 0, "byte write-one-to-clear");
    }

    #[test]
    fn nine_channel_layout_converts_the_ninth_input() {
        let mut adc = RenesasAdcC::new(9, 1800);
        assert_eq!(adc.adc_channel_count(), Some(9));
        assert!(adc.set_adc_channel_input(8, 900));
        assert!(!adc.set_adc_channel_input(9, 900));
        fsp_open(&mut adc, MS);
        adc.write_u32(ADM2, 1 << 8).unwrap();
        adc.write_u32(ADM0, SRESB | PWDWNB | ADCE).unwrap();
        assert_eq!(adc.read_u32(ADSTS).unwrap(), 1 << 8);
        assert_eq!(adc.read_u32(ADCR0 + 32).unwrap(), 2047);
        // The 8-channel G2L ignores bit 8.
        let mut adc = RenesasAdcC::default();
        assert!(!adc.set_adc_channel_input(8, 900));
        adc.write_u32(ADM2, 0x1FF).unwrap();
        assert_eq!(adc.read_u32(ADM2).unwrap(), 0xFF);
    }

    #[test]
    fn system_reset_keeps_routed_inputs_and_vref() {
        let mut adc = adc_with(&[(4, 1000)]);
        assert!(adc.set_adc_reference_voltage_v(3.3));
        fsp_open(&mut adc, MS);
        adc.write_u32(ADM2, 1 << 4).unwrap();
        adc.write_u32(ADM0, SRESB | PWDWNB | ADCE).unwrap();
        adc.on_system_reset();
        assert_eq!(adc.read_u32(ADM0).unwrap(), 0);
        assert_eq!(adc.read_u32(ADCR0 + 16).unwrap(), 0);
        fsp_open(&mut adc, MS);
        assert_eq!(linux_read(&mut adc, 4), 1000 * 4095 / 3300);
    }
}
