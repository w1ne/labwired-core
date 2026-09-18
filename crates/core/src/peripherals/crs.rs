// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! CRS (Clock Recovery System) — STM32, first modelled for the STM32U575.
//!
//! Four registers (RM0456 §13): CR, CFGR, ISR, ICR. The block calibrates
//! HSI48 against a SYNC signal (USB SOF / LSE / GPIO). The U5 Arduino core's
//! `SystemClock_Config` ends in `HAL_RCCEx_CRSConfig`, which force-resets the
//! block through RCC, writes CFGR/TRIM, and enables AUTOTRIMEN|CEN — it never
//! polls a completion, so the model just needs the register surface to keep
//! the writes off the bus-fault path while staying honest about what it does
//! and does not simulate:
//!
//! * CR/CFGR/ISR/ICR keep the vendored-SVD reset values and writable fields.
//! * `SWSYNC` is a write-only trigger: it reads 0 and latches `SYNCOKF`, the
//!   way silicon reports the sync event the firmware asked for. No external
//!   SYNC source (USB SOF / LSE) is wired, so automatic synchronization never
//!   completes on its own — `SYNCOKF` stays clear until SWSYNC.
//! * HSI48 trimming itself is not simulated (no trim loop / FECAP movement).
//! * The APB1RSTR force-reset pulse `HAL_RCCEx_CRSConfig` issues first is not
//!   modelled: CRS state survives it instead of returning to reset values.

use crate::SimResult;

/// CR writable bits: SYNCOKIE(0), SYNCWARNIE(1), ERRIE(2), ESYNCIE(3),
/// CEN(5), AUTOTRIMEN(6), SWSYNC(7), TRIM(14:8); bit 4 is reserved (SVD).
const CR_WRITABLE_MASK: u32 = 0x0000_7FEF;
/// CFGR writable bits: RELOAD(15:0), FELIM(23:16), SYNCDIV(26:24),
/// SYNCSRC(29:28), SYNCPOL(31); bits 27/30 are reserved.
const CFGR_WRITABLE_MASK: u32 = 0xB7FF_FFFF;
/// ISR latched flags: SYNCOKF(0), SYNCWARNF(1), ERRF(2), ESYNCF(3).
const ISR_FLAG_MASK: u32 = 0x0000_000F;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Crs {
    cr: u32,
    cfgr: u32,
    isr: u32,
}

impl Crs {
    pub fn new() -> Self {
        Self {
            cr: 0x0000_4000,
            cfgr: 0x2022_BB7F,
            isr: 0,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.cfgr,
            0x08 => self.isr,
            0x0C => 0,
            _ => {
                crate::census_reg!("crs:Crs", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // CR: status-free control register. SWSYNC is a trigger, not
            // state — it is dropped from the stored value and latches the
            // SYNCOKF flag like the software sync event it generates.
            0x00 => {
                self.cr = value & CR_WRITABLE_MASK & !(1 << 7);
                if value & (1 << 7) != 0 {
                    self.isr |= 1 << 0;
                }
            }
            0x04 => self.cfgr = value & CFGR_WRITABLE_MASK,
            0x08 => {} // ISR is read-only
            0x0C => self.isr &= !(value & ISR_FLAG_MASK),
            _ => {
                crate::census_reg!("crs:Crs", offset, "write");
            }
        }
    }
}

impl Default for Crs {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Crs {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

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

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::Crs;
    use crate::Peripheral;

    /// Reset values straight from the vendored SVD
    /// (`tests/fixtures/real_world/stm32u575.svd`, CRS CR/CFGR/ISR).
    #[test]
    fn crs_reset_values_match_svd() {
        let crs = Crs::new();
        assert_eq!(crs.read_u32(0x00).unwrap(), 0x0000_4000, "CR reset (TRIM)");
        assert_eq!(crs.read_u32(0x04).unwrap(), 0x2022_BB7F, "CFGR reset");
        assert_eq!(crs.read_u32(0x08).unwrap(), 0, "ISR reset");
    }

    /// The Cube HAL's `HAL_RCCEx_CRSConfig` sequence end-to-end: CFGR write,
    /// TRIM update, AUTOTRIMEN|CEN enable — plus the SWSYNC trigger and the
    /// ICR write-1-to-clear path.
    #[test]
    fn crs_config_write_then_software_sync_latches_flag() {
        let mut crs = Crs::new();

        // CFGR: complete register write, masked to the defined fields.
        crs.write_u32(0x04, 0x2022_BB7F).unwrap();
        assert_eq!(crs.read_u32(0x04).unwrap(), 0x2022_BB7F, "CFGR round-trip");
        // SYNCPOL(31) / SYNCDIV(26) stick; reserved bits 27 and 30 do not.
        crs.write_u32(0x04, 0xCC00_0000).unwrap();
        assert_eq!(
            crs.read_u32(0x04).unwrap(),
            0x8400_0000,
            "SYNCDIV/SYNCPOL stick, reserved bits 27/30 do not"
        );

        // CR: TRIM[14:8] update, then AUTOTRIMEN|CEN.
        crs.write_u32(0x00, 0x0000_4060).unwrap();
        assert_eq!(
            crs.read_u32(0x00).unwrap(),
            0x0000_4060,
            "TRIM + AUTOTRIMEN + CEN"
        );
        // Reserved CR bit 4 does not stick.
        crs.write_u32(0x00, 0x0000_4070).unwrap();
        assert_eq!(
            crs.read_u32(0x00).unwrap(),
            0x0000_4060,
            "reserved CR bit4 dropped"
        );

        // SWSYNC (bit7) is a write-only trigger: reads back 0, latches SYNCOKF.
        crs.write_u32(0x00, 0x0000_40E0).unwrap();
        assert_eq!(
            crs.read_u32(0x00).unwrap() & (1 << 7),
            0,
            "SWSYNC self-clears"
        );
        assert_ne!(crs.read_u32(0x08).unwrap() & 1, 0, "SYNCOKF latched");

        // ISR is read-only; only ICR write-1-to-clear drops the flag.
        crs.write_u32(0x08, 0xFFFF_FFFF).unwrap();
        assert_eq!(crs.read_u32(0x08).unwrap() & 1, 1, "ISR write ignored");
        crs.write_u32(0x0C, 1).unwrap();
        assert_eq!(crs.read_u32(0x08).unwrap() & 1, 0, "SYNCOKF cleared");
    }
}
