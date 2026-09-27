// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Firmware statement-, branch- and function-coverage report.
//!
//! Maps the set of executed instruction addresses (from the runtime PC-coverage
//! observer, [`labwired_core::pc_coverage::PcCoverageObserver`]) against the
//! DWARF statement rows and the sized function symbols of the firmware ELF,
//! producing a per-file/per-line/per-function report serialisable to LCOV and
//! JSON. This is firmware coverage, distinct from the SVD register-faithfulness
//! `labwired_core::coverage` module.
//!
//! It lives in the loader, not the CLI, because the loader owns the DWARF
//! parser and is a library the CLI, the wasm build and the Python module all
//! link: one report, one set of numbers, on every surface.

use crate::{StmtRow, SymbolProvider};
use labwired_core::pc_coverage::PcCoverageObserver;
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

/// Coverage of one function (a sized ELF function symbol).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FunctionCoverage {
    /// Demangled name.
    pub name: String,
    /// Start address, Thumb bit cleared.
    pub address: u64,
    pub size: u64,
    /// Source file and first line of the function's statements, when DWARF
    /// maps any of its addresses.
    pub file: Option<String>,
    pub line: Option<u32>,
    /// `true` when the instruction at the function's first address executed.
    pub entered: bool,
    /// Distinct `(file, line)` statements inside the function.
    pub lines_found: usize,
    pub lines_hit: usize,
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
    /// Per-function summary, ascending by address. Empty when the ELF carries
    /// no sized function symbols.
    pub functions: Vec<FunctionCoverage>,
    pub total_functions: usize,
    pub covered_functions: usize,
    /// Distinct instruction addresses the run executed.
    pub executed_addresses: usize,
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
            functions: Vec::new(),
            total_functions: 0,
            covered_functions: 0,
            executed_addresses: 0,
        }
    }

    /// The full report for one run: statements, branches resolved to source
    /// lines, and the per-function summary, from the firmware's symbols and
    /// the run's PC-coverage observer.
    pub fn from_run(symbols: &SymbolProvider, observer: &PcCoverageObserver) -> Self {
        let is_executed = |addr: u64| observer.was_executed(addr as u32);
        let mut report = Self::build(symbols.statement_rows(), is_executed);
        // Resolve each observed branch site to its source line. statement_rows
        // uses the line-program file name; lookup() returns the full path, so
        // both are normalised to the basename to attach to the right file.
        let branches = observer
            .branch_sites()
            .into_iter()
            .filter_map(|(src, counts)| {
                let loc = symbols.lookup(u64::from(src))?;
                let line = loc.line?;
                let file = loc.file.rsplit('/').next().unwrap_or(&loc.file).to_string();
                Some(BranchCoverage {
                    file,
                    line,
                    taken: counts.taken,
                    not_taken: counts.not_taken,
                })
            })
            .collect();
        report.set_branches(branches);
        report.set_functions(symbols.functions(), symbols.statement_rows(), is_executed);
        report.executed_addresses = observer.covered_count();
        report
    }

    /// Attach the per-function summary. A function's statements are the
    /// `is_stmt` rows inside `[start, start + size)`.
    pub fn set_functions(
        &mut self,
        functions: &[(u64, u64, String)],
        rows: &[StmtRow],
        is_executed: impl Fn(u64) -> bool,
    ) {
        let mut stmt: Vec<&StmtRow> = rows.iter().filter(|r| r.is_stmt).collect();
        stmt.sort_by_key(|r| r.addr);
        let mut out = Vec::with_capacity(functions.len());
        for (start, size, name) in functions {
            let end = start.saturating_add(*size);
            let from = stmt.partition_point(|r| r.addr < *start);
            let inside = stmt[from..].iter().take_while(|r| r.addr < end);
            let mut lines: BTreeMap<(&str, u32), bool> = BTreeMap::new();
            let mut first: Option<(&str, u32)> = None;
            for r in inside {
                first.get_or_insert((r.file.as_str(), r.line));
                *lines.entry((r.file.as_str(), r.line)).or_insert(false) |= is_executed(r.addr);
            }
            out.push(FunctionCoverage {
                name: addr2line::demangle_auto(name.into(), None).into_owned(),
                address: *start,
                size: *size,
                file: first.map(|(f, _)| f.to_string()),
                line: first.map(|(_, l)| l),
                entered: is_executed(*start),
                lines_found: lines.len(),
                lines_hit: lines.values().filter(|c| **c).count(),
            });
        }
        self.total_functions = out.len();
        self.covered_functions = out.iter().filter(|f| f.entered).count();
        self.functions = out;
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

    /// Percentage of functions entered (0.0 when there are none).
    pub fn function_percent(&self) -> f64 {
        if self.total_functions == 0 {
            0.0
        } else {
            (self.covered_functions as f64 / self.total_functions as f64) * 100.0
        }
    }

    /// Serialise to LCOV `.info` text (function, statement / line and branch
    /// coverage).
    pub fn to_lcov(&self) -> String {
        let mut out = String::new();
        for f in &self.files {
            out.push_str("TN:\n");
            out.push_str(&format!("SF:{}\n", f.file));
            let fns: Vec<&FunctionCoverage> = self
                .functions
                .iter()
                .filter(|func| func.file.as_deref() == Some(f.file.as_str()))
                .collect();
            for func in &fns {
                out.push_str(&format!("FN:{},{}\n", func.line.unwrap_or(0), func.name));
            }
            for func in &fns {
                out.push_str(&format!("FNDA:{},{}\n", u8::from(func.entered), func.name));
            }
            if !fns.is_empty() {
                out.push_str(&format!("FNF:{}\n", fns.len()));
                out.push_str(&format!(
                    "FNH:{}\n",
                    fns.iter().filter(|f| f.entered).count()
                ));
            }
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
