// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! NXP i.MX RT CCM (Clock Controller Module) — behavioural stub.
//!
//! Models the RT1060 digital CCM enough for firmware clock bring-up:
//! sticky `CCGR0`…`CCGR7` ungating (GPIO2 = CCGR0[CG15], LPUART6 =
//! CCGR3[CG3]) and instant `CSR.COSC_READY` when `CCR.COSC_EN` is written
//! (same settle-now pattern as [`crate::peripherals::ra_clock::RaSysc`] /
//! [`crate::peripherals::mcg::Mcg`]).
//!
//! Offsets from IMXRT1060RM / MIMXRT1062.h: CCR@0x00, CSR@0x08,
//! CCGR0@0x68 … CCGR7@0x84.

use crate::{Peripheral, SimResult};
use std::any::Any;
use std::collections::HashMap;

const CCR: u64 = 0x00;
const CSR: u64 = 0x08;
const CCGR0: u64 = 0x68;
const CCGR1: u64 = 0x6C;
const CCGR2: u64 = 0x70;
const CCGR3: u64 = 0x74;
const CCGR4: u64 = 0x78;
const CCGR5: u64 = 0x7C;
const CCGR6: u64 = 0x80;
const CCGR7: u64 = 0x84;

/// CCR bit12 — on-chip oscillator enable (CCM_CCR_COSC_EN).
const COSC_EN: u32 = 1 << 12;
/// CSR bit5 — on-chip oscillator ready (CCM_CSR_COSC_READY).
const COSC_READY: u32 = 1 << 5;

/// CCGR0[CG15] bits 31:30 — gpio2_clk_enable (IMXRT1060RM).
#[allow(dead_code)]
const CCGR0_CG15_GPIO2: u32 = 0b11 << 30;
/// CCGR3[CG3] bits 7:6 — lpuart6_clk_enable (IMXRT1060RM).
#[allow(dead_code)]
const CCGR3_CG3_LPUART6: u32 = 0b11 << 6;

/// i.MX RT CCM — sticky CCGRs + instant COSC ready.
#[derive(Debug)]
pub struct ImxCcm {
    regs: HashMap<u64, u32>,
}

impl Default for ImxCcm {
    fn default() -> Self {
        Self::new()
    }
}

impl ImxCcm {
    pub fn new() -> Self {
        // Silicon reset leaves most CCGRs at 0xFFFFFFFF (clocks on). Seed
        // zero so tests observe ungating writes; ready starts clear.
        let mut regs = HashMap::new();
        regs.insert(CCR, 0);
        regs.insert(CSR, 0);
        for off in [CCGR0, CCGR1, CCGR2, CCGR3, CCGR4, CCGR5, CCGR6, CCGR7] {
            regs.insert(off, 0);
        }
        Self { regs }
    }

    fn word_offset(offset: u64) -> u64 {
        offset & !3
    }

    fn read_word(&self, offset: u64) -> u32 {
        let off = Self::word_offset(offset);
        if off == CSR {
            // COSC_READY tracks COSC_EN instantly (no OSCNT delay).
            let ccr = self.regs.get(&CCR).copied().unwrap_or(0);
            let mut csr = self.regs.get(&CSR).copied().unwrap_or(0);
            if ccr & COSC_EN != 0 {
                csr |= COSC_READY;
            } else {
                csr &= !COSC_READY;
            }
            return csr;
        }
        self.regs.get(&off).copied().unwrap_or(0)
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let off = Self::word_offset(offset);
        if off == CSR {
            return; // status is read-only
        }
        self.regs.insert(off, value);
    }
}

impl Peripheral for ImxCcm {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset);
        let shift = ((offset & 3) * 8) as u32;
        Ok(((word >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = Self::word_offset(offset);
        if off == CSR {
            return Ok(());
        }
        let shift = ((offset & 3) * 8) as u32;
        let mut word = self.regs.get(&off).copied().unwrap_or(0);
        word = (word & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_word(off, word);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "imx_ccm",
            "ccr": self.read_word(CCR),
            "csr": self.read_word(CSR),
            "ccgr0": self.read_word(CCGR0),
            "ccgr3": self.read_word(CCGR3),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn ccgr_ungate_gpio2_and_lpuart6_stick() {
        let mut ccm = ImxCcm::new();
        // GPIO2 = CCGR0[CG15] bits 31:30; LPUART6 = CCGR3[CG3] bits 7:6.
        ccm.write_u32(CCGR0, CCGR0_CG15_GPIO2).unwrap();
        ccm.write_u32(CCGR3, CCGR3_CG3_LPUART6).unwrap();
        assert_eq!(
            ccm.read_u32(CCGR0).unwrap() & CCGR0_CG15_GPIO2,
            CCGR0_CG15_GPIO2
        );
        assert_eq!(
            ccm.read_u32(CCGR3).unwrap() & CCGR3_CG3_LPUART6,
            CCGR3_CG3_LPUART6
        );
    }

    #[test]
    fn cosc_enable_sets_ready_instantly() {
        let mut ccm = ImxCcm::new();
        assert_eq!(ccm.read_u32(CSR).unwrap() & COSC_READY, 0);
        ccm.write_u32(CCR, COSC_EN).unwrap();
        assert_eq!(ccm.read_u32(CCR).unwrap() & COSC_EN, COSC_EN);
        assert_eq!(ccm.read_u32(CSR).unwrap() & COSC_READY, COSC_READY);
    }

    #[test]
    fn csr_is_read_only() {
        let mut ccm = ImxCcm::new();
        ccm.write_u32(CSR, 0xFFFF_FFFF).unwrap();
        assert_eq!(ccm.read_u32(CSR).unwrap() & COSC_READY, 0);
    }
}
