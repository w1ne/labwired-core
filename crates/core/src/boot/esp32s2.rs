// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Fast boot for an ESP-IDF / Arduino-ESP32 application ELF on the ESP32-S2.
//!
//! On silicon the 2nd-stage bootloader reads the factory app image from flash
//! `0x10000`, copies its RAM segments to IRAM/DRAM, and programs the MMU so
//! the app's `.flash.rodata` (DROM0, `0x3F00_xxxx`) and `.flash.text` (IRAM0
//! cache, `0x4008_xxxx`) pages read from the flash pages that hold them.
//! This does the same from the ELF: RAM segments go through the bus, flash
//! segments into the flash backing, one 64 KiB page per virtual page, DROM
//! pages first from `0x10000` and IROM pages after them, and each page's MMU
//! entry is written as the bootloader writes it (`mmu_ll_write_entry`:
//! `paddr_page | SOC_MMU_ACCESS_FLASH`). The partition table, if given, is
//! placed at `0x8000`, where `esp_partition` reads it.

use crate::boot::esp32s3::BootSummary;
use crate::boot::{BootError, BootResult};
use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::system::xtensa::{
    Esp32s2Flash, ESP32S2_DROM0_BASE, ESP32S2_DROM0_ENTRY, ESP32S2_IROM0_BASE, ESP32S2_IROM0_ENTRY,
    ESP32S2_MMU_ACCESS_FLASH,
};
use crate::{Bus, Cpu};
use goblin::elf::program_header::PT_LOAD;
use goblin::elf::Elf;

const PAGE: u32 = 0x1_0000;
/// The factory app partition of the default Arduino partition tables.
const APP_FLASH_OFFSET: u32 = 0x1_0000;
const PARTITION_TABLE_OFFSET: usize = 0x8000;

/// The cache window holding `vaddr`, as (window base, first MMU entry).
/// DROM0 is 0x3F00_0000..0x3F3F_0000 and the IRAM0 cache part
/// 0x4008_0000..0x4040_0000 (`ext_mem_defs.h`).
fn window_of(vaddr: u32) -> Option<(u32, u32)> {
    if (ESP32S2_DROM0_BASE..0x3F3F_0000).contains(&vaddr) {
        Some((ESP32S2_DROM0_BASE, ESP32S2_DROM0_ENTRY))
    } else if (ESP32S2_IROM0_BASE..0x4040_0000).contains(&vaddr) {
        Some((ESP32S2_IROM0_BASE, ESP32S2_IROM0_ENTRY))
    } else {
        None
    }
}

/// Load `elf_bytes`, lay out its flash segments and MMU entries, and set the
/// CPU to the post-bootloader state at the ELF entry.
pub fn fast_boot(
    elf_bytes: &[u8],
    bus: &mut SystemBus,
    cpu: &mut XtensaLx7,
    flash: &Esp32s2Flash,
    partition_table: Option<&[u8]>,
) -> BootResult<BootSummary> {
    let elf = Elf::parse(elf_bytes).map_err(|e| BootError::ElfParse(format!("{e}")))?;

    // Virtual pages each window uses, in address order: DROM0 first, then
    // IROM0, the order esptool writes the segments and the bootloader maps them.
    let mut pages: Vec<(u32, u32)> = Vec::new(); // (window base, page vaddr)
    for ph in &elf.program_headers {
        if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let vaddr = ph.p_vaddr as u32;
        if let Some((base, _)) = window_of(vaddr) {
            let mut page = vaddr & !(PAGE - 1);
            let end = vaddr + ph.p_filesz as u32;
            while page < end {
                if !pages.contains(&(base, page)) {
                    pages.push((base, page));
                }
                page += PAGE;
            }
        }
    }
    pages.sort_by_key(|&(base, page)| (base != ESP32S2_DROM0_BASE, page));

    let mut backing = flash.backing.lock().unwrap();
    let mut mmu = flash.mmu.entries.lock().unwrap();
    let mut phys_of = std::collections::HashMap::new();
    for (i, &(base, page)) in pages.iter().enumerate() {
        let phys = APP_FLASH_OFFSET + i as u32 * PAGE;
        if (phys + PAGE) as usize > backing.len() {
            return Err(BootError::SegmentOutsideMap {
                addr: page,
                size: PAGE as usize,
            });
        }
        let (_, slice) = window_of(base).expect("window base");
        let entry = (slice + ((page & 0x3F_FFFF) >> 16)) as usize;
        if entry >= mmu.len() {
            return Err(BootError::TooManyXipPages {
                requested: pages.len(),
            });
        }
        mmu[entry] = (phys >> 16) | ESP32S2_MMU_ACCESS_FLASH;
        phys_of.insert(page, phys);
    }
    drop(mmu);
    flash
        .mmu
        .generation
        .fetch_add(1, std::sync::atomic::Ordering::Release);
    if let Some(table) = partition_table {
        let n = table.len().min(0xC00);
        backing[PARTITION_TABLE_OFFSET..PARTITION_TABLE_OFFSET + n].copy_from_slice(&table[..n]);
    }

    let mut segments_loaded = 0;
    for ph in &elf.program_headers {
        if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let vaddr = ph.p_vaddr as u32;
        let off = ph.p_offset as usize;
        let size = ph.p_filesz as usize;
        let bytes = elf_bytes
            .get(off..off.saturating_add(size))
            .ok_or_else(|| BootError::ElfParse(format!("segment @ {vaddr:#010x} beyond file")))?;
        if window_of(vaddr).is_some() {
            for (i, &b) in bytes.iter().enumerate() {
                let a = vaddr + i as u32;
                let phys = phys_of[&(a & !(PAGE - 1))] + (a & (PAGE - 1));
                backing[phys as usize] = b;
            }
        } else {
            // Host placement, not a CPU store (same contract as the S3 fast boot).
            bus.with_debug_watch_bypass(|bus| {
                bus.with_memory_policy_bypass(|bus| {
                    for (i, &b) in bytes.iter().enumerate() {
                        let addr = vaddr.wrapping_add(i as u32);
                        bus.write_u8(addr as u64, b)
                            .map_err(|_| BootError::SegmentOutsideMap {
                                addr,
                                size: size - i,
                            })?;
                    }
                    Ok::<(), BootError>(())
                })
            })?;
        }
        segments_loaded += 1;
    }
    // `system_early_init` reads the app image header (`esp_image_header_t`,
    // magic 0xE9) and the first segment header through the cache, just below
    // `_rodata_reserved_start`, i.e. at the start of the first DROM page, where
    // esptool writes them. Stamp them there, only into an erased slot.
    if let Some((first_vaddr, first_len)) = elf
        .program_headers
        .iter()
        .filter(|ph| ph.p_type == PT_LOAD && ph.p_filesz > 0)
        .map(|ph| (ph.p_vaddr as u32, ph.p_filesz as u32))
        .filter(|&(v, _)| window_of(v).map(|w| w.0) == Some(ESP32S2_DROM0_BASE))
        .min_by_key(|&(v, _)| v)
    {
        let page_phys = phys_of[&(first_vaddr & !(PAGE - 1))] as usize;
        let slot = page_phys..page_phys + 0x20;
        if first_vaddr & (PAGE - 1) == 0x20 && backing[slot.clone()].iter().all(|&b| b == 0xFF) {
            let segment_count = elf
                .program_headers
                .iter()
                .filter(|ph| ph.p_type == PT_LOAD && ph.p_filesz > 0)
                .count() as u8;
            let size_code = match backing.len() {
                0..=0x10_0000 => 0u8, // 1 MB
                0x10_0001..=0x20_0000 => 1,
                0x20_0001..=0x40_0000 => 2,
                0x40_0001..=0x80_0000 => 3,
                _ => 4, // 16 MB
            };
            let mut hdr = [0u8; 0x20];
            hdr[0] = 0xE9; // ESP_IMAGE_HEADER_MAGIC
            hdr[1] = segment_count;
            hdr[2] = 2; // ESP_IMAGE_SPI_MODE_DIO
            hdr[3] = (size_code << 4) | 0xF; // spi_size | ESP_IMAGE_SPI_SPEED_DIV_1 (80 MHz)
            hdr[4..8].copy_from_slice(&(elf.entry as u32).to_le_bytes());
            hdr[8] = 0xEE; // wp_pin: not set via efuse
            hdr[12..14].copy_from_slice(&2u16.to_le_bytes()); // ESP_CHIP_ID_ESP32S2
            hdr[17..19].copy_from_slice(&0xFFFFu16.to_le_bytes()); // max_chip_rev_full
            hdr[24..28].copy_from_slice(&first_vaddr.to_le_bytes()); // segment load_addr
            hdr[28..32].copy_from_slice(&first_len.to_le_bytes()); // segment data_len
            backing[slot].copy_from_slice(&hdr);
        }
    }
    drop(backing);

    let stack = elf
        .syms
        .iter()
        .find(|sym| elf.strtab.get_at(sym.st_name) == Some("_stack_start_cpu0"))
        .map(|sym| sym.st_value as u32)
        // Top of the S2's internal SRAM data view, below the ROM's own area.
        .unwrap_or(0x3FFF_C000);
    let entry = elf.entry as u32;
    // Enter `call_start_cpu0` as a CALL4 from an outermost frame, the shape
    // the boot ROM's own first frame has: window 0 is that caller (a0 = 0
    // ends every backtrace, a1 = the stack), PS.CALLINC = 1 so the app's
    // ENTRY rotates to window 1. The first full spill (FreeRTOS start) then
    // saves the caller as a 4-register frame, which never reads a caller's
    // caller's stack pointer: a jump with CALLINC = 0 instead makes the app
    // the outermost CALL8 frame, and spilling it dereferences `sp - 12`.
    cpu.regs.set_windowbase(0);
    cpu.regs.set_windowstart(1);
    cpu.regs.write_logical(0, 0);
    cpu.regs.write_logical(4, 0);
    cpu.set_pc(entry);
    cpu.set_sp(stack);
    // The bootloader hands over with PS = { WOE=1, EXCM=0, INTLEVEL=0 }.
    let mut ps = crate::cpu::xtensa_regs::Ps::from_raw(1 << 18);
    ps.set_callinc(1);
    cpu.ps = ps;
    Ok(BootSummary {
        entry,
        stack,
        segments_loaded,
    })
}
