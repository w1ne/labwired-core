// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Classic ESP32 flash-cache MMU + hybrid XIP windows.
//!
//! Silicon layout (TRM / `soc/dport_reg.h` / `hal/mmu_ll.h`):
//! - `DPORT_PRO_FLASH_MMU_TABLE` @ `0x3FF1_0000` (384 × u32)
//! - `DPORT_APP_FLASH_MMU_TABLE` @ `0x3FF1_2000` (384 × u32)
//! - Entry: bit8 = invalid (`SOC_MMU_INVALID`), bits[7:0] = flash page (64 KiB)
//! - Page size always 64 KiB
//!
//! Fast-boot loads the ELF into an overlay (`write` path). Dirty overlay pages
//! serve instruction/data for execution. Clean pages with a valid MMU entry
//! serve the shared flash backing — that is how temporary `spi_flash_mmap` of
//! the partition table (flash `0x8000`) becomes visible without a firmware
//! OTA thunk. Pre-seeded MMU entries for the app IROM/DROM ranges make
//! `spi_flash_cache2phys` return a physical address inside `app0`.

use crate::{Peripheral, SimResult};
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

/// ESP32 flash MMU page size (bytes).
pub const PAGE_SIZE: u32 = 64 * 1024;
/// Number of MMU table entries used by IDF for flash (excludes PSRAM tail).
pub const ENTRY_NUM: usize = 384;
/// `SOC_MMU_INVALID` — entry is free / unmapped when this bit is set.
pub const MMU_INVALID: u32 = 1 << 8;

const VADDR_MASK: u32 = 0x003F_FFFF;

/// Shared PRO/APP MMU tables + physical flash array.
#[derive(Debug)]
pub struct Esp32FlashShared {
    pub flash: Arc<Mutex<Vec<u8>>>,
    pub pro_mmu: Mutex<Vec<u32>>,
    pub app_mmu: Mutex<Vec<u32>>,
}

impl Esp32FlashShared {
    pub fn new(flash_size: usize) -> Arc<Self> {
        Arc::new(Self {
            flash: Arc::new(Mutex::new(vec![0xFFu8; flash_size])),
            pro_mmu: Mutex::new(vec![MMU_INVALID; ENTRY_NUM]),
            app_mmu: Mutex::new(vec![MMU_INVALID; ENTRY_NUM]),
        })
    }

    /// Write `data` into the physical flash image at `offset`.
    pub fn write_flash(this: &Arc<Self>, offset: usize, data: &[u8]) {
        let mut flash = this.flash.lock().unwrap();
        if offset >= flash.len() {
            return;
        }
        let n = data.len().min(flash.len() - offset);
        flash[offset..offset + n].copy_from_slice(&data[..n]);
    }

    /// Pre-seed MMU so `cache2phys` of app IROM/DROM lands inside `app0`
    /// (flash base `app_flash_base`, typically `0x1_0000`). Does **not**
    /// change overlay contents — execution still comes from the ELF load.
    pub fn seed_app_xip_mmu(this: &Arc<Self>, app_flash_base: u32, num_pages: u32) {
        let first_phys = app_flash_base / PAGE_SIZE;
        let mut pro = this.pro_mmu.lock().unwrap();
        let mut app = this.app_mmu.lock().unwrap();
        for i in 0..num_pages {
            let phys = first_phys.wrapping_add(i) & 0xFF;
            let drom_va = 0x3F40_0000u32.wrapping_add(i * PAGE_SIZE);
            if let Some(e) = entry_id(drom_va) {
                if e < ENTRY_NUM {
                    pro[e] = phys;
                    app[e] = phys;
                }
            }
            let irom_va = 0x400D_0000u32.wrapping_add(i * PAGE_SIZE);
            if let Some(e) = entry_id(irom_va) {
                if e < ENTRY_NUM {
                    pro[e] = phys;
                    app[e] = phys;
                }
            }
        }
    }
}

/// Map classic-ESP32 vaddr → MMU entry index (`mmu_ll_get_entry_id`).
pub fn entry_id(vaddr: u32) -> Option<usize> {
    let page = ((vaddr & VADDR_MASK) >> 16) as usize;
    if (0x3F40_0000..0x3F80_0000).contains(&vaddr) {
        Some(page) // DROM0, offset 0
    } else if (0x400D_0000..0x4040_0000).contains(&vaddr) {
        Some(64 + page) // IRAM0 cache
    } else if (0x4040_0000..0x4080_0000).contains(&vaddr) {
        Some(128 + page) // IRAM1 cache
    } else if (0x4080_0000..0x40C0_0000).contains(&vaddr) {
        Some(192 + page) // IROM0 cache
    } else {
        None
    }
}

/// MMU table register block (PRO or APP).
#[derive(Debug)]
pub struct Esp32FlashMmuRegs {
    shared: Arc<Esp32FlashShared>,
    is_pro: bool,
}

impl Esp32FlashMmuRegs {
    pub fn new_pro(shared: Arc<Esp32FlashShared>) -> Self {
        Self {
            shared,
            is_pro: true,
        }
    }
    pub fn new_app(shared: Arc<Esp32FlashShared>) -> Self {
        Self {
            shared,
            is_pro: false,
        }
    }

    pub fn shared(&self) -> &Arc<Esp32FlashShared> {
        &self.shared
    }

    fn table(&self) -> std::sync::MutexGuard<'_, Vec<u32>> {
        if self.is_pro {
            self.shared.pro_mmu.lock().unwrap()
        } else {
            self.shared.app_mmu.lock().unwrap()
        }
    }
}

impl Peripheral for Esp32FlashMmuRegs {
    fn needs_legacy_walk(&self) -> bool {
        false
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

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = (offset as usize) & !3;
        let shift = ((offset as usize) & 3) * 8;
        let idx = word_off / 4;
        let table = self.table();
        let w = table.get(idx).copied().unwrap_or(MMU_INVALID);
        Ok(((w >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = (offset as usize) & !3;
        let shift = ((offset as usize) & 3) * 8;
        let idx = word_off / 4;
        if idx >= ENTRY_NUM {
            return Ok(());
        }
        let mut table = self.table();
        let mut w = table[idx];
        w = (w & !(0xFFu32 << shift)) | ((value as u32) << shift);
        table[idx] = w;
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let idx = (offset as usize) / 4;
        let table = self.table();
        Ok(table.get(idx).copied().unwrap_or(MMU_INVALID))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let idx = (offset as usize) / 4;
        if idx < ENTRY_NUM {
            self.table()[idx] = value;
        }
        Ok(())
    }
}

/// Hybrid flash XIP window: dirty overlay (ELF) + clean MMU→flash path.
#[derive(Debug)]
pub struct ClassicFlashWindow {
    base: u32,
    size: usize,
    overlay: RefCell<Vec<u8>>,
    /// Bit i set ⇒ page i was written (serve overlay).
    dirty: RefCell<u64>,
    shared: Arc<Esp32FlashShared>,
}

impl ClassicFlashWindow {
    pub fn new(base: u32, size: usize, shared: Arc<Esp32FlashShared>) -> Self {
        assert!(size <= 4 * 1024 * 1024);
        Self {
            base,
            size,
            overlay: RefCell::new(vec![0u8; size]),
            dirty: RefCell::new(0),
            shared,
        }
    }

    fn page_of(offset: u64) -> usize {
        (offset / PAGE_SIZE as u64) as usize
    }

    fn mark_dirty(&self, offset: u64, nbytes: usize) {
        if nbytes == 0 {
            return;
        }
        let start = Self::page_of(offset);
        let end = Self::page_of(offset + nbytes as u64 - 1);
        let mut d = self.dirty.borrow_mut();
        for p in start..=end.min(63) {
            *d |= 1u64 << p;
        }
    }

    fn page_dirty(&self, page: usize) -> bool {
        page < 64 && (*self.dirty.borrow() & (1u64 << page)) != 0
    }

    fn translate_flash(&self, offset: u64) -> Option<usize> {
        let vaddr = self.base.wrapping_add(offset as u32);
        let entry = entry_id(vaddr)?;
        let mmu = self.shared.pro_mmu.lock().unwrap();
        let val = *mmu.get(entry)?;
        if val & MMU_INVALID != 0 {
            return None;
        }
        let phys_page = (val & 0xFF) as usize;
        let in_page = (offset as usize) & (PAGE_SIZE as usize - 1);
        Some(phys_page * PAGE_SIZE as usize + in_page)
    }

    fn read_byte(&self, offset: u64) -> u8 {
        if offset as usize >= self.size {
            return 0;
        }
        let page = Self::page_of(offset);
        if self.page_dirty(page) {
            return self.overlay.borrow()[offset as usize];
        }
        if let Some(phys) = self.translate_flash(offset) {
            let flash = self.shared.flash.lock().unwrap();
            return flash.get(phys).copied().unwrap_or(0xFF);
        }
        self.overlay.borrow()[offset as usize]
    }
}

impl Peripheral for ClassicFlashWindow {
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.read_byte(offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if (offset as usize) < self.size {
            self.overlay.borrow_mut()[offset as usize] = value;
            self.mark_dirty(offset, 1);
        }
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let b0 = self.read_byte(offset) as u16;
        let b1 = self.read_byte(offset + 1) as u16;
        Ok(b0 | (b1 << 8))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let b0 = self.read_byte(offset) as u32;
        let b1 = self.read_byte(offset + 1) as u32;
        let b2 = self.read_byte(offset + 2) as u32;
        let b3 = self.read_byte(offset + 3) as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write(offset, value as u8)?;
        self.write(offset + 1, (value >> 8) as u8)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write(offset, value as u8)?;
        self.write(offset + 1, (value >> 8) as u8)?;
        self.write(offset + 2, (value >> 16) as u8)?;
        self.write(offset + 3, (value >> 24) as u8)
    }
}

/// Seed partition table at flash `0x8000`, pre-map app XIP MMU for
/// `cache2phys`, and attach the same flash image to SPI0/SPI1.
pub fn seed_esp32_flash_image(
    bus: &mut crate::bus::SystemBus,
    partitions: Option<&[u8]>,
) -> anyhow::Result<()> {
    let idx = bus
        .find_peripheral_index_by_name("flash_mmu_pro")
        .ok_or_else(|| anyhow::anyhow!("flash_mmu_pro missing — configure_xtensa_esp32 first"))?;
    let shared = {
        let any = bus.peripherals[idx]
            .dev
            .as_any()
            .ok_or_else(|| anyhow::anyhow!("flash_mmu_pro as_any missing"))?;
        let regs = any
            .downcast_ref::<Esp32FlashMmuRegs>()
            .ok_or_else(|| anyhow::anyhow!("flash_mmu_pro type mismatch"))?;
        regs.shared().clone()
    };

    if let Some(pt) = partitions {
        Esp32FlashShared::write_flash(&shared, 0x8000, pt);
    }
    // App image identity for cache2phys: map ~1 MiB of IROM/DROM → flash 0x10000+.
    Esp32FlashShared::seed_app_xip_mmu(&shared, 0x1_0000, 16);

    for name in ["spi0", "spi1"] {
        if let Some(spi_idx) = bus.find_peripheral_index_by_name(name) {
            if let Some(spi) = bus.peripherals[spi_idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<crate::peripherals::esp32::spi::Esp32Spi>())
            {
                spi.set_flash_backing(shared.flash.clone());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_id_irom_app_text() {
        // 0x400D_4748 → page 0xD within IRAM0 → entry 64+13 = 77
        assert_eq!(entry_id(0x400D_4748), Some(77));
    }

    #[test]
    fn entry_id_drom_base() {
        // 0x3F40_0000 & 0x3F_FFFF == 0 → first DROM MMU entry
        assert_eq!(entry_id(0x3F40_0000), Some(0));
        assert_eq!(entry_id(0x3F41_0000), Some(1));
    }

    #[test]
    fn hybrid_clean_page_serves_flash() {
        let shared = Esp32FlashShared::new(4 * 1024 * 1024);
        Esp32FlashShared::write_flash(&shared, 0x8000, &[0xAA, 0x50]);
        {
            let e = entry_id(0x3F40_0000).unwrap();
            shared.pro_mmu.lock().unwrap()[e] = 0; // valid, page 0
        }
        let win = ClassicFlashWindow::new(0x3F40_0000, 0x40_0000, shared);
        assert_eq!(win.read_byte(0x8000), 0xAA);
        assert_eq!(win.read_byte(0x8001), 0x50);
    }

    #[test]
    fn hybrid_dirty_page_prefers_overlay() {
        let shared = Esp32FlashShared::new(4 * 1024 * 1024);
        {
            let e = entry_id(0x400D_0000).unwrap();
            shared.pro_mmu.lock().unwrap()[e] = 1;
        }
        let mut win = ClassicFlashWindow::new(0x400D_0000, 0x40_0000, shared);
        win.write(0, 0xE9).unwrap();
        assert_eq!(win.read_byte(0), 0xE9);
    }
}
