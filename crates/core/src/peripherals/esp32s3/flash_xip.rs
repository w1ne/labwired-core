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

/// ESP32-S3 hardware MMU constants (soc/esp32s3 `ext_mem_defs.h`). The flash
/// cache MMU has 512 entries of 64 KiB each, covering a 32 MiB linear window
/// shared by the D-bus (0x3C00_0000) and I-bus (0x4200_0000) cache regions.
/// The per-entry valid/invalid + page-number layout now lives in [`MmuFmt`]
/// (see [`MMU_FMT_S3`]/[`MMU_FMT_C3`]); only the reset/invalid flag and the
/// table length are needed here to allocate and reset a fresh table.
const SOC_MMU_INVALID: u32 = 1 << 14; // S3 entry-invalid flag (reset state)
pub const SOC_MMU_ENTRY_NUM: usize = 512;

/// A flash-cache MMU table shared between the MMU-register peripheral (which
/// the boot ROM / bootloader program) and the XIP windows that translate
/// through it. 512 little-endian u32 entries.
pub type SharedMmuTable = Arc<Mutex<Vec<u32>>>;

/// Allocate a fresh MMU table with every entry marked invalid, matching the
/// silicon reset state.
pub fn new_mmu_table() -> SharedMmuTable {
    Arc::new(Mutex::new(vec![SOC_MMU_INVALID; SOC_MMU_ENTRY_NUM]))
}

/// Per-chip flash-cache MMU entry format. The MMU table register block is the
/// same (`0x600C_5000`) and the entries are u32, but the valid/invalid flag and
/// physical-page-number field differ by SoC (soc/<chip>/ext_mem_defs.h).
#[derive(Debug, Clone, Copy)]
pub struct MmuFmt {
    /// Bit that marks an entry invalid (S3: BIT(14); C3: BIT(8)).
    pub invalid_bit: u32,
    /// Mask of the physical-page-number field (S3: 0x3FFF; C3: 0xFF).
    pub valid_val_mask: u32,
    /// Linear virtual-address span mask (entry_num*64KiB - 1).
    pub vaddr_mask: u32,
}

/// ESP32-S3 MMU format: 512 × 64 KiB entries, invalid = BIT(14).
pub const MMU_FMT_S3: MmuFmt = MmuFmt {
    invalid_bit: 1 << 14,
    valid_val_mask: 0x3FFF,
    vaddr_mask: 0x1FF_FFFF, // 32 MiB
};

/// ESP32-C3 MMU format: 128 × 64 KiB entries, invalid = BIT(8), 8 MiB span.
pub const MMU_FMT_C3: MmuFmt = MmuFmt {
    invalid_bit: 1 << 8,
    valid_val_mask: 0xFF,
    vaddr_mask: 0x7F_FFFF, // 8 MiB
};

#[derive(Debug, Clone)]
pub struct FlashXipPeripheral {
    backing: Arc<Mutex<Vec<u8>>>,
    /// Maps virtual page index (offset within the 4 MiB window) to physical
    /// page index (offset within the flash backing).  `None` = unmapped.
    /// Used only when `mmu_table` is `None` (fast-boot static mapping).
    page_table: [Option<u16>; PAGE_TABLE_ENTRIES],
    /// Proper-model translation: when present, reads translate through the
    /// real hardware MMU table the firmware programs, exactly as silicon does.
    mmu_table: Option<SharedMmuTable>,
    /// MMU entry format for this chip (defaults to S3).
    fmt: MmuFmt,
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
            mmu_table: None,
            fmt: MMU_FMT_S3,
            base,
        }
    }

    /// Proper-model constructor with an explicit chip MMU format (e.g.
    /// [`MMU_FMT_C3`]). Use for chips whose entry layout isn't the S3 default.
    pub fn new_mmu_fmt(
        backing: Arc<Mutex<Vec<u8>>>,
        base: u32,
        mmu_table: SharedMmuTable,
        fmt: MmuFmt,
    ) -> Self {
        Self {
            backing,
            page_table: [None; PAGE_TABLE_ENTRIES],
            mmu_table: Some(mmu_table),
            fmt,
            base,
        }
    }

    /// Proper-model constructor: translate reads through the shared hardware
    /// MMU table the firmware programs (at `DR_REG_MMU_TABLE`), over a flash
    /// backing shared with the SPI-flash controller. This is the faithful XIP
    /// path used when booting the real ROM.
    pub fn new_mmu(backing: Arc<Mutex<Vec<u8>>>, base: u32, mmu_table: SharedMmuTable) -> Self {
        Self {
            backing,
            page_table: [None; PAGE_TABLE_ENTRIES],
            mmu_table: Some(mmu_table),
            fmt: MMU_FMT_S3,
            base,
        }
    }

    /// Map virtual page `virt` (0..=63) to physical page `phys` in the
    /// backing buffer.
    pub fn map_page(&mut self, virt: u8, phys: u16) {
        assert!(
            (virt as usize) < PAGE_TABLE_ENTRIES,
            "virt page out of range"
        );
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
        // Proper-model path: translate through the real hardware MMU table.
        if let Some(mmu) = &self.mmu_table {
            let vaddr = self.base.wrapping_add(offset as u32);
            // entry_id = (vaddr & vaddr_mask) >> 16  (mmu_ll_get_entry_id)
            let entry_id = ((vaddr & self.fmt.vaddr_mask) >> 16) as usize;
            let in_page = (vaddr & (PAGE_SIZE - 1)) as u64;
            let table = mmu.lock().unwrap();
            let entry = *table.get(entry_id)?;
            if entry & self.fmt.invalid_bit != 0 {
                return None; // unmapped MMU entry
            }
            let phys_page = (entry & self.fmt.valid_val_mask) as u64;
            return Some(phys_page * PAGE_SIZE as u64 + in_page);
        }
        // Fast-boot static mapping.
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
        let r = match self.translate(offset) {
            Some(phys) => {
                let backing = self.backing.lock().unwrap();
                *backing.get(phys as usize).unwrap_or(&0)
            }
            None => 0, // unmapped page reads as 0
        };
        if offset < 0x40 && std::env::var("LABWIRED_XIP_DEBUG").is_ok() {
            eprintln!(
                "xip: base=0x{:08x} off=0x{offset:x} -> phys={:?} = 0x{r:02x}",
                self.base,
                self.translate(offset)
            );
        }
        Ok(r)
    }

    fn write(&mut self, offset: u64, _value: u8) -> SimResult<()> {
        Err(SimulationError::MemoryViolation(self.base as u64 + offset))
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The flash-cache MMU table register block at `DR_REG_MMU_TABLE`
/// (`0x600C_5000`). The boot ROM and 2nd-stage bootloader program the
/// virtual→physical flash page mappings by writing 32-bit entries here; the
/// XIP windows translate through the same shared table. 512 word entries.
#[derive(Debug)]
pub struct Esp32s3MmuTable {
    table: SharedMmuTable,
}

impl Esp32s3MmuTable {
    pub fn new(table: SharedMmuTable) -> Self {
        Self { table }
    }

    fn entry_index(offset: u64) -> Option<usize> {
        let idx = (offset / 4) as usize;
        (idx < SOC_MMU_ENTRY_NUM).then_some(idx)
    }
}

impl Peripheral for Esp32s3MmuTable {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = match Self::entry_index(offset & !3) {
            Some(i) => self.table.lock().unwrap()[i],
            None => 0,
        };
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if let Some(i) = Self::entry_index(offset & !3) {
            let mut t = self.table.lock().unwrap();
            let byte_off = (offset & 3) * 8;
            t[i] = (t[i] & !(0xFFu32 << byte_off)) | ((value as u32) << byte_off);
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match Self::entry_index(offset & !3) {
            Some(i) => self.table.lock().unwrap()[i],
            None => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if let Some(i) = Self::entry_index(offset & !3) {
            if std::env::var("LABWIRED_XIP_DEBUG").is_ok() {
                eprintln!("mmu: entry[{i}] <- 0x{value:08x}");
            }
            self.table.lock().unwrap()[i] = value;
        }
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Capture the live MMU page table. This is boot-critical state on the
    /// rom-boot resume path: the 2nd-stage bootloader programs the
    /// virtual->flash page mapping here, and the XIP windows (0x4200_0000 /
    /// 0x3C00_0000) translate through the same shared table. Without it a
    /// resume would fetch from unmapped flash and decode garbage. The
    /// registered `mmu_table` peripheral owns the `Arc`, so restoring it here
    /// also fixes every FlashXip window that shares it.
    fn runtime_snapshot(&self) -> Vec<u8> {
        let table = self.table.lock().unwrap();
        let mut out = Vec::with_capacity(table.len() * 4);
        for &word in table.iter() {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.len() % 4 != 0 {
            return Err(SimulationError::NotImplemented(format!(
                "Esp32s3MmuTable snapshot must be a whole number of u32 words, got {} bytes",
                bytes.len()
            )));
        }
        let mut table = self.table.lock().unwrap();
        let n = bytes.len() / 4;
        if n != table.len() {
            return Err(SimulationError::NotImplemented(format!(
                "Esp32s3MmuTable snapshot has {n} entries, table has {}",
                table.len()
            )));
        }
        for (i, chunk) in bytes.chunks_exact(4).enumerate() {
            table[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        Ok(())
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
    fn mmu_translation_reads_mapped_flash_page() {
        // 16 MiB flash backing; put a marker at physical page 128 (0x80_0000).
        let mut flash = vec![0u8; 16 * 1024 * 1024];
        let phys = 128usize * PAGE_SIZE as usize;
        flash[phys] = 0xDE;
        flash[phys + 1] = 0xAD;
        let backing = Arc::new(Mutex::new(flash));
        let mmu = new_mmu_table();
        // Map D-bus virtual 0x3C80_0000 (entry 128) → physical page 128 (valid).
        let entry_id = ((0x3C80_0000u32 & MMU_FMT_S3.vaddr_mask) >> 16) as usize;
        assert_eq!(entry_id, 128);
        mmu.lock().unwrap()[entry_id] = 128 & MMU_FMT_S3.valid_val_mask; // VALID (bit14=0)
        let d = FlashXipPeripheral::new_mmu(backing, 0x3C00_0000, mmu);
        // Read at the window offset for vaddr 0x3C80_0000.
        assert_eq!(d.read(0x80_0000).unwrap(), 0xDE);
        assert_eq!(d.read(0x80_0001).unwrap(), 0xAD);
    }

    #[test]
    fn mmu_invalid_entry_reads_zero() {
        let backing = Arc::new(Mutex::new(vec![0xFFu8; PAGE_SIZE as usize]));
        let mmu = new_mmu_table(); // all entries invalid
        let d = FlashXipPeripheral::new_mmu(backing, 0x3C00_0000, mmu);
        assert_eq!(d.read(0).unwrap(), 0);
    }

    #[test]
    fn mmu_table_peripheral_round_trips_entries() {
        let mmu = new_mmu_table();
        let mut p = Esp32s3MmuTable::new(mmu.clone());
        // Write entry 128 (offset 0x200) = physical page 64, valid.
        p.write_u32(128 * 4, 64).unwrap();
        assert_eq!(p.read_u32(128 * 4).unwrap(), 64);
        assert_eq!(mmu.lock().unwrap()[128], 64);
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
