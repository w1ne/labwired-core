// Regression ratchet: per-peripheral `modelled` coverage must never DROP below
// the baseline. Gated on the ESP32-S3 SVD being discoverable — skips cleanly
// where the toolchain isn't installed (like the firmware e2e tests).
//
// THE BASELINE IS A DIFFERENT COMMIT, AND THAT IS THE WHOLE GUARANTEE.
// This used to read the snapshot with
// `include_str!("../../../docs/coverage/esp32s3-coverage.json")` — the file as
// it exists in the tree being graded. So a change that lowered coverage and
// regenerated the snapshot in the same commit compared the tree to itself and
// passed. That is exactly the change this gate exists to stop, and it was the
// one shape it could not see.
//
// It now resolves the blob at the merge-base with the baseline ref, through the
// same `crate::baseline` resolver the tier1 ratchet uses.

use std::collections::BTreeMap;
use std::path::Path;

const COVERAGE_PATH: &str = "docs/coverage/esp32s3-coverage.json";

/// Repo root = crates/cli/../.. (same convention as the other CLI tests).
fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

#[test]
fn esp32s3_coverage_does_not_regress() {
    if labwired_cli::coverage::discover_svd().is_none() {
        eprintln!("SKIP: ESP32-S3 SVD not found (set LABWIRED_ESP32S3_SVD)");
        return;
    }
    let (live, _text) = labwired_cli::coverage::run().expect("coverage run");

    let root = repo_root();
    let base_ref = labwired_cli::baseline::baseline_ref();
    let found = labwired_cli::baseline::resolve(
        &root,
        &base_ref,
        COVERAGE_PATH,
        "svd coverage ratchet",
    )
    .expect("resolve the coverage baseline");

    let Some(blob) = found.blob else {
        // The snapshot is new in this commit: nothing has been promised yet, so
        // there is no earlier number to protect.
        eprintln!("SKIP: no baseline coverage snapshot yet ({})", found.label);
        return;
    };

    let snapshot: labwired_cli::coverage::CoverageMatrix = serde_json::from_str(&blob)
        .unwrap_or_else(|e| panic!("baseline {COVERAGE_PATH} at {} does not parse: {e}", found.label));

    let mut regressions: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (name, snap) in &snapshot.0 {
        let cur_modelled = live.0.get(name).map(|c| c.modelled).unwrap_or(0);
        if cur_modelled < snap.modelled {
            regressions.insert(name.clone(), (snap.modelled, cur_modelled));
        }
    }
    assert!(
        regressions.is_empty(),
        "register coverage regressed against baseline {} (baseline modelled -> current): {regressions:?}. \
         If intentional, regenerate: cargo run -p labwired-cli -- coverage --json-out {COVERAGE_PATH}",
        found.label
    );
}
