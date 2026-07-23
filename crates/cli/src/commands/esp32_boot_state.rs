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

/// Install hybrid-preserve TCB base + xthal window spill CPU-model workaround
/// (not a flash-init firmware thunk). Shared by classic and S3 test paths.
pub fn install_xtensa_freertos_workarounds(bus: &mut SystemBus, elf_bytes: &[u8]) {
    use labwired_core::peripherals::esp_xtensa_common::rom_thunks;
    for sym in ["pxCurrentTCBs", "pxCurrentTCB"] {
        if let Some(a) = labwired_loader::resolve_symbol_in_elf(elf_bytes, sym) {
            rom_thunks::PX_CURRENT_TCB_ADDR.with(|s| s.set(Some(a)));
            eprintln!("labwired-cli test: pxCurrentTCBs @0x{a:08x} (hybrid preserve key)");
            break;
        }
    }
    if let Some(pc) = labwired_loader::resolve_symbol_in_elf(elf_bytes, "xthal_window_spill_nw") {
        if let Err(e) = bus.install_flash_thunk(pc, rom_thunks::xthal_window_spill_thunk) {
            eprintln!("labwired-cli test: warn: xthal_window_spill_nw install failed: {e}");
        } else {
            eprintln!(
                "labwired-cli test: installed xthal_window_spill_nw CPU spill workaround @0x{pc:08x}"
            );
        }
    }
}

/// Install ESP32-C3 fast-boot behavioral twins used by `labwired test`.
///
/// Chip yaml alone is not enough for Arduino FreeRTOS app-entry: SPIMEM CMD
/// clear, ANA I2C, cache, SYSTIMER VALUE_VALID, RMT TX_END, MMU/XIP, and
/// matrix IRQ routing. Prefer [`SystemBus::replace_or_add_peripheral`] for
/// named stubs so `--watch-gpio` / inspect match the MMIO owner.
pub fn install_esp32c3_fast_boot(bus: &mut SystemBus, firmware_path: &Path) {
    use labwired_core::peripherals::esp32s3::flash_xip::{
        Esp32s3MmuTable, FlashXipPeripheral, SharedMmu, MMU_FMT_C3,
    };
    use std::sync::atomic::AtomicU64;
    use std::sync::{Arc, Mutex};

    let mut flash_img = vec![0xFFu8; 4 * 1024 * 1024];
    if let Some(p) = resolve_esp_partitions_bin(firmware_path) {
        if let Ok(pt) = std::fs::read(&p) {
            let n = pt.len().min(0xC00);
            flash_img[0x8000..0x8000 + n].copy_from_slice(&pt[..n]);
            eprintln!(
                "labwired-cli test: seeded C3 flash partitions ({} bytes) from {}",
                n,
                p.display()
            );
        }
    }
    let flash = Arc::new(Mutex::new(flash_img));
    bus.add_peripheral(
        "spimem1_flash",
        0x6000_2000,
        0x100,
        None,
        Box::new(
            labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(flash.clone()),
        ),
    );
    bus.add_peripheral(
        "spimem0_flash",
        0x6000_3000,
        0x100,
        None,
        Box::new(
            labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(flash.clone()),
        ),
    );
    bus.add_peripheral(
        "rtc_i2c_ana",
        0x6000_E000,
        0x400,
        None,
        Box::new(labwired_core::peripherals::esp32c3::ana_i2c::Esp32c3AnaI2c::new()),
    );
    bus.add_peripheral(
        "extmem_cache",
        0x600C_4000,
        0x400,
        None,
        Box::new(labwired_core::peripherals::esp32c3::cache::Esp32c3Cache::new()),
    );
    bus.replace_or_add_peripheral(
        "systimer",
        0x6002_3000,
        0x100,
        None,
        Box::new(
            labwired_core::peripherals::esp32s3::systimer::Systimer::new_with_source(
                160_000_000,
                37,
            ),
        ),
    );
    bus.add_peripheral(
        "apb_saradc",
        0x6004_0000,
        0x100,
        None,
        Box::new(labwired_core::peripherals::esp32c3::sar_adc::Esp32c3SarAdc::new()),
    );
    bus.replace_or_add_peripheral(
        "rmt",
        0x6001_6000,
        0x800,
        None,
        Box::new(labwired_core::peripherals::esp32c3::rmt::Esp32c3Rmt::new_default()),
    );
    let entries = vec![MMU_FMT_C3.invalid_bit; 128];
    let mmu_table = Arc::new(SharedMmu {
        entries: Mutex::new(entries),
        generation: AtomicU64::new(1),
    });
    bus.add_peripheral(
        "mmu_table",
        0x600C_5000,
        0x800,
        None,
        Box::new(Esp32s3MmuTable::new(mmu_table.clone())),
    );
    bus.add_peripheral(
        "flash_xip_drom",
        0x3C00_0000,
        0x80_0000,
        None,
        Box::new(FlashXipPeripheral::new_mmu_fmt(
            flash,
            0x3C00_0000,
            mmu_table,
            MMU_FMT_C3,
        )),
    );
    bus.config.optimized_bus_access = false;
    // FreeRTOS first yield needs FROM_CPU matrix → riscv_irq_lines.
    bus.esp32c3_irq_routing = true;
    bus.refresh_peripheral_index();
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
