// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RATCHET: release-only code must live where the release lane compiles it.
//!
//! # The bug this exists to prevent
//!
//! Every cargo step in `core-ci.yml` builds DEBUG. A
//! `#[cfg(not(debug_assertions))]` block is therefore not just untested — the
//! compiler never sees it, so it can drift out of sync with the API it calls
//! and every lane stays green.
//!
//! That happened. `crates/core/tests/event_scheduler.rs` called
//! `drain_due(&[0u32])` from inside a release-only block long after
//! `EventScheduler::drain_due` lost its argument. `cargo test --release -p
//! labwired-core` could not COMPILE; `cargo test -p labwired-core` compiled
//! and passed, because the broken block was excluded. The release lane was not
//! passing — it was unrun.
//!
//! # The contract
//!
//! `core-ci.yml`'s `Release-gated tests` step names an explicit, short list of
//! targets to build in release. That list is only as good as its coverage, and
//! a list in a YAML file cannot notice a new release-only block appearing in a
//! file it does not name. This test is the noticing.
//!
//! It scans the crate for `cfg(not(debug_assertions))` and fails when one
//! appears outside [`RELEASE_LANE_TARGETS`]. Adding release-only code is fine —
//! it just has to come with the CI target that compiles it, in the same commit.
//!
//! Deliberately a plain source scan, not a compile-time trick: the whole point
//! is to see code the compiler is NOT looking at.

use std::fs;
use std::path::{Path, PathBuf};

/// Source files the `Release-gated tests` CI step actually builds in release.
///
/// Paths are relative to `crates/core/`. Keep in exact sync with the `--lib` /
/// `--test <name>` list in that step: `--lib` covers everything under `src/`
/// that the library graph reaches, and each `--test <name>` covers
/// `tests/<name>.rs`.
const RELEASE_LANE_TARGETS: &[&str] = &[
    // `--lib`
    "src/",
    // `--test event_scheduler`
    "tests/event_scheduler.rs",
    // `--test release_only_cfg_ratchet` (this file)
    "tests/release_only_cfg_ratchet.rs",
    // `--test world_esp32c3_ble_pong`
    "tests/world_esp32c3_ble_pong.rs",
    // `--test esp32c3_pms_shipped_firmware`
    "tests/esp32c3_pms_shipped_firmware.rs",
];

/// The marker that makes code invisible to a debug build.
const RELEASE_ONLY_MARKER: &str = "cfg(not(debug_assertions))";

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `dir`, relative to the crate root.
fn rust_sources(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_path_buf());
            }
        }
    }
}

fn covered_by_release_lane(rel: &Path) -> bool {
    let rel = rel.to_string_lossy().replace('\\', "/");
    RELEASE_LANE_TARGETS.iter().any(|t| {
        if t.ends_with('/') {
            rel.starts_with(t)
        } else {
            rel == *t
        }
    })
}

#[test]
fn release_only_code_is_compiled_by_the_release_lane() {
    let root = crate_root();
    let mut files = Vec::new();
    rust_sources(&root.join("src"), &root, &mut files);
    rust_sources(&root.join("tests"), &root, &mut files);
    files.sort();

    // A scan that finds nothing because it looked nowhere is the exact failure
    // mode this file is about. Prove the walk saw the crate.
    assert!(
        files.len() > 50,
        "source walk found only {} files — it is not scanning the crate",
        files.len()
    );

    let mut orphans = Vec::new();
    let mut seen_any = false;
    for rel in &files {
        let Ok(text) = fs::read_to_string(root.join(rel)) else {
            continue;
        };
        if !text.contains(RELEASE_ONLY_MARKER) {
            continue;
        }
        seen_any = true;
        if !covered_by_release_lane(rel) {
            orphans.push(rel.display().to_string());
        }
    }

    // If the marker has vanished crate-wide, the ratchet is vacuous and the CI
    // step it guards is dead weight — say so rather than pass silently.
    assert!(
        seen_any,
        "no `{RELEASE_ONLY_MARKER}` remains in labwired-core. Either delete this \
         ratchet and the `Release-gated tests` CI step together, or restore the \
         release-only code they exist for."
    );

    assert!(
        orphans.is_empty(),
        "release-only code in files the release lane never compiles:\n  {}\n\n\
         `#[cfg({RELEASE_ONLY_MARKER})]` is invisible to a debug build — not \
         untested, UNCOMPILED — so it drifts out of sync with the API it calls \
         while every debug lane stays green (that is how \
         `event_scheduler.rs` came to call a 0-argument `drain_due` with an \
         argument). Add the target to the `Release-gated tests` step in \
         .github/workflows/core-ci.yml AND to RELEASE_LANE_TARGETS here, in \
         this commit.",
        orphans.join("\n  ")
    );
}
