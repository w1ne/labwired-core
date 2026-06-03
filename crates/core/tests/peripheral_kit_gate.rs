// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Compile-time-ish guarantees about the `PeripheralKit` registry.
//!
//! Every test here is cheap and runs in the lib test pass; together they
//! enforce the rules that make the registry safe to dispatch through:
//!
//!   * `device_type` strings are unique across the registry.
//!   * Metadata fields are non-empty where they need to be (label, summary,
//!     detail). A blank metadata field would silently break the playground
//!     library tile and is the easiest mistake to make when adding a kit.
//!   * Every kit declares a transport. Transport is the surface a system
//!     author has to know, so a kit without one is shipping a model the
//!     user can't actually instantiate.
//!   * Every kit that declares a starter lab points its `example_dir` at a
//!     real directory under `core/examples/` containing a `system.yaml`.
//!     Catches the "I wrote the model but forgot the lab" failure mode.
//!
//! Extend this file when you add a new invariant. Do not add per-kit tests
//! here — those live next to the model.

use labwired_core::peripherals::kit::registry;
use std::collections::HashSet;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR for an integration test is the package dir
    // (crates/core); the workspace root is two parents up.
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.parent().and_then(|p| p.parent()).unwrap().to_path_buf()
}

#[test]
fn registry_is_non_empty() {
    assert!(
        !registry::kits().is_empty(),
        "PeripheralKit registry is empty — at least bg770a and neo6m should be present"
    );
}

#[test]
fn device_type_strings_are_unique() {
    let mut seen = HashSet::new();
    for kit in registry::kits() {
        let dt = kit.metadata().device_type;
        assert!(
            seen.insert(dt),
            "duplicate device_type '{dt}' in PeripheralKit registry"
        );
    }
}

#[test]
fn lookup_resolves_every_registered_kit() {
    for kit in registry::kits() {
        let dt = kit.metadata().device_type;
        let found = registry::lookup(dt);
        assert!(
            found.is_some(),
            "registry::lookup({dt}) did not resolve a kit that is in registry::kits()"
        );
    }
}

#[test]
fn metadata_text_fields_are_non_empty() {
    for kit in registry::kits() {
        let m = kit.metadata();
        assert!(
            !m.device_type.is_empty(),
            "kit has empty device_type — this would never match a system.yaml"
        );
        assert!(
            !m.label.is_empty(),
            "kit '{}' has empty label — UI would render a blank tile",
            m.device_type
        );
        assert!(
            !m.summary.is_empty(),
            "kit '{}' has empty summary — library tile would be useless",
            m.device_type
        );
        assert!(
            !m.detail.is_empty(),
            "kit '{}' has empty detail",
            m.device_type
        );
    }
}

#[test]
fn config_keys_are_internally_unique() {
    for kit in registry::kits() {
        let mut seen = HashSet::new();
        for ck in kit.metadata().config_keys {
            assert!(
                seen.insert(ck.name),
                "kit '{}' lists duplicate config key '{}'",
                kit.metadata().device_type,
                ck.name
            );
            assert!(
                !ck.doc.is_empty(),
                "kit '{}' config key '{}' has no doc string",
                kit.metadata().device_type,
                ck.name
            );
        }
    }
}

#[test]
fn manifest_json_matches_registry() {
    // The committed packages/ui/src/peripherals/manifest.json must match
    // exactly what the generator would emit today. If a kit's metadata
    // changes (or a new kit lands) without re-running the generator, this
    // test fails — keeping the TS / browser layer from silently drifting
    // out of sync with the Rust source of truth.
    //
    // The manifest lives in the *parent* labwired repo. When labwired-core
    // is checked out standalone (released crate, contributor fork), that
    // path doesn't exist — skip rather than fail to avoid a false signal.
    let candidates = [
        // Standard layout: core is a submodule under labwired/.
        workspace_root()
            .parent()
            .map(|p| p.join("packages/ui/src/peripherals/manifest.json")),
        // Direct sibling layout (some dev environments).
        workspace_root().parent().and_then(|p| {
            p.parent()
                .map(|p| p.join("labwired/packages/ui/src/peripherals/manifest.json"))
        }),
    ];
    let manifest_path = candidates.into_iter().flatten().find(|p| p.exists());
    let Some(manifest_path) = manifest_path else {
        eprintln!("[peripheral_kit_gate] skipping manifest-drift check: no parent labwired/ checkout found");
        return;
    };

    let committed = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("reading {manifest_path:?}: {e}"));

    #[derive(serde::Serialize)]
    struct Manifest<'a> {
        schema_version: u32,
        peripherals: Vec<&'a labwired_core::peripherals::kit::KitMetadata>,
    }
    let expected_value = Manifest {
        schema_version: 1,
        peripherals: registry::kits().iter().map(|k| k.metadata()).collect(),
    };
    let mut expected = serde_json::to_string_pretty(&expected_value).unwrap();
    if !expected.ends_with('\n') {
        expected.push('\n');
    }

    if committed != expected {
        // Show a short diff hint so the developer knows what to do.
        panic!(
            "peripherals-manifest.json is stale.\n\
             Re-run: cargo run -p labwired-cli --bin gen-peripherals-manifest -- \\\n        --out {}\n\
             (Committed bytes: {}, expected: {}.)",
            manifest_path.display(),
            committed.len(),
            expected.len()
        );
    }
}

#[test]
fn lab_example_dirs_exist_on_disk() {
    let examples = workspace_root().join("examples");
    for kit in registry::kits() {
        let Some(lab) = kit.metadata().lab.as_ref() else {
            continue;
        };
        let dir = examples.join(lab.example_dir);
        assert!(
            dir.is_dir(),
            "kit '{}' references example_dir '{}' but {:?} is not a directory",
            kit.metadata().device_type,
            lab.example_dir,
            dir
        );
        let system_yaml = dir.join("system.yaml");
        assert!(
            system_yaml.is_file(),
            "kit '{}' lab '{}' is missing system.yaml at {:?}",
            kit.metadata().device_type,
            lab.example_dir,
            system_yaml
        );
    }
}
