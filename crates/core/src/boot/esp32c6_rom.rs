// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C6 Arduino-ELF flash / NVS substrate for the fast-boot path.
//!
//! Why this module exists
//! ======================
//! The C6 L1 smoke path treats the unified external-memory window at
//! `0x4200_0000` as plain memory — there is no SPI flash controller behind it,
//! so a bare ELF loads straight at its link addresses (see
//! `docs/boards/esp32c6-devkitc.md`). That is enough for gpio-net demos. It is
//! **not** enough for an Arduino-ESP32 sketch that calls `Preferences`: those
//! APIs go through ESP-IDF's `nvs_flash_init` → partition table @ flash offset
//! `0x8000` → SPI NOR reads/writes issued on the MSPI SPI_MEM controllers
//! (SPI0 @ `0x6000_2000`, SPI1 @ `0x6000_3000`, matching `esp32c6.svd`).
//!
//! Without those controllers and a seeded table, `nvs_flash_init` returns
//! `ESP_ERR_NOT_FOUND` (0x105) and every Preferences begin/get/put/remove fails
//! on the twin while working on silicon — the one thing this product must never
//! do. This module is the one home for that recipe so the RISC-V builder and a
//! world node cannot diverge.
//!
//! What this is not
//! ================
//! Full mask-ROM flash-image boot (bootloader + partitions + app through the
//! BROM) stays out of scope at L1 — `KNOWN_LIMITATIONS.md` already says so.
//! C3's MMU/XIP/DROM path is **not** copied here: on the C6, INTPRI already
//! owns `0x600C_5000` (the C3 MMU table base), and IROM/DROM share one window.
//! Reuses [`crate::peripherals::esp32s3::spi_mem_flash::SpiMemFlash`] and
//! [`super::esp_partition_table`] rather than forking either.

use std::sync::{Arc, Mutex};

use labwired_config::ChipDescriptor;

use crate::bus::SystemBus;
use crate::peripherals::esp32s3::flash_xip::{
    new_mmu_table_c6, FlashXipPeripheral, SharedMmuTable, MMU_FMT_C6,
};
use crate::peripherals::esp32s3::spi_mem_flash::SpiMemFlash;

use super::esp_partition_table::{
    default_partition_table, PartitionSubtype, PartitionType, DEFAULT_ARDUINO_4MB,
    MAX_PARTITION_TABLE_LEN, MODELLED_FLASH_SIZE, PARTITION_TABLE_OFFSET,
};

/// Unified external-memory / XIP window (IROM == DROM on C6).
pub const XIP_BASE: u64 = 0x4200_0000;
/// C6 MMU covers 256 × 64 KiB = 16 MiB of the 0x4200_0000 window.
pub const XIP_WINDOW: u64 = 0x100_0000;
/// Default factory app partition offset (Arduino 4 MiB table).
pub const FACTORY_APP_OFFSET: u32 = 0x1_0000;
/// SPI_MEM MMU_ITEM_CONTENT / INDEX offsets (from SPI0/1 base).
pub const MMU_ITEM_CONTENT_OFF: u64 = 0x37C;
pub const MMU_ITEM_INDEX_OFF: u64 = 0x380;
/// `SOC_MMU_VALID` — BIT(9).
pub const MMU_VALID: u32 = 1 << 9;

/// SPI0 (MSPI) — `esp32c6.svd` `SPI0` @ `0x6000_2000`.
pub const SPI0_BASE: u64 = 0x6000_2000;
/// SPI1 (MSPI) — `esp32c6.svd` `SPI1` @ `0x6000_3000`.
pub const SPI1_BASE: u64 = 0x6000_3000;
/// Full MSPI SPI_MEM register block (SVD ~1KB). Was 0x100; yaml `spi0`/`spi1`
/// stubs are also 1KB — name-only replace left those stubs in place and a
/// too-narrow layered window left high offsets on the stub.
const SPI_MEM_WINDOW: u64 = 0x1000;

/// End of IDF `sram_seg` (exclusive). App stack must stay below this; HP SRAM
/// from here to `0x4088_0000` is reserved for mask-ROM `.data` / `.bss` /
/// interface pointers (`memory.ld`: `len = 0x4086E610 - 0x40800000`).
pub const APP_SRAM_END: u32 = 0x4086_E610;

/// Fast-boot SP for C6 Arduino ELFs — 16-byte aligned top of app SRAM.
/// Using the full 512 KiB RAM top (`0x4088_0000`) smashes `rom_cache_internal_table_ptr`
/// and friends at `0x4087_ffc8`.
pub fn app_stack_top() -> u32 {
    APP_SRAM_END & !0xF
}

/// True when `chip` is an ESP32-C6 (name match, same style as `is_esp32c3`).
pub fn is_esp32c6(chip: &ChipDescriptor) -> bool {
    chip.name.to_ascii_lowercase().contains("esp32c6")
}

/// Seed `partitions` (or the default Arduino 4 MiB table) at flash offset
/// [`PARTITION_TABLE_OFFSET`] inside a shared flash backing buffer.
///
/// The buffer is erased (`0xFF`) first — matching NOR — then the table is
/// written. A caller-supplied `partitions.bin` always wins over the default
/// (same precedence as classic ESP32 / C3).
pub fn seed_flash_partitions(flash: &mut [u8], partitions: Option<&[u8]>) {
    let default_pt;
    let pt: &[u8] = match partitions {
        Some(pt) => pt,
        None => {
            default_pt = default_partition_table();
            &default_pt
        }
    };
    let off = PARTITION_TABLE_OFFSET as usize;
    let n = pt
        .len()
        .min(MAX_PARTITION_TABLE_LEN)
        .min(flash.len().saturating_sub(off));
    if n == 0 {
        return;
    }
    flash[off..off + n].copy_from_slice(&pt[..n]);
}

/// Install MSPI SPI_MEM twins + a partition-seeded flash backing + shared MMU
/// on a C6 bus.
///
/// Returns `(flash_backing, mmu_table)`. Idempotent on the peripheral names: a
/// second call replaces the controllers and attaches a fresh backing/MMU.
///
/// Does **not** install the FlashXip window yet — ELF load still writes the
/// bare `0x4200_0000` LinearMemory. Call [`install_xip_window`] after the
/// factory image has been copied into the backing and the MMU programmed
/// (bootloader-equivalent), so instruction fetch and `spi_flash_mmap` both
/// translate through the real table.
pub fn install_arduino_flash(
    bus: &mut SystemBus,
    partitions: Option<&[u8]>,
) -> (Arc<Mutex<Vec<u8>>>, SharedMmuTable) {
    let mut img = vec![0xFFu8; MODELLED_FLASH_SIZE as usize];
    seed_flash_partitions(&mut img, partitions);
    let flash = Arc::new(Mutex::new(img));
    let mmu = new_mmu_table_c6();

    // Match by base so yaml `spi0`/`spi1` stubs at these addresses are
    // replaced (name-only replace_or_add left the stubs and layered a twin;
    // IDF partition reads then hit the stub → 0xFF → "No MD5 found").
    // SPI0 owns the MMU_ITEM_* window (`mmu_ll_*` uses SPI_MEM_*_REG(0)).
    bus.replace_or_add_peripheral_at(
        "spimem0_flash",
        SPI0_BASE,
        SPI_MEM_WINDOW,
        None,
        Box::new(SpiMemFlash::new(flash.clone()).with_mmu(mmu.clone())),
    );
    bus.replace_or_add_peripheral_at(
        "spimem1_flash",
        SPI1_BASE,
        SPI_MEM_WINDOW,
        None,
        Box::new(SpiMemFlash::new(flash.clone()).with_mmu(mmu.clone())),
    );
    // SPI_MEM twins must not be elided by the optimized-access fast path: the
    // C3 Arduino install flips the same flag for the same reason.
    bus.config.optimized_bus_access = false;
    bus.refresh_peripheral_index();
    (flash, mmu)
}

/// Install the C6 FlashXip window at `0x4200_0000` over `flash`/`mmu`.
///
/// Call **after** ELF load + factory MMU programming. With
/// `optimized_bus_access = false`, this peripheral wins over the LinearMemory
/// flash image so fetches and mmap reads translate through the MMU exactly as
/// on silicon. Unused MMU entries stay invalid so `spi_flash_mmap` can claim
/// free pages for the partition table at flash `0x8000`.
pub fn install_xip_window(bus: &mut SystemBus, flash: Arc<Mutex<Vec<u8>>>, mmu: SharedMmuTable) {
    bus.replace_or_add_peripheral_at(
        "flash_irom_xip",
        XIP_BASE,
        XIP_WINDOW,
        None,
        Box::new(FlashXipPeripheral::new_mmu_fmt(
            flash,
            XIP_BASE as u32,
            mmu,
            MMU_FMT_C6,
        )),
    );
    bus.config.optimized_bus_access = false;
    bus.refresh_peripheral_index();
}

/// Program one C6 MMU entry through SPI0's MMU_ITEM_INDEX/CONTENT (silicon path).
pub fn write_mmu_entry(bus: &mut SystemBus, entry_id: u32, phys_page: u32) {
    use crate::Bus;
    let _ = bus.write_u32(SPI0_BASE + MMU_ITEM_INDEX_OFF, entry_id);
    let _ = bus.write_u32(
        SPI0_BASE + MMU_ITEM_CONTENT_OFF,
        (phys_page & MMU_FMT_C6.valid_val_mask) | MMU_VALID,
    );
}

/// C6 mask-ROM `.data` the fast-boot path must seed because it skips the ROM
/// `unpackloop` (12-byte `{dst_start,dst_end,src}` records at `_data_table_start`).
/// Without this, `rom_cache_internal_table_ptr` @ `0x4087_ffc8` stays null and
/// Arduino `cache_hal_init` dispatches ROM `Cache_*` helpers through a null
/// table → panic_abort. Values taken from Espressif `esp32c6_rev0_rom.elf`
/// PROGBITS (interface pointers + cache/spi_flash/.data_ets).
fn c6_rom_data_init_writes() -> &'static [(u32, &'static [u8])] {
    &[
        // rom_cache_internal_table_ptr → rom_default_cache_internal_table
        (0x4087_ffc8, &[0x10, 0xdb, 0x04, 0x40]),
        // heap_tlsf_table_ptr
        (0x4087_ffd8, &[0xd0, 0xd0, 0x04, 0x40]),
        // spi_flash chip table ptrs
        (
            0x4087_ffdc,
            &[0x80, 0xd9, 0x04, 0x40, 0x40, 0xd9, 0x04, 0x40],
        ),
        // esp_flash ops ptr
        (0x4087_ffe4, &[0xcc, 0xd8, 0x04, 0x40]),
        // spiflash_legacy ptrs (DRAM targets — OK if unused early)
        (
            0x4087_ffec,
            &[0x08, 0xfa, 0x87, 0x40, 0x24, 0xfa, 0x87, 0x40],
        ),
        // ets_ops_table_ptr
        (0x4087_fff8, &[0x80, 0xa6, 0x04, 0x40]),
        // .data_ets / .data_uart flags
        (0x4087_e610, &[0x01, 0x00, 0x00, 0x00]),
        (0x4087_f564, &[0x01, 0x00, 0x00, 0x00]),
        // .data_cache
        (
            0x4087_fa84,
            &[0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00],
        ),
        // .data_ets_delay
        (0x4087_faa0, &[0x28, 0x00, 0x00, 0x00]),
        // .data_c: __global_locale_ptr → ROM locale object (newlib
        // __get_locale_string / assert paths load *ptr+0xF0 as a C string).
        (0x4087_faac, &[0xd0, 0x95, 0x04, 0x40]),
    ]
}

/// Write ROM `.data` interface pointers into HP SRAM (fast-boot only).
pub fn seed_rom_data(bus: &mut SystemBus) {
    use crate::Bus;
    for (dst, bytes) in c6_rom_data_init_writes() {
        for (i, b) in bytes.iter().enumerate() {
            let addr = *dst as u64 + i as u64;
            if let Err(e) = bus.write_u8(addr, *b) {
                tracing::debug!("seed_rom_data write fail @ {addr:#x}: {e:?}");
            }
        }
    }
}

/// Install the C6 Arduino flash substrate when `chip` is a C6; no-op otherwise.
///
/// One call site for the RISC-V builder and world-node ELF path.
pub fn maybe_install_arduino_flash(bus: &mut SystemBus, chip: &ChipDescriptor) {
    if is_esp32c6(chip) {
        let _ = install_arduino_flash(bus, None);
        // Fast-boot skips mask-ROM unpackloop — seed the ROM `.data` tables
        // Arduino cache_hal / Preferences dispatch through.
        seed_rom_data(bus);
        // FlashXip is installed later (after ELF → factory map) by the CLI.
    }
}

/// Flash offset of the NVS data partition in [`DEFAULT_ARDUINO_4MB`].
pub fn default_nvs_offset() -> u32 {
    DEFAULT_ARDUINO_4MB
        .iter()
        .find(|p| p.ptype == PartitionType::Data && p.subtype == PartitionSubtype::DataNvs)
        .map(|p| p.offset)
        .expect("DEFAULT_ARDUINO_4MB always declares nvs")
}

/// Flash size of the NVS data partition in [`DEFAULT_ARDUINO_4MB`].
pub fn default_nvs_size() -> u32 {
    DEFAULT_ARDUINO_4MB
        .iter()
        .find(|p| p.ptype == PartitionType::Data && p.subtype == PartitionSubtype::DataNvs)
        .map(|p| p.size)
        .expect("DEFAULT_ARDUINO_4MB always declares nvs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bus;
    use labwired_config::{ChipDescriptor, SystemManifest};

    fn repo_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn c6_bus() -> SystemBus {
        let chip = ChipDescriptor::from_file(repo_root().join("configs/chips/esp32c6.yaml"))
            .expect("load esp32c6.yaml");
        let manifest =
            SystemManifest::from_file(repo_root().join("configs/systems/esp32c6-devkitc.yaml"))
                .expect("load esp32c6-devkitc.yaml");
        SystemBus::from_config(&chip, &manifest).expect("build C6 bus")
    }

    /// Decode partition entries the way ESP-IDF walks the table — not by
    /// calling back into our encoder.
    fn decode_table(blob: &[u8]) -> Vec<(u8, u8, u32, u32, String)> {
        let mut out = Vec::new();
        for chunk in blob.chunks_exact(32) {
            let magic = u16::from_le_bytes([chunk[0], chunk[1]]);
            if magic != 0x50AA {
                break;
            }
            out.push((
                chunk[2],
                chunk[3],
                u32::from_le_bytes(chunk[4..8].try_into().unwrap()),
                u32::from_le_bytes(chunk[8..12].try_into().unwrap()),
                String::from_utf8_lossy(&chunk[12..28])
                    .trim_end_matches('\0')
                    .to_string(),
            ));
        }
        out
    }

    #[test]
    fn app_stack_top_below_rom_reserve() {
        let sp = app_stack_top();
        assert_eq!(sp, 0x4086_E610);
        assert!(
            sp < 0x4087_ffc8,
            "must not smash rom_cache_internal_table_ptr"
        );
        assert_eq!(sp & 0xF, 0, "16-byte aligned");
    }

    #[test]
    fn seed_rom_data_writes_heap_tlsf_ptr() {
        let mut bus = c6_bus();
        seed_rom_data(&mut bus);
        let mut b = [0u8; 4];
        for (i, byte) in b.iter_mut().enumerate() {
            *byte = bus.read_u8(0x4087_ffd8 + i as u64).expect("read");
        }
        assert_eq!(u32::from_le_bytes(b), 0x4004_d0d0, "heap_tlsf_table_ptr");
        // And the table entry ROM serves at +52
        let mut e = [0u8; 4];
        for (i, byte) in e.iter_mut().enumerate() {
            *byte = bus.read_u8(0x4004_d0d0 + 52 + i as u64).expect("rom");
        }
        // The ROM image is not committed: without LABWIRED_ESP32C6_ROM the
        // region stays zero; with Espressif's esp32c6_rev0_rom image the tlsf
        // table entry at +0x34 is the ROM function 0x400210b2.
        let v = u32::from_le_bytes(e);
        let expected = if std::env::var_os("LABWIRED_ESP32C6_ROM").is_some() {
            0x4002_10b2
        } else {
            0
        };
        assert_eq!(v, expected, "tlsf table entry @ ROM+0x34");
    }

    #[test]
    fn is_esp32c6_matches_chip_name() {
        let mut chip = ChipDescriptor::from_file(repo_root().join("configs/chips/esp32c6.yaml"))
            .expect("load esp32c6.yaml");
        assert!(is_esp32c6(&chip));
        chip.name = "esp32c3".into();
        assert!(!is_esp32c6(&chip));
    }

    #[test]
    fn install_seeds_nvs_partition_at_0x8000() {
        let mut bus = c6_bus();
        let (flash, _mmu) = install_arduino_flash(&mut bus, None);
        let img = flash.lock().expect("flash lock");
        let off = PARTITION_TABLE_OFFSET as usize;
        let parts = decode_table(&img[off..off + MAX_PARTITION_TABLE_LEN]);
        let nvs = parts
            .iter()
            .find(|p| p.0 == 1 && p.1 == 2)
            .expect("no NVS partition — Preferences begin would fail with ESP_ERR_NOT_FOUND");
        assert_eq!(nvs.4, "nvs");
        assert!(nvs.3 > 0, "NVS partition has zero size");
        let core = parts
            .iter()
            .find(|p| p.0 == 1 && p.1 == 3)
            .expect("no coredump partition");
        assert_eq!(core.4, "coredump");
    }

    #[test]
    fn spimem_controllers_land_at_svd_bases() {
        let mut bus = c6_bus();
        let _ = install_arduino_flash(&mut bus, None);
        assert!(
            bus.find_peripheral_index_by_name("spimem0_flash").is_some(),
            "SPI0 MSPI twin missing"
        );
        assert!(
            bus.find_peripheral_index_by_name("spimem1_flash").is_some(),
            "SPI1 MSPI twin missing"
        );
        // Probe the SPI0 CMD register through the bus — must not fault.
        let _ = bus.read_u32(SPI0_BASE).expect("SPI0 CMD readable");
        let _ = bus.read_u32(SPI1_BASE).expect("SPI1 CMD readable");
    }

    /// Preferences put/get needs the NVS region to accept a write and return
    /// the same bytes on read. Drive that through the SPI_MEM twin the way
    /// ESP-IDF's flash driver does (dedicated FLASH_PP + USR READ), not by
    /// poking the Arc behind the model's back for the write half.
    #[test]
    fn nvs_region_round_trips_through_spimem() {
        let mut bus = c6_bus();
        let (flash, _mmu) = install_arduino_flash(&mut bus, None);
        let nvs_off = default_nvs_offset() as u64;
        assert!(
            nvs_off + 16 <= default_nvs_offset() as u64 + default_nvs_size() as u64,
            "test blob must sit inside the NVS partition"
        );

        // Erase the page's worth first isn't required for a fresh 0xFF backing,
        // but FLASH_WREN + FLASH_PP is the silicon sequence.
        const USR_BIT: u32 = 1 << 18;
        const FLASH_WREN: u32 = 1 << 30;
        const FLASH_PP: u32 = 1 << 25;
        const ADDR: u64 = 0x04;
        const USER2: u64 = 0x20;
        const MOSI_DLEN: u64 = 0x24;
        const MISO_DLEN: u64 = 0x28;
        const W0: u64 = 0x58;
        const CMD: u64 = 0x00;

        let payload: u32 = 0xA1B2_C3D4;
        bus.write_u32(SPI0_BASE + CMD, FLASH_WREN).expect("WREN");
        bus.write_u32(SPI0_BASE + ADDR, nvs_off as u32)
            .expect("ADDR");
        bus.write_u32(SPI0_BASE + W0, payload).expect("W0");
        bus.write_u32(SPI0_BASE + MOSI_DLEN, 32 - 1)
            .expect("MOSI_DLEN");
        bus.write_u32(SPI0_BASE + CMD, FLASH_PP).expect("PP");
        assert_eq!(
            bus.read_u32(SPI0_BASE + CMD).expect("CMD after PP"),
            0,
            "FLASH_PP must auto-clear"
        );

        // Host-side confirm the backing took the write (Preferences' storage).
        {
            let img = flash.lock().expect("flash lock");
            let off = nvs_off as usize;
            let got = u32::from_le_bytes(img[off..off + 4].try_into().unwrap());
            assert_eq!(got, payload, "NVS backing did not keep the programmed word");
        }

        // Read back through SPI USR READ (0x03) — the path load_partitions /
        // nvs reads use after mmap fallbacks.
        bus.write_u32(SPI0_BASE + USER2, 0x03).expect("USER2 READ");
        bus.write_u32(SPI0_BASE + ADDR, nvs_off as u32)
            .expect("ADDR");
        bus.write_u32(SPI0_BASE + MISO_DLEN, 32 - 1)
            .expect("MISO_DLEN");
        bus.write_u32(SPI0_BASE + CMD, USR_BIT).expect("USR");
        assert_eq!(bus.read_u32(SPI0_BASE + CMD).expect("CMD after READ"), 0);
        assert_eq!(
            bus.read_u32(SPI0_BASE + W0).expect("W0"),
            payload,
            "SPI READ of NVS offset must return the programmed word"
        );
    }

    #[test]
    fn maybe_install_is_noop_for_non_c6() {
        let chip = ChipDescriptor::from_file(repo_root().join("configs/chips/esp32c3.yaml"))
            .expect("load esp32c3.yaml");
        let manifest =
            SystemManifest::from_file(repo_root().join("configs/systems/esp32c3-devkit.yaml"))
                .expect("load esp32c3 system");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("C3 bus");
        maybe_install_arduino_flash(&mut bus, &chip);
        assert!(
            bus.find_peripheral_index_by_name("spimem0_flash").is_none(),
            "C6 install must not fire on a C3 bus"
        );
    }

    #[test]
    fn mmu_item_and_xip_translate_factory_page() {
        use crate::Bus;
        let mut bus = c6_bus();
        let (flash, mmu) = install_arduino_flash(&mut bus, None);
        // Put a marker at factory page 1 (flash 0x10000)
        {
            let mut f = flash.lock().unwrap();
            f[0x10000] = 0xE9;
            f[0x10001] = 0xA5;
        }
        // Map virt page 0 → phys page 1 (factory)
        write_mmu_entry(&mut bus, 0, 1);
        assert_eq!(
            mmu.entries.lock().unwrap()[0],
            MMU_VALID | 1,
            "SPI0 MMU_ITEM_CONTENT must program SharedMmu"
        );
        install_xip_window(&mut bus, flash.clone(), mmu.clone());
        assert_eq!(bus.read_u8(XIP_BASE).unwrap(), 0xE9);
        assert_eq!(bus.read_u8(XIP_BASE + 1).unwrap(), 0xA5);
        // Unmapped virt page 2: FlashXip returns 0 (not Err). Seed the
        // identity flash offset so a LinearMemory fallback would show 0x5A.
        {
            let mut f = flash.lock().unwrap();
            f[0x2_0000] = 0x5A;
        }
        assert_eq!(
            bus.read_u8(XIP_BASE + 0x2_0000).unwrap(),
            0,
            "unmapped MMU page must read 0 via FlashXip, not LinearMemory identity"
        );
    }
}
