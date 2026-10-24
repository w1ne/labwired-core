// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! LCOV tracefiles: parse, merge, write, and convert to Cobertura XML.
//!
//! This works on any LCOV file, not only ours. That is the point: a team can
//! merge the coverage of many simulator runs with the `gcov`/`llvm-cov`
//! coverage of their host unit tests, and hand one file to CI.
//!
//! Merge rules follow `lcov -a`: line and function counts add up; a branch
//! outcome that was never evaluated (`-`) stays `-` only if it was `-` in
//! every input.
//!
//! Cobertura output targets GitLab's `coverage_report` (`coverage-04.dtd`):
//! every required attribute is present, `<source>` lists the compilation
//! directories so relative `filename`s resolve, and the timestamp is fixed
//! at 0 so the same run always gives the same bytes.

use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// The `TN:` of a merge result. Lines are not attributed to it (it is not a
/// test), and a directory scan uses it to skip earlier merge results.
pub const MERGED_TEST_NAME: &str = "labwired-merged";

/// One branch outcome's key in a file: `BRDA:<line>,[e]<block>,<branch>,…`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BranchKey {
    pub line: u32,
    /// The block is an exception path (`e` prefix, LCOV 2.x).
    pub exception: bool,
    pub block: u32,
    pub branch: BranchId,
}

/// A branch id: a number, or (LCOV 2.x) the source expression it stands for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BranchId {
    Num(u32),
    Expr(String),
}

impl BranchKey {
    /// A plain numbered branch (what LabWired writes).
    pub fn new(line: u32, block: u32, branch: u32) -> Self {
        BranchKey {
            line,
            exception: false,
            block,
            branch: BranchId::Num(branch),
        }
    }
}

impl std::fmt::Display for BranchKey {
    /// `<line>,[e]<block>,<branch>`, as it appears in `BRDA:`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let e = if self.exception { "e" } else { "" };
        match &self.branch {
            BranchId::Num(n) => write!(f, "{},{e}{},{n}", self.line, self.block),
            BranchId::Expr(x) => write!(f, "{},{e}{},{x}", self.line, self.block),
        }
    }
}

/// Coverage of one source file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceFile {
    /// Function name -> (first line, call count).
    pub functions: BTreeMap<String, (u32, u64)>,
    /// Line -> execution count.
    pub lines: BTreeMap<u32, u64>,
    /// Branch outcome -> taken count, `None` when never evaluated.
    pub branches: BTreeMap<BranchKey, Option<u64>>,
    /// Line -> names of the tests (`TN:`) that executed it.
    pub line_tests: BTreeMap<u32, BTreeSet<String>>,
}

/// A whole LCOV tracefile, keyed by the `SF:` path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tracefile {
    pub files: BTreeMap<String, SourceFile>,
}

/// Totals over a tracefile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Summary {
    pub lines_found: usize,
    pub lines_hit: usize,
    pub branches_found: usize,
    pub branches_hit: usize,
    pub functions_found: usize,
    pub functions_hit: usize,
}

fn pct(hit: usize, found: usize) -> f64 {
    if found == 0 {
        0.0
    } else {
        hit as f64 * 100.0 / found as f64
    }
}

impl Summary {
    pub fn line_percent(&self) -> f64 {
        pct(self.lines_hit, self.lines_found)
    }
    pub fn branch_percent(&self) -> f64 {
        pct(self.branches_hit, self.branches_found)
    }
    pub fn function_percent(&self) -> f64 {
        pct(self.functions_hit, self.functions_found)
    }
}

impl SourceFile {
    fn summary(&self) -> Summary {
        Summary {
            lines_found: self.lines.len(),
            lines_hit: self.lines.values().filter(|n| **n > 0).count(),
            branches_found: self.branches.len(),
            branches_hit: self
                .branches
                .values()
                .filter(|n| matches!(n, Some(x) if *x > 0))
                .count(),
            functions_found: self.functions.len(),
            functions_hit: self.functions.values().filter(|(_, n)| *n > 0).count(),
        }
    }

    fn merge(&mut self, other: &SourceFile) {
        for (name, (line, n)) in &other.functions {
            let e = self.functions.entry(name.clone()).or_insert((*line, 0));
            e.1 = e.1.saturating_add(*n);
        }
        for (line, n) in &other.lines {
            let e = self.lines.entry(*line).or_insert(0);
            *e = e.saturating_add(*n);
        }
        for (key, n) in &other.branches {
            let e = self.branches.entry(key.clone()).or_insert(None);
            *e = match (*e, *n) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
            };
        }
        for (line, tests) in &other.line_tests {
            self.line_tests
                .entry(*line)
                .or_default()
                .extend(tests.iter().cloned());
        }
    }
}

fn num<T: std::str::FromStr>(s: &str, what: &str, lineno: usize) -> Result<T> {
    s.trim()
        .parse()
        .ok()
        .with_context(|| format!("line {lineno}: bad {what} '{s}'"))
}

impl Tracefile {
    /// Parse LCOV text. Unknown record types are ignored (LCOV 2.x adds
    /// several), so files from newer tools still load.
    pub fn parse(text: &str) -> Result<Self> {
        let mut out = Tracefile::default();
        let mut test_name = String::new();
        let mut current: Option<(String, SourceFile)> = None;
        // FNL/FNA (LCOV 2.x) give functions an index instead of a name.
        let mut fn_index: BTreeMap<String, (String, u32)> = BTreeMap::new();

        for (i, raw) in text.lines().enumerate() {
            let lineno = i + 1;
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if line == "end_of_record" {
                let Some((path, file)) = current.take() else {
                    bail!("line {lineno}: end_of_record without SF");
                };
                out.files.entry(path).or_default().merge(&file);
                fn_index.clear();
                continue;
            }
            let Some((tag, value)) = line.split_once(':') else {
                continue;
            };
            if tag == "TN" {
                test_name = value.trim().to_string();
                continue;
            }
            if tag == "SF" {
                if current.is_some() {
                    bail!("line {lineno}: SF before end_of_record");
                }
                current = Some((value.trim().to_string(), SourceFile::default()));
                continue;
            }
            let Some((_, file)) = current.as_mut() else {
                continue;
            };
            match tag {
                "FN" => {
                    // FN:<line>[,<end line>],<name>. The name may itself hold
                    // commas (`foo(int, char)`), so only an all-digit second
                    // field is taken as the end line.
                    let (first, rest) = value
                        .split_once(',')
                        .with_context(|| format!("line {lineno}: bad FN '{value}'"))?;
                    let start: u32 = num(first, "FN line", lineno)?;
                    let name = match rest.split_once(',') {
                        Some((end, name))
                            if !end.is_empty() && end.bytes().all(|b| b.is_ascii_digit()) =>
                        {
                            name
                        }
                        _ => rest,
                    };
                    file.functions
                        .entry(name.to_string())
                        .or_insert((start, 0))
                        .0 = start;
                }
                "FNDA" => {
                    let (n, name) = value
                        .split_once(',')
                        .with_context(|| format!("line {lineno}: bad FNDA"))?;
                    let n: u64 = num(n, "FNDA count", lineno)?;
                    let e = file.functions.entry(name.to_string()).or_insert((0, 0));
                    e.1 = e.1.saturating_add(n);
                }
                "FNL" => {
                    // FNL:<index>,<line>[,<end line>]
                    let mut p = value.split(',');
                    let idx = p.next().unwrap_or("").to_string();
                    let start: u32 = num(p.next().unwrap_or(""), "FNL line", lineno)?;
                    fn_index.insert(idx, (String::new(), start));
                }
                "FNA" => {
                    // FNA:<index>,<count>,<name>
                    let p: Vec<&str> = value.splitn(3, ',').collect();
                    if p.len() == 3 {
                        let n: u64 = num(p[1], "FNA count", lineno)?;
                        let start = fn_index.get(p[0]).map(|(_, l)| *l).unwrap_or(0);
                        let e = file.functions.entry(p[2].to_string()).or_insert((start, 0));
                        e.1 = e.1.saturating_add(n);
                    }
                }
                "DA" => {
                    // DA:<line>,<count>[,<checksum>]
                    let mut p = value.split(',');
                    let l: u32 = num(p.next().unwrap_or(""), "DA line", lineno)?;
                    let n: u64 = num(p.next().unwrap_or(""), "DA count", lineno)?;
                    let e = file.lines.entry(l).or_insert(0);
                    *e = e.saturating_add(n);
                    if n > 0 && !test_name.is_empty() && test_name != MERGED_TEST_NAME {
                        file.line_tests
                            .entry(l)
                            .or_default()
                            .insert(test_name.clone());
                    }
                }
                "BRDA" => {
                    // BRDA:<line>,[e]<block>,<branch>,<taken>. LCOV 2.x lets
                    // the branch id be an expression, which may hold commas:
                    // it is everything between the block and the last field.
                    let p: Vec<&str> = value.split(',').collect();
                    if p.len() < 4 {
                        bail!("line {lineno}: bad BRDA '{value}'");
                    }
                    let l: u32 = num(p[0], "BRDA line", lineno)?;
                    let blk = p[1].trim();
                    let exception = blk.starts_with('e');
                    let block: u32 = num(blk.trim_start_matches('e'), "BRDA block", lineno)?;
                    let id = p[2..p.len() - 1].join(",");
                    let branch = match id.trim().parse::<u32>() {
                        Ok(n) => BranchId::Num(n),
                        Err(_) => BranchId::Expr(id),
                    };
                    let taken = match p[p.len() - 1].trim() {
                        "-" => None,
                        t => Some(num::<u64>(t, "BRDA taken", lineno)?),
                    };
                    let key = BranchKey {
                        line: l,
                        exception,
                        block,
                        branch,
                    };
                    let e = file.branches.entry(key).or_insert(None);
                    *e = match (*e, taken) {
                        (None, None) => None,
                        (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
                    };
                }
                _ => {} // LF/LH/BRF/BRH/FNF/FNH are recomputed; others ignored
            }
        }
        if current.is_some() {
            bail!("unterminated record: missing end_of_record");
        }
        Ok(out)
    }

    /// Add `other` into `self` (sums counts, keeps `-` only if `-` in both).
    pub fn merge(&mut self, other: &Tracefile) {
        for (path, file) in &other.files {
            self.files.entry(path.clone()).or_default().merge(file);
        }
    }

    /// Totals over every file.
    pub fn summary(&self) -> Summary {
        let mut s = Summary::default();
        for f in self.files.values() {
            let t = f.summary();
            s.lines_found += t.lines_found;
            s.lines_hit += t.lines_hit;
            s.branches_found += t.branches_found;
            s.branches_hit += t.branches_hit;
            s.functions_found += t.functions_found;
            s.functions_hit += t.functions_hit;
        }
        s
    }

    /// Write LCOV text. `test_name` goes in every record's `TN:`.
    pub fn to_lcov(&self, test_name: &str) -> String {
        let mut out = String::new();
        for (path, f) in &self.files {
            let _ = writeln!(out, "TN:{test_name}");
            let _ = writeln!(out, "SF:{path}");
            let mut fns: Vec<(&String, &(u32, u64))> = f.functions.iter().collect();
            fns.sort_by_key(|(name, (line, _))| (*line, (*name).clone()));
            for (name, (line, _)) in &fns {
                let _ = writeln!(out, "FN:{line},{name}");
            }
            for (name, (_, n)) in &fns {
                let _ = writeln!(out, "FNDA:{n},{name}");
            }
            let s = f.summary();
            if !fns.is_empty() {
                let _ = writeln!(out, "FNF:{}", s.functions_found);
                let _ = writeln!(out, "FNH:{}", s.functions_hit);
            }
            for (key, taken) in &f.branches {
                match taken {
                    Some(n) => {
                        let _ = writeln!(out, "BRDA:{key},{n}");
                    }
                    None => {
                        let _ = writeln!(out, "BRDA:{key},-");
                    }
                }
            }
            if !f.branches.is_empty() {
                let _ = writeln!(out, "BRF:{}", s.branches_found);
                let _ = writeln!(out, "BRH:{}", s.branches_hit);
            }
            for (line, n) in &f.lines {
                let _ = writeln!(out, "DA:{line},{n}");
            }
            let _ = writeln!(out, "LF:{}", s.lines_found);
            let _ = writeln!(out, "LH:{}", s.lines_hit);
            out.push_str("end_of_record\n");
        }
        out
    }

    /// Cobertura XML (coverage-04.dtd) for GitLab / Jenkins / Azure DevOps.
    ///
    /// `sources` are the base directories relative `SF:` paths resolve
    /// against (the DWARF compilation directories, or the repo root).
    pub fn to_cobertura(&self, sources: &[String], version: &str) -> String {
        let total = self.summary();
        let rate = |hit: usize, found: usize| {
            if found == 0 {
                // Nothing to cover counts as fully covered, like gcovr.
                "1".to_string()
            } else {
                format!("{:.4}", hit as f64 / found as f64)
            }
        };

        // Package = directory of the file.
        let mut packages: BTreeMap<String, Vec<(&String, &SourceFile)>> = BTreeMap::new();
        for (path, f) in &self.files {
            let dir = match path.rsplit_once('/') {
                Some((d, _)) => d.to_string(),
                None => ".".to_string(),
            };
            packages.entry(dir).or_default().push((path, f));
        }

        let mut x = String::new();
        x.push_str("<?xml version=\"1.0\" ?>\n");
        x.push_str(
            "<!DOCTYPE coverage SYSTEM \"http://cobertura.sourceforge.net/xml/coverage-04.dtd\">\n",
        );
        let _ = writeln!(
            x,
            "<coverage line-rate=\"{}\" branch-rate=\"{}\" lines-covered=\"{}\" lines-valid=\"{}\" \
             branches-covered=\"{}\" branches-valid=\"{}\" complexity=\"0\" version=\"{}\" \
             timestamp=\"0\">",
            rate(total.lines_hit, total.lines_found),
            rate(total.branches_hit, total.branches_found),
            total.lines_hit,
            total.lines_found,
            total.branches_hit,
            total.branches_found,
            xml_escape(version)
        );
        x.push_str("  <sources>\n");
        if sources.is_empty() {
            x.push_str("    <source>.</source>\n");
        }
        for s in sources {
            let _ = writeln!(x, "    <source>{}</source>", xml_escape(s));
        }
        x.push_str("  </sources>\n  <packages>\n");
        for (dir, files) in &packages {
            let mut ps = Summary::default();
            for (_, f) in files {
                let t = f.summary();
                ps.lines_found += t.lines_found;
                ps.lines_hit += t.lines_hit;
                ps.branches_found += t.branches_found;
                ps.branches_hit += t.branches_hit;
            }
            let _ = writeln!(
                x,
                "    <package name=\"{}\" line-rate=\"{}\" branch-rate=\"{}\" complexity=\"0\">",
                xml_escape(dir),
                rate(ps.lines_hit, ps.lines_found),
                rate(ps.branches_hit, ps.branches_found)
            );
            x.push_str("      <classes>\n");
            for (path, f) in files {
                let t = f.summary();
                let name = path.rsplit('/').next().unwrap_or(path);
                let _ = writeln!(
                    x,
                    "        <class name=\"{}\" filename=\"{}\" line-rate=\"{}\" branch-rate=\"{}\" complexity=\"0\">",
                    xml_escape(name),
                    xml_escape(path),
                    rate(t.lines_hit, t.lines_found),
                    rate(t.branches_hit, t.branches_found)
                );
                // Branch outcomes grouped by line.
                let mut by_line: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
                for (key, n) in &f.branches {
                    let e = by_line.entry(key.line).or_default();
                    e.1 += 1;
                    if matches!(n, Some(v) if *v > 0) {
                        e.0 += 1;
                    }
                }
                x.push_str("          <methods>\n");
                let mut fns: Vec<(&String, &(u32, u64))> = f.functions.iter().collect();
                fns.sort_by_key(|(name, (line, _))| (*line, (*name).clone()));
                for (fname, (fline, calls)) in fns {
                    let _ = writeln!(
                        x,
                        "            <method name=\"{}\" signature=\"\" line-rate=\"{}\" branch-rate=\"1\">\n              <lines>\n                <line number=\"{}\" hits=\"{}\"/>\n              </lines>\n            </method>",
                        xml_escape(fname),
                        if *calls > 0 { "1" } else { "0" },
                        fline,
                        calls
                    );
                }
                x.push_str("          </methods>\n          <lines>\n");
                let mut line_numbers: BTreeSet<u32> = f.lines.keys().copied().collect();
                line_numbers.extend(by_line.keys().copied());
                for line in line_numbers {
                    let hits = f.lines.get(&line).copied().unwrap_or(0);
                    match by_line.get(&line) {
                        Some((hit, found)) => {
                            let _ = writeln!(
                                x,
                                "            <line number=\"{line}\" hits=\"{hits}\" branch=\"true\" condition-coverage=\"{}% ({hit}/{found})\"/>",
                                hit * 100 / found
                            );
                        }
                        None => {
                            let _ = writeln!(
                                x,
                                "            <line number=\"{line}\" hits=\"{hits}\" branch=\"false\"/>"
                            );
                        }
                    }
                }
                x.push_str("          </lines>\n        </class>\n");
            }
            x.push_str("      </classes>\n    </package>\n");
        }
        x.push_str("  </packages>\n</coverage>\n");
        x
    }
}

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "TN:boot\nSF:src/main.c\nFN:3,main\nFN:9,handle\nFNDA:1,main\nFNDA:0,handle\n\
BRDA:4,0,0,2\nBRDA:4,0,1,0\nBRDA:10,0,0,-\nBRDA:10,0,1,-\nDA:3,1\nDA:4,3\nDA:10,0\nLF:3\nLH:2\nend_of_record\n";
    const B: &str = "TN:fault\nSF:src/main.c\nFN:3,main\nFN:9,handle\nFNDA:1,main\nFNDA:2,handle\n\
BRDA:4,0,0,0\nBRDA:4,0,1,1\nBRDA:10,0,0,2\nBRDA:10,0,1,-\nDA:3,1\nDA:4,1\nDA:10,2\nend_of_record\n";

    #[test]
    fn parses_counts_and_never_evaluated_branches() {
        let t = Tracefile::parse(A).unwrap();
        let f = &t.files["src/main.c"];
        assert_eq!(f.lines[&4], 3);
        assert_eq!(f.functions["handle"], (9, 0));
        assert_eq!(f.branches[&BranchKey::new(10, 0, 0)], None);
        assert_eq!(f.branches[&BranchKey::new(4, 0, 0)], Some(2));
        let s = t.summary();
        assert_eq!((s.lines_hit, s.lines_found), (2, 3));
        assert_eq!((s.branches_hit, s.branches_found), (1, 4));
        assert_eq!((s.functions_hit, s.functions_found), (1, 2));
        assert!(f.line_tests[&4].contains("boot"));
    }

    #[test]
    fn merge_sums_like_lcov_add() {
        let mut t = Tracefile::parse(A).unwrap();
        t.merge(&Tracefile::parse(B).unwrap());
        let f = &t.files["src/main.c"];
        assert_eq!(f.lines[&4], 4);
        assert_eq!(f.lines[&10], 2);
        assert_eq!(f.functions["handle"], (9, 2));
        assert_eq!(f.branches[&BranchKey::new(4, 0, 0)], Some(2));
        assert_eq!(f.branches[&BranchKey::new(4, 0, 1)], Some(1));
        assert_eq!(f.branches[&BranchKey::new(10, 0, 0)], Some(2), "- + 2 = 2");
        assert_eq!(f.branches[&BranchKey::new(10, 0, 1)], None, "- + - stays -");
        let s = t.summary();
        assert_eq!((s.lines_hit, s.lines_found), (3, 3));
        assert_eq!((s.branches_hit, s.branches_found), (3, 4));
        let tests: Vec<&str> = f.line_tests[&4].iter().map(String::as_str).collect();
        assert_eq!(tests, ["boot", "fault"], "which tests ran line 4");
    }

    #[test]
    fn merge_is_order_independent() {
        let (a, b) = (Tracefile::parse(A).unwrap(), Tracefile::parse(B).unwrap());
        let mut ab = a.clone();
        ab.merge(&b);
        let mut ba = b.clone();
        ba.merge(&a);
        assert_eq!(ab, ba);
    }

    #[test]
    fn lcov_round_trips() {
        let t = Tracefile::parse(A).unwrap();
        let text = t.to_lcov("");
        assert!(text.contains("BRDA:10,0,0,-\n"));
        assert!(text.contains("BRF:4\nBRH:1\n"));
        let mut back = Tracefile::parse(&text).unwrap();
        // TN is empty on the way back, so per-test attribution is not kept.
        for f in back.files.values_mut() {
            f.line_tests.clear();
        }
        let mut orig = t.clone();
        for f in orig.files.values_mut() {
            f.line_tests.clear();
        }
        assert_eq!(back, orig);
    }

    #[test]
    fn reads_lcov2_function_records_and_ignores_unknown_tags() {
        let text = "SF:a.c\nFNL:0,5,9\nFNA:0,7,init\nVER:abc\nDA:5,7\nend_of_record\n";
        let t = Tracefile::parse(text).unwrap();
        assert_eq!(t.files["a.c"].functions["init"], (5, 7));
    }

    #[test]
    fn rejects_unterminated_record() {
        assert!(Tracefile::parse("SF:a.c\nDA:1,1\n").is_err());
    }

    #[test]
    fn cobertura_has_required_attributes_and_branch_conditions() {
        let t = Tracefile::parse(A).unwrap();
        let x = t.to_cobertura(&["/work/fw".to_string()], "test");
        assert!(x.contains(
            "<coverage line-rate=\"0.6667\" branch-rate=\"0.2500\" lines-covered=\"2\" \
             lines-valid=\"3\" branches-covered=\"1\" branches-valid=\"4\" complexity=\"0\" \
             version=\"test\" timestamp=\"0\">"
        ));
        assert!(x.contains("<source>/work/fw</source>"));
        assert!(x.contains("<package name=\"src\""));
        assert!(x.contains("<class name=\"main.c\" filename=\"src/main.c\""));
        assert!(x.contains(
            "<line number=\"4\" hits=\"3\" branch=\"true\" condition-coverage=\"50% (1/2)\"/>"
        ));
        assert!(x.contains(
            "<line number=\"10\" hits=\"0\" branch=\"true\" condition-coverage=\"0% (0/2)\"/>"
        ));
        assert!(x.contains("<line number=\"3\" hits=\"1\" branch=\"false\"/>"));
        // coverage-04.dtd gives <method> no `complexity` attribute.
        assert!(
            x.contains("<method name=\"handle\" signature=\"\" line-rate=\"0\" branch-rate=\"1\">")
        );
    }

    #[test]
    fn cobertura_escapes_xml() {
        let t = Tracefile::parse("SF:a&b<c>.c\nFN:1,op<>\nFNDA:1,op<>\nDA:1,1\nend_of_record\n")
            .unwrap();
        let x = t.to_cobertura(&[], "v\"1");
        assert!(x.contains("filename=\"a&amp;b&lt;c&gt;.c\""));
        assert!(x.contains("name=\"op&lt;&gt;\""));
        assert!(x.contains("version=\"v&quot;1\""));
        assert!(!x.contains("a&b"));
    }

    /// C++ names hold commas; the end-line field is only taken when numeric.
    #[test]
    fn function_names_with_commas_round_trip() {
        let text = "SF:a.cc\nFN:3,foo(int, char)\nFN:9,12,bar(int, int)\n\
FNDA:2,foo(int, char)\nFNDA:0,bar(int, int)\nDA:3,2\nend_of_record\n";
        let t = Tracefile::parse(text).unwrap();
        let f = &t.files["a.cc"];
        assert_eq!(f.functions.len(), 2, "{:?}", f.functions);
        assert_eq!(f.functions["foo(int, char)"], (3, 2));
        assert_eq!(f.functions["bar(int, int)"], (9, 0));
        let back = Tracefile::parse(&t.to_lcov("")).unwrap();
        assert_eq!(back.files["a.cc"].functions, f.functions);
        assert_eq!(back.summary().functions_found, 2);
    }

    /// Exception blocks and expression branch ids stay distinct and survive
    /// a round trip.
    #[test]
    fn lcov2_branch_ids_round_trip() {
        let text = "SF:a.c\nBRDA:5,0,0,1\nBRDA:5,e0,0,0\nBRDA:5,1,(x > 0) && y,3\n\
BRDA:5,1,(x, y),-\nDA:5,1\nend_of_record\n";
        let t = Tracefile::parse(text).unwrap();
        assert_eq!(t.files["a.c"].branches.len(), 4);
        let out = t.to_lcov("");
        assert!(out.contains("BRDA:5,e0,0,0\n"), "{out}");
        assert!(out.contains("BRDA:5,1,(x > 0) && y,3\n"), "{out}");
        assert!(out.contains("BRDA:5,1,(x, y),-\n"), "{out}");
        assert_eq!(Tracefile::parse(&out).unwrap(), t);
    }
}
