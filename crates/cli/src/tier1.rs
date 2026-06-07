// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Tier-1 chip × peripheral validation matrix (spec:
//! labwired docs/superpowers/specs/2026-06-07-tier1-chip-matrix-design.md).

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

/// The six rubric classes every chip reports.
pub const RUBRIC_CLASSES: &[&str] = &["clock", "gpio", "uart", "timer", "dma", "irq"];

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
                out.classes.insert(class.to_string(), CellStatus::Pass);
            }
            (Some(class), Some("FAIL")) => {
                out.classes.insert(class.to_string(), CellStatus::Blocked);
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
    ///   mid-sequence).
    /// - Classes never reported are Blocked.
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
                    None => CellStatus::Blocked,
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

/// peripheral-id substring → tier1 class. First match wins; currently no pair
/// is order-sensitive — keep it that way or document the pair explicitly if one
/// is added.
const CLASS_MARKERS: &[(&str, &str)] = &[
    ("uart", "uart"),
    ("usb_serial", "uart"), // S3 console can be USB-Serial-JTAG
    ("gpio", "gpio"),
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
    ("system", "clock"),
    ("mcpwm", "mcpwm"),
    ("i2c", "i2c"),
    ("rmt", "rmt"),
];

#[derive(Deserialize)]
struct ChipYamlPeripheral {
    id: String,
}

#[derive(Deserialize)]
struct ChipYamlDoc {
    #[serde(default)]
    peripherals: Vec<ChipYamlPeripheral>,
}

/// Which tier1 classes a chip YAML declares, by peripheral-id heuristics.
pub fn declared_classes_from_yaml(yaml: &str) -> Result<BTreeSet<String>, String> {
    let doc: ChipYamlDoc = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
    let mut classes = BTreeSet::new();
    for p in &doc.peripherals {
        let id = p.id.to_lowercase();
        for (marker, class) in CLASS_MARKERS {
            if id.contains(marker) {
                classes.insert(class.to_string());
                break;
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

pub const TIER1_TARGETS: &[Tier1Target] = &[
    Tier1Target {
        chip: "esp32s3",
        chip_yaml: "configs/chips/esp32s3.yaml",
        elf: "tests/fixtures/tier1/esp32s3.elf",
        flash_bin: Some("tests/fixtures/tier1/esp32s3-flash.bin"),
        rom_boot: true,
        max_steps: 40_000_000, // real ROM + bootloader + app + self-tests
        extra_classes: &["mcpwm", "i2c", "rmt"],
    },
    Tier1Target {
        chip: "esp32s3-zero",
        chip_yaml: "configs/chips/esp32s3-zero.yaml",
        elf: "tests/fixtures/tier1/esp32s3.elf", // same silicon, same fixture
        flash_bin: Some("tests/fixtures/tier1/esp32s3-flash.bin"),
        rom_boot: true,
        max_steps: 40_000_000,
        extra_classes: &["mcpwm", "i2c", "rmt"],
    },
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
/// Returns Err naming the first mismatching file.
pub fn verify_fixture_manifest(dir: &Path) -> Result<(), String> {
    let manifest_path = dir.join("MANIFEST.json");
    let manifest: BTreeMap<String, ManifestEntry> = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?,
    )
    .map_err(|e| e.to_string())?;
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
    Ok(())
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
    if target.rom_boot {
        cmd.arg("--rom-boot");
        let flash = target.flash_bin.ok_or("rom_boot target needs flash_bin")?;
        cmd.env("LABWIRED_ESP32S3_FLASH", root.join(flash));
    }
    let out = cmd.output().map_err(|e| format!("spawn labwired: {e}"))?;
    // UART echoes on stdout; the sim may exit nonzero on step-limit — that's
    // fine, the protocol lines are the verdict.
    let parsed = parse_tier1_uart(&out.stdout);
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
    if fixture_dir.join("MANIFEST.json").exists() {
        verify_fixture_manifest(&fixture_dir)?;
    }
    let mut matrix = Tier1Matrix::default();
    let mut skipped = Vec::new();
    for target in TIER1_TARGETS {
        if !root.join(target.elf).exists() {
            skipped.push(target.chip.to_string());
            continue;
        }
        let row = run_target(target, labwired_bin)?;
        matrix.0.insert(target.chip.to_string(), row);
    }
    Ok((matrix, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(row["gpio"].status, CellStatus::Blocked);
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
    fn cell_status_as_str_matches_serde() {
        assert_eq!(serde_json::to_string(&CellStatus::Na).unwrap(), "\"na\"");
        assert_eq!(CellStatus::Na.as_str(), "na");
    }

    #[test]
    fn target_table_paths_resolve_relative_to_workspace_root() {
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
        let dir = std::env::temp_dir().join("tier1-manifest-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("esp32s3.elf"), b"not the real elf").unwrap();
        let manifest = r#"{ "esp32s3.elf": { "sha256": "0000000000000000000000000000000000000000000000000000000000000000" } }"#;
        std::fs::write(dir.join("MANIFEST.json"), manifest).unwrap();
        let err = verify_fixture_manifest(&dir).unwrap_err();
        assert!(err.contains("esp32s3.elf"), "{err}");
    }
}
