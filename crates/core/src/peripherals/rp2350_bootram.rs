// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2350 boot RAM endpoint (datasheet 4.3, base 0x400E0000).
//!
//! - 0x000..0x400: 1 KB boot RAM (256 x 32-bit), plain read/write storage.
//! - 0x800 / 0x804 WRITE_ONCE0/1: writes OR into the current value; a set bit
//!   clears only on reset (Table 436).
//! - 0x808 BOOTLOCK_STAT: bit n = 1 while boot lock n is unclaimed, reset 0xFF
//!   (Table 437).
//! - 0x80C..0x828 BOOTLOCK0..7: "Read to claim and check. Write to unclaim.
//!   The value returned on successful claim is 1 << n, and on failed claim is
//!   zero" (Table 438) — the SIO spinlock protocol.
//!
//! pico-sdk's pre-main runtime touches it twice: `runtime_init_boot_locks_reset`
//! writes every BOOTLOCKn (release), and `runtime_init_bootrom_locking_enable`
//! spins reading BOOTLOCK7 until the claim succeeds (5.4.4: lock 7 claimed =
//! bootrom lock checking enabled).
//!
//! FIDELITY: modeled, NOT HW-validated (2026-11-01). Single-core: a free lock
//! is granted on read. Not modelled: the Secure-only access restriction and
//! the APB wait states.

use crate::{Peripheral, SimResult};
use std::cell::Cell;

const RAM_WORDS: usize = 256;
const WRITE_ONCE0: u64 = 0x800;
const WRITE_ONCE1: u64 = 0x804;
const BOOTLOCK_STAT: u64 = 0x808;
const BOOTLOCK0: u64 = 0x80C;
const BOOTLOCK7: u64 = 0x828;

#[derive(Debug)]
pub struct Rp2350BootRam {
    ram: [u32; RAM_WORDS],
    write_once: [u32; 2],
    /// Bit n set = boot lock n unclaimed (BOOTLOCK_STAT polarity). `Cell`
    /// because a BOOTLOCKn read is a claim on the `&self` read path.
    unclaimed: Cell<u8>,
}

impl Default for Rp2350BootRam {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2350BootRam {
    pub fn new() -> Self {
        Self {
            ram: [0; RAM_WORDS],
            write_once: [0; 2],
            unclaimed: Cell::new(0xFF),
        }
    }

    fn lock_bit(offset: u64) -> Option<u8> {
        if (BOOTLOCK0..=BOOTLOCK7).contains(&offset) && offset & 3 == 0 {
            Some(1u8 << ((offset - BOOTLOCK0) / 4))
        } else {
            None
        }
    }
}

impl Peripheral for Rp2350BootRam {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if let Some(bit) = Self::lock_bit(offset) {
            let free = self.unclaimed.get();
            return Ok(if free & bit != 0 {
                self.unclaimed.set(free & !bit);
                u32::from(bit)
            } else {
                0
            });
        }
        Ok(match offset {
            o if o < (RAM_WORDS as u64) * 4 => self.ram[(o / 4) as usize],
            WRITE_ONCE0 => self.write_once[0],
            WRITE_ONCE1 => self.write_once[1],
            BOOTLOCK_STAT => u32::from(self.unclaimed.get()),
            _ => {
                crate::census_reg!("rp2350.bootram:Rp2350BootRam", offset, "read");
                0
            }
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if let Some(bit) = Self::lock_bit(offset) {
            self.unclaimed.set(self.unclaimed.get() | bit);
            return Ok(());
        }
        match offset {
            o if o < (RAM_WORDS as u64) * 4 => self.ram[(o / 4) as usize] = value,
            WRITE_ONCE0 => self.write_once[0] |= value,
            WRITE_ONCE1 => self.write_once[1] |= value,
            BOOTLOCK_STAT => self.unclaimed.set(value as u8),
            _ => {
                crate::census_reg!("rp2350.bootram:Rp2350BootRam", offset, "write");
            }
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        // Sub-word reads of the RAM do not claim locks: only a word access
        // at a BOOTLOCKn address is a claim.
        if Self::lock_bit(offset & !3).is_some() {
            return Ok(0);
        }
        let word = self.read_u32(offset & !3)?;
        Ok((word >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        if Self::lock_bit(aligned).is_some() {
            return self.write_u32(aligned, u32::from(value));
        }
        let shift = (offset & 3) * 8;
        let cur = match aligned {
            WRITE_ONCE0 | WRITE_ONCE1 => 0, // OR-in only the written byte
            _ => self.read_u32(aligned)?,
        };
        let new = (cur & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_u32(aligned, new)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        // Side-effect-free view: a debugger peek must not claim a lock.
        let aligned = offset & !3;
        let word = match Self::lock_bit(aligned) {
            Some(bit) => u32::from(self.unclaimed.get() & bit == 0),
            None => self.read_u32(aligned).ok()?,
        };
        Some((word >> ((offset & 3) * 8)) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootlock_read_claims_once_and_write_releases() {
        let mut b = Rp2350BootRam::new();
        assert_eq!(b.read_u32(BOOTLOCK_STAT).unwrap(), 0xFF);
        // BOOTLOCK7 (runtime_init_bootrom_locking_enable spins on this).
        assert_eq!(b.read_u32(BOOTLOCK7).unwrap(), 1 << 7);
        assert_eq!(b.read_u32(BOOTLOCK7).unwrap(), 0, "already claimed");
        assert_eq!(b.read_u32(BOOTLOCK_STAT).unwrap(), 0x7F);
        b.write_u32(BOOTLOCK7, 0).unwrap();
        assert_eq!(b.read_u32(BOOTLOCK_STAT).unwrap(), 0xFF);
        assert_eq!(b.read_u32(BOOTLOCK0).unwrap(), 1);
    }

    #[test]
    fn write_once_ors_and_ram_stores() {
        let mut b = Rp2350BootRam::new();
        b.write_u32(WRITE_ONCE0, 0x5).unwrap();
        b.write_u32(WRITE_ONCE0, 0x2).unwrap();
        assert_eq!(b.read_u32(WRITE_ONCE0).unwrap(), 0x7);
        b.write_u32(0x3FC, 0xDEAD_BEEF).unwrap();
        assert_eq!(b.read_u32(0x3FC).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn peek_does_not_claim() {
        let b = Rp2350BootRam::new();
        let _ = b.peek(BOOTLOCK7);
        assert_eq!(b.read_u32(BOOTLOCK_STAT).unwrap(), 0xFF);
    }
}
