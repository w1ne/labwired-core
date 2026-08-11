// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Main-stack paint range and high-water scan helpers.
//!
//! Pure bounds/fill/scan math for resource-metrics P0. Bus fill and CLI wiring
//! live elsewhere; this module only computes paint windows and scans words.

use serde::{Deserialize, Serialize};

/// How main-stack depth was obtained for a test run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MainStackMethod {
    Paint,
    Unsupported,
    Disabled,
}

/// Main-stack report block for `result.json` (`memory`).
///
/// Optional fields are omitted when unset (no `null` placeholders).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MainStackReport {
    pub main_stack_method: MainStackMethod,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_stack_limit_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_stack_high_water_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_stack_free_min_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_stack_base: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_stack_top: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_stack_overflow_suspected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_stack_unsupported_reason: Option<String>,
}

impl MainStackReport {
    /// Paint was skipped by YAML/env kill switch.
    pub fn disabled() -> Self {
        Self {
            main_stack_method: MainStackMethod::Disabled,
            main_stack_limit_bytes: None,
            main_stack_high_water_bytes: None,
            main_stack_free_min_bytes: None,
            main_stack_base: None,
            main_stack_top: None,
            main_stack_overflow_suspected: None,
            main_stack_unsupported_reason: None,
        }
    }

    /// Paint could not be performed safely; `reason` is a stable code string.
    pub fn unsupported(reason: &str) -> Self {
        Self {
            main_stack_method: MainStackMethod::Unsupported,
            main_stack_limit_bytes: None,
            main_stack_high_water_bytes: None,
            main_stack_free_min_bytes: None,
            main_stack_base: None,
            main_stack_top: None,
            main_stack_overflow_suspected: None,
            main_stack_unsupported_reason: Some(reason.to_string()),
        }
    }
}

/// Chip RAM region used for stack placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RamRegion {
    pub base: u64,
    pub size: u64,
}

impl RamRegion {
    /// Exclusive end address (`base + size`, saturating).
    pub fn end(&self) -> u64 {
        self.base.saturating_add(self.size)
    }

    /// True if `addr` is in `[base, end)`.
    pub fn contains(&self, addr: u64) -> bool {
        addr >= self.base && addr < self.end()
    }
}

/// Load segment extent for paint floor / safety.
///
/// - `memsz` (`p_memsz`): full runtime image including BSS — used for the default
///   heap floor so paint starts after zeroed globals.
/// - `filesz` (`p_filesz`): bytes actually present in the ELF file. Pure NOBITS
///   reservations (`filesz == 0`), such as GNU `._user_heap_stack`, are free
///   space (heap/stack) and may be painted when a heap-floor symbol is known.
#[derive(Debug, Clone, Copy)]
pub struct LoadExtent {
    pub vaddr: u64,
    pub memsz: u64,
    pub filesz: u64,
}

/// Word pattern written into the unused stack window before the run.
pub const PAINT_WORD: u32 = 0xA5A5_A5A5;

/// Minimum paint window size; smaller ranges are rejected as unsupported.
pub const MIN_PAINT_BYTES: u64 = 64;

/// Compute half-open paint range `[paint_lo, paint_hi)` or an unsupported reason.
///
/// - `sp_top`: reset SP, word-aligned down (exclusive top of paint).
/// - `ram`: chip RAM region containing the stack.
/// - `load_extents`: PT_LOAD-style extents using **memsz** so BSS is excluded.
/// - `heap_floor_symbol`: optional `_end` / `__bss_end__` / `__heap_start` if in RAM.
///
/// Error codes: `stack_ram_region_unknown`, `stack_region_too_small_or_unknown`,
/// `stack_range_unsafe`.
pub fn compute_paint_range(
    sp_top: u64,
    ram: RamRegion,
    load_extents: &[LoadExtent],
    heap_floor_symbol: Option<u64>,
) -> Result<(u64, u64), &'static str> {
    let sp_top = sp_top & !0x3; // word align down

    // SP at top of RAM (one past last byte) is OK when sp_top == ram.end().
    // Otherwise SP (or the last stack byte) must lie in the RAM region.
    if !ram.contains(sp_top.saturating_sub(1)) && sp_top != ram.end() {
        if sp_top != ram.end() && !ram.contains(sp_top) {
            return Err("stack_ram_region_unknown");
        }
    }

    let mut image_ram_end = ram.base;
    for ext in load_extents {
        let start = ext.vaddr;
        let end = ext.vaddr.saturating_add(ext.memsz);
        // Intersect load extent with RAM.
        let lo = start.max(ram.base);
        let hi = end.min(ram.end());
        if hi > lo {
            image_ram_end = image_ram_end.max(hi);
        }
    }

    let heap_floor = match heap_floor_symbol {
        Some(s) if ram.contains(s) || s == ram.end() => s,
        _ => image_ram_end,
    };

    let paint_lo = ram.base.max(heap_floor);
    let paint_hi = sp_top;
    if paint_hi <= paint_lo || paint_hi - paint_lo < MIN_PAINT_BYTES {
        return Err("stack_region_too_small_or_unknown");
    }

    // Refuse if file-backed image (data + BSS via memsz) overlaps the paint
    // window. Pure NOBITS segments (`filesz == 0`) are skipped: they are either
    // a dedicated BSS PT_LOAD (already covered by image_ram_end / heap-floor
    // symbols) or a heap/stack reserve (GNU `._user_heap_stack`) that paint
    // is *meant* to cover when `_end` / `__bss_end__` is known.
    for ext in load_extents {
        if ext.filesz == 0 {
            continue;
        }
        let lo = ext.vaddr.max(paint_lo);
        let hi = ext.vaddr.saturating_add(ext.memsz).min(paint_hi);
        if hi > lo {
            return Err("stack_range_unsafe");
        }
    }

    Ok((paint_lo, paint_hi))
}

/// Scan painted region from low addresses upward while words equal [`PAINT_WORD`].
///
/// Returns `(high_water_bytes, free_min_bytes, overflow_suspected)`.
///
/// - `words` must cover `[paint_lo, paint_hi)` as little-endian words in address order.
/// - Overflow is suspected if no paint remains, or `final_sp` is outside the window.
pub fn scan_paint(words: &[u32], final_sp: u64, paint_lo: u64, paint_hi: u64) -> (u64, u64, bool) {
    let limit = paint_hi - paint_lo;
    let mut unused_words = 0u64;
    for &w in words {
        if w == PAINT_WORD {
            unused_words += 1;
        } else {
            break;
        }
    }
    let unused = unused_words * 4;
    let high_water = limit.saturating_sub(unused);
    let overflow = unused == 0 || final_sp < paint_lo || final_sp > paint_hi;
    (high_water, unused, overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_range_uses_memsz_not_filesz() {
        let ram = RamRegion {
            base: 0x2000_0000,
            size: 0x5000,
        };
        // .data filesz small but memsz includes bss — paint must start after memsz.
        let extents = [LoadExtent {
            vaddr: 0x2000_0000,
            memsz: 0x1000, // 4K image in RAM
            filesz: 0x80,
        }];
        let sp = 0x2000_5000;
        let (lo, hi) = compute_paint_range(sp, ram, &extents, None).unwrap();
        assert_eq!(lo, 0x2000_1000);
        assert_eq!(hi, 0x2000_5000);
    }

    #[test]
    fn user_heap_stack_nobits_is_paintable_with_end_symbol() {
        // Matches stm32f103-blinky.elf: .data+.bss LOAD + pure NOBITS heap/stack.
        let ram = RamRegion {
            base: 0x2000_0000,
            size: 0x5000,
        };
        let extents = [
            LoadExtent {
                vaddr: 0x2000_0000,
                memsz: 0x46c,
                filesz: 0x7c,
            },
            LoadExtent {
                vaddr: 0x2000_046c,
                memsz: 0x604,
                filesz: 0,
            },
        ];
        let sp = 0x2000_5000;
        let end = 0x2000_0470;
        let (lo, hi) = compute_paint_range(sp, ram, &extents, Some(end)).unwrap();
        assert_eq!(lo, end);
        assert_eq!(hi, sp);
    }

    #[test]
    fn scan_half_used() {
        let mut words = vec![PAINT_WORD; 64]; // 256 bytes
        // "use" top 128 bytes → last 32 words clobbered
        for w in words.iter_mut().skip(32) {
            *w = 0;
        }
        let paint_lo = 0x2000_0000;
        let paint_hi = paint_lo + 256;
        let (hw, free, ov) = scan_paint(&words, paint_hi - 4, paint_lo, paint_hi);
        assert_eq!(free, 128);
        assert_eq!(hw, 128);
        assert!(!ov);
    }

    #[test]
    fn too_small_is_unsupported() {
        let ram = RamRegion {
            base: 0x2000_0000,
            size: 0x100,
        };
        let sp = 0x2000_0030;
        let err = compute_paint_range(sp, ram, &[], None).unwrap_err();
        assert_eq!(err, "stack_region_too_small_or_unknown");
    }
}
