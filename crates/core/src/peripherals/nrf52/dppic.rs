// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF5340 DPPIC channel-enable subset (PS, DPPIC CHEN/CHENSET/CHENCLR).
//!
//! UARTE's local ENDTX -> STOPTX route consults this state. General DPPI
//! event delivery, cross-instance routes, channel groups and SPU security
//! attributes are not modelled.

use crate::{Peripheral, SimResult};

#[derive(Debug, Default)]
pub struct NrfDppic {
    chen: u32,
}

impl NrfDppic {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for NrfDppic {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_u32(offset & !3)? >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let bits = u32::from(value) << shift;
        let word = if aligned == 0x500 {
            (self.chen & !(0xff << shift)) | bits
        } else {
            bits
        };
        self.write_u32(aligned, word)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            0x500 | 0x504 | 0x508 => self.chen,
            _ => {
                crate::census_reg!("nrf5340.dppic:NrfDppic", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            0x500 => self.chen = value,
            0x504 => self.chen |= value,
            0x508 => self.chen &= !value,
            _ => {
                crate::census_reg!("nrf5340.dppic:NrfDppic", offset, "write");
            }
        }
        Ok(())
    }
}
