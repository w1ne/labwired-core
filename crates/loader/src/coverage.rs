// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Firmware statement-, branch- and function-coverage report.
//!
//! Maps one run's execution counts (from the runtime PC-coverage observer,
//! [`labwired_core::pc_coverage::PcCoverageObserver`]) onto the firmware ELF:
//!
//! - **Lines**: DWARF `is_stmt` rows. A line's hit count is the highest
//!   execution count among its rows' addresses.
//! - **Functions**: sized ELF function symbols. A function's call count is
//!   how many times its first instruction ran.
//! - **Branches**: every conditional branch instruction in the ELF, found by
//!   [`crate::branch_sites`], including the ones the run never reached. Each
//!   has two outcomes, "taken" and "not taken", counted from the run's
//!   `(from, to)` steps. A branch the run never reached is written `-` in
//!   LCOV, so it counts against coverage instead of disappearing.
//!
//! The report serialises to JSON, LCOV (through [`crate::lcov`]) and
//! Cobertura XML. This is firmware coverage, distinct from the SVD
//! register-faithfulness `labwired_core::coverage` module.
//!
//! It lives in the loader, not the CLI, because the loader owns the DWARF
//! parser and is a library the CLI, the wasm build and the Python module all
//! link: one report, one set of numbers, on every surface.

use crate::lcov::{SourceFile, Tracefile};
use crate::{StmtRow, SymbolProvider};
use labwired_core::pc_coverage::PcCoverageObserver;
use serde::Serialize;
use std::collections::BTreeMap;

/// Coverage state of a single source line.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LineCoverage {
    pub line: u32,
    pub covered: bool,
    /// Times the line ran (the highest count among its addresses).
    pub hits: u64,
}

/// Coverage of one source file.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileCoverage {
    /// File name as the debug info records it (often bare: `main.c`).
    pub file: String,
    /// Path with its directory (`src/main.c`): unique per file, and what
    /// LCOV `SF:` and Cobertura `filename` use. Relative paths are relative
    /// to the report's `comp_dirs`.
    pub path: String,
    pub lines: Vec<LineCoverage>,
    pub lines_found: usize,
    pub lines_hit: usize,
}

/// One conditional branch instruction and how its two outcomes went.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BranchCoverage {
    pub file: String,
    pub path: String,
    pub line: u32,
    /// Address of the branch instruction.
    pub addr: u64,
    /// Times the condition held and execution jumped.
    pub taken: u64,
    /// Times the condition failed and execution fell through.
    pub not_taken: u64,
    /// Whether the branch instruction ran at all. `false` means neither
    /// outcome was ever evaluated (LCOV `-`).
    pub reached: bool,
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
    pub path: Option<String>,
    pub line: Option<u32>,
    /// `true` when the instruction at the function's first address executed.
    pub entered: bool,
    /// Times the first instruction executed.
    pub calls: u64,
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
    /// Branch *outcomes*: two per conditional branch instruction.
    pub total_branches: usize,
    pub covered_branches: usize,
    /// Instruction set the branches were decoded as (`thumb`, `riscv`), or
    /// `None` when this ELF's ISA has no branch decoder; branch numbers are
    /// then empty rather than wrong.
    pub branch_isa: Option<String>,
    /// Per-function summary, ascending by address. Empty when the ELF carries
    /// no sized function symbols.
    pub functions: Vec<FunctionCoverage>,
    pub total_functions: usize,
    pub covered_functions: usize,
    /// Distinct instruction addresses the run executed.
    pub executed_addresses: usize,
    /// Compilation directories relative `path`s resolve against.
    pub comp_dirs: Vec<String>,
}

/// Demangle C++ (`_Z…`) and Rust (`_ZN…h`, `_R…`) names; leave every other
/// name alone. Handing a plain C name to the demangler is not safe: `f` would
/// come back as `float` and `d` as `double`.
fn demangle(name: &str) -> String {
    if name.starts_with("_Z") || name.starts_with("_R") {
        addr2line::demangle_auto(name.into(), None).into_owned()
    } else {
        name.to_string()
    }
}

impl CoverageReport {
    /// Build a statement-coverage report from DWARF statement rows and an
    /// execution count per address. A source line is a statement if any
    /// `is_stmt` row maps to it; its count is the highest count among those
    /// rows. Files are grouped by full path and ordered deterministically.
    pub fn build(rows: &[StmtRow], hits: impl Fn(u64) -> u64) -> Self {
        // path -> (bare name, line -> hits)
        let mut per_file: BTreeMap<&str, (&str, BTreeMap<u32, u64>)> = BTreeMap::new();
        for row in rows {
            if !row.is_stmt {
                continue;
            }
            let entry = per_file
                .entry(row.path.as_str())
                .or_insert((row.file.as_str(), BTreeMap::new()));
            let n = entry.1.entry(row.line).or_insert(0);
            *n = (*n).max(hits(row.addr));
        }

        let mut files = Vec::new();
        let mut total = 0usize;
        let mut covered_total = 0usize;
        for (path, (file, lines)) in per_file {
            let line_cov: Vec<LineCoverage> = lines
                .into_iter()
                .map(|(line, hits)| LineCoverage {
                    line,
                    covered: hits > 0,
                    hits,
                })
                .collect();
            let hit = line_cov.iter().filter(|l| l.covered).count();
            total += line_cov.len();
            covered_total += hit;
            files.push(FileCoverage {
                file: file.to_string(),
                path: path.to_string(),
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
            branch_isa: None,
            functions: Vec::new(),
            total_functions: 0,
            covered_functions: 0,
            executed_addresses: 0,
            comp_dirs: Vec::new(),
        }
    }

    /// The full report for one run: statements, every conditional branch of
    /// the ELF with its outcomes, and the per-function summary.
    pub fn from_run(symbols: &SymbolProvider, observer: &PcCoverageObserver) -> Self {
        let hits = |addr: u64| observer.hit_count(addr as u32);
        let mut report = Self::build(symbols.statement_rows(), hits);

        let decoded = crate::branch_sites::decode_with_code_ranges(
            symbols.elf_bytes(),
            symbols.line_sequences(),
        );
        report.branch_isa = decoded.isa.map(|i| i.as_str().to_string());
        let branches = decoded
            .sites
            .iter()
            .filter_map(|site| {
                // Branches with no line info (libraries built without -g)
                // can't be shown against source, so they are left out.
                let row = symbols.line_at(site.addr)?;
                let reached = hits(site.addr) > 0;
                Some(BranchCoverage {
                    file: row.file.clone(),
                    path: row.path.clone(),
                    line: row.line,
                    addr: site.addr,
                    taken: observer.edge_count(site.addr as u32, site.target as u32),
                    not_taken: observer.edge_count(site.addr as u32, site.fallthrough() as u32),
                    reached,
                })
            })
            .collect();
        report.set_branches(branches);
        report.set_functions(symbols.functions(), symbols.statement_rows(), hits);
        report.executed_addresses = observer.covered_count();
        report.comp_dirs = symbols.comp_dirs().to_vec();
        report
    }

    /// Attach the per-function summary. A function's statements are the
    /// `is_stmt` rows inside `[start, start + size)`.
    pub fn set_functions(
        &mut self,
        functions: &[(u64, u64, String)],
        rows: &[StmtRow],
        hits: impl Fn(u64) -> u64,
    ) {
        let mut stmt: Vec<&StmtRow> = rows.iter().filter(|r| r.is_stmt).collect();
        stmt.sort_by_key(|r| r.addr);
        let mut out = Vec::with_capacity(functions.len());
        for (start, size, name) in functions {
            let end = start.saturating_add(*size);
            let from = stmt.partition_point(|r| r.addr < *start);
            let inside = stmt[from..].iter().take_while(|r| r.addr < end);
            let mut lines: BTreeMap<(&str, u32), bool> = BTreeMap::new();
            let mut first: Option<&StmtRow> = None;
            for r in inside {
                first.get_or_insert(r);
                *lines.entry((r.path.as_str(), r.line)).or_insert(false) |= hits(r.addr) > 0;
            }
            let calls = hits(*start);
            out.push(FunctionCoverage {
                name: demangle(name),
                address: *start,
                size: *size,
                file: first.map(|r| r.file.clone()),
                path: first.map(|r| r.path.clone()),
                line: first.map(|r| r.line),
                entered: calls > 0,
                calls,
                lines_found: lines.len(),
                lines_hit: lines.values().filter(|c| **c).count(),
            });
        }
        self.total_functions = out.len();
        self.covered_functions = out.iter().filter(|f| f.entered).count();
        self.functions = out;
    }

    /// Attach branch coverage, sorted deterministically, and roll up totals.
    /// Each branch contributes two outcomes (taken, not taken); an outcome is
    /// covered when it was observed at least once.
    pub fn set_branches(&mut self, mut branches: Vec<BranchCoverage>) {
        branches.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then(a.line.cmp(&b.line))
                .then(a.addr.cmp(&b.addr))
        });
        self.total_branches = branches.len() * 2;
        self.covered_branches = branches
            .iter()
            .map(|b| usize::from(b.taken > 0) + usize::from(b.not_taken > 0))
            .sum();
        self.branches = branches;
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

    /// The report as an LCOV model, keyed by full path. Branches on the same
    /// line get block numbers 0, 1, 2, … in address order; branch 0 is
    /// "taken", branch 1 is "not taken".
    pub fn to_tracefile(&self) -> Tracefile {
        let mut t = Tracefile::default();
        for f in &self.files {
            let sf = t.files.entry(f.path.clone()).or_default();
            for l in &f.lines {
                sf.lines.insert(l.line, l.hits);
            }
        }
        for func in &self.functions {
            if let (Some(path), Some(line)) = (&func.path, func.line) {
                let sf = t.files.entry(path.clone()).or_default();
                let e = sf.functions.entry(func.name.clone()).or_insert((line, 0));
                // Aliased symbols (same name twice) add up.
                e.1 = e.1.saturating_add(func.calls);
            }
        }
        let mut block: BTreeMap<(&str, u32), u32> = BTreeMap::new();
        for b in &self.branches {
            let n = block.entry((b.path.as_str(), b.line)).or_insert(0);
            let sf: &mut SourceFile = t.files.entry(b.path.clone()).or_default();
            let (taken, not_taken) = if b.reached {
                (Some(b.taken), Some(b.not_taken))
            } else {
                (None, None)
            };
            sf.branches
                .insert(crate::lcov::BranchKey::new(b.line, *n, 0), taken);
            sf.branches
                .insert(crate::lcov::BranchKey::new(b.line, *n, 1), not_taken);
            *n += 1;
        }
        t
    }

    /// Serialise to LCOV `.info` text (functions, lines and branches, with
    /// execution counts).
    pub fn to_lcov(&self) -> String {
        self.to_tracefile().to_lcov("")
    }

    /// LCOV text with `test_name` in every `TN:` record, so a merged file
    /// still knows which test ran which line.
    pub fn to_lcov_named(&self, test_name: &str) -> String {
        self.to_tracefile().to_lcov(test_name)
    }

    /// Cobertura XML, with the compilation directories as `<source>`s.
    pub fn to_cobertura(&self) -> String {
        self.to_tracefile().to_cobertura(
            &self.comp_dirs,
            concat!("labwired ", env!("CARGO_PKG_VERSION")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stmt(addr: u64, file: &str, line: u32) -> StmtRow {
        StmtRow {
            addr,
            file: file.to_string(),
            path: file.to_string(),
            line,
            is_stmt: true,
        }
    }

    fn branch(line: u32, addr: u64, taken: u64, not_taken: u64, reached: bool) -> BranchCoverage {
        BranchCoverage {
            file: "a.rs".to_string(),
            path: "a.rs".to_string(),
            line,
            addr,
            taken,
            not_taken,
            reached,
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
        let report = CoverageReport::build(&rows, |addr| u64::from(addr == 0x100 || addr == 0x108));

        assert_eq!(report.total_statements, 3, "a.rs:1, a.rs:2, b.rs:5");
        assert_eq!(
            report.covered_statements, 2,
            "a.rs:1 and a.rs:2 (covered via its second address)"
        );
        let b = report.files.iter().find(|f| f.file == "b.rs").unwrap();
        assert_eq!(b.lines_hit, 0, "b.rs:5 never executed");
    }

    #[test]
    fn line_hits_are_the_highest_row_count() {
        let rows = vec![stmt(0x100, "a.rs", 1), stmt(0x110, "a.rs", 1)];
        let report = CoverageReport::build(&rows, |addr| if addr == 0x100 { 1 } else { 9 });
        assert_eq!(report.files[0].lines[0].hits, 9);
        assert!(report.to_lcov().contains("DA:1,9\n"));
    }

    /// Two files with the same bare name in different directories stay apart.
    #[test]
    fn same_name_different_directory_does_not_collide() {
        let mk = |addr, path: &str| StmtRow {
            addr,
            file: "main.c".to_string(),
            path: path.to_string(),
            line: 1,
            is_stmt: true,
        };
        let rows = vec![mk(0x100, "app/main.c"), mk(0x200, "boot/main.c")];
        let report = CoverageReport::build(&rows, |a| u64::from(a == 0x100));
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.total_statements, 2);
        assert_eq!(report.covered_statements, 1);
        let lcov = report.to_lcov();
        assert!(lcov.contains("SF:app/main.c\n"));
        assert!(lcov.contains("SF:boot/main.c\n"));
    }

    #[test]
    fn non_stmt_rows_are_excluded() {
        let mut row = stmt(0x100, "a.rs", 1);
        row.is_stmt = false;
        let report = CoverageReport::build(&[row], |_| 1);
        assert_eq!(report.total_statements, 0);
    }

    #[test]
    fn emits_lcov_records() {
        let rows = vec![stmt(0x100, "a.rs", 1), stmt(0x104, "a.rs", 2)];
        let report = CoverageReport::build(&rows, |addr| u64::from(addr == 0x100));
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
        let mut report = CoverageReport::build(&rows, |_| 1);
        report.set_branches(vec![
            // Both outcomes seen.
            branch(1, 0x100, 3, 1, true),
            // Ran, but only ever jumped.
            branch(2, 0x104, 5, 0, true),
            // Never reached: both outcomes unevaluated.
            branch(2, 0x106, 0, 0, false),
        ]);

        assert_eq!(report.total_branches, 6, "three sites, two outcomes each");
        assert_eq!(report.covered_branches, 3);
        assert_eq!(report.branch_percent(), 50.0);

        let lcov = report.to_lcov();
        assert!(lcov.contains("BRDA:1,0,0,3\n"));
        assert!(lcov.contains("BRDA:1,0,1,1\n"));
        assert!(lcov.contains("BRDA:2,0,0,5\n"));
        assert!(lcov.contains("BRDA:2,0,1,0\n"));
        assert!(
            lcov.contains("BRDA:2,1,0,-\n"),
            "unreached branch is '-': {lcov}"
        );
        assert!(lcov.contains("BRDA:2,1,1,-\n"));
        assert!(lcov.contains("BRF:6\n"));
        assert!(lcov.contains("BRH:3\n"));
    }

    #[test]
    fn functions_carry_call_counts_into_lcov() {
        let rows = vec![stmt(0x100, "a.rs", 3), stmt(0x104, "a.rs", 4)];
        let mut report = CoverageReport::build(&rows, |a| if a == 0x100 { 4 } else { 0 });
        report.set_functions(
            &[(0x100, 8, "f".to_string()), (0x200, 4, "g".to_string())],
            &rows,
            |a| if a == 0x100 { 4 } else { 0 },
        );
        assert_eq!(report.functions[0].calls, 4);
        assert!(!report.functions[1].entered);
        let lcov = report.to_lcov();
        assert!(
            lcov.contains("FN:3,f\n"),
            "a C name is not demangled: {lcov}"
        );
        assert!(lcov.contains("FNDA:4,f\n"));
        // g has no line info, so it can't be placed in a file.
        assert!(!lcov.contains(",g\n"));
    }

    #[test]
    fn lcov_output_parses_back_to_the_same_totals() {
        let rows = vec![stmt(0x100, "a.rs", 1), stmt(0x104, "a.rs", 2)];
        let mut report = CoverageReport::build(&rows, |a| u64::from(a == 0x100));
        report.set_branches(vec![branch(1, 0x100, 1, 0, true)]);
        let t = crate::lcov::Tracefile::parse(&report.to_lcov()).unwrap();
        let s = t.summary();
        assert_eq!(s.lines_found, report.total_statements);
        assert_eq!(s.lines_hit, report.covered_statements);
        assert_eq!(s.branches_found, report.total_branches);
        assert_eq!(s.branches_hit, report.covered_branches);
    }

    #[test]
    fn only_mangled_names_are_demangled() {
        assert_eq!(demangle("f"), "f");
        assert_eq!(demangle("d"), "d");
        assert_eq!(demangle("_Z3fooi"), "foo(int)");
    }
}
