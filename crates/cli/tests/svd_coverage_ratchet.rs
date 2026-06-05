// Regression ratchet: per-peripheral `modelled` coverage must never DROP below
// the committed snapshot. Gated on the ESP32-S3 SVD being discoverable — skips
// cleanly where the toolchain isn't installed (like the firmware e2e tests).

use std::collections::BTreeMap;

#[test]
fn esp32s3_coverage_does_not_regress() {
    if labwired_cli::coverage::discover_svd().is_none() {
        eprintln!("SKIP: ESP32-S3 SVD not found (set LABWIRED_ESP32S3_SVD)");
        return;
    }
    let (live, _text) = labwired_cli::coverage::run().expect("coverage run");

    let snapshot_json = include_str!("../../../docs/coverage/esp32s3-coverage.json");
    let snapshot: labwired_cli::coverage::CoverageMatrix =
        serde_json::from_str(snapshot_json).expect("parse snapshot");

    let mut regressions: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (name, snap) in &snapshot.0 {
        let cur_modelled = live.0.get(name).map(|c| c.modelled).unwrap_or(0);
        if cur_modelled < snap.modelled {
            regressions.insert(name.clone(), (snap.modelled, cur_modelled));
        }
    }
    assert!(
        regressions.is_empty(),
        "register coverage regressed (snapshot modelled -> current): {regressions:?}. \
         If intentional, regenerate: cargo run -p labwired-cli -- coverage --json-out docs/coverage/esp32s3-coverage.json"
    );
}
