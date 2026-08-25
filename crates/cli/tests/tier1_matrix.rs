// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

// Per-PR Tier-1 matrix harness. Runs every target whose committed fixture
// exists; skips cleanly (like svd_coverage_ratchet) on fresh clones or before
// the fixture blobs land.
//
// TIER1_TARGETS holds 17 entries: one written as a literal Tier1Target and
// sixteen built by the fast_boot() helper. Grepping for `chip: "…"` finds only
// the literal — worth knowing before anyone counts them that way.
//
// THE FLOOR. This used to assert row completeness by iterating the matrix it
// got back — so when `run_all` returned NOTHING, the loop body never executed
// and the test passed having checked nothing. "Skips cleanly" was doing more
// work than intended: an absent fixture and a harness that silently produced no
// rows were the same green. The skip is still legitimate, but it now has to be
// EXPLAINED: every target is either exercised or named as skipped for a reason
// that is visible from disk, and the two sets must together account for every
// declared target.
use labwired_cli::tier1;

#[test]
// ~5.5 min per run in debug (rom-boot, 30M steps). CI runs this in the
// dedicated release step (core-ci.yml); locally: cargo test --release.
#[cfg_attr(
    debug_assertions,
    ignore = "tier1 matrix sims run in release (see core-ci.yml tier1 step)"
)]
fn tier1_matrix_runs_all_available_fixtures() {
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_labwired"));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (matrix, skipped) = tier1::run_all(bin).unwrap_or_else(|e| panic!("tier1 run_all: {e}"));
    for chip in &skipped {
        eprintln!("SKIP: {chip} (fixture not present)");
    }

    // Every declared target is accounted for exactly once, as either exercised
    // or skipped. A target that appears in neither is a target the harness
    // quietly dropped.
    let exercised: std::collections::BTreeSet<&str> = matrix.0.keys().map(|s| s.as_str()).collect();
    let skipped_set: std::collections::BTreeSet<&str> =
        skipped.iter().map(|s| s.as_str()).collect();
    let unaccounted: Vec<&str> = tier1::TIER1_TARGETS
        .iter()
        .map(|t| t.chip)
        .filter(|c| !exercised.contains(c) && !skipped_set.contains(c))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "targets neither exercised nor skipped: {unaccounted:?} — the harness dropped them"
    );

    // A target may only be skipped because its fixture is genuinely absent from
    // disk. Without this, "skipped" is an unfalsifiable excuse and an empty
    // matrix is indistinguishable from a working one.
    for chip in &skipped {
        let target = tier1::TIER1_TARGETS
            .iter()
            .find(|t| t.chip == chip.as_str())
            .unwrap_or_else(|| panic!("skipped chip {chip} is not a declared target"));
        let elf = root.join(target.elf);
        assert!(
            !elf.exists(),
            "{chip} was skipped but its fixture IS present at {} — that is a silent drop, not a skip",
            elf.display()
        );
    }

    // THE FLOOR: if any fixture is on disk, the matrix must not be empty. This
    // is the assertion whose absence let a no-op run report success.
    let present: Vec<&str> = tier1::TIER1_TARGETS
        .iter()
        .filter(|t| root.join(t.elf).exists())
        .map(|t| t.chip)
        .collect();
    assert_eq!(
        exercised.len(),
        present.len(),
        "fixtures on disk: {present:?}, but the matrix exercised {exercised:?} — \
         a matrix that runs fewer chips than it has fixtures for is not a matrix"
    );

    // Every exercised chip must produce a full row (rubric + extra classes).
    for (chip, row) in &matrix.0 {
        let target = tier1::TIER1_TARGETS
            .iter()
            .find(|t| t.chip == chip.as_str())
            .expect("target for chip");
        let expected = tier1::RUBRIC_CLASSES.len() + target.extra_classes.len();
        assert_eq!(row.len(), expected, "{chip}: row incomplete: {row:?}");
    }
}
