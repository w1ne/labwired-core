// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Tier-1 chip × peripheral validation matrix.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One cell's status. `Na` = chip YAML declares no peripheral of this class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellStatus {
    Pass,
    Partial,
    Blocked,
    Na,
    Unrecorded,
}

impl CellStatus {
    /// Snapshot vocabulary — must stay in sync with the serde snake_case names.
    pub fn as_str(self) -> &'static str {
        match self {
            CellStatus::Pass => "pass",
            CellStatus::Partial => "partial",
            CellStatus::Blocked => "blocked",
            CellStatus::Na => "na",
            CellStatus::Unrecorded => "unrecorded",
        }
    }
}

/// A cell with its evidence link (CI run that produced it; None until CI stamps it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub status: CellStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_url: Option<String>,
}

/// chip → class → cell. BTreeMaps keep JSON output deterministic (sorted keys).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier1Matrix(pub BTreeMap<String, BTreeMap<String, Cell>>);

/// The standard classes every chip's row carries: the six bring-up rubric
/// classes plus the typical MCU peripheral set. Classes a chip doesn't declare
/// render `na`; classes a fixture hasn't attempted yet render `unrecorded`.
pub const RUBRIC_CLASSES: &[&str] = &[
    "clock", "gpio", "uart", "timer", "dma", "irq", // bring-up rubric
    "i2c", "spi", "adc", "pwm", "wdt", "rtc", // typical peripherals
];

/// Parsed `TIER1` protocol from a UART capture.
#[derive(Debug, Default)]
pub struct ParsedTier1 {
    /// class → status from explicit `TIER1 <class> PASS|FAIL` lines.
    /// Repeated reports for a class take the last occurrence — supports
    /// fixture-internal retries. Class tokens are case-sensitive: `TIER1 GPIO
    /// PASS` records a class `GPIO` that no standard row consumes.
    pub classes: BTreeMap<String, CellStatus>,
    /// `TIER1 done` seen — the fixture completed its sequence.
    pub done: bool,
}

/// Parse `TIER1 <class> PASS|FAIL[ code=..]` lines + `TIER1 done` out of a raw
/// UART byte capture. Non-UTF8 and unrelated lines are skipped; malformed
/// `TIER1` lines are ignored (never fatal — boot noise is expected). Leading and
/// trailing whitespace on each token is normalised by `split_whitespace`; CRLF
/// line endings are handled by `lines()`.
/// Printed-class aliases: fixtures may report a chip-specific peripheral name
/// that maps onto a standard column (the ESP32-S3 fixture prints `mcpwm`, that
/// chip's PWM block). Applied at parse time so committed fixture blobs keep
/// working as columns standardize.
const CLASS_ALIASES: &[(&str, &str)] = &[("mcpwm", "pwm"), ("ledc", "pwm")];

fn canonical_class(class: &str) -> String {
    for (alias, std) in CLASS_ALIASES {
        if class == *alias {
            return (*std).to_string();
        }
    }
    class.to_string()
}

pub fn parse_tier1_uart(uart: &[u8]) -> ParsedTier1 {
    let mut out = ParsedTier1::default();
    for line in String::from_utf8_lossy(uart).lines() {
        let mut it = line.split_whitespace();
        if it.next() != Some("TIER1") {
            continue;
        }
        match (it.next(), it.next()) {
            (Some("done"), _) => out.done = true,
            (Some(class), Some("PASS")) => {
                out.classes.insert(canonical_class(class), CellStatus::Pass);
            }
            (Some(class), Some("FAIL")) => {
                out.classes
                    .insert(canonical_class(class), CellStatus::Blocked);
            }
            _ => {} // malformed TIER1 line — ignore
        }
    }
    out
}

impl ParsedTier1 {
    /// Resolve a full row over `classes`. Rules (spec §2 conventions):
    ///
    /// - If the fixture explicitly reported `uart` (a `TIER1 uart PASS|FAIL`
    ///   line), that explicit status wins, subject to the same done-degradation
    ///   rule as every other class (explicit Pass without done → Partial).
    /// - Otherwise `uart` is Pass iff `done` was seen — receiving a `TIER1
    ///   done` line over UART is itself the proof of a working UART channel.
    ///   `!classes.is_empty()` is **not** required.
    /// - Missing `done` degrades every reported Pass to Partial (hung
    ///   mid-sequence), and classes never reported are Blocked (the fixture
    ///   hung before reaching them).
    /// - With `done` seen, classes never reported are Unrecorded — the fixture
    ///   simply doesn't attempt them yet; no claim either way.
    pub fn resolve_row(&self, classes: &[&str]) -> BTreeMap<String, Cell> {
        let mut row = BTreeMap::new();
        for &class in classes {
            let status = if class == "uart" {
                match self.classes.get("uart") {
                    // Explicit uart verdict from the fixture — honour it, same
                    // done-degradation as every other class.
                    Some(CellStatus::Pass) if !self.done => CellStatus::Partial,
                    Some(s) => *s,
                    // No explicit uart line: done alone proves UART is alive.
                    None if self.done => CellStatus::Pass,
                    None => CellStatus::Blocked,
                }
            } else {
                match self.classes.get(class) {
                    Some(CellStatus::Pass) if !self.done => CellStatus::Partial,
                    Some(s) => *s,
                    // Not attempted by this fixture: no claim either way. The
                    // ratchet flags pass->unrecorded if a check is removed.
                    None if self.done => CellStatus::Unrecorded,
                    None => CellStatus::Blocked, // hung before reaching it
                }
            };
            row.insert(
                class.to_string(),
                Cell {
                    status,
                    run_url: None,
                },
            );
        }
        row
    }
}

/// peripheral-id substring → tier1 class. First match wins. Order-sensitive
/// pair: `"_pwm"` must precede `"tim"` — STM32 advanced-control timers declare
/// the pwm class via an `_pwm` id suffix (e.g. `tim1_pwm`), which would
/// otherwise be swallowed by the `tim`→timer marker. (The timer class itself
/// comes from the plain `timN` instances.)
const CLASS_MARKERS: &[(&str, &str)] = &[
    ("_pwm", "pwm"),
    ("uart", "uart"),
    ("usart", "uart"),      // STM32 naming: usart1 does not substring-match "uart"
    ("usb_serial", "uart"), // S3 console can be USB-Serial-JTAG
    ("gpio", "gpio"),
    ("sio", "gpio"), // RP2040 single-cycle IO block (id/type `sio`) is its GPIO
    ("timg", "timer"),
    ("systimer", "timer"),
    ("tim", "timer"),
    ("gdma", "dma"),
    ("dma", "dma"),
    ("intmatrix", "irq"),
    ("interrupt", "irq"),
    ("nvic", "irq"),
    ("rcc", "clock"),
    ("clk", "clock"),
    ("rtc_cntl", "clock"),
    ("clock", "clock"), // nRF CLOCK block (id `clock` / type `nrf_clock`)
    ("system", "clock"),
    ("i2c", "i2c"),
    ("twi", "i2c"), // Nordic naming: TWI/TWIM/TWIS are the I²C blocks
    ("spi", "spi"),
    ("sar_adc", "adc"),
    ("adc", "adc"),
    ("mcpwm", "pwm"),
    ("ledc", "pwm"),
    ("pwm", "pwm"),
    ("iwdg", "wdt"),
    ("wwdg", "wdt"),
    ("wdt", "wdt"),
    // Deliberately "fdcan", not "can": bxCAN instances (stm32f103
    // `bxcan1`, stm32l476 `can1`) must not declare the class until
    // their fixtures actually check it.
    ("fdcan", "can"),
    // NOTE: "rtc_cntl" -> clock is matched FIRST (listed above); bare "rtc"
    // ids map to the rtc class.
    ("rtc", "rtc"),
    ("rmt", "rmt"),
];

#[derive(Deserialize)]
struct ChipYamlPeripheral {
    id: String,
    #[serde(default)]
    r#type: String,
}

#[derive(Deserialize)]
struct ChipYamlDoc {
    #[serde(default)]
    peripherals: Vec<ChipYamlPeripheral>,
}

/// Which tier1 classes a chip YAML declares, by peripheral heuristics.
///
/// Both the instance `id` and the model `type` are matched: instance ids
/// follow chip-vendor naming that the marker table can't enumerate (`twi1`
/// is the nRF I²C, `clock` the nRF CLOCK), while the `type` field carries
/// the family-qualified model name (`nrf52840_i2c`, `nrf_clock`) that the
/// markers reliably hit. Matching only ids made whole modeled subsystems
/// render as "not modeled" in the public matrix.
pub fn declared_classes_from_yaml(yaml: &str) -> Result<BTreeSet<String>, String> {
    let doc: ChipYamlDoc = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
    let mut classes = BTreeSet::new();
    for p in &doc.peripherals {
        for name in [p.id.to_lowercase(), p.r#type.to_lowercase()] {
            for (marker, class) in CLASS_MARKERS {
                if name.contains(marker) {
                    classes.insert(class.to_string());
                    break;
                }
            }
        }
    }
    Ok(classes)
}

/// Cells whose class is not declared by the chip become `Na`. This deliberately
/// downgrades even Pass cells — heuristic misses surface as `pass -> na` in the
/// ratchet diff rather than silently shadow-passing.
pub fn apply_na(row: &mut BTreeMap<String, Cell>, declared: &BTreeSet<String>) {
    for (class, cell) in row.iter_mut() {
        if !declared.contains(class) {
            cell.status = CellStatus::Na;
            cell.run_url = None;
        }
    }
}

/// Chips that the snapshot records with at least one `pass` cell but which the
/// live run skipped (fixture missing). A deleted fixture must not silently
/// disarm the ratchet — these are reported as regressions by the gate.
pub fn skipped_chips_with_recorded_passes(
    snapshot: &Tier1Matrix,
    skipped: &[String],
) -> Vec<String> {
    skipped
        .iter()
        .filter(|chip| {
            snapshot
                .0
                .get(chip.as_str())
                .is_some_and(|row| row.values().any(|c| c.status == CellStatus::Pass))
        })
        .cloned()
        .collect()
}

/// Cells recorded `Pass` in the snapshot must still pass live. Everything else
/// (partial/blocked/na/unrecorded, chips missing from the live run) moves freely.
pub fn ratchet_regressions(snapshot: &Tier1Matrix, live: &Tier1Matrix) -> Vec<String> {
    let mut out = Vec::new();
    for (chip, row) in &snapshot.0 {
        for (class, snap_cell) in row {
            if snap_cell.status != CellStatus::Pass {
                continue;
            }
            let live_status = live
                .0
                .get(chip)
                .and_then(|r| r.get(class))
                .map(|c| c.status);
            match live_status {
                Some(CellStatus::Pass) | None => {} // None = chip not exercised in this run
                Some(s) => out.push(format!("{chip}/{class}: pass -> {}", s.as_str())),
            }
        }
    }
    out
}

/// One matrix target. Paths are workspace-root-relative.
pub struct Tier1Target {
    pub chip: &'static str,
    pub chip_yaml: &'static str,
    pub elf: &'static str,
    /// Flash image for `--rom-boot` (None = fast-boot ELF entry).
    pub flash_bin: Option<&'static str>,
    pub rom_boot: bool,
    pub max_steps: u64,
    /// Beachhead classes beyond RUBRIC_CLASSES (spec wedge-alignment §4).
    pub extra_classes: &'static [&'static str],
}

/// Shorthand for the common fast-boot, rubric-only target shape.
const fn fast_boot(chip: &'static str, chip_yaml: &'static str, elf: &'static str) -> Tier1Target {
    Tier1Target {
        chip,
        chip_yaml,
        elf,
        flash_bin: None,
        rom_boot: false,
        max_steps: 8_000_000,
        extra_classes: &[],
    }
}

impl Tier1Target {
    /// Beachhead classes on top of a `fast_boot` shape.
    const fn with_extra_classes(mut self, extra: &'static [&'static str]) -> Self {
        self.extra_classes = extra;
        self
    }
}

// One row per SILICON — board variants share their chip's row
// (esp32s3-zero → esp32s3, stm32f401cdu6 → stm32f401).
// Targets whose fixture ELF is not committed yet appear in the matrix as
// full rows of `unrecorded` cells: visible breadth, zero claims.
pub const TIER1_TARGETS: &[Tier1Target] = &[
    Tier1Target {
        chip: "esp32s3",
        chip_yaml: "configs/chips/esp32s3.yaml",
        elf: "tests/fixtures/tier1/esp32s3.elf",
        flash_bin: Some("tests/fixtures/tier1/esp32s3-flash.bin"),
        rom_boot: true,
        // Real ROM + bootloader + app + self-tests. Measured: the full TIER1
        // transcript lands between 16M and 24M steps; 30M = measured + headroom.
        max_steps: 30_000_000,
        extra_classes: &["rmt"],
    },
    fast_boot(
        "esp32",
        "configs/chips/esp32.yaml",
        "tests/fixtures/tier1/esp32.elf",
    ),
    fast_boot(
        "esp32c3",
        "configs/chips/esp32c3.yaml",
        "tests/fixtures/tier1/esp32c3.elf",
    ),
    fast_boot(
        "nrf52832",
        "configs/chips/nrf52832.yaml",
        "tests/fixtures/tier1/nrf52832.elf",
    ),
    fast_boot(
        "nrf52840",
        "configs/chips/nrf52840.yaml",
        "tests/fixtures/tier1/nrf52840.elf",
    ),
    fast_boot(
        "rp2040",
        "configs/chips/rp2040.yaml",
        "tests/fixtures/tier1/rp2040.elf",
    ),
    fast_boot(
        "stm32f103",
        "configs/chips/stm32f103.yaml",
        "tests/fixtures/tier1/stm32f103.elf",
    ),
    fast_boot(
        "stm32f401",
        "configs/chips/stm32f401.yaml",
        "tests/fixtures/tier1/stm32f401.elf",
    ),
    fast_boot(
        "stm32f407",
        "configs/chips/stm32f407.yaml",
        "tests/fixtures/tier1/stm32f407.elf",
    ),
    fast_boot(
        "stm32g474re",
        "configs/chips/stm32g474re.yaml",
        "tests/fixtures/tier1/stm32g474re.elf",
    ),
    fast_boot(
        "stm32h563",
        "configs/chips/stm32h563.yaml",
        "tests/fixtures/tier1/stm32h563.elf",
    )
    .with_extra_classes(&["can"]),
    // First fully-modelled Cortex-M7 chip. H7-family (RM0468); sim-derived
    // (no bench part). Exercises clock/gpio/timer/pwm/i2c/spi/wdt/irq + uart.
    fast_boot(
        "stm32h735",
        "configs/chips/stm32h735.yaml",
        "tests/fixtures/tier1/stm32h735.elf",
    ),
    fast_boot(
        "stm32l073",
        "configs/chips/stm32l073.yaml",
        "tests/fixtures/tier1/stm32l073.elf",
    ),
    fast_boot(
        "stm32l476",
        "configs/chips/stm32l476.yaml",
        "tests/fixtures/tier1/stm32l476.elf",
    ),
    fast_boot(
        "stm32wb55",
        "configs/chips/stm32wb55.yaml",
        "tests/fixtures/tier1/stm32wb55.elf",
    ),
    fast_boot(
        "stm32wba52",
        "configs/chips/stm32wba52.yaml",
        "tests/fixtures/tier1/stm32wba52.elf",
    ),
];

/// Workspace root = two parents up from the cli crate (crates/cli → core).
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

#[derive(Deserialize)]
struct ManifestEntry {
    sha256: String,
}

/// Verify every blob listed in `<dir>/MANIFEST.json` against its sha256.
/// Returns the set of verified file names on success, or Err naming the first
/// mismatching file. The returned set is used by `run_all` to check that every
/// blob a target uses is explicitly listed.
pub fn verify_fixture_manifest(dir: &Path) -> Result<BTreeSet<String>, String> {
    let manifest_path = dir.join("MANIFEST.json");
    let manifest: BTreeMap<String, ManifestEntry> = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?,
    )
    .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    for (file, entry) in &manifest {
        let bytes = std::fs::read(dir.join(file)).map_err(|e| format!("{file}: {e}"))?;
        let got = format!("{:x}", Sha256::digest(&bytes));
        if got != entry.sha256 {
            return Err(format!(
                "{file}: sha256 mismatch (manifest {}, got {got})",
                entry.sha256
            ));
        }
    }
    Ok(manifest.into_keys().collect())
}

/// Run one target through the `labwired` binary and parse its TIER1 row.
/// `labwired_bin` lets integration tests pass `env!("CARGO_BIN_EXE_labwired")`.
pub fn run_target(
    target: &Tier1Target,
    labwired_bin: &Path,
) -> Result<BTreeMap<String, Cell>, String> {
    let root = workspace_root();
    let mut cmd = std::process::Command::new(labwired_bin);
    cmd.arg("run")
        .arg("--chip")
        .arg(root.join(target.chip_yaml))
        .arg("--firmware")
        .arg(root.join(target.elf))
        .arg("--max-steps")
        .arg(target.max_steps.to_string());

    // Scrub any inherited LABWIRED_* vars so the matrix is deterministic
    // regardless of the caller's shell environment, then set only the ones
    // this target actually needs.
    for (key, _) in std::env::vars() {
        if key.starts_with("LABWIRED_") {
            cmd.env_remove(&key);
        }
    }

    if target.rom_boot {
        cmd.arg("--rom-boot");
        let flash = target.flash_bin.ok_or("rom_boot target needs flash_bin")?;
        cmd.env("LABWIRED_ESP32S3_FLASH", root.join(flash));
    }
    let out = cmd
        .output()
        .map_err(|e| format!("spawn {}: {e}", labwired_bin.display()))?;

    // UART echoes on stdout; the sim may exit nonzero on step-limit — that's
    // fine, the protocol lines are the verdict.
    //
    // No wall-clock timeout here: step-count bound is sufficient because the
    // sim step loop has no blocking paths (no I/O waits, no sleeps).
    let parsed = parse_tier1_uart(&out.stdout);

    // A crash (non-zero exit, no TIER1 output, no `done`) must surface as an
    // error rather than silently producing a row of Blocked cells.
    if parsed.classes.is_empty() && !parsed.done && !out.status.success() {
        let stderr_tail = {
            let s = String::from_utf8_lossy(&out.stderr);
            let trimmed = s.trim_end();
            if trimmed.len() > 500 {
                let cut = trimmed.len().saturating_sub(500);
                let cut = (cut..=trimmed.len())
                    .find(|&i| trimmed.is_char_boundary(i))
                    .unwrap_or(trimmed.len());
                trimmed[cut..].to_string()
            } else {
                trimmed.to_string()
            }
        };
        return Err(format!(
            "{}: labwired exited {} with no TIER1 output; stderr tail: {}",
            target.chip, out.status, stderr_tail,
        ));
    }

    let classes: Vec<&str> = RUBRIC_CLASSES
        .iter()
        .chain(target.extra_classes.iter())
        .copied()
        .collect();
    let mut row = parsed.resolve_row(&classes);
    let chip_yaml =
        std::fs::read_to_string(root.join(target.chip_yaml)).map_err(|e| e.to_string())?;
    apply_na(&mut row, &declared_classes_from_yaml(&chip_yaml)?);
    Ok(row)
}

/// Run every target whose fixture blobs exist. Returns the live matrix and the
/// list of skipped chips (missing fixtures — fresh clone or fixtures not landed yet).
pub fn run_all(labwired_bin: &Path) -> Result<(Tier1Matrix, Vec<String>), String> {
    let root = workspace_root();
    let fixture_dir = root.join("tests/fixtures/tier1");

    // Determine which targets have ELF files present.
    let any_elf_present = TIER1_TARGETS.iter().any(|t| root.join(t.elf).exists());

    // If any ELF exists, MANIFEST.json is mandatory and must cover every blob
    // that a non-skipped target uses.
    let verified: Option<BTreeSet<String>> = if any_elf_present {
        let manifest_path = fixture_dir.join("MANIFEST.json");
        if !manifest_path.exists() {
            return Err(format!(
                "MANIFEST.json is required when fixture ELFs are present but was not found at {}",
                manifest_path.display()
            ));
        }
        Some(verify_fixture_manifest(&fixture_dir)?)
    } else {
        None
    };

    // Before running any target, verify that every blob it uses is listed in
    // the manifest — an omitted blob is an error naming the file.
    if let Some(ref listed) = verified {
        for target in TIER1_TARGETS {
            if !root.join(target.elf).exists() {
                continue; // will be skipped below
            }
            let elf_name = Path::new(target.elf)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(target.elf);
            if !listed.contains(elf_name) {
                return Err(format!(
                    "fixture blob '{elf_name}' used by target '{}' is not listed in MANIFEST.json",
                    target.chip
                ));
            }
            if let Some(flash) = target.flash_bin {
                let flash_name = Path::new(flash)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(flash);
                if !listed.contains(flash_name) {
                    return Err(format!(
                        "fixture blob '{flash_name}' used by target '{}' is not listed in MANIFEST.json",
                        target.chip
                    ));
                }
            }
        }
    }

    let mut matrix = Tier1Matrix::default();
    let mut skipped = Vec::new();
    for target in TIER1_TARGETS {
        if !root.join(target.elf).exists() {
            // Planned-but-unfixtured silicon stays VISIBLE: a full row of
            // `unrecorded` cells (breadth without claims) instead of being
            // silently absent. The ratchet ignores unrecorded; the scoreboard
            // and /validation page render `·`.
            skipped.push(target.chip.to_string());
            matrix
                .0
                .insert(target.chip.to_string(), unrecorded_row(target));
            continue;
        }
        let row = run_target(target, labwired_bin)?;
        matrix.0.insert(target.chip.to_string(), row);
    }
    Ok((matrix, skipped))
}

/// Full row of `unrecorded` cells for a target with no committed fixture.
fn unrecorded_row(target: &Tier1Target) -> BTreeMap<String, Cell> {
    RUBRIC_CLASSES
        .iter()
        .chain(target.extra_classes.iter())
        .map(|class| {
            (
                class.to_string(),
                Cell {
                    status: CellStatus::Unrecorded,
                    run_url: None,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_targets_emit_full_unrecorded_rows() {
        let target = &TIER1_TARGETS[1]; // a planned fast-boot target
        let row = unrecorded_row(target);
        assert_eq!(row.len(), RUBRIC_CLASSES.len() + target.extra_classes.len());
        assert!(row
            .values()
            .all(|c| c.status == CellStatus::Unrecorded && c.run_url.is_none()));
    }

    #[test]
    fn parses_pass_fail_lines_and_done() {
        let uart =
            b"boot noise\nTIER1 clock PASS\nTIER1 gpio PASS\nTIER1 dma FAIL code=gdma-idle\nTIER1 done\ntrailing";
        let parsed = parse_tier1_uart(uart);
        assert!(parsed.done);
        assert_eq!(parsed.classes["clock"], CellStatus::Pass);
        assert_eq!(parsed.classes["gpio"], CellStatus::Pass);
        assert_eq!(parsed.classes["dma"], CellStatus::Blocked);
    }

    #[test]
    fn missing_done_marks_row_partial_for_reported_passes() {
        let uart = b"TIER1 clock PASS\nTIER1 gpio PASS\n"; // hung before done
        let parsed = parse_tier1_uart(uart);
        assert!(!parsed.done);
        let row = parsed.resolve_row(&["clock", "gpio", "uart"]);
        // reported passes degrade to partial; unreported classes are blocked
        assert_eq!(row["clock"].status, CellStatus::Partial);
        assert_eq!(row["gpio"].status, CellStatus::Partial);
        assert_eq!(row["uart"].status, CellStatus::Blocked);
    }

    #[test]
    fn no_tier1_lines_blocks_uart_and_everything_else() {
        let parsed = parse_tier1_uart(b"garbage \xff\xfe binary noise");
        assert!(!parsed.done);
        assert!(parsed.classes.is_empty());
        let row = parsed.resolve_row(RUBRIC_CLASSES);
        for class in RUBRIC_CLASSES {
            assert_eq!(row[*class].status, CellStatus::Blocked, "{class}");
        }
    }

    #[test]
    fn garbage_tier1_lines_are_ignored_not_fatal() {
        let uart = b"TIER1 gpio MAYBE\nTIER1\nTIER1 gpio PASS\nTIER1 done\n";
        let parsed = parse_tier1_uart(uart);
        assert_eq!(parsed.classes["gpio"], CellStatus::Pass);
        assert_eq!(parsed.classes.len(), 1);
    }

    #[test]
    fn uart_class_is_implicitly_pass_when_done_arrives() {
        // The fixture never prints "TIER1 uart ..." — receiving the protocol IS the proof.
        let parsed = parse_tier1_uart(b"TIER1 clock PASS\nTIER1 done\n");
        let row = parsed.resolve_row(&["clock", "uart"]);
        assert_eq!(row["uart"].status, CellStatus::Pass);
    }

    #[test]
    fn explicit_uart_fail_wins_over_implicit_rule() {
        let parsed =
            parse_tier1_uart(b"TIER1 clock PASS\nTIER1 uart FAIL code=parity\nTIER1 done\n");
        let row = parsed.resolve_row(&["clock", "uart"]);
        assert_eq!(row["uart"].status, CellStatus::Blocked);
    }

    #[test]
    fn done_alone_proves_uart() {
        let parsed = parse_tier1_uart(b"TIER1 done\n");
        let row = parsed.resolve_row(&["uart", "gpio"]);
        assert_eq!(row["uart"].status, CellStatus::Pass);
        // Unattempted class with done seen: no claim (NOT blocked) — blocked
        // is reserved for explicit FAILs and hung-before-done sequences.
        assert_eq!(row["gpio"].status, CellStatus::Unrecorded);
    }

    #[test]
    fn duplicate_class_lines_last_wins() {
        let parsed = parse_tier1_uart(b"TIER1 gpio PASS\nTIER1 gpio FAIL code=retry\nTIER1 done\n");
        assert_eq!(parsed.classes["gpio"], CellStatus::Blocked);
        // and the reverse: a retry that recovers
        let parsed = parse_tier1_uart(b"TIER1 gpio FAIL code=first\nTIER1 gpio PASS\nTIER1 done\n");
        assert_eq!(parsed.classes["gpio"], CellStatus::Pass);
    }

    #[test]
    fn whitespace_and_crlf_are_tolerated() {
        let parsed = parse_tier1_uart(b"  TIER1\tclock   PASS\r\nTIER1 done\r\n");
        assert_eq!(parsed.classes["clock"], CellStatus::Pass);
        assert!(parsed.done);
    }

    #[test]
    fn derives_na_from_chip_yaml_peripheral_ids() {
        // Minimal chip yaml shape — only `peripherals[].id` matters here.
        let yaml = r#"
name: "fakechip"
arch: "xtensa"
peripherals:
  - { id: "uart0", type: "uart", base_address: 0x60000000 }
  - { id: "gpio", type: "declarative", base_address: 0x60004000 }
  - { id: "timg0", type: "declarative", base_address: 0x6001F000 }
  - { id: "interrupt_core0", type: "declarative", base_address: 0x600C2000 }
"#;
        let declared = declared_classes_from_yaml(yaml).unwrap();
        assert!(declared.contains("uart"));
        assert!(declared.contains("gpio"));
        assert!(declared.contains("timer"));
        assert!(declared.contains("irq"));
        assert!(!declared.contains("dma")); // not declared → n/a, not blocked
        assert!(!declared.contains("mcpwm"));
    }

    #[test]
    fn na_overrides_blocked_in_row_resolution() {
        let parsed = parse_tier1_uart(b"TIER1 clock PASS\nTIER1 done\n");
        let mut row = parsed.resolve_row(RUBRIC_CLASSES);
        let declared: BTreeSet<String> = ["clock", "uart"].iter().map(|s| s.to_string()).collect();
        apply_na(&mut row, &declared);
        assert_eq!(row["clock"].status, CellStatus::Pass);
        assert_eq!(row["dma"].status, CellStatus::Na); // undeclared
        assert_eq!(row["gpio"].status, CellStatus::Na); // undeclared
    }

    fn cell(s: CellStatus) -> Cell {
        Cell {
            status: s,
            run_url: None,
        }
    }

    #[test]
    fn ratchet_flags_pass_regressions_only() {
        let mut snap = Tier1Matrix::default();
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Pass));
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("dma".into(), cell(CellStatus::Blocked));

        let mut live = Tier1Matrix::default();
        live.0
            .entry("esp32s3".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Blocked)); // regression!
        live.0
            .entry("esp32s3".into())
            .or_default()
            .insert("dma".into(), cell(CellStatus::Pass)); // improvement — fine

        let regressions = ratchet_regressions(&snap, &live);
        assert_eq!(
            regressions,
            vec!["esp32s3/gpio: pass -> blocked".to_string()]
        );
    }

    #[test]
    fn ratchet_ignores_unrecorded_and_missing_chips() {
        let mut snap = Tier1Matrix::default();
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Unrecorded));
        let live = Tier1Matrix::default(); // chip absent from live run
        assert!(ratchet_regressions(&snap, &live).is_empty());
    }

    #[test]
    fn snapshot_roundtrip_is_deterministic() {
        let mut m = Tier1Matrix::default();
        m.0.entry("b".into())
            .or_default()
            .insert("z".into(), cell(CellStatus::Pass));
        m.0.entry("a".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Na));
        let j1 = serde_json::to_string_pretty(&m).unwrap();
        let j2 = serde_json::to_string_pretty(&serde_json::from_str::<Tier1Matrix>(&j1).unwrap())
            .unwrap();
        assert_eq!(j1, j2);
        assert!(j1.find("\"a\"").unwrap() < j1.find("\"b\"").unwrap());
    }

    #[test]
    fn skipped_chips_with_passes_detects_disarmed_fixture() {
        // Snapshot has esp32s3 with a pass cell — it gets flagged when skipped.
        let mut snap = Tier1Matrix::default();
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Pass));

        let skipped = vec!["esp32s3".to_string(), "other".to_string()];
        let disarmed = skipped_chips_with_recorded_passes(&snap, &skipped);
        assert_eq!(disarmed, vec!["esp32s3".to_string()]);
    }

    #[test]
    fn skipped_chips_with_only_blocked_cells_not_flagged() {
        // Snapshot has a chip but only blocked/na cells — not a disarmed gate.
        let mut snap = Tier1Matrix::default();
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Blocked));
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("dma".into(), cell(CellStatus::Na));

        let skipped = vec!["esp32s3".to_string()];
        let disarmed = skipped_chips_with_recorded_passes(&snap, &skipped);
        assert!(disarmed.is_empty());
    }

    #[test]
    fn cell_status_as_str_matches_serde() {
        assert_eq!(serde_json::to_string(&CellStatus::Na).unwrap(), "\"na\"");
        assert_eq!(CellStatus::Na.as_str(), "na");
    }

    #[test]
    fn target_table_paths_resolve_relative_to_workspace_root() {
        let root = workspace_root();
        for t in TIER1_TARGETS {
            assert!(
                t.chip_yaml.ends_with(".yaml"),
                "{}: chip_yaml does not end with .yaml",
                t.chip
            );
            assert!(
                root.join(t.chip_yaml).exists(),
                "{}: chip_yaml {} does not exist",
                t.chip,
                t.chip_yaml
            );
        }
        let t = &TIER1_TARGETS[0];
        assert_eq!(t.chip, "esp32s3");
        assert!(t.chip_yaml.ends_with("configs/chips/esp32s3.yaml"));
        assert!(t.elf.ends_with("tests/fixtures/tier1/esp32s3.elf"));
        assert!(t
            .flash_bin
            .unwrap()
            .ends_with("tests/fixtures/tier1/esp32s3-flash.bin"));
    }

    #[test]
    fn manifest_verification_rejects_corrupt_blob() {
        let dir =
            std::env::temp_dir().join(format!("tier1-manifest-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("esp32s3.elf"), b"not the real elf").unwrap();
        let manifest = r#"{ "esp32s3.elf": { "sha256": "0000000000000000000000000000000000000000000000000000000000000000" } }"#;
        std::fs::write(dir.join("MANIFEST.json"), manifest).unwrap();
        let err = verify_fixture_manifest(&dir).unwrap_err();
        assert!(err.contains("esp32s3.elf"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_verification_happy_path() {
        let dir = std::env::temp_dir().join(format!("tier1-manifest-happy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let body = b"good-blob-bytes";
        std::fs::write(dir.join("esp32s3.elf"), body).unwrap();
        let sha = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(body));
        std::fs::write(
            dir.join("MANIFEST.json"),
            format!(r#"{{ "esp32s3.elf": {{ "sha256": "{sha}" }} }}"#),
        )
        .unwrap();
        let verified = verify_fixture_manifest(&dir).unwrap();
        assert!(verified.contains("esp32s3.elf"), "{verified:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_verification_missing_blob_file() {
        let dir =
            std::env::temp_dir().join(format!("tier1-manifest-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // MANIFEST.json references a file that doesn't exist in the dir.
        let manifest = r#"{ "nonexistent.bin": { "sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789" } }"#;
        std::fs::write(dir.join("MANIFEST.json"), manifest).unwrap();
        let err = verify_fixture_manifest(&dir).unwrap_err();
        assert!(err.contains("nonexistent.bin"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_all_style_manifest_listing_is_enforced() {
        // verify_fixture_manifest returns the set of verified file names
        let dir = std::env::temp_dir().join(format!("tier1-manifest-list-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let body = b"blob-bytes";
        std::fs::write(dir.join("esp32s3.elf"), body).unwrap();
        let sha = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(body));
        std::fs::write(
            dir.join("MANIFEST.json"),
            format!(r#"{{ "esp32s3.elf": {{ "sha256": "{sha}" }} }}"#),
        )
        .unwrap();
        let verified = verify_fixture_manifest(&dir).unwrap();
        assert!(verified.contains("esp32s3.elf"));
        assert!(!verified.contains("esp32s3-flash.bin"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_target_surfaces_child_crash_instead_of_blocked_row() {
        let dir = std::env::temp_dir().join(format!("tier1-crash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("labwired-fake");
        std::fs::write(&fake, "#!/bin/sh\necho boom-stderr >&2\nexit 3\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let target = &TIER1_TARGETS[0];
        // chip yaml exists in-repo, ELF path doesn't need to exist for the spawn itself
        let err = run_target(target, &fake).unwrap_err();
        assert!(err.contains("boom-stderr"), "{err}");
        assert!(err.contains("esp32s3"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stderr_tail_truncation_is_char_boundary_safe() {
        let dir = std::env::temp_dir().join(format!("tier1-crash-utf8-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("labwired-fake");
        // >500 bytes of multibyte stderr (U+2744 = 3 bytes each × 200 = 600 bytes)
        // so the naive len-500 cut lands mid-char.
        std::fs::write(&fake, "#!/bin/sh\npython3 << 'PYEOF'\nimport sys\nfor i in range(200):\n    sys.stderr.buffer.write(b'\\xe2\\x9d\\x84')\nsys.exit(3)\nPYEOF\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let target = &TIER1_TARGETS[0];
        let err = run_target(target, &fake).unwrap_err();
        assert!(err.contains("exited"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
