// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

// The ESP32-S3 TIER1 image must give the same verdict for every class whether
// it runs stepped or batched.
//
// The Tier-1 matrix runs stepped only, and the browser runs batched. #68 lived
// in that gap: `TIER1 dma` passed stepped and failed batched
// (`gdma-eof-timeout`), because a reset-held secondary's batch windows left
// the tick grid and peripherals stopped ticking. Nothing that ran on a PR
// could see it. This test is the integration-level holder; the planner rule
// itself is held by
// `machine_advance::reset_held_secondary_windows_end_on_the_tick_grid_above_interval_one`.
//
// No skip path: the fixture images are committed (not LFS), so a missing one
// is a broken checkout, and a skip would count as a pass.
use labwired_cli::tier1::{self, CellStatus};
use std::path::Path;

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "two 30M-step rom-boot runs; CI runs this in release (xtensa-flash-boot)"
)]
fn esp32s3_tier1_verdicts_match_between_step_and_batched() {
    let target = tier1::TIER1_TARGETS
        .iter()
        .find(|t| t.chip == "esp32s3")
        .expect("esp32s3 row in TIER1_TARGETS");
    let bin = Path::new(env!("CARGO_BIN_EXE_labwired"));

    let stepped = tier1::run_target_mode(target, bin, false).expect("stepped run");
    let batched = tier1::run_target_mode(target, bin, true).expect("batched run");

    // Anti-vacuity: agreement between two empty or all-blocked rows would pass.
    // The class #68 lost must be a PASS on the reference (stepped) path.
    let dma = stepped.get("dma").map(|c| c.status);
    assert_eq!(
        dma,
        Some(CellStatus::Pass),
        "precondition: stepped `dma` must PASS; row: {stepped:?}"
    );

    let differing: Vec<String> = stepped
        .keys()
        .chain(batched.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|k| stepped.get(*k) != batched.get(*k))
        .map(|k| {
            format!(
                "{k}: stepped {:?} / batched {:?}",
                stepped.get(k),
                batched.get(k)
            )
        })
        .collect();
    assert!(
        differing.is_empty(),
        "ESP32-S3 TIER1 verdicts differ between step and batched mode:\n  {}",
        differing.join("\n  ")
    );
}
