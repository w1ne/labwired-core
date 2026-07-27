// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Cross-registry consistency: `validation/manifest.yaml` is the single source
//! of truth for which boards exist and what is claimed about them. Four
//! registries independently name chips — the manifest, `SURVIVAL_CASES`
//! (firmware_survival.rs), `TIER1_TARGETS` (labwired-cli), and the F4 silicon
//! oracle's `F4Target` table. A chip path typo, or a board added to one
//! registry and forgotten in another, is silent under-coverage. This test
//! makes it a failure.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Every `chip:` path named by `validation/manifest.yaml`, as a chip stem
/// (`configs/chips/stm32f401.yaml` → `stm32f401`).
fn manifest_chip_stems() -> BTreeSet<String> {
    let text = std::fs::read_to_string(workspace_root().join("validation/manifest.yaml"))
        .expect("read validation/manifest.yaml");
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("chip:") {
            let path = rest.trim();
            let stem = Path::new(path)
                .file_stem()
                .unwrap_or_else(|| panic!("malformed chip path in manifest: {path}"))
                .to_string_lossy()
                .to_string();
            out.insert(stem);
        }
    }
    assert!(!out.is_empty(), "parsed no chip: entries from the manifest");
    out
}

/// Every `doc:` path named by `validation/manifest.yaml`.
fn manifest_doc_paths() -> Vec<String> {
    let text = std::fs::read_to_string(workspace_root().join("validation/manifest.yaml"))
        .expect("read validation/manifest.yaml");
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("doc:") {
            out.push(rest.trim().to_string());
        }
    }
    assert!(!out.is_empty(), "parsed no doc: entries from the manifest");
    out
}

/// Chip stems named by `SURVIVAL_CASES`. Read from the test source rather than
/// imported: integration tests are separate binaries.
fn survival_chip_stems() -> BTreeSet<String> {
    let text =
        std::fs::read_to_string(workspace_root().join("crates/core/tests/firmware_survival.rs"))
            .expect("read firmware_survival.rs");
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("chip: \"") {
            if let Some(name) = rest.split('"').next() {
                out.insert(name.to_string());
            }
        }
    }
    assert!(
        !out.is_empty(),
        "parsed no chip: fields from SURVIVAL_CASES"
    );
    out
}

#[test]
fn every_manifest_chip_path_exists_on_disk() {
    for stem in manifest_chip_stems() {
        let path = workspace_root()
            .join("configs/chips")
            .join(format!("{stem}.yaml"));
        assert!(
            path.exists(),
            "validation/manifest.yaml names chip {stem:?} but {path:?} does not exist"
        );
    }
}

#[test]
fn every_manifest_doc_path_exists_on_disk() {
    for doc in manifest_doc_paths() {
        let path = workspace_root().join(&doc);
        assert!(
            path.exists(),
            "validation/manifest.yaml names doc {doc:?} but {path:?} does not exist"
        );
    }
}

#[test]
fn every_survival_chip_is_a_known_manifest_board() {
    let manifest = manifest_chip_stems();
    for stem in survival_chip_stems() {
        assert!(
            manifest.contains(&stem),
            "SURVIVAL_CASES runs chip {stem:?} but no board in \
             validation/manifest.yaml declares it. Add a board entry — the \
             manifest is the source of truth for what is claimed."
        );
    }
}

// The equivalent `every_tier1_target_is_a_known_manifest_board` assertion lives
// in crates/cli/src/tier1.rs's own #[cfg(test)] mod tests instead of here:
// labwired-core cannot take a dev-dependency on labwired-cli without creating a
// dependency cycle (labwired-cli already depends on labwired-core), and
// TIER1_TARGETS is only reachable from within the cli crate.
