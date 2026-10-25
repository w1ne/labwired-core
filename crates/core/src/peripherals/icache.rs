// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32U5 instruction cache controller (RM0456 §8).
//!
//! The cache contents are NOT simulated — the engine's memory is flat, so an
//! invalidation has no data to drop. What matters for firmware is the register
//! contract: `HAL_ICACHE_Enable/Disable/Invalidate` programs `CR`, polls
//! `SR.BUSYF`, and expects `CACHEINV` to self-clear. A read-as-zero stub
//! leaves the completion flag off and modes that wait for it hang.
//!
//! Modelled semantics (RM0456 §8.7, pinned by the 60North U5 peripheral test):
//! - reset `SR = BSYENDF` (the reset invalidation has completed; a documented
//!   deviation from the manual's BUSYF, matching the reference model);
//! - `CR.CACHEINV` is a self-clearing command that completes immediately and
//!   sets `SR.BSYENDF`;
//! - `CR.WAYSEL` (associativity) is writable only while `CR.EN = 0`;
//! - `FCR` is write-1-to-clear for `SR`; `BSYENDIE` gates the interrupt line.

use crate::SimResult;

const CR: u64 = 0x00;
const SR: u64 = 0x04;
const IER: u64 = 0x08;
const FCR: u64 = 0x0C;

const CR_EN: u32 = 1 << 0;
const CR_CACHEINV: u32 = 1 << 1;
const CR_WAYSEL: u32 = 1 << 2;
const SR_BUSYF: u32 = 1 << 0;
const SR_BSYENDF: u32 = 1 << 1;
const IER_BUSYFIE: u32 = 1 << 0;
const IER_BSYENDIE: u32 = 1 << 1;
const FCR_CBUSYF: u32 = 1 << 0;
const FCR_CBSYENDF: u32 = 1 << 1;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Icache {
    cr: u32,
    sr: u32,
    ier: u32,
}

impl Icache {
    pub fn new() -> Self {
        Self {
            cr: 0,
            // Reset invalidation already finished (60North/reference deviation).
            sr: SR_BSYENDF,
            ier: 0,
        }
    }

    fn irq_level(&self) -> bool {
        (self.sr & self.ier & (SR_BUSYF | SR_BSYENDF)) != 0
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            CR => self.cr,
            SR => self.sr,
            IER => self.ier,
            // FCR is write-only.
            FCR => 0,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            CR => {
                let was_enabled = self.cr & CR_EN != 0;
                let mut next = value;
                if was_enabled {
                    // Associativity is locked while the cache is enabled.
                    next = (next & !CR_WAYSEL) | (self.cr & CR_WAYSEL);
                }
                // CACHEINV is a self-clearing maintenance command that
                // completes immediately in the twin.
                if next & CR_CACHEINV != 0 {
                    next &= !CR_CACHEINV;
                    self.sr |= SR_BSYENDF;
                }
                self.cr = next & (CR_EN | CR_WAYSEL);
            }
            SR => {} // read-only status
            IER => self.ier = value & (IER_BUSYFIE | IER_BSYENDIE),
            FCR => {
                if value & FCR_CBUSYF != 0 {
                    self.sr &= !SR_BUSYF;
                }
                if value & FCR_CBSYENDF != 0 {
                    self.sr &= !SR_BSYENDF;
                }
            }
            _ => {}
        }
    }
}

impl crate::Peripheral for Icache {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_level())
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn reset_invalidation_has_completed() {
        let i = Icache::new();
        assert_eq!(i.read_u32(SR).unwrap(), SR_BSYENDF);
        assert_eq!(i.read_u32(CR).unwrap(), 0);
        assert_eq!(i.irq_line_level(), Some(false));
    }

    #[test]
    fn cacheinv_self_clears_and_completes() {
        let mut i = Icache::new();
        i.write_u32(FCR, FCR_CBSYENDF).unwrap();
        assert_eq!(i.read_u32(SR).unwrap(), 0);
        i.write_u32(CR, CR_WAYSEL | CR_CACHEINV).unwrap();
        assert_eq!(i.read_u32(CR).unwrap(), CR_WAYSEL);
        assert_eq!(i.read_u32(SR).unwrap(), SR_BSYENDF);
    }

    #[test]
    fn waysel_is_locked_while_enabled() {
        let mut i = Icache::new();
        i.write_u32(CR, CR_EN).unwrap();
        i.write_u32(CR, CR_EN | CR_WAYSEL).unwrap();
        assert_eq!(i.read_u32(CR).unwrap(), CR_EN);
        i.write_u32(CR, 0).unwrap();
        i.write_u32(CR, CR_WAYSEL).unwrap();
        assert_eq!(i.read_u32(CR).unwrap(), CR_WAYSEL);
    }

    #[test]
    fn busy_end_interrupt_follows_enable_and_clear() {
        let mut i = Icache::new();
        i.write_u32(IER, IER_BSYENDIE).unwrap();
        assert_eq!(i.irq_line_level(), Some(true));
        i.write_u32(FCR, FCR_CBSYENDF).unwrap();
        assert_eq!(i.irq_line_level(), Some(false));
        i.write_u32(CR, CR_WAYSEL | CR_CACHEINV).unwrap();
        assert_eq!(i.irq_line_level(), Some(true));
    }
}
