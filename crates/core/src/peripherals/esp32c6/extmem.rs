// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Minimal ESP32-C6 EXTMEM / L1 cache controller (`0x600C_8000`).
//!
//! Arduino-ESP32 / esp-hal cache init drives invalidate / sync / preload /
//! autoload through launch/done handshakes. The simulator has no cache
//! latency, so each op completes atomically on the launching write — clear
//! enable bits, assert done — matching the C3 [`super::super::esp32c3::cache`]
//! and S3 [`super::super::esp32s3::extmem`] contracts.
//!
//! Grown against the Smart Board Arduino twin's early `cache_hal` path
//! (Cache_Suspend polls `L1_CACHE_PRELOAD_CTRL` @+0xd8 bit1; sync uses
//! `CACHE_SYNC_CTRL` @+0x98 bit4).

use crate::{Peripheral, SimResult};
use std::collections::HashMap;

/// `(offset, launch_mask, done_bit)` — one-shot ops.
const LAUNCH_DONE_REGS: [(u64, u32, u32); 12] = [
    // CACHE_LOCK_CTRL: LOCK_ENA|UNLOCK_ENA → LOCK_DONE
    (0x88, 0b11, 1 << 2),
    // CACHE_SYNC_CTRL: INVALIDATE|CLEAN|WRITEBACK|WB_INVALIDATE → SYNC_DONE
    (0x98, 0b1111, 1 << 4),
    // L1_ICACHEn_PRELOAD_CTRL / L1_CACHE_PRELOAD_CTRL: ENA → DONE
    (0xa8, 1 << 0, 1 << 1),
    (0xb4, 1 << 0, 1 << 1),
    (0xc0, 1 << 0, 1 << 1),
    (0xcc, 1 << 0, 1 << 1),
    (0xd8, 1 << 0, 1 << 1),
    // L1_ICACHEn_AUTOLOAD_CTRL / L1_CACHE_AUTOLOAD_CTRL: ENA → DONE
    (0xe4, 1 << 0, 1 << 1),
    (0xf8, 1 << 0, 1 << 1),
    (0x10c, 1 << 0, 1 << 1),
    (0x120, 1 << 0, 1 << 1),
    (0x134, 1 << 0, 1 << 1),
];

/// L1_CACHE_FREEZE_CTRL @0x2c — level handshakes: EN mirrors to DONE.
/// SVD layout (4 bits/bank): EN @ 4n, MODE @ 4n+1, DONE @ 4n+2 for
/// n=0..4 (icache0..3 + L1_CACHE). ROM Cache_Suspend sets bit16 and polls bit18.
const FREEZE_CTRL: u64 = 0x2c;
const FREEZE_BANKS: [(u32, u32); 5] = [
    (1 << 0, 1 << 2),   // L1_ICACHE0
    (1 << 4, 1 << 6),   // L1_ICACHE1
    (1 << 8, 1 << 10),  // L1_ICACHE2
    (1 << 12, 1 << 14), // L1_ICACHE3
    (1 << 16, 1 << 18), // L1_CACHE
];

/// Idle seeds so polls exit before any launch write (SVD reset + DONE asserted).
const IDLE_STATUS_SEEDS: [(u64, u32); 12] = [
    (0x88, 1 << 2), // CACHE_LOCK_DONE
    (0x98, 1 << 4), // CACHE_SYNC_DONE (not SVD reset 1 = INVALIDATE_ENA)
    (0xa8, 1 << 1),
    (0xb4, 1 << 1),
    (0xc0, 1 << 1),
    (0xcc, 1 << 1),
    (0xd8, 1 << 1), // L1_CACHE_PRELOAD_DONE
    (0xe4, 1 << 1),
    (0xf8, 1 << 1),
    (0x10c, 1 << 1),
    (0x120, 1 << 1),
    (0x134, 1 << 1), // L1_CACHE_AUTOLOAD_DONE
];

#[derive(Debug)]
pub struct Esp32c6Extmem {
    words: HashMap<u64, u32>,
}

impl Default for Esp32c6Extmem {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32c6Extmem {
    pub fn new() -> Self {
        let mut words = HashMap::new();
        for (off, val) in IDLE_STATUS_SEEDS {
            words.insert(off, val);
        }
        Self { words }
    }

    fn reg(&self, off: u64) -> u32 {
        self.words.get(&off).copied().unwrap_or(0)
    }

    fn on_word_write(&mut self, word_off: u64) {
        for (off, launch_mask, done_bit) in LAUNCH_DONE_REGS {
            if word_off == off {
                let v = self.reg(off);
                if v & launch_mask != 0 {
                    self.words.insert(off, (v & !launch_mask) | done_bit);
                }
            }
        }
        if word_off == FREEZE_CTRL {
            let mut v = self.reg(FREEZE_CTRL);
            for (en, done) in FREEZE_BANKS {
                if v & en != 0 {
                    v |= done;
                } else {
                    v &= !done;
                }
            }
            self.words.insert(FREEZE_CTRL, v);
        }
    }
}

impl Peripheral for Esp32c6Extmem {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.reg(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.words.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
        self.on_word_write(word_off);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let word_off = offset & !3;
        self.words.insert(word_off, value);
        self.on_word_write(word_off);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preload_done_idle() {
        let e = Esp32c6Extmem::new();
        assert_eq!(e.read_u32(0xd8).unwrap() & (1 << 1), 1 << 1);
    }

    #[test]
    fn sync_completes_atomically() {
        let mut e = Esp32c6Extmem::new();
        e.write_u32(0x98, 0b0001).unwrap(); // INVALIDATE_ENA
        let v = e.read_u32(0x98).unwrap();
        assert_eq!(v & 0b1111, 0, "ENA bits cleared");
        assert_eq!(v & (1 << 4), 1 << 4, "SYNC_DONE set");
    }

    #[test]
    fn freeze_mirrors_en() {
        let mut e = Esp32c6Extmem::new();
        // L1_CACHE bank (ROM Cache_Suspend): EN bit16 → DONE bit18
        e.write_u32(0x2c, 1 << 16).unwrap();
        assert_eq!(e.read_u32(0x2c).unwrap() & (1 << 18), 1 << 18);
        e.write_u32(0x2c, 0).unwrap();
        assert_eq!(e.read_u32(0x2c).unwrap() & (1 << 18), 0);
        // ICACHE3 still maps EN12 → DONE14
        e.write_u32(0x2c, 1 << 12).unwrap();
        assert_eq!(e.read_u32(0x2c).unwrap() & (1 << 14), 1 << 14);
    }
}
