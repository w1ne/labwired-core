// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 boot-ROM provisioning for the faithful `--rom-boot` path.
//!
//! The two flat images the model loads (see `configs/chips/esp32c3.yaml`
//! memory regions `rom` / `rom_data`):
//!   * IROM (code + `.data` copy sources) 0x4000_0000..0x4006_0000 (384 KiB)
//!   * DROM (ROM constant data)           0x3FF0_0000..0x3FF2_0000 (128 KiB)
//!
//! Resolution order mirrors [`super::esp32s3_rom`]: explicit env pins
//! (`LABWIRED_ESP32C3_ROM` / `LABWIRED_ESP32C3_ROM_DATA`), then the ROM ELF
//! shipped with the user's installed toolchain (PlatformIO/ESP-IDF,
//! `esp32c3_rev3_rom.elf`), then the vendored images under
//! `crates/core/roms/esp32c3/` (embedded at build time, non-wasm only) so
//! `--rom-boot` works out of the box with no toolchain installed.
//!
//! Extraction reuses the S3 module's window builder: PT_LOAD placement, the
//! boot ROM's `.data` copy-source reconstruction against the C3 DRAM window
//! (the startup `unpackloop` at 0x40001ef8 walks 16-byte
//! {dst_start, dst_end, src, 0} records at 0x40059200), and a PROGBITS
//! overlay for sections in no PT_LOAD segment.

use super::esp32s3_rom::{build_window, DramWindow, RomImages};
use goblin::elf::Elf;
use std::path::PathBuf;

pub const IROM_BASE: u32 = 0x4000_0000;
pub const IROM_SIZE: usize = 0x6_0000; // 384 KiB
pub const DROM_BASE: u32 = 0x3FF0_0000;
pub const DROM_SIZE: usize = 0x2_0000; // 128 KiB

/// C3 DRAM: the ROM's `.data` runtime window. `.data` ends exactly at the
/// DRAM top (`ets_ops_table_ptr` @ 0x3FCD_FFFC..0x3FCE_0000).
const DRAM: DramWindow = DramWindow {
    lo: 0x3FC8_0000,
    hi: 0x3FCE_0000,
};

/// Extract the IROM and DROM flat images from the genuine C3 ROM ELF bytes.
pub fn extract_rom_images(elf_bytes: &[u8]) -> Result<RomImages, String> {
    let elf = Elf::parse(elf_bytes).map_err(|e| format!("parse ROM ELF: {e}"))?;
    let irom = build_window(&elf, elf_bytes, IROM_BASE, IROM_SIZE, true, DRAM);
    let drom = build_window(&elf, elf_bytes, DROM_BASE, DROM_SIZE, false, DRAM);
    Ok(RomImages { irom, drom })
}

/// Resolve the C3 ROM images for the faithful path, or `None` when nothing
/// resolves (native builds always resolve via the vendored images).
///
/// Order:
///   1. Explicit pre-extracted `LABWIRED_ESP32C3_ROM`/`_ROM_DATA` bins —
///      the same env pins `from_config` honors for the chip's rom regions.
///   2. Discover the toolchain ROM ELF, extract (cached by content hash).
///   3. Vendored images embedded at build time (non-wasm only).
pub fn provision_rom_images() -> Option<RomImages> {
    if let (Ok(rp), Ok(dp)) = (
        std::env::var("LABWIRED_ESP32C3_ROM"),
        std::env::var("LABWIRED_ESP32C3_ROM_DATA"),
    ) {
        if let (Ok(irom), Ok(drom)) = (std::fs::read(&rp), std::fs::read(&dp)) {
            return Some(RomImages { irom, drom });
        }
    }

    if let Some(elf_path) = discover_rom_elf() {
        if let Ok(elf_bytes) = std::fs::read(&elf_path) {
            let key = fnv1a_64(&elf_bytes);
            let dir = cache_dir();
            let irom_path = dir.join(format!("esp32c3_irom_{key:016x}.bin"));
            let drom_path = dir.join(format!("esp32c3_drom_{key:016x}.bin"));

            if let (Ok(irom), Ok(drom)) = (std::fs::read(&irom_path), std::fs::read(&drom_path)) {
                if irom.len() == IROM_SIZE && drom.len() == DROM_SIZE {
                    return Some(RomImages { irom, drom });
                }
            }

            if let Ok(images) = extract_rom_images(&elf_bytes) {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(&irom_path, &images.irom);
                let _ = std::fs::write(&drom_path, &images.drom);
                return Some(images);
            }
        }
    }

    vendored_rom_images()
}

#[cfg(not(target_arch = "wasm32"))]
fn vendored_rom_images() -> Option<RomImages> {
    static VENDORED_IROM: &[u8] = include_bytes!("../../roms/esp32c3/esp32c3_rom.bin");
    static VENDORED_DROM: &[u8] = include_bytes!("../../roms/esp32c3/esp32c3_drom.bin");
    debug_assert_eq!(VENDORED_IROM.len(), IROM_SIZE);
    debug_assert_eq!(VENDORED_DROM.len(), DROM_SIZE);
    Some(RomImages {
        irom: VENDORED_IROM.to_vec(),
        drom: VENDORED_DROM.to_vec(),
    })
}

#[cfg(target_arch = "wasm32")]
fn vendored_rom_images() -> Option<RomImages> {
    None
}

/// Cache directory for extracted ROM images (`$XDG_CACHE_HOME` or `~/.cache`).
fn cache_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(x).join("labwired/esp32c3-rom");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache/labwired/esp32c3-rom");
    }
    std::env::temp_dir().join("labwired-esp32c3-rom")
}

/// FNV-1a 64-bit hash (no extra deps; stable cache key across runs).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Locate the genuine ESP32-C3 ROM ELF in the user's installed toolchain.
///
/// Preference order:
///   1. `LABWIRED_ESP32C3_ROM_ELF` env var (explicit path).
///   2. PlatformIO (`packages/` and `tools/` layouts): `esp32c3_rev3_rom.elf`.
///   3. ESP-IDF: `~/.espressif/tools/esp-rom-elfs/<ver>/esp32c3_rev3_rom.elf`.
pub(crate) fn discover_rom_elf() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LABWIRED_ESP32C3_ROM_ELF") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let home = std::env::var("HOME").ok()?;

    for layout in ["packages", "tools"] {
        let pio = PathBuf::from(format!(
            "{home}/.platformio/{layout}/tool-esp-rom-elfs/esp32c3_rev3_rom.elf"
        ));
        if pio.is_file() {
            return Some(pio);
        }
    }

    // ESP-IDF nests the elfs under a version directory; scan one level.
    let idf_root = PathBuf::from(format!("{home}/.espressif/tools/esp-rom-elfs"));
    if let Ok(entries) = std::fs::read_dir(&idf_root) {
        for entry in entries.flatten() {
            let candidate = entry.path().join("esp32c3_rev3_rom.elf");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vendored images must be exactly what the extractor produces from
    /// the toolchain's ROM ELF — including the `.data` copy sources whose
    /// last record ends exactly at the DRAM top (`ets_ops_table_ptr`). A
    /// zeroed ops table is the failure mode this pins down: boot dies in
    /// `ets_run_flash_bootloader` with a jalr to 0.
    #[test]
    fn vendored_images_match_extraction_and_ops_table_is_populated() {
        let Some(elf_path) = discover_rom_elf() else {
            eprintln!("no C3 ROM ELF installed; skipping consistency check");
            return;
        };
        let elf_bytes = std::fs::read(&elf_path).expect("read ROM ELF");
        let images = extract_rom_images(&elf_bytes).expect("extract");
        assert_eq!(images.irom.len(), IROM_SIZE);
        assert_eq!(images.drom.len(), DROM_SIZE);

        // ets_ops_table_ptr copy source (IROM 0x40059660, per the startup
        // unpack table) must hold a non-zero DRAM pointer.
        let rel = (0x4005_9660 - IROM_BASE) as usize;
        let ops = u32::from_le_bytes(images.irom[rel..rel + 4].try_into().unwrap());
        assert_ne!(ops, 0, "ets_ops_table_ptr copy source is zero");
        // The ops table itself lives in ROM constant data (DROM); accept a
        // DRAM pointer too in case a future ROM revision moves it.
        assert!(
            (0x3FC8_0000..0x3FCE_0000).contains(&ops)
                || (DROM_BASE..DROM_BASE + DROM_SIZE as u32).contains(&ops),
            "ets_ops_table_ptr copy source {ops:#010x} points at neither DRAM nor DROM"
        );

        let vendored = vendored_rom_images().expect("vendored images");
        assert_eq!(vendored.irom, images.irom, "vendored IROM drifted");
        assert_eq!(vendored.drom, images.drom, "vendored DROM drifted");
    }
}
