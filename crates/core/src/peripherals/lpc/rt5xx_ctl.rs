// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! i.MX RT500 clock/reset control register banks: CLKCTL0 (0x4000_1000),
//! CLKCTL1 (0x4002_1000), RSTCTL0 (0x4000_0000) and RSTCTL1 (0x4002_0000).
//!
//! Source: the MCUXpresso SDK device header `MIMXRT595S_cm33.h`
//! (`CLKCTL0_Type`, `CLKCTL1_Type`, `RSTCTL0_Type`, `RSTCTL1_Type`). All four
//! share one shape at the front: three control words at 0x10/0x14/0x18
//! (`PSCCTLn` clock enables, `PRSTCTLn` reset holds), their SET aliases at
//! 0x40/0x44/0x48 and CLR aliases at 0x70/0x74/0x78. Everything else in the
//! 4 KiB window (clock selects, dividers, PLL/oscillator controls, FRG and
//! FLEXCOMM clock selects) is a plain read/write word.
//!
//! The same plain bank, without the SET/CLR aliases ([`Rt5xxCtl::plain`]),
//! stands in for IOPCTL (0x4000_4000): `PIO[port][pin]` @ `0x80·port + 4·pin`
//! (FSEL \[3:0\], PUPDENA \[4\], PUPDSEL \[5\], IBENA \[6\], SLEWRATE,
//! FULLDRIVE, AMENA, ODENA, IIENA), stored only: a pad's function select,
//! pull and input buffer have no effect on the GPIO or FLEXCOMM models.
//!
//! Modelled: storage of every word, and the SET/CLR aliases acting on the
//! three control words (the aliases themselves read 0).
//!
//! CLKCTL0 / CLKCTL1 ([`Rt5xxCtl::clkctl0`] / [`Rt5xxCtl::clkctl1`]) also
//! answer the hand-shakes the SDK `fsl_clock.c` polls, as if every clock
//! settled at once (field masks from `MIMXRT595S_cm33.h`):
//! - every `*DIV` divider register's `REQFLAG` (bit 31) reads 0: a divider
//!   change has already taken effect (`CLOCK_SetClkDiv`);
//! - CLKCTL0 `FROCLKSTATUS.CLK_OK` (0x10C bit 0) and `LPOSCCTL0.CLKRDY`
//!   (0x190 bit 31) read 1 (`CLOCK_EnableFroClk`, `CLOCK_EnableLpOscClk`);
//! - in CLKCTL0 `SYSPLL0PFD` / CLKCTL1 `AUDIOPLL0PFD` (0x218), each
//!   `PFDn_CLKRDY` reads 1 exactly while its `PFDn_CLKGATE` is clear
//!   (`CLOCK_InitSysPfd` / `CLOCK_InitAudioPfd`); the stored CLKRDY bit is
//!   ignored, so its write-1 acknowledge is harmless;
//! - CLKCTL0 FRO tuner: while `FRO_CONTROL.ENA_TUNE` (0x80 bit 31) is set,
//!   `FRO_CAPVAL` (0x84) reads `DATA_VALID` with a capture equal to
//!   `FRO_CONTROL.EXP_COUNT` — the simulated FRO runs exactly at its nominal
//!   frequency, so `CLOCK_FroTuneToFreq` / `CLOCK_EnableFroTuning` see an
//!   in-range count on the first capture and the trim is never stepped.
//!
//! NOT modelled: any effect. A clock enable or a reset hold changes nothing
//! (every peripheral runs ungated and out of reset), clock selects/dividers
//! do not change any simulated frequency, PLL lock time is not simulated
//! (the SDK only waits a delay for it), and the remaining status words
//! (`FRO_CAPVAL`, `SYSRSTSTAT`) read back what was written, 0 from reset.
//! The silicon reset values (UM11147) are not reproduced: every word resets
//! to 0.

use crate::{Peripheral, SimResult};

const WINDOW: usize = 0x1000;
const CTRL0: u64 = 0x10;
const SET0: u64 = 0x40;
const CLR0: u64 = 0x70;
const CTRL_WORDS: u64 = 3;

/// `*DIV.REQFLAG`: a divider change is still in progress.
const DIV_REQFLAG: u32 = 1 << 31;
/// CLKCTL0 `*DIV` registers (`MIMXRT595S_cm33.h` `CLKCTL0_Type`).
const CLKCTL0_DIVS: &[u64] = &[
    0x130, 0x240, 0x244, 0x248, 0x24C, 0x400, 0x500, 0x504, 0x624, 0x634, 0x644, 0x664, 0x684,
    0x694, 0x6D8, 0x734, 0x764, 0x774, 0x77C, 0x780, 0x794, 0x7A4,
];
/// CLKCTL1 `*DIV` registers (`CLKCTL1_Type`).
const CLKCTL1_DIVS: &[u64] = &[
    0x240, 0x400, 0x740, 0x760, 0x784, 0x7C4, 0x7E8, 0x808, 0x80C, 0x810, 0x834,
];
const CLKCTL0_FROCLKSTATUS: u64 = 0x10C;
const FROCLKSTATUS_CLK_OK: u32 = 1 << 0;
const CLKCTL0_LPOSCCTL0: u64 = 0x190;
const LPOSCCTL0_CLKRDY: u32 = 1 << 31;
/// CLKCTL0 `SYSPLL0PFD` / CLKCTL1 `AUDIOPLL0PFD`.
const PLL0PFD: u64 = 0x218;
/// `PFDn_CLKRDY` (bit 6 of each byte); `PFDn_CLKGATE` is the bit above it.
const PFD_CLKRDY: u32 = 0x4040_4040;
const CLKCTL0_FRO_CONTROL: u64 = 0x80;
const FRO_CONTROL_ENA_TUNE: u32 = 1 << 31;
const FRO_CONTROL_EXP_COUNT: u32 = 0xFFFF;
const CLKCTL0_FRO_CAPVAL: u64 = 0x84;
const FRO_CAPVAL_DATA_VALID: u32 = 1 << 31;

/// Which RT500 bank this is: decides the read-side status behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
enum Bank {
    /// RSTCTL0/1, IOPCTL: stored words only.
    #[default]
    Plain,
    Clkctl0,
    Clkctl1,
}

/// One RT500 CLKCTL/RSTCTL bank (see the module docs).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Rt5xxCtl {
    words: Vec<u32>,
    /// False for IOPCTL: no SET/CLR aliases at 0x40/0x70.
    set_clr: bool,
    #[serde(default)]
    bank: Bank,
}

impl Default for Rt5xxCtl {
    fn default() -> Self {
        Self::new()
    }
}

impl Rt5xxCtl {
    /// A CLKCTL/RSTCTL bank, with the SET/CLR aliases.
    pub fn new() -> Self {
        Self {
            words: vec![0; WINDOW / 4],
            set_clr: true,
            bank: Bank::Plain,
        }
    }

    /// CLKCTL0 (0x4000_1000), with its clock-ready status bits.
    pub fn clkctl0() -> Self {
        Self {
            bank: Bank::Clkctl0,
            ..Self::new()
        }
    }

    /// CLKCTL1 (0x4002_1000), with its clock-ready status bits.
    pub fn clkctl1() -> Self {
        Self {
            bank: Bank::Clkctl1,
            ..Self::new()
        }
    }

    /// A plain stored bank (IOPCTL).
    pub fn plain() -> Self {
        Self {
            set_clr: false,
            ..Self::new()
        }
    }

    fn alias(&self, offset: u64) -> Option<(u64, bool)> {
        if !self.set_clr {
            None
        } else if (SET0..SET0 + 4 * CTRL_WORDS).contains(&offset) {
            Some((CTRL0 + (offset - SET0), true))
        } else if (CLR0..CLR0 + 4 * CTRL_WORDS).contains(&offset) {
            Some((CTRL0 + (offset - CLR0), false))
        } else {
            None
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        if self.alias(offset).is_some() {
            return 0;
        }
        let stored = self.stored(offset);
        let divs = match self.bank {
            Bank::Plain => return stored,
            Bank::Clkctl0 => CLKCTL0_DIVS,
            Bank::Clkctl1 => CLKCTL1_DIVS,
        };
        if divs.contains(&offset) {
            return stored & !DIV_REQFLAG;
        }
        match (self.bank, offset) {
            (_, PLL0PFD) => {
                let ungated = !(stored >> 1) & PFD_CLKRDY;
                (stored & !PFD_CLKRDY) | ungated
            }
            (Bank::Clkctl0, CLKCTL0_FROCLKSTATUS) => stored | FROCLKSTATUS_CLK_OK,
            (Bank::Clkctl0, CLKCTL0_LPOSCCTL0) => stored | LPOSCCTL0_CLKRDY,
            (Bank::Clkctl0, CLKCTL0_FRO_CAPVAL) => {
                let control = self.stored(CLKCTL0_FRO_CONTROL);
                if control & FRO_CONTROL_ENA_TUNE != 0 {
                    FRO_CAPVAL_DATA_VALID | (control & FRO_CONTROL_EXP_COUNT)
                } else {
                    stored
                }
            }
            _ => stored,
        }
    }

    fn stored(&self, offset: u64) -> u32 {
        self.words.get((offset / 4) as usize).copied().unwrap_or(0)
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match self.alias(offset) {
            Some((target, true)) => self.words[(target / 4) as usize] |= value,
            Some((target, false)) => self.words[(target / 4) as usize] &= !value,
            None => {
                if let Some(word) = self.words.get_mut((offset / 4) as usize) {
                    *word = value;
                }
            }
        }
    }
}

impl Peripheral for Rt5xxCtl {
    /// A pure register bank: no `tick()`, no IRQ, DMA or event.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let lane = u32::from(value) << shift;
        let word = if self.alias(aligned).is_some() {
            lane
        } else {
            (self.stored(aligned) & !(0xFF << shift)) | lane
        };
        self.write_word(aligned, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset & !3, value);
        Ok(())
    }

    fn on_node_reset(&mut self) {
        self.words.fill(0);
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        *self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("Rt5xxCtl snapshot decode: {error}"))
        })?;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize Rt5xxCtl")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        *self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "Rt5xxCtl runtime snapshot decode: {error}"
            ))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // CLKCTL1 PSCCTL0 bit 8 = FC0 clock; RSTCTL1 PRSTCTL0 bit 8 = FLEXCOMM0
    // reset (MIMXRT595S_cm33.h).
    const FC0: u32 = 1 << 8;

    #[test]
    fn set_and_clr_aliases_act_on_the_control_words() {
        let mut c = Rt5xxCtl::new();
        c.write_u32(0x40, FC0).unwrap();
        c.write_u32(0x44, 0x3).unwrap();
        assert_eq!(c.read_u32(0x10).unwrap(), FC0);
        assert_eq!(c.read_u32(0x14).unwrap(), 0x3);
        assert_eq!(c.read_u32(0x40).unwrap(), 0);
        c.write_u32(0x74, 0x1).unwrap();
        assert_eq!(c.read_u32(0x14).unwrap(), 0x2);
        c.write_u32(0x70, FC0).unwrap();
        assert_eq!(c.read_u32(0x10).unwrap(), 0);
    }

    #[test]
    fn plain_words_store_and_narrow_set_does_not_rmw() {
        let mut c = Rt5xxCtl::new();
        // CLKCTL1 FLEXCOMM[0].FCFCLKSEL @ 0x508.
        c.write_u32(0x508, 0x3).unwrap();
        assert_eq!(c.read_u32(0x508).unwrap(), 0x3);
        c.write_u32(0x10, 0xFF).unwrap();
        c.write(0x41, 0x01).unwrap();
        assert_eq!(c.read_u32(0x10).unwrap(), 0x1FF);
        c.write(0x70, 0x0F).unwrap();
        assert_eq!(c.read_u32(0x10).unwrap(), 0x1F0);
    }

    #[test]
    fn plain_bank_has_no_aliases() {
        // IOPCTL PIO0_16 @ 0x40 and PIO0_28 @ 0x70 are ordinary words.
        let mut c = Rt5xxCtl::plain();
        c.write_u32(0x40, 0x41).unwrap();
        c.write_u32(0x70, 0x01).unwrap();
        assert_eq!(c.read_u32(0x40).unwrap(), 0x41);
        assert_eq!(c.read_u32(0x70).unwrap(), 0x01);
        assert_eq!(c.read_u32(0x10).unwrap(), 0);
    }

    #[test]
    fn clkctl_dividers_never_report_a_pending_change() {
        let mut c0 = Rt5xxCtl::clkctl0();
        // SYSCPUAHBCLKDIV @ 0x400: CLOCK_SetClkDiv writes DIV, polls REQFLAG.
        c0.write_u32(0x400, DIV_REQFLAG | 1).unwrap();
        assert_eq!(c0.read_u32(0x400).unwrap(), 1);
        let mut c1 = Rt5xxCtl::clkctl1();
        // FRGPLLCLKDIV @ 0x760.
        c1.write_u32(0x760, DIV_REQFLAG | 10).unwrap();
        assert_eq!(c1.read_u32(0x760).unwrap(), 10);
        // Not a divider: bit 31 is stored as written.
        c1.write_u32(0x508, DIV_REQFLAG).unwrap();
        assert_eq!(c1.read_u32(0x508).unwrap(), DIV_REQFLAG);
        // The plain bank (RSTCTL) has no dividers.
        let mut r = Rt5xxCtl::new();
        r.write_u32(0x400, DIV_REQFLAG).unwrap();
        assert_eq!(r.read_u32(0x400).unwrap(), DIV_REQFLAG);
    }

    #[test]
    fn clkctl0_fro_and_lposc_report_ready() {
        let c0 = Rt5xxCtl::clkctl0();
        assert_eq!(c0.read_u32(CLKCTL0_FROCLKSTATUS).unwrap() & 1, 1);
        assert_eq!(c0.read_u32(CLKCTL0_LPOSCCTL0).unwrap() >> 31, 1);
        // CLKCTL1 has neither at those offsets.
        let c1 = Rt5xxCtl::clkctl1();
        assert_eq!(c1.read_u32(CLKCTL0_LPOSCCTL0).unwrap(), 0);
    }

    #[test]
    fn pfd_clkrdy_follows_its_clock_gate() {
        let mut c0 = Rt5xxCtl::clkctl0();
        // CLOCK_InitSysPfd(kCLOCK_Pfd2, 24): gate PFD2, then ungate with
        // divider 24 and wait for PFD2_CLKRDY (bit 22).
        c0.write_u32(PLL0PFD, 0x8080_8080).unwrap();
        assert_eq!(c0.read_u32(PLL0PFD).unwrap() & PFD_CLKRDY, 0);
        c0.write_u32(PLL0PFD, 0x8000_8080 | (24 << 16)).unwrap();
        let v = c0.read_u32(PLL0PFD).unwrap();
        assert_eq!(v & PFD_CLKRDY, 1 << 22);
        assert_eq!((v >> 16) & 0x3F, 24);
        // The write-1 acknowledge of CLKRDY does not latch it.
        c0.write_u32(PLL0PFD, v | (1 << 22) | (1 << 6)).unwrap();
        assert_eq!(c0.read_u32(PLL0PFD).unwrap() & PFD_CLKRDY, 1 << 22);
    }

    #[test]
    fn fro_tuner_captures_the_expected_count() {
        let mut c0 = Rt5xxCtl::clkctl0();
        assert_eq!(c0.read_u32(CLKCTL0_FRO_CAPVAL).unwrap(), 0);
        // CLOCK_FroTuneToFreq(192 MHz) on a 24 MHz crystal: EXP_COUNT 4094,
        // UP 6, LOW 5, ENA_TUNE.
        let control = FRO_CONTROL_ENA_TUNE | (5 << 21) | (6 << 16) | 4094;
        c0.write_u32(CLKCTL0_FRO_CONTROL, control).unwrap();
        assert_eq!(
            c0.read_u32(CLKCTL0_FRO_CAPVAL).unwrap(),
            FRO_CAPVAL_DATA_VALID | 4094
        );
        c0.write_u32(CLKCTL0_FRO_CONTROL, control & !FRO_CONTROL_ENA_TUNE)
            .unwrap();
        assert_eq!(c0.read_u32(CLKCTL0_FRO_CAPVAL).unwrap(), 0);
    }

    #[test]
    fn byte_writes_do_not_latch_status_bits() {
        let mut c0 = Rt5xxCtl::clkctl0();
        c0.write(CLKCTL0_LPOSCCTL0, 0x5).unwrap();
        assert_eq!(c0.stored(CLKCTL0_LPOSCCTL0), 0x5);
    }

    #[test]
    fn reset_clears_everything() {
        let mut c = Rt5xxCtl::new();
        c.write_u32(0x18, 7).unwrap();
        c.on_node_reset();
        assert_eq!(c.read_u32(0x18).unwrap(), 0);
        // Offsets past the window read 0 and ignore writes.
        c.write_u32(0x1000, 1).unwrap();
        assert_eq!(c.read_u32(0x1000).unwrap(), 0);
    }
}
