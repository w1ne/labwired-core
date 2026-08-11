// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Firmware footprint and main-stack paint helpers for `labwired test`.
//!
//! Paint is a load/reset-time RAM fill (not a `SimulationObserver`). Footprint
//! uses Berkeley-style section totals from the loader.

use crate::artifacts::{footprint_from_elf_totals, FootprintReport};
use goblin::elf::program_header::PT_LOAD;
use goblin::elf::Elf;
use labwired_core::stack_paint::{
    compute_paint_range, scan_paint, LoadExtent, MainStackMethod, MainStackReport, RamRegion,
    PAINT_WORD,
};
use labwired_core::Bus; // write_u32 / read_u32 on SystemBus

/// Half-open paint window `[lo, hi)` filled with [`PAINT_WORD`] before the run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PaintSession {
    pub lo: u64,
    pub hi: u64,
}

/// Chip flash/RAM capacities and primary RAM region for paint bounds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChipMemoryMap {
    pub ram: RamRegion,
    pub flash_total: Option<u64>,
    pub ram_total: Option<u64>,
}

impl ChipMemoryMap {
    /// Build from a chip descriptor; size strings use [`labwired_config::parse_size`].
    pub(crate) fn from_chip(chip: &labwired_config::ChipDescriptor) -> Self {
        let flash_total = labwired_config::parse_size(&chip.flash.size).ok();
        let ram_total = labwired_config::parse_size(&chip.ram.size).ok();
        let ram_size = ram_total.unwrap_or(0);
        Self {
            ram: RamRegion {
                base: chip.ram.base,
                size: ram_size,
            },
            flash_total,
            ram_total,
        }
    }
}

/// YAML `stack_paint` plus `LABWIRED_STACK_PAINT` env kill switch.
///
/// Env values `0` / `false` / `off` (case-insensitive) force paint off even when
/// the script requests it. Missing env leaves the script flag as-is.
pub(crate) fn stack_paint_enabled(script: &labwired_config::TestScript) -> bool {
    stack_paint_enabled_flag(script.stack_paint)
}

/// Same kill switch when only the boolean flag is available (legacy scripts).
pub(crate) fn stack_paint_enabled_flag(script_wants: bool) -> bool {
    if !script_wants {
        return false;
    }
    match std::env::var("LABWIRED_STACK_PAINT") {
        Ok(v) => {
            let v = v.to_ascii_lowercase();
            !(v == "0" || v == "false" || v == "off")
        }
        Err(_) => true,
    }
}

/// Berkeley footprint from ELF bytes and optional chip catalog totals.
///
/// Notes always include `section_sum_not_bin_image`. When either total is known,
/// also notes `totals_from_chip_catalog`.
pub(crate) fn compute_footprint(
    firmware_bytes: &[u8],
    chip_mem: Option<&ChipMemoryMap>,
) -> Option<FootprintReport> {
    if firmware_bytes.is_empty() {
        return None;
    }
    let totals = labwired_loader::elf_section_totals_v1(firmware_bytes).ok()?;
    let (flash_total, ram_total) = match chip_mem {
        Some(m) => (m.flash_total, m.ram_total),
        None => (None, None),
    };
    let mut report = footprint_from_elf_totals(&totals, flash_total, ram_total);
    report.notes.push("section_sum_not_bin_image".to_string());
    if flash_total.is_some() || ram_total.is_some() {
        report.notes.push("totals_from_chip_catalog".to_string());
    }
    Some(report)
}

/// PT_LOAD extents with `p_vaddr` / `p_memsz` / `p_filesz`.
///
/// `memsz` includes BSS; `filesz` is 0 for pure NOBITS reserves (heap/stack).
pub(crate) fn load_extents_from_elf(firmware_bytes: &[u8]) -> Vec<LoadExtent> {
    let Ok(elf) = Elf::parse(firmware_bytes) else {
        return Vec::new();
    };
    let mut extents = Vec::new();
    for ph in &elf.program_headers {
        if ph.p_type != PT_LOAD {
            continue;
        }
        if ph.p_memsz == 0 {
            continue;
        }
        extents.push(LoadExtent {
            vaddr: ph.p_vaddr,
            memsz: ph.p_memsz,
            filesz: ph.p_filesz,
        });
    }
    extents
}

/// First of `_end`, `__bss_end__`, `__heap_start` present in the ELF.
pub(crate) fn heap_floor_symbol(firmware_bytes: &[u8]) -> Option<u64> {
    for name in ["_end", "__bss_end__", "__heap_start"] {
        if let Some(addr) = labwired_loader::resolve_symbol_in_elf(firmware_bytes, name) {
            return Some(addr as u64);
        }
    }
    None
}

/// Apply main-stack paint after load/reset, or return a disabled/unsupported report.
///
/// On `Ok`, fills `[lo, hi)` with [`PAINT_WORD`] and returns a [`PaintSession`]
/// for post-run scanning. Paint is ARM-only in P0.
pub(crate) fn apply_stack_paint(
    bus: &mut labwired_core::bus::SystemBus,
    sp_top: u64,
    arch: labwired_core::Arch,
    stack_paint: bool,
    firmware_bytes: &[u8],
    chip_mem: Option<&ChipMemoryMap>,
) -> (MainStackReport, Option<PaintSession>) {
    if !stack_paint {
        return (MainStackReport::disabled(), None);
    }
    if !matches!(arch, labwired_core::Arch::Arm) {
        return (MainStackReport::unsupported("arch_not_implemented"), None);
    }
    let Some(mem) = chip_mem else {
        return (
            MainStackReport::unsupported("stack_ram_region_unknown"),
            None,
        );
    };
    if mem.ram.size == 0 {
        return (
            MainStackReport::unsupported("stack_ram_region_unknown"),
            None,
        );
    }

    let extents = load_extents_from_elf(firmware_bytes);
    let heap_floor = heap_floor_symbol(firmware_bytes);
    let (lo, hi) = match compute_paint_range(sp_top, mem.ram, &extents, heap_floor) {
        Ok(r) => r,
        Err(reason) => return (MainStackReport::unsupported(reason), None),
    };

    let mut addr = lo;
    while addr + 4 <= hi {
        if bus.write_u32(addr, PAINT_WORD).is_err() {
            return (
                MainStackReport::unsupported("stack_paint_write_failed"),
                None,
            );
        }
        addr += 4;
    }

    (
        // Placeholder until post-run scan; not written to result.json.
        MainStackReport::unsupported("paint_pending"),
        Some(PaintSession { lo, hi }),
    )
}

/// Read the paint window and build a full [`MainStackReport`] with method Paint.
pub(crate) fn finalize_paint_report(
    bus: &labwired_core::bus::SystemBus,
    final_sp: u64,
    session: PaintSession,
) -> MainStackReport {
    let PaintSession { lo, hi } = session;
    let word_count = ((hi - lo) / 4) as usize;
    let mut words = Vec::with_capacity(word_count);
    let mut addr = lo;
    while addr + 4 <= hi {
        words.push(bus.read_u32(addr).unwrap_or(0));
        addr += 4;
    }
    let (high_water, free_min, overflow) = scan_paint(&words, final_sp, lo, hi);
    MainStackReport {
        main_stack_method: MainStackMethod::Paint,
        main_stack_limit_bytes: Some(hi - lo),
        main_stack_high_water_bytes: Some(high_water),
        main_stack_free_min_bytes: Some(free_min),
        main_stack_base: Some(lo),
        main_stack_top: Some(hi),
        main_stack_overflow_suspected: Some(overflow),
        main_stack_unsupported_reason: None,
    }
}

/// Active SP via register index 13 (ARM r13 / AAPCS). Used for paint SP top
/// and post-run high-water scan on Cortex-M.
pub(crate) fn arm_sp(cpu: &impl labwired_core::Cpu) -> u64 {
    cpu.get_register(13) as u64
}

// ── Execution metrics: PC histogram ──────────────────────────────────────────

/// Sample every N retired primary steps (cheap statistical PC histogram).
pub(crate) const PC_SAMPLE_EVERY: u64 = 256;

/// Keep the top-N hottest PCs in `result.json` metrics.
pub(crate) const PC_SAMPLE_TOP_N: usize = 16;

/// Record one PC hit in the sample histogram.
#[inline]
pub(crate) fn note_pc_sample(hist: &mut std::collections::HashMap<u32, u64>, pc: u32) {
    *hist.entry(pc).or_insert(0) += 1;
}

/// Sort histogram entries by count descending (then PC ascending for stability)
/// and take the top `n`.
pub(crate) fn top_pc_samples(
    hist: &std::collections::HashMap<u32, u64>,
    n: usize,
) -> Vec<(u32, u64)> {
    let mut entries: Vec<(u32, u64)> = hist.iter().map(|(&pc, &count)| (pc, count)).collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if entries.len() > n {
        entries.truncate(n);
    }
    entries
}

/// Resolve optional function names for top PC samples via DWARF.
pub(crate) fn resolve_pc_sample_symbols(
    samples: &[(u32, u64)],
    firmware_path: &std::path::Path,
) -> Vec<crate::artifacts::PcSample> {
    let symbols = labwired_loader::SymbolProvider::new(firmware_path).ok();
    samples
        .iter()
        .map(|&(pc, count)| {
            // Thumb: try both with and without LSB for DWARF lookup.
            let symbol = symbols.as_ref().and_then(|sp| {
                sp.lookup(pc as u64)
                    .or_else(|| sp.lookup((pc & !1) as u64))
                    .and_then(|loc| loc.function)
            });
            crate::artifacts::PcSample {
                pc: pc as u64,
                count,
                symbol,
            }
        })
        .collect()
}

#[cfg(test)]
mod pc_sample_tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn top_pc_samples_sorts_by_count_then_pc() {
        let mut hist = HashMap::new();
        hist.insert(0x1000, 10);
        hist.insert(0x2000, 50);
        hist.insert(0x1500, 50); // same count as 0x2000 → lower PC first
        hist.insert(0x3000, 5);
        hist.insert(0x4000, 1);

        let top = top_pc_samples(&hist, 3);
        assert_eq!(top.len(), 3);
        assert_eq!(top[0], (0x1500, 50));
        assert_eq!(top[1], (0x2000, 50));
        assert_eq!(top[2], (0x1000, 10));
    }

    #[test]
    fn top_pc_samples_empty_hist() {
        let hist = HashMap::new();
        assert!(top_pc_samples(&hist, 16).is_empty());
    }

    #[test]
    fn note_pc_sample_accumulates() {
        let mut hist = HashMap::new();
        note_pc_sample(&mut hist, 0xABCD);
        note_pc_sample(&mut hist, 0xABCD);
        note_pc_sample(&mut hist, 0x1234);
        assert_eq!(hist.get(&0xABCD), Some(&2));
        assert_eq!(hist.get(&0x1234), Some(&1));
    }
}
