// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 ACL (Access Control List).
//!
//! Source: nRF52840 PS rev 1.7 §6.0 (ACL). Region-level flash access
//! protection. 8 ACL regions, each with ADDR / SIZE / PERM. Register-
//! surface only.

use crate::{Peripheral, SimResult};

const N_REGIONS: usize = 8;

#[derive(Debug, Default, Clone, Copy)]
struct AclRegion {
    addr: u32,
    size: u32,
    perm: u32,
}

#[derive(Debug, Default)]
pub struct Nrf52Acl {
    regions: [AclRegion; N_REGIONS],
}

impl Nrf52Acl {
    pub fn new() -> Self {
        Self::default()
    }

    fn region_field(offset: u64) -> Option<(usize, u8)> {
        // Each region starts at 0x500 + 0x10*i with three 32-bit slots
        // (ADDR, SIZE, PERM) at +0x00 / +0x04 / +0x08.
        if !(0x500..0x580).contains(&offset) || !offset.is_multiple_of(4) {
            return None;
        }
        let rel = offset - 0x500;
        let region = (rel / 0x10) as usize;
        let field = ((rel % 0x10) / 4) as u8;
        if region < N_REGIONS && field < 3 {
            Some((region, field))
        } else {
            None
        }
    }
}

impl Peripheral for Nrf52Acl {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if let Some((r, f)) = Self::region_field(offset) {
            Ok(match f {
                0 => self.regions[r].addr,
                1 => self.regions[r].size,
                2 => self.regions[r].perm & 0x6,
                _ => 0,
            })
        } else {
            Ok(0)
        }
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if let Some((r, f)) = Self::region_field(offset) {
            match f {
                0 => self.regions[r].addr = value & !0xFFF, // 4 KiB-aligned
                1 => self.regions[r].size = value & !0xFFF,
                2 => {
                    // PERM bits 1 (WRITE protect) and 2 (READ protect) are
                    // write-1-to-set per PS §6.0.5.
                    self.regions[r].perm |= value & 0x6;
                }
                _ => {}
            }
        }
        Ok(())
    }
}
