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
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const PAGE_SIZE: u32 = 64 * 1024;
const PAGE_TABLE_ENTRIES: usize = 64;
const PAGE_BYTES: usize = PAGE_SIZE as usize;

/// One mirrored physical flash page for lock-free instruction fetch.
///
/// # Safety / threading
/// LabWired runs one `Machine` on one thread. The mirror is filled under the
/// flash-backing mutex and then read without locks. `UnsafeCell` makes the
/// byte array interior-mutable; [`Sync`] is asserted because the only writer
/// is the same thread that reads (no concurrent fill vs fetch).
struct PageMirror {
    /// Physical page index currently held, or `u32::MAX` if empty.
    phys: AtomicU32,
    bytes: UnsafeCell<[u8; PAGE_BYTES]>,
}

// SAFETY: single-threaded Machine ownership (see struct docs).
unsafe impl Sync for PageMirror {}

impl std::fmt::Debug for PageMirror {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageMirror")
            .field("phys", &self.phys.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl PageMirror {
    fn empty() -> Self {
        Self {
            phys: AtomicU32::new(u32::MAX),
            bytes: UnsafeCell::new([0u8; PAGE_BYTES]),
        }
    }

    fn matches(&self, phys_page: u32) -> bool {
        self.phys.load(Ordering::Acquire) == phys_page
    }

    /// Copy `src` (exactly one physical page, zero-padded if short) into the
    /// mirror and publish `phys_page`.
    fn fill(&self, phys_page: u32, src: &[u8]) {
        // SAFETY: exclusive fill on the Machine thread; no concurrent read_bytes
        // mid-fill (read_bytes calls fill then reads, never re-enters).
        let dst = unsafe { &mut *self.bytes.get() };
        let n = src.len().min(PAGE_BYTES);
        dst[..n].copy_from_slice(&src[..n]);
        if n < PAGE_BYTES {
            dst[n..].fill(0);
        }
        self.phys.store(phys_page, Ordering::Release);
    }

    /// Read `len` bytes at `in_page` into `out`. Caller must ensure `matches`.
    fn copy_from(&self, in_page: usize, out: &mut [u8]) {
        debug_assert!(in_page + out.len() <= PAGE_BYTES);
        // SAFETY: page published with Release; caller checked matches() with
        // Acquire. Single-threaded Machine: no concurrent fill.
        let src = unsafe { &*self.bytes.get() };
        out.copy_from_slice(&src[in_page..in_page + out.len()]);
    }
}

/// ESP32-S3 hardware MMU constants (soc/esp32s3 `ext_mem_defs.h`). The flash
/// cache MMU has 512 entries of 64 KiB each, covering a 32 MiB linear window
/// shared by the D-bus (0x3C00_0000) and I-bus (0x4200_0000) cache regions.
/// The per-entry valid/invalid + page-number layout now lives in [`MmuFmt`]
/// (see [`MMU_FMT_S3`]/[`MMU_FMT_C3`]); only the reset/invalid flag and the
/// table length are needed here to allocate and reset a fresh table.
const SOC_MMU_INVALID: u32 = 1 << 14; // S3 entry-invalid flag (reset state)
pub const SOC_MMU_ENTRY_NUM: usize = 512;

/// Flash-cache MMU table shared between the MMU-register peripheral (which
/// the boot ROM / bootloader program) and the XIP windows that translate
/// through it.
///
/// `generation` bumps on every entry write so XIP windows can cache the last
/// page translation without re-locking the table on every instruction fetch
/// (the dominant cost on C3 OLED — see native sample: ~60% of time in
/// `FlashXipPeripheral::translate` + `read_u32` byte-splitting).
#[derive(Debug)]
pub struct SharedMmu {
    pub entries: Mutex<Vec<u32>>,
    /// Monotonic; XIP caches compare against this before trusting a hit.
    pub generation: AtomicU64,
}

/// Shared handle for the flash-cache MMU table.
pub type SharedMmuTable = Arc<SharedMmu>;

/// Allocate a fresh MMU table with every entry marked invalid, matching the
/// silicon reset state.
pub fn new_mmu_table() -> SharedMmuTable {
    Arc::new(SharedMmu {
        entries: Mutex::new(vec![SOC_MMU_INVALID; SOC_MMU_ENTRY_NUM]),
        generation: AtomicU64::new(1),
    })
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

#[derive(Debug)]
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
    /// Last MMU page translation (generation, entry_id, phys_page). Atomic so
    /// `&self` reads stay Sync without taking the MMU mutex on the hot path.
    xlat_gen: AtomicU64,
    xlat_entry_id: AtomicU32,
    xlat_phys_page: AtomicU32,
    /// Mirrored physical page — steady-state instruction fetch never takes
    /// `backing`'s mutex (profile: ~half of post-word-read_u32 cost was
    /// `pthread_mutex_lock` on the flash Vec). Filled under the backing lock
    /// on miss; SPI flash does not mutate the Vec today (program/erase only
    /// update status regs), so the mirror stays coherent for current models.
    page_mirror: PageMirror,
}

// Manual Clone: atomics copy by load; cache is a hint so a clone starting cold
// is fine (next translate refills).
impl Clone for FlashXipPeripheral {
    fn clone(&self) -> Self {
        Self {
            backing: self.backing.clone(),
            page_table: self.page_table,
            mmu_table: self.mmu_table.clone(),
            fmt: self.fmt,
            base: self.base,
            xlat_gen: AtomicU64::new(0),
            xlat_entry_id: AtomicU32::new(u32::MAX),
            xlat_phys_page: AtomicU32::new(0),
            page_mirror: PageMirror::empty(),
        }
    }
}

impl FlashXipPeripheral {
    /// Create a new instance with a shared backing buffer and an unpopulated
    /// page table.  `base` is `0x4200_0000` for I-cache or `0x3C00_0000`
    /// for D-cache.
    fn new_fields(
        backing: Arc<Mutex<Vec<u8>>>,
        base: u32,
        mmu_table: Option<SharedMmuTable>,
        fmt: MmuFmt,
    ) -> Self {
        Self {
            backing,
            page_table: [None; PAGE_TABLE_ENTRIES],
            mmu_table,
            fmt,
            base,
            xlat_gen: AtomicU64::new(0),
            xlat_entry_id: AtomicU32::new(u32::MAX),
            xlat_phys_page: AtomicU32::new(0),
            page_mirror: PageMirror::empty(),
        }
    }

    pub fn new_shared(backing: Arc<Mutex<Vec<u8>>>, base: u32) -> Self {
        Self::new_fields(backing, base, None, MMU_FMT_S3)
    }

    /// Proper-model constructor with an explicit chip MMU format (e.g.
    /// [`MMU_FMT_C3`]). Use for chips whose entry layout isn't the S3 default.
    pub fn new_mmu_fmt(
        backing: Arc<Mutex<Vec<u8>>>,
        base: u32,
        mmu_table: SharedMmuTable,
        fmt: MmuFmt,
    ) -> Self {
        Self::new_fields(backing, base, Some(mmu_table), fmt)
    }

    /// Proper-model constructor: translate reads through the shared hardware
    /// MMU table the firmware programs (at `DR_REG_MMU_TABLE`), over a flash
    /// backing shared with the SPI-flash controller. This is the faithful XIP
    /// path used when booting the real ROM.
    pub fn new_mmu(backing: Arc<Mutex<Vec<u8>>>, base: u32, mmu_table: SharedMmuTable) -> Self {
        Self::new_fields(backing, base, Some(mmu_table), MMU_FMT_S3)
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
            let entry_id = (vaddr & self.fmt.vaddr_mask) >> 16;
            let in_page = (vaddr & (PAGE_SIZE - 1)) as u64;
            let gen = mmu.generation.load(Ordering::Acquire);
            // Hot path: same MMU generation + same page → reuse phys_page with
            // no mutex. Generation bumps on every MMU register write, so a
            // remap cannot leave a stale phys_page in the cache.
            if self.xlat_gen.load(Ordering::Relaxed) == gen
                && self.xlat_entry_id.load(Ordering::Relaxed) == entry_id
            {
                let phys_page = self.xlat_phys_page.load(Ordering::Relaxed) as u64;
                // Re-check generation so a concurrent MMU write cannot leave us
                // with a stale phys_page under a recycled entry_id.
                if mmu.generation.load(Ordering::Acquire) == gen {
                    return Some(phys_page * PAGE_SIZE as u64 + in_page);
                }
            }
            let table = mmu.entries.lock().unwrap();
            let entry = *table.get(entry_id as usize)?;
            if entry & self.fmt.invalid_bit != 0 {
                return None; // unmapped MMU entry
            }
            let phys_page = entry & self.fmt.valid_val_mask;
            // Publish cache for subsequent fetches in this page.
            self.xlat_phys_page.store(phys_page, Ordering::Relaxed);
            self.xlat_entry_id.store(entry_id, Ordering::Relaxed);
            self.xlat_gen.store(gen, Ordering::Release);
            return Some(phys_page as u64 * PAGE_SIZE as u64 + in_page);
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

    /// Ensure the page mirror holds `phys_page`, filling from `backing` on miss.
    fn ensure_page_mirror(&self, phys_page: u32) {
        if self.page_mirror.matches(phys_page) {
            return;
        }
        let backing = self.backing.lock().unwrap();
        // Re-check under the lock in case of nested fills (should not happen).
        if self.page_mirror.matches(phys_page) {
            return;
        }
        let start = (phys_page as usize).saturating_mul(PAGE_BYTES);
        let end = (start + PAGE_BYTES).min(backing.len());
        let slice = if start < backing.len() {
            &backing[start..end]
        } else {
            &[][..]
        };
        self.page_mirror.fill(phys_page, slice);
    }

    /// Read consecutive bytes starting at window `offset` into `out`.
    /// One MMU translate (often cached) + lock-free page-mirror hit for the
    /// in-page instruction-fetch case. Mirror miss takes the backing mutex once
    /// per physical page (64 KiB of guest code).
    fn read_bytes(&self, offset: u64, out: &mut [u8]) {
        if out.is_empty() {
            return;
        }
        let Some(phys0) = self.translate(offset) else {
            out.fill(0);
            return;
        };
        let in_page = (offset % PAGE_SIZE as u64) as usize;
        if in_page + out.len() <= PAGE_BYTES {
            let phys_page = (phys0 / PAGE_SIZE as u64) as u32;
            self.ensure_page_mirror(phys_page);
            self.page_mirror.copy_from(in_page, out);
            return;
        }
        // Rare: multi-page span — fall back to per-byte path via mirror.
        for (i, b) in out.iter_mut().enumerate() {
            match self.translate(offset + i as u64) {
                Some(phys) => {
                    let phys_page = (phys / PAGE_SIZE as u64) as u32;
                    let in_p = (phys % PAGE_SIZE as u64) as usize;
                    self.ensure_page_mirror(phys_page);
                    let mut one = [0u8; 1];
                    self.page_mirror.copy_from(in_p, &mut one);
                    *b = one[0];
                }
                None => *b = 0,
            }
        }
    }

    /// Drop the mirrored page (call if a future SPI path mutates flash bytes).
    #[allow(dead_code)]
    pub fn invalidate_page_mirror(&self) {
        self.page_mirror.phys.store(u32::MAX, Ordering::Release);
    }

    /// Bulk read for the CPU instruction-fetch window. Same bytes as
    /// repeated `read_u32` / `read` over the span (translate + page mirror).
    pub(crate) fn read_span(&self, offset: u64, out: &mut [u8]) {
        self.read_bytes(offset, out);
    }
}

impl Peripheral for FlashXipPeripheral {
    // Inert walk: read-only XIP window translating through the shared MMU table on access; tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    /// Code/rodata XIP has no side effects; ignore for host poll-coalesce.
    fn mmio_access_class(&self, _offset: u64) -> crate::MmioAccessClass {
        crate::MmioAccessClass::SideEffectFree
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let mut b = [0u8; 1];
        self.read_bytes(offset, &mut b);
        let r = b[0];
        if offset < 0x40 && std::env::var("LABWIRED_XIP_DEBUG").is_ok() {
            eprintln!(
                "xip: base=0x{:08x} off=0x{offset:x} -> phys={:?} = 0x{r:02x}",
                self.base,
                self.translate(offset)
            );
        }
        Ok(r)
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let mut b = [0u8; 2];
        self.read_bytes(offset, &mut b);
        Ok(u16::from_le_bytes(b))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        // Instruction fetch hot path: one translate (often cached) + one
        // backing lock for 4 bytes, instead of 4× (translate+lock) from the
        // default byte-splitting Peripheral::read_u32.
        let mut b = [0u8; 4];
        self.read_bytes(offset, &mut b);
        Ok(u32::from_le_bytes(b))
    }

    fn write(&mut self, offset: u64, _value: u8) -> SimResult<()> {
        Err(SimulationError::MemoryViolation(self.base as u64 + offset))
    }

    fn legacy_tick_active(&self) -> bool {
        false
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
    // Inert walk: MMU page-table register file (entries stored at the write); tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = match Self::entry_index(offset & !3) {
            Some(i) => self.table.entries.lock().unwrap()[i],
            None => 0,
        };
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if let Some(i) = Self::entry_index(offset & !3) {
            let mut t = self.table.entries.lock().unwrap();
            let byte_off = (offset & 3) * 8;
            t[i] = (t[i] & !(0xFFu32 << byte_off)) | ((value as u32) << byte_off);
            // Drop the lock before bumping gen so XIP windows never observe a
            // half-updated entry with a new generation (or vice versa).
            drop(t);
            self.table.generation.fetch_add(1, Ordering::Release);
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match Self::entry_index(offset & !3) {
            Some(i) => self.table.entries.lock().unwrap()[i],
            None => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if let Some(i) = Self::entry_index(offset & !3) {
            if std::env::var("LABWIRED_XIP_DEBUG").is_ok() {
                eprintln!("mmu: entry[{i}] <- 0x{value:08x}");
            }
            self.table.entries.lock().unwrap()[i] = value;
            self.table.generation.fetch_add(1, Ordering::Release);
        }
        Ok(())
    }

    fn legacy_tick_active(&self) -> bool {
        false
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
        let table = self.table.entries.lock().unwrap();
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
        let mut table = self.table.entries.lock().unwrap();
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
        drop(table);
        // Invalidate every XIP window's page-translation cache.
        self.table.generation.fetch_add(1, Ordering::Release);
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
        mmu.entries.lock().unwrap()[entry_id] = 128 & MMU_FMT_S3.valid_val_mask; // VALID (bit14=0)
        mmu.generation.fetch_add(1, Ordering::Release);
        let d = FlashXipPeripheral::new_mmu(backing, 0x3C00_0000, mmu);
        // Read at the window offset for vaddr 0x3C80_0000.
        assert_eq!(d.read(0x80_0000).unwrap(), 0xDE);
        assert_eq!(d.read(0x80_0001).unwrap(), 0xAD);
        // Word fetch (instruction path) must match byte path.
        assert_eq!(d.read_u32(0x80_0000).unwrap() & 0xFFFF, 0xADDE);
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
        assert_eq!(mmu.entries.lock().unwrap()[128], 64);
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
