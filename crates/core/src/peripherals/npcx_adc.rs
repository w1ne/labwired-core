// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Nuvoton NPCX9 10-bit ADC at `0x400D_1000` (IRQ 10).
//!
//! Sources:
//! - Zephyr v4.0.0 `soc/nuvoton/npcx/common/reg/reg_def.h` (`struct adc_reg`:
//!   `ADCSTS` @0x00, `ADCCNF` @0x02, `ATCTL` @0x04, `ASCADD` @0x06, `ADCCS`
//!   @0x08, `ADCCS2` @0x0A, `THRCTS` @0x1A, `ADCCNF2` @0x20, `GENDLY` @0x22,
//!   `MEAST` @0x26, and every field position) and
//!   `soc/nuvoton/npcx/npcx9/soc.h` (`CHNDAT(n)` @0x40 + 2n, `THRCTL(n)`
//!   @0x60 + 2n, `THRCTL.THEN` 15, `L_H` 14, `CHNSEL` [13:10], `THRVAL`
//!   [9:0]).
//! - Zephyr v4.0.0 `dts/arm/nuvoton/npcx/npcx.dtsi` + `npcx9.dtsi` (base,
//!   IRQ 10, `vref-mv` 2816, 12 channels, 6 threshold detectors),
//!   `npcx9-pinctrl.dtsi` (pads) and `drivers/adc/adc_npcx.c`: scan mode
//!   (`ADCMD` = 1) converts the `ADCCS` set and the ISR waits on
//!   `ADCSTS.EOCCEV` with `ADCCNF.INTECCEN`; it clears status by writing the
//!   read value back (write-1-to-clear); results are `CHNDAT[9:0]`, fixed
//!   10 bits; a threshold event is `data > THRVAL` for `L_H` = 0 and
//!   `data <= THRVAL` for `L_H` = 1 (`adc_npcx_threshold.h`), raised in
//!   `THRCTS[i]` and interrupting with `THRCTS.THRi_IEN` (bit 8 + i), status
//!   cleared by writing 1 to it alone.
//! - Chromium EC `chip/npcx/adc.c`, `registers-npcx9.h`, `adc_chip.h` @
//!   ad41bf94: channel mode (`ADCMD` = 0) converts `ASCADD.SADDR`, ends in
//!   `ADCSTS.EOCEV` with `ADCCNF.INTECEN`, and the reader checks
//!   `CHNDAT.NEW` (bit 15); `ADCRPTC` makes the scan repetitive until
//!   `ADCCNF.STOP`; `ADC_MAX_VOLT` 2816 over `ADC_READ_MAX + 1` = 1024 codes,
//!   so code = mV * 1024 / 2816; `THRCTL(n)` is 1-based at 0x5E + 2n, the
//!   same addresses as Zephyr's 0-based form; IRQ 10 (`NPCX_IRQ_ADC`).
//!
//! Modelled: `START` with `ADCEN` converts at once (one channel or the whole
//! `ADCCS` set) and self-clears, as does `STOP`; `ADCRPTC` keeps the cycle
//! running and every tick converts it again until `STOP` or `ADCEN` = 0;
//! `CHNDAT.NEW` is set by each conversion; channels 0..11 are pads, an
//! undriven pad converts to 0; the six threshold detectors compare each
//! conversion of their `CHNSEL` channel.
//!
//! Not sourced, so kept minimal: register reset values (0; every driver
//! writes the timing registers before use); which event the other mode
//! raises (channel mode raises only `EOCEV`, scan mode only `EOCCEV`); when
//! `CHNDAT.NEW` clears (it stays set until reset); `SADDR` >= 12 (internal
//! sources) converts nothing and raises no event; `ADCSTS` overflow,
//! `THRCTS.ADC_EVENT` and `ADCTTE` (stored where they have a bit, no
//! behaviour); conversion time.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const CHANNELS: usize = 12;
const THRESHOLDS: usize = 6;
const CODE_MAX: u32 = (1 << 10) - 1;

const ADCSTS: u64 = 0x00;
const ADCCNF: u64 = 0x02;
const ATCTL: u64 = 0x04;
const ASCADD: u64 = 0x06;
const ADCCS: u64 = 0x08;
const ADCCS2: u64 = 0x0A;
const THRCTS: u64 = 0x1A;
const ADCCNF2: u64 = 0x20;
const GENDLY: u64 = 0x22;
const MEAST: u64 = 0x26;
const CHNDAT0: u64 = 0x40;
const THRCTL0: u64 = 0x60;

const EOCEV: u16 = 1 << 0;
const EOCCEV: u16 = 1 << 1;

const ADCEN: u16 = 1 << 0;
const ADCMD_SHIFT: u16 = 1;
const ADCRPTC: u16 = 1 << 3;
const START: u16 = 1 << 4;
const INTECEN: u16 = 1 << 6;
const INTECCEN: u16 = 1 << 7;
const STOP: u16 = 1 << 11;
/// ADCEN, ADCMD, ADCRPTC, ADCTTE, INTECEN, INTECCEN, INTETCEN, INTOVFEN.
/// START and STOP are commands and read 0.
const ADCCNF_STORED: u16 = 0x03EF;

const ATCTL_MASK: u16 = 0x073F;
const SADDR_MASK: u16 = 0x1F;
/// THR1..6 status in [5:0], write-1-to-clear.
const THR_STS: u16 = 0x3F;
/// THR1..6_IEN [13:8] and ADC_WKEN 15.
const THRCTS_STORED: u16 = 0xBF00;
const CHNDAT_NEW: u16 = 1 << 15;
const THEN: u16 = 1 << 15;
const L_H: u16 = 1 << 14;

/// NPCX ADC: channel or scan conversions, repetitive mode, thresholds.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct NpcxAdc {
    adcsts: u16,
    adccnf: u16,
    atctl: u16,
    ascadd: u16,
    adccs: u16,
    adccs2: u16,
    thrcts: u16,
    adccnf2: u16,
    gendly: u16,
    meast: u16,
    chndat: [u16; CHANNELS],
    thrctl: [u16; THRESHOLDS],
    /// A repetitive cycle is running (`START` with `ADCRPTC`, until `STOP`).
    running: bool,
    vref_mv: u32,
    /// Routed levels in millivolts. `u16::MAX` means undriven.
    inputs_mv: [u16; CHANNELS],
}

impl Default for NpcxAdc {
    fn default() -> Self {
        Self::new(2816)
    }
}

impl NpcxAdc {
    /// `vref_mv`: the full-scale reference (Zephyr `vref-mv`, 2816 on NPCX9).
    pub fn new(vref_mv: u32) -> Self {
        Self {
            adcsts: 0,
            adccnf: 0,
            atctl: 0,
            ascadd: 0,
            adccs: 0,
            adccs2: 0,
            thrcts: 0,
            adccnf2: 0,
            gendly: 0,
            meast: 0,
            chndat: [0; CHANNELS],
            thrctl: [0; THRESHOLDS],
            running: false,
            vref_mv: vref_mv.max(1),
            inputs_mv: [u16::MAX; CHANNELS],
        }
    }

    fn code(&self, channel: usize) -> u16 {
        let mv = match self.inputs_mv[channel] {
            u16::MAX => 0,
            mv => u32::from(mv),
        };
        (mv * 1024 / self.vref_mv).min(CODE_MAX) as u16
    }

    fn convert_channel(&mut self, channel: usize) {
        let code = self.code(channel);
        self.chndat[channel] = CHNDAT_NEW | code;
        for (i, ctl) in self.thrctl.iter().enumerate() {
            if ctl & THEN == 0 || usize::from((ctl >> 10) & 0xF) != channel {
                continue;
            }
            let thrval = ctl & 0x3FF;
            let hit = if ctl & L_H == 0 {
                code > thrval
            } else {
                code <= thrval
            };
            if hit {
                self.thrcts |= 1 << i;
            }
        }
    }

    /// One conversion cycle in the current `ADCMD` mode.
    fn cycle(&mut self) {
        match (self.adccnf >> ADCMD_SHIFT) & 3 {
            0 => {
                let channel = usize::from(self.ascadd & SADDR_MASK);
                if channel < CHANNELS {
                    self.convert_channel(channel);
                    self.adcsts |= EOCEV;
                }
            }
            1 => {
                for channel in 0..CHANNELS {
                    if self.adccs & (1 << channel) != 0 {
                        self.convert_channel(channel);
                    }
                }
                self.adcsts |= EOCCEV;
            }
            _ => {}
        }
    }

    fn read_reg(&self, offset: u64) -> u16 {
        match offset {
            ADCSTS => self.adcsts,
            ADCCNF => self.adccnf,
            ATCTL => self.atctl,
            ASCADD => self.ascadd,
            ADCCS => self.adccs,
            ADCCS2 => self.adccs2,
            THRCTS => self.thrcts,
            ADCCNF2 => self.adccnf2,
            GENDLY => self.gendly,
            MEAST => self.meast,
            o if (CHNDAT0..CHNDAT0 + 2 * CHANNELS as u64).contains(&o) => {
                self.chndat[((o - CHNDAT0) / 2) as usize]
            }
            o if (THRCTL0..THRCTL0 + 2 * THRESHOLDS as u64).contains(&o) => {
                self.thrctl[((o - THRCTL0) / 2) as usize]
            }
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u16) {
        match offset {
            ADCSTS => self.adcsts &= !(value & (EOCEV | EOCCEV)),
            ADCCNF => {
                self.adccnf = value & ADCCNF_STORED;
                if value & STOP != 0 || self.adccnf & ADCEN == 0 {
                    self.running = false;
                }
                if value & START != 0 && self.adccnf & ADCEN != 0 {
                    self.cycle();
                    self.running = self.adccnf & ADCRPTC != 0;
                }
            }
            ATCTL => self.atctl = value & ATCTL_MASK,
            ASCADD => self.ascadd = value & SADDR_MASK,
            ADCCS => self.adccs = value,
            ADCCS2 => self.adccs2 = value,
            THRCTS => {
                let cleared = self.thrcts & THR_STS & !(value & THR_STS);
                self.thrcts = cleared | (value & THRCTS_STORED);
            }
            ADCCNF2 => self.adccnf2 = value,
            GENDLY => self.gendly = value,
            MEAST => self.meast = value,
            o if (THRCTL0..THRCTL0 + 2 * THRESHOLDS as u64).contains(&o) => {
                self.thrctl[((o - THRCTL0) / 2) as usize] = value;
            }
            _ => {}
        }
    }

    fn irq_asserted(&self) -> bool {
        (self.adcsts & EOCEV != 0 && self.adccnf & INTECEN != 0)
            || (self.adcsts & EOCCEV != 0 && self.adccnf & INTECCEN != 0)
            || self.thrcts & THR_STS & (self.thrcts >> 8) != 0
    }
}

impl Peripheral for NpcxAdc {
    fn adc_channel_count(&self) -> Option<u8> {
        Some(CHANNELS as u8)
    }

    fn set_adc_channel_input(&mut self, channel: u8, millivolts: u16) -> bool {
        match self.inputs_mv.get_mut(channel as usize) {
            Some(slot) => {
                *slot = millivolts;
                true
            }
            None => false,
        }
    }

    fn clear_adc_channel_input(&mut self, channel: u8) -> bool {
        match self.inputs_mv.get_mut(channel as usize) {
            Some(slot) => {
                *slot = u16::MAX;
                true
            }
            None => false,
        }
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_asserted())
    }

    fn on_system_reset(&mut self) {
        let (inputs, vref) = (self.inputs_mv, self.vref_mv);
        *self = Self::new(vref);
        self.inputs_mv = inputs;
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_reg(offset & !1) >> ((offset & 1) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !1;
        let shift = (offset & 1) * 8;
        let byte = u16::from(value) << shift;
        // Status bits are write-1-to-clear: the other byte writes 0.
        let next = if aligned == ADCSTS || aligned == THRCTS {
            let keep = if aligned == THRCTS {
                self.thrcts & THRCTS_STORED & !(0xFF << shift)
            } else {
                0
            };
            keep | byte
        } else {
            (self.read_reg(aligned) & !(0xFF << shift)) | byte
        };
        self.write_reg(aligned, next);
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(self.read_reg(offset & !1))
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write_reg(offset & !1, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.running {
            self.cycle();
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

    const SCAN: u16 = 1 << ADCMD_SHIFT;

    fn adc_with(inputs: &[(u8, u16)]) -> NpcxAdc {
        let mut adc = NpcxAdc::default();
        for &(ch, mv) in inputs {
            assert!(adc.set_adc_channel_input(ch, mv), "channel {ch}");
        }
        adc
    }

    fn code(mv: u32) -> u16 {
        (mv * 1024 / 2816).min(1023) as u16
    }

    /// Chromium EC `start_single_and_wait` + `adc_read_channel`.
    fn ec_read(adc: &mut NpcxAdc, ch: u16) -> Option<u16> {
        adc.write_u16(ADCCNF, ADCEN).unwrap();
        let cnf = adc.read_u16(ADCCNF).unwrap();
        adc.write_u16(ADCCNF, cnf | STOP).unwrap();
        let cnf = adc.read_u16(ADCCNF).unwrap() & !(3 << ADCMD_SHIFT) & !ADCRPTC;
        adc.write_u16(ADCCNF, cnf).unwrap();
        adc.write_u16(ASCADD, ch).unwrap();
        let sts = adc.read_u16(ADCSTS).unwrap();
        adc.write_u16(ADCSTS, sts | EOCEV).unwrap();
        let cnf = adc.read_u16(ADCCNF).unwrap();
        adc.write_u16(ADCCNF, cnf | INTECEN).unwrap();
        let cnf = adc.read_u16(ADCCNF).unwrap();
        adc.write_u16(ADCCNF, cnf | START).unwrap();
        if adc.read_u16(ADCSTS).unwrap() & EOCEV == 0 {
            return None;
        }
        assert!(adc.irq_line_level().unwrap(), "INTECEN raises IRQ 10");
        let dat = adc.read_u16(CHNDAT0 + 2 * u64::from(ch)).unwrap();
        adc.write_u16(ADCSTS, EOCEV).unwrap();
        assert!(!adc.irq_line_level().unwrap(), "EOCEV is write-1-to-clear");
        (dat & CHNDAT_NEW != 0).then_some(dat & 0x3FF)
    }

    #[test]
    fn ec_channel_mode_converts_each_pad_at_10_bits_against_2816_mv() {
        let mut adc = adc_with(&[(0, 0), (1, 1408), (5, 2000), (11, 2815), (6, 3300)]);
        assert_eq!(ec_read(&mut adc, 0), Some(0));
        assert_eq!(ec_read(&mut adc, 1), Some(512), "half scale");
        assert_eq!(ec_read(&mut adc, 5), Some(code(2000)));
        assert_eq!(ec_read(&mut adc, 11), Some(1023), "just below VREF");
        assert_eq!(ec_read(&mut adc, 6), Some(1023), "clamped above VREF");
        assert_eq!(ec_read(&mut adc, 7), Some(0), "undriven pad");
        // SADDR 12..31 is not a pad: nothing converts, no event.
        assert_eq!(ec_read(&mut adc, 12), None);
        // START without ADCEN converts nothing.
        let mut adc = adc_with(&[(2, 1000)]);
        adc.write_u16(ASCADD, 2).unwrap();
        adc.write_u16(ADCCNF, START).unwrap();
        assert_eq!(adc.read_u16(ADCSTS).unwrap(), 0);
        assert_eq!(adc.read_u16(CHNDAT0 + 4).unwrap(), 0);
        assert_eq!(adc.read_u16(ADCCNF).unwrap(), 0, "START reads 0");
    }

    /// Zephyr `adc_npcx_start_scan` + ISR.
    #[test]
    fn zephyr_scan_converts_the_adccs_set_and_ends_in_eoccev() {
        let mut adc = adc_with(&[(5, 2400), (6, 650), (7, 100)]);
        let cnf = adc.read_u16(ADCCNF).unwrap();
        adc.write_u16(ADCCNF, cnf | ADCEN | STOP).unwrap();
        let sts = adc.read_u16(ADCSTS).unwrap();
        adc.write_u16(ADCSTS, sts | EOCCEV).unwrap();
        adc.write_u16(ADCCS, (1 << 5) | (1 << 6)).unwrap();
        let cnf = adc.read_u16(ADCCNF).unwrap();
        adc.write_u16(ADCCNF, cnf | SCAN | INTECCEN).unwrap();
        assert!(!adc.irq_line_level().unwrap());
        let cnf = adc.read_u16(ADCCNF).unwrap();
        adc.write_u16(ADCCNF, cnf | START).unwrap();
        let status = adc.read_u16(ADCSTS).unwrap();
        assert_eq!(status, EOCCEV, "scan raises only the cycle event");
        assert!(adc.irq_line_level().unwrap());
        adc.write_u16(ADCSTS, status).unwrap();
        assert!(!adc.irq_line_level().unwrap());
        assert_eq!(adc.read_u16(CHNDAT0 + 10).unwrap(), CHNDAT_NEW | code(2400));
        assert_eq!(adc.read_u16(CHNDAT0 + 12).unwrap(), CHNDAT_NEW | code(650));
        assert_eq!(adc.read_u16(CHNDAT0 + 14).unwrap(), 0, "CH7 not in ADCCS");
        // One-shot: a tick does not convert again.
        adc.set_adc_channel_input(5, 100);
        adc.tick();
        assert_eq!(adc.read_u16(CHNDAT0 + 10).unwrap() & 0x3FF, code(2400));
    }

    #[test]
    fn repetitive_scan_follows_the_input_until_stop_and_thresholds_fire() {
        let mut adc = adc_with(&[(3, 500)]);
        // Zephyr adc_npcx_threshold: THR2 on CH3, event when data > 600 mV.
        let thr = code(600);
        adc.write_u16(THRCTL0 + 2, THEN | (3 << 10) | thr).unwrap();
        // THR3 on CH3, L_H = 1: event when data <= 400 mV.
        adc.write_u16(THRCTL0 + 4, THEN | L_H | (3 << 10) | code(400))
            .unwrap();
        adc.write_u16(THRCTS, (1 << 9) | (1 << 10)).unwrap();
        adc.write_u16(ADCCS, 1 << 3).unwrap();
        adc.write_u16(ADCCNF, ADCEN | SCAN | ADCRPTC | START)
            .unwrap();
        assert_eq!(adc.read_u16(CHNDAT0 + 6).unwrap() & 0x3FF, code(500));
        assert_eq!(adc.read_u16(THRCTS).unwrap() & THR_STS, 0);
        assert!(!adc.irq_line_level().unwrap());
        adc.set_adc_channel_input(3, 700);
        assert!(adc.tick().irq, "THR2 interrupts");
        assert_eq!(adc.read_u16(CHNDAT0 + 6).unwrap() & 0x3FF, code(700));
        assert_eq!(adc.read_u16(THRCTS).unwrap() & THR_STS, 1 << 1);
        // The driver clears THR2 alone, keeping the IEN bits.
        let v = (adc.read_u16(THRCTS).unwrap() & !THR_STS) | (1 << 1);
        adc.write_u16(THRCTS, v).unwrap();
        assert_eq!(adc.read_u16(THRCTS).unwrap(), (1 << 9) | (1 << 10));
        assert!(!adc.irq_line_level().unwrap());
        // Down to 300 mV: THR3 (<=) fires, THR2 does not.
        adc.set_adc_channel_input(3, 300);
        adc.tick();
        assert_eq!(adc.read_u16(THRCTS).unwrap() & THR_STS, 1 << 2);
        // THR1 has no IEN bit set, yet still latches its status.
        adc.write_u16(THRCTS, 1 << 2).unwrap();
        adc.write_u16(THRCTL0, THEN | L_H | (3 << 10) | 1023)
            .unwrap();
        adc.tick();
        assert_eq!(adc.read_u16(THRCTS).unwrap() & THR_STS, (1 << 0) | (1 << 2));
        adc.write_u16(THRCTS, THR_STS).unwrap();
        // THEN = 0 disables a detector.
        adc.write_u16(THRCTL0, 0).unwrap();
        adc.write_u16(THRCTL0 + 4, 0).unwrap();
        adc.tick();
        assert_eq!(adc.read_u16(THRCTS).unwrap() & THR_STS, 0);
        // STOP ends the repetitive cycle.
        let cnf = adc.read_u16(ADCCNF).unwrap();
        adc.write_u16(ADCCNF, cnf | STOP).unwrap();
        adc.set_adc_channel_input(3, 2000);
        adc.tick();
        assert_eq!(adc.read_u16(CHNDAT0 + 6).unwrap() & 0x3FF, code(300));
        // Restart, then ADCEN = 0 also stops it.
        let cnf = adc.read_u16(ADCCNF).unwrap();
        adc.write_u16(ADCCNF, cnf | START).unwrap();
        assert_eq!(adc.read_u16(CHNDAT0 + 6).unwrap() & 0x3FF, code(2000));
        adc.write_u16(ADCCNF, SCAN | ADCRPTC).unwrap();
        adc.set_adc_channel_input(3, 50);
        adc.tick();
        assert_eq!(adc.read_u16(CHNDAT0 + 6).unwrap() & 0x3FF, code(2000));
    }

    #[test]
    fn byte_access_masks_routing_and_reset() {
        let mut adc = adc_with(&[(4, 1000)]);
        assert!(!adc.set_adc_channel_input(12, 1));
        assert!(adc.clear_adc_channel_input(4));
        adc.set_adc_channel_input(4, 1000);
        assert_eq!(adc.adc_channel_count(), Some(12));
        // Byte writes: ASCADD then ADCCNF low byte with START.
        adc.write(ASCADD, 4).unwrap();
        adc.write(ADCCNF, (ADCEN | START) as u8).unwrap();
        assert_eq!(adc.read(ADCSTS).unwrap(), EOCEV as u8);
        assert_eq!(adc.read_u16(CHNDAT0 + 8).unwrap(), CHNDAT_NEW | code(1000));
        // A byte write to ADCCNF's high byte keeps ADCEN and does not restart.
        adc.set_adc_channel_input(4, 10);
        adc.write(ADCCNF + 1, 0).unwrap();
        assert_eq!(adc.read_u16(ADCCNF).unwrap(), ADCEN);
        assert_eq!(adc.read_u16(CHNDAT0 + 8).unwrap() & 0x3FF, code(1000));
        // ADCSTS high-byte write clears nothing; low byte clears EOCEV.
        adc.write(ADCSTS + 1, 0xFF).unwrap();
        assert_eq!(adc.read_u16(ADCSTS).unwrap(), EOCEV);
        adc.write(ADCSTS, EOCEV as u8).unwrap();
        assert_eq!(adc.read_u16(ADCSTS).unwrap(), 0);
        // Masks from reg_def.h field widths.
        adc.write_u16(ATCTL, 0xFFFF).unwrap();
        adc.write_u16(ASCADD, 0xFFFF).unwrap();
        adc.write_u16(ADCCNF, !START).unwrap();
        assert_eq!(adc.read_u16(ATCTL).unwrap(), 0x073F);
        assert_eq!(adc.read_u16(ASCADD).unwrap(), 0x1F);
        assert_eq!(adc.read_u16(ADCCNF).unwrap(), ADCCNF_STORED);
        // Zephyr adc_npcx_init speed values are stored.
        adc.write_u16(ADCCNF2, 0x8B07).unwrap();
        adc.write_u16(GENDLY, 0x0100).unwrap();
        adc.write_u16(MEAST, 0x0001).unwrap();
        assert_eq!(adc.read_u16(ADCCNF2).unwrap(), 0x8B07);
        assert_eq!(adc.read_u16(GENDLY).unwrap(), 0x0100);
        assert_eq!(adc.read_u16(MEAST).unwrap(), 0x0001);
        adc.on_system_reset();
        for off in [ADCSTS, ADCCNF, ATCTL, ASCADD, ADCCNF2, CHNDAT0 + 8] {
            assert_eq!(adc.read_u16(off).unwrap(), 0, "offset {off:#x}");
        }
        assert_eq!(ec_read(&mut adc, 4), Some(code(10)), "inputs survive");
    }
}
