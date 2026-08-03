// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! THE ESP-IDF partition table the twin presents at flash `0x8000`.
//!
//! Why this exists
//! ===============
//! On silicon, `esptool` writes a partition table to flash `0x8000` alongside
//! the app image. The sim skips flashing entirely — it loads an ELF — so unless
//! something puts a table there, flash `0x8000` reads as erased (`0xFF`) and
//! every ESP-IDF partition lookup comes back empty. What the user then sees on
//! a stock Arduino-ESP32 sketch is:
//!
//! ```text
//! E (0) esp_core_dump_flash: No core dump partition found!
//! [    88][E][esp32-hal-misc.c:264] initArduino(): Failed to initialize NVS! Error: 261
//! ```
//!
//! `261` is `0x105` = `ESP_ERR_NOT_FOUND`: `nvs_flash_init()` could not find an
//! NVS partition. That is not log noise. `Preferences` — WiFi credentials,
//! calibration constants, boot counters — is one of the most-used Arduino-ESP32
//! APIs, and it is built directly on NVS. A sketch calling `Preferences.begin()`
//! would fail on the twin and work on real silicon, which is the one thing this
//! product must never do.
//!
//! One home
//! ========
//! The CLI already knew how to overlay a REAL `partitions.bin` when the
//! PlatformIO build left one next to the ELF
//! (`cli::commands::esp32_boot_state::resolve_esp_partitions_bin`). The browser
//! has no filesystem, so it never could — and neither could any test that boots
//! an ELF straight out of `configure_xtensa_esp32`. This module is the fallback
//! those paths were missing: a table generated in-process, byte-compatible with
//! `gen_esp32part.py`, used whenever a firmware-specific one is not available.
//!
//! Precedence is deliberate and one-way: a real `partitions.bin` shipped with
//! the firmware always wins, because it describes THAT image's flash layout.
//! The generated default only fills the hole where there was nothing at all.
//!
//! Format (`esp_partition_info_t`, `components/esp_partition/esp_partition.c`)
//! =========================================================================
//! 32 bytes per entry, little-endian:
//!
//! ```text
//!   0..2   magic  = 0x50AA
//!   2      type   (0 = app, 1 = data)
//!   3      subtype
//!   4..8   offset in flash
//!   8..12  size
//!   12..28 label, NUL-padded
//!   28..32 flags
//! ```
//!
//! followed by an MD5 entry: magic `0xEBEB`, 14 bytes of `0xFF`, then the MD5
//! of every preceding entry. The whole thing is padded to
//! [`MAX_PARTITION_TABLE_LEN`] with `0xFF`.
//!
//! **The MD5 is not optional.** `CONFIG_PARTITION_TABLE_MD5` is on for
//! Arduino-ESP32, so `esp_partition_table_verify` recomputes the digest through
//! the ROM MD5 routines the sim models at `0x4005_da7c` / `_da9c` / `_db1c` (see
//! `system::xtensa::esp32`). A table with a wrong or missing digest does not get
//! ignored — `load_partitions` fails with `ESP_ERR_INVALID_STATE` and the boot
//! is worse off than with no table at all.

use crate::peripherals::esp_xtensa_common::rom_thunks::md5_digest;
use crate::Bus;

/// Bytes an ESP-IDF partition table occupies in flash (`ESP_PARTITION_TABLE_MAX_LEN`).
pub const MAX_PARTITION_TABLE_LEN: usize = 0xC00;

/// Flash offset the bootloader reads the table from (`CONFIG_PARTITION_TABLE_OFFSET`).
pub const PARTITION_TABLE_OFFSET: u32 = 0x8000;

/// `ESP_PARTITION_MAGIC`.
const PARTITION_MAGIC: u16 = 0x50AA;
/// `ESP_PARTITION_MAGIC_MD5`.
const PARTITION_MAGIC_MD5: u16 = 0xEBEB;

/// `esp_partition_type_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PartitionType {
    App = 0x00,
    Data = 0x01,
}

/// `esp_partition_subtype_t` — only the subtypes this table uses.
///
/// Values are from `esp_partition.h`; they are ABI, not ours to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PartitionSubtype {
    /// `ESP_PARTITION_SUBTYPE_APP_OTA_0`
    AppOta0 = 0x10,
    /// `ESP_PARTITION_SUBTYPE_APP_OTA_1`
    AppOta1 = 0x11,
    /// `ESP_PARTITION_SUBTYPE_DATA_OTA` (the `otadata` selector partition)
    DataOta = 0x00,
    /// `ESP_PARTITION_SUBTYPE_DATA_NVS`
    DataNvs = 0x02,
    /// `ESP_PARTITION_SUBTYPE_DATA_COREDUMP`
    DataCoredump = 0x03,
    /// `ESP_PARTITION_SUBTYPE_DATA_SPIFFS`
    DataSpiffs = 0x82,
}

/// One row of the table, in the same shape as a `partitions.csv` line.
#[derive(Debug, Clone, Copy)]
pub struct PartitionEntry {
    pub label: &'static str,
    pub ptype: PartitionType,
    pub subtype: PartitionSubtype,
    pub offset: u32,
    pub size: u32,
}

/// Arduino-ESP32's stock 4 MiB layout (`tools/partitions/default.csv`).
///
/// Kept as data rather than a generated blob so the entries are readable and a
/// reviewer can check them against the CSV. `nvs` and `coredump` are the two
/// that matter for a clean `initArduino()`; the rest are here because a table
/// that omits `app0` would make `esp_ota_get_running_partition` and
/// `esp_partition_find_first(APP, ...)` lie in a different direction.
pub const DEFAULT_ARDUINO_4MB: &[PartitionEntry] = &[
    PartitionEntry {
        label: "nvs",
        ptype: PartitionType::Data,
        subtype: PartitionSubtype::DataNvs,
        offset: 0x9000,
        size: 0x5000,
    },
    PartitionEntry {
        label: "otadata",
        ptype: PartitionType::Data,
        subtype: PartitionSubtype::DataOta,
        offset: 0xE000,
        size: 0x2000,
    },
    PartitionEntry {
        label: "app0",
        ptype: PartitionType::App,
        subtype: PartitionSubtype::AppOta0,
        offset: 0x1_0000,
        size: 0x14_0000,
    },
    PartitionEntry {
        label: "app1",
        ptype: PartitionType::App,
        subtype: PartitionSubtype::AppOta1,
        offset: 0x15_0000,
        size: 0x14_0000,
    },
    PartitionEntry {
        label: "spiffs",
        ptype: PartitionType::Data,
        subtype: PartitionSubtype::DataSpiffs,
        offset: 0x29_0000,
        size: 0x16_0000,
    },
    PartitionEntry {
        label: "coredump",
        ptype: PartitionType::Data,
        subtype: PartitionSubtype::DataCoredump,
        offset: 0x3F_0000,
        size: 0x1_0000,
    },
];

/// Serialise `entries` into the exact bytes `esptool` would write at `0x8000`.
///
/// Always emits the MD5 entry — see the module note on why a table without one
/// is not a lesser table but a broken one on Arduino-ESP32.
pub fn build_partition_table(entries: &[PartitionEntry]) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::with_capacity(entries.len() * 32);
    for e in entries {
        let label = e.label.as_bytes();
        assert!(
            label.len() <= 16,
            "partition label {:?} exceeds the 16-byte field",
            e.label
        );
        body.extend_from_slice(&PARTITION_MAGIC.to_le_bytes());
        body.push(e.ptype as u8);
        body.push(e.subtype as u8);
        body.extend_from_slice(&e.offset.to_le_bytes());
        body.extend_from_slice(&e.size.to_le_bytes());
        let mut lbl = [0u8; 16];
        lbl[..label.len()].copy_from_slice(label);
        body.extend_from_slice(&lbl);
        body.extend_from_slice(&0u32.to_le_bytes()); // flags
    }

    let digest = md5_digest(&body);
    let mut out = body;
    out.extend_from_slice(&PARTITION_MAGIC_MD5.to_le_bytes());
    out.extend_from_slice(&[0xFFu8; 14]);
    out.extend_from_slice(&digest);

    assert!(
        out.len() <= MAX_PARTITION_TABLE_LEN,
        "partition table is {} bytes, over the {MAX_PARTITION_TABLE_LEN}-byte sector budget",
        out.len()
    );
    out.resize(MAX_PARTITION_TABLE_LEN, 0xFF);
    out
}

/// The table used when the caller has no firmware-specific `partitions.bin`.
pub fn default_partition_table() -> Vec<u8> {
    build_partition_table(DEFAULT_ARDUINO_4MB)
}

/// Flash size the twin's classic-ESP32 model backs the bus with, and therefore
/// the size [`DEFAULT_ARDUINO_4MB`] is laid out for.
pub const MODELLED_FLASH_SIZE: u32 = 4 * 1024 * 1024;

/// Seed `g_rom_flashchip` at `addr` — the `esp_rom_spiflash_chip_t` the boot ROM
/// fills in when it attaches the SPI flash.
///
/// **A table at `0x8000` is useless without this.** `spi_flash_mmap` opens with
/// `if (src_addr + size > g_rom_flashchip.chip_size) return
/// ESP_ERR_INVALID_ARG;`, so a zeroed descriptor makes every mmap fail —
/// including the one `load_partitions()` uses to read the table. That is what
/// `E (0) partition: load_partitions returned 0x102` was on every classic-ESP32
/// boot: not a bad table, a flash chip the firmware believes is 0 bytes long.
///
/// This is boot state, not a thunk: it is a fact about where boot ended and you
/// can read it back out of memory. Values describe the Winbond W25Q32-class
/// 4 MiB part the SPI model answers `RDID` for, so the descriptor and the model
/// cannot disagree.
///
/// One home, because there were two: `install_arduino_esp32_profile` covers the
/// browser, `labwired snapshot capture` and `labwired test`'s arduino profile;
/// `cli::commands::esp32_boot_state::seed_esp32_post_brom_dram` covers the
/// CLI's non-profile ESP32 path. Both call this.
pub fn seed_rom_flashchip(bus: &mut dyn Bus, addr: u32) {
    let base = addr as u64;
    for (off, val) in [
        (0u64, 0x0016_40EFu32),  // device_id — Winbond W25Q32
        (4, MODELLED_FLASH_SIZE), // chip_size
        (8, 64 * 1024),           // block_size
        (12, 4 * 1024),           // sector_size
        (16, 256),                // page_size
        (20, 0xFFFF),             // status_mask
    ] {
        let _ = bus.write_u32(base + off, val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode the blob the way ESP-IDF's `esp_partition_load_table` does,
    /// from the struct layout in `esp_partition.h` — NOT by calling back into
    /// the encoder. A mirror of the writer would pass no matter what it wrote.
    fn decode(blob: &[u8]) -> Vec<(u8, u8, u32, u32, String)> {
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
    fn default_table_declares_nvs_and_coredump() {
        let blob = default_partition_table();
        let parts = decode(&blob);
        // ESP_PARTITION_TYPE_DATA = 1; SUBTYPE_DATA_NVS = 2, _COREDUMP = 3.
        let nvs = parts
            .iter()
            .find(|p| p.0 == 1 && p.1 == 2)
            .expect("no NVS partition — nvs_flash_init() returns ESP_ERR_NOT_FOUND (0x105)");
        assert_eq!(nvs.4, "nvs");
        assert!(nvs.3 > 0, "NVS partition has zero size");
        let core = parts
            .iter()
            .find(|p| p.0 == 1 && p.1 == 3)
            .expect("no coredump partition — esp_core_dump_flash logs an error every boot");
        assert_eq!(core.4, "coredump");
    }

    /// The MD5 trailer, checked the way `esp_partition_table_verify` checks it:
    /// magic `0xEBEB` at the entry start, digest at byte 16 of that entry, over
    /// exactly the bytes that precede it.
    #[test]
    fn md5_trailer_matches_preceding_entries() {
        let blob = default_partition_table();
        let n = DEFAULT_ARDUINO_4MB.len() * 32;
        let md5_entry = &blob[n..n + 32];
        assert_eq!(u16::from_le_bytes([md5_entry[0], md5_entry[1]]), 0xEBEB);
        assert_eq!(&md5_entry[2..16], &[0xFFu8; 14]);
        assert_eq!(&md5_entry[16..32], &md5_digest(&blob[..n])[..]);
    }

    /// Known-answer test for the digest itself, so the check above cannot pass
    /// by both sides being wrong the same way. Vectors are RFC 1321 A.5.
    #[test]
    fn md5_digest_matches_rfc1321_vectors() {
        assert_eq!(
            hex(&md5_digest(b"")),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
        assert_eq!(
            hex(&md5_digest(b"abc")),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            hex(&md5_digest(
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
            )),
            "d174ab98d277d9f5a5611c2c9f419d9f"
        );
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn table_fits_the_sector_budget() {
        assert_eq!(default_partition_table().len(), MAX_PARTITION_TABLE_LEN);
    }

    /// Partitions must not overlap, and must stay inside the 4 MiB the twin's
    /// flash model backs. A table that lies here would have NVS scribbling over
    /// the app image, which is exactly the class of bug the twin is for.
    #[test]
    fn default_table_is_non_overlapping_and_within_4mib() {
        let mut sorted: Vec<_> = DEFAULT_ARDUINO_4MB.to_vec();
        sorted.sort_by_key(|p| p.offset);
        let mut end = 0u32;
        for p in &sorted {
            assert!(
                p.offset >= end,
                "{} starts at 0x{:x}, inside the previous partition (ends 0x{end:x})",
                p.label,
                p.offset
            );
            end = p.offset + p.size;
        }
        assert!(end <= 4 * 1024 * 1024, "table runs past 4 MiB: 0x{end:x}");
    }
}
