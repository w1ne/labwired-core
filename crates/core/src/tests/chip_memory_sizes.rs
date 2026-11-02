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
//! Scope: `configs/chips/*.yaml`, the chip descriptors this engine ships — the
//! same corpus `cpu_hz_single_source.rs` gates, including declared-only SoC
//! geometry. This checks memory sizes, not modeled board/peripheral support.
//! `configs/chips/onboarding/`
//! is deliberately excluded: it is a generated Renode-derived catalogue that
//! states every size as a bare byte count, so the `MB` spelling cannot arise
//! there, and it carries placeholder geometry (one entry declares a 4-byte
//! flash) that this invariant would fail for unrelated reasons.

use labwired_config::{Arch, ChipDescriptor};
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
///   RV1103/Cortex-A7 is the explicit exception to a positive flash size: the
///   Luckfox Mini A boots from external MicroSD, not internal flash. Its zero
///   flash placeholder maps no storage; RAM/window checks still apply.
/// * RAM and the extra CPU-visible windows are whole KiB. Erase granularity
///   does not apply to them (the ATmega328P has 2 KiB of SRAM). The PIC24F KA
///   family is the documented exception: the 04KA200/201 have 512 B of data
///   SRAM (DS30009937C Table 1-1 and the §4.2 stack example ending at 0x09FF)
///   and the 08KA/16KA parts have 1536 B (DS39927C Table 1-1). The PIC18F4520
///   also has 1536 B (six 256 B GPR banks, DS39631E device table), and the
///   PIC16F877A 368 B (DS39582B device table). A whole-KiB rule can express
///   none of these.
///
/// This is the backstop for the mistake with no unit attached: `size: 4000000`
/// spelled as a bare byte count reaches the same wrong number by hand.
#[test]
fn declared_memory_sizes_are_real_part_sizes() {
    let mut offenders: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for path in chip_files() {
        let chip = ChipDescriptor::from_file(&path)
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let name = path.file_name().unwrap().to_string_lossy().into_owned();

        checked += 1;
        if !flash_geometry_is_valid(&chip) {
            offenders.push(format!(
                "{name}: flash.size = {} bytes = {:.3} KiB — not a whole number of \
                 4 KiB erase sectors",
                chip.flash.size,
                chip.flash.size as f64 / 1024.0
            ));
        }
        if !ram_geometry_is_valid(&chip) {
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

/// Non-KiB-aligned SRAM is real on exactly these PIC24F KA, PIC18 and PIC16 parts.
/// Everywhere else a non-KiB-aligned RAM size is still a transcription error.
const NON_KIB_RAM_PARTS: &[(&str, u64)] = &[
    ("pic24f04ka200", 512),  // DS30009937C Table 1-1, §4.2
    ("pic24f04ka201", 512),  // DS30009937C Table 1-1, §4.2
    ("pic24f08ka101", 1536), // DS39927C Table 1-1
    ("pic24f08ka102", 1536), // DS39927C Table 1-1
    ("pic24f16ka101", 1536), // DS39927C Table 1-1
    ("pic24f16ka102", 1536), // DS39927C Table 1-1
    ("pic18f4520", 1536),    // DS39631E: GPR banks 0-5
    ("pic16f877a", 368),     // DS39582B: 96 + 80 + 96 + 96 B GPR, 0x70-0x7F shared
    ("pic16f1829", 1024),    // DS40001440E: 1024 B incl. 16 B common RAM
];

fn ram_geometry_is_valid(chip: &ChipDescriptor) -> bool {
    if chip.ram.size == 0 {
        return false;
    }
    if chip.ram.size % 1024 == 0 {
        return true;
    }
    NON_KIB_RAM_PARTS
        .iter()
        .any(|(name, size)| chip.name == *name && chip.ram.size == *size)
}

/// Zero is an honest absence of internal flash only for this declared SoC.
/// The base must also be zero: the placeholder must not imply a storage map.
fn flash_geometry_is_valid(chip: &ChipDescriptor) -> bool {
    const SECTOR: u64 = 4096;
    if chip.flash.size != 0 {
        return chip.flash.size % SECTOR == 0;
    }
    // RV1103 boots from MicroSD; the XCK26 RPU runs from TCM/DDR the FSBL
    // loads; the AM6234 A53s run from DDR that U-Boot (or here, the loader)
    // fills. None has internal flash.
    [
        ("rv1103", "cortex-a7", Arch::Arm),
        ("xck26-rpu", "cortex-r5f", Arch::Arm),
        ("am6234", "cortex-a53", Arch::Aarch64),
    ]
    .iter()
    .any(|(name, core, arch)| {
        chip.name == *name && chip.core.as_deref() == Some(*core) && chip.arch == *arch
    }) && chip.flash.base == 0
}

#[test]
fn non_kib_ram_exception_is_exactly_the_listed_pic_parts() {
    for (name, size) in NON_KIB_RAM_PARTS {
        let path = chips_dir().join(format!("{name}.yaml"));
        let chip = ChipDescriptor::from_file(&path).expect("load non-KiB RAM part");
        assert_eq!(chip.ram.size, *size, "{name}");
        assert!(ram_geometry_is_valid(&chip), "{name}");
    }
    let mut other = ChipDescriptor::from_file(chips_dir().join("pic24f04ka200.yaml")).unwrap();
    other.name = "not-a-pic24f-ka".into();
    assert!(
        !ram_geometry_is_valid(&other),
        "the exception must not leak by size alone"
    );
    other.name = "pic24f04ka200".into();
    other.ram.size = 768;
    assert!(
        !ram_geometry_is_valid(&other),
        "the exception is 512/1536 B, not any non-KiB size"
    );
}

#[test]
fn zero_internal_flash_is_only_allowed_for_the_declared_socs() {
    let rpu = ChipDescriptor::from_file(chips_dir().join("xck26-rpu.yaml")).unwrap();
    assert!(flash_geometry_is_valid(&rpu));
    let am62 = ChipDescriptor::from_file(chips_dir().join("am6234.yaml")).unwrap();
    assert!(flash_geometry_is_valid(&am62));
    let mut am62_as_arm = am62.clone();
    am62_as_arm.arch = Arch::Arm;
    assert!(
        !flash_geometry_is_valid(&am62_as_arm),
        "the arch is part of the exemption"
    );
    let mut wrong_core = rpu.clone();
    wrong_core.core = Some("cortex-a7".into());
    assert!(!flash_geometry_is_valid(&wrong_core));

    let chip = ChipDescriptor::from_file(chips_dir().join("rv1103.yaml")).unwrap();
    assert_eq!(chip.flash.size, 0);
    assert_eq!(chip.flash.base, 0);
    assert!(flash_geometry_is_valid(&chip));

    let mut other_model = chip.clone();
    other_model.name = "another-cortex-a7".into();
    assert!(!flash_geometry_is_valid(&other_model));

    let mut other_core = chip.clone();
    other_core.core = Some("cortex-m3".into());
    assert!(!flash_geometry_is_valid(&other_core));

    let mut undeclared_core = chip.clone();
    undeclared_core.core = None;
    assert!(!flash_geometry_is_valid(&undeclared_core));

    let mut other_arch = chip.clone();
    other_arch.arch = Arch::RiscV;
    assert!(!flash_geometry_is_valid(&other_arch));

    let mut mapped_storage = chip.clone();
    mapped_storage.flash.base = 0x2000_0000;
    assert!(!flash_geometry_is_valid(&mapped_storage));

    let mut mcu = ChipDescriptor::from_file(chips_dir().join("stm32f103.yaml")).unwrap();
    assert!(flash_geometry_is_valid(&mcu));
    mcu.flash.size = 0;
    assert!(!flash_geometry_is_valid(&mcu));
    mcu.flash.size = 4_000_000;
    assert!(!flash_geometry_is_valid(&mcu));
    mcu.flash.size = 4096 * 1024;
    assert!(flash_geometry_is_valid(&mcu));
}

/// The `stm32f103.yaml` descriptor is named `stm32f103c8`, and a C8 has 64 KiB
/// of main flash and 20 KiB of SRAM: DS5319 Rev 20, Table 2 ("STM32F103xx
/// medium-density device features and peripheral counts") gives Flash 64
/// Kbytes for the `x8` parts and 128 Kbytes for `xB`, SRAM 20 Kbytes for both.
/// RM0008 Rev 21 §3.3.3 Table 5 lays the medium-density main block out as
/// 1 KiB pages from 0x0800_0000. The descriptor once declared 1 MiB, so an
/// image too large for the part it names loaded and ran in the simulator.
#[test]
fn stm32f103c8_declares_the_datasheet_flash_and_sram() {
    let chip = ChipDescriptor::from_file(chips_dir().join("stm32f103.yaml")).unwrap();
    assert_eq!(chip.name, "stm32f103c8");
    assert_eq!(chip.flash.base, 0x0800_0000);
    assert_eq!(chip.flash.size, 64 * 1024, "DS5319 Table 2: C8 = 64 KiB");
    assert_eq!(chip.ram.base, 0x2000_0000);
    assert_eq!(chip.ram.size, 20 * 1024, "DS5319 Table 2: 20 KiB SRAM");
}
