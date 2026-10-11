// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RA pin function select block: `PmnPFS` and `PWPR`.
//!
//! RA4M1 (R01UH0887 §19.2.5, §19.5): one 32-bit `PmnPFS` per pin at
//! `0x40·m + 4·n` from the PFS base (0x4004_0800), and the write-protect
//! register `PWPR` at +0x503 (in `PMISC`). `PmnPFS` holds the pin's input
//! pull-up (PCR, bit 4), which the GPIO port reports and reads as a pull
//! through the bus's pad-control link (`crate::pins::PadControl::RaPfs`).
//!
//! Write protection is modelled because FSP's `R_BSP_PinAccessEnable` depends
//! on it: a `PmnPFS` store lands only while `PWPR.PFSWE` is 1, and `PFSWE` is
//! writable only while `PWPR.B0WI` is 0 (both reset to the protected state).
//!
//! NOT modelled: `PmnPFS` PODR/PDR (bits 0 and 2) are the same flip-flops as
//! the port's `PCNTR1` on silicon, but here they are stored only and do not
//! move the GPIO port; PIDR (bit 1) reads 0, not the pad; PSEL/PMR (the
//! peripheral mux) has no routing effect.

use crate::{Peripheral, SimResult};
use std::any::Any;

/// `PmnPFS` words for ports 0..9, 16 pins each.
const PFS_WORDS: usize = 10 * 16;
/// `PWPR` offset from the PFS base (`PMISC.PWPR`, 0x4004_0D03).
const PWPR: u64 = 0x503;
const PWPR_B0WI: u8 = 1 << 7;
const PWPR_PFSWE: u8 = 1 << 6;

/// The RA pin function select registers.
#[derive(Debug, serde::Serialize)]
pub struct RaPfs {
    #[serde(skip)]
    pfs: Vec<u32>,
    pwpr: u8,
}

impl Default for RaPfs {
    fn default() -> Self {
        Self::new()
    }
}

impl RaPfs {
    pub fn new() -> Self {
        Self {
            pfs: vec![0; PFS_WORDS],
            // B0WI set: PFSWE locked, PFS writes refused.
            pwpr: PWPR_B0WI,
        }
    }

    fn word_index(offset: u64) -> Option<usize> {
        let k = (offset / 4) as usize;
        (k < PFS_WORDS).then_some(k)
    }
}

impl RaPfs {
    /// Where a `width`-byte access at `offset` lands: the word, and the bit
    /// shift inside it. `PmnPFS_HA` (+2, 16-bit) and `PmnPFS_BY` (+3, 8-bit)
    /// are aliases of the LOW half / byte of `PmnPFS`, not its upper bytes
    /// (R01UH0887 §19.2.5); every other access is little-endian.
    fn lane(offset: u64, width: u64) -> Option<(usize, u64)> {
        let k = Self::word_index(offset)?;
        let shift = match (offset % 4, width) {
            (2, 2) | (3, 1) => 0,
            (lane, _) => lane * 8,
        };
        Some((k, shift))
    }

    fn load(&self, offset: u64, width: u64) -> u32 {
        let mask = if width >= 4 {
            u32::MAX
        } else {
            (1u32 << (width * 8)) - 1
        };
        match Self::lane(offset, width) {
            Some((k, shift)) => (self.pfs[k] >> shift) & mask,
            None => 0,
        }
    }

    fn store(&mut self, offset: u64, width: u64, value: u32) {
        if self.pwpr & PWPR_PFSWE == 0 {
            return;
        }
        let mask = if width >= 4 {
            u32::MAX
        } else {
            (1u32 << (width * 8)) - 1
        };
        if let Some((k, shift)) = Self::lane(offset, width) {
            self.pfs[k] = (self.pfs[k] & !(mask << shift)) | ((value & mask) << shift);
        }
    }
}

impl Peripheral for RaPfs {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        if offset == PWPR {
            return Ok(self.pwpr);
        }
        Ok(self.load(offset, 1) as u8)
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok(self.load(offset, 2) as u16)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.load(offset & !3, 4))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset == PWPR {
            // PFSWE is writable only while B0WI is (already) 0.
            let pfswe = if self.pwpr & PWPR_B0WI == 0 {
                value & PWPR_PFSWE
            } else {
                self.pwpr & PWPR_PFSWE
            };
            self.pwpr = (value & PWPR_B0WI) | pfswe;
            return Ok(());
        }
        self.store(offset, 1, u32::from(value));
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.store(offset, 2, u32::from(value));
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.store(offset & !3, 4, value);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "ra_pfs",
            "pwpr": self.pwpr,
            "pfs": self.pfs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pfs_writes_need_the_fsp_unlock_sequence() {
        let mut pfs = RaPfs::new();
        // P111PFS = port 1, pin 11: PCR (pull-up).
        let p111 = 0x40 + 4 * 11;
        pfs.write_u32(p111, 1 << 4).unwrap();
        assert_eq!(pfs.read_u32(p111).unwrap(), 0, "protected after reset");
        // Setting PFSWE while B0WI is still 1 does nothing.
        pfs.write(PWPR, PWPR_B0WI | PWPR_PFSWE).unwrap();
        assert_eq!(pfs.read(PWPR).unwrap(), PWPR_B0WI);
        // R_BSP_PinAccessEnable: PWPR = 0, then PWPR = PFSWE.
        pfs.write(PWPR, 0).unwrap();
        pfs.write(PWPR, PWPR_PFSWE).unwrap();
        pfs.write_u32(p111, 1 << 4).unwrap();
        assert_eq!(pfs.read_u32(p111).unwrap(), 1 << 4);
        // R_BSP_PinAccessDisable: PWPR = 0, then PWPR = B0WI.
        pfs.write(PWPR, 0).unwrap();
        pfs.write(PWPR, PWPR_B0WI).unwrap();
        pfs.write_u32(p111, 0).unwrap();
        assert_eq!(pfs.read_u32(p111).unwrap(), 1 << 4, "locked again");
    }

    #[test]
    fn the_ha_and_by_aliases_are_the_low_half_and_byte() {
        let mut pfs = RaPfs::new();
        pfs.write(PWPR, 0).unwrap();
        pfs.write(PWPR, PWPR_PFSWE).unwrap();
        pfs.write_u32(0x40, 0x0100_0000).unwrap();
        // PmnPFS_BY (+3) is bits 7:0 of PmnPFS.
        pfs.write(0x43, 0x10).unwrap();
        assert_eq!(pfs.read_u32(0x40).unwrap(), 0x0100_0010);
        assert_eq!(pfs.read(0x43).unwrap(), 0x10);
        // PmnPFS_HA (+2) is bits 15:0.
        pfs.write_u16(0x42, 0x0404).unwrap();
        assert_eq!(pfs.read_u32(0x40).unwrap(), 0x0100_0404);
        assert_eq!(pfs.read_u16(0x42).unwrap(), 0x0404);
    }
}
