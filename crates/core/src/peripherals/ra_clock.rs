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
//!
//! RA8M1 (`ra8_sysc`, FSP `R7FA8M1AH.h`): HOCOCR/OSCSF at the same offsets,
//! plus the SCI clock: SCICKDIVCR @ 0x54 (CKDIV[2:0]) and SCICKCR @ 0x55
//! (SCICKSEL[3:0], CKSREQ bit6, CKSRDY bit7). FSP's
//! `bsp_peripheral_clock_set` sets CKSREQ, spins until CKSRDY reads 1,
//! writes the source and divider, then clears CKSREQ and spins until CKSRDY
//! reads 0. CKSRDY follows CKSREQ at once here. The selected source is
//! stored only: no peripheral rate is derived from it.

use crate::{Peripheral, SimResult};
use std::any::Any;

const HOCOCR: u64 = 0x36;
const OSCSF: u64 = 0x3C;

const SCICKDIVCR: u64 = 0x54;
const SCICKCR: u64 = 0x55;
const CKSREQ: u8 = 1 << 6;
const CKSRDY: u8 = 1 << 7;

const HCSTP: u8 = 1 << 0;
const HOCOSF: u8 = 1 << 0;

/// Renesas RA SYSTEM block — HOCO enable + OSCSF ready flag.
#[derive(Debug)]
pub struct RaSysc {
    /// HOCOCR: bit0 HCSTP (0 = HOCO operating).
    hococr: u8,
    /// OSCSF: bit0 HOCOSF (1 = HOCO stable). Read-only to firmware.
    oscsf: u8,
    /// RA8 SCICKDIVCR / SCICKCR (without CKSRDY), or `None` on parts whose
    /// SYSTEM window has no SCI clock (RA4M1, RA6M5).
    sci_clock: Option<(u8, u8)>,
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
            sci_clock: None,
        };
        s.recompute_oscsf();
        s
    }

    /// The RA8 SYSTEM window: adds the SCICKDIVCR/SCICKCR handshake. Both
    /// reset to 0 here; the HW-manual reset values are not cross-checked.
    pub fn new_ra8() -> Self {
        Self {
            sci_clock: Some((0, 0)),
            ..Self::new()
        }
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
            SCICKDIVCR => self.sci_clock.map_or(0, |(div, _)| div),
            SCICKCR => self
                .sci_clock
                .map_or(0, |(_, cr)| if cr & CKSREQ != 0 { cr | CKSRDY } else { cr }),
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
            SCICKDIVCR => {
                if let Some((div, _)) = &mut self.sci_clock {
                    *div = value & 0x07;
                }
            }
            SCICKCR => {
                if let Some((_, cr)) = &mut self.sci_clock {
                    *cr = value & (CKSREQ | 0x0F);
                }
            }
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
            "sci_clock": self.sci_clock,
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
    fn ra8_sci_clock_switch_completes_the_fsp_handshake() {
        let mut sys = RaSysc::new_ra8();
        // bsp_peripheral_clock_set: request the stop, wait for CKSRDY.
        sys.write(SCICKCR, CKSREQ).unwrap();
        assert_eq!(sys.read(SCICKCR).unwrap() & CKSRDY, CKSRDY);
        // Divider /1, source MOCO (1), REQ|RDY written back.
        sys.write(SCICKDIVCR, 0).unwrap();
        sys.write(SCICKCR, 1 | CKSREQ | CKSRDY).unwrap();
        // Release the request; CKSRDY drops and the source reads back.
        sys.write(SCICKCR, 1).unwrap();
        assert_eq!(sys.read(SCICKCR).unwrap(), 1);
        assert_eq!(sys.read(SCICKDIVCR).unwrap(), 0);
        sys.write(SCICKDIVCR, 0xFF).unwrap();
        assert_eq!(sys.read(SCICKDIVCR).unwrap(), 0x07, "CKDIV is 3 bits");
    }

    #[test]
    fn ra4_and_ra6_sysc_have_no_sci_clock() {
        let mut sys = RaSysc::new();
        sys.write(SCICKCR, CKSREQ).unwrap();
        assert_eq!(sys.read(SCICKCR).unwrap(), 0);
    }

    #[test]
    fn oscsf_is_read_only() {
        let mut sys = RaSysc::new();
        sys.write(HOCOCR, HCSTP).unwrap();
        sys.write(OSCSF, 0xFF).unwrap();
        assert_eq!(sys.read(OSCSF).unwrap() & HOCOSF, 0);
    }
}
