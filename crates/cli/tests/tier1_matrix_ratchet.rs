// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

// Regression ratchet: recorded `pass` cells in docs/coverage/tier1-matrix.json
// may never silently regress. Skips before the snapshot exists.
use labwired_cli::tier1;

#[test]
fn tier1_matrix_does_not_regress() {
    let root = tier1::workspace_root();
    let snapshot_path = root.join("docs/coverage/tier1-matrix.json");
    if !snapshot_path.exists() {
        eprintln!("SKIP: no tier1 snapshot at {}", snapshot_path.display());
        return;
    }
    let snapshot: tier1::Tier1Matrix =
        serde_json::from_str(&std::fs::read_to_string(&snapshot_path).expect("read snapshot"))
            .expect("parse snapshot");

    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_labwired"));
    let (live, skipped) = tier1::run_all(bin).expect("tier1 run_all");
    for chip in &skipped {
        eprintln!("SKIP: {chip} (fixture not present)");
    }

    let regressions = tier1::ratchet_regressions(&snapshot, &live);
    assert!(
        regressions.is_empty(),
        "tier1 matrix regressed: {regressions:?}. If intentional, edit the snapshot \
         explicitly; to record improvements regenerate: \
         cargo run -p labwired-cli -- tier1-matrix --json-out docs/coverage/tier1-matrix.json"
    );
}
