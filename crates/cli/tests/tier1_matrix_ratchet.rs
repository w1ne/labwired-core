// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

// Regression ratchet: recorded `pass` cells in docs/coverage/tier1-matrix.json
// may never silently regress. Skips before the snapshot exists.
use labwired_cli::tier1;

#[test]
// ~5.5 min per run in debug (rom-boot, 30M steps). CI runs this in the
// dedicated release step (core-ci.yml); locally: cargo test --release.
#[cfg_attr(
    debug_assertions,
    ignore = "tier1 matrix sims run in release (see core-ci.yml tier1 step)"
)]
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
    let (live, skipped) = tier1::run_all(bin).unwrap_or_else(|e| panic!("tier1 run_all: {e}"));
    for chip in &skipped {
        eprintln!("SKIP: {chip} (fixture not present)");
    }

    let disarmed = tier1::skipped_chips_with_recorded_passes(&snapshot, &skipped);
    assert!(
        disarmed.is_empty(),
        "fixtures missing for chips with recorded passes (gate would be silently disarmed): {disarmed:?}. \
         Restore tests/fixtures/tier1/ blobs or explicitly edit the snapshot."
    );

    let regressions = tier1::ratchet_regressions(&snapshot, &live);
    assert!(
        regressions.is_empty(),
        "tier1 matrix regressed: {regressions:?}. If intentional, edit the snapshot \
         explicitly; to record improvements regenerate: \
         cargo run -p labwired-cli -- tier1-matrix --json-out docs/coverage/tier1-matrix.json"
    );
}
