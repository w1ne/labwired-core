// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP bring-up helpers used by `labwired test` (and only that path).
//!
//! Keep chip-specific flash/DRAM seeds out of the generic test driver so
//! `test.rs` stays a script runner, not a second ESP-IDF stage.

use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::{Path, PathBuf};

/// Resolve an ESP-IDF `partitions.bin` for flash seeding @ 0x8000.
///
/// Firmware-adjacent paths only (matrix copies next to the ELF). Never
/// hard-code matrix PIO work-dir L0 fall-backs.
pub fn resolve_esp_partitions_bin(firmware_path: &Path) -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(parent) = firmware_path.parent() {
        cands.push(parent.join("partitions.bin"));
        // out/<board>/<sketch>/firmware.elf → out/_pio_work/<board>__<sketch>/...
        if let (Some(sketch), Some(board_dir), Some(out)) = (
            parent.file_name(),
            parent.parent(),
            parent.parent().and_then(|p| p.parent()),
        ) {
            if let Some(board) = board_dir.file_name() {
                let cell = format!("{}__{}", board.to_string_lossy(), sketch.to_string_lossy());
                cands.push(
                    out.join("_pio_work")
                        .join(cell)
                        .join(".pio/build/matrix/partitions.bin"),
                );
            }
        }
    }
    cands.into_iter().find(|p| p.is_file())
}

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
