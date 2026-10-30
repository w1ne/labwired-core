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
//! NOT modelled: any effect. A clock enable or a reset hold changes nothing
//! (every peripheral runs ungated and out of reset), clock selects/dividers
//! do not change any simulated frequency, and status words (`FROCLKSTATUS`,
//! `FRO_CAPVAL`, `SYSRSTSTAT`, PLL lock) read back what was written, 0 from
//! reset. The silicon reset values (UM11147) are not reproduced: every word
//! resets to 0.

use crate::{Peripheral, SimResult};

const WINDOW: usize = 0x1000;
const CTRL0: u64 = 0x10;
const SET0: u64 = 0x40;
const CLR0: u64 = 0x70;
const CTRL_WORDS: u64 = 3;

/// One RT500 CLKCTL/RSTCTL bank (see the module docs).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Rt5xxCtl {
    words: Vec<u32>,
    /// False for IOPCTL: no SET/CLR aliases at 0x40/0x70.
    set_clr: bool,
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
            (self.read_word(aligned) & !(0xFF << shift)) | lane
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
