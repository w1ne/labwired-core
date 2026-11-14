// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Cross-registry consistency: `validation/boards/` (one file per board) is the single source
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

/// Every board file under `validation/boards/` (one `<id>.yaml` per board; the
/// `_index.yaml` header is skipped), concatenated in file-name order. Callers
/// scan it line by line, so concatenation is equivalent to the old single file.
fn manifest_text() -> String {
    let dir = workspace_root().join("validation/boards");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "yaml")
                && p.file_name().is_some_and(|n| n != "_index.yaml")
        })
        .collect();
    paths.sort();
    let mut out = String::new();
    for p in paths {
        out.push_str(
            &std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display())),
        );
        out.push('\n');
    }
    out
}

/// Every `chip:` path named by `validation/boards/`, as a chip stem
/// (`configs/chips/stm32f401.yaml` → `stm32f401`).
fn manifest_chip_stems() -> BTreeSet<String> {
    let text = manifest_text();
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

/// Every `doc:` path named by `validation/boards/`.
fn manifest_doc_paths() -> Vec<String> {
    let text = manifest_text();
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
            "validation/boards/ names chip {stem:?} but {path:?} does not exist"
        );
    }
}

#[test]
fn every_manifest_doc_path_exists_on_disk() {
    for doc in manifest_doc_paths() {
        let path = workspace_root().join(&doc);
        assert!(
            path.exists(),
            "validation/boards/ names doc {doc:?} but {path:?} does not exist"
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
             validation/boards/ declares it. Add a board entry — the \
             manifest is the source of truth for what is claimed."
        );
    }
}

/// The survival harness resolves systems from `configs/systems/`, while the CLI
/// smoke lanes read `examples/<board>/system.yaml`. Where both exist, the pair
/// is two sources of truth for one machine: if they drift, the PR-run gate
/// exercises a different board than the CLI lane. Pin the pairs the Tier-1
/// survival gates rely on. `name` differs by convention (the example carries an
/// `-example` suffix), so compare the chip file stem, `board_io` and
/// `external_devices` as parsed YAML.
#[test]
fn survival_systems_match_their_example_twins() {
    const TWINS: &[&str] = &[
        "stm32f401cdu6-blackpill",
        "stm32f411ceu6-blackpill",
        "feather-f405",
        "nucleo-f767zi",
        "stm32h735-smoke",
    ];
    for name in TWINS {
        let configs = read_yaml(&format!("configs/systems/{name}.yaml"));
        let example = read_yaml(&format!("examples/{name}/system.yaml"));
        assert_eq!(
            chip_stem(&configs),
            chip_stem(&example),
            "{name}: the configs/systems and examples systems resolve different chips"
        );
        assert_eq!(
            configs.get("board_io"),
            example.get("board_io"),
            "{name}: board_io drifted between configs/systems and examples"
        );
        assert_eq!(
            configs.get("external_devices"),
            example.get("external_devices"),
            "{name}: external_devices drifted between configs/systems and examples"
        );
    }
}

fn read_yaml(rel: &str) -> serde_yaml::Value {
    let path = workspace_root().join(rel);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn chip_stem(v: &serde_yaml::Value) -> String {
    let chip = v
        .get("chip")
        .and_then(|c| c.as_str())
        .unwrap_or_else(|| panic!("system has no `chip:` field"));
    Path::new(chip)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// The equivalent `every_tier1_target_is_a_known_manifest_board` assertion lives
// in crates/cli/src/tier1.rs's own #[cfg(test)] mod tests instead of here:
// labwired-core cannot take a dev-dependency on labwired-cli without creating a
// dependency cycle (labwired-cli already depends on labwired-core), and
// TIER1_TARGETS is only reachable from within the cli crate.
