// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-P4 fast-boot substrate for the bare-ELF path.
//!
//! The P4 L1 smoke treats the unified external-memory window at `0x4000_0000`
//! as plain memory — there is no SPI flash controller behind it, so a bare ELF
//! loads straight at its link addresses (mirroring the C6/C3 bare-ELF path).
//! This module owns the one P4-specific fact that path needs: where the app
//! stack may sit without colliding with the memory the mask ROM keeps for
//! itself.
//!
//! ## Why the app stack is capped
//!
//! `esp32p4` `soc.h` reserves the top of HP SRAM for the ROM: the mask-ROM boot
//! stack starts at `SOC_ROM_STACK_START = 0x4ff3_cfc0` (`SOC_ROM_STACK_SIZE =
//! 0x2000`), and `SOC_DIRAM_ROM_RESERVE_HIGH = 0x4ff4_0000` marks the ceiling of
//! the ROM-reserved region. The P4's HP SRAM is `0x4ff0_0000..0x4ffc_0000`
//! (`SOC_IRAM_LOW/HIGH`), so the app-usable span ends at `0x4ff4_0000`. Seeding
//! SP at the full SRAM top (`0x4ffc_0000`) would let the first deep call frame
//! cross into the ROM reservation — the same failure the C6 module documents at
//! its own `APP_SRAM_END`.
//!
//! Unlike the C6 this module does **not** install MSPI/SPI_MEM controllers or
//! seed a partition table: the P4 fast-boot path does not model IDF
//! `nvs_flash_init`/Preferences at L1, and the P4 flash XIP window is plain
//! memory (see `examples/esp32p4-guition-jc4880p443/KNOWN_LIMITATIONS.md`).

use labwired_config::ChipDescriptor;

/// End of the app-usable HP SRAM (exclusive). From `esp32p4` `soc.h`
/// `SOC_DIRAM_ROM_RESERVE_HIGH = 0x4ff4_0000` — the mask ROM keeps HP SRAM from
/// here to the 0x4ffc_0000 top for its `.data`/`.bss` and interface pointers.
pub const APP_SRAM_END: u32 = 0x4ff4_0000;

/// Fast-boot SP for P4 bare ELFs — 16-byte aligned top of app-usable SRAM.
pub fn app_stack_top() -> u32 {
    APP_SRAM_END & !0xF
}

/// True when `chip` is an ESP32-P4 (name match, same style as
/// [`crate::boot::esp32c6_rom::is_esp32c6`]).
pub fn is_esp32p4(chip: &ChipDescriptor) -> bool {
    chip.name.to_ascii_lowercase().contains("esp32p4")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn is_esp32p4_matches_chip_name() {
        let mut chip = ChipDescriptor::from_file(repo_root().join("configs/chips/esp32p4.yaml"))
            .expect("load esp32p4.yaml");
        assert!(is_esp32p4(&chip));
        chip.name = "esp32c6".into();
        assert!(!is_esp32p4(&chip));
    }

    #[test]
    fn app_stack_top_is_below_the_rom_reservation() {
        // 16-byte aligned, and never above the ROM-reserved HP-SRAM ceiling.
        assert_eq!(app_stack_top() & 0xF, 0);
        assert!(u64::from(app_stack_top()) <= u64::from(APP_SRAM_END));
        assert!(u64::from(app_stack_top()) < 0x4ffc_0000);
    }
}
