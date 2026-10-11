// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! THE CONTRACT: no test target in this workspace may report success while
//! executing zero tests.
//!
//! # The failure
//!
//! A libtest binary that contains no tests prints
//!
//! ```text
//! running 0 tests
//! test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
//! ```
//!
//! and **exits 0**. To CI, and to a human reading a green tick, that is
//! indistinguishable from a suite that ran and passed. This repo sells a
//! simulator as a hardware oracle — "it said PASS" has to mean something ran.
//!
//! The way a target gets into that state here is an inner attribute:
//! `crates/core/tests/board_batch_width.rs` opens with
//! `#![cfg(feature = "event-scheduler")]`, so without the feature the whole
//! file compiles to nothing. `cargo test -p labwired-core --test
//! board_batch_width` printed `0 passed` and exited 0 while three real tests
//! sat in the file. `census_probe` (`silent-census`, 3 tests) did the same.
//! Both were noticed by accident, not by a gate. The workspace had **56** test
//! targets in that state across `labwired-core`, `labwired-cli` and
//! `labwired-hw-oracle`.
//!
//! # The fix this gate enforces
//!
//! Cargo already has the right mechanism: a `[[test]]` block with
//! `required-features`. With it, cargo **skips** the target when the feature is
//! off — no binary, no `0 passed` line, nothing to misread — and turns an
//! explicit `--test <name>` without the feature into a hard error:
//!
//! ```text
//! error: target `board_batch_width` in package `labwired-core` requires the
//! features: `event-scheduler`
//! ```
//!
//! So the rule is: **an integration test file gated by an inner
//! `#![cfg(feature = ...)]` must carry a matching `required-features`
//! declaration in its crate's manifest.** This test reads both sides and fails
//! when they disagree. It is static — no firmware, no fixtures, milliseconds —
//! and it runs in `cargo test -p labwired-core --lib`, which every PR lane
//! already runs.
//!
//! # How this differs from `scheduler_lane_coverage`
//!
//! That file asks a different question: "does a merge-gating lane RUN this
//! scheduler test at all?" It is about `core-ci.yml`. This one asks "can this
//! target ever report green without running?" and is about the manifests. A
//! target can satisfy one and fail the other, and both failures have shipped.
//!
//! # What this gate does NOT cover
//!
//! * A target whose every test is `#[ignore]`d executes nothing in a normal
//!   lane. `required-features` cannot express that, and this repo has lanes
//!   that legitimately run such files with `-- --ignored`. Those are reasoned
//!   about by name in `NIGHTLY_ONLY` in `scheduler_lane_coverage.rs`.
//! * A test that runs but returns early on a missing env var. It executes and
//!   reports honestly; that is a coverage question, not a false green.
//! * The trailing `test result: ok. 0 passed` that `cargo test -p <crate>`
//!   prints for a crate with **no doc examples**. That line is the *doctest*
//!   target and is legitimate — `cargo test -p labwired-config` really runs
//!   107 tests and then prints that zero. Reading it as the whole verdict is
//!   how this class gets misdiagnosed in the other direction.
//!
//! The runtime half lives in `scripts/ci/cargo-test-nonvacuous.sh`, which asks
//! each test binary a CI step builds how many tests it holds and fails on zero.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Feature-gated integration tests allowed to keep reporting green with
/// zero tests. **Empty, and meant to stay that way** — `required-features`
/// works for every gate cargo can express, so an entry here is a claim that
/// cargo cannot express this one.
///
/// SHRINK-ONLY, content-keyed on `(package, target)`. Each entry needs a
/// reason someone can argue with; `allowlist_entries_are_real_and_still_needed`
/// deletes the excuse the moment the target stops needing it.
const ALLOWED_VACUOUS: &[(&str, &str, &str)] = &[];

/// Gated files whose `#![cfg(...)]` ALSO carries `not(debug_assertions)`.
/// `required-features` covers the feature half and cannot cover the other:
/// in a debug build with the feature ON these still compile to nothing.
/// That residue is deliberate — they are release-only by construction — so
/// each one is named here with where it actually runs.
const RELEASE_ONLY: &[(&str, &str)] = &[
    (
        "world_esp32c3_ble_pong",
        "Release-only by construction (it plays a two-node BLE election out \
         over ~180M cycles; debug is minutes per assertion). It RUNS in \
         core-ci.yml's `--release ... --features event-scheduler` step and \
         in core-nightly.yml. A debug lane could not run it at any price.",
    ),
    (
        "esp32c3_pms_shipped_firmware",
        "Release-only by construction: it boots the shipped PMS firmware on \
         the C3 mask ROM, which is minutes in debug. It RUNS in core-ci.yml's \
         `--release ... --features event-scheduler` step.",
    ),
    (
        "esp32c3_ble_pong_perf_probe",
        "Release-only AND every test in it is #[ignore]d — it is a perf probe \
         whose wall clock is the measurement, so a lane that ran it \
         automatically would be measuring the runner, not the engine. It is \
         invoked by hand with `-- --ignored`; no lane claims to cover it.",
    ),
];

// ── Anti-vacuity floors ─────────────────────────────────────────────────
// A gate against suites that check nothing must not be a suite that checks
// nothing. These are the numbers the scan sees on the tree that introduced
// it (12 crates, 267 files, 57 gated). They are floors, not equalities, so
// normal growth does not trip them — but a mis-rooted path, a bad
// extension filter or a `tests/` layout change collapses the scanned set to
// ~0 and fails LOUDLY instead of passing over an empty set.
const MIN_CRATES_WITH_TESTS: usize = 8;
const MIN_TEST_FILES: usize = 200;
const MIN_GATED_FILES: usize = 40;

#[derive(Debug)]
struct GatedTest {
    package: String,
    manifest: PathBuf,
    /// Cargo target name = the file stem.
    target: String,
    /// Features named inside the inner `#![cfg(...)]`, sorted and deduped.
    features: Vec<String>,
    /// The attribute line itself, for the failure message.
    attr: String,
    /// The `#![cfg(...)]` also carries `not(debug_assertions)`.
    release_only: bool,
}

struct Scan {
    gated: Vec<GatedTest>,
    crates_with_tests: usize,
    test_files: usize,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Every directory under the repo that holds both a `Cargo.toml` and a
/// `tests/` directory. Walked rather than hard-coded so a crate added
/// later is covered without anyone remembering to list it.
fn crates_with_test_dirs(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > 4 {
            return;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            // Build output, vendored source and VCS internals are not ours
            // and would make the walk enormous.
            if matches!(
                name,
                "target" | ".git" | "node_modules" | "third_party" | ".github"
            ) {
                continue;
            }
            if is_package_with_tests(&path) {
                out.push(path.clone());
            }
            walk(&path, depth + 1, out);
        }
    }
    /// A directory counts only if it is a real package (the workspace root
    /// manifest is virtual — `[workspace]`, no `[package]` — and the repo's
    /// top-level `tests/` directory holds fixtures, not cargo targets).
    fn is_package_with_tests(dir: &Path) -> bool {
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() || !dir.join("tests").is_dir() {
            return false;
        }
        std::fs::read_to_string(&manifest)
            .map(|src| src.lines().any(|l| l.trim() == "[package]"))
            .unwrap_or(false)
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out.sort();
    out.dedup();
    out
}

fn package_name(manifest_src: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest_src.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') && !t.contains('=') {
            in_package = t == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = t.strip_prefix("name") {
                if let Some(v) = first_quoted(rest) {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn first_quoted(s: &str) -> Option<String> {
    let start = s.find('"')? + 1;
    let end = s[start..].find('"')? + start;
    Some(s[start..end].to_string())
}

fn quoted_items(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        match after.find('"') {
            Some(end) => {
                out.push(after[..end].to_string());
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// `[[test]]` blocks in a manifest, as `target name -> required-features`.
/// A target declared without `required-features` maps to an empty vector.
fn declared_test_targets(manifest_src: &str) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let mut in_block = false;
    let mut name: Option<String> = None;
    let mut required: Vec<String> = Vec::new();
    let mut array_open = false;

    let mut flush = |name: &mut Option<String>, required: &mut Vec<String>| {
        if let Some(n) = name.take() {
            out.insert(n, std::mem::take(required));
        } else {
            required.clear();
        }
    };

    for line in manifest_src.lines() {
        let t = line.trim();

        if array_open {
            required.extend(quoted_items(t));
            if t.contains(']') {
                array_open = false;
            }
            continue;
        }

        // A table header — `[[test]]`, `[dev-dependencies]`, `[features]`.
        // `required-features = [..]` also starts with a bracket further in,
        // but not at the start, and it contains `=`.
        if t.starts_with('[') && t.ends_with(']') && !t.contains('=') {
            if in_block {
                flush(&mut name, &mut required);
            }
            in_block = t == "[[test]]";
            continue;
        }
        if !in_block || t.starts_with('#') {
            continue;
        }
        if let Some(rest) = t.strip_prefix("name") {
            if rest.trim_start().starts_with('=') {
                name = first_quoted(rest);
            }
        } else if let Some(rest) = t.strip_prefix("required-features") {
            if rest.trim_start().starts_with('=') {
                required.extend(quoted_items(rest));
                if !rest.contains(']') {
                    array_open = true;
                }
            }
        }
    }
    if in_block {
        flush(&mut name, &mut required);
    }
    out
}

fn scan() -> Scan {
    let root = repo_root();
    let mut gated = Vec::new();
    let mut test_files = 0usize;
    let crates = crates_with_test_dirs(&root);

    for crate_dir in &crates {
        let manifest = crate_dir.join("Cargo.toml");
        let manifest_src = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
        let package = package_name(&manifest_src).unwrap_or_else(|| {
            panic!("no [package] name in {}", manifest.display());
        });

        let tests_dir = crate_dir.join("tests");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&tests_dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", tests_dir.display()))
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("rs"))
            .collect();
        files.sort();

        for path in files {
            test_files += 1;
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let target = path
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("test file stem")
                .to_string();

            for line in src.lines() {
                let t = line.trim();
                if !t.starts_with("#![cfg(") {
                    continue;
                }
                // A multi-line inner attribute would make the rest of this
                // scan read half a condition. Nothing in the tree does it;
                // say so rather than guess.
                assert!(
                    t.ends_with(")]"),
                    "{}: inner `#![cfg(...)]` spans multiple lines, which this \
                     scan cannot read. Put it on one line.",
                    path.display()
                );
                let features = features_named(t);
                if features.is_empty() {
                    continue;
                }
                assert!(
                    !t.contains("any("),
                    "{}: `#![cfg(any(feature = ...))]` cannot be expressed by \
                     cargo's `required-features`, which is an AND. Split the \
                     file, or add it to ALLOWED_VACUOUS with a reason.",
                    path.display()
                );
                gated.push(GatedTest {
                    package: package.clone(),
                    manifest: manifest.clone(),
                    target: target.clone(),
                    features,
                    attr: t.to_string(),
                    release_only: t.contains("not(debug_assertions)"),
                });
                break;
            }
        }
    }

    Scan {
        gated,
        crates_with_tests: crates.len(),
        test_files,
    }
}

/// Cargo features named inside an attribute line, sorted and deduped.
fn features_named(attr: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    let mut rest = attr;
    while let Some(pos) = rest.find("feature") {
        rest = &rest[pos + "feature".len()..];
        let trimmed = rest.trim_start();
        if !trimmed.starts_with('=') {
            continue;
        }
        if let Some(v) = first_quoted(trimmed) {
            out.insert(v);
        }
    }
    out.into_iter().collect()
}

/// The scan must be looking at the repository it claims to be looking at.
/// A wrong root, a renamed `tests/` layout or a broken extension filter all
/// present as "nothing to report", which is exactly the shape of bug this
/// file exists to make impossible.
#[test]
fn the_scan_is_not_vacuous() {
    let scan = scan();
    // Printed, not just asserted: run this with `-- --nocapture` and the log
    // says what the scan actually saw. A gate whose log cannot distinguish
    // "swept 267 files" from "swept nothing" is the bug it is guarding.
    println!(
        "no_vacuous_test_targets: {} crates with tests/, {} integration test \
         files, {} feature-gated",
        scan.crates_with_tests,
        scan.test_files,
        scan.gated.len()
    );
    assert!(
        scan.crates_with_tests >= MIN_CRATES_WITH_TESTS,
        "found only {} crates with a tests/ directory (expected >= {}). The \
         walk is looking in the wrong place, not at a repo that lost its \
         integration tests.",
        scan.crates_with_tests,
        MIN_CRATES_WITH_TESTS
    );
    assert!(
        scan.test_files >= MIN_TEST_FILES,
        "found only {} integration test files (expected >= {}). See above — \
         an empty scan must fail, never pass.",
        scan.test_files,
        MIN_TEST_FILES
    );
    assert!(
        scan.gated.len() >= MIN_GATED_FILES,
        "found only {} feature-gated integration test files (expected >= {}). \
         Either the inner-attribute detection broke, or the files moved.",
        scan.gated.len(),
        MIN_GATED_FILES
    );

    // Named sentinels, so a scan that "finds things" but not the RIGHT
    // things still fails. board_batch_width is the target that was found
    // reporting green with 3 tests unrun; the other two prove the walk
    // reaches past labwired-core into the other two affected crates.
    for (pkg, target) in [
        ("labwired-core", "board_batch_width"),
        ("labwired-cli", "e2e_esp32c3_ble_two_node"),
        ("labwired-hw-oracle", "nrf52_mmio_diff"),
    ] {
        assert!(
            scan.gated
                .iter()
                .any(|g| g.package == pkg && g.target == target),
            "the scan did not find the known feature-gated target {pkg}/{target}. \
             It is not measuring what it claims to measure."
        );
    }
}

/// The gate itself.
#[test]
fn every_feature_gated_test_target_declares_required_features() {
    let scan = scan();
    let excused: BTreeSet<(&str, &str)> =
        ALLOWED_VACUOUS.iter().map(|(p, t, _)| (*p, *t)).collect();

    let mut manifests: BTreeMap<PathBuf, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    let mut failures = Vec::new();

    for g in &scan.gated {
        if excused.contains(&(g.package.as_str(), g.target.as_str())) {
            continue;
        }
        let declared = manifests.entry(g.manifest.clone()).or_insert_with(|| {
            let src = std::fs::read_to_string(&g.manifest).expect("read manifest");
            declared_test_targets(&src)
        });

        match declared.get(&g.target) {
            None => failures.push(format!(
                "{}/{}: {}\n      no `[[test]] name = \"{}\"` block in {}",
                g.package,
                g.target,
                g.attr,
                g.target,
                g.manifest.display()
            )),
            Some(req) => {
                let req_set: BTreeSet<&str> = req.iter().map(String::as_str).collect();
                let cfg_set: BTreeSet<&str> = g.features.iter().map(String::as_str).collect();
                if req_set != cfg_set {
                    failures.push(format!(
                        "{}/{}: {}\n      required-features = {:?} but the cfg names {:?}",
                        g.package, g.target, g.attr, req, g.features
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} integration test target(s) can report `test result: ok. 0 passed` \
         and exit 0:\n\n    {}\n\n\
         Built without the feature the file compiles to an EMPTY libtest \
         binary, which is indistinguishable from a suite that ran and passed \
         — that is how `board_batch_width` (3 tests) and `census_probe` \
         (3 tests) sat green in CI.\n\n\
         Fix it in the crate's Cargo.toml:\n\n    \
         [[test]]\n    name = \"<target>\"\n    required-features = [\"<feature>\"]\n\n\
         Cargo then SKIPS the target when the feature is off — no binary and \
         no `0 passed` line — and turns an explicit `--test <target>` without \
         the feature into a hard error instead of a green tick. Do not fix \
         this by deleting the tests, and do not fix it by removing the \
         `#![cfg]`: the feature exists for a reason.",
        failures.len(),
        failures.join("\n    ")
    );
}

/// `required-features` cannot express `not(debug_assertions)`, so a file
/// gated on both is still empty in a debug build with the feature on. Each
/// one must say, in writing, where it really runs.
#[test]
fn release_only_gated_tests_are_named_with_a_reason() {
    let scan = scan();
    let listed: BTreeMap<&str, &str> = RELEASE_ONLY.iter().copied().collect();

    let missing: Vec<&str> = scan
        .gated
        .iter()
        .filter(|g| g.release_only && !listed.contains_key(g.target.as_str()))
        .map(|g| g.target.as_str())
        .collect();
    assert!(
        missing.is_empty(),
        "these tests are gated on a feature AND `not(debug_assertions)`, so \
         they still compile to an empty binary in a debug build with the \
         feature ON — `required-features` cannot reach that half:\n  {}\n\n\
         Add each to RELEASE_ONLY in this file with the lane that actually \
         runs it.",
        missing.join("\n  ")
    );

    let actual: BTreeSet<&str> = scan
        .gated
        .iter()
        .filter(|g| g.release_only)
        .map(|g| g.target.as_str())
        .collect();
    for (name, reason) in RELEASE_ONLY {
        assert!(
            actual.contains(name),
            "RELEASE_ONLY names `{name}`, which is no longer a test gated on a \
             feature AND `not(debug_assertions)` (renamed? deleted? un-gated?). \
             Remove the entry."
        );
        assert!(
            reason.len() > 40,
            "RELEASE_ONLY entry `{name}` needs a reason someone can argue \
             with, not a label"
        );
    }
}

/// An allowlist entry must correspond to a target that really is gated and
/// really is missing its declaration — otherwise the table rots into a list
/// of excuses for problems that no longer exist.
#[test]
fn allowlist_entries_are_real_and_still_needed() {
    let scan = scan();
    for (pkg, target, reason) in ALLOWED_VACUOUS {
        let g = scan
            .gated
            .iter()
            .find(|g| g.package == *pkg && g.target == *target)
            .unwrap_or_else(|| {
                panic!(
                    "ALLOWED_VACUOUS names {pkg}/{target}, which is not a \
                     feature-gated integration test. Remove the entry."
                )
            });
        let src = std::fs::read_to_string(&g.manifest).expect("read manifest");
        let declared = declared_test_targets(&src);
        assert!(
            !declared.contains_key(&g.target),
            "ALLOWED_VACUOUS excuses {pkg}/{target}, but its manifest already \
             declares `[[test]] required-features` for it. Drop the excuse."
        );
        assert!(
            reason.len() > 40,
            "ALLOWED_VACUOUS entry {pkg}/{target} needs a reason someone can \
             argue with, not a label"
        );
    }
}
