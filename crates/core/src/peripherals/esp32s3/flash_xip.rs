// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Flash-XIP backing peripheral for ESP32-S3.
//!
//! The S3 exposes the in-package SPI flash to the CPU through two MMIO
//! windows: 0x4200_0000 (I-cache, instruction fetch) and 0x3C00_0000
//! (D-cache, data load).  Both are read-only; writes raise a bus fault.
//!
//! Real silicon translates virt addresses through a 64-entry × 64 KiB MMU
//! page table that the firmware programs at boot via the EXTMEM peripheral.
//! For Plan 2 (fast-boot, static page table) we accept a `page_table` from
//! the boot path and consult it on every read.
//!
//! ## Sharing
//!
//! The same physical flash backing is mapped twice on the bus (once at
//! 0x4200_0000, once at 0x3C00_0000).  Both mappings share an
//! `Arc<Mutex<Vec<u8>>>` backing buffer so writes through either alias —
//! though writes are forbidden in Plan 2 — would be coherent.

use crate::{Peripheral, SimResult, SimulationError};
use std::sync::{Arc, Mutex};

const PAGE_SIZE: u32 = 64 * 1024;
const PAGE_TABLE_ENTRIES: usize = 64;

#[derive(Debug, Clone)]
pub struct FlashXipPeripheral {
    backing: Arc<Mutex<Vec<u8>>>,
    /// Maps virtual page index (offset within the 4 MiB window) to physical
    /// page index (offset within the flash backing).  `None` = unmapped.
    page_table: [Option<u16>; PAGE_TABLE_ENTRIES],
    base: u32,
}

impl FlashXipPeripheral {
    /// Create a new instance with a shared backing buffer and an unpopulated
    /// page table.  `base` is `0x4200_0000` for I-cache or `0x3C00_0000`
    /// for D-cache.
    pub fn new_shared(backing: Arc<Mutex<Vec<u8>>>, base: u32) -> Self {
        Self {
            backing,
            page_table: [None; PAGE_TABLE_ENTRIES],
            base,
        }
    }

    /// Map virtual page `virt` (0..=63) to physical page `phys` in the
    /// backing buffer.
    pub fn map_page(&mut self, virt: u8, phys: u16) {
        assert!((virt as usize) < PAGE_TABLE_ENTRIES, "virt page out of range");
        self.page_table[virt as usize] = Some(phys);
    }

    /// Identity-map all pages (virt page N → phys page N).  Useful for tests
    /// and fast-boot fallback when the firmware's expected segment layout
    /// matches a 1:1 mapping.
    pub fn map_identity(&mut self) {
        for i in 0..PAGE_TABLE_ENTRIES {
            self.page_table[i] = Some(i as u16);
        }
    }

    /// Returns the number of currently-mapped pages.
    pub fn pages_mapped(&self) -> usize {
        self.page_table.iter().filter(|p| p.is_some()).count()
    }

    fn translate(&self, offset: u64) -> Option<u64> {
        let virt_page = (offset / PAGE_SIZE as u64) as usize;
        let in_page = offset % PAGE_SIZE as u64;
        if virt_page >= PAGE_TABLE_ENTRIES {
            return None;
        }
        let phys_page = self.page_table[virt_page]?;
        Some(phys_page as u64 * PAGE_SIZE as u64 + in_page)
    }
}

impl Peripheral for FlashXipPeripheral {
    fn read(&self, offset: u64) -> SimResult<u8> {
        match self.translate(offset) {
            Some(phys) => {
                let backing = self.backing.lock().unwrap();
                Ok(*backing.get(phys as usize).unwrap_or(&0))
            }
            None => Ok(0), // unmapped page reads as 0
        }
    }

    fn write(&mut self, offset: u64, _value: u8) -> SimResult<()> {
        Err(SimulationError::MemoryViolation(
            self.base as u64 + offset,
        ))
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
    fn unmapped_pages_read_as_zero() {
        let backing = Arc::new(Mutex::new(vec![0xAAu8; 64 * 1024]));
        let p = FlashXipPeripheral::new_shared(backing, 0x4200_0000);
        // No pages mapped: read returns 0 even though backing has 0xAA.
        assert_eq!(p.read(0).unwrap(), 0);
    }

    #[test]
    fn mapped_page_reads_through_to_backing() {
        let mut backing = vec![0u8; PAGE_SIZE as usize];
        backing[0] = 0xCA;
        backing[1] = 0xFE;
        let backing = Arc::new(Mutex::new(backing));
        let mut p = FlashXipPeripheral::new_shared(backing, 0x4200_0000);
        p.map_page(0, 0);
        assert_eq!(p.read(0).unwrap(), 0xCA);
        assert_eq!(p.read(1).unwrap(), 0xFE);
    }

    #[test]
    fn writes_are_forbidden() {
        let backing = Arc::new(Mutex::new(vec![0u8; 64 * 1024]));
        let mut p = FlashXipPeripheral::new_shared(backing, 0x4200_0000);
        p.map_identity();
        let err = p.write(0, 0xAA).unwrap_err();
        match err {
            SimulationError::MemoryViolation(_) => {}
            other => panic!("expected MemoryViolation, got {other:?}"),
        }
    }

    #[test]
    fn cross_page_remap_works() {
        let mut backing = vec![0u8; PAGE_SIZE as usize * 2];
        backing[PAGE_SIZE as usize] = 0xAB; // first byte of physical page 1
        let backing = Arc::new(Mutex::new(backing));
        let mut p = FlashXipPeripheral::new_shared(backing, 0x4200_0000);
        // Map virtual page 0 → physical page 1.
        p.map_page(0, 1);
        assert_eq!(p.read(0).unwrap(), 0xAB);
    }

    #[test]
    fn shared_backing_visible_to_both_aliases() {
        let mut buf = vec![0u8; PAGE_SIZE as usize];
        buf[0] = 0x42;
        let backing = Arc::new(Mutex::new(buf));
        let mut p_icache = FlashXipPeripheral::new_shared(backing.clone(), 0x4200_0000);
        let mut p_dcache = FlashXipPeripheral::new_shared(backing.clone(), 0x3C00_0000);
        p_icache.map_identity();
        p_dcache.map_identity();
        assert_eq!(p_icache.read(0).unwrap(), 0x42);
        assert_eq!(p_dcache.read(0).unwrap(), 0x42);
    }
}
