// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

// Per-PR Tier-1 matrix harness. Runs every target whose committed fixture
// exists; skips cleanly (like svd_coverage_ratchet) on fresh clones or before
// the fixture blobs land.
use labwired_cli::tier1;

#[test]
fn tier1_matrix_runs_all_available_fixtures() {
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_labwired"));
    let (matrix, skipped) = tier1::run_all(bin).expect("tier1 run_all");
    for chip in &skipped {
        eprintln!("SKIP: {chip} (fixture not present)");
    }
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
