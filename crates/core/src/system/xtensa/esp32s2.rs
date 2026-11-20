// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S2 external flash: one SPI NOR backing behind the two flash-cache
//! windows (through the S2 MMU) and the SPI1 flash-command controller.
//!
//! The chip descriptor carries every other window of the S2 memory map. Only
//! these three share state, which a descriptor cannot express, so they are
//! registered here, over whatever flash windows the descriptor declared.

use crate::bus::SystemBus;
use crate::peripherals::esp32s3::flash_xip::{
    new_mmu_table_s2, Esp32s3MmuTable, FlashXipPeripheral, SharedMmuTable, MMU_FMT_S2,
};
use crate::peripherals::esp32s3::spi_mem_flash::SpiMemFlash;
use std::sync::{Arc, Mutex};

/// `SOC_DROM0_ADDRESS_LOW`: the data-bus flash window (IBUS2 slice).
pub const ESP32S2_DROM0_BASE: u32 = 0x3F00_0000;
/// `SOC_IRAM0_CACHE_ADDRESS_LOW`: the instruction-bus flash window (IBUS0 slice).
pub const ESP32S2_IROM0_BASE: u32 = 0x4008_0000;
/// `DR_REG_MMU_TABLE`.
pub const ESP32S2_MMU_TABLE: u32 = 0x6180_1000;
/// `DR_REG_SPI1_BASE`: the flash-command controller (`spi_mem_reg.h`).
pub const ESP32S2_SPI1_BASE: u32 = 0x3F40_2000;
/// `PRO_CACHE_IBUS2_MMU_START / 4`: first MMU entry of the DROM0 window.
pub const ESP32S2_DROM0_ENTRY: u32 = 0x200 / 4;
/// `PRO_CACHE_IBUS0_MMU_START / 4`: first MMU entry of the IRAM0 cache window.
pub const ESP32S2_IROM0_ENTRY: u32 = 0;
/// `SOC_MMU_ACCESS_FLASH`: the target bit of a flash MMU entry.
pub const ESP32S2_MMU_ACCESS_FLASH: u32 = 1 << 15;

/// DROM0 spans 0x3F00_0000..0x3F3F_0000 and IRAM0's cache part
/// 0x4008_0000..0x4040_0000 (`ext_mem_defs.h`). Each slice has 64 entries.
const DROM0_SIZE: u64 = 0x3F_0000;
const IROM0_SIZE: u64 = 0x38_0000;

/// What [`attach_esp32s2_flash`] wired: the flash backing and the MMU table
/// the two cache windows translate through.
pub struct Esp32s2Flash {
    pub backing: Arc<Mutex<Vec<u8>>>,
    pub mmu: SharedMmuTable,
}

/// Register the S2 flash windows, MMU table and SPI1 over one erased flash
/// part of `flash_size` bytes. Any descriptor window that overlaps a cache
/// window (the chip YAML's `flash:` region) is removed first, so a read of
/// those addresses can only reach the MMU-translated view. The MMU table and
/// SPI1 are narrower than any register block around them, and the bus routes
/// an address to the narrowest window that holds it.
pub fn attach_esp32s2_flash(bus: &mut SystemBus, flash_size: usize) -> Esp32s2Flash {
    let windows = [
        (ESP32S2_DROM0_BASE as u64, DROM0_SIZE),
        (ESP32S2_IROM0_BASE as u64, IROM0_SIZE),
    ];
    bus.peripherals.retain(|p| {
        let end = p.base.saturating_add(p.size);
        !windows
            .iter()
            .any(|&(b, s)| p.base < b.saturating_add(s) && b < end)
    });

    // The descriptor's `flash:` region is the bus's primary linear flash,
    // which the bus reads before any peripheral window. Empty it so the
    // IRAM0 cache window below is the only thing at 0x4008_0000.
    bus.flash = crate::memory::LinearMemory::new(0, ESP32S2_IROM0_BASE as u64);
    let backing = Arc::new(Mutex::new(vec![0xFFu8; flash_size]));
    let mmu = new_mmu_table_s2();
    let drom = FlashXipPeripheral::new_mmu_fmt(
        backing.clone(),
        ESP32S2_DROM0_BASE,
        mmu.clone(),
        MMU_FMT_S2,
    )
    .with_entry_base(ESP32S2_DROM0_ENTRY);
    let irom = FlashXipPeripheral::new_mmu_fmt(
        backing.clone(),
        ESP32S2_IROM0_BASE,
        mmu.clone(),
        MMU_FMT_S2,
    )
    .with_entry_base(ESP32S2_IROM0_ENTRY);
    bus.add_peripheral(
        "flash_drom0",
        ESP32S2_DROM0_BASE as u64,
        DROM0_SIZE,
        None,
        Box::new(drom),
    );
    bus.add_peripheral(
        "flash_irom0",
        ESP32S2_IROM0_BASE as u64,
        IROM0_SIZE,
        None,
        Box::new(irom),
    );
    bus.add_peripheral(
        "mmu_table",
        ESP32S2_MMU_TABLE as u64,
        0x600,
        None,
        Box::new(Esp32s3MmuTable::new(mmu.clone())),
    );
    // SPI_MEM register offsets used by the command executor (CMD, ADDR,
    // USER1/2, MOSI/MISO_DLEN, RD_STATUS, MISC, W0..) are identical on the S2
    // and S3 (`soc/esp32s2/include/soc/spi_mem_reg.h`).
    bus.add_peripheral(
        "spi1",
        ESP32S2_SPI1_BASE as u64,
        0x100,
        None,
        Box::new(SpiMemFlash::new(backing.clone())),
    );
    bus.refresh_peripheral_index();
    Esp32s2Flash { backing, mmu }
}
