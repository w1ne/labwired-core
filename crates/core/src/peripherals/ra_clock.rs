// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Renesas RA SYSTEM clock (HOCO / OSCSF) — behavioural model.
//!
//! RA4M1 `R_SYSTEM` (R01UH0887 / FSP `R7FA4M1AB.h`): HOCOCR @ 0x36 (HCSTP bit0;
//! 0 = operating, 1 = stopped), OSCSF @ 0x3C (HOCOSF bit0). Firmware enables
//! HOCO then spins on OSCSF.HOCOSF; this model settles instantly like
//! [`crate::peripherals::mcg::Mcg`].

use crate::{Peripheral, SimResult};
use std::any::Any;

const HOCOCR: u64 = 0x36;
const OSCSF: u64 = 0x3C;

const HCSTP: u8 = 1 << 0;
const HOCOSF: u8 = 1 << 0;

/// Renesas RA SYSTEM block — HOCO enable + OSCSF ready flag.
#[derive(Debug)]
pub struct RaSysc {
    /// HOCOCR: bit0 HCSTP (0 = HOCO operating).
    hococr: u8,
    /// OSCSF: bit0 HOCOSF (1 = HOCO stable). Read-only to firmware.
    oscsf: u8,
}

impl Default for RaSysc {
    fn default() -> Self {
        Self::new()
    }
}

impl RaSysc {
    pub fn new() -> Self {
        // SVD/CMSIS: HOCOCR reset 0 (operating). HOCOSF is typically already
        // set when OFS1 leaves HOCO enabled out of reset.
        let mut s = Self {
            hococr: 0,
            oscsf: HOCOSF,
        };
        s.recompute_oscsf();
        s
    }

    fn recompute_oscsf(&mut self) {
        if self.hococr & HCSTP == 0 {
            self.oscsf |= HOCOSF;
        } else {
            self.oscsf &= !HOCOSF;
        }
    }
}

impl Peripheral for RaSysc {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(match offset {
            HOCOCR => self.hococr,
            OSCSF => self.oscsf,
            _ => 0,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            HOCOCR => {
                self.hococr = value;
                self.recompute_oscsf();
            }
            OSCSF => {} // read-only status
            _ => {}
        }
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
            "peripheral": "ra_sysc",
            "hococr": self.hococr,
            "oscsf": self.oscsf,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hoco_enable_sets_oscsf_hocosf() {
        let mut sys = RaSysc::new();
        // Stop HOCO — HOCOSF must clear.
        sys.write(HOCOCR, HCSTP).unwrap();
        assert_eq!(sys.read(HOCOCR).unwrap() & HCSTP, HCSTP);
        assert_eq!(sys.read(OSCSF).unwrap() & HOCOSF, 0);
        // Enable (HCSTP=0) — HOCOSF settles immediately (no spin).
        sys.write(HOCOCR, 0).unwrap();
        assert_eq!(sys.read(HOCOCR).unwrap() & HCSTP, 0);
        assert_eq!(sys.read(OSCSF).unwrap() & HOCOSF, HOCOSF);
    }

    #[test]
    fn oscsf_is_read_only() {
        let mut sys = RaSysc::new();
        sys.write(HOCOCR, HCSTP).unwrap();
        sys.write(OSCSF, 0xFF).unwrap();
        assert_eq!(sys.read(OSCSF).unwrap() & HOCOSF, 0);
    }
}
