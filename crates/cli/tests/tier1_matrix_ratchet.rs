// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

// Regression ratchet: recorded `pass` cells in docs/coverage/tier1-matrix.json
// may never silently regress.
//
// TWO checks over ONE live run (the sim pass costs ~160 s; adding a second job
// would double it for no reason):
//
//   1. COMMITTED-vs-live — catches "the engine broke a cell and nobody
//      regenerated the snapshot". Skips before the snapshot exists.
//   2. BASELINE-vs-live — catches "the change broke a cell AND regenerated the
//      snapshot", which check 1 cannot see: regenerating makes the committed
//      snapshot equal the live run by construction. The baseline comes from
//      outside the change (merge base with the trunk), so the author cannot
//      move it. This is how the ESP32-C3 row went 8 pass -> 0 pass across two
//      onboarding PRs that touched no C3 file, with CI green throughout.
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
    let snapshot_path = root.join(tier1::MATRIX_PATH);
    if !snapshot_path.exists() {
        eprintln!("SKIP: no tier1 snapshot at {}", snapshot_path.display());
        return;
    }
    let snapshot: tier1::Tier1Matrix =
        serde_json::from_str(&std::fs::read_to_string(&snapshot_path).expect("read snapshot"))
            .expect("parse snapshot");

    // Resolved BEFORE the 160 s sim run so a misconfigured checkout fails fast.
    // Any failure here is fatal on purpose: a ratchet that cannot establish its
    // baseline and passes anyway is precisely the bug it exists to prevent.
    let (baseline_desc, baseline) =
        tier1::resolve_baseline_matrix(&root).unwrap_or_else(|e| panic!("tier1 ratchet: {e}"));
    let acks = tier1::load_ratchet_acks(&root)
        .unwrap_or_else(|e| panic!("tier1 ratchet: cannot read acknowledgements: {e}"));
    eprintln!("tier1 baseline: {baseline_desc}");

    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_labwired"));
    let (live, skipped) = tier1::run_all(bin).unwrap_or_else(|e| panic!("tier1 run_all: {e}"));
    for chip in &skipped {
        eprintln!("SKIP: {chip} (fixture not present)");
    }

    // A deleted fixture must not disarm either side of the gate.
    for (label, reference) in [("snapshot", &snapshot), ("baseline", &baseline)] {
        let disarmed = tier1::skipped_chips_with_recorded_passes(reference, &skipped);
        assert!(
            disarmed.is_empty(),
            "fixtures missing for chips the {label} records as passing (gate would be \
             silently disarmed): {disarmed:?}. Restore tests/fixtures/tier1/ blobs."
        );
    }

    // 1. Committed snapshot vs live — engine drift without a regen.
    let regressions = tier1::ratchet_regressions(&snapshot, &live);
    assert!(
        regressions.is_empty(),
        "tier1 matrix regressed against the committed snapshot: {regressions:?}. \
         If intentional, edit the snapshot explicitly; to record improvements regenerate: \
         cargo run -p labwired-cli -- tier1-matrix --json-out docs/coverage/tier1-matrix.json"
    );

    // 2. Immovable baseline vs live — the regen-defeats-the-ratchet hole.
    let failures = tier1::baseline_gate_failures(&baseline, &live, &acks);
    assert!(
        failures.is_empty(),
        "tier1 matrix dropped below the baseline ({baseline_desc}):\n  {}\n\n\
         Regenerating docs/coverage/tier1-matrix.json does NOT clear this — the baseline \
         is read from the merge base, outside your change. Either fix the regression, or \
         acknowledge it explicitly in {} with the chip/class, the status you now expect, \
         and why the drop is intentional.",
        failures.join("\n  "),
        tier1::RATCHET_ACK_PATH,
    );
}
