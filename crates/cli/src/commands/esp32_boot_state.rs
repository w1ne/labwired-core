// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Post-BROM DRAM seeds for classic ESP32 when the boot ROM is skipped.
//!
//! This is **not** a firmware flash-thunk path: it writes the same DRAM
//! globals a real `esp_rom_spiflash_attach` + clock init would leave so
//! IDF `esp_flash_init_default_chip` / delay math observe valid state.

use labwired_core::bus::SystemBus;
use labwired_core::Bus;

/// Seed `g_rom_flashchip` + `g_ticks_per_us_*` from ELF symbols.
pub fn seed_esp32_post_brom_dram(bus: &mut SystemBus, elf_bytes: &[u8]) {
    // esp_rom_spiflash_chip_t — Winbond W25Q32-class (4 MiB).
    if let Some(addr) = labwired_loader::resolve_symbol_in_elf(elf_bytes, "g_rom_flashchip") {
        let base = addr as u64;
        let _ = bus.write_u32(base, 0x0016_40EF); // device_id
        let _ = bus.write_u32(base + 4, 4 * 1024 * 1024); // chip_size
        let _ = bus.write_u32(base + 8, 64 * 1024); // block_size
        let _ = bus.write_u32(base + 12, 4 * 1024); // sector_size
        let _ = bus.write_u32(base + 16, 256); // page_size
        let _ = bus.write_u32(base + 20, 0xFFFF); // status_mask
        eprintln!(
            "labwired-cli test: seeded g_rom_flashchip @0x{addr:08x} (post-BROM flash attach state)"
        );
    }

    for name in ["g_ticks_per_us_pro", "g_ticks_per_us_app"] {
        if let Some(addr) = labwired_loader::resolve_symbol_in_elf(elf_bytes, name) {
            let _ = bus.write_u32(addr as u64, 240); // 240 MHz
        }
    }
}
