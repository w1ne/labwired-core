// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired coverage`: the register-modeling coverage report (no
//! subcommand) and firmware-coverage tools (`merge`).

use crate::*;
use labwired_loader::lcov::{Summary, Tracefile, MERGED_TEST_NAME};
use std::path::{Path, PathBuf};

pub(crate) fn run_coverage(args: CoverageArgs) -> ExitCode {
    if let Some(action) = args.action {
        return match action {
            CoverageAction::Merge(m) => run_merge(m),
        };
    }
    if let Some(p) = &args.svd {
        std::env::set_var("LABWIRED_ESP32S3_SVD", p);
    }
    match coverage::run() {
        Some((matrix, text)) => {
            print!("{text}");
            if let Some(out) = &args.json_out {
                let json = serde_json::to_string_pretty(&matrix).expect("serialize matrix");
                std::fs::write(out, &json).expect("write json");
                eprintln!("wrote {}", out.display());
            }
            ExitCode::SUCCESS
        }
        None => {
            eprintln!(
                "error: ESP32-S3 SVD not found; set --svd or LABWIRED_ESP32S3_SVD, \
                 or install the espressif32 PlatformIO platform"
            );
            ExitCode::from(EXIT_CONFIG_ERROR)
        }
    }
}

/// Every `*.info` file under `dir`, sorted so the merge order (and therefore
/// nothing about the output) depends on directory listing order.
fn find_info_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            find_info_files(&p, out)?;
        } else if p.extension().is_some_and(|e| e == "info") {
            out.push(p);
        }
    }
    Ok(())
}

/// One uncovered branch outcome, for the report and the JSON.
#[derive(serde::Serialize)]
struct Gap<'a> {
    file: &'a str,
    line: u32,
    /// `never-reached`: the branch instruction never ran.
    /// `jump-never-taken` / `fall-through-never-taken`: it ran, but one
    /// direction never happened (LabWired numbers branch 0 = jump,
    /// 1 = fall-through; other tools' numbering is passed through as
    /// `outcome-N-never-taken`).
    kind: String,
    /// Tests (`TN:`) that ran this line, when the inputs were named.
    tests: Vec<&'a str>,
}

/// One branch's place in a file: (line, exception path, block).
type BlockKey = (u32, bool, u32);
/// One outcome of that branch: its id and taken count (`None` = never ran).
type Outcome<'a> = (&'a labwired_loader::lcov::BranchId, Option<u64>);

fn gaps(t: &Tracefile) -> Vec<Gap<'_>> {
    let mut out = Vec::new();
    for (path, f) in &t.files {
        // Group the outcomes of one branch (same line and block).
        let mut by_branch: std::collections::BTreeMap<BlockKey, Vec<Outcome<'_>>> =
            std::collections::BTreeMap::new();
        for (key, n) in &f.branches {
            by_branch
                .entry((key.line, key.exception, key.block))
                .or_default()
                .push((&key.branch, *n));
        }
        for ((line, _, _), outcomes) in by_branch {
            let tests: Vec<&str> = f
                .line_tests
                .get(&line)
                .map(|s| s.iter().map(String::as_str).collect())
                .unwrap_or_default();
            if outcomes.iter().all(|(_, n)| n.is_none()) {
                out.push(Gap {
                    file: path,
                    line,
                    kind: "never-reached".into(),
                    tests,
                });
                continue;
            }
            for (branch, n) in outcomes {
                if n.unwrap_or(0) == 0 {
                    use labwired_loader::lcov::BranchId;
                    let kind = match branch {
                        BranchId::Num(0) => "jump-never-taken".to_string(),
                        BranchId::Num(1) => "fall-through-never-taken".to_string(),
                        BranchId::Num(b) => format!("outcome-{b}-never-taken"),
                        BranchId::Expr(x) => format!("outcome-{x}-never-taken"),
                    };
                    out.push(Gap {
                        file: path,
                        line,
                        kind,
                        tests: tests.clone(),
                    });
                }
            }
        }
    }
    out
}

fn bar(pct: f64) -> String {
    let filled = ((pct / 100.0) * 20.0).round().clamp(0.0, 20.0) as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(20 - filled))
}

fn print_summary(t: &Tracefile, inputs: usize, s: &Summary) {
    println!(
        "Merged {inputs} coverage file(s), {} source file(s)\n",
        t.files.len()
    );
    let row = |name: &str, hit: usize, found: usize, pct: f64| {
        if found == 0 {
            println!("  {name:<10} {:>20}   n/a", "");
        } else {
            println!("  {name:<10} {} {pct:>5.1}%  {hit}/{found}", bar(pct));
        }
    };
    row("lines", s.lines_hit, s.lines_found, s.line_percent());
    row(
        "branches",
        s.branches_hit,
        s.branches_found,
        s.branch_percent(),
    );
    row(
        "functions",
        s.functions_hit,
        s.functions_found,
        s.function_percent(),
    );

    let g = gaps(t);
    if !g.is_empty() {
        println!("\nBranches your tests never took ({}):", g.len());
        const SHOW: usize = 40;
        for gap in g.iter().take(SHOW) {
            let kind = gap.kind.replace('-', " ");
            let who = if gap.tests.is_empty() {
                String::new()
            } else {
                format!("   (line ran in: {})", gap.tests.join(", "))
            };
            println!("  {}:{:<5} {kind}{who}", gap.file, gap.line);
        }
        if g.len() > SHOW {
            println!("  … and {} more (see --json-out)", g.len() - SHOW);
        }
    }
}

fn run_merge(args: CoverageMergeArgs) -> ExitCode {
    let mut files = Vec::new();
    for input in &args.inputs {
        if input.is_dir() {
            let mut found = Vec::new();
            if let Err(e) = find_info_files(input, &mut found) {
                eprintln!("error: reading {}: {e}", input.display());
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
            // A directory scan must not pick up an earlier merge result
            // (ours, or the file this run is about to write): it would count
            // every run twice.
            let out = args.output.as_ref().and_then(|o| o.canonicalize().ok());
            found.retain(|p| {
                let is_output = out.is_some() && p.canonicalize().ok() == out;
                let is_merge_result = std::fs::read_to_string(p)
                    .map(|t| {
                        t.lines().next().and_then(|l| l.strip_prefix("TN:"))
                            == Some(MERGED_TEST_NAME)
                    })
                    .unwrap_or(false);
                if is_output || is_merge_result {
                    eprintln!("skipping {} (a merge result, not a run)", p.display());
                }
                !is_output && !is_merge_result
            });
            files.extend(found);
        } else {
            files.push(input.clone());
        }
    }
    if files.is_empty() {
        eprintln!("error: no .info files found in the given inputs");
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let mut merged = Tracefile::default();
    for f in &files {
        let text = match std::fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: reading {}: {e}", f.display());
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        };
        match Tracefile::parse(&text) {
            Ok(t) => merged.merge(&t),
            Err(e) => {
                eprintln!("error: {} is not valid LCOV: {e:#}", f.display());
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        }
    }

    let summary = merged.summary();
    print_summary(&merged, files.len(), &summary);

    let write = |path: &Path, body: &str| -> bool {
        match std::fs::write(path, body) {
            Ok(()) => {
                eprintln!("wrote {}", path.display());
                true
            }
            Err(e) => {
                eprintln!("error: writing {}: {e}", path.display());
                false
            }
        }
    };
    let mut ok = true;
    if let Some(out) = &args.output {
        ok &= write(out, &merged.to_lcov(MERGED_TEST_NAME));
    }
    if let Some(out) = &args.cobertura {
        let roots = if args.source_roots.is_empty() {
            vec![".".to_string()]
        } else {
            args.source_roots.clone()
        };
        let version = concat!("labwired ", env!("CARGO_PKG_VERSION"));
        ok &= write(out, &merged.to_cobertura(&roots, version));
    }
    if let Some(out) = &args.json_out {
        let per_file: std::collections::BTreeMap<&str, Summary> = merged
            .files
            .iter()
            .map(|(p, f)| {
                let mut one = Tracefile::default();
                one.files.insert(p.clone(), f.clone());
                (p.as_str(), one.summary())
            })
            .collect();
        let doc = serde_json::json!({
            "inputs": files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "summary": summary,
            "line_percent": summary.line_percent(),
            "branch_percent": summary.branch_percent(),
            "function_percent": summary.function_percent(),
            "files": per_file,
            "uncovered_branches": gaps(&merged),
        });
        ok &= write(
            out,
            &serde_json::to_string_pretty(&doc).expect("serialize merge summary"),
        );
    }
    if !ok {
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    let mut failed = false;
    if let Some(min) = args.fail_under_lines {
        if summary.line_percent() < min {
            eprintln!(
                "FAIL: line coverage {:.1}% is below --fail-under-lines {min}",
                summary.line_percent()
            );
            failed = true;
        }
    }
    if let Some(min) = args.fail_under_branches {
        if summary.branch_percent() < min {
            eprintln!(
                "FAIL: branch coverage {:.1}% is below --fail-under-branches {min}",
                summary.branch_percent()
            );
            failed = true;
        }
    }
    if failed {
        ExitCode::from(EXIT_ASSERT_FAIL)
    } else {
        ExitCode::SUCCESS
    }
}
