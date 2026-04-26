// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Interrupt Matrix peripheral for ESP32-S3.
//!
//! Per ESP32-S3 TRM §9.4. The interrupt matrix routes 99 peripheral
//! source IDs to 32 cpu0 IRQ slots (and a parallel 32 cpu1 IRQ slots,
//! not modeled in Plan 3).
//!
//! Each peripheral source ID has a 32-bit map register at:
//!   PRO_<source>_INTR_MAP_REG = 0x000 + 4 * source_id     (cpu0)
//!   APP_<source>_INTR_MAP_REG = 0x400 + 4 * source_id     (cpu1, accepted but not modeled)
//!
//! The register stores the cpu IRQ slot (0..31) the source delivers to.
//! Slot 0 is reserved for software interrupts; we treat any value 0..31
//! as a valid binding and only return None when the peripheral has never
//! been written.

use crate::{Peripheral, SimResult};

const NUM_SOURCES: usize = 99;

#[derive(Debug)]
pub struct Esp32s3IntMatrix {
    /// For each peripheral source ID (0..99), which cpu0 IRQ slot it's
    /// bound to. None = never written (no binding).
    cpu0_route: [Option<u8>; NUM_SOURCES],
}

impl Esp32s3IntMatrix {
    pub fn new() -> Self {
        Self {
            cpu0_route: [None; NUM_SOURCES],
        }
    }

    /// Look up the cpu0 IRQ slot for `source_id`. Returns None if unbound
    /// or if `source_id` is out of range.
    pub fn route(&self, source_id: u32) -> Option<u8> {
        let idx = source_id as usize;
        if idx >= NUM_SOURCES {
            return None;
        }
        self.cpu0_route[idx]
    }
}

impl Default for Esp32s3IntMatrix {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32s3IntMatrix {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Both PRO and APP map ranges accepted; PRO at 0x000-0x18C, APP at 0x400+.
        // PRO read returns the bound IRQ slot; APP read returns 0 (we don't model cpu1).
        if offset < 0x400 {
            let word_off = offset & !3;
            let byte_off = (offset & 3) * 8;
            let src = (word_off / 4) as usize;
            let slot = if src < NUM_SOURCES {
                self.cpu0_route[src].unwrap_or(0) as u32
            } else {
                0
            };
            Ok(((slot >> byte_off) & 0xFF) as u8)
        } else {
            Ok(0)
        }
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // Only respond to PRO map writes; APP writes silently accepted.
        if offset >= 0x400 {
            return Ok(());
        }
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let src = (word_off / 4) as usize;
        if src >= NUM_SOURCES {
            return Ok(());
        }
        // Read current word, R-M-W the byte.
        let current = self.cpu0_route[src].unwrap_or(0) as u32;
        let mut word = current;
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        // Slot is bits[4:0]; only bind if a non-default value was written
        // OR if the byte that contained the slot bits was touched.
        // For simplicity: any write to the source's word records the binding.
        let slot = (word & 0x1F) as u8;
        self.cpu0_route[src] = Some(slot);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_unbound() {
        let m = Esp32s3IntMatrix::new();
        for src in 0u32..99 {
            assert!(m.route(src).is_none(), "source {src} should be unbound");
        }
    }

    #[test]
    fn out_of_range_source_returns_none() {
        let m = Esp32s3IntMatrix::new();
        assert!(m.route(99).is_none());
        assert!(m.route(1000).is_none());
    }

    #[test]
    fn bind_source_via_mmio_write() {
        let mut m = Esp32s3IntMatrix::new();
        // Bind SYSTIMER_TARGET0 (source 79) to cpu0 IRQ slot 15.
        // Offset = 79 * 4 = 316 = 0x13C. Write byte 0 = 15.
        let off = 79 * 4u64;
        m.write(off, 15).unwrap();
        m.write(off + 1, 0).unwrap();
        m.write(off + 2, 0).unwrap();
        m.write(off + 3, 0).unwrap();
        assert_eq!(m.route(79), Some(15));
    }

    #[test]
    fn bind_max_source() {
        let mut m = Esp32s3IntMatrix::new();
        // Source 98 (last valid).
        let off = 98 * 4u64;
        m.write(off, 7).unwrap();
        assert_eq!(m.route(98), Some(7));
    }

    #[test]
    fn slot_only_lower_5_bits_kept() {
        let mut m = Esp32s3IntMatrix::new();
        // Write 0x3F (would be IRQ 63 if all bits used; ESP32-S3 has 32 slots).
        m.write(0, 0x3F).unwrap();
        // Spec says bits[4:0] only.
        assert_eq!(m.route(0), Some(0x1F));
    }

    #[test]
    fn read_back_bound_source() {
        let mut m = Esp32s3IntMatrix::new();
        let off = 5 * 4u64;
        m.write(off, 12).unwrap();
        assert_eq!(m.read(off).unwrap(), 12);
        assert_eq!(m.read(off + 1).unwrap(), 0);
    }

    #[test]
    fn app_map_writes_silently_accepted() {
        let mut m = Esp32s3IntMatrix::new();
        m.write(0x400, 0xAB).unwrap();
        assert_eq!(m.read(0x400).unwrap(), 0);
        // PRO route for source 0 unaffected.
        assert!(m.route(0).is_none());
    }

    #[test]
    fn out_of_range_pro_offset_silently_dropped() {
        let mut m = Esp32s3IntMatrix::new();
        // Source index 99 would be at 0x18C. Anything in 0x18C..0x400 is unmapped.
        m.write(99 * 4, 0xAB).unwrap();
        assert!(m.route(99).is_none());
    }
}
