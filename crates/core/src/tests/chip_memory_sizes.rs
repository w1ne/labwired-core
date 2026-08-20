// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A chip may not declare a memory size that no silicon has.
//!
//! `labwired_config::parse_size` reads `KB` as BINARY (1024) and `MB` as
//! DECIMAL (1_000_000) — two bases inside one parser, the opposite of what the
//! spelling suggests. That asymmetry is pinned by
//! `labwired_config::memory_size_tests::kb_is_1024_and_mb_is_1000000` and it
//! stays pinned: the multipliers are the wire format every committed chip,
//! every hosted manifest and every out-of-tree descriptor was written against,
//! so moving them would shift partition tables, XIP windows and
//! memory-violation limits across every chip at once. The spelling gets
//! policed instead, and that is what this file does.
//!
//! It is not hypothetical. Nine committed chips spelled flash in `MB` and so
//! modelled less flash than the part has — esp32s3 was 16_000_000 where the
//! module holds 16 MiB = 16_777_216, and the shortfall is invisible until an
//! image uses the top of flash, at which point `esp_flash` aborts with
//! "Detected size(8192k) smaller than the size in the binary image
//! header(16384k)". A wrong answer from an oracle, arrived at silently.
//!
//! Two gates, because one spelling rule and one arithmetic rule catch
//! different mistakes and neither subsumes the other:
//!
//! * [`no_chip_declares_a_size_in_decimal_mb`] rejects the spelling at source.
//!   It is the only one that catches `64MB` — 64_000_000 happens to be an
//!   exact multiple of 4 KiB, so the arithmetic gate below waves it through.
//! * [`declared_memory_sizes_are_real_part_sizes`] rejects the arithmetic. It
//!   is the only one that catches a byte count typed straight in
//!   (`size: 4000000`), where there is no unit to object to.
//!
//! Scope: `configs/chips/*.yaml`, the chips this engine ships and models — the
//! same corpus `cpu_hz_single_source.rs` gates. `configs/chips/onboarding/`
//! is deliberately excluded: it is a generated Renode-derived catalogue that
//! states every size as a bare byte count, so the `MB` spelling cannot arise
//! there, and it carries placeholder geometry (one entry declares a 4-byte
//! flash) that this invariant would fail for unrelated reasons.

use labwired_config::ChipDescriptor;
use std::path::PathBuf;

fn chips_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips")
}

/// Every shipped chip descriptor, sorted. Panics if the corpus is missing or
/// suspiciously small — a gate that silently scans nothing passes forever.
fn chip_files() -> Vec<PathBuf> {
    let dir = chips_dir();
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .collect();
    out.sort();
    // Vacuity guard: the corpus is ~30 chips today. A path typo or a moved
    // configs/ directory must fail here, not read as "no violations".
    assert!(
        out.len() >= 20,
        "expected the shipped chip corpus at {}, found {} yaml files",
        dir.display(),
        out.len()
    );
    let names: Vec<String> = out
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    for expected in ["esp32c3.yaml", "rp2040.yaml", "stm32f103.yaml"] {
        assert!(
            names.iter().any(|n| n == expected),
            "chip corpus does not contain {expected}; scanning the wrong directory?"
        );
    }
    out
}

/// The quoted (or bare) value of every `size:` key in a chip YAML, with the
/// 1-based line it sits on. A line scan rather than a YAML walk on purpose:
/// `size:` appears at the top level (`flash`, `ram`), inside
/// `memory_regions:` and inside every peripheral, and all of them are equally
/// wrong when spelled in decimal MB.
fn size_declarations(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let trimmed = line
                .trim_start()
                .strip_prefix("- ")
                .unwrap_or(line.trim_start());
            let value = trimmed.strip_prefix("size:")?;
            // Drop a trailing `# comment`, then the quotes.
            let value = value.split('#').next().unwrap_or("").trim();
            let value = value.trim_matches(|c| c == '"' || c == '\'');
            if value.is_empty() {
                return None;
            }
            Some((i + 1, value.to_string()))
        })
        .collect()
}

/// No shipped chip spells a size in decimal `MB`/`GB`.
///
/// `MiB`/`KiB`/`GiB` and `KB` are binary and mean what a datasheet means;
/// `MB` and `GB` are the SI powers of ten and are always a mistake in a part
/// geometry. The check is on the spelling because that is where the mistake is
/// made — a reviewer who sees `4MB` next to a 4 MiB part reads it as correct.
#[test]
fn no_chip_declares_a_size_in_decimal_mb() {
    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    for path in chip_files() {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        for (line, value) in size_declarations(&text) {
            scanned += 1;
            let lower = value.to_ascii_lowercase();
            // `mib`/`gib` end in "ib", so they do not match here.
            if lower.ends_with("mb") || lower.ends_with("gb") {
                offenders.push(format!("{name}:{line} size: \"{value}\""));
            }
        }
    }
    // Vacuity guard: every chip declares at least flash and ram.
    assert!(
        scanned >= 40,
        "only {scanned} `size:` declarations found — the scanner matched nothing"
    );
    assert!(
        offenders.is_empty(),
        "these sizes are spelled in decimal MB/GB, which `parse_size` reads as \
         powers of TEN — \"4MB\" is 4_000_000 bytes, 194_304 short of the \
         4 MiB the part has. Spell binary sizes in KB (1024) or MiB: 1 MiB is \
         \"1024KB\", 2 MiB is \"2048KB\", 4 MiB is \"4096KB\". Offenders: \
         {offenders:#?}"
    );
}

/// Declared memory geometry is geometry a real part can have.
///
/// * Flash is erased in sectors. The smallest erase unit on anything this
///   engine models is the 4 KiB NOR sector (RP2040/ESP32 external QSPI, nRF
///   and EFR32 pages); the STM32s erase in 16/64/128 KiB sectors, which are
///   multiples of it. A flash size that is not a whole number of 4 KiB sectors
///   is not a part size — and every decimal-MB spelling from 1MB to 32MB
///   lands there, because 10^6 = 2^6 · 5^6 carries only six factors of two.
/// * RAM and the extra CPU-visible windows are whole KiB. Erase granularity
///   does not apply to them (the ATmega328P has 2 KiB of SRAM), but no part
///   exposes a window that stops part-way through a KiB.
///
/// This is the backstop for the mistake with no unit attached: `size: 4000000`
/// spelled as a bare byte count reaches the same wrong number by hand.
#[test]
fn declared_memory_sizes_are_real_part_sizes() {
    const SECTOR: u64 = 4096;
    let mut offenders: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for path in chip_files() {
        let chip = ChipDescriptor::from_file(&path)
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let name = path.file_name().unwrap().to_string_lossy().into_owned();

        checked += 1;
        if chip.flash.size == 0 || chip.flash.size % SECTOR != 0 {
            offenders.push(format!(
                "{name}: flash.size = {} bytes = {:.3} KiB — not a whole number of \
                 4 KiB erase sectors",
                chip.flash.size,
                chip.flash.size as f64 / 1024.0
            ));
        }
        if chip.ram.size == 0 || chip.ram.size % 1024 != 0 {
            offenders.push(format!(
                "{name}: ram.size = {} bytes = {:.3} KiB — not a whole number of KiB",
                chip.ram.size,
                chip.ram.size as f64 / 1024.0
            ));
        }
        for region in &chip.memory_regions {
            if region.size == 0 || region.size % 1024 != 0 {
                offenders.push(format!(
                    "{name}: memory_regions[{}].size = {} bytes = {:.3} KiB — not a \
                     whole number of KiB",
                    region.name,
                    region.size,
                    region.size as f64 / 1024.0
                ));
            }
        }
    }

    assert!(checked >= 20, "only {checked} chips loaded");
    assert!(
        offenders.is_empty(),
        "these declared sizes are not sizes silicon has. The usual cause is a \
         decimal-MB spelling (`parse_size` reads MB as 10^6, so \"1MB\" is \
         1_000_000 — 48_576 bytes short of 1 MiB); write the binary size \
         instead (\"1024KB\", \"2048KB\", \"4096KB\"). Offenders: {offenders:#?}"
    );
}
