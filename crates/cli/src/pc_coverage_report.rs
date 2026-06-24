// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Firmware statement-coverage report.
//!
//! Maps the set of executed instruction addresses (from the runtime PC-coverage
//! observer) against the DWARF statement rows of the firmware ELF, producing a
//! per-file/per-line report serialisable to LCOV and JSON. This is firmware
//! coverage, distinct from the SVD register-faithfulness `coverage` module.

use labwired_loader::StmtRow;
use serde::Serialize;
use std::collections::BTreeMap;

/// Coverage state of a single source line.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LineCoverage {
    pub line: u32,
    pub covered: bool,
}

/// Coverage of one source file.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileCoverage {
    pub file: String,
    pub lines: Vec<LineCoverage>,
    pub lines_found: usize,
    pub lines_hit: usize,
}

/// A whole-run statement-coverage report.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CoverageReport {
    pub files: Vec<FileCoverage>,
    pub total_statements: usize,
    pub covered_statements: usize,
}

impl CoverageReport {
    /// Build a statement-coverage report by mapping DWARF statement rows against
    /// an executed-address predicate. A source line is a statement if any
    /// `is_stmt` row maps to it, and is covered if any of those rows' addresses
    /// were executed. Files and lines are ordered deterministically.
    pub fn build(rows: &[StmtRow], is_executed: impl Fn(u64) -> bool) -> Self {
        // file -> line -> covered
        let mut per_file: BTreeMap<&str, BTreeMap<u32, bool>> = BTreeMap::new();
        for row in rows {
            if !row.is_stmt {
                continue;
            }
            let covered = per_file
                .entry(row.file.as_str())
                .or_default()
                .entry(row.line)
                .or_insert(false);
            if is_executed(row.addr) {
                *covered = true;
            }
        }

        let mut files = Vec::new();
        let mut total = 0usize;
        let mut covered_total = 0usize;
        for (file, lines) in per_file {
            let mut line_cov = Vec::new();
            let mut hit = 0usize;
            for (line, covered) in lines {
                if covered {
                    hit += 1;
                }
                line_cov.push(LineCoverage { line, covered });
            }
            total += line_cov.len();
            covered_total += hit;
            files.push(FileCoverage {
                file: file.to_string(),
                lines_found: line_cov.len(),
                lines_hit: hit,
                lines: line_cov,
            });
        }

        CoverageReport {
            files,
            total_statements: total,
            covered_statements: covered_total,
        }
    }

    /// Percentage of statements covered (0.0 when there are no statements).
    pub fn statement_percent(&self) -> f64 {
        if self.total_statements == 0 {
            0.0
        } else {
            (self.covered_statements as f64 / self.total_statements as f64) * 100.0
        }
    }

    /// Serialise to LCOV `.info` text (statement / line coverage).
    pub fn to_lcov(&self) -> String {
        let mut out = String::new();
        for f in &self.files {
            out.push_str("TN:\n");
            out.push_str(&format!("SF:{}\n", f.file));
            for l in &f.lines {
                out.push_str(&format!(
                    "DA:{},{}\n",
                    l.line,
                    if l.covered { 1 } else { 0 }
                ));
            }
            out.push_str(&format!("LF:{}\n", f.lines_found));
            out.push_str(&format!("LH:{}\n", f.lines_hit));
            out.push_str("end_of_record\n");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stmt(addr: u64, file: &str, line: u32) -> StmtRow {
        StmtRow {
            addr,
            file: file.to_string(),
            line,
            is_stmt: true,
        }
    }

    #[test]
    fn aggregates_per_line_covered_if_any_address_executed() {
        let rows = vec![
            stmt(0x100, "a.rs", 1),
            stmt(0x104, "a.rs", 2),
            stmt(0x108, "a.rs", 2), // same line, second address range
            stmt(0x10c, "b.rs", 5),
        ];
        // Execute 0x100 and 0x108 only.
        let report = CoverageReport::build(&rows, |addr| addr == 0x100 || addr == 0x108);

        assert_eq!(report.total_statements, 3, "a.rs:1, a.rs:2, b.rs:5");
        assert_eq!(
            report.covered_statements, 2,
            "a.rs:1 and a.rs:2 (covered via its second address)"
        );
        let b = report.files.iter().find(|f| f.file == "b.rs").unwrap();
        assert_eq!(b.lines_hit, 0, "b.rs:5 never executed");
    }

    #[test]
    fn non_stmt_rows_are_excluded() {
        let rows = vec![StmtRow {
            addr: 0x100,
            file: "a.rs".to_string(),
            line: 1,
            is_stmt: false,
        }];
        let report = CoverageReport::build(&rows, |_| true);
        assert_eq!(report.total_statements, 0);
    }

    #[test]
    fn emits_lcov_records() {
        let rows = vec![stmt(0x100, "a.rs", 1), stmt(0x104, "a.rs", 2)];
        let report = CoverageReport::build(&rows, |addr| addr == 0x100);
        let lcov = report.to_lcov();

        assert!(lcov.contains("SF:a.rs\n"));
        assert!(lcov.contains("DA:1,1\n"));
        assert!(lcov.contains("DA:2,0\n"));
        assert!(lcov.contains("LF:2\n"));
        assert!(lcov.contains("LH:1\n"));
        assert!(lcov.contains("end_of_record\n"));
        assert_eq!(report.statement_percent(), 50.0);
    }
}
