// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Berkeley-style ELF footprint totals matching GNU `arm-none-eabi-size`.
//!
//! Classifies `SHF_ALLOC` sections into text / data / bss using the
//! `elf_section_totals_v1` rules. Pure function over ELF bytes — no I/O
//! beyond what the caller provides.

use anyhow::{Context, Result};
use goblin::elf::section_header::{SHF_ALLOC, SHF_EXECINSTR, SHF_WRITE, SHT_NOBITS, SHT_PROGBITS};
use goblin::elf::Elf;

/// Method string identifying this classifier version.
pub const FOOTPRINT_METHOD: &str = "elf_section_totals_v1";

/// Berkeley-style section totals (bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElfSectionTotals {
    pub text: u64,
    pub data: u64,
    pub bss: u64,
}

impl ElfSectionTotals {
    /// Flash occupancy: text + data (initialized image in flash).
    pub fn flash_used(&self) -> u64 {
        self.text + self.data
    }

    /// Static RAM occupancy: data + bss (runtime RAM for globals / zeroed / heap-stack reserve).
    pub fn ram_static(&self) -> u64 {
        self.data + self.bss
    }
}

/// Classify `SHF_ALLOC` sections into Berkeley text/data/bss.
///
/// Rules (`elf_section_totals_v1`):
/// - Only sections with `SHF_ALLOC` are counted.
/// - `SHT_NOBITS` alloc → **bss** (includes GNU `._user_heap_stack`).
/// - Writable non-exec `SHT_PROGBITS` → **data**.
/// - Everything else alloc (code, rodata, vectors, init arrays, …) → **text**.
pub fn elf_section_totals_v1(buffer: &[u8]) -> Result<ElfSectionTotals> {
    let elf = Elf::parse(buffer).context("failed to parse ELF for section totals")?;

    let mut text: u64 = 0;
    let mut data: u64 = 0;
    let mut bss: u64 = 0;

    for sh in &elf.section_headers {
        let flags = sh.sh_flags as u32;
        if flags & SHF_ALLOC == 0 {
            continue;
        }

        let size = sh.sh_size;
        if size == 0 {
            continue;
        }

        if sh.sh_type == SHT_NOBITS {
            bss += size;
            continue;
        }

        let writable = flags & SHF_WRITE != 0;
        let executable = flags & SHF_EXECINSTR != 0;
        if sh.sh_type == SHT_PROGBITS && writable && !executable {
            data += size;
        } else {
            text += size;
        }
    }

    Ok(ElfSectionTotals { text, data, bss })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn stm32f103_blinky_matches_gnu_berkeley_size() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/stm32f103-blinky.elf");
        let buffer = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));

        let totals = elf_section_totals_v1(&buffer).expect("classify fixture");

        // Verified with `arm-none-eabi-size` on tests/fixtures/stm32f103-blinky.elf:
        //   text=12760 data=124 bss=2548
        // bss includes .bss (1008) + ._user_heap_stack (1540).
        assert_eq!(totals.text, 12760, "text");
        assert_eq!(totals.data, 124, "data");
        assert_eq!(totals.bss, 2548, "bss");
        assert_eq!(totals.flash_used(), 12760 + 124);
        assert_eq!(totals.ram_static(), 124 + 2548);
        assert_eq!(FOOTPRINT_METHOD, "elf_section_totals_v1");
    }
}
