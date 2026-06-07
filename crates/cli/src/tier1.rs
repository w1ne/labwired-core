// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Tier-1 chip × peripheral validation matrix (spec:
//! labwired docs/superpowers/specs/2026-06-07-tier1-chip-matrix-design.md).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    pub classes: BTreeMap<String, CellStatus>,
    /// `TIER1 done` seen — the fixture completed its sequence.
    pub done: bool,
}

/// Parse `TIER1 <class> PASS|FAIL[ code=..]` lines + `TIER1 done` out of a raw
/// UART byte capture. Non-UTF8 and unrelated lines are skipped; malformed
/// `TIER1` lines are ignored (never fatal — boot noise is expected).
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
    /// - `uart` is implicitly Pass once any protocol arrived AND done was seen
    ///   (receiving the lines is the proof), Blocked otherwise.
    /// - missing `done` degrades every reported Pass to Partial (hung mid-sequence);
    /// - classes never reported are Blocked.
    pub fn into_row(&self, classes: &[&str]) -> BTreeMap<String, Cell> {
        let mut row = BTreeMap::new();
        for &class in classes {
            let status = if class == "uart" {
                if self.done && !self.classes.is_empty() {
                    CellStatus::Pass
                } else {
                    CellStatus::Blocked
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

/// peripheral-id substring → tier1 class. First match wins; order matters
/// (e.g. "gdma" must map to dma before "dma" generic).
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
pub fn declared_classes_from_yaml(
    yaml: &str,
) -> Result<std::collections::BTreeSet<String>, String> {
    let doc: ChipYamlDoc = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
    let mut classes = std::collections::BTreeSet::new();
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

/// Cells whose class is not declared by the chip become `Na`.
pub fn apply_na(row: &mut BTreeMap<String, Cell>, declared: &std::collections::BTreeSet<String>) {
    for (class, cell) in row.iter_mut() {
        if !declared.contains(class) {
            cell.status = CellStatus::Na;
            cell.run_url = None;
        }
    }
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
        let row = parsed.into_row(&["clock", "gpio", "uart"]);
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
        let row = parsed.into_row(RUBRIC_CLASSES);
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
        let row = parsed.into_row(&["clock", "uart"]);
        assert_eq!(row["uart"].status, CellStatus::Pass);
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
        let mut row = parsed.into_row(RUBRIC_CLASSES);
        let declared: std::collections::BTreeSet<String> =
            ["clock", "uart"].iter().map(|s| s.to_string()).collect();
        apply_na(&mut row, &declared);
        assert_eq!(row["clock"].status, CellStatus::Pass);
        assert_eq!(row["dma"].status, CellStatus::Na); // undeclared
        assert_eq!(row["gpio"].status, CellStatus::Na); // undeclared
    }
}
