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

/// Branch coverage at one source position: how many times the instruction took
/// a divergent path versus fell through.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BranchCoverage {
    pub file: String,
    pub line: u32,
    pub taken: u64,
    pub not_taken: u64,
}

/// A whole-run statement- and branch-coverage report.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CoverageReport {
    pub files: Vec<FileCoverage>,
    pub total_statements: usize,
    pub covered_statements: usize,
    pub branches: Vec<BranchCoverage>,
    pub total_branches: usize,
    pub covered_branches: usize,
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
            branches: Vec::new(),
            total_branches: 0,
            covered_branches: 0,
        }
    }

    /// Attach branch coverage, sorted deterministically, and roll up totals.
    /// Each branch site contributes two outcomes (taken, not-taken); an outcome
    /// counts as covered when it was observed at least once.
    pub fn set_branches(&mut self, mut branches: Vec<BranchCoverage>) {
        branches.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
        let mut total = 0usize;
        let mut covered = 0usize;
        for b in &branches {
            total += 2;
            if b.taken > 0 {
                covered += 1;
            }
            if b.not_taken > 0 {
                covered += 1;
            }
        }
        self.branches = branches;
        self.total_branches = total;
        self.covered_branches = covered;
    }

    /// Percentage of statements covered (0.0 when there are no statements).
    pub fn statement_percent(&self) -> f64 {
        if self.total_statements == 0 {
            0.0
        } else {
            (self.covered_statements as f64 / self.total_statements as f64) * 100.0
        }
    }

    /// Percentage of branch outcomes covered (0.0 when there are no branches).
    pub fn branch_percent(&self) -> f64 {
        if self.total_branches == 0 {
            0.0
        } else {
            (self.covered_branches as f64 / self.total_branches as f64) * 100.0
        }
    }

    /// Serialise to LCOV `.info` text (statement / line + branch coverage).
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

            // Branch data for this file. Each site emits two outcomes: block 0
            // branch 0 = divergent (taken), branch 1 = fall-through (not taken).
            let mut br_found = 0usize;
            let mut br_hit = 0usize;
            for b in self.branches.iter().filter(|b| b.file == f.file) {
                out.push_str(&format!("BRDA:{},0,0,{}\n", b.line, b.taken));
                out.push_str(&format!("BRDA:{},0,1,{}\n", b.line, b.not_taken));
                br_found += 2;
                if b.taken > 0 {
                    br_hit += 1;
                }
                if b.not_taken > 0 {
                    br_hit += 1;
                }
            }
            if br_found > 0 {
                out.push_str(&format!("BRF:{}\n", br_found));
                out.push_str(&format!("BRH:{}\n", br_hit));
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

    #[test]
    fn branches_roll_up_and_emit_brda() {
        let rows = vec![stmt(0x100, "a.rs", 1), stmt(0x104, "a.rs", 2)];
        let mut report = CoverageReport::build(&rows, |_| true);
        report.set_branches(vec![
            // Fully exercised conditional: both outcomes seen.
            BranchCoverage {
                file: "a.rs".to_string(),
                line: 1,
                taken: 3,
                not_taken: 1,
            },
            // Only ever taken (e.g. an unconditional jump): one outcome covered.
            BranchCoverage {
                file: "a.rs".to_string(),
                line: 2,
                taken: 5,
                not_taken: 0,
            },
        ]);

        assert_eq!(report.total_branches, 4, "two sites, two outcomes each");
        assert_eq!(report.covered_branches, 3, "both + taken-only");
        assert_eq!(report.branch_percent(), 75.0);

        let lcov = report.to_lcov();
        assert!(lcov.contains("BRDA:1,0,0,3\n"));
        assert!(lcov.contains("BRDA:1,0,1,1\n"));
        assert!(lcov.contains("BRDA:2,0,0,5\n"));
        assert!(lcov.contains("BRDA:2,0,1,0\n"));
        assert!(lcov.contains("BRF:4\n"));
        assert!(lcov.contains("BRH:3\n"));
    }
}
